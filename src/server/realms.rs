//! Plusieurs mondes ouverts à la fois. Chaque joueur est dans un monde ; les créatures, les
//! objets au sol, la météo, l'heure et le terrain sont propres à chacun.
//!
//! Tout le reste du serveur a été écrit pour un seul monde : plutôt que de le réécrire, le
//! serveur « entre » dans un monde avant d'y travailler. Entrer, c'est échanger d'un coup ce
//! qui appartient au monde (ses joueurs compris) avec ce que le serveur tient en main : le
//! code qui suit ne voit alors que ce monde-là. Un paquet reçu fait entrer dans le monde de
//! son joueur ; à chaque tick, le serveur passe par tous les mondes, l'un après l'autre.

use super::*;

/// Mondes ouverts à la fois, au plus.
const MAX_WORLDS: usize = 32;
/// Dossier de tous les mondes, à côté de server.properties : `worlds/<nom>`, comme le serveur
/// du jeu (le monde principal est `worlds/<level-name>`).
const WORLDS_DIR: &str = "worlds";

/// Ce qui appartient à un monde (voir les champs du même nom dans `Server`).
pub(super) struct Realm {
    dir: String,
    clients: HashMap<SocketAddr, Client>,
    npcs: HashMap<String, Npc>,
    mobs: HashMap<i64, mobs::Mob>,
    mob_store: mob_store::MobStore,
    mob_chunks: HashSet<(i32, i32)>,
    knots: HashMap<i64, (i32, i32, i32)>,
    spits: Vec<mobs::Spit>,
    mobs_dirty: bool,
    ground: HashMap<i64, GroundItem>,
    time_base: i64,
    bed_colors: HashMap<(i32, i32, i32), u8>,
    falling: HashSet<(i32, i32, i32)>,
    fires: HashSet<(i32, i32, i32)>,
    tnt: HashMap<i64, fire::Tnt>,
    shots: HashMap<i64, shots::Shot>,
    booms: Vec<(i64, (f32, f32, f32), f32, &'static str)>,
    decaying: HashMap<(i32, i32, i32), u64>,
    orbs: HashMap<i64, survival::Orb>,
    thorned: Vec<(i64, f32)>,
    weather: weather::Weather,
    bolts: Vec<i64>,
    boats: HashMap<i64, boats::Boat>,
    boats_dirty: bool,
    lids: HashSet<(i32, i32, i32)>,
    hooks: HashMap<SocketAddr, fishing::Hook>,
    pairs: HashMap<(i32, i32, i32), (i32, i32)>,
    frost: HashMap<(i32, i32, i32), u64>,
    releases: Vec<(u64, (i32, i32, i32), u32)>,
    open_doors: Vec<((i32, i32, i32), u64)>,
    world: crate::world::World,
    spawn: (i32, i32, i32),
    liquids: Option<crate::world::liquid::Liquids>,
    losses_told: usize,
    time_frozen: Option<i64>,
    do_weather: bool,
}

impl Realm {
    /// Un monde vide, à charger depuis ce dossier.
    fn empty(dir: String, tick: u64) -> Realm {
        Realm {
            mob_store: mob_store::MobStore::new(std::path::PathBuf::from(&dir).join("entities")),
            dir,
            clients: HashMap::new(),
            npcs: HashMap::new(),
            mobs: HashMap::new(),
            mob_chunks: HashSet::new(),
            knots: HashMap::new(),
            spits: Vec::new(),
            mobs_dirty: false,
            ground: HashMap::new(),
            time_base: 6000,
            bed_colors: HashMap::new(),
            falling: HashSet::new(),
            fires: HashSet::new(),
            tnt: HashMap::new(),
            shots: HashMap::new(),
            booms: Vec::new(),
            decaying: HashMap::new(),
            orbs: HashMap::new(),
            thorned: Vec::new(),
            weather: weather::Weather { sky: weather::Sky::Clear, until: tick + 12_000 },
            bolts: Vec::new(),
            boats: HashMap::new(),
            boats_dirty: false,
            lids: HashSet::new(),
            hooks: HashMap::new(),
            pairs: HashMap::new(),
            frost: HashMap::new(),
            releases: Vec::new(),
            open_doors: Vec::new(),
            world: crate::world::World::new(),
            spawn: (0, 64, 0),
            liquids: None,
            losses_told: 0,
            time_frozen: None,
            do_weather: true,
        }
    }
}

/// Un nom de monde : lettres, chiffres, tiret et souligné, 32 au plus (c'est aussi un nom de dossier).
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Server {
    /// Échange ce que le serveur tient en main avec ce monde rangé.
    fn swap_realm(&mut self, r: &mut Realm) {
        use std::mem::swap;
        swap(&mut self.cfg.world_dir, &mut r.dir);
        macro_rules! each {
            ($($field:ident),*) => { $( swap(&mut self.$field, &mut r.$field); )* };
        }
        each!(
            clients, npcs, mobs, mob_store, mob_chunks, knots, spits, mobs_dirty, ground, time_base, bed_colors, falling, fires, tnt, shots, booms, decaying, orbs, thorned, weather, bolts, boats, boats_dirty,
            lids, hooks, pairs, frost, releases, open_doors, world, spawn, liquids, losses_told, time_frozen, do_weather
        );
    }

    /// Entre dans un monde ouvert : la suite du code ne voit plus que lui. Faux s'il n'est pas ouvert.
    pub(super) fn enter(&mut self, name: &str) -> bool {
        if self.realm == name {
            return true;
        }
        let Some(mut target) = self.realms.remove(name) else { return false };
        // (ce que le serveur tient passe dans `target`, qui devient le rangement du monde quitté)
        self.swap_realm(&mut target);
        let left = std::mem::replace(&mut self.realm, name.to_string());
        self.realms.insert(left, target);
        true
    }

    /// Entre dans le monde du joueur de cette adresse (le monde principal pour une connexion nouvelle).
    pub(super) fn enter_home(&mut self, addr: &SocketAddr) {
        let home = self.homes.get(addr).cloned().unwrap_or_else(|| self.main_realm.clone());
        if !self.enter(&home) {
            let main = self.main_realm.clone();
            self.enter(&main);
        }
    }

    pub(super) fn enter_main(&mut self) {
        let main = self.main_realm.clone();
        self.enter(&main);
    }

    /// Les mondes ouverts : le principal d'abord, puis les autres par ordre alphabétique.
    pub(super) fn world_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.realms.keys().cloned().chain(std::iter::once(self.realm.clone())).filter(|n| *n != self.main_realm).collect();
        names.sort_unstable();
        names.insert(0, self.main_realm.clone());
        names
    }

