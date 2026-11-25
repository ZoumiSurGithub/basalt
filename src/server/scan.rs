//! Analyse de la carte : où sont les objets, et ce qui ressemble à une duplication.
//!
//! L'analyse compte tous les objets rangés quelque part — coffres et autres conteneurs,
//! inventaires des joueurs en jeu, inventaires enregistrés des joueurs absents — puis
//! relève ce qui sort de l'ordinaire :
//!
//! - une **pile impossible** (plus d'objets dans une case que le jeu n'en permet) ;
//! - des **conteneurs jumeaux** (plusieurs conteneurs bien remplis au contenu identique,
//!   case pour case) ;
//! - un objet **désactivé** encore présent ;
//! - qui détient le plus d'**objets rares**.
//!
//! Ce sont des indices, pas des preuves : deux coffres rangés de la même façon existent
//! aussi chez les joueurs honnêtes. L'outil dit où regarder, pas qui punir.
//!
//! Le serveur ne fait qu'une copie de ce qu'il a en mémoire (quelques millisecondes) ; le
//! comptage et la lecture des fichiers des joueurs absents se font sur un fil à part.

use super::*;
use std::sync::Mutex;

/// État de l'analyse, partagé avec le fil qui la fait.
#[derive(Default)]
pub(super) struct ScanState {
    pub running: bool,
    /// Dernier rapport (JSON), s'il y en a un
    pub report: Option<String>,
}

/// Ce qu'un détenteur garde dans une case : (objet, quantité).
type Slot = (String, u32);

/// Un endroit où des objets sont rangés.
pub(super) struct Holder {
    /// « coffre 12 64 -30 », « joueur Alex », « joueur absent alex »
    pub label: String,
    /// "container", "player" ou "offline"
    pub kind: &'static str,
    /// Les cases, vides comprises (pour comparer deux conteneurs case pour case)
    pub slots: Vec<Option<Slot>>,
}

/// Objets dont on veut savoir qui en détient le plus.
const RARE: [&str; 16] = [
    "diamond_block", "diamond", "netherite_ingot", "netherite_block", "ancient_debris", "emerald_block", "enchanted_golden_apple", "totem_of_undying", "elytra", "nether_star", "beacon", "dragon_egg", "shulker_box", "gold_block", "iron_block", "trident",
];

/// Nombre de lignes gardées dans chaque partie du rapport
const TOP: usize = 300;

fn short(name: &str) -> &str {
    name.trim_start_matches("minecraft:")
}

