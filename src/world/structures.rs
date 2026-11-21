//! Structures du terrain vanilla : villages, pyramides du désert, temples de la jungle et
//! mines abandonnées.
//!
//! Les modèles officiels du jeu (fichiers de structures) ne sont pas disponibles : chaque
//! structure est reconstruite à la main, au plus près de celle du jeu.
//!
//! Comme le reste du terrain, tout se déduit de la graine. Une structure est d'abord
//! calculée en entier (un « plan » : les blocs rangés par chunk, et ses coffres), une seule
//! fois, puis chaque chunk qu'elle touche y prend sa part au moment de sa génération. Les
//! emplacements suivent une grille comme dans le jeu : un village au plus par carré de 34
//! chunks de côté, un temple par carré de 32, et une mine dans environ 0,4 % des chunks.
//!
//! Les coffres sont vides dans le terrain : leur butin est tiré au premier clic du joueur
//! (`loot`), à partir de leur position, donc le même pour une graine donnée.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::generator::{mix, Palette, Rng, SEA_LEVEL};
use super::vanilla::{Vanilla, B};
use super::{MAX_Y, MIN_Y};

/// Ce que contient un coffre de structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loot {
    Village,
    DesertPyramid,
    JungleTemple,
    Mineshaft,
}

/// Une structure calculée : ses blocs et ses coffres, rangés par chunk.
#[derive(Default)]
pub struct Plan {
    blocks: HashMap<(i32, i32), Vec<(u8, i16, u8, u32)>>,
    chests: HashMap<(i32, i32), Vec<(u8, i16, u8, Loot)>>,
}

impl Plan {
    pub fn blocks_in(&self, cx: i32, cz: i32) -> &[(u8, i16, u8, u32)] {
        self.blocks.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn chests_in(&self, cx: i32, cz: i32) -> &[(u8, i16, u8, Loot)] {
        self.chests.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

// --- Emplacements -----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Village,
    DesertPyramid,
    JungleTemple,
    Mineshaft,
}

/// Plans déjà calculés (y compris « rien ici »), partagés par les fils de génération.
#[derive(Default)]
pub struct PlanCache {
    plans: Mutex<HashMap<(Kind, i32, i32), Arc<Plan>>>,
}

const CACHE_PLANS: usize = 512;

/// Chunk de départ d'une structure dans une case de la grille (`spacing` chunks de côté,
/// au moins `separation` chunks entre deux structures).
fn grid_start(seed: u64, rx: i32, rz: i32, spacing: i32, separation: i32, salt: u64) -> (i32, i32) {
    let mut r = Rng::new(mix(seed, rx as i64, rz as i64, salt));
    let range = (spacing - separation) as u32;
    (rx * spacing + r.below(range) as i32, rz * spacing + r.below(range) as i32)
}

/// Les structures dont un morceau tombe dans ce chunk.
pub fn near(v: &Vanilla, p: &Palette, cache: &PlanCache, cx: i32, cz: i32) -> Vec<Arc<Plan>> {
    let seed = v.seed();
    let mut starts: Vec<(Kind, i32, i32)> = Vec::new();
    // (sorte, espacement, séparation, sel, portée en chunks)
    for (kind, spacing, separation, salt, reach) in [
        (Kind::Village, 34, 8, 10_387_312u64, 6),
        (Kind::DesertPyramid, 32, 8, 14_357_617, 2),
        (Kind::JungleTemple, 32, 8, 14_357_619, 2),
    ] {
        let (rx0, rz0) = (cx.div_euclid(spacing), cz.div_euclid(spacing));
        for rx in rx0 - 1..=rx0 + 1 {
            for rz in rz0 - 1..=rz0 + 1 {
                let (sx, sz) = grid_start(seed, rx, rz, spacing, separation, salt);
                if (sx - cx).abs() <= reach && (sz - cz).abs() <= reach {
                    starts.push((kind, sx, sz));
                }
            }
        }
    }
    for dx in -5..=5 {
        for dz in -5..=5 {
            if mix(seed, (cx + dx) as i64, (cz + dz) as i64, 77_001) % 1000 < 4 {
                starts.push((Kind::Mineshaft, cx + dx, cz + dz));
            }
        }
    }
    let mut out = Vec::new();
    for key in starts {
        let known = cache.plans.lock().ok().and_then(|m| m.get(&key).cloned());
        let plan = match known {
            Some(plan) => plan,
            None => {
                let plan = Arc::new(build(v, p, key));
                if let Ok(mut m) = cache.plans.lock() {
                    if m.len() >= CACHE_PLANS {
                        m.clear();
                    }
                    m.insert(key, plan.clone());
                }
                plan
            }
        };
        if !plan.is_empty() {
            out.push(plan);
        }
    }
    out
}

fn build(v: &Vanilla, p: &Palette, (kind, sx, sz): (Kind, i32, i32)) -> Plan {
    let mut b = Builder::new(v, p);
    let mut rng = Rng::new(mix(v.seed(), sx as i64, sz as i64, 31_337 + kind as u64));
    let (x, z) = (sx * 16 + 8, sz * 16 + 8);
    match kind {
        Kind::Village => {
            let biome = b.biome(x, z);
            if let Some(style) = Style::for_biome(biome) {
                village(&mut b, &mut rng, x, z, &style);
            }
        }
        Kind::DesertPyramid => {
            if b.biome(x, z) == B::Desert && b.top(x, z) > SEA_LEVEL {
                desert_pyramid(&mut b, x - 10, z - 10);
            }
        }
        Kind::JungleTemple => {
            if matches!(b.biome(x, z), B::Jungle | B::BambooJungle) && b.top(x, z) > SEA_LEVEL {
                jungle_temple(&mut b, &mut rng, x - 6, z - 7);
            }
        }
        Kind::Mineshaft => mineshaft(&mut b, &mut rng, x, z),
    }
    b.finish()
}

// --- Construction -----------------------------------------------------------------------

/// Directions, dans le sens des aiguilles d'une montre.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum D {
    N = 0,
    E = 1,
    S = 2,
    W = 3,
}

impl D {
    fn from(n: u8) -> D {
        [D::N, D::E, D::S, D::W][(n & 3) as usize]
    }
    fn turn(self, quarter: u8) -> D {
        D::from(self as u8 + quarter)
    }
    fn step(self) -> (i32, i32) {
        match self {
            D::N => (0, -1),
            D::E => (1, 0),
            D::S => (0, 1),
            D::W => (-1, 0),
        }
    }
    fn name(self) -> &'static str {
        ["north", "east", "south", "west"][self as usize]
    }
    /// `weirdo_direction` d'un escalier qui monte vers cette direction
    fn stairs(self) -> u8 {
        match self {
            D::E => 0,
            D::W => 1,
            D::S => 2,
            D::N => 3,
        }
    }
    /// `minecraft:cardinal_direction` d'une porte dont l'extérieur est de ce côté (relevé
    /// sur les portes posées par un joueur : un quart de tour par rapport au regard)
    fn door(self) -> &'static str {
        match self {
            D::S => "east",
            D::N => "west",
            D::E => "north",
            D::W => "south",
        }
    }
    /// `facing_direction` (échelle, distributeur) tourné vers cette direction
    fn facing(self) -> u8 {
        match self {
            D::N => 2,
            D::S => 3,
            D::W => 4,
            D::E => 5,
        }
    }
}

/// Repère d'un bâtiment : origine et rotation. Dans le repère local, `x` va vers la droite
/// et `z` vers la façade (le « sud » local), où se trouve la porte.
#[derive(Clone, Copy)]
struct Frame {
    x: i32,
    y: i32,
    z: i32,
    rot: u8,
}

impl Frame {
    fn at(&self, lx: i32, ly: i32, lz: i32) -> (i32, i32, i32) {
        let (dx, dz) = match self.rot & 3 {
            0 => (lx, lz),
            1 => (-lz, lx),
            2 => (-lx, -lz),
            _ => (lz, -lx),
        };
        (self.x + dx, self.y + ly, self.z + dz)
    }
    fn dir(&self, local: D) -> D {
        local.turn(self.rot)
    }
}

struct Builder<'a> {
    v: &'a Vanilla,
    p: &'a Palette,
    blocks: HashMap<(i32, i32, i32), u32>,
    chests: Vec<((i32, i32, i32), Loot)>,
    tops: HashMap<(i32, i32), i32>,
}

