//! Créatures : les animaux paisibles (vache, cochon, mouton, poule).
//!
//! Ils apparaissent par petits groupes sur l'herbe autour des joueurs, se promènent, sautent
//! les marches d'un bloc, évitent les chutes, nagent, fuient quand on les frappe, meurent et
//! laissent leur butin. Ils ne sont pas enregistrés : loin de tous les joueurs, ils
//! disparaissent, et d'autres apparaissent ailleurs.

use super::*;
use crate::world::generator::Rng;

/// Espèces connues.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Species {
    Cow,
    Pig,
    Sheep,
    Chicken,
}

impl Species {
    const ALL: [Species; 4] = [Species::Cow, Species::Pig, Species::Sheep, Species::Chicken];

    fn id(self) -> &'static str {
        match self {
            Species::Cow => "minecraft:cow",
            Species::Pig => "minecraft:pig",
            Species::Sheep => "minecraft:sheep",
            Species::Chicken => "minecraft:chicken",
        }
    }
    fn health(self) -> f32 {
        match self {
            Species::Cow | Species::Pig => 10.0,
            Species::Sheep => 8.0,
            Species::Chicken => 4.0,
        }
    }
    /// Largeur et hauteur de la boîte de collision (celles du jeu)
    fn size(self) -> (f32, f32) {
        match self {
            Species::Cow => (0.9, 1.3),
            Species::Pig => (0.9, 0.9),
            Species::Sheep => (0.9, 1.3),
            Species::Chicken => (0.4, 0.7),
        }
    }
    /// Vitesse de marche (blocs par tick)
    fn speed(self) -> f32 {
        match self {
            Species::Chicken => 0.07,
            _ => 0.09,
        }
    }
    /// Butin : (objet, quantité minimale, maximale)
    fn drops(self) -> &'static [(&'static str, u8, u8)] {
        match self {
            Species::Cow => &[("minecraft:beef", 1, 3), ("minecraft:leather", 0, 2)],
            Species::Pig => &[("minecraft:porkchop", 1, 3)],
            Species::Sheep => &[("minecraft:mutton", 1, 2), ("minecraft:white_wool", 1, 1)],
            Species::Chicken => &[("minecraft:chicken", 1, 1), ("minecraft:feather", 0, 2)],
        }
    }
}

/// Une créature dans le monde.
#[derive(Clone)]
pub(super) struct Mob {
    pub id: i64,
    pub species: Species,
    /// Position des pieds
    pub pos: (f32, f32, f32),
    pub vel: (f32, f32, f32),
    pub yaw: f32,
    pub on_ground: bool,
    pub health: f32,
    /// Destination de la promenade en cours
    target: Option<(f32, f32)>,
    /// Ticks de fuite restants (après un coup)
    panic: u32,
    /// Ticks de recul restants (après un coup)
    knock: u32,
    /// Tick du dernier coup reçu (un coup par demi-seconde)
    hurt_tick: u64,
    /// Tick de la mort (retirée une seconde plus tard)
    pub dead: Option<u64>,
    /// Dernière position et orientation envoyées
    sent: (f32, f32, f32),
    sent_yaw: f32,
    /// Couleur de la laine (mouton)
    color: u8,
}

/// Créatures au plus par joueur (dans un rayon de 96 blocs), et au total.
const PER_PLAYER: usize = 10;
const MAX_MOBS: usize = 300;
/// Au-delà, une créature disparaît
const DESPAWN: f32 = 128.0;
/// Rayon d'affichage
const VIEW: f32 = 80.0;

