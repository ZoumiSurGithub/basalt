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

#[cfg(test)]
pub(super) const SPEED: i32 = 1;
pub(super) const STRENGTH: i32 = 5;
pub(super) const INSTANT_HEALTH: i32 = 6;
pub(super) const INSTANT_DAMAGE: i32 = 7;
pub(super) const REGENERATION: i32 = 10;
pub(super) const RESISTANCE: i32 = 11;
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

    /// Un tick d'effets : ceux qui sont finis s'effacent, les périodiques battent.
    pub(super) fn effects_tick(&mut self) {
        let tick = self.tick;
        let mut beats: Vec<(SocketAddr, i32, i32)> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if c.effects.is_empty() {
                continue;
            }
            // (le jeu fait disparaître l'icône de lui-même, à la fin de la durée annoncée)
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