/// Compose le rapport à partir de tout ce qui a été relevé.
pub(super) fn report(holders: &[Holder], max_stack: &dyn Fn(&str) -> u32, disabled: &HashSet<String>, seconds: f64) -> Json {
    let obj = |pairs: Vec<(&str, Json)>| Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
    let num = |n: usize| Json::Num(n as f64);
    let text = |s: &str| Json::Str(s.to_string());
    // Totaux par objet, et qui en a le plus
    struct Total {
        all: u64,
        containers: u64,
        players: u64,
        holders: Vec<(u64, usize)>,
    }
    let mut totals: HashMap<&str, Total> = HashMap::new();
    let mut alerts: Vec<Json> = Vec::new();
    let mut impossible = 0usize;
    for (index, h) in holders.iter().enumerate() {
        let mut mine: HashMap<&str, u64> = HashMap::new();
        for (item, count) in h.slots.iter().flatten() {
            *mine.entry(short(item)).or_default() += *count as u64;
            if *count > max_stack(item) {
                impossible += 1;
                if alerts.len() < TOP {
                    alerts.push(obj(vec![("kind", text("stack")), ("where", text(&h.label)), ("item", text(short(item))), ("count", num(*count as usize)), ("max", num(max_stack(item) as usize))]));
                }
            }
        }
        for (item, count) in mine {
            let t = totals.entry(item).or_insert(Total { all: 0, containers: 0, players: 0, holders: Vec::new() });
            t.all += count;
            if h.kind == "container" {
                t.containers += count;
            } else {
                t.players += count;
            }
            t.holders.push((count, index));
        }
    }
    // Conteneurs jumeaux : même contenu, case pour case, et assez rempli pour que ce ne soit
    // pas un hasard (au moins trois cases différentes et 64 objets)
    let mut twins: HashMap<&[Option<Slot>], Vec<usize>> = HashMap::new();
    for (index, h) in holders.iter().enumerate().filter(|(_, h)| h.kind == "container") {
        let filled = h.slots.iter().flatten().count();
        let amount: u64 = h.slots.iter().flatten().map(|s| s.1 as u64).sum();
        let kinds: HashSet<&str> = h.slots.iter().flatten().map(|s| s.0.as_str()).collect();
        if filled >= 3 && amount >= 64 && kinds.len() >= 2 {
            twins.entry(h.slots.as_slice()).or_default().push(index);
        }
    }
    let mut groups: Vec<&Vec<usize>> = twins.values().filter(|g| g.len() >= 2).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let twin_groups = groups.len();
    for g in groups.into_iter().take(50) {
        let amount: u64 = holders[g[0]].slots.iter().flatten().map(|s| s.1 as u64).sum();
        alerts.push(obj(vec![("kind", text("twins")), ("count", num(g.len())), ("items", num(amount as usize)), ("where", Json::Arr(g.iter().take(12).map(|i| text(&holders[*i].label)).collect()))]));
    }
    // Objets désactivés encore présents
    let mut off: Vec<(&str, u64)> = totals.iter().filter(|(item, _)| disabled.contains(**item)).map(|(item, t)| (*item, t.all)).collect();
    off.sort();
    for (item, count) in off {
        alerts.push(obj(vec![("kind", text("disabled")), ("item", text(item)), ("count", num(count as usize))]));
    }
    // Les totaux, du plus abondant au plus rare
    let mut rows: Vec<(&str, &mut Total)> = totals.iter_mut().map(|(k, v)| (*k, v)).collect();
    rows.sort_by(|a, b| b.1.all.cmp(&a.1.all).then(a.0.cmp(b.0)));
    let kinds_found = rows.len();
    let items_found: u64 = rows.iter().map(|r| r.1.all).sum();
    let line = |item: &str, t: &mut Total| {
        t.holders.sort_by_key(|h| std::cmp::Reverse(h.0));
        obj(vec![
            ("item", text(item)),
            ("total", num(t.all as usize)),
            ("containers", num(t.containers as usize)),
            ("players", num(t.players as usize)),
            ("top", Json::Arr(t.holders.iter().take(5).map(|(count, i)| obj(vec![("where", text(&holders[*i].label)), ("count", num(*count as usize))])).collect())),
        ])
    };
    let rare: Vec<Json> = rows.iter_mut().filter(|(item, _)| RARE.contains(item)).map(|(item, t)| line(item, t)).collect();
    let all: Vec<Json> = rows.iter_mut().take(TOP).map(|(item, t)| line(item, t)).collect();
    let count_of = |kind: &str| holders.iter().filter(|h| h.kind == kind).count();
    obj(vec![
        ("seconds", Json::Num((seconds * 100.0).round() / 100.0)),
        ("containers", num(count_of("container"))),
        ("players_online", num(count_of("player"))),
        ("players_offline", num(count_of("offline"))),
        ("kinds", num(kinds_found)),
        ("items", num(items_found as usize)),
        ("impossible_stacks", num(impossible)),
        ("twin_groups", num(twin_groups)),
        ("alerts", Json::Arr(alerts)),
        ("rare", Json::Arr(rare)),
        ("totals", Json::Arr(all)),
    ])
}

/// Les cases d'un inventaire enregistré d'un joueur absent.
fn saved_slots(player: &crate::player::Player) -> Vec<Option<Slot>> {
    player.items.iter().filter(|i| i.3 > 0).map(|i| Some((i.2.clone(), i.3 as u32))).collect()
}

impl Server {
    /// Lance une analyse. Faux si une autre est déjà en cours.
    pub(super) fn start_scan(&mut self) -> bool {
        let Some(d) = self.data.clone() else { return false };
        {
            let Ok(mut state) = self.scan.lock() else { return false };
            if state.running {
                return false;
            }
            state.running = true;
        }
        let started = Instant::now();
        let name_of = |id: i16| d.item_by_id(id).map(|def| def.name.clone());
        let slots_of = |slots: &[inventory::ItemStack]| -> Vec<Option<Slot>> { slots.iter().map(|s| if s.is_empty() { None } else { name_of(s.id).map(|n| (n, s.count as u32)) }).collect() };
        // La copie de ce qui est en mémoire : conteneurs, puis joueurs en jeu
        let mut holders: Vec<Holder> = Vec::new();
        for pos in self.world.tiles.positions() {
            let Some(inv) = self.world.tiles.get(pos).and_then(|t| t.inventory()) else { continue };
            let slots = slots_of(inv.slots());
            if slots.iter().any(|s| s.is_some()) {
                holders.push(Holder { label: format!("conteneur {} {} {}", pos.0, pos.1, pos.2), kind: "container", slots });
            }
        }
        let mut online: HashSet<String> = HashSet::new();
        for c in self.clients.values().filter(|c| c.joined) {
            let Some(name) = c.name.as_ref() else { continue };
            online.insert(name.to_lowercase());
            let mut slots = slots_of(&c.inv.main.slots);
            slots.extend(slots_of(&c.inv.armor.slots));
            slots.extend(slots_of(&c.inv.offhand.slots));
            holders.push(Holder { label: format!("joueur {}", name), kind: "player", slots });
        }
        let (state, root, disabled) = (self.scan.clone(), self.cfg.root.clone(), self.disabled.clone());
        let spawned = std::thread::Builder::new().name("analyse".into()).spawn(move || {
            // Les joueurs absents : leurs fichiers, lus ici pour ne pas ralentir le serveur
            if let Ok(entries) = std::fs::read_dir(root.join("players")) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    let (Some(stem), Some("dat")) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|e| e.to_str())) else { continue };
                    if online.contains(&stem.to_lowercase()) {
                        continue;
                    }
                    if let Ok(Some(player)) = crate::player::Player::load(&root, stem) {
                        let slots = saved_slots(&player);
                        if !slots.is_empty() {
                            holders.push(Holder { label: format!("joueur absent {}", player.name), kind: "offline", slots });
                        }
                    }
                }
            }
            let max_stack = |item: &str| VanillaItems::get(item).max_stack_size() as u32;
            let json = report(&holders, &max_stack, &disabled, started.elapsed().as_secs_f64()).to_json();
            if let Ok(mut s) = state.lock() {
                s.report = Some(json);
                s.running = false;
            }
        });
        if spawned.is_err() {
            if let Ok(mut s) = self.scan.lock() {
                s.running = false;
            }
            return false;
        }
        true
    }

    /// Où en est l'analyse : `{"state":"idle|running|done","report":{…}}`.
    pub(super) fn scan_status(&self) -> String {
        match self.scan.lock() {
            Ok(s) if s.running => "{\"state\":\"running\"}".to_string(),
            Ok(s) => match &s.report {
                Some(report) => format!("{{\"state\":\"done\",\"report\":{}}}", report),
                None => "{\"state\":\"idle\"}".to_string(),
            },
            Err(_) => "{\"state\":\"idle\"}".to_string(),
        }
    }
}