    /// Fait la même chose dans chaque monde, puis revient au monde principal.
    pub(super) fn each_world(&mut self, mut f: impl FnMut(&mut Server)) {
        for name in self.world_names() {
            if self.enter(&name) {
                f(self);
            }
        }
        self.enter_main();
    }

    /// Un joueur, quel que soit son monde.
    pub(super) fn client_anywhere(&self, addr: &SocketAddr) -> Option<&Client> {
        self.clients.get(addr).or_else(|| self.realms.values().find_map(|r| r.clients.get(addr)))
    }

    pub(super) fn client_mut_anywhere(&mut self, addr: &SocketAddr) -> Option<&mut Client> {
        if self.clients.contains_key(addr) {
            return self.clients.get_mut(addr);
        }
        self.realms.values_mut().find_map(|r| r.clients.get_mut(addr))
    }

    /// Tous les joueurs de tous les mondes, avec le nom de leur monde.
    pub(super) fn everyone(&self) -> Vec<(&str, &SocketAddr, &Client)> {
        let here = self.clients.iter().map(|(a, c)| (self.realm.as_str(), a, c));
        let elsewhere = self.realms.iter().flat_map(|(name, r)| r.clients.iter().map(move |(a, c)| (name.as_str(), a, c)));
        here.chain(elsewhere).collect()
    }

    /// Un paquet pour tous les joueurs en jeu, dans tous les mondes (tchat, annonces).
    pub(super) fn broadcast_all(&mut self, id: u32, body: &[u8]) {
        let targets: Vec<SocketAddr> = self.everyone().into_iter().filter(|(_, _, c)| c.stage == Stage::InGame && c.protocol >= crate::versions::MIN_PROTOCOL).map(|(_, a, _)| *a).collect();
        for addr in targets {
            self.send(&addr, id, body);
        }
    }

    /// Le monde où se trouve ce joueur (pseudo), s'il est en ligne.
    pub(super) fn world_of_player(&self, player: &str) -> Option<String> {
        self.everyone().into_iter().find(|(_, _, c)| c.name.as_deref().map(|n| n.eq_ignore_ascii_case(player)).unwrap_or(false)).map(|(w, _, _)| w.to_string())
    }