impl<'a> Builder<'a> {
    fn new(v: &'a Vanilla, p: &'a Palette) -> Builder<'a> {
        Builder { v, p, blocks: HashMap::new(), chests: Vec::new(), tops: HashMap::new() }
    }

    fn id(&self, name: &str) -> u32 {
        match self.p.named.get(name) {
            Some(s) => *s,
            None => {
                debug_assert!(false, "bloc de structure non déclaré : {}", name);
                self.p.stone
            }
        }
    }

    fn put(&mut self, (x, y, z): (i32, i32, i32), state: u32) {
        if (MIN_Y + 1..=MAX_Y).contains(&y) {
            self.blocks.insert((x, y, z), state);
        }
    }

    fn set(&mut self, pos: (i32, i32, i32), name: &str) {
        let s = self.id(name);
        self.put(pos, s);
    }

    fn air(&mut self, pos: (i32, i32, i32)) {
        let a = self.p.air;
        self.put(pos, a);
    }

    /// Hauteur du sol (dernier bloc du relief) d'une colonne.
    fn top(&mut self, x: i32, z: i32) -> i32 {
        if let Some(t) = self.tops.get(&(x, z)) {
            return *t;
        }
        let t = self.v.surface(x, z).0;
        self.tops.insert((x, z), t);
        t
    }

    fn biome(&self, x: i32, z: i32) -> B {
        self.v.surface(x, z).1
    }

    fn chest(&mut self, pos: (i32, i32, i32), front: D, loot: Loot) {
        self.set(pos, &format!("chest[minecraft:cardinal_direction={}]", front.name()));
        self.chests.push((pos, loot));
    }

    fn finish(self) -> Plan {
        let mut plan = Plan::default();
        for ((x, y, z), s) in self.blocks {
            plan.blocks.entry((x >> 4, z >> 4)).or_default().push(((x & 15) as u8, y as i16, (z & 15) as u8, s));
        }
        for ((x, y, z), loot) in self.chests {
            plan.chests.entry((x >> 4, z >> 4)).or_default().push(((x & 15) as u8, y as i16, (z & 15) as u8, loot));
        }
        plan
    }

    /// Socle sous un bâtiment : de la surface du terrain jusque sous le plancher.
    fn foundation(&mut self, pos: (i32, i32, i32), name: &str) {
        let (x, y, z) = pos;
        let ground = self.top(x, z);
        let s = self.id(name);
        for yy in (ground - 1).min(y - 1).max(MIN_Y + 1)..y {
            self.put((x, yy, z), s);
        }
    }
}

// --- Blocs employés ---------------------------------------------------------------------

const WOODS: [&str; 3] = ["oak", "spruce", "acacia"];
const STAIRS: [&str; 6] = ["oak", "spruce", "acacia", "stone", "sandstone", "mossy_cobblestone"];
const DOORS: [&str; 3] = ["wooden_door", "spruce_door", "acacia_door"];
const PLAIN: [&str; 41] = [
    "oak_planks", "spruce_planks", "acacia_planks", "oak_log", "spruce_log", "acacia_log", "stripped_oak_log", "cobblestone", "mossy_cobblestone",
    "stone_bricks", "mossy_stone_bricks", "cracked_stone_bricks", "chiseled_stone_bricks", "sandstone", "cut_sandstone", "chiseled_sandstone",
    "smooth_sandstone", "orange_terracotta", "blue_terracotta", "white_terracotta", "glass", "crafting_table", "bookshelf", "smithing_table",
    "cartography_table", "fletching_table", "web", "grass_path", "dirt", "white_wool", "yellow_wool", "white_carpet", "oak_fence", "spruce_fence",
    "acacia_fence", "water", "lava", "stone", "tnt", "gravel", "coarse_dirt",
];

