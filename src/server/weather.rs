//! La météo : la pluie et l'orage, leur cycle naturel, la foudre, les feux que la pluie
//! éteint. Le jeu dessine lui-même la pluie (ou la neige) selon le biome où il se trouve.

use super::*;

/// Événements de niveau : la pluie et l'orage commencent ou s'arrêtent. La donnée du début
/// est l'intensité, de 0 à 65 535.
const START_RAIN: i32 = 3001;
const START_THUNDER: i32 = 3002;
const STOP_RAIN: i32 = 3003;
const STOP_THUNDER: i32 = 3004;
const FULL: i32 = 65_535;

/// Dégâts d'un éclair, et rayon (en blocs) où il blesse.
const BOLT_DAMAGE: f32 = 5.0;
const BOLT_REACH: f32 = 3.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Sky {
    Clear,
    Rain,
    Thunder,
}

impl Sky {
    pub(super) fn parse(word: &str) -> Option<Sky> {
        match word {
            "clear" => Some(Sky::Clear),
            "rain" => Some(Sky::Rain),
            "thunder" => Some(Sky::Thunder),
            _ => None,
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Sky::Clear => "ciel dégagé",
            Sky::Rain => "pluie",
            Sky::Thunder => "orage",
        }
    }
    pub(super) fn rains(self) -> bool {
        self != Sky::Clear
    }
}

pub(super) struct Weather {
    pub sky: Sky,
    /// Tick où le temps change de lui-même
    pub until: u64,
}

/// Durée naturelle d'un temps, en ticks : le beau temps tient d'une demi-journée à sept jours
/// et demi, la pluie et l'orage d'une demi-journée à une journée. `roll` : de 0 à 999.
pub(super) fn natural_span(sky: Sky, roll: u32) -> u64 {
    let (least, most): (u64, u64) = if sky == Sky::Clear { (12_000, 180_000) } else { (12_000, 24_000) };
    least + (most - least) * roll.min(999) as u64 / 999
}

/// Les paquets qui font passer l'écran d'un joueur d'un temps à un autre.
pub(super) fn packets(from: Sky, to: Sky) -> Vec<Vec<u8>> {
    let event = |e: i32, data: i32| bedrock::level_event(e, (0.0, 0.0, 0.0), data);
    let mut list = Vec::new();
    if from == Sky::Thunder && to != Sky::Thunder {
        list.push(event(STOP_THUNDER, 0));
    }
    if from.rains() && !to.rains() {
        list.push(event(STOP_RAIN, 0));
    }
    if to.rains() && !from.rains() {
        list.push(event(START_RAIN, FULL));
    }
    if to == Sky::Thunder && from != Sky::Thunder {
        list.push(event(START_THUNDER, FULL));
    }
    list
}

/// La neige tient-elle sur ce bloc (nom sans `minecraft:`) ? Pas sur la neige ou la glace
/// déjà là, ni sur ce qui n'offre pas de dessus plein.
pub(super) fn snow_settles_on(name: &str) -> bool {
    const NEVER: [&str; 14] = ["snow_layer", "ice", "fence", "wall", "pane", "chest", "farmland", "path", "slab", "stairs", "cauldron", "lantern", "bars", "magma"];
    !name.is_empty() && !NEVER.iter().any(|n| name.contains(n))
}

/// Le contenu de `weather.txt`.
pub(super) fn parse_saved(text: &str) -> Option<(Sky, u64)> {
    let mut words = text.split_whitespace();
    let sky = Sky::parse(words.next()?)?;
    let left = words.next()?.parse::<u64>().ok()?.min(1_000_000 * 20);
    Some((sky, left))
}

impl Server {
    /// Change le temps pour `span` ticks (None : une durée naturelle) et le montre à tous.
    pub(super) fn set_weather(&mut self, sky: Sky, span: Option<u64>) {
        let from = self.weather.sky;
        let span = span.unwrap_or_else(|| natural_span(sky, self.mob_rng.below(1000)));
        self.weather = Weather { sky, until: self.tick + span };
        for packet in packets(from, sky) {
            self.broadcast(bedrock::ID_LEVEL_EVENT, &packet, None);
        }
    }

    /// Le temps qu'il fait, pour un joueur qui arrive.
    pub(super) fn show_weather(&mut self, addr: &SocketAddr) {
        for packet in packets(Sky::Clear, self.weather.sky) {
            self.send(addr, bedrock::ID_LEVEL_EVENT, &packet);
        }
    }

