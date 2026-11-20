//! Écoulement de l'eau et de la lave, comme dans le jeu.
//!
//! Chaque bloc liquide a une profondeur (`liquid_depth`) : 0 pour une source, 1 à 7 pour
//! l'eau qui s'étale (la lave avance de 2 en 2, donc sur 3 blocs), et 8 pour un liquide qui
//! tombe. Une source s'appelle `water` (ou `lava`), le liquide qui coule `flowing_water`.
//!
//! Rien ne bouge tout seul : un liquide n'est recalculé que lorsqu'un bloc change à côté de
//! lui (bloc cassé, posé, seau vidé). La mise à jour est planifiée quelques ticks plus tard
//! (5 pour l'eau, 30 pour la lave), comme dans le jeu, ce qui donne la même vitesse
//! d'écoulement. Le nombre de mises à jour par tick est plafonné : une grande inondation se
//! répand plus lentement au lieu de ralentir le serveur.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::data::{GameData, Prop};

use super::World;

/// Mises à jour de liquide traitées au plus par tick
const BUDGET: usize = 3000;
/// Mises à jour en attente au plus (au-delà, les nouvelles sont ignorées)
const MAX_PENDING: usize = 200_000;
/// Profondeur d'un liquide qui tombe
const FALLING: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fluid {
    Water,
    Lava,
}

impl Fluid {
    fn step(self) -> u8 {
        match self {
            Fluid::Water => 1,
            Fluid::Lava => 2,
        }
    }

    fn delay(self) -> u64 {
        match self {
            Fluid::Water => 5,
            Fluid::Lava => 30,
        }
    }

    /// Distance jusqu'à laquelle le liquide cherche un trou où descendre
    fn reach(self) -> u8 {
        match self {
            Fluid::Water => 4,
            Fluid::Lava => 2,
        }
    }
}

/// Les liquides en mouvement d'un monde.
pub struct Liquids {
    /// États de chaque liquide : [source, coulant] × profondeur
    states: HashMap<(Fluid, bool), [u32; 16]>,
    known: HashMap<u32, (Fluid, u8)>,
    stone: u32,
    cobblestone: u32,
    obsidian: u32,
    queue: BTreeMap<u64, Vec<(i32, i32, i32)>>,
    planned: HashSet<(i32, i32, i32)>,
}

const SIDES: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

impl Liquids {
    /// `None` sans les données du jeu (tests) : les liquides restent alors immobiles.
    pub fn new(data: &GameData) -> Option<Liquids> {
        let mut states = HashMap::new();
        let mut known = HashMap::new();
        for (fluid, still, flowing) in [(Fluid::Water, "minecraft:water", "minecraft:flowing_water"), (Fluid::Lava, "minecraft:lava", "minecraft:flowing_lava")] {
            for (moving, name) in [(false, still), (true, flowing)] {
                let base = data.default_state(name)?;
                let mut list = [base; 16];
                for (depth, slot) in list.iter_mut().enumerate() {
                    *slot = data.with_props(base, &[("liquid_depth", Prop::Int(depth as i32))])?;
                    known.insert(*slot, (fluid, depth as u8));
                }
                states.insert((fluid, moving), list);
            }
        }
        Some(Liquids {
            states,
            known,
            stone: data.default_state("minecraft:stone")?,
            cobblestone: data.default_state("minecraft:cobblestone")?,
            obsidian: data.default_state("minecraft:obsidian")?,
            queue: BTreeMap::new(),
            planned: HashSet::new(),
        })
    }

    /// Liquide et profondeur d'un bloc.
    pub fn fluid(&self, state: u32) -> Option<(Fluid, u8)> {
        self.known.get(&state).copied()
    }

    /// Bloc source d'un liquide (ce que pose un seau).
    pub fn source(&self, fluid: Fluid) -> u32 {
        self.states[&(fluid, false)][0]
    }

    fn state(&self, fluid: Fluid, depth: u8) -> u32 {
        self.states[&(fluid, depth != 0)][depth as usize & 15]
    }

    /// Nombre de mises à jour en attente.
    pub fn pending(&self) -> usize {
        self.planned.len()
    }