/// Tous les états de blocs que les structures emploient, sous la forme acceptée par la
/// palette (`nom[propriété=valeur,…]`).
pub fn block_specs() -> Vec<String> {
    let mut out: Vec<String> = PLAIN.iter().map(|s| s.to_string()).collect();
    for m in STAIRS {
        for d in [D::N, D::E, D::S, D::W] {
            for up in 0..2 {
                out.push(format!("{}_stairs[weirdo_direction={},upside_down_bit={}]", m, d.stairs(), up));
            }
        }
    }
    for door in DOORS {
        for d in [D::N, D::E, D::S, D::W] {
            for upper in 0..2 {
                out.push(format!("{}[minecraft:cardinal_direction={},upper_block_bit={}]", door, d.door(), upper));
            }
        }
    }
    for d in [D::N, D::E, D::S, D::W] {
        out.push(format!("chest[minecraft:cardinal_direction={}]", d.name()));
        out.push(format!("furnace[minecraft:cardinal_direction={}]", d.name()));
        out.push(format!("ladder[facing_direction={}]", d.facing()));
        out.push(format!("dispenser[facing_direction={}]", d.facing()));
    }
    for w in WOODS {
        for axis in ["x", "z"] {
            out.push(format!("{}_log[pillar_axis={}]", w, axis));
        }
        out.push(format!("{}_slab[minecraft:vertical_half=bottom]", w));
        out.push(format!("{}_slab[minecraft:vertical_half=top]", w));
    }
    for (pane, conn) in [("ew", "minecraft:connection_east=1,minecraft:connection_west=1"), ("ns", "minecraft:connection_north=1,minecraft:connection_south=1")] {
        let _ = pane;
        out.push(format!("glass_pane[{}]", conn));
    }
    out.extend(
        [
            "torch[torch_facing_direction=top]",
            "lantern[hanging=0]",
            "lantern[hanging=1]",
            "rail[rail_direction=0]",
            "rail[rail_direction=1]",
            "farmland[moisturized_amount=7]",
            "wheat[growth=7]",
            "wheat[growth=4]",
            "carrots[growth=7]",
            "potatoes[growth=7]",
            "beetroot[growth=7]",
            "hay_block[pillar_axis=y]",
            "hay_block[pillar_axis=x]",
            "snow_layer[height=0]",
            "cobblestone_wall[wall_post_bit=1]",
            "cobblestone_slab[minecraft:vertical_half=bottom]",
            "stone_brick_slab[minecraft:vertical_half=bottom]",
            "sandstone_slab[minecraft:vertical_half=bottom]",
            "stone_pressure_plate",
            "cauldron[cauldron_liquid=water,fill_level=6]",
            "composter",
            "barrel[facing_direction=1]",
            "bell[attachment=standing]",
            "vine[vine_direction_bits=1]",
            "vine[vine_direction_bits=2]",
            "vine[vine_direction_bits=4]",
            "vine[vine_direction_bits=8]",
            "tripwire_hook[direction=0]",
            "lever[lever_direction=north]",
            "campfire[extinguished=0]",
            "flower_pot",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    out
}

// --- Villages ---------------------------------------------------------------------------

/// Matériaux d'un village, selon le biome.
struct Style {
    planks: &'static str,
    log: &'static str,
    wood: &'static str,
    door: &'static str,
    fence: &'static str,
    base: &'static str,
    path: &'static str,
    /// Toits plats en grès (désert) au lieu de toits en pente
    flat_roof: bool,
    snow: bool,
}

impl Style {
    fn for_biome(b: B) -> Option<Style> {
        let wood = |wood: &'static str, planks, log, door, fence| Style { planks, log, wood, door, fence, base: "cobblestone", path: "grass_path", flat_roof: false, snow: false };
        Some(match b {
            B::Plains | B::SunflowerPlains | B::Meadow => wood("oak", "oak_planks", "oak_log", "wooden_door", "oak_fence"),
            B::Savanna | B::SavannaPlateau => wood("acacia", "acacia_planks", "acacia_log", "acacia_door", "acacia_fence"),
            B::Taiga => wood("spruce", "spruce_planks", "spruce_log", "spruce_door", "spruce_fence"),
            B::SnowyPlains => Style { snow: true, ..wood("spruce", "spruce_planks", "spruce_log", "spruce_door", "spruce_fence") },
            B::Desert => Style {
                planks: "cut_sandstone",
                log: "smooth_sandstone",
                wood: "sandstone",
                door: "wooden_door",
                fence: "oak_fence",
                base: "sandstone",
                path: "smooth_sandstone",
                flat_roof: true,
                snow: false,
            },
            _ => return None,
        })
    }
}

/// Sorte de bâtiment, avec sa taille (largeur, profondeur).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Building {
    SmallHouse,
    BigHouse,
    Farm,
    Smithy,
    Tower,
    Pen,
    Lamp,
}

impl Building {
    fn size(self) -> (i32, i32) {
        match self {
            Building::SmallHouse => (5, 5),
            Building::BigHouse => (7, 9),
            Building::Farm => (9, 7),
            Building::Smithy => (7, 6),
            Building::Tower => (5, 7),
            Building::Pen => (7, 7),
            Building::Lamp => (1, 1),
        }
    }
}

/// Rayon maximal d'un village autour de son puits.
const VILLAGE_RADIUS: i32 = 72;

fn village(b: &mut Builder, rng: &mut Rng, cx: i32, cz: i32, style: &Style) {
    let y0 = b.top(cx, cz);
    if y0 <= SEA_LEVEL || y0 > SEA_LEVEL + 60 {
        return;
    }
    let mut taken: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    // Puits au centre (4×4)
    let well = Frame { x: cx - 2, y: y0, z: cz - 2, rot: 0 };
    village_well(b, well, style);
    for x in cx - 3..=cx + 2 {
        for z in cz - 3..=cz + 2 {
            taken.insert((x, z));
        }
    }
    // Rues : quatre depuis le puits, puis des rues secondaires
    let mut streets: Vec<(i32, i32, D, i32, u8)> = Vec::new();
    for d in [D::N, D::E, D::S, D::W] {
        let (dx, dz) = d.step();
        streets.push((cx + dx * 3 - if dx > 0 { 1 } else { 0 }, cz + dz * 3 - if dz > 0 { 1 } else { 0 }, d, 16 + rng.below(20) as i32, 0));
    }
    let mut buildings = 0;
    while let Some((sx, sz, dir, len, depth)) = streets.pop() {
        let (dx, dz) = dir.step();
        let (px, pz) = dir.turn(1).step(); // perpendiculaire
        let mut next_plot = 3 + rng.below(3) as i32;
        for i in 0..len {
            let (x, z) = (sx + dx * i, sz + dz * i);
            if (x - cx).abs() > VILLAGE_RADIUS || (z - cz).abs() > VILLAGE_RADIUS {
                break;
            }
            // Pas de rue dans l'eau ni dans un bâtiment
            let ground = b.top(x, z);
            if ground < SEA_LEVEL || (ground - y0).abs() > 12 {
                break;
            }
            for w in -1..=1 {
                let (rx, rz) = (x + px * w, z + pz * w);
                if taken.contains(&(rx, rz)) && i > 1 {
                    continue;
                }
                taken.insert((rx, rz));
                let t = b.top(rx, rz);
                if t >= SEA_LEVEL {
                    b.set((rx, t, rz), style.path);
                    b.air((rx, t + 1, rz));
                    b.air((rx, t + 2, rz));
                }
            }
            if i == next_plot {
                next_plot = i + 7 + rng.below(5) as i32;
                for side in [1i32, -1] {
                    if buildings >= 26 {
                        break;
                    }
                    let kind = pick_building(rng);
                    // Façade tournée vers la rue
                    let towards = if side == 1 { dir.turn(3) } else { dir.turn(1) };
                    if try_building(b, &mut taken, kind, style, rng, (x + px * side * 2, z + pz * side * 2), towards, y0) {
                        buildings += 1;
                    }
                }
                // Rue secondaire
                if depth < 2 && rng.below(3) == 0 {
                    let side = if rng.below(2) == 0 { dir.turn(1) } else { dir.turn(3) };
                    let (ox, oz) = side.step();
                    streets.push((x + ox * 2, z + oz * 2, side, 10 + rng.below(14) as i32, depth + 1));
                }
            } else if i % 9 == 4 && rng.below(2) == 0 {
                let side = if rng.below(2) == 0 { 1 } else { -1 };
                let towards = if side == 1 { dir.turn(3) } else { dir.turn(1) };
                try_building(b, &mut taken, Building::Lamp, style, rng, (x + px * side * 2, z + pz * side * 2), towards, y0);
            }
        }
    }
}

fn pick_building(rng: &mut Rng) -> Building {
    match rng.below(100) {
        0..=34 => Building::SmallHouse,
        35..=54 => Building::BigHouse,
        55..=71 => Building::Farm,
        72..=81 => Building::Smithy,
        82..=89 => Building::Tower,
        _ => Building::Pen,
    }
}

/// Pose un bâtiment dont la porte donne sur `door` (la case qui touche la rue), façade
/// tournée vers `facing`. Faux si la place est prise ou le terrain trop accidenté.
#[allow(clippy::too_many_arguments)]
fn try_building(b: &mut Builder, taken: &mut std::collections::HashSet<(i32, i32)>, kind: Building, style: &Style, rng: &mut Rng, door: (i32, i32), facing: D, y0: i32) -> bool {
    let (w, d) = kind.size();
    let rot = (facing as u8 + 2) & 3; // la façade locale (sud) tournée vers `facing`
    let probe = Frame { x: 0, y: 0, z: 0, rot };
    let (ox, _, oz) = probe.at(w / 2, 0, d - 1);
    let frame0 = Frame { x: door.0 - ox, y: 0, z: door.1 - oz, rot };
    // Emprise (avec une case de marge) libre, sur la terre ferme, sans trop de dénivelé
    let mut cells = Vec::new();
    // (pas de marge côté façade : elle donne sur la rue)
    for lx in -1..=w {
        for lz in -1..d {
            let (x, _, z) = frame0.at(lx, 0, lz);
            if taken.contains(&(x, z)) {
                return false;
            }
            cells.push((x, z));
        }
    }
    let corners = [frame0.at(0, 0, 0), frame0.at(w - 1, 0, 0), frame0.at(0, 0, d - 1), frame0.at(w - 1, 0, d - 1), frame0.at(w / 2, 0, d / 2)];
    let tops: Vec<i32> = corners.iter().map(|(x, _, z)| b.top(*x, *z)).collect();
    let (lo, hi) = (*tops.iter().min().unwrap_or(&0), *tops.iter().max().unwrap_or(&0));
    if lo < SEA_LEVEL || hi - lo > 4 || (b.top(door.0, door.1) - y0).abs() > 10 {
        return false;
    }
    let (cx, _, cz) = corners[4];
    if matches!(b.biome(cx, cz), B::River | B::FrozenRiver | B::Beach) {
        return false;
    }
    for c in cells {
        taken.insert(c);
    }
    let floor = b.top(door.0, door.1).max(SEA_LEVEL);
    let f = Frame { y: floor, ..frame0 };
    match kind {
        Building::SmallHouse => house(b, f, style, rng, 5, 5, false),
        Building::BigHouse => house(b, f, style, rng, 7, 9, true),
        Building::Farm => farm(b, f, style, rng),
        Building::Smithy => smithy(b, f, style),
        Building::Tower => tower(b, f, style),
        Building::Pen => pen(b, f, style, rng),
        Building::Lamp => lamp(b, f, style),
    }
    true
}

/// Dégage le volume d'un bâtiment et pose son socle.
fn prepare(b: &mut Builder, f: Frame, w: i32, d: i32, height: i32, base: &str) {
    for lx in -1..=w {
        for lz in -1..=d {
            for ly in 1..=height {
                b.air(f.at(lx, ly, lz));
            }
        }
    }
    for lx in 0..w {
        for lz in 0..d {
            b.foundation(f.at(lx, 0, lz), base);
        }
    }
}

fn village_well(b: &mut Builder, f: Frame, s: &Style) {
    prepare(b, f, 4, 4, 5, s.base);
    for lx in 0..4 {
        for lz in 0..4 {
            let edge = lx == 0 || lx == 3 || lz == 0 || lz == 3;
            b.set(f.at(lx, 0, lz), if edge { s.base } else { "water" });
            if !edge {
                b.set(f.at(lx, -1, lz), "water");
                b.set(f.at(lx, -2, lz), "water");
                b.set(f.at(lx, -3, lz), s.base);
            }
            b.set(f.at(lx, 4, lz), s.base);
        }
    }
    for (lx, lz) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
        for ly in 1..=3 {
            b.set(f.at(lx, ly, lz), s.fence);
        }
    }
    if !s.flat_roof {
        b.set(f.at(1, 5, 1), "bell[attachment=standing]");
    }
}

