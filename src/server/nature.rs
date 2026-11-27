//! Le monde qui réagit : ce que deviennent les blocs quand leurs voisins changent.
//!
//! Ce fichier applique les règles ; ce qu'est chaque bloc (il tombe, il lui faut un sol, c'est
//! une culture…) est décrit dans `block/behavior.rs`.
//!
//! - **Feuilles** : privées de tronc, elles dépérissent en quelques secondes et laissent
//!   parfois une pousse, un bâton ou une pomme. Les feuilles posées par un joueur restent.

use std::net::SocketAddr;

use super::*;
use crate::block::behavior::{falls, is_crop, is_leaves, is_log, needs_floor, two_halves};

/// Distance (en blocs, de feuille en feuille) au-delà de laquelle une feuille n'est plus
/// nourrie par un tronc.
const LEAF_REACH: u8 = 6;
/// Demi-côté de la zone de feuilles examinée autour d'un tronc cassé.
const LEAF_BOX: i32 = 7;

impl Server {
    /// Un tronc vient de disparaître : les feuilles qu'aucun autre tronc ne nourrit sont
    /// promises à tomber, chacune à son heure dans les secondes qui suivent.
    pub(super) fn leaves_after_log(&mut self, (cx, cy, cz): (i32, i32, i32)) {
        let Some(d) = self.data.clone() else { return };
        let outer = LEAF_BOX + LEAF_REACH as i32;
        let mut leaves: HashMap<(i32, i32, i32), bool> = HashMap::new();
        let mut reached: std::collections::VecDeque<((i32, i32, i32), u8)> = std::collections::VecDeque::new();
        let air = self.world.air;
        for x in cx - outer..=cx + outer {
            for z in cz - outer..=cz + outer {
                for y in (cy - outer).max(-64)..=(cy + outer).min(319) {
                    let s = self.world.get(x, y, z);
                    if s == air {
                        continue;
                    }
                    match d.block_name(s) {
                        Some(n) if is_log(n) => reached.push_back(((x, y, z), 0)),
                        Some(n) if is_leaves(n) => {
                            leaves.insert((x, y, z), d.prop(s, "persistent_bit") == Some(Prop::Int(1)));
                        }
                        _ => {}
                    }
                }
            }
        }
        if leaves.is_empty() {
            return;
        }
        // De proche en proche depuis tous les troncs, à travers les feuilles
        let mut fed: HashSet<(i32, i32, i32)> = HashSet::new();
        while let Some(((x, y, z), dist)) = reached.pop_front() {
            if dist >= LEAF_REACH {
                continue;
            }
            for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let p = (x + dx, y + dy, z + dz);
                if leaves.contains_key(&p) && fed.insert(p) {
                    reached.push_back((p, dist + 1));
                }
            }
        }
        let tick = self.tick;
        for ((x, y, z), kept) in leaves {
            let inside = (x - cx).abs() <= LEAF_BOX && (y - cy).abs() <= LEAF_BOX && (z - cz).abs() <= LEAF_BOX;
            if inside && !kept && !fed.contains(&(x, y, z)) {
                let wait = 20 + self.mob_rng.below(140) as u64;
                self.decaying.entry((x, y, z)).or_insert(tick + wait);
            }
        }
    }

    /// Feuilles dont l'heure est venue : elles disparaissent, avec leurs miettes et leur butin.
    pub(super) fn decay_tick(&mut self) {
        if !self.cfg.vanilla.leaf_decay {
            self.decaying.clear();
        }
        if self.decaying.is_empty() {
            return;
        }
        let tick = self.tick;
        let due: Vec<(i32, i32, i32)> = self.decaying.iter().filter(|(_, t)| **t <= tick).map(|(p, _)| *p).take(64).collect();
        let Some(d) = self.data.clone() else {
            self.decaying.clear();
            return;
        };
        let air = self.world.air;
        for (x, y, z) in due {
            self.decaying.remove(&(x, y, z));
            let state = self.world.get(x, y, z);
            let name = match d.block_name(state) {
                Some(n) if is_leaves(n) => n.to_string(),
                _ => continue,
            };
            if self.plugins.listens("LeavesDecayEvent") {
                let out = self.fire("LeavesDecayEvent", vec![("block", self.block_json(state, (x, y, z)))]);
                if cancelled(&out) {
                    continue;
                }
            }
            if !self.world.set(x, y, z, air) {
                continue;
            }
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, air), None);
            let centre = (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
            self.send_near(centre, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_DESTROY, centre, state as i32));
            self.support_check((x, y, z));
            // Butin : une pousse (1 sur 20), un ou deux bâtons (1 sur 50), une pomme (chêne, 1 sur 200)
            let wood = name.trim_start_matches("minecraft:").trim_end_matches("_leaves").to_string();
            let mut loot: Vec<(String, u8)> = Vec::new();
            if self.mob_rng.below(20) == 0 {
                loot.push((format!("{}_sapling", wood), 1));
            }
            if self.mob_rng.below(50) == 0 {
                loot.push(("stick".into(), 1 + self.mob_rng.below(2) as u8));
            }
            if matches!(wood.as_str(), "oak" | "dark_oak") && self.mob_rng.below(200) == 0 {
                loot.push(("apple".into(), 1));
            }
            for (item, count) in loot {
                if let Some(id) = self.item_id(&item) {
                    self.drop_item(centre, (0.0, 0.1, 0.0), id, count, 10);
                }
            }
        }
    }
}