    /// Un bloc a changé : lui et ses voisins liquides seront recalculés.
    pub fn block_changed(&mut self, world: &World, now: u64, pos: (i32, i32, i32)) {
        let (x, y, z) = pos;
        for (dx, dy, dz) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0), (0, -1, 0)] {
            let p = (x + dx, y + dy, z + dz);
            let fluid = self.fluid(world.get(p.0, p.1, p.2)).map(|f| f.0).or(if world.is_wet(p.0, p.1, p.2) { Some(Fluid::Water) } else { None });
            if let Some(f) = fluid {
                self.plan(now + f.delay(), p);
            }
        }
    }

    fn plan(&mut self, due: u64, pos: (i32, i32, i32)) {
        if self.planned.len() >= MAX_PENDING || !World::in_bounds(pos.1) || !self.planned.insert(pos) {
            return;
        }
        self.queue.entry(due).or_default().push(pos);
    }

    /// Traite les mises à jour arrivées à échéance. Renvoie les blocs changés, à envoyer aux
    /// joueurs. `washable` : blocs emportés par le courant (herbes, fleurs, torches…).
    pub fn tick(&mut self, world: &mut World, now: u64, washable: &dyn Fn(u32) -> bool) -> Vec<(i32, i32, i32, u32)> {
        let mut changed = Vec::new();
        let mut done = 0;
        while done < BUDGET {
            let due = match self.queue.keys().next() {
                Some(t) if *t <= now => *t,
                _ => break,
            };
            let mut batch = self.queue.remove(&due).unwrap_or_default();
            while let Some(pos) = batch.pop() {
                if done >= BUDGET {
                    // Le reste attend le tick suivant
                    batch.push(pos);
                    self.queue.entry(now + 1).or_default().extend(batch.drain(..));
                    break;
                }
                done += 1;
                self.planned.remove(&pos);
                self.update(world, now, pos, washable, &mut changed);
            }
        }
        changed
    }

    fn set(&mut self, world: &mut World, now: u64, pos: (i32, i32, i32), state: u32, changed: &mut Vec<(i32, i32, i32, u32)>) {
        if world.get(pos.0, pos.1, pos.2) == state || !world.set(pos.0, pos.1, pos.2, state) {
            return;
        }
        changed.push((pos.0, pos.1, pos.2, state));
        self.block_changed(world, now, pos);
    }

    /// Liquide présent à une position (un bloc posé dans l'eau compte comme une source).
    fn at(&self, w: &World, p: (i32, i32, i32)) -> Option<(Fluid, u8)> {
        match self.fluid(w.get(p.0, p.1, p.2)) {
            Some(f) => Some(f),
            None if w.is_wet(p.0, p.1, p.2) => Some((Fluid::Water, 0)),
            None => None,
        }
    }

    /// Le liquide peut-il entrer dans ce bloc avec cette profondeur ?
    fn can_enter(&self, world: &World, pos: (i32, i32, i32), fluid: Fluid, depth: u8, washable: &dyn Fn(u32) -> bool) -> bool {
        if !World::in_bounds(pos.1) || world.is_wet(pos.0, pos.1, pos.2) {
            return false;
        }
        let b = world.get(pos.0, pos.1, pos.2);
        if b == world.air || washable(b) {
            return true;
        }
        match self.fluid(b) {
            // Même liquide, moins haut (et ni source ni chute) : il monte
            Some((f, d)) if f == fluid => d != 0 && d < FALLING && depth < d,
            _ => false,
        }
    }

    fn update(&mut self, world: &mut World, now: u64, pos: (i32, i32, i32), washable: &dyn Fn(u32) -> bool, changed: &mut Vec<(i32, i32, i32, u32)>) {
        let (x, y, z) = pos;
        let here = world.get(x, y, z);
        let wet = world.is_wet(x, y, z);
        let (fluid, mut depth) = match (self.fluid(here), wet) {
            (Some(f), _) => f,
            // Bloc posé dans l'eau : une source qui ne s'écoule que sur les côtés
            (None, true) => (Fluid::Water, 0),
            _ => return,
        };

        // Lave touchée par de l'eau : obsidienne (source) ou pierre (lave qui coule)
        if fluid == Fluid::Lava {
            let touches_water = SIDES.iter().map(|(dx, dz)| (x + dx, y, z + dz)).chain([(x, y + 1, z)]).any(|p| matches!(self.at(world, p), Some((Fluid::Water, _))));
            if touches_water {
                let into = if depth == 0 { self.obsidian } else { self.cobblestone };
                self.set(world, now, pos, into, changed);
                return;
            }
        }

        if !wet && depth != 0 {
            // Liquide qui coule : sa profondeur dépend de ses voisins
            let above = self.at(world, (x, y + 1, z)).filter(|f| f.0 == fluid).is_some();
            let mut best: Option<u8> = None;
            let mut sources = 0;
            for (dx, dz) in SIDES {
                if let Some((f, d)) = self.at(world, (x + dx, y, z + dz)) {
                    if f == fluid {
                        if d == 0 {
                            sources += 1;
                        }
                        let level = if d >= FALLING { 0 } else { d };
                        best = Some(best.map_or(level, |b| b.min(level)));
                    }
                }
            }
            let below = world.get(x, y - 1, z);
            let solid_below = below != world.air && !washable(below) && self.fluid(below).is_none();
            let new = if fluid == Fluid::Water && sources >= 2 && (solid_below || self.fluid(below) == Some((Fluid::Water, 0))) {
                0 // deux sources côte à côte en font une troisième
            } else if above {
                FALLING
            } else {
                match best {
                    Some(b) if b + fluid.step() < FALLING => b + fluid.step(),
                    _ => {
                        // Plus rien ne l'alimente : il disparaît
                        let air = world.air;
                        self.set(world, now, pos, air, changed);
                        return;
                    }
                }
            };
            if new != depth {
                depth = new;
                let state = self.state(fluid, depth);
                self.set(world, now, pos, state, changed);
            }
        }

        // Il descend s'il peut. Comme dans le jeu, un liquide au-dessus d'un vide ou du même
        // liquide en train de couler ne s'étale pas sur les côtés, même une source (vidée
        // contre une falaise, elle fait une seule cascade), sauf entre trois sources
        let down = (x, y - 1, z);
        let below = self.fluid(world.get(down.0, down.1, down.2));
        let falls = !wet && self.can_enter(world, down, fluid, FALLING, washable);
        let over_flow = !wet && matches!(below, Some((f, d)) if f == fluid && d != 0);
        if falls || over_flow {
            if falls {
                if fluid == Fluid::Water && matches!(below, Some((Fluid::Lava, _))) {
                    // L'eau qui tombe sur de la lave la change en pierre
                    let stone = self.stone;
                    self.set(world, now, down, stone, changed);
                } else {
                    let state = self.state(fluid, FALLING);
                    self.set(world, now, down, state, changed);
                }
            }
            let sources = SIDES.iter().filter(|(dx, dz)| self.at(world, (x + dx, y, z + dz)) == Some((fluid, 0))).count();
            if sources < 3 {
                return;
            }
        } else if depth != 0 && matches!(below, Some((f, _)) if f == fluid) {
            // Liquide qui coule au-dessus d'une source du même liquide : il s'y fond
            return;
        }
        // Puis s'étale sur les côtés, de préférence vers le trou le plus proche
        let level = if depth >= FALLING { 0 } else { depth };
        let next = level + fluid.step();
        if next >= FALLING {
            return;
        }
        // Directions possibles : libres, ou déjà occupées par ce liquide en train de couler.
        // Comme dans le jeu, celles déjà mouillées comptent dans le choix : sinon, une fois
        // le chemin vers le trou rempli, le liquide repartirait dans toutes les autres.
        let passable: Vec<(i32, i32)> = SIDES
            .iter()
            .copied()
            .filter(|(dx, dz)| {
                let p = (x + dx, y, z + dz);
                if !World::in_bounds(p.1) || world.is_wet(p.0, p.1, p.2) {
                    return false;
                }
                let b = world.get(p.0, p.1, p.2);
                b == world.air || washable(b) || matches!(self.fluid(b), Some((f, d)) if f == fluid && d != 0)
            })
            .collect();
        if passable.is_empty() {
            return;
        }
        let distances: Vec<u8> = passable.iter().map(|(dx, dz)| self.hole_distance(world, (x + dx, y, z + dz), (-dx, -dz), 1, fluid, washable)).collect();
        let nearest = distances.iter().copied().min().unwrap_or(u8::MAX);
        let state = self.state(fluid, next);
        for (i, (dx, dz)) in passable.iter().enumerate() {
            let p = (x + dx, y, z + dz);
            if distances[i] == nearest && self.can_enter(world, p, fluid, next, washable) {
                self.set(world, now, p, state, changed);
            }
        }
    }

    /// Distance (en blocs) jusqu'à un trou où le liquide peut descendre, en partant de `pos`
    /// sans revenir en arrière ; `u8::MAX` si aucun à portée.
    fn hole_distance(&self, world: &World, pos: (i32, i32, i32), from: (i32, i32), dist: u8, fluid: Fluid, washable: &dyn Fn(u32) -> bool) -> u8 {
        let below = (pos.0, pos.1 - 1, pos.2);
        let b = world.get(below.0, below.1, below.2);
        if World::in_bounds(below.1) && (b == world.air || washable(b) || matches!(self.fluid(b), Some((f, _)) if f == fluid)) {
            return dist;
        }
        if dist >= fluid.reach() {
            return u8::MAX;
        }
        let mut best = u8::MAX;
        for (dx, dz) in SIDES {
            if (dx, dz) == from {
                continue;
            }
            let p = (pos.0 + dx, pos.1, pos.2 + dz);
            let s = world.get(p.0, p.1, p.2);
            if s == world.air || washable(s) {
                best = best.min(self.hole_distance(world, p, (-dx, -dz), dist + 1, fluid, washable));
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Option<(World, Liquids, GameData)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = GameData::load(&dir).ok()?;
        let liquids = Liquids::new(&data)?;
        Some((World::new(), liquids, data))
    }

    fn run(world: &mut World, l: &mut Liquids, ticks: u64) {
        for t in 0..ticks {
            l.tick(world, t, &|_| false);
        }
    }

    /// Le sol du monde plat est en y = -61 : on travaille juste au-dessus.
    #[test]
    fn water_spreads_seven_blocks_and_dries_up() {
        let Some((mut w, mut l, _)) = setup() else { return };
        let source = l.source(Fluid::Water);
        w.set(0, -60, 0, source);
        l.block_changed(&w, 0, (0, -60, 0));
        run(&mut w, &mut l, 200);
        assert_eq!(l.fluid(w.get(1, -60, 0)), Some((Fluid::Water, 1)));
        assert_eq!(l.fluid(w.get(7, -60, 0)), Some((Fluid::Water, 7)));
        assert_eq!(l.fluid(w.get(8, -60, 0)), None);
        assert_eq!(l.fluid(w.get(3, -60, 4)), Some((Fluid::Water, 7)));
        // Source retirée : tout s'assèche
        let air = w.air;
        w.set(0, -60, 0, air);
        l.block_changed(&w, 200, (0, -60, 0));
        for t in 200..600 {
            l.tick(&mut w, t, &|_| false);
        }
        for x in 0..8 {
            assert_eq!(l.fluid(w.get(x, -60, 0)), None, "reste de l'eau en x = {}", x);
        }
    }

    #[test]
    fn water_falls_then_spreads_and_lava_meets_water() {
        let Some((mut w, mut l, data)) = setup() else { return };
        let (water, lava) = (l.source(Fluid::Water), l.source(Fluid::Lava));
        // Une source posée en hauteur tombe jusqu'au sol puis s'étale
        w.set(0, -55, 0, water);
        l.block_changed(&w, 0, (0, -55, 0));
        run(&mut w, &mut l, 200);
        assert_eq!(l.fluid(w.get(0, -58, 0)), Some((Fluid::Water, FALLING)));
        // Une source en l'air tombe en une seule colonne, qui s'étale sur 7 blocs au sol
        assert_eq!(l.fluid(w.get(1, -55, 0)), None);
        assert_eq!(l.fluid(w.get(0, -60, 0)), Some((Fluid::Water, FALLING)));
        assert_eq!(l.fluid(w.get(7, -60, 0)), Some((Fluid::Water, 7)));
        assert_eq!(l.fluid(w.get(8, -60, 0)), None);
        // La lave avance de 2 en 2, sur 3 blocs
        w.set(40, -60, 40, lava);
        l.block_changed(&w, 200, (40, -60, 40));
        for t in 200..600 {
            l.tick(&mut w, t, &|_| false);
        }
        assert_eq!(l.fluid(w.get(43, -60, 40)), Some((Fluid::Lava, 6)));
        assert_eq!(l.fluid(w.get(44, -60, 40)), None);
        // Une source d'eau contre une source de lave : obsidienne
        w.set(39, -60, 40, water);
        l.block_changed(&w, 600, (39, -60, 40));
        for t in 600..700 {
            l.tick(&mut w, t, &|_| false);
        }
        assert_eq!(data.block_name(w.get(40, -60, 40)), Some("minecraft:obsidian"));
    }

    /// Un trou à 2 blocs : l'eau n'y va que par le chemin le plus court, et ne repart pas
    /// dans les autres directions une fois ce chemin rempli (bug constaté sur une falaise).
    #[test]
    fn water_keeps_heading_for_the_nearest_hole() {
        let Some((mut w, mut l, _)) = setup() else { return };
        let (air, water) = (w.air, l.source(Fluid::Water));
        for y in -63..=-61 {
            w.set(2, y, 0, air);
        }
        w.set(0, -60, 0, water);
        l.block_changed(&w, 0, (0, -60, 0));
        run(&mut w, &mut l, 300);
        assert_eq!(l.fluid(w.get(1, -60, 0)), Some((Fluid::Water, 1)));
        assert!(l.fluid(w.get(2, -61, 0)).is_some(), "l'eau doit tomber dans le trou");
        for (x, z) in [(-1, 0), (0, 1), (0, -1)] {
            assert_eq!(l.fluid(w.get(x, -60, z)), None, "l'eau est partie en {} {}", x, z);
        }
    }

    /// Deux sources côte à côte remplissent la case entre elles (source infinie).
    #[test]
    fn two_sources_make_a_third() {
        let Some((mut w, mut l, _)) = setup() else { return };
        let water = l.source(Fluid::Water);
        w.set(0, -60, 0, water);
        w.set(2, -60, 0, water);
        l.block_changed(&w, 0, (0, -60, 0));
        l.block_changed(&w, 0, (2, -60, 0));
        run(&mut w, &mut l, 100);
        assert_eq!(l.fluid(w.get(1, -60, 0)), Some((Fluid::Water, 0)));
    }
}

#[cfg(test)]
mod real_palette {
    use super::*;
    use crate::world::generator::{Generator, Kind, Palette};

    /// Avec la palette du serveur (et non celle des tests), l'eau générée est bien une source
    /// reconnue, et elle coule dans un trou creusé à côté.
    #[test]
    fn generated_water_flows_into_a_hole() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let Ok(data) = GameData::load(&dir) else { return };
        let p = Palette::new(&|n| data.state_from_spec(n));
        let mut l = Liquids::new(&data).unwrap();
        assert_eq!(l.fluid(p.water), Some((Fluid::Water, 0)), "eau du terrain non reconnue : {:?}", data.block_name(p.water));
        let mut w = World::with_generator(Generator::new(Kind::Flat, 0, p));
        let water = w.generator().blocks.water;
        w.set(0, -60, 0, water);
        l.block_changed(&w, 0, (0, -60, 0));
        for t in 0..100 {
            l.tick(&mut w, t, &|_| false);
        }
        assert!(l.fluid(w.get(3, -60, 0)).is_some());
    }
}