/// Maison : plancher, murs à poteaux, fenêtres, porte, toit en pente (ou plat au désert).
#[allow(clippy::too_many_arguments)]
fn house(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng, w: i32, d: i32, big: bool) {
    let wall_top = if big { 4 } else { 3 };
    prepare(b, f, w, d, wall_top + (w + 1) / 2 + 3, s.base);
    for lx in 0..w {
        for lz in 0..d {
            b.set(f.at(lx, 0, lz), if s.flat_roof { "smooth_sandstone" } else { s.planks });
            let corner = (lx == 0 || lx == w - 1) && (lz == 0 || lz == d - 1);
            let edge = lx == 0 || lx == w - 1 || lz == 0 || lz == d - 1;
            for ly in 1..=wall_top {
                if corner {
                    b.set(f.at(lx, ly, lz), s.log);
                } else if edge {
                    let window = ly == 2 && ((lx == 0 || lx == w - 1) && lz % 2 == 0 || (lz == 0 && lx % 2 == 1));
                    if window {
                        let along_x = lz == 0 || lz == d - 1;
                        let local_ew = along_x;
                        let pane = match (f.rot % 2 == 0) == local_ew {
                            true => "glass_pane[minecraft:connection_east=1,minecraft:connection_west=1]",
                            false => "glass_pane[minecraft:connection_north=1,minecraft:connection_south=1]",
                        };
                        b.set(f.at(lx, ly, lz), pane);
                    } else {
                        b.set(f.at(lx, ly, lz), if ly == 1 && !s.flat_roof { s.base } else { s.planks });
                    }
                }
            }
        }
    }
    // Porte au milieu de la façade
    let door = f.dir(D::S);
    b.set(f.at(w / 2, 1, d - 1), &format!("{}[minecraft:cardinal_direction={},upper_block_bit=0]", s.door, door.door()));
    b.set(f.at(w / 2, 2, d - 1), &format!("{}[minecraft:cardinal_direction={},upper_block_bit=1]", s.door, door.door()));
    // Marche devant la porte
    let step = f.at(w / 2, 0, d);
    if b.top(step.0, step.2) < step.1 {
        b.set(step, s.base);
    }
    // Intérieur
    b.set(f.at(1, 1, 1), "crafting_table");
    if big {
        b.chest(f.at(w - 2, 1, 1), f.dir(D::S), Loot::Village);
        b.set(f.at(w - 2, 1, d - 3), "bookshelf");
        b.set(f.at(1, 1, d - 3), &format!("furnace[minecraft:cardinal_direction={}]", f.dir(D::E).name()));
    } else if rng.below(3) == 0 {
        b.chest(f.at(w - 2, 1, 1), f.dir(D::S), Loot::Village);
    }
    b.set(f.at(w - 2, 1, d - 2), "lantern[hanging=0]");
    roof(b, f, s, w, d, wall_top + 1);
}

/// Toit à deux pentes (faîte dans la profondeur), avec débord ; plat au désert.
fn roof(b: &mut Builder, f: Frame, s: &Style, w: i32, d: i32, base: i32) {
    if s.flat_roof {
        for lx in 0..w {
            for lz in 0..d {
                b.set(f.at(lx, base, lz), "smooth_sandstone");
                let edge = lx == 0 || lx == w - 1 || lz == 0 || lz == d - 1;
                if edge {
                    b.set(f.at(lx, base + 1, lz), "sandstone_slab[minecraft:vertical_half=bottom]");
                }
            }
        }
        return;
    }
    let (left, right) = (f.dir(D::E), f.dir(D::W));
    for k in 0..=(w + 1) / 2 {
        let ly = base + k;
        let (xl, xr) = (k - 1, w - k);
        if xl > xr {
            break;
        }
        for lz in -1..=d {
            if xl == xr {
                b.set(f.at(xl, ly, lz), &format!("{}_slab[minecraft:vertical_half=bottom]", if s.wood == "sandstone" { "oak" } else { s.wood }));
            } else {
                b.set(f.at(xl, ly, lz), &format!("{}_stairs[weirdo_direction={},upside_down_bit=0]", s.wood, left.stairs()));
                b.set(f.at(xr, ly, lz), &format!("{}_stairs[weirdo_direction={},upside_down_bit=0]", s.wood, right.stairs()));
            }
            if s.snow {
                b.set(f.at(xl, ly + 1, lz), "snow_layer[height=0]");
                b.set(f.at(xr, ly + 1, lz), "snow_layer[height=0]");
            }
        }
        // Pignons
        for lx in (xl + 1)..xr {
            b.set(f.at(lx, ly, 0), s.planks);
            b.set(f.at(lx, ly, d - 1), s.planks);
        }
    }
}

fn farm(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng) {
    prepare(b, f, 9, 7, 3, "dirt");
    let crop = ["wheat[growth=7]", "wheat[growth=4]", "carrots[growth=7]", "potatoes[growth=7]", "beetroot[growth=7]"][rng.below(5) as usize];
    let log_x = format!("{}[pillar_axis=x]", if s.flat_roof { "oak_log" } else { s.log });
    for lx in 0..9 {
        for lz in 0..7 {
            let edge = lx == 0 || lx == 8 || lz == 0 || lz == 6;
            if edge {
                b.set(f.at(lx, 0, lz), if s.flat_roof { "sandstone" } else { &log_x });
            } else if lx == 4 {
                b.set(f.at(lx, 0, lz), "water");
            } else {
                b.set(f.at(lx, 0, lz), "farmland[moisturized_amount=7]");
                b.set(f.at(lx, 1, lz), crop);
            }
        }
    }
    b.set(f.at(0, 1, 6), "composter");
}

fn smithy(b: &mut Builder, f: Frame, s: &Style) {
    prepare(b, f, 7, 6, 6, s.base);
    for lx in 0..7 {
        for lz in 0..6 {
            b.set(f.at(lx, 0, lz), "cobblestone");
            let back = lz == 0;
            let side = lx == 0 || lx == 6;
            if back || side {
                for ly in 1..=3 {
                    let corner = (lx == 0 || lx == 6) && (lz == 0 || lz == 5);
                    b.set(f.at(lx, ly, lz), if corner { s.log } else if s.flat_roof { "cut_sandstone" } else { "cobblestone" });
                }
            }
            if lz == 5 && (lx == 0 || lx == 6) {
                for ly in 1..=3 {
                    b.set(f.at(lx, ly, lz), s.fence);
                }
            }
            b.set(f.at(lx, 4, lz), if s.flat_roof { "smooth_sandstone" } else { "cobblestone_slab[minecraft:vertical_half=bottom]" });
        }
    }
    let front = f.dir(D::S);
    b.set(f.at(1, 1, 1), &format!("furnace[minecraft:cardinal_direction={}]", front.name()));
    b.set(f.at(2, 1, 1), &format!("furnace[minecraft:cardinal_direction={}]", front.name()));
    b.set(f.at(3, 1, 1), "smithing_table");
    b.chest(f.at(5, 1, 1), front, Loot::Village);
    b.set(f.at(4, 1, 1), "lava");
    b.set(f.at(4, 0, 1), "cobblestone");
}

fn tower(b: &mut Builder, f: Frame, s: &Style) {
    prepare(b, f, 5, 7, 14, s.base);
    let wall = if s.flat_roof { "cut_sandstone" } else { "cobblestone" };
    for lx in 0..5 {
        for lz in 0..7 {
            b.set(f.at(lx, 0, lz), wall);
            let edge = lx == 0 || lx == 4 || lz == 0 || lz == 6;
            let tall = lz <= 3; // clocher au fond, nef devant
            let top = if tall { 11 } else { 4 };
            if edge {
                for ly in 1..=top {
                    let window = (ly == 2 || ly == 8) && (lx == 2 || lz == 2 || lz == 4) && !(lz == 6 && lx == 2);
                    if window {
                        let ew = (lz == 0 || lz == 6) == (f.rot % 2 == 0);
                        b.set(f.at(lx, ly, lz), if ew { "glass_pane[minecraft:connection_east=1,minecraft:connection_west=1]" } else { "glass_pane[minecraft:connection_north=1,minecraft:connection_south=1]" });
                    } else {
                        b.set(f.at(lx, ly, lz), wall);
                    }
                }
            }
            b.set(f.at(lx, if tall { 12 } else { 5 }, lz), wall);
            if tall && lz == 3 && edge {
                for ly in 5..=11 {
                    b.set(f.at(lx, ly, lz), wall);
                }
            }
        }
    }
    // Plancher de l'étage, échelle et porte
    for lx in 1..4 {
        for lz in 1..3 {
            b.set(f.at(lx, 7, lz), s.planks);
        }
    }
    b.air(f.at(2, 7, 1));
    for ly in 1..=7 {
        b.set(f.at(2, ly, 1), &format!("ladder[facing_direction={}]", f.dir(D::S).facing()));
    }
    let door = f.dir(D::S);
    b.set(f.at(2, 1, 6), &format!("{}[minecraft:cardinal_direction={},upper_block_bit=0]", s.door, door.door()));
    b.set(f.at(2, 2, 6), &format!("{}[minecraft:cardinal_direction={},upper_block_bit=1]", s.door, door.door()));
    b.set(f.at(2, 4, 4), "lantern[hanging=1]");
    b.set(f.at(2, 13, 2), "torch[torch_facing_direction=top]");
}

