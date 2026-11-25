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
    /// Potion lancée : (variante, part de durée gardée). Elle éclabousse en tombant.
    pub potion: Option<(u16, f32)>,
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
        self.shots.insert(id, Shot { kind, pos: from, vel, gravity, damage, power, owner, archer: None, potion: None, cause, age: 0 });
        Some(id)
    }

    /// Une créature tire sur un point : la flèche part de ses yeux, un peu relevée pour
    /// compenser sa chute, avec un rien d'imprécision.
    pub(super) fn mob_shoots(&mut self, m: &mobs::Mob, kind: &str, at: (f32, f32, f32), damage: f32, cause: &'static str) -> Option<i64> {
        let (_, h) = m.size();
        let from = (m.pos.0, m.pos.1 + h * 0.85, m.pos.2);
        let (dx, dy, dz) = (at.0 - from.0, at.1 - from.1, at.2 - from.2);
        let flat = (dx * dx + dz * dz).sqrt().max(0.01);
        // Une boule de feu file droit ; le reste retombe
        let straight = kind.contains("fireball");
        // (une potion se lance moins fort qu'une flèche, en cloche)
        let lobbed = kind.contains("potion");
        // (les forces des fiches du jeu : petite boule de feu 1,3, grosse 1,6, potion 0,75, flèche 1,6)
        let speed = if straight { if kind.contains("small") { 1.3 } else { 1.6 } } else if lobbed { 0.75 } else { 1.6 };
        // (visée relevée d'un cinquième de la distance, comme dans le jeu, potion ou flèche)
        let lift = if straight { 0.0 } else { flat * 0.2 };
        let d = (dx * dx + (dy + lift) * (dy + lift) + dz * dz).sqrt().max(0.01);
        let wobble = |s: &mut Self| (s.mob_rng.unit() as f32 - 0.5) * 0.06;
        let vel = (dx / d * speed + wobble(self), (dy + lift) / d * speed + wobble(self), dz / d * speed + wobble(self));
        let start = (from.0 + vel.0 * 0.5, from.1, from.2 + vel.2 * 0.5);
        // (la grosse boule de feu du ghast explose ; la petite du blaze brûle sans exploser)
        let power = if straight && !kind.contains("small") { 1.0 } else { 0.0 };
        let id = self.shoot(kind, start, vel, damage, if straight { 0.0 } else { 0.05 }, power, Some(m.id), cause);
        // Le bruit du tir : celui de l'espèce pour une boule de feu (le souffle du blaze, le
        // cri du ghast), celui de l'arc ou du lancer sinon
        let sound = if straight { bedrock::level_sound("shoot", from, m.sound_id(), false, -1) } else { bedrock::level_sound(if lobbed { "throw" } else { "bow" }, from, "", false, -1) };
        self.send_near(from, bedrock::ID_LEVEL_SOUND_EVENT, &sound);
        // (le ghast tire de loin : le jeu annonce sa boule de feu par un bruit à part, qui porte)
        if m.sound_id() == "minecraft:ghast" {
            self.send_near(from, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_SOUND_GHAST_FIREBALL, from, 0));
        }
        id
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
        // Arc enchanté : puissance (un quart de dégâts en plus par niveau, plus un quart) et
        // infinité (la flèche n'est pas dépensée, mais il en faut une)
        let bow = c.inv.held().ench;
        let (power, endless) = (bow.level(crate::item::enchant::POWER) as f32, bow.level(crate::item::enchant::INFINITY) > 0);
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
                Some(_) if endless => {}
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
        if let Some(id) = self.shoot("minecraft:arrow", from, (dir.0 * speed, dir.1 * speed, dir.2 * speed), (speed * 2.0 * if power > 0.0 { 1.25 + 0.25 * power } else { 1.0 }).ceil(), 0.05, 0.0, None, "une flèche") {
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

    /// Clic avec une potion jetable en main : elle part là où le joueur regarde. Vrai si
    /// l'objet tenu était une potion à lancer.
    pub(super) fn throw_potion(&mut self, addr: &SocketAddr) -> bool {
        let tick = self.tick;
        let Some(c) = self.clients.get(addr).filter(|c| c.joined && c.dead.is_none() && !c.mode.is_spectator()) else { return false };
        let held = c.inv.held();
        let name = self.held_name(c);
        let Some(share) = crate::item::potion::thrown_share(&name) else { return false };
        // (le jeu répète le clic tant que le bouton est tenu : une potion par quart de seconde au plus)
        if tick.saturating_sub(c.threw_at) < 5 {
            return true;
        }
        let (eye, yaw, pitch, survival, variant) = (c.pos, c.yaw.to_radians(), c.pitch.to_radians(), c.mode.takes_damage(), held.meta);
        // Lancée un peu au-dessus du regard, comme dans le jeu
        let dir = (-yaw.sin() * pitch.cos(), -(pitch - 0.35).sin(), yaw.cos() * pitch.cos());
        // (la force du lancer ne dépend pas de l'angle : 0,5 bloc par tick, comme dans le jeu)
        let length = (dir.0 * dir.0 + dir.1 * dir.1 + dir.2 * dir.2).sqrt().max(0.01);
        let dir = (dir.0 / length, dir.1 / length, dir.2 / length);
        let from = (eye.0 + dir.0 * 0.5, eye.1 - 0.1, eye.2 + dir.2 * 0.5);
        let Some(id) = self.shoot("minecraft:splash_potion", from, (dir.0 * 0.5, dir.1 * 0.5, dir.2 * 0.5), 0.0, 0.05, 0.0, None, "une potion") else { return true };
        if let Some(s) = self.shots.get_mut(&id) {
            s.archer = Some(*addr);
            s.potion = Some((variant, share));
        }
        self.send_near(eye, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("throw", eye, "", false, -1));
        if let Some(c) = self.clients.get_mut(addr) {
            c.threw_at = tick;
            if survival {
                c.inv.consume_held();
                c.inv_touched = true;
            }
        }
        if survival {
            self.send_inventory(addr);
            self.show_held(addr);
        }
        true
    }

    /// Une potion lancée se brise : ses effets touchent les joueurs à moins de quatre blocs,
    /// d'autant plus longtemps qu'ils sont près (en entier pour celui qu'elle a frappé).
    fn splash(&mut self, at: (f32, f32, f32), variant: u16, share: f32, direct: Option<SocketAddr>, owner: Option<i64>) {
        self.send_near(at, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_PARTICLE_SPLASH, at, crate::item::potion::potion_color(variant)));
        self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("glass", at, "", false, -1));
        let doses = crate::item::potion::potion_effects(variant);
        if doses.is_empty() {
            return;
        }
        let near: Vec<(SocketAddr, f32)> = self
            .clients
            .iter()
            .filter(|(_, c)| c.joined && c.stage == Stage::InGame && c.dead.is_none() && !c.mode.is_spectator())
            .filter_map(|(a, c)| {
                let d = ((c.pos.0 - at.0).powi(2) + (c.pos.1 - EYE + 0.9 - at.1).powi(2) + (c.pos.2 - at.2).powi(2)).sqrt();
                let strength = if Some(*a) == direct { 1.0 } else { 1.0 - d / 4.0 };
                (strength > 0.0).then_some((*a, strength))
            })
            .collect();
        for (addr, strength) in near {
            for (effect, level, ticks) in doses {
                if *ticks <= 1 {
                    // Instantané : il agit à partir d'une demi-portée
                    if strength >= 0.5 {
                        self.give_effect(&addr, *effect, *level, 1, true);
                    }
                } else {
                    let ticks = (*ticks as f32 * share * strength) as u64;
                    if ticks >= 20 {
                        self.give_effect(&addr, *effect, *level, ticks, true);
                    }
                }
            }
        }
        // Les créatures ne connaissent que les dégâts instantanés
        if let Some((_, level, _)) = doses.iter().find(|d| d.0 == 7) {
            let hit: Vec<i64> = self.mobs.values().filter(|m| m.dead.is_none() && Some(m.id) != owner && (m.pos.0 - at.0).powi(2) + (m.pos.1 - at.1).powi(2) + (m.pos.2 - at.2).powi(2) < 9.0).map(|m| m.id).collect();
            for id in hit {
                self.hit_mob(id, at, (6i32 << (*level).min(5)) as f32, Attacker::Mob(owner.unwrap_or(id)));
            }
        }
    }

    /// Un joueur frappe une boule de feu : elle repart là où il regarde, et c'est désormais
    /// la sienne (elle peut tuer le ghast qui l'a crachée). Vrai si c'en était une.
    pub(super) fn deflect(&mut self, addr: &SocketAddr, id: i64) -> bool {
        let Some(c) = self.clients.get(addr).filter(|c| c.joined && c.dead.is_none() && !c.mode.is_spectator()) else { return false };
        let (yaw, pitch) = (c.yaw.to_radians(), c.pitch.to_radians());
        let Some(s) = self.shots.get_mut(&id).filter(|s| s.kind.contains("fireball")) else { return false };
        let speed = (s.vel.0 * s.vel.0 + s.vel.1 * s.vel.1 + s.vel.2 * s.vel.2).sqrt().max(0.9);
        s.vel = (-yaw.sin() * pitch.cos() * speed, -pitch.sin() * speed, yaw.cos() * pitch.cos() * speed);
        s.owner = None;
        s.archer = Some(*addr);
        s.age = 0;
        true
    }

    /// Fait voler les projectiles d'un tick.
    pub(super) fn shots_tick(&mut self) {
        if self.shots.is_empty() {
            return;
        }
        let ids: Vec<i64> = self.shots.keys().copied().collect();
        // Qui se trouve où (cases de deux blocs) : un projectile ne regarde que ses voisins,
        // pas les mille joueurs et les milliers de créatures du serveur
        let crowd = self.crowd();
        let players: HashMap<i64, SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|(a, c)| (c.id, *a)).collect();
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
                if let Some(target) = self.shot_touches(pos, owner, archer, &crowd, &players) {
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
    #[allow(clippy::type_complexity)]
    fn shot_touches(&self, p: (f32, f32, f32), owner: Option<i64>, archer: Option<SocketAddr>, crowd: &HashMap<(i32, i32), Vec<(i64, (f32, f32, f32), f32, f32)>>, players: &HashMap<i64, SocketAddr>) -> Option<Touched> {
        let reach = 0.3;
        let (cx, cz) = ((p.0 / 2.0).floor() as i32, (p.2 / 2.0).floor() as i32);
        // (les cases voisines aussi : la plus grosse créature, le ghast, déborde de deux blocs)
        for dx in -2..=2 {
            for dz in -2..=2 {
                let Some(bodies) = crowd.get(&(cx + dx, cz + dz)) else { continue };
                for (id, feet, w, h) in bodies {
                    if (feet.0 - p.0).abs() >= w / 2.0 + reach || (feet.2 - p.2).abs() >= w / 2.0 + reach || p.1 <= feet.1 - reach || p.1 >= feet.1 + h + reach {
                        continue;
                    }
                    match players.get(id) {
                        Some(addr) if Some(*addr) != archer => return Some(Touched::Player(*addr)),
                        Some(_) => {}
                        // (ni joueur vulnérable ni créature : un joueur en créatif, que le projectile traverse)
                        None if self.mobs.contains_key(id) && Some(*id) != owner => return Some(Touched::Mob(*id)),
                        None => {}
                    }
                }
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
        let (kind, damage, power, owner, cause, vel, archer, potion) = (s.kind.clone(), s.damage, s.power, s.owner, s.cause, s.vel, s.archer, s.potion);
        if self.plugins.listens("ProjectileHitEvent") {
            let mut fields = vec![
                ("projectile", Json::Str(kind.clone())),
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
        if let Some((variant, share)) = potion {
            let direct = match target {
                Touched::Player(a) => Some(a),
                _ => None,
            };
            return self.splash(at, variant, share, direct, owner);
        }
        if power > 0.0 {
            self.explode(at, power, owner, cause);
            return;
        }
        // (le bruit de la flèche qui se plante : pour les flèches seulement)
        if kind.contains("arrow") {
            self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("bow.hit", at, "", false, -1));
        }
        match target {
            Touched::Player(a) if damage > 0.0 => {
                let pid = self.clients.get(&a).map(|c| c.id).unwrap_or(-1);
                // (tirée par un joueur : c'est lui que nomme le message de mort)
                let by = archer.and_then(|s| self.clients.get(&s)).and_then(|c| c.name.clone());
                self.hurt(&a, damage, by.map(Cause::Player).unwrap_or(Cause::Mob(cause)));
                // La petite boule de feu du blaze enflamme sa cible cinq secondes
                if kind.contains("small_fireball") {
                    self.ignite(&a, 100);
                }
                // La flèche pousse un peu dans son sens
                let flat = (vel.0 * vel.0 + vel.2 * vel.2).sqrt().max(0.01);
                if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                    self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, (vel.0 / flat * 0.3, 0.3, vel.2 / flat * 0.3)));
                }
            }
            Touched::Mob(m) if damage > 0.0 => {
                let from = (at.0 - vel.0, at.1, at.2 - vel.2);
                self.hit_mob(m, from, damage, archer.map(Attacker::Player).unwrap_or(Attacker::Mob(owner.unwrap_or(m))));
                if kind.contains("small_fireball") {
                    self.set_mob_fire(m, self.tick + 100);
                }
            }
            // Contre un bloc, la petite boule de feu y met le feu
            Touched::Block if kind.contains("small_fireball") && self.cfg.mob_griefing => {
                let cell = ((at.0 - vel.0 * 0.3).floor() as i32, (at.1 - vel.1 * 0.3).floor() as i32, (at.2 - vel.2 * 0.3).floor() as i32);
                self.light_fire(cell);
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
        self.explode_with(at, power, owner, cause, false);
    }

    /// `every_drop` : chaque bloc soufflé laisse son butin (la TNT), au lieu d'une partie.
    pub(super) fn explode_with(&mut self, at: (f32, f32, f32), power: f32, owner: Option<i64>, cause: &'static str, every_drop: bool) {
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
            let air = self.world.air;
            // Comme dans le jeu : 1352 rayons partent du centre vers la surface d'un cube. Chacun
            // a une force tirée au sort, qui s'use à chaque pas et dans chaque bloc traversé
            // selon sa résistance ; tant qu'il lui en reste, le bloc cède. Un mur solide abrite
            // donc ce qui est derrière lui.
            let mut broken: HashMap<(i32, i32, i32), u32> = HashMap::new();
            let mut known: HashMap<u32, (f32, bool)> = HashMap::new();
            for i in 0..16 {
                for j in 0..16 {
                    for k in 0..16 {
                        if !(i == 0 || i == 15 || j == 0 || j == 15 || k == 0 || k == 15) {
                            continue;
                        }
                        let (dx, dy, dz) = (i as f32 / 15.0 * 2.0 - 1.0, j as f32 / 15.0 * 2.0 - 1.0, k as f32 / 15.0 * 2.0 - 1.0);
                        let len = (dx * dx + dy * dy + dz * dz).sqrt();
                        let (dx, dy, dz) = (dx / len * 0.3, dy / len * 0.3, dz / len * 0.3);
                        let mut force = power * (0.7 + self.mob_rng.unit() as f32 * 0.6);
                        let (mut x, mut y, mut z) = at;
                        // (un rayon fait trois pas par bloc : le bloc n'est relu que quand il change)
                        let mut last: Option<((i32, i32, i32), u32)> = None;
                        while force > 0.0 {
                            let cell = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
                            let state = match last {
                                Some((c, s)) if c == cell => s,
                                _ => {
                                    if !crate::world::World::in_bounds(cell.1) || !self.world.is_ready(cell.0 >> 4, cell.2 >> 4) {
                                        break;
                                    }
                                    let s = self.world.get(cell.0, cell.1, cell.2);
                                    last = Some((cell, s));
                                    s
                                }
                            };
                            if state != air {
                                // (résistance du bloc, et s'il peut céder : ni liquide, ni incassable, ni conteneur)
                                let (resistance, gives) = *known.entry(state).or_insert_with(|| {
                                    let name = d.block_name(state).unwrap_or("");
                                    // (la bedrock, les barrières et leurs pareils ne cèdent jamais, quoi
                                    // qu'en disent les données ou un plugin, et arrêtent le rayon)
                                    if indestructible(name) {
                                        return (3_600_000.0, false);
                                    }
                                    let solid = !(name.contains("water") || name.contains("lava") || keeps_things(name) || d.hardness(state).map(|h| h < 0.0).unwrap_or(true));
                                    (d.block_props(name).map(|p| p.blast_resistance).unwrap_or(6.0), solid)
                                });
                                force -= (resistance + 0.3) * 0.3;
                                if force > 0.0 && gives {
                                    broken.insert(cell, state);
                                }
                            }
                            (x, y, z) = (x + dx, y + dy, z + dz);
                            force -= 0.225;
                        }
                    }
                }
            }
            // Le butin est regroupé par sorte d'objet : quelques piles au lieu de centaines
            // d'objets à faire tomber un par un
            let mut loot: HashMap<i16, (u32, (i32, i32, i32))> = HashMap::new();
            for (p, state) in broken {
                // Comme dans le jeu, une partie seulement des blocs soufflés laisse son butin
                // (une TNT laisse tout tomber, comme le veut la règle par défaut du jeu)
                if every_drop || self.mob_rng.unit() as f32 * power < 1.0 {
                    for (item, count) in self.natural_drops(state) {
                        let pile = loot.entry(item).or_insert((0, p));
                        pile.0 += count as u32;
                    }
                }
                // (une TNT soufflée s'amorce à son tour, mèche courte)
                if d.block_name(state) == Some("minecraft:tnt") {
                    let fuse = 10 + self.mob_rng.below(31);
                    self.prime_tnt(p, fuse);
                    continue;
                }
                self.change_block(p, air);
            }
            for (item, (mut count, p)) in loot {
                while count > 0 {
                    let pile = count.min(64);
                    self.drop_item((p.0 as f32 + 0.5, p.1 as f32 + 0.3, p.2 as f32 + 0.5), (0.0, 0.2, 0.0), item, pile as u8, 10);
                    count -= pile;
                }
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
/// Blocs que rien ne détruit, sinon un joueur en créatif : bedrock, barrière, blocs techniques.
pub(super) fn indestructible(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    matches!(n, "bedrock" | "barrier" | "invisible_bedrock" | "end_portal" | "end_portal_frame" | "end_gateway" | "portal" | "structure_block" | "structure_void" | "jigsaw" | "border_block" | "allow" | "deny") || n.contains("command_block") || n.starts_with("light_block")
}

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
