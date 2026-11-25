//! Effets des joueurs (vitesse, poison, régénération…) : ceux de la commande `/effect`, des
//! plugins et des monstres.
//!
//! Le jeu affiche lui-même un effet annoncé (icône, particules, décompte) et applique ceux
//! qui touchent le déplacement ou la vue (vitesse, saut, vision nocturne, cécité…). Le
//! serveur applique ceux qui touchent la vie, la faim et les coups.

use super::*;

/// Effets du jeu : (numéro, nom).
pub(super) const EFFECTS: [(i32, &str); 30] = [
    (1, "speed"),
    (2, "slowness"),
    (3, "haste"),
    (4, "mining_fatigue"),
    (5, "strength"),
    (6, "instant_health"),
    (7, "instant_damage"),
    (8, "jump_boost"),
    (9, "nausea"),
    (10, "regeneration"),
    (11, "resistance"),
    (12, "fire_resistance"),
    (13, "water_breathing"),
    (14, "invisibility"),
    (15, "blindness"),
    (16, "night_vision"),
    (17, "hunger"),
    (18, "weakness"),
    (19, "poison"),
    (20, "wither"),
    (21, "health_boost"),
    (22, "absorption"),
    (23, "saturation"),
    (24, "levitation"),
    (25, "fatal_poison"),
    (26, "conduit_power"),
    (27, "slow_falling"),
    (28, "bad_omen"),
    (29, "village_hero"),
    (30, "darkness"),
];

pub(super) const SPEED: i32 = 1;
pub(super) const SLOWNESS: i32 = 2;
pub(super) const STRENGTH: i32 = 5;
pub(super) const INSTANT_HEALTH: i32 = 6;
pub(super) const INSTANT_DAMAGE: i32 = 7;
pub(super) const REGENERATION: i32 = 10;
pub(super) const RESISTANCE: i32 = 11;
pub(super) const FIRE_RESISTANCE: i32 = 12;
pub(super) const HUNGER: i32 = 17;
pub(super) const WEAKNESS: i32 = 18;
pub(super) const POISON: i32 = 19;
pub(super) const WITHER: i32 = 20;
pub(super) const SATURATION: i32 = 23;
pub(super) const FATAL_POISON: i32 = 25;

/// Numéro d'un effet d'après son nom (`speed`, `minecraft:speed`) ou son numéro.
pub(super) fn effect_id(word: &str) -> Option<i32> {
    let short = word.trim_start_matches("minecraft:").to_ascii_lowercase();
    if let Ok(n) = short.parse::<i32>() {
        return EFFECTS.iter().any(|e| e.0 == n).then_some(n);
    }
    EFFECTS.iter().find(|e| e.1 == short).map(|e| e.0)
}

/// Nom d'un effet d'après son numéro.
pub(super) fn effect_name(id: i32) -> &'static str {
    EFFECTS.iter().find(|e| e.0 == id).map(|e| e.1).unwrap_or("?")
}

/// Ticks entre deux battements d'un effet périodique à ce niveau (0 : le premier), comme
/// dans le jeu : poison toutes les 25, flétrissement toutes les 40, régénération toutes
/// les 50, deux fois plus vite à chaque niveau.
pub(super) fn beat(effect: i32, level: i32) -> u64 {
    let base: u64 = match effect {
        POISON | FATAL_POISON => 25,
        WITHER => 40,
        REGENERATION => 50,
        HUNGER => 20,
        _ => return 0,
    };
    (base >> level.clamp(0, 6)).max(1)
}

/// Un effet en cours : (numéro, niveau à partir de 0, tick de fin).
pub(super) type Effect = (i32, i32, u64);

impl Server {
    /// Niveau d'un effet en cours chez un joueur (1 pour le premier niveau), 0 sans l'effet.
    pub(super) fn effect_level(&self, addr: &SocketAddr, effect: i32) -> i32 {
        let tick = self.tick;
        self.clients.get(addr).and_then(|c| c.effects.iter().find(|e| e.0 == effect && e.2 > tick)).map(|e| e.1 + 1).unwrap_or(0)
    }