fn pen(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng) {
    prepare(b, f, 7, 7, 3, "dirt");
    for lx in 0..7 {
        for lz in 0..7 {
            b.set(f.at(lx, 0, lz), if rng.below(4) == 0 { "coarse_dirt" } else { "dirt" });
            let edge = lx == 0 || lx == 6 || lz == 0 || lz == 6;
            if edge && !(lz == 6 && lx == 3) {
                b.set(f.at(lx, 1, lz), s.fence);
            }
        }
    }
    b.set(f.at(1, 1, 1), "hay_block[pillar_axis=y]");
    b.set(f.at(2, 1, 1), "hay_block[pillar_axis=x]");
    b.set(f.at(1, 2, 1), "hay_block[pillar_axis=y]");
    b.set(f.at(5, 1, 1), "cauldron[cauldron_liquid=water,fill_level=6]");
}

fn lamp(b: &mut Builder, f: Frame, s: &Style) {
    prepare(b, f, 1, 1, 4, s.base);
    b.set(f.at(0, 0, 0), s.base);
    if s.flat_roof {
        b.set(f.at(0, 1, 0), "sandstone");
        b.set(f.at(0, 2, 0), "torch[torch_facing_direction=top]");
    } else {
        b.set(f.at(0, 1, 0), s.fence);
        b.set(f.at(0, 2, 0), s.fence);
        b.set(f.at(0, 3, 0), "white_wool");
        b.set(f.at(0, 4, 0), "torch[torch_facing_direction=top]");
    }
}

// --- Temples ----------------------------------------------------------------------------

/// Pyramide du désert (21×21), comme dans le jeu : salle centrale au sol de terre cuite,
/// deux tours en façade, et sous le centre une chambre secrète à quatre coffres, piégée de
/// TNT sous une plaque de pression.
fn desert_pyramid(b: &mut Builder, x0: i32, z0: i32) {
    let y0 = b.top(x0 + 10, z0 + 10);
    let f = Frame { x: x0, y: y0, z: z0, rot: 0 };
    for lx in 0..21 {
        for lz in 0..21 {
            b.foundation(f.at(lx, 0, lz), "sandstone");
            for ly in 1..=16 {
                b.air(f.at(lx, ly, lz));
            }
        }
    }
    // Gradins pleins
    for k in 0..=10 {
        for lx in k..=20 - k {
            for lz in k..=20 - k {
                b.set(f.at(lx, k, lz), if k == 10 { "chiseled_sandstone" } else { "sandstone" });
            }
        }
    }
    // Salle centrale et couloir d'entrée (façade au sud)
    for lx in 5..=15 {
        for lz in 5..=15 {
            for ly in 1..=4 {
                b.air(f.at(lx, ly, lz));
            }
        }
    }
    for lz in 15..=20 {
        for lx in 9..=11 {
            for ly in 1..=3 {
                b.air(f.at(lx, ly, lz));
            }
        }
    }
    for lz in 5..=15 {
        for lx in 5..=15 {
            b.set(f.at(lx, 0, lz), "sandstone");
        }
    }
    // Motif de terre cuite au centre
    for (dx, dz) in [(0, 0)] {
        b.set(f.at(10 + dx, 0, 10 + dz), "blue_terracotta");
    }
    for (dx, dz) in [(-1, -1), (1, -1), (-1, 1), (1, 1), (0, -2), (0, 2), (-2, 0), (2, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
        b.set(f.at(10 + dx, 0, 10 + dz), "orange_terracotta");
    }
    // Tours en façade
    for tx in [0, 16] {
        for lx in tx..tx + 5 {
            for lz in 16..21 {
                let edge = lx == tx || lx == tx + 4 || lz == 16 || lz == 20;
                for ly in 1..=9 {
                    let name = if ly == 9 { "cut_sandstone" } else if ly == 7 && edge { "orange_terracotta" } else if edge { "sandstone" } else { "" };
                    if !name.is_empty() {
                        b.set(f.at(lx, ly, lz), name);
                    } else {
                        b.air(f.at(lx, ly, lz));
                    }
                }
            }
        }
        b.set(f.at(tx + 2, 10, 18), "chiseled_sandstone");
    }
    // Décor de la façade
    for lx in [8, 12] {
        for ly in 1..=4 {
            b.set(f.at(lx, ly, 20 - (ly.min(3))), "cut_sandstone");
        }
    }
    b.set(f.at(10, 4, 18), "orange_terracotta");
    // Chambre secrète sous le centre
    for lx in 7..=13 {
        for lz in 7..=13 {
            for ly in -15..=-9 {
                b.set(f.at(lx, ly, lz), "sandstone");
            }
        }
    }
    for lx in 8..=12 {
        for lz in 8..=12 {
            for ly in -13..=-10 {
                b.air(f.at(lx, ly, lz));
            }
        }
    }
    for lx in 9..=11 {
        for lz in 9..=11 {
            b.set(f.at(lx, -14, lz), "tnt");
        }
    }
    b.set(f.at(10, -13, 10), "stone_pressure_plate");
    b.chest(f.at(10, -13, 8), D::S, Loot::DesertPyramid);
    b.chest(f.at(10, -13, 12), D::N, Loot::DesertPyramid);
    b.chest(f.at(8, -13, 10), D::E, Loot::DesertPyramid);
    b.chest(f.at(12, -13, 10), D::W, Loot::DesertPyramid);
    // Puits d'accès sous la terre cuite bleue, comme les joueurs le creusent
    for ly in -12..=-1 {
        b.set(f.at(10, ly, 10), "sandstone");
    }
    for lx in [8, 12] {
        for lz in [8, 12] {
            b.set(f.at(lx, -13, lz), "orange_terracotta");
        }
    }
}

/// Temple de la jungle (12×15) : pierre et pierre moussue, deux étages, toit en gradins,
/// lianes, un coffre à chaque étage et un piège à flèches.
fn jungle_temple(b: &mut Builder, rng: &mut Rng, x0: i32, z0: i32) {
    let y0 = b.top(x0 + 6, z0 + 7);
    let f = Frame { x: x0, y: y0, z: z0, rot: 0 };
    let stone = |rng: &mut Rng| if rng.below(3) == 0 { "mossy_cobblestone" } else { "cobblestone" };
    for lx in 0..12 {
        for lz in 0..15 {
            b.foundation(f.at(lx, 0, lz), "cobblestone");
            for ly in 1..=14 {
                b.air(f.at(lx, ly, lz));
            }
        }
    }
    for lx in 0..12 {
        for lz in 0..15 {
            let edge = lx == 0 || lx == 11 || lz == 0 || lz == 14;
            b.set(f.at(lx, 0, lz), stone(rng));
            b.set(f.at(lx, 5, lz), stone(rng));
            b.set(f.at(lx, 9, lz), stone(rng));
            if edge {
                for ly in 1..=8 {
                    if ly != 5 {
                        b.set(f.at(lx, ly, lz), stone(rng));
                    }
                }
            }
        }
    }
    // Gradins du toit
    for (inset, ly) in [(2, 10), (4, 11)] {
        for lx in inset..12 - inset {
            for lz in inset..15 - inset {
                b.set(f.at(lx, ly, lz), stone(rng));
            }
        }
    }
    b.set(f.at(5, 12, 7), "chiseled_stone_bricks");
    b.set(f.at(6, 12, 7), "chiseled_stone_bricks");
    // Entrée (sud), fenêtres
    for lx in 5..=6 {
        for ly in 1..=3 {
            b.air(f.at(lx, ly, 14));
        }
        b.set(f.at(lx, 0, 15), &format!("stone_stairs[weirdo_direction={},upside_down_bit=0]", D::N.stairs()));
    }
    for (lx, lz) in [(0, 4), (0, 10), (11, 4), (11, 10)] {
        b.air(f.at(lx, 7, lz));
    }
    // Échelle vers l'étage, trou dans le plancher
    for ly in 1..=5 {
        b.set(f.at(1, ly, 7), &format!("ladder[facing_direction={}]", D::E.facing()));
    }
    // Coffres
    b.chest(f.at(9, 6, 2), D::S, Loot::JungleTemple);
    b.chest(f.at(10, 1, 2), D::W, Loot::JungleTemple);
    b.set(f.at(8, 1, 1), "lever[lever_direction=north]");
    b.set(f.at(7, 1, 1), "lever[lever_direction=north]");
    b.set(f.at(6, 1, 1), "lever[lever_direction=north]");
    // Piège : distributeur face au couloir, fil tendu
    b.set(f.at(10, 1, 7), &format!("dispenser[facing_direction={}]", D::W.facing()));
    b.set(f.at(1, 1, 9), "tripwire_hook[direction=0]");
    // Lianes sur les murs extérieurs
    for _ in 0..40 {
        let side = rng.below(4);
        let ly = 1 + rng.below(9) as i32;
        let (pos, bits) = match side {
            0 => (f.at(rng.below(12) as i32, ly, -1), 1),  // devant le mur nord, accrochée au sud
            1 => (f.at(rng.below(12) as i32, ly, 15), 4),  // devant le mur sud, accrochée au nord
            2 => (f.at(-1, ly, rng.below(15) as i32), 8),  // à l'ouest, accrochée à l'est
            _ => (f.at(12, ly, rng.below(15) as i32), 2),  // à l'est, accrochée à l'ouest
        };
        let top = b.top(pos.0, pos.2);
        if pos.1 > top {
            b.set(pos, &format!("vine[vine_direction_bits={}]", bits));
        }
    }
}

// --- Mines abandonnées ------------------------------------------------------------------

/// Mine abandonnée : une salle, puis des galeries de 3×3 étayées de bois tous les 5 blocs,
/// avec rails, toiles d'araignée et coffres, qui bifurquent aux croisements.
fn mineshaft(b: &mut Builder, rng: &mut Rng, x: i32, z: i32) {
    let top = b.top(x, z);
    let highest = (top - 20).min(30);
    if highest < -40 {
        return;
    }
    let y = -40 + rng.below((highest + 40) as u32 + 1) as i32;
    // Salle de départ
    let (rw, rd) = (4 + rng.below(4) as i32, 4 + rng.below(4) as i32);
    for dx in -rw..=rw {
        for dz in -rd..=rd {
            b.set((x + dx, y - 1, z + dz), "dirt");
            for dy in 0..=3 {
                b.air((x + dx, y + dy, z + dz));
            }
        }
    }
    let mut todo: Vec<(i32, i32, i32, D, u8)> = Vec::new();
    for d in [D::N, D::E, D::S, D::W] {
        let (dx, dz) = d.step();
        let reach = if dx != 0 { rw } else { rd };
        todo.push((x + dx * (reach + 1), y, z + dz * (reach + 1), d, 0));
    }
    let mut corridors = 0;
    while let Some((sx, sy, sz, dir, depth)) = todo.pop() {
        if corridors > 30 {
            break;
        }
        corridors += 1;
        let (dx, dz) = dir.step();
        let (px, pz) = dir.turn(1).step();
        let segments = 2 + rng.below(4) as i32;
        let len = segments * 5;
        let rail = format!("rail[rail_direction={}]", if dx == 0 { 0 } else { 1 });
        let mut end = len;
        for i in 0..len {
            let (cx, cz) = (sx + dx * i, sz + dz * i);
            if (cx - x).abs() > 76 || (cz - z).abs() > 76 || sy > b.top(cx, cz) - 6 {
                end = i;
                break;
            }
            for w in -1..=1 {
                for h in 0..=2 {
                    b.air((cx + px * w, sy + h, cz + pz * w));
                }
            }
            if i % 5 == 2 {
                // Étai : deux poteaux et une poutre
                b.set((cx - px, sy, cz - pz), "oak_fence");
                b.set((cx - px, sy + 1, cz - pz), "oak_fence");
                b.set((cx + px, sy, cz + pz), "oak_fence");
                b.set((cx + px, sy + 1, cz + pz), "oak_fence");
                for w in -1..=1 {
                    b.set((cx + px * w, sy + 2, cz + pz * w), "oak_planks");
                }
            } else {
                if rng.below(10) < 7 {
                    b.set((cx, sy, cz), &rail);
                }
                if rng.below(25) == 0 {
                    let w = if rng.below(2) == 0 { 1 } else { -1 };
                    b.set((cx + px * w, sy + 2, cz + pz * w), "web");
                }
                if rng.below(90) == 0 {
                    let w = if rng.below(2) == 0 { 1 } else { -1 };
                    let front = if w == 1 { dir.turn(3) } else { dir.turn(1) };
                    b.chest((cx + px * w, sy, cz + pz * w), front, Loot::Mineshaft);
                }
            }
        }
        if end < len || depth >= 6 {
            continue;
        }
        // Croisement au bout de la galerie
        let (ex, ez) = (sx + dx * len, sz + dz * len);
        for w in -1..=1 {
            for v in -1..=1 {
                for h in 0..=2 {
                    b.air((ex + px * w + dx * v, sy + h, ez + pz * w + dz * v));
                }
            }
        }
        for (turn, chance) in [(0u8, 2u32), (1, 3), (3, 3)] {
            if rng.below(chance) == 0 {
                let d = dir.turn(turn);
                let (ox, oz) = d.step();
                todo.push((ex + ox * 2, sy, ez + oz * 2, d, depth + 1));
            }
        }
    }
}

// --- Recherche (/locate) ----------------------------------------------------------------

/// Ce que `/locate structure` sait trouver.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Village,
    DesertPyramid,
    JungleTemple,
    /// Pyramide du désert ou temple de la jungle, comme `temple` dans le jeu
    Temple,
    Mineshaft,
}