impl Server {
    /// Fait apparaître de temps en temps un petit troupeau sur l'herbe, autour des joueurs qui
    /// n'ont pas assez d'animaux près d'eux.
    pub(super) fn spawn_mobs(&mut self) {
        if !self.cfg.spawn_animals || self.mobs_refused || self.mobs.len() >= MAX_MOBS {
            return;
        }
        let players: Vec<(f32, f32, f32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none()).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)).collect();
        let grass = self.world.generator().blocks.grass;
        for p in players {
            let near = self.mobs.values().filter(|m| (m.pos.0 - p.0).powi(2) + (m.pos.2 - p.2).powi(2) < 96.0 * 96.0).count();
            if near >= PER_PLAYER {
                continue;
            }
            // Un endroit au hasard, hors de vue immédiate
            let angle = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
            let dist = 24.0 + self.mob_rng.unit() as f32 * 40.0;
            let (x, z) = ((p.0 + angle.cos() * dist).floor() as i32, (p.2 + angle.sin() * dist).floor() as i32);
            let Some(y) = self.mob_ground(x, z, p.1 as i32) else { continue };
            if self.world.get(x, y - 1, z) != grass {
                continue;
            }
            let species = Species::ALL[self.mob_rng.below(4) as usize];
            let count = 2 + self.mob_rng.below(3);
            for _ in 0..count {
                let (dx, dz) = (self.mob_rng.below(5) as i32 - 2, self.mob_rng.below(5) as i32 - 2);
                let Some(yy) = self.mob_ground(x + dx, z + dz, y) else { continue };
                if self.world.get(x + dx, yy - 1, z + dz) == grass {
                    self.add_mob(species, ((x + dx) as f32 + 0.5, yy as f32, (z + dz) as f32 + 0.5));
                }
            }
        }
    }

    /// Hauteur où une créature peut se tenir dans cette colonne (sol solide, deux blocs libres
    /// au-dessus), en cherchant autour de `near`. None si le chunk n'est pas prêt.
    fn mob_ground(&self, x: i32, z: i32, near: i32) -> Option<i32> {
        if !self.world.is_ready(x >> 4, z >> 4) {
            return None;
        }
        let top = (near + 40).min(crate::world::MAX_Y - 2);
        let bottom = (near - 40).max(crate::world::MIN_Y + 1);
        let mut y = top;
        while y > bottom {
            if !self.mob_free(x, y - 1, z) && self.mob_free(x, y, z) && self.mob_free(x, y + 1, z) {
                let water = self.world.generator().blocks.water;
                if self.world.get(x, y - 1, z) == water {
                    return None;
                }
                return Some(y);
            }
            y -= 1;
        }
        None
    }

    /// Un bloc que l'on traverse (air, herbe haute, fleur, eau…).
    fn mob_free(&self, x: i32, y: i32, z: i32) -> bool {
        let s = self.world.get(x, y, z);
        s == self.world.generator().blocks.water || self.npc_passable(s)
    }

    fn mob_water(&self, x: f32, y: f32, z: f32) -> bool {
        self.world.get(x.floor() as i32, y.floor() as i32, z.floor() as i32) == self.world.generator().blocks.water
    }

    /// Ajoute une créature.
    pub(super) fn add_mob(&mut self, species: Species, pos: (f32, f32, f32)) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let color = if species == Species::Sheep {
            // Laine : surtout blanche, parfois grise, noire ou marron, rarement rose (le jeu)
            match self.mob_rng.below(1000) {
                0..=49 => 7,
                50..=99 => 8,
                100..=149 => 15,
                150..=179 => 12,
                180..=181 => 6,
                _ => 0,
            }
        } else {
            0
        };
        let yaw = self.mob_rng.unit() as f32 * 360.0;
        let mob = Mob { id, species, pos, vel: (0.0, 0.0, 0.0), yaw, on_ground: true, health: species.health(), target: None, panic: 0, knock: 0, hurt_tick: 0, dead: None, sent: pos, sent_yaw: yaw, color };
        self.mobs.insert(id, mob);
        id
    }

    /// Paquet d'apparition d'une créature.
    fn mob_packet(m: &Mob) -> Vec<u8> {
        let (w, h) = m.species.size();
        let meta = bedrock::mob_metadata(w, h, 0, m.color, bedrock::MOB_FLAGS);
        bedrock::add_actor(m.id, m.species.id(), m.pos, m.vel, m.yaw, m.species.health(), &meta)
    }

    /// Montre à chaque joueur les créatures proches de lui, retire les autres, et fait
    /// disparaître celles qui sont loin de tout le monde.
    pub(super) fn show_mobs(&mut self) {
        let players: Vec<(f32, f32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).map(|c| (c.pos.0, c.pos.2)).collect();
        let gone: Vec<i64> = self.mobs.values().filter(|m| players.iter().all(|p| (m.pos.0 - p.0).powi(2) + (m.pos.2 - p.1).powi(2) > DESPAWN * DESPAWN)).map(|m| m.id).collect();
        for id in gone {
            self.remove_mob(id);
        }
        let mut out: Vec<(SocketAddr, u32, Vec<u8>)> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if !c.joined || c.stage != Stage::InGame {
                continue;
            }
            for m in self.mobs.values() {
                let far = (m.pos.0 - c.pos.0).powi(2) + (m.pos.2 - c.pos.2).powi(2);
                if far < VIEW * VIEW && m.dead.is_none() {
                    if c.mobs_seen.insert(m.id) {
                        out.push((*addr, bedrock::ID_ADD_ACTOR, Self::mob_packet(m)));
                    }
                } else if far > (VIEW + 16.0) * (VIEW + 16.0) && c.mobs_seen.remove(&m.id) {
                    out.push((*addr, bedrock::ID_REMOVE_ACTOR, bedrock::remove_actor(m.id)));
                }
            }
        }
        for (addr, id, packet) in out {
            self.send(&addr, id, &packet);
        }
    }

    /// Retire une créature (chez tous les joueurs qui la voient).
    pub(super) fn remove_mob(&mut self, id: i64) {
        self.mobs.remove(&id);
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| c.mobs_seen.remove(&id).then_some(*a)).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
        }
    }

    /// Envoie un paquet aux joueurs qui voient cette créature.
    fn send_to_viewers(&mut self, id: i64, packet_id: u32, packet: &[u8]) {
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.mobs_seen.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, packet_id, packet);
        }
    }

    /// Un tick de vie des créatures : promenade, fuite, gravité, collisions.
    pub(super) fn move_mobs(&mut self) {
        let ids: Vec<i64> = self.mobs.keys().copied().collect();
        let tick = self.tick;
        for id in ids {
            let Some(mut m) = self.mobs.get(&id).cloned() else { continue };
            if let Some(t) = m.dead {
                if tick.saturating_sub(t) >= 20 {
                    self.remove_mob(id);
                }
                continue;
            }
            let (cx, cz) = ((m.pos.0.floor() as i32) >> 4, (m.pos.2.floor() as i32) >> 4);
            if !self.world.is_ready(cx, cz) {
                continue;
            }
            self.mob_think(&mut m);
            self.mob_physics(&mut m);
            let moved = (m.pos.0 - m.sent.0).abs() + (m.pos.1 - m.sent.1).abs() + (m.pos.2 - m.sent.2).abs() > 0.01 || (m.yaw - m.sent_yaw).abs() > 3.0;
            if moved {
                m.sent = m.pos;
                m.sent_yaw = m.yaw;
                let packet = bedrock::move_actor_absolute(m.id, m.pos, 0.0, m.yaw, m.yaw, m.on_ground);
                self.send_to_viewers(m.id, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
            }
            self.mobs.insert(id, m);
        }
    }

    /// Choix de la direction : se promener, s'arrêter, fuir.
    fn mob_think(&mut self, m: &mut Mob) {
        if m.knock > 0 {
            m.knock -= 1;
            m.vel.0 *= 0.7;
            m.vel.2 *= 0.7;
            return;
        }
        let speed = if m.panic > 0 { m.species.speed() * 2.0 } else { m.species.speed() };
        if m.panic > 0 {
            m.panic -= 1;
            // En fuite : une nouvelle direction de temps en temps
            if m.target.is_none() || self.mob_rng.below(20) == 0 {
                let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                m.target = Some((m.pos.0 + a.cos() * 8.0, m.pos.2 + a.sin() * 8.0));
            }
        } else if m.target.is_none() && self.mob_rng.below(120) == 0 {
            let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
            let d = 2.0 + self.mob_rng.unit() as f32 * 8.0;
            m.target = Some((m.pos.0 + a.cos() * d, m.pos.2 + a.sin() * d));
        }
        match m.target {
            Some((tx, tz)) => {
                let (dx, dz) = (tx - m.pos.0, tz - m.pos.2);
                let d = (dx * dx + dz * dz).sqrt();
                if d < 0.4 {
                    m.target = None;
                    m.vel.0 = 0.0;
                    m.vel.2 = 0.0;
                } else {
                    m.vel.0 = dx / d * speed;
                    m.vel.2 = dz / d * speed;
                    // Orientation du jeu : 0 vers le sud (+z), 90 vers l'ouest (-x)
                    let want = (-dx).atan2(dz).to_degrees();
                    let diff = (want - m.yaw + 540.0).rem_euclid(360.0) - 180.0;
                    m.yaw = (m.yaw + diff.clamp(-20.0, 20.0)).rem_euclid(360.0);
                }
            }
            None => {
                m.vel.0 = 0.0;
                m.vel.2 = 0.0;
            }
        }
    }

    /// Gravité, nage et collisions avec les blocs (marches d'un bloc sautées, vide évité).
    fn mob_physics(&mut self, m: &mut Mob) {
        let (w, h) = m.species.size();
        let half = w / 2.0;
        let in_water = self.mob_water(m.pos.0, m.pos.1 + 0.2, m.pos.2);
        // Vertical
        if in_water {
            m.vel.1 = (m.vel.1 + 0.02).min(0.08);
        } else {
            m.vel.1 = (m.vel.1 - 0.08) * 0.98;
            if m.species == Species::Chicken && !m.on_ground {
                m.vel.1 = m.vel.1.max(-0.1); // les poules battent des ailes
            }
        }
        // Le bloc occupé par un point du corps est-il libre ?
        let body_free = |s: &Self, x: f32, y: f32, z: f32| -> bool {
            let mut yy = y;
            while yy < y + h {
                if !s.mob_free(x.floor() as i32, yy.floor() as i32, z.floor() as i32) {
                    return false;
                }
                yy += 1.0;
            }
            s.mob_free(x.floor() as i32, (y + h - 0.01).floor() as i32, z.floor() as i32)
        };
        // Horizontal, axe par axe : le bord du corps dans le sens du déplacement
        for axis in 0..2 {
            let v = if axis == 0 { m.vel.0 } else { m.vel.2 };
            if v == 0.0 {
                continue;
            }
            let edge = v.signum() * half;
            let (nx, nz) = if axis == 0 { (m.pos.0 + v, m.pos.2) } else { (m.pos.0, m.pos.2 + v) };
            let (ex, ez) = if axis == 0 { (nx + edge, nz) } else { (nx, nz + edge) };
            let ok = body_free(self, ex, m.pos.1 + 0.01, ez);
            if ok {
                // Pas de chute de plus de 3 blocs (sauf en fuite ou dans l'eau)
                let ground_below = (1..=4).any(|d| !self.mob_free(ex.floor() as i32, m.pos.1.floor() as i32 - d, ez.floor() as i32));
                if !ground_below && m.panic == 0 && m.knock == 0 && !in_water {
                    m.target = None;
                    continue;
                }
                if axis == 0 {
                    m.pos.0 = nx;
                } else {
                    m.pos.2 = nz;
                }
            } else if m.on_ground && body_free(self, ex, m.pos.1.floor() + 1.01, ez) {
                // Marche d'un bloc : saute
                m.vel.1 = 0.42;
                m.on_ground = false;
            } else if in_water {
                m.vel.1 = 0.1; // sort de l'eau en grimpant au bord
            } else {
                m.target = None;
            }
        }
        // Vertical : atterrissage et plafond
        let ny = m.pos.1 + m.vel.1;
        if m.vel.1 <= 0.0 {
            let feet = ny.floor() as i32;
            if !self.mob_free(m.pos.0.floor() as i32, feet, m.pos.2.floor() as i32) && !in_water {
                m.pos.1 = (feet + 1) as f32;
                m.vel.1 = 0.0;
                m.on_ground = true;
            } else {
                m.pos.1 = ny;
                m.on_ground = false;
            }
        } else if self.mob_free(m.pos.0.floor() as i32, (ny + h).floor() as i32, m.pos.2.floor() as i32) {
            m.pos.1 = ny;
            m.on_ground = false;
        } else {
            m.vel.1 = 0.0;
        }
        // Coincé dans un bloc (bloc posé dessus) : remonte
        if !self.mob_free(m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32) {
            m.pos.1 = m.pos.1.floor() + 1.0;
            m.vel.1 = 0.0;
        }
        if m.pos.1 < crate::world::MIN_Y as f32 - 16.0 {
            m.health = 0.0;
            m.dead = Some(self.tick);
        }
    }

    /// Coup porté par un joueur à une créature. Vrai si la cible est une créature.
    pub(super) fn hit_mob(&mut self, id: i64, from: (f32, f32, f32), damage: f32) -> bool {
        let tick = self.tick;
        let Some(m) = self.mobs.get_mut(&id) else { return false };
        if m.dead.is_some() || tick.saturating_sub(m.hurt_tick) < 10 {
            return true;
        }
        let (dx, dz) = (m.pos.0 - from.0, m.pos.2 - from.2);
        if dx * dx + dz * dz > 6.0 * 6.0 {
            return true;
        }
        m.hurt_tick = tick;
        m.health -= damage;
        let flat = (dx * dx + dz * dz).sqrt().max(0.01);
        m.vel = (dx / flat * 0.4, 0.36, dz / flat * 0.4);
        m.knock = 8;
        m.on_ground = false;
        m.panic = 100;
        m.target = None;
        let dead = m.health <= 0.0;
        if dead {
            m.dead = Some(tick);
        }
        let (pos, species) = (m.pos, m.species);
        self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_HURT, 0));
        if dead {
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_DEATH, 0));
            self.mob_loot(species, pos, id);
        }
        true
    }

    /// Butin d'une créature morte, lâché sur place.
    fn mob_loot(&mut self, species: Species, pos: (f32, f32, f32), id: i64) {
        let color = self.mobs.get(&id).map(|m| m.color).unwrap_or(0);
        for &(name, lo, hi) in species.drops() {
            let count = lo + self.mob_rng.below((hi - lo) as u32 + 1) as u8;
            // La laine prend la couleur du mouton
            let name = if name == "minecraft:white_wool" {
                const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
                format!("minecraft:{}_wool", COLORS[color as usize & 15])
            } else {
                name.to_string()
            };
            let Some(item) = self.data.as_ref().and_then(|d| d.item(&name)).map(|i| i.id) else { continue };
            let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.2, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
            self.drop_item((pos.0, pos.1 + 0.5, pos.2), vel, item, count, 10);
        }
    }
}

/// Générateur de hasard des créatures, d'après l'heure de démarrage.
pub(super) fn mob_rng() -> Rng {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    Rng::new(seed)
}