#[cfg(test)]
mod shape_dump {
    use super::*;
    #[test]
    #[ignore]
    fn dump_shapes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = GameData::load(&dir).unwrap();
        let mut l = Liquids::new(&data).unwrap();
        let mut w = World::new();
        let air = w.air;
        // Plateau : sol à -61 ; on creuse un trou en (3, -61, 0) et une falaise pour x >= 6
        for x in 6..12 { for z in -8..9 { for y in -63..=-61 { w.set(x, y, z, air); } } }
        w.set(3, -61, 0, air);
        let water = l.source(Fluid::Water);
        w.set(0, -60, 0, water);
        l.block_changed(&w, 0, (0, -60, 0));
        // Source vidée contre la paroi de la falaise, en l'air
        w.set(6, -61, 6, water);
        l.block_changed(&w, 0, (6, -61, 6));
        for t in 0..400 { l.tick(&mut w, t, &|_| false); }
        for y in [-60, -61, -62, -63] {
            eprintln!("y = {}", y);
            for z in -8..9 {
                let mut s = String::new();
                for x in -8..12 {
                    let b = w.get(x, y, z);
                    s.push(match l.fluid(b) { Some((_, 0)) => 'S', Some((_, d)) if d >= 8 => 'F', Some((_, d)) => char::from(b'0' + d), None => if b == air { '.' } else { '#' } });
                }
                eprintln!("{}", s);
            }
        }
    }
}