impl Target {
    /// Nom donné à la commande (ceux du jeu, et quelques variantes).
    pub fn parse(name: &str) -> Option<Target> {
        Some(match name.trim().trim_start_matches("minecraft:").to_ascii_lowercase().as_str() {
            "village" => Target::Village,
            "desert_pyramid" | "desert_temple" | "pyramid" => Target::DesertPyramid,
            "jungle_temple" | "jungle_pyramid" => Target::JungleTemple,
            "temple" => Target::Temple,
            "mineshaft" => Target::Mineshaft,
            _ => return None,
        })
    }

    /// Début de la réponse de `/locate`.
    pub fn label(self) -> &'static str {
        match self {
            Target::Village => "Le village le plus proche",
            Target::DesertPyramid => "La pyramide du désert la plus proche",
            Target::JungleTemple => "Le temple de la jungle le plus proche",
            Target::Temple => "Le temple le plus proche",
            Target::Mineshaft => "La mine abandonnée la plus proche",
        }
    }
}

/// Structures du jeu que la commande connaît mais que Basalt ne génère pas encore.
pub const NOT_GENERATED: [&str; 14] = [
    "ancient_city", "bastion_remnant", "buried_treasure", "end_city", "fortress", "mansion", "monument", "ocean_ruins", "pillager_outpost",
    "ruined_portal", "shipwreck", "stronghold", "trail_ruins", "trial_chambers",
];

/// Cases d'une couronne carrée de rayon `r` autour de l'origine.
fn ring(r: i32) -> impl Iterator<Item = (i32, i32)> {
    (-r..=r).flat_map(move |dx| (-r..=r).map(move |dz| (dx, dz))).filter(move |(dx, dz)| dx.abs().max(dz.abs()) == r)
}

/// La structure de cette sorte est-elle bien générée à cet emplacement de départ ?
/// Renvoie sa position (bloc) — mêmes règles que `build`.
fn placed(v: &Vanilla, kind: Kind, sx: i32, sz: i32) -> Option<(i32, i32, i32)> {
    let (x, z) = (sx * 16 + 8, sz * 16 + 8);
    match kind {
        Kind::Village => {
            let (top, biome) = v.surface(x, z);
            (Style::for_biome(biome).is_some() && top > SEA_LEVEL && top <= SEA_LEVEL + 60).then_some((x, top + 1, z))
        }
        Kind::DesertPyramid => {
            let (top, biome) = v.surface(x, z);
            (biome == B::Desert && top > SEA_LEVEL).then_some((x, top + 1, z))
        }
        Kind::JungleTemple => {
            let (top, biome) = v.surface(x, z);
            (matches!(biome, B::Jungle | B::BambooJungle) && top > SEA_LEVEL).then_some((x, top + 1, z))
        }
        Kind::Mineshaft => {
            let top = v.surface(x, z).0;
            let highest = (top - 20).min(30);
            if highest < -40 {
                return None;
            }
            // Même tirage que `mineshaft` : la hauteur de la salle de départ
            let mut rng = Rng::new(mix(v.seed(), sx as i64, sz as i64, 31_337 + kind as u64));
            Some((x, -40 + rng.below((highest + 40) as u32 + 1) as i32, z))
        }
    }
}

