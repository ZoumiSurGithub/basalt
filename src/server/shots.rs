//! Projectiles (flèches, boules de neige, boules de feu…) et explosions.
//!
//! Un projectile est une petite entité que le serveur fait voler lui-même : à chaque tick il
//! avance, retombe un peu, et s'arrête sur le premier bloc plein, joueur ou créature qu'il
//! touche. Les monstres en tirent (squelette, pilleur), et les plugins aussi
//! (`server.entities.shoot`).

use super::mobs::Attacker;
use super::*;

/// Un projectile en vol.
pub(super) struct Shot {
    /// Son apparence : une entité du jeu (`minecraft:arrow`)
    pub kind: String,
    pub pos: (f32, f32, f32),
    pub vel: (f32, f32, f32),
    /// Vitesse perdue vers le bas à chaque tick (0,05 pour une flèche, 0 pour une boule de feu)
    pub gravity: f32,
    /// Points de vie retirés à ce qu'il touche
    pub damage: f32,
    /// Explosion à l'impact (0 : aucune)
    pub power: f32,
    /// Créature qui l'a tiré : il ne la touche pas
    pub owner: Option<i64>,
    /// Joueur qui l'a tiré : il ne le touche pas, et ses victimes lui sont comptées
    pub archer: Option<SocketAddr>,
    /// Son tireur dans le message de mort (« tué par un squelette »)
    pub cause: &'static str,
    age: u32,
}

/// Nombre de projectiles en vol à la fois
const MAX_SHOTS: usize = 512;
/// Durée de vol au plus (dix secondes)
const SHOT_LIFE: u32 = 200;

/// Dégâts d'une explosion à cette distance, selon la formule du jeu (sans abri).
pub(super) fn blast_damage(power: f32, distance: f32) -> f32 {
    let reach = power * 2.0;
    if distance >= reach {
        return 0.0;
    }
    let f = 1.0 - distance / reach;
    (f * f + f) / 2.0 * 7.0 * reach + 1.0
}

/// Un bloc de cette résistance cède-t-il à cette distance ? `luck` (de 0 à 1) fait le bord
/// irrégulier du cratère, comme dans le jeu.
pub(super) fn blast_breaks(power: f32, distance: f32, resistance: f32, luck: f32) -> bool {
    power * (0.7 + luck * 0.6) - 0.75 * distance - (resistance + 0.3) * 0.3 > 0.0
}

impl Server {
    /// Lance un projectile. Renvoie son identifiant, ou None si le ciel en est déjà plein.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn shoot(&mut self, kind: &str, from: (f32, f32, f32), vel: (f32, f32, f32), damage: f32, gravity: f32, power: f32, owner: Option<i64>, cause: &'static str) -> Option<i64> {
        if self.shots.len() >= MAX_SHOTS || ![from.0, from.1, from.2, vel.0, vel.1, vel.2].iter().all(|v| v.is_finite()) {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let kind = if kind.contains(':') { kind.to_string() } else { format!("minecraft:{}", kind) };
        let (yaw, _) = heading(vel);
        let meta = bedrock::mob_metadata(0.25, 0.25, 0, 0, 0);
        self.send_near(from, bedrock::ID_ADD_ACTOR, &bedrock::add_actor(id, &kind, from, vel, yaw, (1.0, 1.0), None, None, &meta));
        self.shots.insert(id, Shot { kind, pos: from, vel, gravity, damage, power, owner, archer: None, cause, age: 0 });
        Some(id)
    }

    /// Une créature tire sur un point : la flèche part de ses yeux, un peu relevée pour
    /// compenser sa chute, avec un rien d'imprécision.
    pub(super) fn mob_shoots(&mut self, m: &mobs::Mob, kind: &str, at: (f32, f32, f32), damage: f32, cause: &'static str) {
        let (_, h) = m.size();
        let from = (m.pos.0, m.pos.1 + h * 0.85, m.pos.2);
        let (dx, dy, dz) = (at.0 - from.0, at.1 - from.1, at.2 - from.2);
        let flat = (dx * dx + dz * dz).sqrt().max(0.01);
        // Une boule de feu file droit ; le reste retombe
        let straight = kind.contains("fireball");
        let speed = if straight { 0.9 } else { 1.6 };
        let lift = if straight { 0.0 } else { flat * 0.2 };
        let d = (dx * dx + (dy + lift) * (dy + lift) + dz * dz).sqrt().max(0.01);
        let wobble = |s: &mut Self| (s.mob_rng.unit() as f32 - 0.5) * 0.06;
        let vel = (dx / d * speed + wobble(self), (dy + lift) / d * speed + wobble(self), dz / d * speed + wobble(self));
        let start = (from.0 + vel.0 * 0.5, from.1, from.2 + vel.2 * 0.5);
        let power = if straight { 1.0 } else { 0.0 };
        self.shoot(kind, start, vel, damage, if straight { 0.0 } else { 0.05 }, power, Some(m.id), cause);
        self.send_near(from, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("bow", from, "", false, -1));
    }

