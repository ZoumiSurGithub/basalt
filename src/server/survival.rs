//! Survie du joueur : la faim, l'expérience et le sommeil.
//!
//! - **Faim** : courir, sauter, nager, casser des blocs et encaisser des coups fatiguent ; la
//!   fatigue entame d'abord la satiété (invisible), puis la barre de faim. Barre presque
//!   pleine, la vie remonte ; vide, le joueur dépérit jusqu'à un demi-cœur.
//! - **Expérience** : créatures tuées, minerais cassés, échanges et naissances en donnent ;
//!   la mort la fait perdre. Elle est créditée directement (pas d'orbes à ramasser).
//! - **Sommeil** : un clic sur un lit fixe le point de réapparition ; la nuit, le joueur s'y
//!   couche, et la nuit passe quand tous les joueurs dorment.

use std::net::SocketAddr;

use super::*;

/// Fatigue qui coûte un point de satiété ou de faim.
const EXHAUSTION_STEP: f32 = 4.0;
/// Marge autour de la durée d'un repas (donnée par l'aliment, voir `item/consumable.rs`) : un
/// clic qui arrive plus de EAT_EARLY ticks avant la fin est ignoré ; sans clic de fin, le
/// serveur conclut le repas EAT_LATE ticks après.
const EAT_EARLY: u64 = 4;
const EAT_LATE: u64 = 8;
/// Heures de la nuit où l'on peut dormir.
const NIGHT: std::ops::RangeInclusive<i64> = 12_542..=23_459;
/// Temps passé couché avant que la nuit passe (5 secondes).
const SLEEP_TICKS: u64 = 100;

/// Une orbe d'expérience au sol.
pub(super) struct Orb {
    pos: (f32, f32, f32),
    value: i32,
    born: u64,
    /// Joueurs à qui elle a été montrée
    seen: Vec<SocketAddr>,
}

/// Orbes au sol au plus (les plus anciennes disparaissent), durée de vie (5 minutes),
/// distance à laquelle un joueur les attire et distance de ramassage.
const MAX_ORBS: usize = 300;
const ORB_LIFE: u64 = 6000;
const ORB_PULL: f32 = 7.0;
const ORB_TAKE: f32 = 1.1;

/// Plantes qui s'étoffent quand on en repose une au même endroit (jusqu'à quatre).
const GROWING: [&str; 3] = ["minecraft:pink_petals", "minecraft:wildflowers", "minecraft:leaf_litter"];

/// Points d'expérience pour passer du niveau `level` au suivant (règle du jeu).
pub(super) fn xp_to_next(level: i32) -> i32 {
    match level {
        l if l < 16 => 2 * l + 7,
        l if l < 31 => 5 * l - 38,
        l => 9 * l - 158,
    }
}

/// Ajoute des points à (niveau, progression de 0 à 1) et renvoie le nouvel état.
pub(super) fn add_xp(mut level: i32, progress: f32, points: i32) -> (i32, f32) {
    let mut have = (progress.clamp(0.0, 1.0) * xp_to_next(level) as f32).round() as i64 + points.max(0) as i64;
    while level < crate::entity::MAX_XP_LEVEL && have >= xp_to_next(level) as i64 {
        have -= xp_to_next(level) as i64;
        level += 1;
    }
    (level, (have as f32 / xp_to_next(level) as f32).clamp(0.0, 0.999))
}

/// Expérience que rend un minerai cassé à la main : (minimum, maximum).
pub(super) fn ore_xp(block: &str) -> Option<(u32, u32)> {
    let short = block.trim_start_matches("minecraft:").trim_start_matches("deepslate_").trim_start_matches("lit_");
    Some(match short {
        "coal_ore" => (0, 2),
        "diamond_ore" | "emerald_ore" => (3, 7),
        "lapis_ore" | "quartz_ore" | "nether_quartz_ore" => (2, 5),
        "redstone_ore" => (1, 5),
        "nether_gold_ore" => (0, 1),
        "sculk" => (1, 1),
        "spawner" | "mob_spawner" => (15, 43),
        _ => return None,
    })
}

impl Server {
    /// Fatigue ajoutée à un joueur (sans effet hors survie).
    pub(super) fn exhaust(&mut self, addr: &SocketAddr, amount: f32) {
        if let Some(c) = self.clients.get_mut(addr) {
            if c.mode.takes_damage() && c.dead.is_none() {
                c.exhaustion += amount;
            }
        }
    }

