//! La pêche : la canne lance un bouchon, qui flotte ; un poisson finit par mordre, et relever
//! la ligne à ce moment rapporte une prise. Les chances viennent des tables de butin du jeu
//! (85 poissons, 10 rebuts, 5 trésors sur 100 ; la chance de la mer déplace ces poids).

use super::*;

/// Événements du bouchon (un poisson approche, puis mord).
const EVENT_FISH_BUBBLE: u8 = 11;
const EVENT_FISH_HOOK: u8 = 13;
/// Donnée d'une entité : son propriétaire (le jeu dessine la ligne jusqu'à lui).
const META_OWNER: u32 = 5;
/// Attente avant une touche, en ticks (5 à 30 secondes) ; l'appât retire 5 secondes par niveau.
const WAIT: (u32, u32) = (100, 600);
/// Temps pour relever la ligne une fois que ça mord.
const BITE: u64 = 30;
/// Au-delà (en blocs), la ligne casse.
const REACH: f32 = 32.0;

pub(super) struct Hook {
    pub(super) id: i64,
    pos: (f32, f32, f32),
    vel: (f32, f32, f32),
    sent: (f32, f32, f32),
    /// Posé sur l'eau
    afloat: bool,
    /// Planté dans un bloc
    stuck: bool,
    /// Ticks avant la prochaine touche (une fois sur l'eau)
    wait: u32,
    /// Ça mord jusqu'à ce tick
    bite_until: u64,
    lure: u8,
    luck: u8,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Catch {
    Fish,
    Junk,
    Treasure,
}

/// La sorte de prise, selon la chance de la mer. `roll` : un nombre au hasard.
/// Poids du jeu : rebut 10 − 2 par niveau, trésor 5 + 2 par niveau, poisson 85 − 1 par niveau.
pub(super) fn catch_kind(luck: u8, roll: u32) -> Catch {
    let luck = luck.min(3) as u32;
    let (junk, treasure, fish) = (10 - 2 * luck, 5 + 2 * luck, 85 - luck);
    let r = roll % (junk + treasure + fish);
    if r < junk {
        Catch::Junk
    } else if r < junk + treasure {
        Catch::Treasure
    } else {
        Catch::Fish
    }
}

/// Tire une entrée d'une table (nom, poids).
fn weighted<'a>(table: &'a [(&'a str, u32)], roll: u32) -> &'a str {
    let total: u32 = table.iter().map(|e| e.1).sum();
    let mut r = roll % total.max(1);
    for (name, weight) in table {
        if r < *weight {
            return name;
        }
        r -= weight;
    }
    table[0].0
}

/// Les tables du jeu (noms d'aujourd'hui : cod pour « fish », tropical_fish pour « clownfish »).
const FISH: [(&str, u32); 4] = [("cod", 60), ("salmon", 25), ("tropical_fish", 2), ("pufferfish", 13)];
const JUNK: [(&str, u32); 12] = [
    ("leather_boots", 10), ("lily_pad", 17), ("ink_sac", 2), ("leather", 10), ("bone", 10), ("potion", 10), ("string", 5), ("fishing_rod", 2), ("bowl", 10), ("stick", 5), ("tripwire_hook", 10),
    ("rotten_flesh", 10),
];
const TREASURE: [(&str, u32); 6] = [("nautilus_shell", 5), ("name_tag", 5), ("saddle", 5), ("bow", 5), ("fishing_rod", 5), ("enchanted_book", 6)];

/// L'objet pêché : (nom, quantité, usé, enchanté).
pub(super) fn catch_item(kind: Catch, roll: u32) -> (&'static str, u8, bool, bool) {
    match kind {
        Catch::Fish => (weighted(&FISH, roll), 1, false, false),
        Catch::Junk => {
            let name = weighted(&JUNK, roll);
            (name, if name == "ink_sac" { 10 } else { 1 }, matches!(name, "leather_boots" | "fishing_rod"), false)
        }
        Catch::Treasure => {
            let name = weighted(&TREASURE, roll);
            (name, 1, matches!(name, "bow" | "fishing_rod"), matches!(name, "bow" | "fishing_rod" | "enchanted_book"))
        }
    }
}

/// L'attente avant une touche avec cet appât, pour un tirage de 0 à 999.
pub(super) fn bite_wait(lure: u8, roll: u32) -> u32 {
    let base = WAIT.0 + (WAIT.1 - WAIT.0) * roll.min(999) / 999;
    base.saturating_sub(100 * lure.min(3) as u32).max(20)
}

impl Server {
    /// Clic dans le vide avec une canne à pêche : elle lance son bouchon, ou le relève s'il
    /// est déjà à l'eau. Vrai si l'objet tenu était une canne.
    pub(super) fn use_rod(&mut self, addr: &SocketAddr) -> bool {
        let tick = self.tick;
        let Some(c) = self.clients.get(addr).filter(|c| c.joined && c.dead.is_none() && !c.mode.is_spectator()) else { return false };
        if self.held_name(c) != "fishing_rod" || !self.cfg.vanilla.fishing {
            return false;
        }
        // (le jeu répète le clic tant que le bouton est tenu)
        if tick.saturating_sub(c.threw_at) < 5 {
            return true;
        }
        let (eye, yaw, pitch, player, ench) = (c.pos, c.yaw.to_radians(), c.pitch.to_radians(), c.id, c.inv.held().ench);
        if let Some(c) = self.clients.get_mut(addr) {
            c.threw_at = tick;
        }
        if self.hooks.contains_key(addr) {
            self.reel_in(addr);
            return true;
        }
        let dir = (-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
        let from = (eye.0 + dir.0 * 0.4, eye.1 - 0.1, eye.2 + dir.2 * 0.4);
        let vel = (dir.0 * 0.6, dir.1 * 0.6 + 0.15, dir.2 * 0.6);
        let id = self.next_id;
        self.next_id += 1;
        let mut meta = bedrock::mob_metadata(0.15, 0.15, 0, 0, 0);
        // (son propriétaire : sans lui, le jeu ne dessine pas la ligne)
        meta.retain(|m| m.0 != META_OWNER);
        meta.push((META_OWNER, bedrock::Meta::Long(player)));
        self.send_near(from, bedrock::ID_ADD_ACTOR, &bedrock::add_actor(id, "minecraft:fishing_hook", from, vel, 0.0, (1.0, 1.0), None, None, &meta));
        self.send_near(eye, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("throw", eye, "", false, -1));
        let (lure, luck) = (ench.level(crate::item::enchant::LURE), ench.level(crate::item::enchant::LUCK_OF_THE_SEA));
        let wait = bite_wait(lure, self.mob_rng.below(1000));
        self.hooks.insert(*addr, Hook { id, pos: from, vel, sent: from, afloat: false, stuck: false, wait, bite_until: 0, lure, luck });
        true
    }

    /// Retire le bouchon d'un joueur (ligne relevée, canne rangée, joueur parti).
    pub(super) fn drop_hook(&mut self, addr: &SocketAddr) {
        if let Some(h) = self.hooks.remove(addr) {
            self.broadcast(bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(h.id), None);
        }
    }

    /// La ligne est relevée : si ça mordait, la prise vole vers le pêcheur, avec un peu
    /// d'expérience, et la canne s'use.
    fn reel_in(&mut self, addr: &SocketAddr) {
        let tick = self.tick;
        let Some(h) = self.hooks.get(addr) else { return };
        let (biting, at, luck, stuck) = (tick < h.bite_until, h.pos, h.luck, h.stuck);
        self.drop_hook(addr);
        if stuck {
            // (décrocher un bouchon planté use la canne, comme dans le jeu)
            self.wear_held(addr, crate::item::Usage::Mine);
            return;
        }
        if !biting {
            return;
        }
        let kind = catch_kind(luck, self.mob_rng.below(10_000));
        let (name, count, worn, enchanted) = catch_item(kind, self.mob_rng.below(10_000));
        // (anciens noms, pour les versions du jeu qui ne connaissent pas les nouveaux)
        let old = match name {
            "cod" => "fish",
            "tropical_fish" => "clownfish",
            "lily_pad" => "waterlily",
            other => other,
        };
        let Some(item) = self.item_id(name).or_else(|| self.item_id(old)) else { return };
        let mut stack = inventory::ItemStack::new(item, count, 0);
        if worn {
            // (rebut : très usé ; trésor : presque neuf)
            let max = self.data.as_ref().and_then(|d| d.item_by_id(item)).map(|def| crate::item::VanillaItems::get(&def.name).max_durability()).unwrap_or(0);
            let share = if kind == Catch::Junk { 10 + self.mob_rng.below(80) } else { self.mob_rng.below(25) };
            stack.damage = (max * share / 100).min(u16::MAX as u32) as u16;
        }
        if enchanted {
            // Un enchantement au hasard parmi ceux que l'objet accepte
            let fitting: Vec<(u8, u8)> = crate::item::enchant::all().into_iter().filter(|(id, _)| crate::item::enchant::fits(*id, name)).collect();
            if !fitting.is_empty() {
                let (id, max) = fitting[self.mob_rng.below(fitting.len() as u32) as usize];
                stack.ench = stack.ench.with(id, 1 + self.mob_rng.below(max.max(1) as u32) as u8);
            }
        }
        let Some(to) = self.clients.get(addr).map(|c| c.pos) else { return };
        let (dx, dy, dz) = (to.0 - at.0, to.1 - at.1, to.2 - at.2);
        let far = (dx * dx + dy * dy + dz * dz).sqrt().max(0.1);
        self.drop_stack(at, (dx * 0.1, dy * 0.1 + far.sqrt() * 0.08, dz * 0.1), stack, 0);
        let xp = self.xp_between(1, 6);
        self.give_xp(addr, xp);
        self.wear_held(addr, crate::item::Usage::Mine);
    }

    /// Chaque tick : le bouchon vole, se pose sur l'eau, attend, puis ça mord.
    pub(super) fn fishing_tick(&mut self) {
        if self.hooks.is_empty() {
            return;
        }
        let tick = self.tick;
        let owners: Vec<SocketAddr> = self.hooks.keys().copied().collect();
        for addr in owners {
            // La canne rangée, le pêcheur mort, parti ou trop loin : la ligne disparaît
            let Some(h) = self.hooks.get(&addr) else { continue };
            let holds = self.clients.get(&addr).filter(|c| c.joined && c.dead.is_none() && self.held_name(c) == "fishing_rod").map(|c| c.pos);
            let near = holds.map(|p| (p.0 - h.pos.0).powi(2) + (p.1 - h.pos.1).powi(2) + (p.2 - h.pos.2).powi(2) < REACH * REACH).unwrap_or(false);
            if !near || h.pos.1 < crate::world::MIN_Y as f32 - 8.0 {
                self.drop_hook(&addr);
                continue;
            }
            let (mut pos, mut vel, mut afloat, mut stuck) = (h.pos, h.vel, h.afloat, h.stuck);
            let (bx, by, bz) = (pos.0.floor() as i32, pos.1.floor() as i32, pos.2.floor() as i32);
            if !self.world.is_ready(bx >> 4, bz >> 4) {
                continue;
            }
            let state = if crate::world::World::in_bounds(by) { self.world.get(bx, by, bz) } else { self.world.air };
            let wet = self.is_water(state);
            if !stuck {
                if wet {
                    // Dans l'eau : il ralentit et remonte jusqu'à la surface
                    let above = if crate::world::World::in_bounds(by + 1) { self.world.get(bx, by + 1, bz) } else { self.world.air };
                    let surface = !self.is_water(above) && pos.1 - by as f32 > 0.7;
                    vel = (vel.0 * 0.6, if surface { 0.0 } else { (vel.1 * 0.6 + 0.04).min(0.1) }, vel.2 * 0.6);
                    afloat = surface;
                } else {
                    afloat = false;
                    vel = (vel.0 * 0.98, vel.1 * 0.98 - 0.05, vel.2 * 0.98);
                }
                let next = (pos.0 + vel.0, pos.1 + vel.1, pos.2 + vel.2);
                // (un bloc plein l'arrête)
                if !wet && !self.mob_free(next.0.floor() as i32, next.1.floor() as i32, next.2.floor() as i32) && !self.is_water(self.world.get(next.0.floor() as i32, next.1.floor().clamp(crate::world::MIN_Y as f32, crate::world::MAX_Y as f32 - 1.0) as i32, next.2.floor() as i32)) {
                    stuck = true;
                    vel = (0.0, 0.0, 0.0);
                } else {
                    pos = next;
                }
            }
            // Sur l'eau : l'attente, les bulles, la touche
            let mut event = None;
            let Some(h) = self.hooks.get_mut(&addr) else { continue };
            h.pos = pos;
            h.vel = vel;
            h.afloat = afloat;
            h.stuck = stuck;
            if afloat && tick >= h.bite_until {
                if h.bite_until != 0 {
                    // (raté : le bouchon remonte, une nouvelle attente commence)
                    h.bite_until = 0;
                    h.pos.1 += 0.25;
                    h.wait = bite_wait(h.lure, (tick as u32).wrapping_mul(2_654_435_761) % 1000);
                } else if h.wait == 0 {
                    h.bite_until = tick + BITE;
                    h.pos.1 -= 0.25;
                    event = Some(EVENT_FISH_HOOK);
                } else {
                    h.wait -= 1;
                    if h.wait == 20 {
                        event = Some(EVENT_FISH_BUBBLE);
                    }
                }
            }
            let (id, at) = (h.id, h.pos);
            let moved = (at.0 - h.sent.0).abs() + (at.1 - h.sent.1).abs() + (at.2 - h.sent.2).abs() > 0.02;
            if moved {
                h.sent = at;
            }
            if moved {
                self.send_near(at, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &bedrock::move_actor_absolute(id, at, 0.0, 0.0, 0.0, false));
            }
            if let Some(e) = event {
                self.send_near(at, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, e, 0));
                if e == EVENT_FISH_HOOK {
                    // (l'éclaboussure et son bruit : la touche se voit et s'entend)
                    self.send_near(at, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_PARTICLE_SPLASH, at, 0x385dc6));
                    self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("splash", at, "minecraft:fishing_hook", false, -1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fishing_follows_the_game_tables() {
        // Sans chance : 10 rebuts, 5 trésors, 85 poissons sur 100
        let count = |luck: u8, kind: Catch| (0..100 - luck as u32).filter(|r| catch_kind(luck, *r) == kind).count();
        assert_eq!((count(0, Catch::Junk), count(0, Catch::Treasure), count(0, Catch::Fish)), (10, 5, 85));
        // Chance de la mer III : 4 rebuts, 11 trésors, 82 poissons (sur 97)
        assert_eq!((count(3, Catch::Junk), count(3, Catch::Treasure), count(3, Catch::Fish)), (4, 11, 82));
        // Les poissons, selon leur poids (60, 25, 2, 13)
        let fish = |name: &str| (0..100).filter(|r| catch_item(Catch::Fish, *r).0 == name).count();
        assert_eq!((fish("cod"), fish("salmon"), fish("tropical_fish"), fish("pufferfish")), (60, 25, 2, 13));
        // Rebut : dix poches d'encre d'un coup, des bottes usées ; trésor : un arc enchanté
        assert!((0..101).any(|r| catch_item(Catch::Junk, r) == ("ink_sac", 10, false, false)));
        assert!((0..101).any(|r| catch_item(Catch::Junk, r) == ("leather_boots", 1, true, false)));
        assert!((0..31).any(|r| catch_item(Catch::Treasure, r) == ("bow", 1, true, true)));
        assert!((0..31).any(|r| catch_item(Catch::Treasure, r) == ("saddle", 1, false, false)));
        // L'attente : 5 à 30 secondes, 5 de moins par niveau d'appât, jamais moins d'une seconde
        assert_eq!((bite_wait(0, 0), bite_wait(0, 999), bite_wait(2, 999), bite_wait(3, 0)), (100, 600, 400, 20));
    }
}