/// (pour créer l'état partagé depuis le serveur)
pub(super) fn new_state() -> Arc<Mutex<ScanState>> {
    Arc::new(Mutex::new(ScanState::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(item: &str, count: u32) -> Option<Slot> {
        Some((format!("minecraft:{}", item), count))
    }

    #[test]
    fn the_report_counts_everything_and_points_at_what_is_odd() {
        let rich = vec![slot("diamond_block", 64), slot("gold_ingot", 32), None, slot("emerald", 10)];
        let holders = vec![
            Holder { label: "conteneur 0 64 0".into(), kind: "container", slots: rich.clone() },
            Holder { label: "conteneur 5 64 0".into(), kind: "container", slots: rich.clone() },
            // (le même contenu rangé autrement n'est pas un jumeau)
            Holder { label: "conteneur 9 64 0".into(), kind: "container", slots: vec![slot("gold_ingot", 32), slot("diamond_block", 64), slot("emerald", 10)] },
            // (trop peu rempli pour compter comme jumeau)
            Holder { label: "conteneur 1 1 1".into(), kind: "container", slots: vec![slot("dirt", 3)] },
            Holder { label: "conteneur 2 2 2".into(), kind: "container", slots: vec![slot("dirt", 3)] },
            Holder { label: "joueur Alex".into(), kind: "player", slots: vec![slot("diamond_block", 200), slot("tnt", 5)] },
            Holder { label: "joueur absent zoe".into(), kind: "offline", slots: vec![slot("diamond_block", 1)] },
        ];
        let disabled: HashSet<String> = ["tnt".to_string()].into_iter().collect();
        let r = report(&holders, &|_| 64, &disabled, 0.5);
        let n = |key: &str| r.get(key).and_then(|v| v.as_f64()).unwrap() as u64;
        assert_eq!((n("containers"), n("players_online"), n("players_offline")), (5, 1, 1));
        assert_eq!((n("impossible_stacks"), n("twin_groups")), (1, 1));
        // 64 × 3 dans les coffres, 200 et 1 chez les joueurs
        let totals = r.get("totals").and_then(|t| t.as_arr()).unwrap();
        let diamonds = &totals[0];
        assert_eq!(diamonds.get("item").and_then(|v| v.as_str()), Some("diamond_block"));
        assert_eq!(diamonds.get("total").and_then(|v| v.as_f64()), Some(393.0));
        assert_eq!((diamonds.get("containers").and_then(|v| v.as_f64()), diamonds.get("players").and_then(|v| v.as_f64())), (Some(192.0), Some(201.0)));
        // Celui qui en a le plus est nommé en premier
        let top = diamonds.get("top").and_then(|t| t.as_arr()).unwrap();
        assert_eq!(top[0].get("where").and_then(|v| v.as_str()), Some("joueur Alex"));
        // Les alertes : la pile de 200, les deux coffres jumeaux, la TNT désactivée
        let alerts = r.get("alerts").and_then(|a| a.as_arr()).unwrap();
        let kinds: Vec<&str> = alerts.iter().filter_map(|a| a.get("kind").and_then(|k| k.as_str())).collect();
        assert_eq!(kinds, vec!["stack", "twins", "disabled"]);
        let twins = alerts[1].get("where").and_then(|w| w.as_arr()).unwrap();
        assert_eq!(twins.len(), 2);
        // Les objets rares ont leur propre liste
        let rare = r.get("rare").and_then(|t| t.as_arr()).unwrap();
        assert_eq!(rare.len(), 1);
    }
}