    /// Entre dans le monde de ce joueur. Faux s'il n'est pas en ligne.
    pub(super) fn enter_player(&mut self, player: &str) -> bool {
        match self.world_of_player(player) {
            Some(w) => self.enter(&w),
            None => false,
        }
    }

    /// Où se trouve ce joueur en jeu : son monde et sa position.
    pub(super) fn locate_player(&self, player: &str) -> Option<(String, (f32, f32, f32))> {
        self.everyone().into_iter().find(|(_, _, c)| c.joined && c.name.as_deref().map(|n| n.eq_ignore_ascii_case(player)).unwrap_or(false)).map(|(w, _, c)| (w.to_string(), c.pos))
    }

    /// Le monde où se trouve cette créature, et on y entre.
    pub(super) fn enter_mob(&mut self, id: i64) {
        if self.mobs.contains_key(&id) {
            return;
        }
        if let Some(w) = self.realms.iter().find(|(_, r)| r.mobs.contains_key(&id)).map(|(n, _)| n.clone()) {
            self.enter(&w);
        }
    }

    /// Le monde où se trouve ce PNJ.
    pub(super) fn world_of_npc(&self, name: &str) -> Option<String> {
        let key = name.to_lowercase();
        if self.npcs.contains_key(&key) {
            return Some(self.realm.clone());
        }
        self.realms.iter().find(|(_, r)| r.npcs.contains_key(&key)).map(|(n, _)| n.clone())
    }

    /// Le terrain des autres mondes que le principal, pour les plugins.
    pub(super) fn other_views(&self) -> Vec<(String, crate::world::WorldView)> {
        let mut views: Vec<(String, crate::world::WorldView)> = self.realms.iter().filter(|(n, _)| **n != self.main_realm).map(|(n, r)| (n.clone(), r.world.view())).collect();
        if self.realm != self.main_realm {
            views.push((self.realm.clone(), self.world.view()));
        }
        views
    }

    /// Le terrain du monde principal, vu par les plugins (ils lisent et posent leurs blocs là).
    pub(super) fn main_view(&self) -> crate::world::WorldView {
        match self.realms.get(&self.main_realm) {
            Some(r) if self.realm != self.main_realm => r.world.view(),
            _ => self.world.view(),
        }
    }

