//! Le feu et la TNT.
//!
//! Le feu est un bloc que le serveur suit dans une liste : deux fois par seconde, chaque
//! flamme tente de brûler ses voisins inflammables (d'après la « flammability » des données
//! du jeu), de gagner un bloc proche, ou s'éteint faute de combustible. La TNT amorcée est
//! une entité qui tombe, puis explose au bout de sa mèche.

use super::*;

/// Une TNT amorcée
pub(super) struct Tnt {
    /// Base de la TNT (son centre au sol)
    pos: (f32, f32, f32),
    vel: (f32, f32, f32),
    fuse: u32,
}

/// Flammes suivies au plus : au-delà, le feu ne gagne plus de terrain
const MAX_FIRES: usize = 2000;
/// Explosions de TNT au plus par tick : une grosse charge s'étale sur quelques ticks au lieu
/// de figer le serveur
const BLASTS_PER_TICK: usize = 4;
/// TNT amorcées au plus à la fois
const MAX_TNT: usize = 256;
/// Hauteur du centre d'une TNT amorcée au-dessus de sa base : c'est ce point que le jeu attend
const TNT_CENTRE: f32 = 0.49;
/// Mèche d'une TNT allumée à la main, en ticks (quatre secondes)
const FUSE: u32 = 80;

const AROUND: [(i32, i32, i32); 6] = [(0, 1, 0), (0, -1, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)];

impl Server {
    fn block_is(&self, state: u32, wanted: &[&str]) -> bool {
        self.data.as_ref().and_then(|d| d.block_name(state)).map(|n| wanted.contains(&n.trim_start_matches("minecraft:"))).unwrap_or(false)
    }

    pub(super) fn is_fire(&self, state: u32) -> bool {
        self.block_is(state, &["fire", "soul_fire"])
    }

    fn is_lava(&self, state: u32) -> bool {
        self.block_is(state, &["lava", "flowing_lava"])
    }

    /// Facilité à brûler du bloc d'une position (0 : il ne brûle pas).
    fn burns_at(&self, (x, y, z): (i32, i32, i32)) -> u8 {
        if !crate::world::World::in_bounds(y) || !self.world.is_ready(x >> 4, z >> 4) {
            return 0;
        }
        let state = self.world.get(x, y, z);
        self.data.as_ref().and_then(|d| d.block_name(state).and_then(|n| d.block_props(n))).map(|p| p.flammability).unwrap_or(0)
    }

    /// Un feu peut-il tenir ici ? Il lui faut un sol plein ou un voisin qui brûle.
    pub(super) fn fire_holds(&self, (x, y, z): (i32, i32, i32)) -> bool {
        self.blocks_movement(self.world.get(x, y - 1, z)) || AROUND.iter().any(|d| self.burns_at((x + d.0, y + d.1, z + d.2)) > 0)
    }

    /// Allume un feu à cette position, s'il peut y tenir : il lui faut de l'air, et un sol
    /// plein ou un voisin qui brûle. Vrai si le feu a pris.
    pub(super) fn light_fire(&mut self, (x, y, z): (i32, i32, i32)) -> bool {
        if !crate::world::World::in_bounds(y) || !self.world.is_ready(x >> 4, z >> 4) || self.fires.len() >= MAX_FIRES || self.world.get(x, y, z) != self.world.air {
            return false;
        }
        if !self.fire_holds((x, y, z)) {
            return false;
        }
        let Some(fire) = self.data.as_ref().and_then(|d| d.default_state("minecraft:fire")) else { return false };
        self.change_block((x, y, z), fire);
        self.fires.insert((x, y, z));
        true
    }