    /// Clic avec un arc en main : le joueur commence à le bander (s'il a une flèche, ou en
    /// créatif). Vrai si l'objet tenu est un arc.
    pub(super) fn draw_bow(&mut self, addr: &SocketAddr) -> bool {
        let tick = self.tick;
        let Some(c) = self.clients.get(addr) else { return false };
        if self.held_name(c) != "bow" {
            return false;
        }
        let arrow = self.item_id("arrow");
        let loaded = !c.mode.takes_damage() || c.inv.main.slots.iter().chain(c.inv.offhand.slots.iter()).any(|s| !s.is_empty() && Some(s.id) == arrow);
        if let Some(c) = self.clients.get_mut(addr) {
            // (le jeu répète le clic tant que le bouton est tenu : seul le premier compte)
            if loaded && c.drawing.is_none() {
                c.drawing = Some(tick);
            }
        }
        true
    }

    /// Bouton relâché : la flèche part, d'autant plus vite que l'arc a été bandé longtemps
    /// (une seconde pour la pleine puissance). Vrai si un arc était bandé.
    pub(super) fn release_bow(&mut self, addr: &SocketAddr) -> bool {
        let tick = self.tick;
        let arrow = self.item_id("arrow");
        let Some(c) = self.clients.get_mut(addr) else { return false };
        let Some(since) = c.drawing.take() else { return false };
        let (eye, yaw, pitch, survival) = (c.pos, c.yaw.to_radians(), c.pitch.to_radians(), c.mode.takes_damage());
        if c.dead.is_some() || c.mode.is_spectator() {
            return true;
        }
        // Force du tir, comme dans le jeu : (t² + 2t) / 3, t en secondes, 1 au plus
        let t = tick.saturating_sub(since) as f32 / 20.0;
        let force = ((t * t + 2.0 * t) / 3.0).min(1.0);
        if force < 0.1 {
            return true; // à peine tendu : rien ne part
        }
        // Une flèche de moins (la seconde main d'abord), sauf en créatif
        if survival {
            let slot = c.inv.offhand.slots.iter_mut().chain(c.inv.main.slots.iter_mut()).find(|s| !s.is_empty() && Some(s.id) == arrow);
            match slot {
                Some(s) => {
                    s.count -= 1;
                    if s.count == 0 {
                        *s = inventory::ItemStack::EMPTY;
                    }
                    c.inv_touched = true;
                }
                None => return true,
            }
        }
        let dir = (-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
        let speed = force * 3.0;
        let from = (eye.0 + dir.0 * 0.5, eye.1 - 0.1, eye.2 + dir.2 * 0.5);
        // Dégâts : deux fois la vitesse, soit 6 à pleine puissance (9 pour un tir critique du jeu)
        if let Some(id) = self.shoot("minecraft:arrow", from, (dir.0 * speed, dir.1 * speed, dir.2 * speed), (speed * 2.0).ceil(), 0.05, 0.0, None, "une flèche") {
            if let Some(s) = self.shots.get_mut(&id) {
                s.archer = Some(*addr);
            }
        }
        self.send_near(eye, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("bow", eye, "", false, -1));
        if survival {
            self.wear_held(addr, crate::item::Usage::Strike);
            self.send_inventory(addr);
        }
        true
    }

    /// Fait voler les projectiles d'un tick.
    pub(super) fn shots_tick(&mut self) {
        if self.shots.is_empty() {
            return;
        }
        let ids: Vec<i64> = self.shots.keys().copied().collect();
        for id in ids {
            let Some(s) = self.shots.get_mut(&id) else { continue };
            s.age += 1;
            let (from, vel, owner, archer, age) = (s.pos, s.vel, s.owner, s.archer, s.age);
            if age > SHOT_LIFE || !self.world.is_ready((from.0.floor() as i32) >> 4, (from.2.floor() as i32) >> 4) {
                self.end_shot(id);
                continue;
            }
            // Par petits pas, pour ne pas traverser un mur ou une créature d'un bond
            let speed = (vel.0 * vel.0 + vel.1 * vel.1 + vel.2 * vel.2).sqrt();
            let steps = (speed / 0.4).ceil().max(1.0) as i32;
            let mut hit = None;
            let mut pos = from;
            for _ in 0..steps {
                pos = (pos.0 + vel.0 / steps as f32, pos.1 + vel.1 / steps as f32, pos.2 + vel.2 / steps as f32);
                if let Some(target) = self.shot_touches(pos, owner, archer) {
                    hit = Some(target);
                    break;
                }
            }
            if let Some(target) = hit {
                self.shot_lands(id, pos, target);
                continue;
            }
            let Some(s) = self.shots.get_mut(&id) else { continue };
            s.pos = pos;
            s.vel = (s.vel.0 * 0.99, s.vel.1 * 0.99 - s.gravity, s.vel.2 * 0.99);
            let (yaw, pitch) = heading(s.vel);
            let packet = bedrock::move_actor_absolute(id, pos, pitch, yaw, yaw, false);
            self.send_near(pos, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
        }
    }

    /// Ce que le projectile touche à cet endroit : un joueur, une créature, ou un bloc plein.
    fn shot_touches(&self, p: (f32, f32, f32), owner: Option<i64>, archer: Option<SocketAddr>) -> Option<Touched> {
        let reach = 0.3;
        for (addr, c) in self.clients.iter() {
            if Some(*addr) == archer || !c.joined || c.stage != Stage::InGame || c.dead.is_some() || !c.mode.takes_damage() {
                continue;
            }
            let feet = c.pos.1 - EYE;
            if (c.pos.0 - p.0).abs() < 0.3 + reach && (c.pos.2 - p.2).abs() < 0.3 + reach && p.1 > feet - reach && p.1 < feet + 1.8 + reach {
                return Some(Touched::Player(*addr));
            }
        }
        for m in self.mobs.values() {
            if Some(m.id) == owner || m.dead.is_some() {
                continue;
            }
            let (w, h) = m.size();
            if (m.pos.0 - p.0).abs() < w / 2.0 + reach && (m.pos.2 - p.2).abs() < w / 2.0 + reach && p.1 > m.pos.1 - reach && p.1 < m.pos.1 + h + reach {
                return Some(Touched::Mob(m.id));
            }
        }
        let (x, y, z) = (p.0.floor() as i32, p.1.floor() as i32, p.2.floor() as i32);
        if !crate::world::World::in_bounds(y) {
            return (y < crate::world::MIN_Y).then_some(Touched::Block);
        }
        let state = self.world.get(x, y, z);
        (state != self.world.generator().blocks.water && !self.npc_passable(state)).then_some(Touched::Block)
    }

    /// Le projectile a touché : dégâts, explosion, puis il disparaît.
    fn shot_lands(&mut self, id: i64, at: (f32, f32, f32), target: Touched) {
        let Some(s) = self.shots.get(&id) else { return };
        let (kind, damage, power, owner, cause, vel, archer) = (s.kind.clone(), s.damage, s.power, s.owner, s.cause, s.vel, s.archer);
        if self.plugins.listens("ProjectileHitEvent") {
            let mut fields = vec![
                ("projectile", Json::Str(kind)),
                ("x", Json::Num(at.0 as f64)),
                ("y", Json::Num(at.1 as f64)),
                ("z", Json::Num(at.2 as f64)),
                ("target", Json::Str(match target { Touched::Player(_) => "player", Touched::Mob(_) => "creature", Touched::Block => "block" }.into())),
            ];
            match target {
                Touched::Player(a) => fields.push(("player", self.name_json(&a))),
                Touched::Mob(m) => fields.push(("entity", self.mob_json(m))),
                Touched::Block => {}
            }
            let out = self.fire("ProjectileHitEvent", fields);
            if cancelled(&out) {
                self.end_shot(id);
                return;
            }
        }
        self.end_shot(id);
        if power > 0.0 {
            self.explode(at, power, owner, cause);
            return;
        }
        self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("bow.hit", at, "", false, -1));
        match target {
            Touched::Player(a) if damage > 0.0 => {
                let pid = self.clients.get(&a).map(|c| c.id).unwrap_or(-1);
                // (tirée par un joueur : c'est lui que nomme le message de mort)
                let by = archer.and_then(|s| self.clients.get(&s)).and_then(|c| c.name.clone());
                self.hurt(&a, damage, by.map(Cause::Player).unwrap_or(Cause::Mob(cause)));
                // La flèche pousse un peu dans son sens
                let flat = (vel.0 * vel.0 + vel.2 * vel.2).sqrt().max(0.01);
                if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                    self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, (vel.0 / flat * 0.3, 0.3, vel.2 / flat * 0.3)));
                }
            }
            Touched::Mob(m) if damage > 0.0 => {
                let from = (at.0 - vel.0, at.1, at.2 - vel.2);
                self.hit_mob(m, from, damage, archer.map(Attacker::Player).unwrap_or(Attacker::Mob(owner.unwrap_or(m))));
            }
            _ => {}
        }
    }

    /// Retire un projectile, ici et chez les joueurs.
    fn end_shot(&mut self, id: i64) {
        if let Some(s) = self.shots.remove(&id) {
            // (plus large que la portée d'envoi : il a pu voler depuis son apparition)
            let to: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && (c.pos.0 - s.pos.0).powi(2) + (c.pos.2 - s.pos.2).powi(2) < 128.0 * 128.0).map(|(a, _)| *a).collect();
            for a in to {
                self.send(&a, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
            }
        }
    }

    /// Explosion : elle creuse (sauf `mob-griefing=false`), blesse et repousse ce qui est
    /// autour. `power` : 3 pour un creeper, 4 pour de la TNT.
    pub(super) fn explode(&mut self, at: (f32, f32, f32), power: f32, owner: Option<i64>, cause: &'static str) {
        let mut power = power.clamp(0.1, 16.0);
        if self.plugins.listens("ExplosionEvent") {
            let out = self.fire("ExplosionEvent", vec![("x", Json::Num(at.0 as f64)), ("y", Json::Num(at.1 as f64)), ("z", Json::Num(at.2 as f64)), ("power", Json::Num(power as f64))]);
            if cancelled(&out) {
                return;
            }
            if let Some(n) = out.as_ref().and_then(|v| v.get("power")).and_then(|n| n.as_f64()) {
                power = (n as f32).clamp(0.1, 16.0);
            }
        }
        self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("explode", at, "", false, -1));
        self.send_near(at, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_PARTICLE_EXPLOSION, at, 0));
        // Les blocs
        if self.cfg.mob_griefing {
            let Some(d) = self.data.clone() else { return };
            let (air, water) = (self.world.air, self.world.generator().blocks.water);
            let r = (power * 1.3).ceil() as i32 + 1;
            let (cx, cy, cz) = (at.0.floor() as i32, at.1.floor() as i32, at.2.floor() as i32);
            let mut broken = Vec::new();
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        let (x, y, z) = (cx + dx, cy + dy, cz + dz);
                        if !crate::world::World::in_bounds(y) || !self.world.is_ready(x >> 4, z >> 4) {
                            continue;
                        }
                        let state = self.world.get(x, y, z);
                        if state == air || state == water {
                            continue;
                        }
                        let Some(name) = d.block_name(state) else { continue };
                        // L'eau, la lave et ce qui garde des objets résistent ; l'incassable aussi
                        if name.contains("water") || name.contains("lava") || keeps_things(name) || d.hardness(state).map(|h| h < 0.0).unwrap_or(true) {
                            continue;
                        }
                        let resistance = d.block_props(name).map(|p| p.blast_resistance).unwrap_or(6.0);
                        let dist = ((x as f32 + 0.5 - at.0).powi(2) + (y as f32 + 0.5 - at.1).powi(2) + (z as f32 + 0.5 - at.2).powi(2)).sqrt();
                        if blast_breaks(power, dist, resistance, self.mob_rng.unit() as f32) {
                            broken.push(((x, y, z), state));
                        }
                    }
                }
            }
            for (p, state) in broken {
                // Comme dans le jeu, une partie seulement des blocs soufflés laisse son butin
                if self.mob_rng.unit() as f32 * power < 1.0 {
                    for (item, count) in self.natural_drops(state) {
                        self.drop_item((p.0 as f32 + 0.5, p.1 as f32 + 0.3, p.2 as f32 + 0.5), (0.0, 0.2, 0.0), item, count, 10);
                    }
                }
                self.change_block(p, air);
            }
        }
        // Les joueurs et les créatures
        let reach = power * 2.0;
        let players: Vec<(SocketAddr, i64, (f32, f32, f32))> =
            self.clients.iter().filter(|(_, c)| c.joined && c.stage == Stage::InGame && c.dead.is_none()).map(|(a, c)| (*a, c.id, (c.pos.0, c.pos.1 - EYE + 0.9, c.pos.2))).collect();
        for (addr, pid, p) in players {
            let (dx, dy, dz) = (p.0 - at.0, p.1 - at.1, p.2 - at.2);
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let damage = blast_damage(power, dist);
            if damage <= 0.0 {
                continue;
            }
            self.hurt(&addr, damage, Cause::Mob(cause));
            if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) && self.clients.get(&addr).map(|c| c.mode.takes_damage()).unwrap_or(false) {
                let push = (1.0 - dist / reach).max(0.0);
                let d = dist.max(0.1);
                self.send(&addr, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, (dx / d * push, 0.3 + push * 0.5, dz / d * push)));
            }
        }
        let mobs: Vec<(i64, (f32, f32, f32))> = self.mobs.values().filter(|m| m.dead.is_none() && Some(m.id) != owner).map(|m| (m.id, m.pos)).collect();
        for (id, p) in mobs {
            let dist = ((p.0 - at.0).powi(2) + (p.1 + 0.5 - at.1).powi(2) + (p.2 - at.2).powi(2)).sqrt();
            let damage = blast_damage(power, dist);
            if damage > 0.0 {
                // (frappée d'assez près pour que le coup porte, quelle que soit la portée)
                let from = (p.0 + (at.0 - p.0).clamp(-3.0, 3.0), at.1, p.2 + (at.2 - p.2).clamp(-3.0, 3.0));
                self.hit_mob(id, from, damage, Attacker::Mob(owner.unwrap_or(id)));
            }
        }
    }
}