#[cfg(test)]
mod real_case {
    use super::*;
    use crate::world::generator::{Generator, Kind, Palette};
    #[test]
    #[ignore]
    fn replay_user_bucket() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = GameData::load(&dir).unwrap();
        let p = Palette::new(&|n| data.state_from_spec(n));
        let mut l = Liquids::new(&data).unwrap();
        let mut w = World::with_generator(Generator::new(Kind::Vanilla, 6692080325188732092, p));
        let (x0, y0, z0): (i32, i32, i32) = (std::env::var("BX").map(|v| v.parse().unwrap()).unwrap_or(-10588), std::env::var("BY").map(|v| v.parse().unwrap()).unwrap_or(71), std::env::var("BZ").map(|v| v.parse().unwrap()).unwrap_or(-1876));
        let washable = |s: u32| data.block_name(s).map(|n| { let n = n.trim_start_matches("minecraft:"); ["grass", "fern", "flower", "bush", "petals", "litter", "poppy", "dandelion", "tulip", "daisy", "bluet", "orchid", "allium", "cornflower", "snow_layer", "sapling", "mushroom"].iter().any(|k| n.contains(k)) && !n.contains("grass_block") }).unwrap_or(false);
        let water = l.source(Fluid::Water);
        w.set(x0, y0, z0, water);
        l.block_changed(&w, 0, (x0, y0, z0));
        let mut total = 0;
        for t in 0..1200 { total += l.tick(&mut w, t, &washable).len(); }
        eprintln!("changements : {}, en attente : {}", total, l.pending());
        // Vue de dessus : hauteur de l'eau la plus haute par colonne (lettre = y - (y0-20))
        let r = 24;
        let mut cells = 0;
        for z in z0 - r..=z0 + r {
            let mut s = String::new();
            for x in x0 - r..=x0 + r {
                let mut c = '.';
                for y in (y0 - 40..=y0).rev() {
                    let b = w.get(x, y, z);
                    if let Some((_, d)) = l.fluid(b) { cells += 1; c = if x == x0 && z == z0 { '@' } else if d == 0 { 'S' } else if d >= 8 { '|' } else { char::from(b'0' + d) }; break; }
                }
                s.push(c);
            }
            eprintln!("{}", s);
        }
        // Coupe verticale le long de x (z = z0)
        for y in (y0 - 25..=y0 + 1).rev() {
            let mut s = format!("{:4} ", y);
            for x in x0 - r..=x0 + r {
                let b = w.get(x, y, z0);
                s.push(match l.fluid(b) { Some((_, 0)) => 'S', Some((_, d)) if d >= 8 => '|', Some((_, d)) => char::from(b'0' + d), None => if b == w.air || washable(b) { ' ' } else { '#' } });
            }
            eprintln!("{}", s);
        }
        eprintln!("colonnes avec eau : {}", cells);
    }
}