    /// Ouvre un monde : celui du dossier `worlds/<nom>`, créé s'il n'existe pas (avec ce type
    /// de terrain et cette graine, sinon ceux de server.properties). Vrai s'il vient d'être créé.
    pub(super) fn load_realm(&mut self, name: &str, kind: Option<&str>, seed: Option<&str>) -> Result<bool, String> {
        if !valid_name(name) {
            return Err("un nom de monde se compose de lettres, de chiffres, de « - » et de « _ » (32 au plus)".into());
        }
        if self.world_names().iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return Err(format!("le monde « {} » est déjà ouvert", name));
        }
        if self.realms.len() + 1 >= MAX_WORLDS {
            return Err(format!("{} mondes sont déjà ouverts, c'est le maximum", MAX_WORLDS));
        }
        if let Some(k) = kind {
            if !matches!(k, "vanilla" | "flat" | "void" | "nether" | "end") && !k.starts_with("plugin:") {
                return Err("type de terrain inconnu : vanilla, flat, void, nether, end ou plugin:<nom>".into());
            }
        }
        self.unpack_worlds();
        let dir = self.cfg.root.join(WORLDS_DIR).join(name);
        let created = !dir.join("level.dat").exists() && !dir.join("level.nbt").exists();
        let back = self.realm.clone();
        self.realms.insert(name.to_string(), Realm::empty(dir.to_string_lossy().into_owned(), self.tick));
        self.enter(name);
        // (le type et la graine demandés ne valent que pour la création de ce monde)
        let (old_kind, old_seed) = (self.cfg.level_type.clone(), self.cfg.level_seed.clone());
        if let Some(k) = kind {
            self.cfg.level_type = k.to_string();
        }
        if let Some(s) = seed {
            self.cfg.level_seed = s.to_string();
        }
        self.load_world_files();
        self.cfg.level_type = old_kind;
        self.cfg.level_seed = old_seed;
        self.enter(&back);
        Ok(created)
    }

    /// Déballe les archives de mondes du jeu (`.mcworld`, `.zip`) déposées dans `worlds/` :
    /// chacune devient un dossier du même nom, et l'archive est gardée sous `.importe`.
    pub(super) fn unpack_worlds(&mut self) {
        let root = self.cfg.root.join(WORLDS_DIR);
        let Ok(list) = std::fs::read_dir(&root) else { return };
        for path in list.flatten().map(|e| e.path()) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !path.is_file() || (ext != "mcworld" && ext != "zip") {
                continue;
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let name: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(32).collect();
            let target = root.join(&name);
            if name.is_empty() || target.exists() {
                continue;
            }
            let done = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| crate::utils::pack_crypt::zip_entries(&bytes)).and_then(|entries| {
                // (certaines archives rangent le monde dans un sous-dossier : on part de level.dat)
                let prefix = entries.iter().map(|(n, _)| n.replace('\\', "/")).find(|n| n == "level.dat" || n.ends_with("/level.dat")).map(|n| n[..n.len() - "level.dat".len()].to_string()).ok_or("pas de level.dat dans l'archive")?;
                for (entry, bytes) in &entries {
                    let entry = entry.replace('\\', "/");
                    let Some(inside) = entry.strip_prefix(&prefix) else { continue };
                    if inside.is_empty() || inside.ends_with('/') || inside.split('/').any(|part| part == ".." || part.is_empty() || part.contains(':')) {
                        continue;
                    }
                    let file = target.join(inside);
                    if let Some(parent) = file.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
                }
                Ok(())
            });
            match done {
                Ok(()) => {
                    let _ = std::fs::rename(&path, path.with_extension(format!("{}.importe", ext)));
                    info!("Monde du jeu déballé : {} -> worlds/{}", path.display(), name);
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&target);
                    warn!("{} : archive de monde illisible ({})", path.display(), e);
                }
            }
        }
    }

    /// Ouvre au démarrage les mondes rangés dans `worlds/`.
    pub(super) fn autoload_realms(&mut self) {
        self.unpack_worlds();
        let Ok(list) = std::fs::read_dir(self.cfg.root.join(WORLDS_DIR)) else { return };
        let main = self.main_realm.clone();
        let mut names: Vec<String> = list.flatten().filter(|e| e.path().join("level.dat").exists() || e.path().join("level.nbt").exists()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| valid_name(n) && !n.eq_ignore_ascii_case(&main)).collect();
        names.sort_unstable();
        for name in names {
            match self.load_realm(&name, None, None) {
                Ok(_) => info!("Monde ouvert : {}", name),
                Err(e) => warn!("Monde « {} » non ouvert : {}", name, e),
            }
        }
    }

    /// Ferme un monde : ses joueurs passent dans le monde principal, il est enregistré, puis oublié.
    pub(super) fn unload_realm(&mut self, name: &str) -> Result<(), String> {
        let Some(found) = self.world_names().into_iter().find(|n| n.eq_ignore_ascii_case(name)) else { return Err(format!("le monde « {} » n'est pas ouvert", name)) };
        if found == self.main_realm {
            return Err("le monde principal ne se ferme pas".into());
        }
        self.enter(&found);
        let inside: Vec<SocketAddr> = self.clients.keys().copied().collect();
        let main = self.main_realm.clone();
        for addr in inside {
            self.enter(&found);
            if !self.move_player(&addr, &main, None) {
                // (pas encore en jeu : la connexion est simplement fermée)
                self.kick(&addr, "Ce monde ferme.");
                self.homes.remove(&addr);
                self.clients.remove(&addr);
            }
        }
        self.enter(&found);
        self.melt_frost();
        self.save_world();
        self.enter_main();
        self.realms.remove(&found);
        Ok(())
    }

    /// Enregistre tous les mondes.
    pub(super) fn save_worlds(&mut self) {
        let back = self.realm.clone();
        for name in self.world_names() {
            if self.enter(&name) {
                self.save_world();
            }
        }
        self.enter(&back);
    }

    /// Fait passer un joueur en jeu dans un autre monde ouvert, à cet endroit (hauteur des
    /// yeux) ou au point d'apparition de ce monde. Le serveur doit être dans le monde du
    /// joueur ; il est dans le monde d'arrivée au retour. Faux si rien n'a changé.
    pub(super) fn move_player(&mut self, addr: &SocketAddr, to: &str, at: Option<(f32, f32, f32)>) -> bool {
        let Some(to) = self.world_names().into_iter().find(|n| n.eq_ignore_ascii_case(to)) else { return false };
        if !self.clients.get(addr).map(|c| c.joined && c.stage == Stage::InGame).unwrap_or(false) {
            return false;
        }
        if to == self.realm {
            if let Some(pos) = at {
                self.teleport(addr, pos);
            }
            return true;
        }
        // 1. Il quitte ce monde : monture, écran ouvert, ligne de pêche, lit
        self.dismount(addr);
        self.force_close(addr);
        self.drop_hook(addr);
        let old_sky = self.weather.sky;
        let old_dimension = self.world.generator().kind.dimension();
        let Some((id, from)) = self.clients.get(addr).map(|c| (c.id, c.pos)) else { return false };
        // Les autres ne le voient plus
        let watchers: Vec<SocketAddr> = self.clients.iter_mut().filter(|(a, _)| *a != addr).filter_map(|(a, c)| c.visible.remove(&id).then_some(*a)).collect();
        let gone = bedrock::remove_actor(id);
        for w in watchers {
            self.send(&w, bedrock::ID_REMOVE_ACTOR, &gone);
        }
        // Et lui ne voit plus rien de ce monde : tout ce qui s'y trouve près de lui est retiré de son écran
        let near = |p: (f32, f32, f32)| (p.0 - from.0).powi(2) + (p.2 - from.2).powi(2) < 160.0 * 160.0;
        let mut seen: Vec<i64> = self.clients.get(addr).map(|c| c.visible.iter().chain(c.mobs_seen.iter()).chain(c.items_seen.iter()).copied().collect()).unwrap_or_default();
        seen.extend(self.orbs.keys().chain(self.shots.keys()).chain(self.tnt.keys()).copied());
        seen.extend(self.hooks.values().map(|h| h.id));
        seen.extend(self.clients.values().filter(|c| c.id != id && near(c.pos)).map(|c| c.id));
        seen.sort_unstable();
        seen.dedup();
        for entity in seen.into_iter().take(4096) {
            self.send(addr, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(entity));
        }
        let Some(mut c) = self.clients.remove(addr) else { return false };
        // 2. Il entre dans l'autre
        self.homes.insert(*addr, to.clone());
        self.enter(&to);
        let pos = at.unwrap_or_else(|| self.spawn_point());
        let center = ((pos.0.floor() as i32) >> 4, (pos.2.floor() as i32) >> 4);
        let radius = c.radius.max(2);
        c.pos = pos;
        c.center = center;
        c.riding = None;
        c.sleeping = None;
        c.eating = None;
        c.mining = None;
        c.drawing = None;
        c.fall_peak = None;
        c.burn_until = 0;
        c.moved_tick = self.tick + 1;
        c.move_budget = SPEED_CREATIVE * 2.0;
        c.move_budget_at = Instant::now();
        for set in [&mut c.visible, &mut c.items_seen, &mut c.mobs_seen] {
            set.clear();
        }
        for set in [&mut c.beds_seen, &mut c.pairs_seen, &mut c.pairs_stale] {
            set.clear();
        }
        // Le terrain : tout est à renvoyer. Les chunks de l'ancien monde encore en cours
        // d'encodage portent un numéro d'avant ce saut : ils seront ignorés à leur retour.
        c.loaded.clear();
        c.delivered.clear();
        c.chunk_ready.clear();
        c.chunk_next = (c.chunk_next | 0xffff_ffff) + 1;
        c.chunk_sent = c.chunk_next;
        let coords = Self::chunks_around(center, radius);
        for (x, z) in &coords {
            self.world.prepare(*x, *z);
        }
        c.queue = coords.into_iter().collect();
        let packet = bedrock::teleport_player(c.protocol, c.id, pos, c.pitch, c.yaw, c.head_yaw);
        // D'une dimension à l'autre (surface, Nether, End) : le jeu est prévenu d'abord, il
        // affiche son écran de chargement et attend le terrain de la nouvelle dimension. Le
        // signal « apparu » qui suit les chunks le referme.
        let dimension = self.world.generator().kind.dimension();
        if dimension != old_dimension {
            // (l'écran de chargement du jeu est inévitable d'une dimension à l'autre : il est
            // refermé dès que les neuf chunks autour du joueur sont partis, pas à la fin)
            c.spawn_pending = true;
            c.spawn_quick = c.chunk_next + 9;
        }
        self.clients.insert(*addr, c);
        if dimension != old_dimension {
            static SCREEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let screen = SCREEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.send(addr, bedrock::ID_CHANGE_DIMENSION, &bedrock::change_dimension(dimension, pos, screen));
        }
        self.send(addr, bedrock::ID_MOVE_PLAYER, &packet);
        self.send(addr, bedrock::ID_CHUNK_PUBLISHER_UPDATE, &bedrock::chunk_publisher_update(pos.0.floor() as i32, pos.1.floor() as i32, pos.2.floor() as i32, (radius as u32 + 1) * 16));
        // L'heure et le temps qu'il fait ici
        self.send(addr, bedrock::ID_SET_TIME, &bedrock::set_time(self.world_time().rem_euclid(i32::MAX as i64) as i32));
        for packet in weather::packets(old_sky, self.weather.sky) {
            self.send(addr, bedrock::ID_LEVEL_EVENT, &packet);
        }
        self.extinguish(addr);
        true
    }

    /// La commande `/world` : `list`, `load <nom> [type] [graine]`, `unload <nom>`,
    /// `tp <nom> [joueur]`. Rend le texte de la réponse.
    pub(super) fn world_command(&mut self, sender: Option<SocketAddr>, args: &[&str]) -> String {
        let usage = "§cUsage : /world list | /world load <nom> [vanilla|flat|void|nether|end] [graine] | /world unload <nom> | /world tp <nom> [joueur]";
        match args.first().copied().unwrap_or("") {
            "list" | "" => {
                let counts: HashMap<String, usize> = self.everyone().into_iter().filter(|(_, _, c)| c.joined).fold(HashMap::new(), |mut m, (w, _, _)| {
                    *m.entry(w.to_string()).or_insert(0) += 1;
                    m
                });
                let lines: Vec<String> = self.world_names().into_iter().map(|n| format!("{}{} ({} joueur(s))", n, if n == self.main_realm { " [principal]" } else { "" }, counts.get(&n).copied().unwrap_or(0))).collect();
                format!("Mondes ouverts : {}", lines.join(", "))
            }
            "load" | "create" => {
                let Some(name) = args.get(1) else { return usage.into() };
                match self.load_realm(name, args.get(2).copied(), args.get(3).copied()) {
                    Ok(true) => format!("Monde « {} » créé et ouvert", name),
                    Ok(false) => format!("Monde « {} » ouvert", name),
                    Err(e) => format!("§c{}", e),
                }
            }
            "unload" => {
                let Some(name) = args.get(1) else { return usage.into() };
                match self.unload_realm(name) {
                    Ok(()) => format!("Monde « {} » enregistré et fermé", name),
                    Err(e) => format!("§c{}", e),
                }
            }
            "tp" => {
                let Some(name) = args.get(1) else { return usage.into() };
                // Le joueur nommé, sinon celui qui tape la commande
                let who = match (args.get(2), sender) {
                    (Some(player), _) => {
                        if !self.enter_player(player) {
                            return format!("§cJoueur introuvable : {}", player);
                        }
                        self.addr_of(player)
                    }
                    (None, Some(addr)) => {
                        self.enter_home(&addr);
                        Some(addr)
                    }
                    (None, None) => return "§cDepuis la console, précisez le joueur : /world tp <nom> <joueur>".into(),
                };
                let moved = who.map(|addr| self.move_player(&addr, name, None)).unwrap_or(false);
                if moved {
                    format!("Téléporté dans le monde « {} »", name)
                } else {
                    format!("§cLe monde « {} » n'est pas ouvert (voir /world list)", name)
                }
            }
            _ => usage.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_answers_share_one_look() {
        assert_eq!(Server::styled("Bloc placé"), "§a» §r§fBloc placé");
        assert_eq!(Server::styled("§cJoueur introuvable : Bob"), "§c» §r§cJoueur introuvable : Bob");
        assert_eq!(Server::styled("§cUsage : /world list"), "§e» §r§7Usage : §f/world list");
        assert_eq!(Server::styled("§7Point de réapparition défini"), "§7» §r§7Point de réapparition défini");
        // Déjà mis en forme : laissé tel quel
        assert_eq!(Server::styled("§a» §r§fBloc placé"), "§a» §r§fBloc placé");
    }

    #[test]
    fn world_names_are_safe_folder_names() {
        assert!(valid_name("lobby") && valid_name("Mini-Jeu_2") && valid_name("a"));
        assert!(!valid_name("") && !valid_name("../secret") && !valid_name("mon monde") && !valid_name("a/b") && !valid_name(&"x".repeat(33)) && !valid_name("monde.old"));
    }
}