// --- Blocs qui tiennent sur un support, blocs qui tombent ----------------------------------

/// Où se trouve le bloc qui porte celui-ci (décalage), s'il lui en faut un.
fn support_of(d: &crate::data::GameData, state: u32, name: &str) -> Option<(i32, i32, i32)> {
    let n = name.trim_start_matches("minecraft:");
    // Torche : le mot désigne le côté où se trouve le bloc qui la porte
    if let Some(Prop::Str(side)) = d.prop(state, "torch_facing_direction") {
        return Some(match side.as_str() {
            "north" => (0, 0, -1),
            "south" => (0, 0, 1),
            "west" => (-1, 0, 0),
            "east" => (1, 0, 0),
            _ => (0, -1, 0),
        });
    }
    if let Some(Prop::Str(way)) = d.prop(state, "lever_direction") {
        return Some(match way.as_str() {
            w if w.starts_with("up") => (0, -1, 0),
            w if w.starts_with("down") => (0, 1, 0),
            "north" => (0, 0, 1),
            "south" => (0, 0, -1),
            "west" => (1, 0, 0),
            _ => (-1, 0, 0),
        });
    }
    // Échelle, bouton, panneau mural : accrochés à la face cliquée, donc portés par le bloc d'en face
    if n == "ladder" || n.ends_with("button") || n.ends_with("wall_sign") || n == "wall_banner" || n == "tripwire_hook" {
        if let Some(Prop::Int(face)) = d.prop(state, "facing_direction") {
            return Some(match face {
                0 => (0, 1, 0),
                1 => (0, -1, 0),
                2 => (0, 0, 1),
                3 => (0, 0, -1),
                4 => (1, 0, 0),
                _ => (-1, 0, 0),
            });
        }
    }
    if n.ends_with("lantern") && !n.contains("sea") && !n.contains("jack") {
        return Some(if d.prop(state, "hanging") == Some(Prop::Int(1)) { (0, 1, 0) } else { (0, -1, 0) });
    }
    if matches!(n, "hanging_roots" | "spore_blossom") {
        return Some((0, 1, 0));
    }
    needs_floor(name).then_some((0, -1, 0))
}

impl Server {
    /// Ce bloc peut-il porter `leaning`, qui s'appuie sur lui ?
    fn can_support(&self, d: &crate::data::GameData, support: u32, leaning: &str) -> bool {
        if support == self.world.air {
            return false;
        }
        let Some(name) = d.block_name(support) else { return true };
        // Canne sur canne, cactus sur cactus, moitié haute sur moitié basse
        if name == leaning {
            return true;
        }
        !(name.contains("water") || name.contains("lava") || support_of(d, support, name).is_some())
    }

    /// Ce bloc peut-il être posé ici ? Il lui faut le support que le jeu exige : de la terre
    /// labourée pour une culture, de la terre ou de l'herbe pour une fleur ou une pousse, du
    /// sable pour un cactus, de l'eau au bord pour la canne à sucre, un bloc plein pour une
    /// torche, un rail ou une échelle.
    pub(super) fn can_stand(&self, state: u32, (x, y, z): (i32, i32, i32)) -> bool {
        let Some(d) = self.data.as_ref() else { return true };
        let Some(name) = d.block_name(state) else { return true };
        // Le feu tient sur un sol plein, ou contre un bloc qui brûle (le flanc d'un tronc)
        if self.is_fire(state) {
            return self.fire_holds((x, y, z));
        }
        let Some(off) = support_of(d, state, name) else { return true };
        let at = (x + off.0, y + off.1, z + off.2);
        if !crate::world::World::in_bounds(at.1) {
            return false;
        }
        let under = self.world.get(at.0, at.1, at.2);
        let ground = d.block_name(under).unwrap_or("").trim_start_matches("minecraft:");
        let n = name.trim_start_matches("minecraft:");
        let soil = matches!(ground, "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "farmland" | "mycelium" | "moss_block" | "pale_moss_block" | "rooted_dirt" | "mud" | "muddy_mangrove_roots" | "dirt_with_roots");
        let sand = matches!(ground, "sand" | "red_sand" | "suspicious_sand");
        // Moitié haute d'une porte ou d'une grande plante : posée par le serveur sur sa moitié basse
        if d.prop(state, "upper_block_bit") == Some(Prop::Int(1)) {
            return true;
        }
        match n {
            "wheat" | "carrots" | "potatoes" | "beetroot" | "melon_stem" | "pumpkin_stem" | "torchflower_crop" | "pitcher_crop" => ground == "farmland",
            "nether_wart" => ground == "soul_sand",
            "cactus" => sand || ground == "cactus",
            "reeds" => {
                // Sur une autre canne, ou sur un sol dont un côté touche l'eau
                ground == "reeds"
                    || ((soil || sand) && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dz)| {
                        let side = self.world.get(at.0 + dx, at.1, at.2 + dz);
                        d.block_name(side).map(|w| w.contains("water") || w.contains("frosted_ice")).unwrap_or(false) || self.world.is_wet(at.0 + dx, at.1, at.2 + dz)
                    }))
            }
            "bamboo" | "bamboo_sapling" => soil || sand || matches!(ground, "gravel" | "bamboo" | "bamboo_sapling"),
            "deadbush" | "dead_bush" => soil || sand || ground.contains("terracotta") || ground == "hardened_clay",
            "crimson_roots" | "warped_roots" | "crimson_fungus" | "warped_fungus" | "nether_sprouts" => soil || ground.contains("nylium") || ground == "soul_soil",
            // Champignons : sur tout bloc plein
            _ if n.contains("mushroom") => self.can_support(d, under, name),
            // Fleurs, herbes, fougères, pousses, buissons, pétales : de la terre ou de l'herbe
            _ if n.contains("sapling")
                || n.contains("flower")
                || n.contains("tulip")
                || n.contains("grass")
                || n.contains("fern")
                || n.contains("petals")
                || n.contains("eyeblossom")
                || n.ends_with("bush")
                || matches!(n, "dandelion" | "poppy" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose" | "blue_orchid" | "lilac" | "peony" | "pitcher_plant") =>
            {
                soil
            }
            _ => self.can_support(d, under, name),
        }
    }

