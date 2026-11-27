//! Portails du Nether et de l'End.
//!
//! Le serveur allume et éteint les portails, et prévient les plugins quand un joueur en
//! franchit un (`PlayerPortalEvent`). Il ne téléporte personne : c'est au plugin de décider
//! où mène le portail (`joueur:setWorld(...)`).

use super::*;

/// Plus grand côté intérieur d'un portail du Nether (comme dans le jeu).
const MAX_SIDE: i32 = 21;
/// Temps passé dans un portail du Nether avant de le franchir, en survie (quatre secondes).
const WARMUP: u16 = 80;
/// Marque : ce joueur a déjà franchi le portail où il se tient.
const CROSSED: u16 = u16::MAX;

impl Server {
    fn portal_name(&self, state: u32) -> Option<&str> {
        self.data.as_ref().and_then(|d| d.block_name(state))
    }

    fn obsidian(&self, p: (i32, i32, i32)) -> bool {
        self.portal_name(self.world.get(p.0, p.1, p.2)) == Some("minecraft:obsidian")
    }

    fn portal_kind(&self, state: u32) -> Option<&'static str> {
        match self.portal_name(state) {
            Some("minecraft:portal") => Some("nether"),
            Some("minecraft:end_portal") => Some("end"),
            _ => None,
        }
    }

    /// Un feu vient de prendre en `p`. S'il est dans un cadre d'obsidienne complet (de 2 × 3 à
    /// 21 × 21 à l'intérieur), le cadre se remplit : le portail du Nether est allumé.
    pub(super) fn try_nether_portal(&mut self, p: (i32, i32, i32)) -> bool {
        let air = self.world.air;
        let inside = |s: &Self, q: (i32, i32, i32)| {
            let state = s.world.get(q.0, q.1, q.2);
            state == air || s.is_fire(state)
        };
        for (axis, (dx, dz)) in [("x", (1, 0)), ("z", (0, 1))] {
            // Le bas du cadre, puis son bord gauche
            let mut y0 = p.1;
            while p.1 - y0 < MAX_SIDE && inside(self, (p.0, y0 - 1, p.2)) {
                y0 -= 1;
            }
            let mut left = 0;
            while left > -MAX_SIDE && inside(self, (p.0 + (left - 1) * dx, y0, p.2 + (left - 1) * dz)) {
                left -= 1;
            }
            let at = |i: i32, y: i32| (p.0 + (left + i) * dx, y, p.2 + (left + i) * dz);
            let width = (0..=MAX_SIDE).take_while(|i| inside(self, at(*i, y0))).count() as i32;
            let height = (0..=MAX_SIDE).take_while(|j| inside(self, at(0, y0 + *j))).count() as i32;
            if !(2..=MAX_SIDE).contains(&width) || !(3..=MAX_SIDE).contains(&height) {
                continue;
            }
            let hollow = (0..width).all(|i| (0..height).all(|j| inside(self, at(i, y0 + j))));
            let framed = (0..width).all(|i| self.obsidian(at(i, y0 - 1)) && self.obsidian(at(i, y0 + height))) && (0..height).all(|j| self.obsidian(at(-1, y0 + j)) && self.obsidian(at(width, y0 + j)));
            if !hollow || !framed {
                continue;
            }
            let Some(portal) = self.data.as_ref().and_then(|d| d.state_from_spec(&format!("minecraft:portal[portal_axis={}]", axis))) else { return false };
            for i in 0..width {
                for j in 0..height {
                    let q = at(i, y0 + j);
                    self.fires.remove(&q);
                    if self.world.set(q.0, q.1, q.2, portal) {
                        self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(q.0, q.1, q.2, portal), None);
                    }
                }
            }
            return true;
        }
        false
    }

    /// Un bloc vient de disparaître en `pos` : le portail du Nether qui le touchait (cadre
    /// cassé, ou un de ses blocs retiré) s'éteint en entier.
    pub(super) fn portal_broken(&mut self, pos: (i32, i32, i32)) {
        let is_portal = |s: &Self, q: (i32, i32, i32)| s.portal_name(s.world.get(q.0, q.1, q.2)) == Some("minecraft:portal");
        let around = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];
        let mut queue: Vec<(i32, i32, i32)> = around.iter().map(|d| (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2)).filter(|q| is_portal(self, *q)).collect();
        let air = self.world.air;
        let mut done = 0;
        while let Some(q) = queue.pop() {
            if done >= 1000 || !is_portal(self, q) {
                continue;
            }
            done += 1;
            if self.world.set(q.0, q.1, q.2, air) {
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(q.0, q.1, q.2, air), None);
            }
            queue.extend(around.iter().map(|d| (q.0 + d.0, q.1 + d.1, q.2 + d.2)));
        }
        if done > 0 {
            let at = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
            self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("glass", at, "", false, -1));
        }
    }

    /// Œil de l'Ender posé sur un cadre de portail de l'End. Quand les douze cadres d'un
    /// anneau portent leur œil, le portail s'ouvre. Vrai si l'objet tenu était un œil et le
    /// bloc visé un cadre.
    pub(super) fn use_ender_eye(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        let Some(c) = self.clients.get(addr) else { return false };
        if self.held_name(c) != "ender_eye" {
            return false;
        }
        let survival = c.mode.takes_damage();
        let state = self.world.get(pos.0, pos.1, pos.2);
        if self.portal_name(state) != Some("minecraft:end_portal_frame") {
            return false;
        }
        let Some(d) = self.data.clone() else { return true };
        if d.prop(state, "end_portal_eye_bit") == Some(Prop::Int(1)) {
            return true;
        }
        let Some(filled) = d.with_props(state, &[("end_portal_eye_bit", Prop::Int(1))]) else { return true };
        self.change_block(pos, filled);
        let centre = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
        self.send_near(centre, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("block.end_portal_frame.fill", centre, "", false, -1));
        if survival {
            if let Some(c) = self.clients.get_mut(addr) {
                c.inv.consume_held();
                c.inv_touched = true;
            }
            self.send_inventory(addr);
            self.show_held(addr);
        }
        // L'anneau : un carré de 3 × 3 entouré de douze cadres. Ce cadre est sur l'un de ses
        // côtés : on cherche le coin du carré parmi les positions possibles.
        let eyed = |s: &Self, q: (i32, i32, i32)| {
            let st = s.world.get(q.0, q.1, q.2);
            s.portal_name(st) == Some("minecraft:end_portal_frame") && d.prop(st, "end_portal_eye_bit") == Some(Prop::Int(1))
        };
        for cx in pos.0 - 3..=pos.0 + 1 {
            for cz in pos.2 - 3..=pos.2 + 1 {
                // (coin nord-ouest de l'intérieur : cx..cx+2, cz..cz+2)
                let ring = (0..3).all(|i| eyed(self, (cx + i, pos.1, cz - 1)) && eyed(self, (cx + i, pos.1, cz + 3)) && eyed(self, (cx - 1, pos.1, cz + i)) && eyed(self, (cx + 3, pos.1, cz + i)));
                if !ring {
                    continue;
                }
                let Some(portal) = d.default_state("minecraft:end_portal") else { return true };
                for i in 0..3 {
                    for j in 0..3 {
                        self.change_block((cx + i, pos.1, cz + j), portal);
                    }
                }
                let at = (cx as f32 + 1.5, pos.1 as f32 + 0.5, cz as f32 + 1.5);
                self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("block.end_portal.spawn", at, "", false, -1));
                return true;
            }
        }
        true
    }

    /// Chaque tick : un joueur qui se tient dans un portail le franchit — tout de suite en
    /// créatif ou dans un portail de l'End, au bout de quatre secondes dans un portail du
    /// Nether en survie. Les plugins sont prévenus une fois ; le serveur ne déplace personne.
    pub(super) fn portals_tick(&mut self) {
        let mut crossed: Vec<(SocketAddr, &'static str, (i32, i32, i32))> = Vec::new();
        let standing: Vec<(SocketAddr, (i32, i32, i32), bool, u16)> = self
            .clients
            .iter()
            .filter(|(_, c)| c.joined && c.stage == Stage::InGame && c.dead.is_none())
            .map(|(a, c)| (*a, (c.pos.0.floor() as i32, (c.pos.1 - player_eye_height() + 0.1).floor() as i32, c.pos.2.floor() as i32), c.mode.takes_damage(), c.in_portal))
            .collect();
        for (addr, feet, survival, mut time) in standing {
            let kind = [feet, (feet.0, feet.1 + 1, feet.2)].iter().find_map(|q| self.portal_kind(self.world.get(q.0, q.1, q.2)));
            match kind {
                None => time = 0,
                Some(_) if time == CROSSED => {}
                Some(kind) => {
                    time += 1;
                    if kind == "end" || !survival || time >= WARMUP {
                        time = CROSSED;
                        crossed.push((addr, kind, feet));
                    }
                }
            }
            if let Some(c) = self.clients.get_mut(&addr) {
                c.in_portal = time;
            }
        }
        for (addr, kind, feet) in crossed {
            if self.plugins.listens("PlayerPortalEvent") {
                let block = self.block_json(self.world.get(feet.0, feet.1, feet.2), feet);
                self.fire("PlayerPortalEvent", vec![("player", self.name_json(&addr)), ("portal", Json::Str(kind.to_string())), ("world", Json::Str(self.realm.clone())), ("block", block)]);
            }
        }
    }
}