    /// Un tick de faim pour tous les joueurs : la fatigue entame la satiété puis la faim ;
    /// la vie remonte le ventre plein et baisse le ventre vide.
    pub(super) fn hunger_tick(&mut self) {
        let mut changed: Vec<SocketAddr> = Vec::new();
        let mut starving: Vec<SocketAddr> = Vec::new();
        // Repas dont le clic de fin n'est pas venu (dernier aliment de la pile : le jeu n'a
        // plus rien à annoncer) : conclu ici, un peu après sa durée normale
        let tick = self.tick;
        let eaters: Vec<(SocketAddr, u64)> = self.clients.iter().filter_map(|(a, c)| c.eating.map(|t| (*a, tick.saturating_sub(t)))).collect();
        let late: Vec<SocketAddr> = eaters.into_iter().filter(|(a, since)| *since >= self.held_food(a).map(|(_, f)| f.eat_ticks as u64).unwrap_or(crate::item::EAT_TICKS as u64) + EAT_LATE).map(|(a, _)| a).collect();
        for a in late {
            self.finish_meal(&a);
        }
        // Écouteurs : la fatigue qui s'apprête à coûter un point peut être annulée (pas de faim)
        let mut spared: HashSet<SocketAddr> = HashSet::new();
        if self.plugins.listens("PlayerExhaustEvent") {
            let tired: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage() && c.exhaustion >= EXHAUSTION_STEP).map(|(a, _)| *a).collect();
            for a in tired {
                let out = self.fire("PlayerExhaustEvent", vec![("player", self.name_json(&a)), ("amount", Json::Num(EXHAUSTION_STEP as f64))]);
                if cancelled(&out) {
                    spared.insert(a);
                }
            }
        }
        for (addr, c) in self.clients.iter_mut() {
            if !(c.joined && c.dead.is_none() && c.mode.takes_damage()) {
                continue;
            }
            let Some(d) = c.data.as_mut() else { continue };
            let mut dirty = false;
            if spared.contains(addr) {
                c.exhaustion = 0.0;
            }
            while c.exhaustion >= EXHAUSTION_STEP {
                c.exhaustion -= EXHAUSTION_STEP;
                if c.saturation > 0.0 {
                    c.saturation = (c.saturation - 1.0).max(0.0);
                } else if d.food > 0.0 {
                    d.food = (d.food - 1.0).max(0.0);
                    dirty = true;
                }
            }
            let hurt = d.health < d.max_health;
            c.food_timer += 1;
            if hurt && d.food >= 20.0 && c.saturation > 0.0 {
                // Ventre plein et rassasié : la satiété se change en vie toutes les
                // demi-secondes (6 points de satiété pour un demi-cœur, pas davantage que
                // ce qu'il en reste : la barre de faim, elle, n'est pas entamée)
                if c.food_timer >= 10 {
                    c.food_timer = 0;
                    let spent = c.saturation.min(6.0);
                    d.heal(spent / 6.0);
                    c.saturation -= spent;
                    dirty = true;
                }
            } else if hurt && d.food >= 18.0 {
                if c.food_timer >= 80 {
                    c.food_timer = 0;
                    d.heal(1.0);
                    c.exhaustion += 6.0;
                    dirty = true;
                }
            } else if d.food <= 0.0 {
                if c.food_timer >= 80 {
                    c.food_timer = 0;
                    // Difficulté normale : la faim laisse un demi-cœur
                    if d.health > 1.0 {
                        starving.push(*addr);
                    }
                }
            } else {
                c.food_timer = 0;
            }
            if dirty {
                changed.push(*addr);
            }
        }
        for a in changed {
            self.sync_attributes(&a);
        }
        for a in starving {
            self.hurt(&a, 1.0, Cause::Hunger);
        }
    }

    /// Clic dans le vide avec un aliment en main. Relevé avec le jeu 1.26.65 : il envoie ce
    /// clic au début du repas, puis de nouveau toutes les 32 ticks tant que le bouton reste
    /// tenu — chaque clic clôt donc le repas précédent et en commence un autre — et annonce
    /// le bouton relâché (voir `stop_eating`). Faux si l'objet tenu ne se mange pas.
    pub(super) fn eat_in_air(&mut self, addr: &SocketAddr) -> bool {
        let tick = self.tick;
        let debug = self.cfg.debug;
        let Some((name, food)) = self.held_food(addr) else { return false };
        let least = (food.eat_ticks as u64).saturating_sub(EAT_EARLY);
        let since = self.clients.get(addr).and_then(|c| c.eating).map(|t| tick.saturating_sub(t));
        match since {
            // Clic répété trop tôt : le repas en cours continue
            Some(t) if t < least => return true,
            // Le repas précédent est fini
            Some(_) => self.finish_meal(addr),
            None => {}
        }
        // … et un autre commence, s'il reste de quoi manger et de l'appétit
        let again = self.held_food(addr).map(|(_, food)| food.always_edible).and_then(|always| {
            let c = self.clients.get(addr)?;
            let hungry = c.data.as_ref().map(|d| d.food < 20.0).unwrap_or(false);
            (!c.mode.takes_damage() || hungry || always).then_some(())
        });
        if let Some(c) = self.clients.get_mut(addr) {
            c.eating = again.map(|_| tick);
        }
        if debug {
            info!("{} : repas de {} — clic précédent {:?} ticks plus tôt, nouveau repas : {}", addr, name, since, again.is_some());
        }
        true
    }

    /// L'aliment tenu en main par un joueur en vie : son nom et ce que le gabarit `Food` en
    /// dit (ce qu'il nourrit, sa durée, ce qu'il laisse). None si l'objet ne se mange pas.
    fn held_food(&self, addr: &SocketAddr) -> Option<(String, crate::item::FoodValues)> {
        let c = self.clients.get(addr).filter(|c| c.joined && c.dead.is_none())?;
        let held = c.inv.held();
        if held.is_empty() {
            return None;
        }
        let name = self.data.as_ref()?.item_by_id(held.id)?.name.clone();
        let food = VanillaItems::get(&name).food()?;
        Some((name, food))
    }

    /// Bouton relâché, ou autre objet pris en main : le repas en cours est abandonné.
    pub(super) fn stop_eating(&mut self, addr: &SocketAddr) {
        let tick = self.tick;
        // Relâché alors que le repas était fini (dernier aliment de la pile : le jeu n'envoie
        // pas d'autre clic) : il compte
        let least = self.held_food(addr).map(|(_, f)| (f.eat_ticks as u64).saturating_sub(EAT_EARLY)).unwrap_or(crate::item::EAT_TICKS as u64);
        if self.clients.get(addr).and_then(|c| c.eating).map(|t| tick.saturating_sub(t) >= least).unwrap_or(false) {
            return self.finish_meal(addr);
        }
        if let Some(c) = self.clients.get_mut(addr) {
            c.eating = None;
        }
    }

    /// Le repas en cours est mené à son terme : l'aliment est mangé.
    fn finish_meal(&mut self, addr: &SocketAddr) {
        if let Some(c) = self.clients.get_mut(addr) {
            c.eating = None;
        }
        let Some((name, meal)) = self.held_food(addr) else { return };
        let (food, saturation) = (meal.hunger as f32, meal.saturation);
        // Écouteurs : le repas peut être refusé (le jeu, qui l'a déjà mimé, retrouve son état)
        if self.plugins.listens("PlayerItemConsumeEvent") {
            let item = self.clients.get(addr).map(|c| c.inv.held()).map(|h| self.item_json(h.id, 1)).unwrap_or(Json::Null);
            let out = self.fire("PlayerItemConsumeEvent", vec![("player", self.name_json(addr)), ("item", item)]);
            if cancelled(&out) {
                self.send_attributes(addr, false);
                self.send_inventory(addr);
                return;
            }
        }
        let Some(c) = self.clients.get_mut(addr) else { return };
        let survival = c.mode.takes_damage();
        if survival {
            c.inv.consume_held();
            c.inv_touched = true;
        }
        if let Some(d) = c.data.as_mut() {
            d.food = (d.food + food).min(20.0);
            c.saturation = (c.saturation + saturation).min(d.food);
        }
        if survival {
            if let Some(id) = meal.leftover.and_then(|n| self.item_id(n)) {
                if let Some(c) = self.clients.get_mut(addr) {
                    c.inv.add(id, 1);
                }
            }
        }
        if self.cfg.debug {
            info!("{} : a mangé {}", addr, name);
        }
        self.sync_attributes(addr);
        self.send_inventory(addr);
        self.show_held(addr);
        self.burp(addr);
    }

    /// Le jeu annonce, plusieurs fois pendant le repas, que le joueur mâche : renvoyé à lui
    /// et à ceux qui le voient, c'est ce qui fait le bruit et les miettes.
    pub(super) fn relay_eating(&mut self, addr: &SocketAddr, body: &[u8]) {
        let mut pos = 0;
        let (Some(_), Some(event)) = (bedrock::read_varu64(body, &mut pos), body.get(pos).copied()) else { return };
        pos += 1;
        let Some(data) = bedrock::read_vari32(body, &mut pos) else { return };
        if event != bedrock::EVENT_EATING {
            return;
        }
        let id = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() && c.eating.is_some() => c.id,
            _ => return,
        };
        let packet = bedrock::actor_event(id, bedrock::EVENT_EATING, data);
        let to: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| *a == addr || c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for a in to {
            self.send(&a, bedrock::ID_ACTOR_EVENT, &packet);
        }
    }

    /// Bruit du bloc que le joueur frappe en le minant : le jeu l'annonce au serveur, qui le
    /// rend à lui et à ceux qui sont autour (sans cela, personne ne l'entend).
    pub(super) fn relay_hit_sound(&mut self, addr: &SocketAddr, body: &[u8]) {
        // [longueur du nom][nom]… : seul « hit » est relayé, tel quel
        if body.len() > 64 || body.get(..4) != Some(&[3, b'h', b'i', b't'][..]) {
            return;
        }
        let at = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() && c.mining.is_some() => c.pos,
            _ => return,
        };
        self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, body);
    }

    /// Le rot de fin de repas, pour le joueur et ceux qui le voient.
    fn burp(&mut self, addr: &SocketAddr) {
        let Some((id, at)) = self.clients.get(addr).map(|c| (c.id, c.pos)) else { return };
        let packet = bedrock::level_sound(bedrock::SOUND_BURP, at, "", false, -1);
        let to: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| *a == addr || c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for a in to {
            self.send(&a, bedrock::ID_LEVEL_SOUND_EVENT, &packet);
        }
    }

    /// Donne des points d'expérience à un joueur (hors créatif et spectateur).
    pub(super) fn give_xp(&mut self, addr: &SocketAddr, points: i32) {
        if points <= 0 {
            return;
        }
        // Écouteurs : gain annulé ou modifié
        let mut points = points;
        if self.plugins.listens("PlayerExperienceChangeEvent") {
            let out = self.fire("PlayerExperienceChangeEvent", vec![("player", self.name_json(addr)), ("amount", Json::Num(points as f64))]);
            if cancelled(&out) {
                return;
            }
            if let Some(n) = out.as_ref().and_then(|v| v.get("amount")).and_then(|n| n.as_f64()) {
                points = n.clamp(0.0, 1.0e6) as i32;
            }
            if points <= 0 {
                return;
            }
        }
        let (at, levelled) = match self.clients.get_mut(addr) {
            Some(c) if c.joined && c.mode.takes_damage() => match c.data.as_mut() {
                Some(d) => {
                    let before = d.xp_level;
                    // (le surplus au-delà d'un niveau est reporté sur le suivant)
                    let (level, progress) = add_xp(d.xp_level, d.xp_progress, points);
                    d.xp_level = level;
                    d.xp_progress = progress;
                    (c.pos, (level > before).then_some(level))
                }
                None => return,
            },
            _ => return,
        };
        self.sync_attributes(addr);
        // Le tintement de l'orbe, et la fanfare d'un niveau gagné
        self.send(addr, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_SOUND_ORB, at, 0));
        if let Some(level) = levelled {
            let pitch = 0x1000_0000 * (level.min(30) / 5);
            self.send(addr, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound_with(bedrock::SOUND_LEVELUP, at, pitch, "", false, -1));
        }
    }

    /// Lâche une orbe d'expérience à cet endroit : le joueur qui s'en approche l'attire et
    /// la ramasse.
    pub(super) fn drop_xp(&mut self, at: (f32, f32, f32), value: i32) {
        if value <= 0 {
            return;
        }
        if self.orbs.len() >= MAX_ORBS {
            if let Some(oldest) = self.orbs.iter().min_by_key(|(_, o)| o.born).map(|(id, _)| *id) {
                self.remove_orb(oldest);
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        let packet = bedrock::add_actor(id, "minecraft:xp_orb", at, (0.0, 0.2, 0.0), 0.0, (1.0, 1.0), None, None, &[(bedrock::META_EXPERIENCE, bedrock::Meta::Int(value))]);
        let seen: Vec<SocketAddr> = self
            .clients
            .iter()
            .filter(|(_, c)| c.joined && c.stage == Stage::InGame && (c.pos.0 - at.0).powi(2) + (c.pos.2 - at.2).powi(2) < 48.0 * 48.0)
            .map(|(a, _)| *a)
            .collect();
        for a in &seen {
            self.send(a, bedrock::ID_ADD_ACTOR, &packet);
        }
        self.orbs.insert(id, Orb { pos: at, value, born: self.tick, seen });
    }

    /// Retire une orbe du monde et de l'écran de ceux qui la voyaient.
    fn remove_orb(&mut self, id: i64) {
        if let Some(orb) = self.orbs.remove(&id) {
            let packet = bedrock::remove_actor(id);
            for a in orb.seen {
                if self.clients.contains_key(&a) {
                    self.send(&a, bedrock::ID_REMOVE_ACTOR, &packet);
                }
            }
        }
    }

    /// Orbes d'expérience (un tick sur deux) : elles filent vers le joueur le plus proche,
    /// qui les ramasse en les touchant, et disparaissent au bout de cinq minutes.
    pub(super) fn orb_tick(&mut self) {
        if self.orbs.is_empty() {
            return;
        }
        let tick = self.tick;
        let players: Vec<(SocketAddr, (f32, f32, f32))> = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|(a, c)| (*a, (c.pos.0, c.pos.1 - 0.9, c.pos.2))).collect();
        let mut gone: Vec<i64> = Vec::new();
        let mut taken: Vec<(i64, SocketAddr, i32)> = Vec::new();
        let mut moved: Vec<(i64, (f32, f32, f32), Vec<SocketAddr>)> = Vec::new();
        for (id, orb) in self.orbs.iter_mut() {
            let age = tick.saturating_sub(orb.born);
            if age > ORB_LIFE {
                gone.push(*id);
                continue;
            }
            // Une demi-seconde sur place, le temps de la voir tomber
            if age < 10 {
                continue;
            }
            let dist = |p: &(f32, f32, f32)| ((p.0 - orb.pos.0).powi(2) + (p.1 - orb.pos.1).powi(2) + (p.2 - orb.pos.2).powi(2)).sqrt();
            let Some((who, target)) = players.iter().min_by(|a, b| dist(&a.1).total_cmp(&dist(&b.1))) else { continue };
            let d = dist(target);
            if d <= ORB_TAKE {
                taken.push((*id, *who, orb.value));
            } else if d <= ORB_PULL {
                // De plus en plus vite en approchant
                let step = (0.25 + (ORB_PULL - d) * 0.08).min(d);
                orb.pos = (orb.pos.0 + (target.0 - orb.pos.0) / d * step, orb.pos.1 + (target.1 - orb.pos.1) / d * step, orb.pos.2 + (target.2 - orb.pos.2) / d * step);
                moved.push((*id, orb.pos, orb.seen.clone()));
            }
        }
        for (id, pos, seen) in moved {
            let packet = bedrock::move_actor_absolute(id, pos, 0.0, 0.0, 0.0, false);
            for a in seen {
                self.send(&a, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
            }
        }
        for id in gone {
            self.remove_orb(id);
        }
        for (id, who, value) in taken {
            self.remove_orb(id);
            self.give_xp(&who, value);
        }
    }

    /// Gestes de minage annoncés par le jeu : les fissures et les éclats du bloc, pour le
    /// joueur et ceux qui sont autour. La durée des fissures suit la dureté du bloc (sans
    /// tenir compte de l'outil : le jeu, lui, casse le bloc au bon moment).
    pub(super) fn on_mining(&mut self, addr: &SocketAddr, body: &[u8]) {
        let Some(actions) = bedrock::read_block_actions(body) else { return };
        let (me, survival) = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() && c.mode.can_build() => (c.pos, c.mode.takes_damage()),
            _ => return,
        };
        let tick = self.tick;
        for (action, (x, y, z), face) in actions {
            let event = match action {
                0 => {
                    if !survival || (x as f32 - me.0).abs() > 8.0 || (y as f32 - me.1).abs() > 8.0 || (z as f32 - me.2).abs() > 8.0 {
                        continue;
                    }
                    let state = self.world.get(x, y, z);
                    // Durée d'après la dureté du bloc et l'outil tenu (le bon outil va plus
                    // vite ; sans l'outil exigé, bien plus lentement)
                    let held = self.clients.get(addr).map(|c| c.inv.held()).filter(|h| !h.is_empty());
                    let tool = match held.and_then(|h| self.data.as_ref().and_then(|d| d.item_by_id(h.id))) {
                        Some(def) => VanillaItems::get(&def.name),
                        None => VanillaItems::hand(),
                    };
                    let seconds = self.block_at_state(state).map(|b| b.break_time(tool.as_ref())).unwrap_or(1.5);
                    if !seconds.is_finite() || seconds <= 0.0 {
                        continue;
                    }
                    let ticks = (seconds * 20.0).ceil().clamp(1.0, 6000.0);
                    // Un autre bloc était en cours : ses fissures s'effacent
                    let before = self.clients.get_mut(addr).and_then(|c| c.mining.replace((x, y, z)));
                    if let Some(p) = before.filter(|p| *p != (x, y, z)) {
                        let at = (p.0 as f32, p.1 as f32, p.2 as f32);
                        self.send_near(at, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_STOP_BREAK, at, 0));
                    }
                    Some((bedrock::LEVEL_EVENT_START_BREAK, (x, y, z), (65535.0 / ticks) as i32))
                }
                18 => {
                    // Éclats : quatre fois par seconde au plus
                    let due = match self.clients.get_mut(addr) {
                        Some(c) if c.mining == Some((x, y, z)) && tick.saturating_sub(c.punched_at) >= 5 => {
                            c.punched_at = tick;
                            true
                        }
                        _ => false,
                    };
                    if !due {
                        continue;
                    }
                    let state = self.world.get(x, y, z);
                    Some((bedrock::LEVEL_EVENT_PUNCH, (x, y, z), (state as i32) | ((face & 7) << 24)))
                }
                1 | 2 => self.clients.get_mut(addr).and_then(|c| c.mining.take()).map(|p| (bedrock::LEVEL_EVENT_STOP_BREAK, p, 0)),
                _ => None,
            };
            if let Some((event, (x, y, z), data)) = event {
                let at = if event == bedrock::LEVEL_EVENT_PUNCH { (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) } else { (x as f32, y as f32, z as f32) };
                self.send_near(at, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(event, at, data));
            }
        }
    }

    /// Envoie un paquet aux joueurs à moins de 32 blocs d'un point.
    pub(super) fn send_near(&mut self, at: (f32, f32, f32), id: u32, body: &[u8]) {
        let to: Vec<SocketAddr> = self
            .clients
            .iter()
            .filter(|(_, c)| c.joined && c.stage == Stage::InGame && (c.pos.0 - at.0).powi(2) + (c.pos.1 - at.1).powi(2) + (c.pos.2 - at.2).powi(2) < 32.0 * 32.0)
            .map(|(a, _)| *a)
            .collect();
        for a in to {
            self.send(&a, id, body);
        }
    }

    /// Pétales, fleurs sauvages, feuilles mortes : en reposer sur une touffe l'étoffe (jusqu'à
    /// quatre) au lieu de poser un second bloc. Vrai si le clic a été traité ici.
    pub(super) fn grow_patch(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        let held = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() && c.mode.can_build() => c.inv.held(),
            _ => return false,
        };
        if held.is_empty() {
            return false;
        }
        let state = self.world.get(pos.0, pos.1, pos.2);
        let Some(d) = self.data.clone() else { return false };
        let name = match d.block_name(state) {
            Some(n) if GROWING.contains(&n) => n,
            _ => return false,
        };
        if self.block_for(held.id).and_then(|b| d.block_name(b)) != Some(name) {
            return false;
        }
        let grown = match d.prop(state, "growth") {
            Some(Prop::Int(g)) if g < 3 => d.with_props(state, &[("growth", Prop::Int(g + 1))]),
            _ => None,
        };
        match grown {
            Some(next) if self.world.set(pos.0, pos.1, pos.2, next) => {
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(pos.0, pos.1, pos.2, next), None);
                if let Some(c) = self.clients.get_mut(addr) {
                    if c.mode.takes_damage() {
                        c.inv.consume_held();
                        c.inv_touched = true;
                    }
                }
                self.send_near((pos.0 as f32 + 0.5, pos.1 as f32, pos.2 as f32 + 0.5), bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound_with(bedrock::SOUND_PLACE, (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5), next as i32, "", false, -1));
            }
            // Touffe déjà complète : rien ne change, le jeu retrouve le bloc tel qu'il est
            _ => self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(pos.0, pos.1, pos.2, state)),
        }
        self.send_inventory(addr);
        true
    }

    /// L'objet tenu s'use d'un usage (miner ou frapper), d'après ce que son gabarit en dit
    /// (voir `item/durable.rs`). Usé jusqu'au bout, il casse.
    pub(super) fn wear_held(&mut self, addr: &SocketAddr, usage: crate::item::Usage) {
        let Some(d) = self.data.clone() else { return };
        let broke = match self.clients.get_mut(addr) {
            Some(c) if c.joined && c.mode.takes_damage() => {
                let slot = (c.inv.selected as usize).min(8);
                let mut held = c.inv.main.slots[slot];
                let Some(def) = d.item_by_id(held.id).filter(|_| !held.is_empty()) else { return };
                let item = VanillaItems::get(&def.name);
                let before = held;
                let broke = crate::item::apply_wear(&mut held, item.as_ref(), usage);
                if held == before {
                    return; // cet objet ne s'use pas
                }
                c.inv.main.slots[slot] = held;
                c.inv_touched = true;
                broke.then_some(c.pos)
            }
            _ => return,
        };
        self.send_inventory(addr);
        if let Some(at) = broke {
            self.show_held(addr);
            self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound(bedrock::SOUND_BREAK, at, "", false, -1));
        }
    }

    /// Un nombre d'expérience tiré entre deux bornes.
    pub(super) fn xp_between(&mut self, lo: u32, hi: u32) -> i32 {
        (lo + self.mob_rng.below(hi.saturating_sub(lo) + 1)) as i32
    }

    /// Clic sur un lit : le point de réapparition est fixé ; la nuit, le joueur se couche.
    pub(super) fn use_bed(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) {
        let tick = self.tick;
        let other = self.bed_other_half(self.world.get(pos.0, pos.1, pos.2), pos);
        let taken = self.clients.iter().any(|(a, c)| a != addr && c.sleeping.is_some() && (c.sleeping == Some(pos) || c.sleeping == other)) || self.mob_sleeps_in(pos) || other.map(|o| self.mob_sleeps_in(o)).unwrap_or(false);
        match self.clients.get_mut(addr) {
            Some(c) if c.joined && c.dead.is_none() && !c.mode.is_spectator() => {
                if let Some(d) = c.data.as_mut() {
                    if d.spawn != Some(pos) {
                        d.spawn = Some(pos);
                        self.reply(Some(*addr), "§7Point de réapparition défini");
                    }
                }
            }
            _ => return,
        }
        if taken {
            return self.reply(Some(*addr), "§7Ce lit est occupé");
        }
        if !NIGHT.contains(&self.world_time().rem_euclid(24_000)) {
            return self.reply(Some(*addr), "§7Vous ne pouvez dormir que la nuit");
        }
        if self.plugins.listens("PlayerBedEnterEvent") {
            let bed = self.block_json(self.world.get(pos.0, pos.1, pos.2), pos);
            let out = self.fire("PlayerBedEnterEvent", vec![("player", self.name_json(addr)), ("bed", bed)]);
            if cancelled(&out) {
                return;
            }
        }
        if let Some(c) = self.clients.get_mut(addr) {
            c.sleeping = Some(pos);
            c.slept_at = tick;
        }
        self.dismount(addr);
        self.show_sleep(addr);
    }

    /// Annonce au joueur et à ceux qui le voient s'il dort (et dans quel lit).
    fn show_sleep(&mut self, addr: &SocketAddr) {
        let Some((id, bed)) = self.clients.get(addr).map(|c| (c.id, c.sleeping)) else { return };
        let (x, y, z) = bed.unwrap_or((0, 0, 0));
        let packet = bedrock::set_actor_data(id, &[(bedrock::META_PLAYER_FLAGS, bedrock::Meta::Byte(if bed.is_some() { bedrock::PLAYER_FLAG_SLEEP } else { 0 })), (bedrock::META_BED_POSITION, bedrock::Meta::Pos(x, y, z))]);
        let mut to: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.visible.contains(&id)).map(|(a, _)| *a).collect();
        to.push(*addr);
        for a in to {
            self.send(&a, bedrock::ID_SET_ACTOR_DATA, &packet);
        }
    }

    /// Le joueur se lève (de lui-même, au matin, ou parce qu'il est frappé).
    pub(super) fn wake(&mut self, addr: &SocketAddr) {
        let (id, tail, bed) = match self.clients.get_mut(addr) {
            Some(c) if c.sleeping.is_some() => {
                let bed = c.sleeping.take();
                (c.id, c.animate_tail.clone(), bed)
            }
            _ => return,
        };
        if let (Some(pos), true) = (bed, self.plugins.listens("PlayerBedLeaveEvent")) {
            let block = self.block_json(self.world.get(pos.0, pos.1, pos.2), pos);
            self.fire("PlayerBedLeaveEvent", vec![("player", self.name_json(addr)), ("bed", block)]);
        }
        self.show_sleep(addr);
        // L'animation du lever : sans elle le jeu garde la posture et la taille du dormeur
        // (il se met à ramper dès qu'il avance)
        if !self.animate_refused {
            let packet = bedrock::animate_wake(id, &tail);
            let mut to: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.visible.contains(&id)).map(|(a, _)| *a).collect();
            to.push(*addr);
            for a in to {
                self.send(&a, bedrock::ID_ANIMATE, &packet);
            }
        }
    }

    /// Une fois par seconde : la nuit passe quand tous les joueurs dorment depuis 5 secondes ;
    /// au matin, tout le monde se lève.
    pub(super) fn sleep_tick(&mut self) {
        let tick = self.tick;
        let sleepers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.sleeping.is_some()).map(|(a, _)| *a).collect();
        if sleepers.is_empty() {
            return;
        }
        let now = self.world_time();
        if !NIGHT.contains(&now.rem_euclid(24_000)) {
            for a in sleepers {
                self.wake(&a);
            }
            return;
        }
        // Tous les joueurs en jeu (hors spectateurs et morts) doivent être couchés
        let everyone = self.clients.values().filter(|c| c.joined && c.dead.is_none() && !c.mode.is_spectator()).all(|c| matches!(c.sleeping, Some(_)) && tick.saturating_sub(c.slept_at) >= SLEEP_TICKS);
        if !everyone {
            return;
        }
        let morning = (now.div_euclid(24_000) + 1) * 24_000;
        self.time_base += morning - now;
        self.broadcast(bedrock::ID_SET_TIME, &bedrock::set_time(morning as i32), None);
        for a in sleepers {
            self.wake(&a);
        }
    }

    /// Point de réapparition d'un joueur : son lit s'il est toujours là, sinon celui du monde.
    pub(super) fn respawn_point(&mut self, addr: &SocketAddr) -> (f32, f32, f32) {
        let bed = self.clients.get(addr).and_then(|c| c.data.as_ref()).and_then(|d| d.spawn);
        if let Some((x, y, z)) = bed {
            let state = self.world.get(x, y, z);
            if self.data.as_ref().and_then(|d| d.block_name(state)) == Some("minecraft:bed") {
                return (x as f32 + 0.5, y as f32 + 0.5625 + player_eye_height(), z as f32 + 0.5);
            }
            if let Some(d) = self.clients.get_mut(addr).and_then(|c| c.data.as_mut()) {
                d.spawn = None;
            }
            self.reply(Some(*addr), "§7Votre lit a disparu ou est inaccessible");
        }
        self.spawn_point()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_follows_the_game_levels() {
        // 7 points pour le premier niveau, 9 pour le deuxième
        assert_eq!((xp_to_next(0), xp_to_next(1), xp_to_next(15), xp_to_next(16), xp_to_next(30), xp_to_next(31)), (7, 9, 37, 42, 112, 121));
        assert_eq!(add_xp(0, 0.0, 7).0, 1);
        let (level, progress) = add_xp(0, 0.0, 10);
        assert_eq!(level, 1);
        assert!((progress - 3.0 / 9.0).abs() < 0.01);
        // Une grosse somme d'un coup : 30 niveaux valent 1395 points
        assert_eq!(add_xp(0, 0.0, 1395).0, 30);
        // Rien ne se perd en plusieurs fois
        let mut state = (0, 0.0);
        for _ in 0..7 {
            state = add_xp(state.0, state.1, 1);
        }
        assert_eq!(state.0, 1);
    }

    #[test]
    fn food_and_ores_are_known() {
        let food = |name: &str| VanillaItems::get(name).food();
        assert_eq!(food("minecraft:bread").map(|f| (f.hunger, f.saturation, f.always_edible)), Some((5, 6.0, false)));
        assert_eq!(food("golden_apple").map(|f| f.always_edible), Some(true));
        assert!(food("minecraft:stone").is_none());
        assert_eq!(ore_xp("minecraft:deepslate_diamond_ore"), Some((3, 7)));
        assert_eq!(ore_xp("minecraft:iron_ore"), None);
        assert_eq!(food("minecraft:rabbit_stew").and_then(|f| f.leftover), Some("bowl"));
        assert_eq!(food("dried_kelp").map(|f| f.eat_ticks), Some(16));
    }
}