    /// Briquet ou boule de feu utilisés sur un bloc : la TNT s'amorce, sinon un feu prend
    /// contre le bloc visé, du côté du joueur. Vrai si l'objet tenu était l'un des deux.
    pub(super) fn use_lighter(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        let Some(c) = self.clients.get(addr) else { return false };
        let held = self.held_name(c);
        if held != "flint_and_steel" && held != "fire_charge" {
            return false;
        }
        let (eye, survival) = (c.pos, c.mode.takes_damage());
        let lit = if self.block_is(self.world.get(pos.0, pos.1, pos.2), &["tnt"]) {
            self.prime_tnt(pos, FUSE)
        } else {
            // (le voisin libre le plus proche des yeux du joueur : la face qu'il regarde)
            let mut spots: Vec<(i32, i32, i32)> = AROUND.iter().map(|d| (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2)).collect();
            spots.sort_by(|a, b| {
                let far = |p: &(i32, i32, i32)| (p.0 as f32 + 0.5 - eye.0).powi(2) + (p.1 as f32 + 0.5 - eye.1).powi(2) + (p.2 as f32 + 0.5 - eye.2).powi(2);
                far(a).total_cmp(&far(b))
            });
            // (d'abord là où le feu a un sol : le jeu ne dessine pas les flammes d'un feu en l'air)
            let grounded: Vec<(i32, i32, i32)> = spots.iter().copied().filter(|p| self.blocks_movement(self.world.get(p.0, p.1 - 1, p.2))).collect();
            grounded.into_iter().chain(spots).any(|p| self.light_fire(p))
        };
        if lit {
            let at = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
            self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("ignite", at, "", false, -1));
            if survival {
                if held == "fire_charge" {
                    if let Some(c) = self.clients.get_mut(addr) {
                        c.inv.consume_held();
                        c.inv_touched = true;
                    }
                    self.send_inventory(addr);
                    self.show_held(addr);
                } else {
                    self.wear_held(addr, crate::item::Usage::Mine);
                }
            }
        }
        true
    }

    /// Deux fois par seconde : chaque flamme brûle, gagne du terrain ou s'éteint.
    pub(super) fn fires_tick(&mut self) {
        if self.fires.is_empty() {
            return;
        }
        let air = self.world.air;
        let spreads = self.cfg.fire_spreads;
        let list: Vec<(i32, i32, i32)> = self.fires.iter().copied().collect();
        for (x, y, z) in list {
            if !self.world.is_ready(x >> 4, z >> 4) {
                continue;
            }
            if !self.is_fire(self.world.get(x, y, z)) {
                self.fires.remove(&(x, y, z));
                continue;
            }
            let below = self.world.get(x, y - 1, z);
            // (sur de la netherrack ou du magma, le feu ne s'éteint jamais)
            let eternal = self.block_is(below, &["netherrack", "magma"]);
            let fuel: Vec<((i32, i32, i32), u8)> = AROUND.iter().map(|d| (x + d.0, y + d.1, z + d.2)).map(|p| (p, self.burns_at(p))).filter(|(_, f)| *f > 0).collect();
            let dies = if eternal {
                false
            } else if fuel.is_empty() {
                !self.blocks_movement(below) || self.mob_rng.below(4) == 0
            } else {
                self.mob_rng.below(60) == 0
            };
            if dies {
                self.fires.remove(&(x, y, z));
                self.change_block((x, y, z), air);
                continue;
            }
            if !spreads {
                continue;
            }
            // Ce qui touche la flamme finit par brûler : plus vite s'il est très inflammable
            for (p, ease) in fuel {
                if self.mob_rng.below(400) >= ease as u32 {
                    continue;
                }
                if self.block_is(self.world.get(p.0, p.1, p.2), &["tnt"]) {
                    self.prime_tnt(p, FUSE);
                    continue;
                }
                // (jamais un bloc indestructible, même si un plugin l'a rendu inflammable)
                if self.data.as_ref().and_then(|d| d.block_name(self.world.get(p.0, p.1, p.2))).map(shots::indestructible).unwrap_or(true) {
                    continue;
                }
                self.change_block(p, air);
                if self.mob_rng.below(10) < 7 {
                    self.light_fire(p);
                }
            }
            // Et le feu saute sur un bloc voisin qui peut brûler
            for _ in 0..2 {
                let p = (x + self.mob_rng.below(3) as i32 - 1, y + self.mob_rng.below(5) as i32 - 1, z + self.mob_rng.below(3) as i32 - 1);
                if self.mob_rng.below(10) < 3 && AROUND.iter().any(|d| self.burns_at((p.0 + d.0, p.1 + d.1, p.2 + d.2)) > 0) {
                    self.light_fire(p);
                }
            }
        }
    }

    /// La lave met le feu à ce qui brûle au-dessus et autour d'elle (appelé au hasard).
    pub(super) fn lava_sparks(&mut self, (x, y, z): (i32, i32, i32)) {
        if !self.cfg.fire_spreads {
            return;
        }
        let p = (x + self.mob_rng.below(3) as i32 - 1, y + 1 + self.mob_rng.below(2) as i32, z + self.mob_rng.below(3) as i32 - 1);
        if AROUND.iter().any(|d| self.burns_at((p.0 + d.0, p.1 + d.1, p.2 + d.2)) > 0) {
            self.light_fire(p);
        }
    }

    /// Amorce la TNT de cette position : le bloc devient une entité qui clignote, tombe et
    /// explose à la fin de sa mèche. Vrai si elle est partie.
    pub(super) fn prime_tnt(&mut self, pos: (i32, i32, i32), fuse: u32) -> bool {
        if self.tnt.len() >= MAX_TNT || !self.block_is(self.world.get(pos.0, pos.1, pos.2), &["tnt"]) {
            return false;
        }
        self.change_block(pos, self.world.air);
        let id = self.next_id;
        self.next_id += 1;
        let at = (pos.0 as f32 + 0.5, pos.1 as f32, pos.2 as f32 + 0.5);
        // (la mèche remplace la valeur par défaut des métadonnées, elle ne s'y ajoute pas)
        let mut meta = bedrock::mob_metadata(0.98, 0.98, 0, 0, 1 << 10);
        meta.retain(|(key, _)| *key != 55);
        meta.push((55, bedrock::Meta::Int(fuse as i32)));
        // Le jeu place une TNT amorcée par son centre, pas par sa base
        self.send_near(at, bedrock::ID_ADD_ACTOR, &bedrock::add_actor(id, "minecraft:tnt", (at.0, at.1 + TNT_CENTRE, at.2), (0.0, 0.0, 0.0), 0.0, (1.0, 1.0), None, None, &meta));
        // Le sifflement de la mèche
        self.send_near(at, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_SOUND_FUSE, at, 0));
        // Comme dans le jeu : un petit saut, dans une direction au hasard
        let angle = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
        self.tnt.insert(id, Tnt { pos: at, vel: (-angle.sin() * 0.02, 0.2, -angle.cos() * 0.02), fuse });
        true
    }

    /// Chaque tick : les TNT amorcées tombent, leur mèche brûle, elles explosent.
    pub(super) fn tnt_tick(&mut self) {
        if self.tnt.is_empty() {
            return;
        }
        let ids: Vec<i64> = self.tnt.keys().copied().collect();
        let mut blasts = 0;
        for id in ids {
            let Some(t) = self.tnt.get(&id) else { continue };
            let (mut pos, mut vel, fuse) = (t.pos, (t.vel.0, t.vel.1 - 0.04, t.vel.2), t.fuse.saturating_sub(1));
            let ready = self.world.is_ready((pos.0.floor() as i32) >> 4, (pos.2.floor() as i32) >> 4);
            let solid = |s: &Self, x: f32, y: f32, z: f32| s.blocks_movement(s.world.get(x.floor() as i32, y.floor() as i32, z.floor() as i32));
            if ready {
                // De côté d'abord (un mur l'arrête), puis de haut en bas
                if !solid(self, pos.0 + vel.0, pos.1 + 0.1, pos.2 + vel.2) {
                    pos.0 += vel.0;
                    pos.2 += vel.2;
                } else {
                    (vel.0, vel.2) = (0.0, 0.0);
                }
                let next = pos.1 + vel.1;
                if vel.1 < 0.0 && solid(self, pos.0, next, pos.2) {
                    // Au sol : elle rebondit à peine et le frottement l'arrête
                    pos.1 = next.floor() + 1.0;
                    vel = (vel.0 * 0.7, -vel.1 * 0.5, vel.2 * 0.7);
                    if vel.1 < 0.1 {
                        vel.1 = 0.0;
                    }
                } else if vel.1 > 0.0 && solid(self, pos.0, next + 0.98, pos.2) {
                    vel.1 = 0.0;
                } else {
                    pos.1 = next;
                }
                vel = (vel.0 * 0.98, vel.1 * 0.98, vel.2 * 0.98);
            }
            // (trop d'explosions ce tick : celle-ci attend le suivant)
            let fuse = if fuse == 0 && blasts >= BLASTS_PER_TICK { 1 } else { fuse };
            if fuse == 0 || !ready || pos.1 < crate::world::MIN_Y as f32 - 8.0 {
                blasts += 1;
                self.tnt.remove(&id);
                self.send_near(pos, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
                // (tombée dans le vide ou sortie du terrain chargé : elle disparaît sans exploser)
                if ready && fuse == 0 {
                    self.explode_with((pos.0, pos.1 + 0.49, pos.2), 4.0, None, "une explosion", true);
                }
                continue;
            }
            if let Some(t) = self.tnt.get_mut(&id) {
                let moved = t.pos != pos;
                (t.pos, t.vel, t.fuse) = (pos, vel, fuse);
                // (au repos aussi, de temps en temps : le jeu ne doit pas la faire tomber de son côté)
                if moved || fuse % 10 == 0 {
                    let packet = bedrock::move_actor_absolute(id, (pos.0, pos.1 + TNT_CENTRE, pos.2), 0.0, 0.0, 0.0, !moved);
                    self.send_near(pos, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                }
            }
        }
    }

    /// Deux fois par seconde : ce qui se tient dans le feu ou la lave brûle. Le feu fait un
    /// demi-cœur par seconde et enflamme huit secondes, la lave deux cœurs et quinze secondes ; l'eau
    /// éteint. Le blaze, le ghast et le wither squelette n'y sont pas sensibles.
    pub(super) fn burn_tick(&mut self) {
        let tick = self.tick;
        let players: Vec<(SocketAddr, (f32, f32, f32))> =
            self.clients.iter().filter(|(_, c)| c.joined && c.stage == Stage::InGame && c.dead.is_none() && c.mode.takes_damage()).map(|(a, c)| (*a, (c.pos.0, c.pos.1 - EYE, c.pos.2))).collect();
        for (addr, feet) in players {
            if !self.world.is_ready((feet.0.floor() as i32) >> 4, (feet.2.floor() as i32) >> 4) {
                continue;
            }
            // Tout le corps compte, pas seulement son centre : un pied dans les flammes suffit
            let y = (feet.1 + 0.1).floor() as i32;
            let mut cells: Vec<u32> = Vec::with_capacity(8);
            for (dx, dz) in [(-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3)] {
                let (x, z) = ((feet.0 + dx).floor() as i32, (feet.2 + dz).floor() as i32);
                cells.push(self.world.get(x, y, z));
                cells.push(self.world.get(x, y + 1, z));
            }
            cells.dedup();
            let (in_lava, in_fire) = (cells.iter().any(|s| self.is_lava(*s)), cells.iter().any(|s| self.is_fire(*s)));
            if in_lava {
                self.ignite(&addr, 300);
                self.hurt(&addr, 4.0, Cause::Fire);
            } else if in_fire {
                // (un demi-cœur par seconde ici, un autre parce qu'on est en feu : deux par
                // seconde en tout, comme dans le jeu)
                self.ignite(&addr, 160);
                if tick % 20 < 10 {
                    self.hurt(&addr, 1.0, Cause::Fire);
                }
            }
        }
        let mobs: Vec<(i64, (f32, f32, f32), u64, bool)> = self.mobs.iter().filter(|(_, m)| m.dead.is_none()).map(|(id, m)| (*id, m.pos, m.fire_until, m.fireproof())).collect();
        for (id, pos, until, fireproof) in mobs {
            let (x, y, z) = (pos.0.floor() as i32, (pos.1 + 0.1).floor() as i32, pos.2.floor() as i32);
            if !self.world.is_ready(x >> 4, z >> 4) {
                continue;
            }
            let state = self.world.get(x, y, z);
            let (lava, fire) = (self.is_lava(state), self.is_fire(state));
            if fireproof || (!lava && !fire && until == 0) {
                continue;
            }
            if state == self.world.generator().blocks.water || (until != 0 && until <= tick && !lava && !fire) {
                self.set_mob_fire(id, 0);
                continue;
            }
            if lava || fire {
                self.set_mob_fire(id, tick + if lava { 300 } else { 160 });
            }
            // (dans les flammes : à chaque passe ; simplement en feu : une fois par seconde)
            if lava {
                self.hit_mob(id, pos, 4.0, mobs::Attacker::Mob(id));
            } else if fire || tick % 20 < 10 {
                self.hit_mob(id, pos, 1.0, mobs::Attacker::Mob(id));
            }
        }
    }

    /// Quatre fois par seconde : la tête sous l'eau, un joueur perd son air en quinze secondes
    /// (les bulles au-dessus de sa barre de faim), puis se noie d'un cœur par seconde. À
    /// l'air libre il le retrouve vite. La respiration aquatique l'en dispense.
    pub(super) fn air_tick(&mut self) {
        let water = self.world.generator().blocks.water;
        let mut changed: Vec<(SocketAddr, i64, i32, bool, i64)> = Vec::new();
        let mut drowned: Vec<SocketAddr> = Vec::new();
        for (addr, c) in self.clients.iter_mut().filter(|(_, c)| c.joined && c.stage == Stage::InGame && c.dead.is_none()) {
            let head = (c.pos.0.floor() as i32, c.pos.1.floor() as i32, c.pos.2.floor() as i32);
            let under = c.mode.takes_damage() && self.world.is_ready(head.0 >> 4, head.2 >> 4) && self.world.get(head.0, head.1, head.2) == water && !c.effects.iter().any(|e| e.0 == 13);
            let before = c.air;
            // (apnée : le casque ne laisse filer l'air qu'une fois sur (niveau + 1))
            let held_breath = c.inv.armor.slots.iter().map(|s| s.ench.level(crate::item::enchant::RESPIRATION)).max().unwrap_or(0) as u32;
            if under && (c.air <= 0 || self.mob_rng.below(held_breath + 1) == 0) {
                c.air -= 5;
                if c.air <= -20 {
                    c.air = 0;
                    drowned.push(*addr);
                }
            } else if !under {
                c.air = (c.air + 25).min(300);
            }
            if c.air != before {
                changed.push((*addr, c.id, c.air.max(0), under, (c.burn_until > 0) as i64 | if c.riding.is_some() { bedrock::FLAG_RIDING } else { 0 }));
            }
        }
        for (addr, id, air, under, extra) in changed {
            // (l'indicateur « respire » retiré : c'est lui qui fait apparaître les bulles)
            let flags = (bedrock::OWN_PLAYER_FLAGS | extra) & !(if under { 1i64 << 35 } else { 0 });
            self.send(&addr, bedrock::ID_SET_ACTOR_DATA, &bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(flags)), (7, bedrock::Meta::Short(air as i16))]));
        }
        for addr in drowned {
            self.hurt(&addr, 2.0, Cause::Drown);
        }
    }

    /// Enflamme une créature jusqu'à ce tick (0 : l'éteint) et le montre aux joueurs.
    pub(super) fn set_mob_fire(&mut self, id: i64, until: u64) {
        let Some(m) = self.mobs.get_mut(&id) else { return };
        if m.fireproof() && until != 0 {
            return;
        }
        let was = m.burning;
        m.fire_until = if until == 0 { 0 } else { m.fire_until.max(until) };
        m.burning = until != 0;
        if was != m.burning {
            self.mob_changed(id);
        }
    }
}