    /// Le temps qu'il fait à cet endroit : None s'il n'y tombe rien (beau temps, ou biome sec),
    /// sinon vrai s'il y neige.
    pub(super) fn falling_at(&self, x: i32, z: i32) -> Option<bool> {
        if !self.weather.sky.rains() {
            return None;
        }
        match self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z)) {
            Some(b) if b.rainless() => None,
            Some(b) => Some(b.snowy()),
            None => Some(false),
        }
    }

    /// Il pleut sur ce bloc : il est à ciel ouvert, dans un biome où la pluie tombe. Le bloc
    /// lui-même peut être occupé (une culture sur sa terre).
    pub(super) fn rained_on(&self, (x, y, z): (i32, i32, i32)) -> bool {
        self.falling_at(x, z) == Some(false) && self.under_sky((x, y, z))
    }

    /// De la neige se dépose, l'eau gèle, les chaudrons se remplissent : quelques colonnes
    /// prises au hasard autour de chaque joueur, une fois par seconde.
    fn precipitation(&mut self) {
        if !self.cfg.vanilla.snow_and_ice {
            return;
        }
        let Some(d) = self.data.clone() else { return };
        let (air, water) = (self.world.air, self.world.generator().blocks.water);
        let around: Vec<(f32, f32, f32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).map(|c| c.pos).collect();
        for pos in around {
            for _ in 0..8 {
                let (x, z) = (pos.0.floor() as i32 + self.mob_rng.below(97) as i32 - 48, pos.2.floor() as i32 + self.mob_rng.below(97) as i32 - 48);
                if !self.world.is_ready(x >> 4, z >> 4) {
                    continue;
                }
                let Some(snow) = self.falling_at(x, z) else { continue };
                let y = self.world.highest(x, z, &|state: u32| state != air);
                if !crate::world::World::in_bounds(y) || !crate::world::World::in_bounds(y + 1) {
                    continue;
                }
                let top = self.world.get(x, y, z);
                let name = d.block_name(top).unwrap_or("").trim_start_matches("minecraft:").to_string();
                if snow {
                    if top == water {
                        if let Some(ice) = d.default_state("minecraft:ice") {
                            self.change_block((x, y, z), ice);
                        }
                    } else if snow_settles_on(&name) && !self.mob_free(x, y, z) {
                        if let Some(layer) = d.default_state("minecraft:snow_layer") {
                            self.change_block((x, y + 1, z), layer);
                        }
                    }
                } else if name == "cauldron" {
                    // (vide ou déjà à l'eau : un cran de plus, jusqu'à plein)
                    let level = match d.prop(top, "fill_level") {
                        Some(crate::data::Prop::Int(n)) => n,
                        _ => continue,
                    };
                    let watery = level == 0 || d.prop(top, "cauldron_liquid") == Some(crate::data::Prop::Str("water".into()));
                    if watery && level < 6 {
                        if let Some(fuller) = d.with_props(top, &[("fill_level", crate::data::Prop::Int(level + 1)), ("cauldron_liquid", crate::data::Prop::Str("water".into()))]) {
                            self.change_block((x, y, z), fuller);
                        }
                    }
                }
            }
        }
    }

    /// Rien au-dessus de ce bloc jusqu'au ciel : la pluie l'atteint.
    pub(super) fn under_sky(&self, (x, y, z): (i32, i32, i32)) -> bool {
        let air = self.world.air;
        self.world.is_ready(x >> 4, z >> 4) && (y + 1..crate::world::MAX_Y).all(|above| self.world.get(x, above, z) == air)
    }

    /// Une fois par seconde : le cycle naturel, les feux que la pluie éteint, la foudre.
    pub(super) fn weather_tick(&mut self) {
        for id in std::mem::take(&mut self.bolts) {
            self.broadcast(bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id), None);
        }
        if self.cfg.weather_cycle && self.do_weather && !self.other_dimension() && self.tick >= self.weather.until {
            // (après le beau temps la pluie, une fois sur cinq un orage ; après la pluie le beau temps)
            let next = if self.weather.sky != Sky::Clear {
                Sky::Clear
            } else if self.mob_rng.below(5) == 0 {
                Sky::Thunder
            } else {
                Sky::Rain
            };
            self.set_weather(next, None);
            info!("Météo : {}", next.name());
        }
        if !self.weather.sky.rains() {
            return;
        }
        // La pluie éteint les feux à ciel ouvert, et les joueurs qui brûlent dehors
        self.precipitation();
        let doused: Vec<(i32, i32, i32)> = self.fires.iter().take(256).copied().filter(|p| self.falling_at(p.0, p.2).is_some() && self.under_sky(*p)).collect();
        let air = self.world.air;
        for p in doused {
            if self.mob_rng.below(3) == 0 {
                self.fires.remove(&p);
                self.change_block(p, air);
            }
        }
        let wet: Vec<SocketAddr> = self
            .clients
            .iter()
            .filter(|(_, c)| c.joined && c.dead.is_none() && c.burn_until > self.tick)
            .filter(|(_, c)| self.falling_at(c.pos.0.floor() as i32, c.pos.2.floor() as i32).is_some() && self.under_sky((c.pos.0.floor() as i32, c.pos.1.floor() as i32, c.pos.2.floor() as i32)))
            .map(|(a, _)| *a)
            .collect();
        for addr in wet {
            self.extinguish(&addr);
        }
        // Les créatures en feu aussi
        let soaked: Vec<i64> = self
            .mobs
            .iter()
            .filter(|(_, m)| m.fire_until > self.tick)
            .filter(|(_, m)| self.falling_at(m.pos.0.floor() as i32, m.pos.2.floor() as i32).is_some() && self.under_sky((m.pos.0.floor() as i32, (m.pos.1 + m.size().1).floor() as i32, m.pos.2.floor() as i32)))
            .map(|(id, _)| *id)
            .collect();
        for id in soaked {
            self.set_mob_fire(id, 0);
        }
        // L'orage : de temps en temps, la foudre tombe près d'un joueur
        if self.weather.sky == Sky::Thunder && self.cfg.vanilla.lightning {
            let around: Vec<(f32, f32, f32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).map(|c| c.pos).collect();
            for pos in around {
                if self.mob_rng.below(12) != 0 {
                    continue;
                }
                let (x, z) = (pos.0.floor() as i32 + self.mob_rng.below(129) as i32 - 64, pos.2.floor() as i32 + self.mob_rng.below(129) as i32 - 64);
                let solid = |state: u32| state != air;
                if self.world.is_ready(x >> 4, z >> 4) {
                    let y = self.world.highest(x, z, &solid) + 1;
                    self.strike((x, y, z));
                }
            }
        }
    }

    /// La météo est gardée avec le monde, dans `weather.txt` : « temps ticks_restants ».
    pub(super) fn save_weather(&self) {
        let word = match self.weather.sky {
            Sky::Clear => "clear",
            Sky::Rain => "rain",
            Sky::Thunder => "thunder",
        };
        let path = std::path::PathBuf::from(&self.cfg.world_dir).join("weather.txt");
        if let Err(e) = std::fs::write(&path, format!("{} {}\n", word, self.weather.until.saturating_sub(self.tick))) {
            warn!("{} : {}", path.display(), e);
        }
    }

    pub(super) fn load_weather(&mut self) {
        // La météo est dans level.dat, comme dans le jeu ; weather.txt (versions précédentes)
        // n'est relu que s'il est encore là
        let dir = std::path::PathBuf::from(&self.cfg.world_dir);
        if !dir.join("weather.txt").exists() {
            if let Ok(Some(level)) = crate::world::mojang::read_level(&dir) {
                let level_of = |key: &str| level.get(key).and_then(|t| t.as_f32()).unwrap_or(0.0) > 0.5;
                let sky = match (level_of("rainLevel"), level_of("lightningLevel")) {
                    (true, true) => Sky::Thunder,
                    (true, false) => Sky::Rain,
                    _ => Sky::Clear,
                };
                let left = level.get("rainTime").and_then(|t| t.as_i64()).filter(|t| (1..=10_000_000).contains(t)).map(|t| t as u64).unwrap_or_else(|| natural_span(sky, self.mob_rng.below(1000)));
                self.weather = Weather { sky, until: self.tick + left };
            }
            return;
        }
        let Ok(text) = std::fs::read_to_string(std::path::PathBuf::from(&self.cfg.world_dir).join("weather.txt")) else { return };
        if let Some((sky, left)) = parse_saved(&text) {
            self.weather = Weather { sky, until: self.tick + left };
        }
    }

    /// La foudre tombe sur ce bloc : l'éclair et le tonnerre, un feu, et des dégâts autour.
    pub(super) fn strike(&mut self, (x, y, z): (i32, i32, i32)) {
        let at = (x as f32 + 0.5, y as f32, z as f32 + 0.5);
        let id = self.next_id;
        self.next_id += 1;
        // (l'éclair se voit de loin : il part chez tous les joueurs à portée de vue)
        let to: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && c.stage == Stage::InGame && (c.pos.0 - at.0).hypot(c.pos.2 - at.2) < 160.0).map(|(a, _)| *a).collect();
        let (bolt, thunder) = (
            bedrock::add_actor(id, "minecraft:lightning_bolt", at, (0.0, 0.0, 0.0), 0.0, (1.0, 1.0), None, None, &[]),
            bedrock::level_sound("thunder", at, "minecraft:lightning_bolt", false, -1),
        );
        for a in &to {
            self.send(a, bedrock::ID_ADD_ACTOR, &bolt);
            self.send(a, bedrock::ID_LEVEL_SOUND_EVENT, &thunder);
        }
        // (l'éclair est retiré à la seconde suivante : le jeu a le temps de le dessiner)
        self.bolts.push(id);
        if self.cfg.fire_spreads {
            self.light_fire((x, y, z));
        }
        let struck: Vec<SocketAddr> = self
            .clients
            .iter()
            .filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage())
            .filter(|(_, c)| (c.pos.0 - at.0).hypot(c.pos.2 - at.2) < BOLT_REACH && (c.pos.1 - EYE - at.1).abs() < BOLT_REACH + 1.0)
            .map(|(a, _)| *a)
            .collect();
        for addr in struck {
            self.hurt(&addr, BOLT_DAMAGE, Cause::Mob("la foudre"));
            self.ignite(&addr, 160);
        }
        let mobs: Vec<i64> = self.mobs.iter().filter(|(_, m)| (m.pos.0 - at.0).hypot(m.pos.2 - at.2) < BOLT_REACH && (m.pos.1 - at.1).abs() < BOLT_REACH + 1.0).map(|(id, _)| *id).collect();
        let until = self.tick + 160;
        for m in mobs {
            self.hit_mob(m, at, BOLT_DAMAGE, mobs::Attacker::Mob(m));
            self.set_mob_fire(m, until);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sky_changes_with_the_right_events() {
        let ids = |from, to| packets(from, to).iter().map(|p| bedrock::read_vari32(p, &mut 0).unwrap()).collect::<Vec<_>>();
        assert_eq!(ids(Sky::Clear, Sky::Rain), vec![START_RAIN]);
        assert_eq!(ids(Sky::Clear, Sky::Thunder), vec![START_RAIN, START_THUNDER]);
        assert_eq!(ids(Sky::Rain, Sky::Thunder), vec![START_THUNDER]);
        assert_eq!(ids(Sky::Thunder, Sky::Rain), vec![STOP_THUNDER]);
        assert_eq!(ids(Sky::Thunder, Sky::Clear), vec![STOP_THUNDER, STOP_RAIN]);
        assert!(ids(Sky::Rain, Sky::Rain).is_empty() && ids(Sky::Clear, Sky::Clear).is_empty());
        // Les durées naturelles : la pluie dure moins que le beau temps
        assert_eq!((natural_span(Sky::Rain, 0), natural_span(Sky::Rain, 999), natural_span(Sky::Clear, 999), natural_span(Sky::Thunder, 5000)), (12_000, 24_000, 180_000, 24_000));
        // Le fichier de la météo, et les blocs où la neige tient
        assert_eq!((parse_saved("rain 4000\n"), parse_saved("thunder 12"), parse_saved("snow 3"), parse_saved("rain"), parse_saved("")), (Some((Sky::Rain, 4000)), Some((Sky::Thunder, 12)), None, None, None));
        assert!(snow_settles_on("grass_block") && snow_settles_on("stone") && snow_settles_on("oak_leaves") && !snow_settles_on("snow_layer") && !snow_settles_on("packed_ice") && !snow_settles_on("oak_fence") && !snow_settles_on(""));
        assert_eq!((Sky::parse("thunder"), Sky::parse("rain"), Sky::parse("clear"), Sky::parse("snow")), (Some(Sky::Thunder), Some(Sky::Rain), Some(Sky::Clear), None));
    }
}