/// Ce qu'un projectile vient de toucher.
#[derive(Clone, Copy)]
enum Touched {
    Player(SocketAddr),
    Mob(i64),
    Block,
}

/// Blocs qui gardent des objets : une explosion les laisse en place plutôt que de tout perdre.
fn keeps_things(name: &str) -> bool {
    ["chest", "barrel", "furnace", "smoker", "shulker", "hopper", "dispenser", "dropper", "brewing", "bed", "sign", "spawner"].iter().any(|k| name.contains(k))
}

/// Orientation d'un projectile d'après sa vitesse : (lacet, tangage), en degrés.
fn heading(vel: (f32, f32, f32)) -> (f32, f32) {
    let flat = (vel.0 * vel.0 + vel.2 * vel.2).sqrt();
    // (convention du jeu pour un projectile : à l'inverse de celle d'une créature, sans quoi
    // la flèche vole la pointe en arrière)
    (vel.0.atan2(vel.2).to_degrees(), vel.1.atan2(flat).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explosions_follow_the_game() {
        // Creeper (puissance 3) : mortel au contact, plus rien à 6 blocs
        assert!((blast_damage(3.0, 0.0) - 43.0).abs() < 0.01);
        assert!(blast_damage(3.0, 3.0) > 15.0 && blast_damage(3.0, 3.0) < 20.0);
        assert_eq!(blast_damage(3.0, 6.0), 0.0);
        // La terre cède à 3 blocs, la pierre seulement tout près, l'obsidienne jamais
        assert!(blast_breaks(3.0, 3.0, 0.5, 0.5));
        assert!(blast_breaks(3.0, 1.0, 6.0, 0.5) && !blast_breaks(3.0, 3.0, 6.0, 1.0));
        assert!(!blast_breaks(3.0, 0.0, 1200.0, 1.0));
    }

    #[test]
    fn projectiles_point_where_they_fly() {
        let (yaw, pitch) = heading((0.0, 0.0, 1.0));
        assert!(yaw.abs() < 0.01 && pitch.abs() < 0.01);
        let (yaw, _) = heading((1.0, 0.0, 0.0));
        assert!((yaw - 90.0).abs() < 0.01);
        // Une flèche qui retombe pique du nez : tangage négatif
        let (_, pitch) = heading((0.0, -1.0, 1.0));
        assert!((pitch + 45.0).abs() < 0.01);
    }
}