/// Structure de cette sorte la plus proche de (`x`, `z`), dans un rayon d'environ 10 000
/// blocs. Renvoie sa position (bloc).
pub fn locate(v: &Vanilla, what: Target, x: i32, z: i32) -> Option<(i32, i32, i32)> {
    let kinds: &[Kind] = match what {
        Target::Village => &[Kind::Village],
        Target::DesertPyramid => &[Kind::DesertPyramid],
        Target::JungleTemple => &[Kind::JungleTemple],
        Target::Temple => &[Kind::DesertPyramid, Kind::JungleTemple],
        Target::Mineshaft => &[Kind::Mineshaft],
    };
    let dist = |p: &(i32, i32, i32)| ((p.0 - x) as i64).pow(2) + ((p.2 - z) as i64).pow(2);
    let mut best: Option<(i32, i32, i32)> = None;
    for kind in kinds {
        // (espacement de la grille en chunks, séparation, sel ; 1 pour les mines : chaque chunk)
        let (spacing, separation, salt) = match kind {
            Kind::Village => (34, 8, 10_387_312u64),
            Kind::DesertPyramid => (32, 8, 14_357_617),
            Kind::JungleTemple => (32, 8, 14_357_619),
            Kind::Mineshaft => (1, 0, 0),
        };
        let rings = (10_000 / (spacing * 16)).clamp(20, 640);
        let (rx0, rz0) = ((x >> 4).div_euclid(spacing), (z >> 4).div_euclid(spacing));
        let mut found_at = None;
        for r in 0..=rings {
            // Une couronne de plus après la première trouvaille : elle peut cacher plus près
            if matches!(found_at, Some(f) if r > f + 1) {
                break;
            }
            for (dx, dz) in ring(r) {
                let (rx, rz) = (rx0 + dx, rz0 + dz);
                let (sx, sz) = if *kind == Kind::Mineshaft {
                    if mix(v.seed(), rx as i64, rz as i64, 77_001) % 1000 >= 4 {
                        continue;
                    }
                    (rx, rz)
                } else {
                    grid_start(v.seed(), rx, rz, spacing, separation, salt)
                };
                if let Some(p) = placed(v, *kind, sx, sz) {
                    found_at.get_or_insert(r);
                    if best.map(|b| dist(&p) < dist(&b)).unwrap_or(true) {
                        best = Some(p);
                    }
                }
            }
        }
    }
    best
}

/// Colonne la plus proche de (`x`, `z`) où ce biome est en surface, dans un rayon d'environ
/// 6 400 blocs (un point tous les 32 blocs, comme le jeu).
pub fn locate_biome(v: &Vanilla, biome: B, x: i32, z: i32) -> Option<(i32, i32, i32)> {
    const STEP: i32 = 32;
    let mut best: Option<(i64, i32, i32)> = None;
    let mut found_at = None;
    for r in 0..=200 {
        if matches!(found_at, Some(f) if r > f + 1) {
            break;
        }
        for (dx, dz) in ring(r) {
            let (px, pz) = (x + dx * STEP, z + dz * STEP);
            if v.surface_biome_at(px, pz) == biome {
                found_at.get_or_insert(r);
                let d = ((px - x) as i64).pow(2) + ((pz - z) as i64).pow(2);
                if best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, px, pz));
                }
            }
        }
    }
    best.map(|(_, px, pz)| (px, v.surface(px, pz).0 + 1, pz))
}

// --- Butin ------------------------------------------------------------------------------

/// Objets possibles : (nom, minimum, maximum, poids).
fn table(kind: Loot) -> (&'static [(&'static str, u8, u8, u32)], (u32, u32)) {
    match kind {
        Loot::Village => (
            &[
                ("bread", 1, 4, 15),
                ("apple", 1, 5, 15),
                ("wheat", 2, 7, 10),
                ("potato", 1, 5, 10),
                ("carrot", 1, 5, 10),
                ("iron_ingot", 1, 3, 8),
                ("emerald", 1, 2, 5),
                ("oak_sapling", 1, 3, 8),
                ("torch", 2, 8, 8),
                ("coal", 1, 5, 8),
                ("leather", 1, 3, 5),
            ],
            (3, 7),
        ),
        Loot::DesertPyramid => (
            &[
                ("bone", 4, 6, 25),
                ("rotten_flesh", 3, 7, 16),
                ("gunpowder", 1, 8, 10),
                ("sand", 1, 8, 10),
                ("string", 1, 8, 10),
                ("spider_eye", 1, 3, 16),
                ("gold_ingot", 2, 7, 15),
                ("iron_ingot", 1, 5, 15),
                ("emerald", 1, 3, 15),
                ("diamond", 1, 3, 5),
                ("golden_apple", 1, 1, 15),
                ("saddle", 1, 1, 20),
                ("iron_horse_armor", 1, 1, 15),
                ("golden_horse_armor", 1, 1, 10),
                ("diamond_horse_armor", 1, 1, 5),
                ("book", 1, 1, 20),
            ],
            (2, 6),
        ),
        Loot::JungleTemple => (
            &[
                ("bamboo", 1, 3, 15),
                ("gold_ingot", 2, 7, 15),
                ("iron_ingot", 1, 5, 15),
                ("emerald", 1, 3, 2),
                ("diamond", 1, 3, 3),
                ("bone", 4, 6, 20),
                ("rotten_flesh", 3, 7, 16),
                ("saddle", 1, 1, 3),
                ("iron_horse_armor", 1, 1, 1),
                ("golden_horse_armor", 1, 1, 1),
                ("book", 1, 1, 1),
            ],
            (2, 6),
        ),
        Loot::Mineshaft => (
            &[
                ("rail", 4, 8, 20),
                ("torch", 1, 16, 15),
                ("bread", 1, 3, 15),
                ("coal", 3, 8, 10),
                ("iron_ingot", 1, 5, 10),
                ("gold_ingot", 1, 3, 5),
                ("redstone", 4, 9, 5),
                ("lapis_lazuli", 4, 9, 5),
                ("diamond", 1, 2, 3),
                ("name_tag", 1, 1, 30),
                ("golden_apple", 1, 1, 20),
                ("melon_seeds", 2, 4, 10),
                ("pumpkin_seeds", 2, 4, 10),
                ("beetroot_seeds", 2, 4, 10),
                ("iron_pickaxe", 1, 1, 5),
            ],
            (3, 7),
        ),
    }
}