    /// Donne un effet à un joueur pour tant de ticks. `level` : 0 pour le premier niveau.
    /// Les effets instantanés (soin, dégâts, satiété) agissent tout de suite.
    pub(super) fn give_effect(&mut self, addr: &SocketAddr, effect: i32, level: i32, ticks: u64, particles: bool) {
        let level = level.clamp(0, 255);
        let tick = self.tick;
        let id = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() && !c.mode.is_spectator() => c.id,
            _ => return,
        };
        match effect {
            INSTANT_HEALTH => return self.heal(addr, (4i32 << level.min(5)) as f32),
            INSTANT_DAMAGE => return self.hurt(addr, (6i32 << level.min(5)) as f32, Cause::Magic),
            SATURATION if ticks <= 1 => {
                if let Some(d) = self.clients.get_mut(addr).and_then(|c| c.data.as_mut()) {
                    d.food = (d.food + (level + 1) as f32).min(20.0);
                }
                return self.sync_attributes(addr);
            }
            _ => {}
        }
        let ticks = ticks.clamp(1, 20 * 60 * 60 * 24);
        if let Some(c) = self.clients.get_mut(addr) {
            c.effects.retain(|e| e.0 != effect);
            c.effects.push((effect, level, tick + ticks));
        }
        if !self.refused_extras.contains(&bedrock::ID_MOB_EFFECT) {
            self.send(addr, bedrock::ID_MOB_EFFECT, &bedrock::mob_effect(id, bedrock::EFFECT_ADD, effect, level, ticks as i32, particles));
        }
        if effect == SPEED || effect == SLOWNESS {
            self.sync_speed(addr);
        }
    }

    /// Ce que les effets font à la vitesse de marche : +20 % par niveau de rapidité, -15 % par
    /// niveau de lenteur (1 sans effet).
    pub(super) fn speed_factor(c: &Client) -> f32 {
        let level = |effect: i32| c.effects.iter().find(|e| e.0 == effect).map(|e| (e.1 + 1) as f32).unwrap_or(0.0);
        (1.0 + 0.2 * level(SPEED)) * (1.0 - 0.15 * level(SLOWNESS)).max(0.0)
    }

    /// Donne au jeu du joueur sa vitesse de marche, effets compris, quand un effet commence ou
    /// finit. Le jeu ne la recalcule de lui-même qu'en changeant d'allure : sans cela, la
    /// rapidité n'agit qu'au premier sprint.
    pub(super) fn sync_speed(&mut self, addr: &SocketAddr) {
        // (jamais pendant un sprint : recevoir sa vitesse l'interromprait, et le jeu la recalcule
        // de lui-même, effets compris, dès qu'il change d'allure)
        let Some(c) = self.clients.get(addr).filter(|c| c.joined && !c.sprinting) else { return };
        let value = 0.1 * Self::speed_factor(c);
        let packet = bedrock::update_movement(c.id as u64, value);
        self.send(addr, bedrock::ID_UPDATE_ATTRIBUTES, &packet);
    }

    /// Retire un effet (ou tous : `None`). Renvoie le nombre d'effets retirés.
    pub(super) fn clear_effects(&mut self, addr: &SocketAddr, only: Option<i32>) -> usize {
        let (id, gone): (i64, Vec<i32>) = match self.clients.get_mut(addr) {
            Some(c) => {
                let gone: Vec<i32> = c.effects.iter().filter(|e| only.map(|o| o == e.0).unwrap_or(true)).map(|e| e.0).collect();
                c.effects.retain(|e| !gone.contains(&e.0));
                (c.id, gone)
            }
            None => return 0,
        };
        if !self.refused_extras.contains(&bedrock::ID_MOB_EFFECT) {
            for effect in &gone {
                self.send(addr, bedrock::ID_MOB_EFFECT, &bedrock::mob_effect(id, bedrock::EFFECT_REMOVE, *effect, 0, 0, false));
            }
        }
        if gone.contains(&SPEED) || gone.contains(&SLOWNESS) {
            self.sync_speed(addr);
        }
        gone.len()
    }

    /// Rend de la vie à un joueur, sans dépasser son maximum.
    pub(super) fn heal(&mut self, addr: &SocketAddr, amount: f32) {
        if let Some(d) = self.clients.get_mut(addr).filter(|c| c.dead.is_none()).and_then(|c| c.data.as_mut()) {
            if d.health < d.max_health {
                d.health = (d.health + amount).min(d.max_health);
                self.sync_attributes(addr);
            }
        }
    }

    /// Met le feu à un joueur pour tant de ticks (rien en créatif, ni avec la résistance au feu).
    pub(super) fn ignite(&mut self, addr: &SocketAddr, ticks: u64) {
        if self.effect_level(addr, FIRE_RESISTANCE) > 0 {
            return;
        }
        let until = self.tick + ticks;
        match self.clients.get_mut(addr) {
            Some(c) if c.joined && c.dead.is_none() && c.mode.takes_damage() => c.burn_until = c.burn_until.max(until),
            _ => return,
        }
        self.show_fire(addr, true);
    }

    /// Éteint un joueur en feu.
    pub(super) fn extinguish(&mut self, addr: &SocketAddr) {
        let was = self.clients.get_mut(addr).map(|c| std::mem::take(&mut c.burn_until) > 0).unwrap_or(false);
        if was {
            self.show_fire(addr, false);
        }
    }

    /// Montre (ou cache) les flammes sur un joueur, chez lui et chez ceux qui le voient.
    fn show_fire(&mut self, addr: &SocketAddr, on: bool) {
        let Some(c) = self.clients.get(addr) else { return };
        let (id, sneaking, sprinting) = (c.id, c.sneaking, c.sprinting);
        let extra = (on as i64) | if c.riding.is_some() { bedrock::FLAG_RIDING } else { 0 };
        self.send(addr, bedrock::ID_SET_ACTOR_DATA, &bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(bedrock::OWN_PLAYER_FLAGS | extra))]));
        let packet = bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(Self::remote_flags(sneaking, sprinting) | extra))]);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| *a != addr && c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_SET_ACTOR_DATA, &packet);
        }
    }

    /// Une fois par seconde : un joueur en feu perd un demi-cœur, jusqu'à ce que le feu
    /// s'éteigne de lui-même ou dans l'eau.
    pub(super) fn fire_tick(&mut self) {
        let tick = self.tick;
        let water = self.world.generator().blocks.water;
        let burning: Vec<(SocketAddr, bool)> = self
            .clients
            .iter()
            .filter(|(_, c)| c.burn_until > 0)
            .map(|(a, c)| {
                let wet = self.world.get(c.pos.0.floor() as i32, (c.pos.1 - EYE).floor() as i32, c.pos.2.floor() as i32) == water;
                (*a, c.burn_until <= tick || wet || c.dead.is_some() || !c.mode.takes_damage())
            })
            .collect();
        for (addr, over) in burning {
            if over || self.effect_level(&addr, FIRE_RESISTANCE) > 0 {
                self.extinguish(&addr);
            } else {
                self.hurt(&addr, 1.0, Cause::Fire);
                // (les flammes sont remontrées : un autre changement d'allure a pu les effacer)
                self.show_fire(&addr, true);
            }
        }
    }

    /// Un tick d'effets : ceux qui sont finis s'effacent, les périodiques battent.
    pub(super) fn effects_tick(&mut self) {
        let tick = self.tick;
        // Deux secondes après l'arrivée : la vie et la faim, que le jeu n'affiche pas toujours
        // quand il les reçoit pendant son chargement
        let late: Vec<SocketAddr> = self.clients.iter_mut().filter(|(_, c)| c.resync_at != 0 && tick >= c.resync_at).map(|(a, c)| { c.resync_at = 0; *a }).collect();
        for addr in late {
            self.send_attributes(&addr, false);
        }
        // Les effets d'avant la déconnexion, rendus deux secondes après le retour
        let back: Vec<(SocketAddr, Vec<i32>)> = self.clients.iter_mut().filter(|(_, c)| !c.returning.is_empty() && c.joined && tick.saturating_sub(c.probe_tick) >= 40).map(|(a, c)| (*a, std::mem::take(&mut c.returning))).collect();
        for (addr, kept) in back {
            for e in kept.chunks_exact(3).filter(|e| e[2] > 0) {
                self.give_effect(&addr, e[0], e[1], e[2] as u64, true);
            }
            self.sync_speed(&addr);
        }
        let mut beats: Vec<(SocketAddr, i32, i32)> = Vec::new();
        // Ceux dont la rapidité ou la lenteur vient de finir : leur vitesse redevient normale
        let mut slowed: Vec<SocketAddr> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if c.effects.is_empty() {
                continue;
            }
            // (le jeu fait disparaître l'icône de lui-même, à la fin de la durée annoncée)
            if c.effects.iter().any(|e| e.2 <= tick && (e.0 == SPEED || e.0 == SLOWNESS)) {
                slowed.push(*addr);
            }
            c.effects.retain(|e| e.2 > tick);
            if !c.joined || c.dead.is_some() {
                continue;
            }
            for (effect, level, until) in c.effects.iter() {
                let every = beat(*effect, *level);
                if every > 0 && (until - tick) % every == 0 {
                    beats.push((*addr, *effect, *level));
                }
            }
        }
        for addr in slowed {
            self.sync_speed(&addr);
        }
        for (addr, effect, level) in beats {
            let health = self.clients.get(&addr).and_then(|c| c.data.as_ref()).map(|d| d.health).unwrap_or(0.0);
            let fair = self.clients.get(&addr).map(|c| c.mode.takes_damage()).unwrap_or(false);
            match effect {
                // Le poison laisse toujours un demi-cœur ; sa forme mortelle et le flétrissement, non
                POISON if fair && health > 1.0 => self.hurt(&addr, 1.0, Cause::Poison),
                FATAL_POISON | WITHER if fair => self.hurt(&addr, 1.0, Cause::Magic),
                REGENERATION => self.heal(&addr, 1.0),
                HUNGER if fair => self.exhaust(&addr, 0.1 * (level + 1) as f32),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_are_found_by_name_or_number() {
        assert_eq!(effect_id("speed"), Some(1));
        assert_eq!(effect_id("minecraft:Poison"), Some(19));
        assert_eq!(effect_id("16"), Some(16));
        assert_eq!((effect_id("nimporte"), effect_id("99")), (None, None));
        assert_eq!(effect_name(10), "regeneration");
        for (i, (id, _)) in EFFECTS.iter().enumerate() {
            assert_eq!(*id, i as i32 + 1, "les numéros se suivent");
        }
    }

    #[test]
    fn periodic_effects_beat_faster_with_each_level() {
        assert_eq!((beat(POISON, 0), beat(POISON, 1), beat(POISON, 2)), (25, 12, 6));
        assert_eq!((beat(REGENERATION, 0), beat(REGENERATION, 1)), (50, 25));
        assert_eq!((beat(WITHER, 0), beat(SPEED, 3)), (40, 0));
        assert_eq!(beat(REGENERATION, 60), 1, "jamais moins d'un tick");
    }
}