    /// Clic sur un buisson à baies qui porte des fruits : ils tombent (un ou deux à
    /// l'avant-dernier stade, deux ou trois au dernier) et le buisson repart du stade jeune.
    /// Vrai si le clic a été traité ici.
    pub(super) fn pick_berries(&mut self, addr: &SocketAddr, (x, y, z): (i32, i32, i32)) -> bool {
        let Some(d) = self.data.clone() else { return false };
        let state = self.world.get(x, y, z);
        if d.block_name(state) != Some("minecraft:sweet_berry_bush") {
            return false;
        }
        let growth = int_of(&d, state, "growth").unwrap_or(0);
        let allowed = self.clients.get(addr).map(|c| c.joined && c.dead.is_none() && !c.mode.is_spectator()).unwrap_or(false);
        if growth < 2 || !allowed {
            return false;
        }
        let count = (if growth >= 3 { 2 } else { 1 }) + self.mob_rng.below(2) as u8;
        if let Some(young) = d.with_props(state, &[("growth", Prop::Int(1))]) {
            if self.world.set(x, y, z, young) {
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, young), None);
                if let Some(berries) = self.item_id("sweet_berries") {
                    self.drop_item((x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5), (0.0, 0.15, 0.0), berries, count, 10);
                }
            }
        }
        true
    }

    /// Houe sur de l'herbe, de la terre ou un chemin : la terre est labourée (rien au-dessus
    /// ne doit gêner). Vrai si le clic a été traité ici.
    pub(super) fn till(&mut self, addr: &SocketAddr, (x, y, z): (i32, i32, i32)) -> bool {
        let Some(d) = self.data.clone() else { return false };
        let hoe = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() && c.mode.can_build() => d.item_by_id(c.inv.held().id).map(|i| i.name.ends_with("_hoe")).unwrap_or(false) && !c.inv.held().is_empty(),
            _ => false,
        };
        if !hoe {
            return false;
        }
        let state = self.world.get(x, y, z);
        let next = match d.block_name(state).unwrap_or("").trim_start_matches("minecraft:") {
            "grass_block" | "dirt" | "grass_path" | "dirt_path" => "minecraft:farmland",
            "coarse_dirt" => "minecraft:dirt",
            _ => return false,
        };
        if y >= 319 || self.world.get(x, y + 1, z) != self.world.air {
            return true; // quelque chose est posé dessus : rien ne se passe
        }
        if let Some(farmland) = d.default_state(next) {
            if self.world.set(x, y, z, farmland) {
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, farmland), None);
                let centre = (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                self.send_near(centre, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound_with(bedrock::SOUND_PLACE, centre, farmland as i32, "", false, -1));
            }
        }
        true
    }

    /// Le bloc en `origin` vient de changer (cassé, tombé, déplacé) : ce qui s'appuyait sur
    /// lui se décroche et tombe en objet, de proche en proche, et ce qui pèse au-dessus se met
    /// à tomber.
    pub(super) fn support_check(&mut self, origin: (i32, i32, i32)) {
        let Some(d) = self.data.clone() else { return };
        let air = self.world.air;
        let mut queue: std::collections::VecDeque<(i32, i32, i32)> = std::collections::VecDeque::new();
        queue.push_back(origin);
        let mut done = 0;
        while let Some(p) = queue.pop_front() {
            done += 1;
            if done > 512 {
                break;
            }
            let here = self.world.get(p.0, p.1, p.2);
            for (dx, dy, dz) in [(0, 1, 0), (0, -1, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)] {
                let n = (p.0 + dx, p.1 + dy, p.2 + dz);
                if !crate::world::World::in_bounds(n.1) {
                    continue;
                }
                let state = self.world.get(n.0, n.1, n.2);
                if state == air {
                    continue;
                }
                let Some(name) = d.block_name(state).map(|s| s.to_string()) else { continue };
                // Sable, gravier… juste au-dessus : ils se mettent à tomber
                if dy == 1 && falls(&name) {
                    self.falling.insert(n);
                    continue;
                }
                let leans_here = support_of(&d, state, &name).map(|o| (n.0 + o.0, n.1 + o.1, n.2 + o.2) == p).unwrap_or(false);
                // Moitié basse dont la moitié haute a disparu
                let lost_top = dy == -1 && two_halves(&name) && d.prop(state, "upper_block_bit") == Some(Prop::Int(0)) && d.block_name(here) != Some(name.as_str());
                if !(leans_here && !self.can_support(&d, here, &name)) && !lost_top {
                    continue;
                }
                if !self.world.set(n.0, n.1, n.2, air) {
                    continue;
                }
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(n.0, n.1, n.2, air), None);
                let centre = (n.0 as f32 + 0.5, n.1 as f32 + 0.5, n.2 as f32 + 0.5);
                self.send_near(centre, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_DESTROY, centre, state as i32));
                // Un seul objet pour un bloc en deux moitiés : celle du bas le rend, sauf si
                // c'est la casse de la moitié haute qui l'emporte (déjà rendu)
                let upper = d.prop(state, "upper_block_bit") == Some(Prop::Int(1));
                if !upper && !lost_top {
                    for (item, count) in self.natural_drops(state) {
                        self.drop_item(centre, (0.0, 0.1, 0.0), item, count, 10);
                    }
                }
                self.world.tiles.remove(n);
                queue.push_back(n);
            }
        }
    }

    /// Ce que rend un bloc qui se casse tout seul (comme cassé à la main).
    pub(super) fn natural_drops(&self, state: u32) -> Vec<(i16, u8)> {
        if let (Some(d), Some(blocks), 2) = (&self.data, &self.blocks, self.items_level) {
            if let Some((name, block)) = d.block_name(state).and_then(|n| blocks.get(n).map(|b| (n, b))) {
                let hand = VanillaItems::hand();
                let found: Vec<(i16, u8)> = block
                    .drops(hand.as_ref())
                    .iter()
                    .filter_map(|drop| {
                        // (objet inconnu sous ce nom — la canne à sucre — : celui que rend le bloc)
                        let id = d.item(&drop.item).map(|i| i.id).or_else(|| d.item_for_block_name(name)).or_else(|| d.drop_for_block(state));
                        id.map(|id| (id, drop.count))
                    })
                    .collect();
                // Herbes, fougères, neige : rien à la main, comme dans le jeu
                let bare = ["grass", "fern", "snow", "deadbush", "dead_bush", "leaf_litter"].iter().any(|p| name.contains(p));
                // (butin fixé par un plugin : tel quel, même vide)
                if !found.is_empty() || bare || crate::block::overrides::has_drops(name) {
                    return found;
                }
            }
        }
        self.drop_for(state).map(|i| vec![(i, 1)]).unwrap_or_default()
    }

    /// Blocs qui tombent (un tick sur deux) : chacun descend d'un bloc tant que rien ne le
    /// retient, en écrasant les plantes et les torches sur son passage.
    pub(super) fn fall_tick(&mut self) {
        if self.falling.is_empty() {
            return;
        }
        let Some(d) = self.data.clone() else {
            self.falling.clear();
            return;
        };
        let air = self.world.air;
        // Du bas vers le haut : une colonne descend d'un bloc entière, sans se rattraper
        let mut list: Vec<(i32, i32, i32)> = self.falling.iter().copied().collect();
        list.sort_by_key(|p| p.1);
        list.truncate(256);
        for p in list {
            self.falling.remove(&p);
            let state = self.world.get(p.0, p.1, p.2);
            if !d.block_name(state).map(falls).unwrap_or(false) {
                continue;
            }
            let below = (p.0, p.1 - 1, p.2);
            if !crate::world::World::in_bounds(below.1) {
                continue;
            }
            let under = self.world.get(below.0, below.1, below.2);
            let soft = self.replaceable(under) || d.block_name(under).map(|n| support_of(&d, under, n).is_some()).unwrap_or(false);
            if !soft {
                continue; // posé
            }
            if self.plugins.listens("BlockFallEvent") {
                let out = self.fire("BlockFallEvent", vec![("block", self.block_json(state, p))]);
                if cancelled(&out) {
                    continue;
                }
            }
            if !self.world.set(p.0, p.1, p.2, air) {
                continue;
            }
            self.world.set(below.0, below.1, below.2, state);
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(p.0, p.1, p.2, air), None);
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(below.0, below.1, below.2, state), None);
            self.falling.insert(below);
            // L'eau et la lave voisines comblent le trou laissé, ou contournent le bloc arrivé
            self.liquid_neighbours(p);
            self.liquid_neighbours(below);
            // Ce qui reposait sur lui suit ; ce qui s'y accrochait se décroche
            self.support_check(p);
        }
    }

    /// Un bloc vient d'être posé : s'il pèse et que rien ne le porte, il tombera.
    pub(super) fn placed_may_fall(&mut self, pos: (i32, i32, i32), state: u32) {
        if self.data.as_ref().and_then(|d| d.block_name(state)).map(falls).unwrap_or(false) {
            self.falling.insert(pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_know_what_holds_them() {
        assert!(falls("minecraft:sand") && falls("minecraft:white_concrete_powder") && !falls("minecraft:sandstone"));
        assert!(needs_floor("minecraft:poppy") && needs_floor("minecraft:oak_door") && needs_floor("minecraft:rail") && needs_floor("minecraft:white_carpet"));
        assert!(!needs_floor("minecraft:oak_trapdoor") && !needs_floor("minecraft:brown_mushroom_block") && !needs_floor("minecraft:stone") && !needs_floor("minecraft:bamboo_planks"));
        assert!(two_halves("minecraft:spruce_door") && two_halves("minecraft:sunflower") && !two_halves("minecraft:poppy"));
        assert!(is_log("minecraft:birch_log") && is_leaves("minecraft:azalea_leaves_flowered") && !is_leaves("minecraft:leaf_litter"));
    }
}

// --- Ce qui pousse et change tout seul ---------------------------------------------------------

/// Blocs tirés au hasard par chunk et par tick, dans la tranche de 48 blocs de haut autour du
/// joueur (la cadence du jeu : 3 blocs par section de 16 × 16 × 16).
const RANDOM_TICKS: u32 = 9;
/// Chunks animés autour de chaque joueur (rayon), et au plus par tick pour tout le serveur.
const GROW_RADIUS: i32 = 2;
const GROW_CHUNKS: usize = 400;

impl Server {
    /// Un tick de croissance : quelques blocs tirés au hasard autour des joueurs poussent,
    /// s'étendent ou se dessèchent, comme les « ticks aléatoires » du jeu.
    pub(super) fn random_ticks(&mut self) {
        let Some(d) = self.data.clone() else { return };
        let mut chunks: HashMap<(i32, i32), i32> = HashMap::new();
        for c in self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame) {
            let (cx, cz) = ((c.pos.0.floor() as i32) >> 4, (c.pos.2.floor() as i32) >> 4);
            for dx in -GROW_RADIUS..=GROW_RADIUS {
                for dz in -GROW_RADIUS..=GROW_RADIUS {
                    if chunks.len() < GROW_CHUNKS {
                        chunks.entry((cx + dx, cz + dz)).or_insert(c.pos.1 as i32);
                    }
                }
            }
        }
        for ((cx, cz), py) in chunks {
            // (chunk pas encore là : rien n'y pousse, plutôt que de le générer sur place)
            if !self.world.is_ready(cx, cz) {
                continue;
            }
            for _ in 0..RANDOM_TICKS {
                let x = cx * 16 + self.mob_rng.below(16) as i32;
                let z = cz * 16 + self.mob_rng.below(16) as i32;
                let y = py - 24 + self.mob_rng.below(48) as i32;
                if crate::world::World::in_bounds(y) {
                    self.random_tick_at(&d, (x, y, z));
                }
            }
        }
    }

    /// L'herbe gagne la terre voisine, et meurt sous un bloc opaque ou sous l'eau.
    fn grass_tick(&mut self, (x, y, z): (i32, i32, i32)) {
        if !self.cfg.vanilla.grass_spread {
            return;
        }
        let Some(d) = self.data.clone() else { return };
        let water = self.world.generator().blocks.water;
        let covered = |s: &Self, p: (i32, i32, i32)| {
            let above = s.world.get(p.0, p.1 + 1, p.2);
            above == water || s.light_cell(above).damp >= 15
        };
        if covered(self, (x, y, z)) {
            if let Some(dirt) = d.default_state("minecraft:dirt") {
                self.grow_to((x, y, z), dirt);
            }
            return;
        }
        let (Some(dirt), Some(grass)) = (d.default_state("minecraft:dirt"), d.default_state("minecraft:grass_block")) else { return };
        for _ in 0..4 {
            let p = (x + self.mob_rng.below(3) as i32 - 1, y + self.mob_rng.below(5) as i32 - 3, z + self.mob_rng.below(3) as i32 - 1);
            if crate::world::World::in_bounds(p.1) && self.world.is_ready(p.0 >> 4, p.2 >> 4) && self.world.get(p.0, p.1, p.2) == dirt && !covered(self, p) {
                self.grow_to(p, grass);
            }
        }
    }

    /// Poudre d'os utilisée sur un bloc : une culture gagne de deux à cinq stades, une pousse
    /// a 45 % de chances de devenir un arbre, l'herbe se couvre d'herbes hautes et de fleurs.
    /// Vrai si la poudre a servi (elle est alors consommée, hors créatif).
    pub(super) fn use_bone_meal(&mut self, addr: &SocketAddr, (x, y, z): (i32, i32, i32)) -> bool {
        let Some(c) = self.clients.get(addr) else { return false };
        if self.held_name(c) != "bone_meal" || !self.cfg.vanilla.bone_meal {
            return false;
        }
        let survival = c.mode.takes_damage();
        let Some(d) = self.data.clone() else { return false };
        let state = self.world.get(x, y, z);
        let Some(name) = d.block_name(state).map(|n| n.trim_start_matches("minecraft:").to_string()) else { return false };
        let air = self.world.air;
        let used = if is_crop(&name) || name == "melon_stem" || name == "pumpkin_stem" {
            let growth = int_of(&d, state, "growth").unwrap_or(7);
            if growth >= 7 {
                false
            } else {
                let next = (growth + 2 + self.mob_rng.below(4) as i32).min(7);
                if let Some(grown) = d.with_props(state, &[("growth", Prop::Int(next))]) {
                    self.grow_to((x, y, z), grown);
                }
                true
            }
        } else if name.ends_with("_sapling") && !name.contains("bamboo") {
            if self.mob_rng.below(100) < 45 {
                self.sapling_tick(name.trim_end_matches("_sapling").to_string(), (x, y, z), true);
            }
            true
        } else if name == "grass_block" && crate::world::World::in_bounds(y + 1) {
            // (herbes et fleurs sur l'herbe libre autour, dans un rayon de trois blocs)
            let tall = d.default_state("minecraft:short_grass").or_else(|| d.default_state("minecraft:tallgrass"));
            let flowers = [d.default_state("minecraft:dandelion").or_else(|| d.default_state("minecraft:yellow_flower")), d.default_state("minecraft:poppy").or_else(|| d.default_state("minecraft:red_flower"))];
            for _ in 0..24 {
                let (px, pz) = (x + self.mob_rng.below(7) as i32 - 3, z + self.mob_rng.below(7) as i32 - 3);
                let plant = if self.mob_rng.below(8) == 0 { flowers[self.mob_rng.below(2) as usize] } else { tall };
                if let Some(plant) = plant {
                    if self.world.is_ready(px >> 4, pz >> 4) && self.world.get(px, y, pz) == state && self.world.get(px, y + 1, pz) == air && self.mob_rng.below(3) != 0 {
                        self.grow_to((px, y + 1, pz), plant);
                    }
                }
            }
            true
        } else {
            false
        };
        if !used {
            return false;
        }
        // (les étincelles vertes de la poudre d'os)
        self.send_near((x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5), bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_PARTICLE_BONE_MEAL, (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5), 0));
        if survival {
            if let Some(c) = self.clients.get_mut(addr) {
                c.inv.consume_held();
                c.inv_touched = true;
            }
            self.send_inventory(addr);
            self.show_held(addr);
        }
        true
    }

    /// Une pousse devient un arbre, s'il a la place de pousser : le même arbre que ceux du
    /// terrain (chêne, grand chêne, bouleau, sapin, acacia, cerisier, arbre de la jungle), et
    /// les grands arbres quand quatre pousses forment un carré (grand sapin, grand arbre de la
    /// jungle, chêne noir, chêne pâle).
    fn sapling_tick(&mut self, wood: String, (x, y, z): (i32, i32, i32), forced: bool) {
        if !forced && (!self.cfg.vanilla.tree_growth || self.mob_rng.below(14) != 0) {
            return;
        }
        let Some(d) = self.data.clone() else { return };
        let air = self.world.air;
        let sapling = self.world.get(x, y, z);
        // Quatre pousses de la même essence en carré : le coin nord-ouest du carré
        let same = |s: &Self, dx: i32, dz: i32| d.block_name(s.world.get(x + dx, y, z + dz)) == d.block_name(sapling);
        let square = [(0, 0), (-1, 0), (0, -1), (-1, -1)].into_iter().find(|(ox, oz)| [(0, 0), (1, 0), (0, 1), (1, 1)].iter().all(|(dx, dz)| same(self, ox + dx, oz + dz)));
        let (wide, corner) = match square {
            Some((ox, oz)) if matches!(wood.as_str(), "spruce" | "jungle" | "dark_oak" | "pale_oak") => (true, (x + ox, y, z + oz)),
            _ => (false, (x, y, z)),
        };
        // Ce que l'arbre peut remplacer : de l'air, des feuilles, des pousses, des herbes
        let soft = |s: &Self, p: (i32, i32, i32)| {
            let state = s.world.get(p.0, p.1, p.2);
            state == air || s.replaceable(state) || d.block_name(state).map(|n| n.contains("leaves") || n.contains("sapling")).unwrap_or(false)
        };
        // Il lui faut de la place au-dessus (six blocs libres), et un chunk chargé autour
        if y + 8 > crate::world::MAX_Y || !(1..=6).all(|dy| soft(self, (corner.0, y + dy, corner.2))) {
            return;
        }
        let seed = self.mob_rng.below(u32::MAX) as u64;
        let tree = {
            let world = &self.world;
            crate::world::vanilla::grow_tree(&world.generator().blocks, &wood, wide, corner, seed, &|bx, by, bz| if world.is_ready(bx >> 4, bz >> 4) { world.get(bx, by, bz) } else { u32::MAX })
        };
        let Some(tree) = tree else { return };
        // (un plugin peut s'y opposer : c'est le tronc qui remplace la pousse)
        let Some(trunk) = tree.iter().find(|b| (b.0, b.1, b.2) == corner).map(|b| b.3) else { return };
        if !self.grow_to(corner, trunk) {
            return;
        }
        // L'arbre ne casse rien : il ne pousse que là où il y a de la place
        let list: Vec<(i32, i32, i32, u32)> = tree.into_iter().filter(|b| (b.0, b.1, b.2) != corner && self.world.is_ready(b.0 >> 4, b.2 >> 4) && soft(self, (b.0, b.1, b.2))).collect();
        self.set_blocks_from_plugin(list);
    }

    /// Change un bloc qui a poussé (les plugins peuvent s'y opposer) et l'annonce à tous.
    fn grow_to(&mut self, (x, y, z): (i32, i32, i32), state: u32) -> bool {
        if self.plugins.listens("BlockGrowEvent") {
            let out = self.fire("BlockGrowEvent", vec![("block", self.block_json(state, (x, y, z)))]);
            if cancelled(&out) {
                return false;
            }
        }
        if !self.world.set(x, y, z, state) {
            return false;
        }
        self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, state), None);
        true
    }

    /// De l'eau à moins de 4 blocs (au niveau de la terre ou juste au-dessus) ?
    fn water_near(&self, d: &crate::data::GameData, (x, y, z): (i32, i32, i32)) -> bool {
        for dy in 0..=1 {
            for dx in -4..=4 {
                for dz in -4..=4 {
                    let s = self.world.get(x + dx, y + dy, z + dz);
                    if s != self.world.air && (d.block_name(s).map(|n| n.contains("water")).unwrap_or(false) || self.world.is_wet(x + dx, y + dy, z + dz)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Œuf de tortue posé sur le sable : il se fêle peu à peu (la nuit, ou une fois sur 500 le
    /// jour, comme dans le jeu), puis éclot : autant de bébés tortues que d'œufs.
    fn turtle_egg_tick(&mut self, d: &crate::data::GameData, state: u32, (x, y, z): (i32, i32, i32)) {
        let on_sand = d.block_name(self.world.get(x, y - 1, z)).map(|n| n.contains("sand")).unwrap_or(false);
        let time = self.world_time().rem_euclid(24000);
        let night = (13000..23000).contains(&time);
        if !on_sand || !(night || self.mob_rng.below(500) == 0) {
            return;
        }
        let text = |key: &str| match d.prop(state, key) {
            Some(Prop::Str(v)) => v,
            _ => String::new(),
        };
        let next = match text("cracked_state").as_str() {
            "no_cracks" => Some("cracked"),
            "cracked" => Some("max_cracked"),
            _ => None,
        };
        let at = (x as f32 + 0.5, y as f32, z as f32 + 0.5);
        self.send_near(at, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound(if next.is_some() { "turtle_egg_crack" } else { "turtle_egg_drop" }, at, "", false, -1));
        if let Some(next) = next {
            if let Some(s) = d.with_props(state, &[("cracked_state", Prop::Str(next.to_string()))]) {
                self.change_block((x, y, z), s);
            }
            return;
        }
        let count = match text("turtle_egg_count").as_str() {
            "four_egg" => 4,
            "three_egg" => 3,
            "two_egg" => 2,
            _ => 1,
        };
        self.change_block((x, y, z), self.world.air);
        for _ in 0..count {
            self.hatch_turtle(at);
        }
    }

    fn random_tick_at(&mut self, d: &crate::data::GameData, (x, y, z): (i32, i32, i32)) {
        let air = self.world.air;
        let state = self.world.get(x, y, z);
        if state == air {
            return;
        }
        // La lave enflamme ce qui brûle près d'elle ; un feu que le serveur ne suivait pas
        // (posé avant un redémarrage) est repris en charge
        if let Some(name) = d.block_name(state) {
            if name.contains("lava") {
                return self.lava_sparks((x, y, z));
            }
            if name == "minecraft:fire" {
                self.fires.insert((x, y, z));
                return;
            }
            if name == "minecraft:grass_block" {
                return self.grass_tick((x, y, z));
            }
            if name == "minecraft:turtle_egg" {
                return self.turtle_egg_tick(d, state, (x, y, z));
            }
            if name.ends_with("_sapling") && !name.contains("bamboo") {
                return self.sapling_tick(name.trim_start_matches("minecraft:").trim_end_matches("_sapling").to_string(), (x, y, z), false);
            }
        }
        // Culture créée par un plugin : un stade de plus de temps en temps, jusqu'à maturité
        if let Some((custom, stage)) = crate::block::custom::variant_of(state) {
            if matches!(custom.variants, crate::block::custom::Variants::Stages(_)) && stage + 1 < custom.variants.count() && self.mob_rng.below(4) == 0 {
                self.grow_to((x, y, z), custom.state(stage + 1));
            }
            return;
        }
        let Some(name) = d.block_name(state).map(|n| n.trim_start_matches("minecraft:").to_string()) else { return };
        let int = |key: &str| match d.prop(state, key) {
            Some(Prop::Int(v)) => Some(v),
            _ => None,
        };
        let above = if y < 319 { self.world.get(x, y + 1, z) } else { air };
        match name.as_str() {
            // Cultures : huit stades ; plus vite sur une terre arrosée
            n if is_crop(n) || n == "melon_stem" || n == "pumpkin_stem" => {
                let growth = int("growth").unwrap_or(7);
                if growth < 7 && self.cfg.vanilla.crop_growth {
                    let wet = int_of(d, self.world.get(x, y - 1, z), "moisturized_amount").unwrap_or(0) > 0;
                    if self.mob_rng.below(if wet { 3 } else { 8 }) == 0 {
                        if let Some(next) = d.with_props(state, &[("growth", Prop::Int(growth + 1))]) {
                            self.grow_to((x, y, z), next);
                        }
                    }
                } else if n.ends_with("_stem") && self.mob_rng.below(3) == 0 {
                    // Tige mûre : un fruit sur un bloc de terre voisin, s'il y a la place
                    let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][self.mob_rng.below(4) as usize];
                    let spot = (x + dx, y, z + dz);
                    let ground = d.block_name(self.world.get(spot.0, y - 1, spot.2)).unwrap_or("").trim_start_matches("minecraft:").to_string();
                    let fruit = if n == "melon_stem" { "minecraft:melon_block" } else { "minecraft:pumpkin" };
                    if self.world.get(spot.0, y, spot.2) == air && matches!(ground.as_str(), "grass_block" | "dirt" | "farmland" | "coarse_dirt" | "podzol") {
                        if let Some(block) = d.default_state(fruit) {
                            self.grow_to(spot, block);
                        }
                    }
                }
            }
            // Canne à sucre et cactus : jusqu'à trois de haut
            "reeds" | "cactus" => {
                if above != air {
                    return;
                }
                let below = (1..3).take_while(|h| d.block_name(self.world.get(x, y - h, z)).map(|b| b.trim_start_matches("minecraft:") == name).unwrap_or(false)).count();
                if below >= 2 {
                    return;
                }
                let age = int("age").unwrap_or(0);
                if age < 15 {
                    if let Some(next) = d.with_props(state, &[("age", Prop::Int(age + 1))]) {
                        self.world.set(x, y, z, next); // (rien à voir : pas d'annonce)
                    }
                } else if let (Some(young), Some(top)) = (d.with_props(state, &[("age", Prop::Int(0))]), d.default_state(&format!("minecraft:{}", name))) {
                    if self.grow_to((x, y + 1, z), top) {
                        self.world.set(x, y, z, young);
                    }
                }
            }
            // L'herbe gagne la terre voisine à l'air libre
            "grass_block" => {
                let spot = (x + self.mob_rng.below(3) as i32 - 1, y + self.mob_rng.below(5) as i32 - 3, z + self.mob_rng.below(3) as i32 - 1);
                if !crate::world::World::in_bounds(spot.1) || spot.1 >= 319 {
                    return;
                }
                let target = self.world.get(spot.0, spot.1, spot.2);
                if d.block_name(target) == Some("minecraft:dirt") && self.world.get(spot.0, spot.1 + 1, spot.2) == air {
                    self.grow_to(spot, state);
                }
            }
            // Terre labourée : arrosée près de l'eau, sinon elle sèche puis redevient de la terre
            "farmland" => {
                let moist = int("moisturized_amount").unwrap_or(0);
                if self.water_near(d, (x, y, z)) || self.rained_on((x, y + 1, z)) {
                    if moist < 7 {
                        if let Some(next) = d.with_props(state, &[("moisturized_amount", Prop::Int(7))]) {
                            self.grow_to((x, y, z), next);
                        }
                    }
                } else if moist > 0 {
                    if let Some(next) = d.with_props(state, &[("moisturized_amount", Prop::Int(moist - 1))]) {
                        self.grow_to((x, y, z), next);
                    }
                } else {
                    let planted = d.block_name(above).map(|n| support_of(d, above, n).is_some()).unwrap_or(false);
                    if !planted {
                        if let Some(dirt) = d.default_state("minecraft:dirt") {
                            self.grow_to((x, y, z), dirt);
                        }
                    }
                }
            }
            "sweet_berry_bush" | "nether_wart" => {
                let key = if name == "nether_wart" { "age" } else { "growth" };
                let now = int(key).unwrap_or(3);
                if now < 3 && self.mob_rng.below(if name == "nether_wart" { 10 } else { 5 }) == 0 {
                    if let Some(next) = d.with_props(state, &[(key, Prop::Int(now + 1))]) {
                        self.grow_to((x, y, z), next);
                    }
                }
            }
            _ => {}
        }
    }

    /// Récolte d'une culture cassée à la main : mûre, elle rend sa récolte et de quoi replanter ;
    /// sinon seulement la graine. None : ce n'est pas une culture.
    pub(super) fn crop_loot(&mut self, state: u32, fortune: u8) -> Option<Vec<(i16, u8)>> {
        let d = self.data.clone()?;
        let name = d.block_name(state)?.trim_start_matches("minecraft:").to_string();
        if !is_crop(&name) {
            return None;
        }
        let ripe = d.prop(state, "growth") == Some(Prop::Int(7));
        // (fortune : jusqu'à niveau graines ou légumes de plus)
        let extra = |s: &mut Server, lo: u32, hi: u32| (lo + s.mob_rng.below(hi + fortune as u32 - lo + 1)) as u8;
        let list: Vec<(&str, u8)> = match (name.as_str(), ripe) {
            ("wheat", true) => vec![("wheat", 1), ("wheat_seeds", extra(self, 0, 3))],
            ("wheat", false) => vec![("wheat_seeds", 1)],
            ("carrots", true) => vec![("carrot", extra(self, 2, 5))],
            ("carrots", false) => vec![("carrot", 1)],
            ("potatoes", true) => vec![("potato", extra(self, 2, 5))],
            ("potatoes", false) => vec![("potato", 1)],
            ("beetroot", true) => vec![("beetroot", 1), ("beetroot_seeds", extra(self, 1, 4))],
            _ => vec![("beetroot_seeds", 1)],
        };
        Some(list.into_iter().filter(|(_, n)| *n > 0).filter_map(|(item, n)| self.item_id(item).map(|id| (id, n))).collect())
    }
}

/// Propriété entière d'un état de bloc.
fn int_of(d: &crate::data::GameData, state: u32, key: &str) -> Option<i32> {
    match d.prop(state, key) {
        Some(Prop::Int(v)) => Some(v),
        _ => None,
    }
}