/// Contenu tiré pour un coffre de structure : (emplacement 0 à 26, nom, nombre). Le même
/// pour une graine et une position données.
pub fn loot(kind: Loot, seed: u64, pos: (i32, i32, i32)) -> Vec<(usize, &'static str, u8)> {
    let (items, (lo, hi)) = table(kind);
    let mut rng = Rng::new(mix(seed ^ (pos.1 as u64).wrapping_mul(0x9e37), pos.0 as i64, pos.2 as i64, 4242));
    let total: u32 = items.iter().map(|i| i.3).sum();
    let rolls = lo + rng.below(hi - lo + 1);
    let mut used = [false; 27];
    let mut out = Vec::new();
    for _ in 0..rolls {
        let mut pick = rng.below(total);
        let item = items.iter().find(|i| {
            if pick < i.3 {
                true
            } else {
                pick -= i.3;
                false
            }
        });
        let Some((name, min, max, _)) = item else { continue };
        let count = min + rng.below((max - min + 1) as u32) as u8;
        let mut slot = rng.below(27) as usize;
        while used[slot] {
            slot = (slot + 1) % 27;
        }
        used[slot] = true;
        out.push((slot, *name, count));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_rotate_like_directions() {
        for rot in 0..4u8 {
            let f = Frame { x: 0, y: 0, z: 0, rot };
            let (x, _, z) = f.at(0, 0, 1);
            assert_eq!((x, z), f.dir(D::S).step(), "rotation {}", rot);
            let (x, _, z) = f.at(1, 0, 0);
            assert_eq!((x, z), f.dir(D::E).step(), "rotation {}", rot);
        }
    }

    #[test]
    fn loot_is_stable_and_fits_a_chest() {
        let a = loot(Loot::DesertPyramid, 42, (10, 20, 30));
        assert_eq!(a, loot(Loot::DesertPyramid, 42, (10, 20, 30)));
        assert!(!a.is_empty() && a.len() <= 27);
        let mut slots: Vec<usize> = a.iter().map(|i| i.0).collect();
        slots.dedup();
        assert!(slots.iter().all(|s| *s < 27));
    }

    /// Sur la graine 12345 : un village, une pyramide, un temple et une mine connus se
    /// génèrent avec leurs coffres, et le monde retrouve ces coffres à leur place.
    #[test]
    fn known_structures_are_built_with_their_chests() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let Ok(data) = crate::data::GameData::load(&dir) else { return };
        let p = Palette::new(&|n| data.state_from_spec(n));
        let v = Vanilla::new(12345);
        let count = |plan: &Plan| plan.chests.values().map(|c| c.len()).sum::<usize>();
        let village = build(&v, &p, (Kind::Village, 46, -17));
        assert!(village.blocks.values().map(|b| b.len()).sum::<usize>() > 3000, "village trop petit");
        assert!(count(&village) >= 1);
        let pyramid = build(&v, &p, (Kind::DesertPyramid, 71, -31));
        assert_eq!(count(&pyramid), 4);
        assert_eq!(count(&build(&v, &p, (Kind::JungleTemple, -120, 151))), 2);
        assert!(!build(&v, &p, (Kind::Mineshaft, 0, 10)).is_empty());
        // Hors désert, pas de pyramide
        assert!(build(&v, &p, (Kind::DesertPyramid, 46, -17)).is_empty());
        // Le monde retrouve un coffre de la pyramide ; remplacé par un joueur, il n'en est plus un
        let ((cx, cz), list) = pyramid.chests.iter().next().unwrap();
        let (x, y, z, _) = list[0];
        let pos = (cx * 16 + x as i32, y as i32, cz * 16 + z as i32);
        let mut world = super::super::World::with_generator(super::super::generator::Generator::new(super::super::generator::Kind::Vanilla, 12345, Palette::new(&|n| data.state_from_spec(n))));
        assert_eq!(world.structure_chest(pos), Some(Loot::DesertPyramid));
        assert!(data.block_name(world.get(pos.0, pos.1, pos.2)).unwrap().ends_with("chest"));
        let air = world.air;
        world.set(pos.0, pos.1, pos.2, air);
        assert_eq!(world.structure_chest(pos), None);
    }

    /// `/locate` retrouve les structures connues de la graine 12345 (celles que génère
    /// `build`), et un biome.
    #[test]
    fn locate_finds_generated_structures() {
        let v = Vanilla::new(12345);
        let p = Palette::new(&|_| None);
        for what in [Target::Village, Target::DesertPyramid, Target::JungleTemple, Target::Mineshaft] {
            let (x, _, z) = locate(&v, what, 0, 0).unwrap_or_else(|| panic!("{:?} introuvable", what));
            let kind = match what {
                Target::Village => Kind::Village,
                Target::DesertPyramid => Kind::DesertPyramid,
                Target::JungleTemple => Kind::JungleTemple,
                _ => Kind::Mineshaft,
            };
            assert!(!build(&v, &p, (kind, (x - 8) >> 4, (z - 8) >> 4)).is_empty(), "{:?} indiquée en {} {} mais absente", what, x, z);
        }
        let (x, _, z) = locate_biome(&v, B::Desert, 0, 0).expect("pas de désert");
        assert_eq!(v.surface_biome_at(x, z), B::Desert);
        assert_eq!(Target::parse("minecraft:village"), Some(Target::Village));
        assert_eq!(Target::parse("temple"), Some(Target::Temple));
    }

    /// Cherche et dessine (vue de dessus) une structure de chaque sorte.
    #[test]
    #[ignore]
    fn dump_structures() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let p = Palette::new(&|n| data.state_from_spec(n));
        let seed: u64 = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(12345);
        let v = Vanilla::new(seed);
        for (kind, spacing, sep, salt) in [(Kind::Village, 34, 8, 10_387_312u64), (Kind::DesertPyramid, 32, 8, 14_357_617), (Kind::JungleTemple, 32, 8, 14_357_619), (Kind::Mineshaft, 1, 0, 0)] {
            let mut found = None;
            'search: for r in 0..40i32 {
                for rx in -r..=r {
                    for rz in -r..=r {
                        if rx.abs() != r && rz.abs() != r {
                            continue;
                        }
                        let (sx, sz) = if kind == Kind::Mineshaft {
                            if mix(seed, rx as i64, rz as i64, 77_001) % 1000 >= 4 { continue; }
                            (rx, rz)
                        } else {
                            grid_start(seed, rx, rz, spacing, sep, salt)
                        };
                        let plan = build(&v, &p, (kind, sx, sz));
                        if !plan.is_empty() {
                            found = Some((sx, sz, plan));
                            break 'search;
                        }
                    }
                }
            }
            let Some((sx, sz, plan)) = found else { eprintln!("{:?} : aucune trouvée", kind); continue };
            let count: usize = plan.blocks.values().map(|v| v.len()).sum();
            let chests: usize = plan.chests.values().map(|v| v.len()).sum();
            eprintln!("{:?} en chunk {} {} (bloc {} {}) : {} blocs, {} coffres", kind, sx, sz, sx * 16 + 8, sz * 16 + 8, count, chests);
            let mut top: HashMap<(i32, i32), (i16, u32)> = HashMap::new();
            for ((cx, cz), list) in &plan.blocks {
                for (x, y, z, s) in list {
                    if *s == p.air { continue; }
                    let key = (cx * 16 + *x as i32, cz * 16 + *z as i32);
                    let e = top.entry(key).or_insert((*y, *s));
                    if *y > e.0 { *e = (*y, *s); }
                }
            }
            let (minx, maxx) = (top.keys().map(|k| k.0).min().unwrap(), top.keys().map(|k| k.0).max().unwrap());
            let (minz, maxz) = (top.keys().map(|k| k.1).min().unwrap(), top.keys().map(|k| k.1).max().unwrap());
            if kind == Kind::Mineshaft {
                let room = plan.blocks.get(&(sx, sz)).and_then(|l| l.iter().find(|b| b.0 == 8 && b.2 == 8).map(|b| b.1));
                eprintln!("  étendue {}..{} x {}..{}, salle vers y = {:?}", minx, maxx, minz, maxz, room);
                continue;
            }
            for z in minz..=maxz {
                let mut line = String::new();
                for x in minx..=maxx {
                    line.push(match top.get(&(x, z)) {
                        None => ' ',
                        Some((_, s)) => {
                            let n = data.block_names.get(s).map(|s| s.as_str()).unwrap_or("?");
                            if n.contains("path") || n.contains("smooth_sandstone") && kind == Kind::Village { '=' }
                            else if n.contains("stairs") || n.contains("slab") { '^' }
                            else if n.contains("water") { '~' }
                            else if n.contains("farmland") || n.contains("wheat") || n.contains("carrot") || n.contains("potato") || n.contains("beetroot") { ',' }
                            else if n.contains("fence") || n.contains("torch") { 'i' }
                            else if n.contains("terracotta") { 'T' }
                            else if n.contains("chest") { 'C' }
                            else { '#' }
                        }
                    });
                }
                eprintln!("{}", line);
            }
        }
    }

    /// Coupes horizontales d'un bâtiment du village trouvé par `dump_structures`.
    #[test]
    #[ignore]
    fn dump_layers() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let p = Palette::new(&|n| data.state_from_spec(n));
        let v = Vanilla::new(12345);
        let plan = build(&v, &p, (Kind::Village, 46, -17));
        let mut all: HashMap<(i32, i32, i32), u32> = HashMap::new();
        for ((cx, cz), list) in &plan.blocks {
            for (x, y, z, s) in list {
                all.insert((cx * 16 + *x as i32, *y as i32, cz * 16 + *z as i32), *s);
            }
        }
        let (x0, z0): (i32, i32) = (std::env::var("LX").ok().and_then(|s| s.parse().ok()).unwrap_or(736), std::env::var("LZ").ok().and_then(|s| s.parse().ok()).unwrap_or(-272));
        let ys: Vec<i32> = all.keys().filter(|(x, _, z)| (*x - x0).abs() < 12 && (*z - z0).abs() < 12).map(|k| k.1).collect();
        let (ymin, ymax) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
        for y in ymin..=ymax {
            eprintln!("y = {}", y);
            for z in z0 - 11..=z0 + 11 {
                let mut line = String::new();
                for x in x0 - 11..=x0 + 11 {
                    line.push(match all.get(&(x, y, z)) {
                        None => ' ',
                        Some(s) if *s == p.air => '.',
                        Some(s) => {
                            let n = data.block_names.get(s).map(|s| s.as_str()).unwrap_or("?").trim_start_matches("minecraft:");
                            match n {
                                _ if n.contains("door") => 'D',
                                _ if n.contains("stairs") => '^',
                                _ if n.contains("slab") => '-',
                                _ if n.contains("log") => 'L',
                                _ if n.contains("planks") => 'P',
                                _ if n.contains("glass") => 'G',
                                _ if n.contains("cobble") => 'c',
                                _ if n.contains("path") => '=',
                                _ if n.contains("water") => '~',
                                _ if n.contains("fence") => 'i',
                                _ if n.contains("chest") => 'C',
                                _ if n.contains("lantern") || n.contains("torch") => '*',
                                _ => n.chars().next().unwrap_or('?').to_ascii_uppercase(),
                            }
                        }
                    });
                }
                eprintln!("{}", line);
            }
        }
    }
}
