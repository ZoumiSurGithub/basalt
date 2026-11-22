//! Structures du terrain vanilla : villages, pyramides du désert, temples de la jungle,
//! mines abandonnées, avant-postes de pillards, cités antiques, ruines de piste et chambres
//! des épreuves.
//!
//! Si les plans officiels du jeu sont fournis (voir `templates`), les villages, avant-postes,
//! cités antiques et ruines de piste sont assemblés avec eux. Sinon, et pour les autres
//! structures, chaque structure est reconstruite à la main, au plus près de celle du jeu.
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
use super::templates::{pick as pick_plan, Layout, Rules, Templates};
use super::vanilla::{Vanilla, B};
use super::{MAX_Y, MIN_Y};

/// Ce que contient un coffre de structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loot {
    Village,
    DesertPyramid,
    JungleTemple,
    Mineshaft,
    PillagerOutpost,
    AncientCity,
    TrialCorridor,
    TrialSupply,
    ShipwreckSupply,
    ShipwreckTreasure,
    ShipwreckMap,
    OceanRuin,
    Igloo,
    RuinedPortal,
    WoodlandMansion,
}

/// Une structure calculée : ses blocs et ses coffres, rangés par chunk.
#[derive(Default)]
pub struct Plan {
    blocks: HashMap<(i32, i32), Vec<(u8, i16, u8, u32)>>,
    /// Colonnes (x, z locaux, hauteur) au-dessus desquelles les arbres sont abattus
    clear: HashMap<(i32, i32), Vec<(u8, u8, i16)>>,
    chests: HashMap<(i32, i32), Vec<(u8, i16, u8, Loot)>>,
    beds: HashMap<(i32, i32), Vec<(u8, i16, u8, u8)>>,
    /// Zones (x min, z min, x max, z max) où aucun arbre ne pousse : un arbre dont le pied y
    /// tombe n'est pas dessiné du tout, plutôt que coupé
    no_trees: Vec<(i32, i32, i32, i32)>,
}

impl Plan {
    /// Blocs de la structure qui tombent dans ce chunk : (x, y, z locaux, état).
    pub fn blocks_in(&self, cx: i32, cz: i32) -> &[(u8, i16, u8, u32)] {
        self.blocks.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Vrai si un arbre dont le pied est en (x, z) gênerait la structure.
    pub fn blocks_tree(&self, x: i32, z: i32) -> bool {
        self.no_trees.iter().any(|&(x0, z0, x1, z1)| x >= x0 && x <= x1 && z >= z0 && z <= z1)
    }

    /// Lits de la structure dans ce chunk, avec leur couleur.
    pub fn beds_in(&self, cx: i32, cz: i32) -> &[(u8, i16, u8, u8)] {
        self.beds.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Colonnes de ce chunk où les arbres sont abattus au-dessus d'une hauteur.
    pub fn clear_in(&self, cx: i32, cz: i32) -> &[(u8, u8, i16)] {
        self.clear.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Coffres de la structure dans ce chunk, avec leur table de butin.
    pub fn chests_in(&self, cx: i32, cz: i32) -> &[(u8, i16, u8, Loot)] {
        self.chests.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Plan vide : rien à construire ici (biome ou terrain refusé).
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
    PillagerOutpost,
    TrailRuins,
    TrialChambers,
    AncientCity,
    Shipwreck,
    OceanRuin,
    Igloo,
    RuinedPortal,
    /// Squelette fossile enfoui (désert, marais) : un chunk sur 64, comme les mines
    Fossil,
    Mansion,
}

/// Structures placées sur une grille : (sorte, espacement, séparation, sel, portée en chunks).
const GRID: [(Kind, i32, i32, u64, i32); 12] = [
    (Kind::Village, 34, 8, 10_387_312, 6),
    (Kind::DesertPyramid, 32, 8, 14_357_617, 2),
    (Kind::JungleTemple, 32, 8, 14_357_619, 2),
    (Kind::PillagerOutpost, 32, 8, 165_745_296, 2),
    (Kind::TrailRuins, 34, 8, 83_469_867, 6),
    (Kind::TrialChambers, 34, 12, 94_251_327, 4),
    (Kind::AncientCity, 24, 8, 20_083_232, 8),
    (Kind::Shipwreck, 24, 4, 165_745_295, 2),
    (Kind::OceanRuin, 20, 8, 14_357_621, 3),
    (Kind::Igloo, 32, 8, 14_357_618, 1),
    (Kind::RuinedPortal, 40, 15, 34_222_645, 2),
    (Kind::Mansion, 80, 20, 10_387_319, 8),
];

/// Plans déjà calculés (y compris « rien ici »), partagés par les fils de génération.
#[derive(Default)]
pub struct PlanCache {
    /// Deux générations : les plans récents, et ceux d'avant le dernier tri. Un plan
    /// demandé de nouveau repasse dans les récents ; les autres sont oubliés au tri suivant.
    /// (Auparavant tout était vidé d'un coup : les grandes structures proches, comme une
    /// cité antique, étaient alors recalculées en boucle.)
    plans: Mutex<(HashMap<(Kind, i32, i32), Arc<Plan>>, HashMap<(Kind, i32, i32), Arc<Plan>>)>,
    /// Vrai dès qu'un chunk a été généré avec les plans officiels (sans eux, les épaves,
    /// ruines océaniques, igloos et portails en ruine n'existent pas : `/locate` le sait)
    official: std::sync::atomic::AtomicBool,
}

const CACHE_PLANS: usize = 2048;

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
    if p.templates.is_some() {
        cache.official.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut starts: Vec<(Kind, i32, i32)> = Vec::new();
    for (kind, spacing, separation, salt, reach) in GRID {
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
            // Fossiles : petits, un chunk de portée suffit
            if dx.abs() <= 1 && dz.abs() <= 1 && p.templates.is_some() && mix(seed, (cx + dx) as i64, (cz + dz) as i64, 77_002) % 64 == 0 {
                starts.push((Kind::Fossil, cx + dx, cz + dz));
            }
        }
    }
    let mut out = Vec::new();
    for key in starts {
        let known = cache.plans.lock().ok().and_then(|mut m| {
            if let Some(p) = m.0.get(&key) {
                return Some(p.clone());
            }
            let p = m.1.remove(&key)?;
            m.0.insert(key, p.clone());
            Some(p)
        });
        let plan = match known {
            Some(plan) => plan,
            None => {
                let plan = Arc::new(build(v, p, key));
                if let Ok(mut m) = cache.plans.lock() {
                    if m.0.len() >= CACHE_PLANS {
                        let recent = std::mem::take(&mut m.0);
                        m.1 = recent;
                    }
                    m.0.insert(key, plan.clone());
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

/// Construit le plan complet d'une structure à partir de son point de départ.
fn build(v: &Vanilla, p: &Palette, (kind, sx, sz): (Kind, i32, i32)) -> Plan {
    let mut b = Builder::new(v, p);
    let mut rng = Rng::new(mix(v.seed(), sx as i64, sz as i64, 31_337 + kind as u64));
    let (x, z) = (sx * 16 + 8, sz * 16 + 8);
    // Plans officiels du jeu, s'ils sont fournis : villages, avant-postes, cités antiques et
    // ruines de piste assemblés comme dans le jeu
    if let Some(t) = p.templates.clone() {
        if official(&mut b, &t, &mut rng, kind, sx, sz) {
            return b.finish();
        }
    }
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
        Kind::PillagerOutpost => {
            if let Some((x, y, z)) = placed(v, kind, sx, sz) {
                pillager_outpost(&mut b, &mut rng, x - 3, y - 1, z - 3);
            }
        }
        Kind::TrailRuins => {
            if placed(v, kind, sx, sz).is_some() {
                trail_ruins(&mut b, &mut rng, x, z);
            }
        }
        Kind::TrialChambers => {
            if let Some((x, y, z)) = placed(v, kind, sx, sz) {
                trial_chambers(&mut b, &mut rng, x, y, z);
            }
        }
        Kind::AncientCity => {
            if placed(v, kind, sx, sz).is_some() {
                ancient_city(&mut b, &mut rng, x, z);
            }
        }
        // Sans les plans du jeu, pas de version faite à la main
        Kind::Shipwreck | Kind::OceanRuin | Kind::Igloo | Kind::RuinedPortal | Kind::Fossil | Kind::Mansion => {}
    }
    b.finish()
}

// --- Manoir des bois --------------------------------------------------------------------
//
// Le manoir ne s'emboîte pas par jigsaws : le jeu dessine d'abord un plan d'étages sur une
// grille de 11 × 11 cases de 8 blocs (couloirs, pièces de 1 × 1, 1 × 2 et 2 × 2), puis pose
// les murs extérieurs en faisant le tour, les toits, les tapis des couloirs, les cloisons et
// enfin une pièce tirée au hasard dans chaque emplacement. Ce qui suit reprend ces règles
// (WoodlandMansionPieces dans le jeu), avec les plans officiels.

/// Directions de la grille du manoir (x vers l'est, y vers le sud).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum M4 {
    N,
    E,
    S,
    W,
    /// Pièce sans porte vers un couloir : son entrée est « par le haut » (pièce secrète)
    Up,
}

impl M4 {
    const HORIZONTAL: [M4; 4] = [M4::N, M4::E, M4::S, M4::W];
    fn step(self) -> (i32, i32) {
        match self {
            M4::N => (0, -1),
            M4::E => (1, 0),
            M4::S => (0, 1),
            M4::W => (-1, 0),
            M4::Up => (0, 0),
        }
    }
    fn cw(self) -> M4 {
        match self {
            M4::N => M4::E,
            M4::E => M4::S,
            M4::S => M4::W,
            M4::W => M4::N,
            M4::Up => M4::Up,
        }
    }
    fn ccw(self) -> M4 {
        self.cw().cw().cw()
    }
    fn opposite(self) -> M4 {
        self.cw().cw()
    }
    /// Direction.from2DDataValue du jeu : 0 sud, 1 ouest, 2 nord, 3 est
    fn from_2d(n: u32) -> M4 {
        [M4::S, M4::W, M4::N, M4::E][(n & 3) as usize]
    }
    /// Tournée de `r` quarts de tour (sens des aiguilles d'une montre)
    fn turn(self, r: u8) -> M4 {
        (0..(r & 3)).fold(self, |d, _| d.cw())
    }
}

const M_CLEAR: i32 = 0;
const M_CORRIDOR: i32 = 1;
const M_ROOM: i32 = 2;
const M_START: i32 = 3;
const M_BLOCKED: i32 = 5;
const M_1X1: i32 = 65536;
const M_1X2: i32 = 131072;
const M_2X2: i32 = 262144;
const M_ORIGIN: i32 = 1048576;
const M_DOOR: i32 = 2097152;
const M_STAIRS: i32 = 4194304;
const M_CORRIDOR_FLAG: i32 = 8388608;
const M_TYPE: i32 = 983040;
const M_ID: i32 = 65535;

/// Grille de 11 × 11 ; hors de la grille, tout est « bloqué ».
#[derive(Clone)]
struct MGrid([[i32; 11]; 11]);

impl MGrid {
    fn new(v: i32) -> MGrid {
        MGrid([[v; 11]; 11])
    }
    fn get(&self, x: i32, y: i32) -> i32 {
        if (0..11).contains(&x) && (0..11).contains(&y) {
            self.0[x as usize][y as usize]
        } else {
            M_BLOCKED
        }
    }
    fn set(&mut self, x: i32, y: i32, v: i32) {
        if (0..11).contains(&x) && (0..11).contains(&y) {
            self.0[x as usize][y as usize] = v;
        }
    }
    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, v: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, v);
            }
        }
    }
    fn set_if(&mut self, x: i32, y: i32, was: i32, v: i32) {
        if self.get(x, y) == was {
            self.set(x, y, v);
        }
    }
    fn edges_to(&self, x: i32, y: i32, v: i32) -> bool {
        self.get(x - 1, y) == v || self.get(x + 1, y) == v || self.get(x, y + 1) == v || self.get(x, y - 1) == v
    }
}

fn is_house(g: &MGrid, x: i32, y: i32) -> bool {
    matches!(g.get(x, y), 1..=4)
}

/// Plan d'étages du manoir (MansionGrid du jeu).
struct MansionGrid {
    base: MGrid,
    third: MGrid,
    rooms: [MGrid; 3],
    entrance: (i32, i32),
}

impl MansionGrid {
    fn new(rng: &mut Rng) -> MansionGrid {
        let (ex, ey) = (7, 4);
        let mut base = MGrid::new(M_CLEAR);
        base.fill(ex, ey, ex + 1, ey + 1, M_START);
        base.fill(ex - 1, ey, ex - 1, ey + 1, M_ROOM);
        base.fill(ex + 2, ey - 2, ex + 3, ey + 3, M_BLOCKED);
        base.fill(ex + 1, ey - 2, ex + 1, ey - 1, M_CORRIDOR);
        base.fill(ex + 1, ey + 2, ex + 1, ey + 3, M_CORRIDOR);
        base.set(ex - 1, ey - 1, M_CORRIDOR);
        base.set(ex - 1, ey + 2, M_CORRIDOR);
        base.fill(0, 0, 11, 1, M_BLOCKED);
        base.fill(0, 9, 11, 11, M_BLOCKED);
        corridor(&mut base, rng, ex, ey - 2, M4::W, 6);
        corridor(&mut base, rng, ex, ey + 3, M4::W, 6);
        corridor(&mut base, rng, ex - 2, ey - 1, M4::W, 3);
        corridor(&mut base, rng, ex - 2, ey + 2, M4::W, 3);
        while clean_edges(&mut base) {}
        let mut g = MansionGrid { base, third: MGrid::new(M_BLOCKED), rooms: [MGrid::new(M_CLEAR), MGrid::new(M_CLEAR), MGrid::new(M_CLEAR)], entrance: (ex, ey) };
        let base = g.base.clone();
        identify_rooms(&base, &mut g.rooms[0], rng);
        identify_rooms(&base, &mut g.rooms[1], rng);
        g.rooms[0].fill(ex + 1, ey, ex + 1, ey + 1, M_CORRIDOR_FLAG);
        g.rooms[1].fill(ex + 1, ey, ex + 1, ey + 1, M_CORRIDOR_FLAG);
        g.setup_third_floor(rng);
        let third = g.third.clone();
        identify_rooms(&third, &mut g.rooms[2], rng);
        g
    }

    fn is_room_id(&self, x: i32, y: i32, floor: usize, id: i32) -> bool {
        (self.rooms[floor].get(x, y) & M_ID) == id
    }

    fn room_1x2_direction(&self, x: i32, y: i32, floor: usize, id: i32) -> Option<M4> {
        M4::HORIZONTAL.into_iter().find(|d| {
            let (dx, dy) = d.step();
            self.is_room_id(x + dx, y + dy, floor, id)
        })
    }

    fn setup_third_floor(&mut self, rng: &mut Rng) {
        let mut list = Vec::new();
        for y in 0..11 {
            for x in 0..11 {
                let k = self.rooms[1].get(x, y);
                if k & M_TYPE == M_1X2 && k & M_DOOR == M_DOOR {
                    list.push((x, y));
                }
            }
        }
        if list.is_empty() {
            self.third.fill(0, 0, 11, 11, M_BLOCKED);
            return;
        }
        let (tx, ty) = list[rng.below(list.len() as u32) as usize];
        let l1 = self.rooms[1].get(tx, ty);
        self.rooms[1].set(tx, ty, l1 | M_STAIRS);
        let Some(d) = self.room_1x2_direction(tx, ty, 1, l1 & M_ID) else {
            self.third.fill(0, 0, 11, 11, M_BLOCKED);
            return;
        };
        let (sx, sy) = d.step();
        let (i2, i1) = (tx + sx, ty + sy);
        for y in 0..11 {
            for x in 0..11 {
                if !is_house(&self.base, x, y) {
                    self.third.set(x, y, M_BLOCKED);
                } else if x == tx && y == ty {
                    self.third.set(x, y, M_START);
                } else if x == i2 && y == i1 {
                    self.third.set(x, y, M_START);
                    self.rooms[2].set(x, y, M_CORRIDOR_FLAG);
                } else {
                    self.third.set(x, y, M_CLEAR);
                }
            }
        }
        let free: Vec<M4> = M4::HORIZONTAL
            .into_iter()
            .filter(|d| {
                let (dx, dy) = d.step();
                self.third.get(i2 + dx, i1 + dy) == M_CLEAR
            })
            .collect();
        if free.is_empty() {
            self.third.fill(0, 0, 11, 11, M_BLOCKED);
            self.rooms[1].set(tx, ty, l1);
        } else {
            let d2 = free[rng.below(free.len() as u32) as usize];
            let (dx, dy) = d2.step();
            corridor(&mut self.third, rng, i2 + dx, i1 + dy, d2, 4);
            while clean_edges(&mut self.third) {}
        }
    }
}

/// Couloir qui avance et bifurque au hasard, bordé de cases à pièces.
fn corridor(g: &mut MGrid, rng: &mut Rng, x: i32, y: i32, d: M4, length: i32) {
    if length <= 0 {
        return;
    }
    let (sx, sy) = d.step();
    g.set(x, y, M_CORRIDOR);
    g.set_if(x + sx, y + sy, M_CLEAR, M_CORRIDOR);
    for _ in 0..8 {
        let d1 = M4::from_2d(rng.below(4));
        if d1 != d.opposite() && (d1 != M4::E || rng.below(2) == 0) {
            let (j, k) = (x + sx, y + sy);
            let (ax, ay) = d1.step();
            if g.get(j + ax, k + ay) == M_CLEAR && g.get(j + ax * 2, k + ay * 2) == M_CLEAR {
                corridor(g, rng, x + sx + ax, y + sy + ay, d1, length - 1);
                break;
            }
        }
    }
    let (c, cc) = (d.cw().step(), d.ccw().step());
    g.set_if(x + c.0, y + c.1, M_CLEAR, M_ROOM);
    g.set_if(x + cc.0, y + cc.1, M_CLEAR, M_ROOM);
    g.set_if(x + sx + c.0, y + sy + c.1, M_CLEAR, M_ROOM);
    g.set_if(x + sx + cc.0, y + sy + cc.1, M_CLEAR, M_ROOM);
    g.set_if(x + sx * 2, y + sy * 2, M_CLEAR, M_ROOM);
    g.set_if(x + c.0 * 2, y + c.1 * 2, M_CLEAR, M_ROOM);
    g.set_if(x + cc.0 * 2, y + cc.1 * 2, M_CLEAR, M_ROOM);
}

/// Bouche les creux du contour (cases presque entourées de maison).
fn clean_edges(g: &mut MGrid) -> bool {
    let mut changed = false;
    for y in 0..11 {
        for x in 0..11 {
            if g.get(x, y) != M_CLEAR {
                continue;
            }
            let k = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().filter(|(dx, dy)| is_house(g, x + dx, y + dy)).count();
            if k >= 3 {
                g.set(x, y, M_ROOM);
                changed = true;
            } else if k == 2 {
                let l = [(1, 1), (-1, 1), (1, -1), (-1, -1)].iter().filter(|(dx, dy)| is_house(g, x + dx, y + dy)).count();
                if l <= 1 {
                    g.set(x, y, M_ROOM);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Regroupe les cases à pièces en pièces de 2 × 2, 1 × 2 ou 1 × 1, avec leur porte.
fn identify_rooms(g: &MGrid, rooms: &mut MGrid, rng: &mut Rng) {
    let mut cells = Vec::new();
    for y in 0..11 {
        for x in 0..11 {
            if g.get(x, y) == M_ROOM {
                cells.push((x, y));
            }
        }
    }
    for i in (1..cells.len()).rev() {
        let j = rng.below(i as u32 + 1) as usize;
        cells.swap(i, j);
    }
    let mut id = 10;
    for (k, l) in cells {
        if rooms.get(k, l) != 0 {
            continue;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (k, k, l, l);
        let free = |rooms: &MGrid, x: i32, y: i32| rooms.get(x, y) == 0 && g.get(x, y) == M_ROOM;
        let kind = if free(rooms, k + 1, l) && free(rooms, k, l + 1) && free(rooms, k + 1, l + 1) {
            x1 = k + 1;
            y1 = l + 1;
            M_2X2
        } else if free(rooms, k - 1, l) && free(rooms, k, l + 1) && free(rooms, k - 1, l + 1) {
            x0 = k - 1;
            y1 = l + 1;
            M_2X2
        } else if free(rooms, k - 1, l) && free(rooms, k, l - 1) && free(rooms, k - 1, l - 1) {
            x0 = k - 1;
            y0 = l - 1;
            M_2X2
        } else if free(rooms, k + 1, l) {
            x1 = k + 1;
            M_1X2
        } else if free(rooms, k, l + 1) {
            y1 = l + 1;
            M_1X2
        } else if free(rooms, k - 1, l) {
            x0 = k - 1;
            M_1X2
        } else if free(rooms, k, l - 1) {
            y0 = l - 1;
            M_1X2
        } else {
            M_1X1
        };
        let mut dx = if rng.below(2) == 0 { x0 } else { x1 };
        let mut dy = if rng.below(2) == 0 { y0 } else { y1 };
        let mut door = M_DOOR;
        if !g.edges_to(dx, dy, M_CORRIDOR) {
            dx = if dx == x0 { x1 } else { x0 };
            dy = if dy == y0 { y1 } else { y0 };
            if !g.edges_to(dx, dy, M_CORRIDOR) {
                dy = if dy == y0 { y1 } else { y0 };
                if !g.edges_to(dx, dy, M_CORRIDOR) {
                    dx = if dx == x0 { x1 } else { x0 };
                    dy = if dy == y0 { y1 } else { y0 };
                    if !g.edges_to(dx, dy, M_CORRIDOR) {
                        door = 0;
                        dx = x0;
                        dy = y0;
                    }
                }
            }
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                if x == dx && y == dy {
                    rooms.set(x, y, M_ORIGIN | door | kind | id);
                } else {
                    rooms.set(x, y, kind | id);
                }
            }
        }
        id += 1;
    }
}

/// Une pièce du manoir à poser : plan, coin, rotation, miroir.
struct MPiece {
    name: String,
    pos: (i32, i32, i32),
    rot: u8,
    mirror: u8,
}

/// Pose des pièces (MansionPiecePlacer du jeu).
struct Placer<'r> {
    rng: &'r mut Rng,
    start: (i32, i32),
    pieces: Vec<MPiece>,
}

/// Position + n pas dans une direction (tournée par la rotation du manoir).
fn rel(p: (i32, i32, i32), d: M4, n: i32) -> (i32, i32, i32) {
    let (dx, dz) = d.step();
    (p.0 + dx * n, p.1, p.2 + dz * n)
}

struct Walk {
    pos: (i32, i32, i32),
    rot: u8,
    wall: &'static str,
}

impl Placer<'_> {
    fn add(&mut self, name: &str, pos: (i32, i32, i32), rot: u8) {
        self.add_m(name, pos, rot, 0);
    }
    fn add_m(&mut self, name: &str, pos: (i32, i32, i32), rot: u8, mirror: u8) {
        self.pieces.push(MPiece { name: name.to_string(), pos, rot: rot & 3, mirror });
    }

    fn create(&mut self, pos: (i32, i32, i32), rot: u8, grid: &MansionGrid) {
        let mut a = Walk { pos, rot, wall: "wall_flat" };
        // Entrée
        let w = M4::W.turn(rot);
        self.add("entrance", rel(a.pos, w, 9), rot);
        a.pos = rel(a.pos, M4::S.turn(rot), 16);
        let mut b = Walk { pos: (a.pos.0, a.pos.1 + 8, a.pos.2), rot, wall: "wall_window" };
        self.start = (grid.entrance.0 + 1, grid.entrance.1 + 1);
        let (i, j) = (grid.entrance.0 + 1, grid.entrance.1);
        let (sx, sy) = self.start;
        self.outer_walls(&mut a, &grid.base, M4::S, sx, sy, i, j);
        self.outer_walls(&mut b, &grid.base, M4::S, sx, sy, i, j);
        let mut c = Walk { pos: (a.pos.0, a.pos.1 + 19, a.pos.2), rot, wall: "wall_window" };
        'third: for k in 0..11 {
            for l in (0..11).rev() {
                if is_house(&grid.third, l, k) {
                    c.pos = rel(c.pos, M4::S.turn(rot), 8 + (k - sy) * 8);
                    c.pos = rel(c.pos, M4::E.turn(rot), (l - sx) * 8);
                    self.wall_piece(&mut c);
                    self.outer_walls(&mut c, &grid.third, M4::S, l, k, l, k);
                    break 'third;
                }
            }
        }
        self.roof((pos.0, pos.1 + 16, pos.2), rot, &grid.base, Some(&grid.third));
        self.roof((pos.0, pos.1 + 27, pos.2), rot, &grid.third, None);
        for floor in 0..3usize {
            let base = (pos.0, pos.1 + 8 * floor as i32 + if floor == 2 { 3 } else { 0 }, pos.2);
            let rooms = &grid.rooms[floor];
            let g = if floor == 2 { &grid.third } else { &grid.base };
            let (south, west) = if floor == 0 { ("carpet_south_1", "carpet_west_1") } else { ("carpet_south_2", "carpet_west_2") };
            for y in 0..11 {
                for x in 0..11 {
                    if g.get(x, y) != M_CORRIDOR {
                        continue;
                    }
                    let p = rel(rel(base, M4::S.turn(rot), 8 + (y - sy) * 8), M4::E.turn(rot), (x - sx) * 8);
                    self.add("corridor_floor", p, rot);
                    let open = |dx: i32, dy: i32| g.get(x + dx, y + dy) == M_CORRIDOR || rooms.get(x + dx, y + dy) & M_CORRIDOR_FLAG == M_CORRIDOR_FLAG;
                    let up = |p: (i32, i32, i32)| (p.0, p.1 + 1, p.2);
                    if open(0, -1) {
                        self.add("carpet_north", up(rel(p, M4::E.turn(rot), 1)), rot);
                    }
                    if open(1, 0) {
                        self.add("carpet_east", up(rel(rel(p, M4::S.turn(rot), 1), M4::E.turn(rot), 5)), rot);
                    }
                    if open(0, 1) {
                        self.add(south, rel(rel(p, M4::S.turn(rot), 5), M4::W.turn(rot), 1), rot);
                    }
                    if open(-1, 0) {
                        self.add(west, rel(rel(p, M4::W.turn(rot), 1), M4::N.turn(rot), 1), rot);
                    }
                }
            }
            let (wall, door) = if floor == 0 { ("indoors_wall_1", "indoors_door_1") } else { ("indoors_wall_2", "indoors_door_2") };
            for y in 0..11 {
                for x in 0..11 {
                    let mut stairs_top = floor == 2 && g.get(x, y) == M_START;
                    if g.get(x, y) != M_ROOM && !stairs_top {
                        continue;
                    }
                    let j2 = rooms.get(x, y);
                    let kind = j2 & M_TYPE;
                    let id = j2 & M_ID;
                    stairs_top = stairs_top && j2 & M_CORRIDOR_FLAG == M_CORRIDOR_FLAG;
                    let mut doors = Vec::new();
                    if j2 & M_DOOR == M_DOOR {
                        for d in M4::HORIZONTAL {
                            let (dx, dy) = d.step();
                            if g.get(x + dx, y + dy) == M_CORRIDOR {
                                doors.push(d);
                            }
                        }
                    }
                    let front = if !doors.is_empty() {
                        Some(doors[self.rng.below(doors.len() as u32) as usize])
                    } else if j2 & M_ORIGIN == M_ORIGIN {
                        Some(M4::Up)
                    } else {
                        None
                    };
                    let p3 = rel(rel(base, M4::S.turn(rot), 8 + (y - sy) * 8), M4::E.turn(rot), -1 + (x - sx) * 8);
                    let pick = |d: M4| if front == Some(d) { door } else { wall };
                    if is_house(g, x - 1, y) && !grid.is_room_id(x - 1, y, floor, id) {
                        self.add(pick(M4::W), p3, rot);
                    }
                    if g.get(x + 1, y) == M_CORRIDOR && !stairs_top {
                        self.add(pick(M4::E), rel(p3, M4::E.turn(rot), 8), rot);
                    }
                    if is_house(g, x, y + 1) && !grid.is_room_id(x, y + 1, floor, id) {
                        self.add(pick(M4::S), rel(rel(p3, M4::S.turn(rot), 7), M4::E.turn(rot), 7), rot + 1);
                    }
                    if g.get(x, y - 1) == M_CORRIDOR && !stairs_top {
                        self.add(pick(M4::N), rel(rel(p3, M4::N.turn(rot), 1), M4::E.turn(rot), 7), rot + 1);
                    }
                    if kind == M_1X1 {
                        self.room_1x1(p3, rot, front, floor);
                    } else if kind == M_1X2 {
                        if let Some(f) = front {
                            let side = grid.room_1x2_direction(x, y, floor, id);
                            let stairs = j2 & M_STAIRS == M_STAIRS;
                            if let Some(side) = side {
                                self.room_1x2(p3, rot, side, f, floor, stairs);
                            }
                        }
                    } else if kind == M_2X2 {
                        match front {
                            Some(M4::Up) => {
                                let name = "2x2_s1";
                                self.add(name, rel(p3, M4::E.turn(rot), 1), rot);
                            }
                            Some(f) => {
                                let mut d2 = f.cw();
                                let (dx, dy) = d2.step();
                                if !grid.is_room_id(x + dx, y + dy, floor, id) {
                                    d2 = d2.opposite();
                                }
                                self.room_2x2(p3, rot, d2, f, floor);
                            }
                            None => {}
                        }
                    }
                }
            }
        }
    }

    fn outer_walls(&mut self, w: &mut Walk, g: &MGrid, mut d: M4, sx: i32, sy: i32, ex: i32, ey: i32) {
        let (mut i, mut j) = (sx, sy);
        let d1 = d;
        loop {
            let (dx, dy) = d.step();
            let (cx, cy) = d.ccw().step();
            if !is_house(g, i + dx, j + dy) {
                self.turn(w);
                d = d.cw();
                if i != ex || j != ey || d1 != d {
                    self.wall_piece(w);
                }
            } else if is_house(g, i + dx + cx, j + dy + cy) {
                self.inner_turn(w);
                i += dx;
                j += dy;
                d = d.ccw();
            } else {
                i += dx;
                j += dy;
                if i != ex || j != ey || d1 != d {
                    self.wall_piece(w);
                }
            }
            if i == ex && j == ey && d1 == d {
                break;
            }
        }
    }

    fn wall_piece(&mut self, w: &mut Walk) {
        let p = rel(w.pos, M4::E.turn(w.rot), 7);
        self.add(w.wall, p, w.rot);
        w.pos = rel(w.pos, M4::S.turn(w.rot), 8);
    }

    fn turn(&mut self, w: &mut Walk) {
        w.pos = rel(w.pos, M4::S.turn(w.rot), -1);
        self.add("wall_corner", w.pos, w.rot);
        w.pos = rel(w.pos, M4::S.turn(w.rot), -7);
        w.pos = rel(w.pos, M4::W.turn(w.rot), -6);
        w.rot = (w.rot + 1) & 3;
    }

    fn inner_turn(&mut self, w: &mut Walk) {
        w.pos = rel(w.pos, M4::S.turn(w.rot), 6);
        w.pos = rel(w.pos, M4::E.turn(w.rot), 8);
        w.rot = (w.rot + 3) & 3;
    }

    fn roof(&mut self, pos: (i32, i32, i32), rot: u8, g: &MGrid, upper: Option<&MGrid>) {
        let (sx, sy) = self.start;
        let (s, e, w, n) = (M4::S.turn(rot), M4::E.turn(rot), M4::W.turn(rot), M4::N.turn(rot));
        let cell = |x: i32, y: i32| rel(rel(pos, s, 8 + (y - sy) * 8), e, (x - sx) * 8);
        for y in 0..11 {
            for x in 0..11 {
                let p = cell(x, y);
                let covered = upper.map(|u| is_house(u, x, y)).unwrap_or(false);
                if is_house(g, x, y) && !covered {
                    self.add("roof", (p.0, p.1 + 3, p.2), rot);
                    if !is_house(g, x + 1, y) {
                        self.add("roof_front", rel(p, e, 6), rot);
                    }
                    if !is_house(g, x - 1, y) {
                        self.add("roof_front", rel(rel(p, e, 0), s, 7), rot + 2);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("roof_front", rel(p, w, 1), rot + 3);
                    }
                    if !is_house(g, x, y + 1) {
                        self.add("roof_front", rel(rel(p, e, 6), s, 6), rot + 1);
                    }
                }
            }
        }
        if let Some(u) = upper {
            for y in 0..11 {
                for x in 0..11 {
                    let p = cell(x, y);
                    if !(is_house(g, x, y) && is_house(u, x, y)) {
                        continue;
                    }
                    if !is_house(g, x + 1, y) {
                        self.add("small_wall", rel(p, e, 7), rot);
                    }
                    if !is_house(g, x - 1, y) {
                        self.add("small_wall", rel(rel(p, w, 1), s, 6), rot + 2);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("small_wall", rel(rel(p, w, 0), n, 1), rot + 3);
                    }
                    if !is_house(g, x, y + 1) {
                        self.add("small_wall", rel(rel(p, e, 6), s, 7), rot + 1);
                    }
                    if !is_house(g, x + 1, y) {
                        if !is_house(g, x, y - 1) {
                            self.add("small_wall_corner", rel(rel(p, e, 7), n, 2), rot);
                        }
                        if !is_house(g, x, y + 1) {
                            self.add("small_wall_corner", rel(rel(p, e, 8), s, 7), rot + 1);
                        }
                    }
                    if !is_house(g, x - 1, y) {
                        if !is_house(g, x, y - 1) {
                            self.add("small_wall_corner", rel(rel(p, w, 2), n, 1), rot + 3);
                        }
                        if !is_house(g, x, y + 1) {
                            self.add("small_wall_corner", rel(rel(p, w, 1), s, 8), rot + 2);
                        }
                    }
                }
            }
        }
        for y in 0..11 {
            for x in 0..11 {
                let p = cell(x, y);
                let covered = upper.map(|u| is_house(u, x, y)).unwrap_or(false);
                if !is_house(g, x, y) || covered {
                    continue;
                }
                if !is_house(g, x + 1, y) {
                    let p16 = rel(p, e, 6);
                    if !is_house(g, x, y + 1) {
                        self.add("roof_corner", rel(p16, s, 6), rot);
                    } else if is_house(g, x + 1, y + 1) {
                        self.add("roof_inner_corner", rel(p16, s, 5), rot);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("roof_corner", p16, rot + 3);
                    } else if is_house(g, x + 1, y - 1) {
                        self.add("roof_inner_corner", rel(rel(p, e, 9), n, 2), rot + 1);
                    }
                }
                if !is_house(g, x - 1, y) {
                    let p17 = p;
                    if !is_house(g, x, y + 1) {
                        self.add("roof_corner", rel(p17, s, 6), rot + 1);
                    } else if is_house(g, x - 1, y + 1) {
                        self.add("roof_inner_corner", rel(rel(p17, s, 8), w, 3), rot + 3);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("roof_corner", p17, rot + 2);
                    } else if is_house(g, x - 1, y - 1) {
                        self.add("roof_inner_corner", rel(p17, s, 1), rot + 2);
                    }
                }
            }
        }
    }

    fn room_1x1(&mut self, pos: (i32, i32, i32), rot: u8, front: Option<M4>, floor: usize) {
        let mut r1 = 0u8;
        let mut name = if floor == 0 { format!("1x1_a{}", self.rng.below(5) + 1) } else { format!("1x1_b{}", self.rng.below(5) + 1) };
        match front {
            Some(M4::E) => {}
            Some(M4::N) => r1 = 3,
            Some(M4::S) => r1 = 1,
            Some(M4::W) => r1 = 2,
            _ => name = format!("1x1_as{}", self.rng.below(4) + 1),
        }
        // StructureTemplate.getZeroPositionWithTransform((1, 0, 0), aucun miroir, r1, 7, 7)
        let (zx, zz) = match r1 {
            1 => (1 + 7 - 1, 0),
            2 => (1 + 7 - 1, 7 - 1),
            3 => (1, 7 - 1),
            _ => (1, 0),
        };
        // Puis tournée avec le manoir
        let (zx, zz) = match rot & 3 {
            0 => (zx, zz),
            1 => (-zz, zx),
            2 => (-zx, -zz),
            _ => (zz, -zx),
        };
        self.add(&name, (pos.0 + zx, pos.1, pos.2 + zz), r1 + rot);
    }

    fn room_1x2(&mut self, pos: (i32, i32, i32), rot: u8, front: M4, side: M4, floor: usize, stairs: bool) {
        let (e, s, w, n) = (M4::E.turn(rot), M4::S.turn(rot), M4::W.turn(rot), M4::N.turn(rot));
        let side_name = |rng: &mut Rng| -> String {
            if floor == 0 {
                format!("1x2_a{}", rng.below(9) + 1)
            } else if stairs {
                "1x2_c_stairs".into()
            } else {
                format!("1x2_c{}", rng.below(4) + 1)
            }
        };
        let front_name = |rng: &mut Rng| -> String {
            if floor == 0 {
                format!("1x2_b{}", rng.below(5) + 1)
            } else if stairs {
                "1x2_d_stairs".into()
            } else {
                format!("1x2_d{}", rng.below(5) + 1)
            }
        };
        let secret = |rng: &mut Rng| -> String {
            if floor == 0 {
                format!("1x2_s{}", rng.below(2) + 1)
            } else {
                "1x2_se1".into()
            }
        };
        use M4::*;
        match (side, front) {
            (E, S) => {
                let n_ = side_name(self.rng);
                self.add(&n_, rel(pos, e, 1), rot)
            }
            (E, N) => {
                let n_ = side_name(self.rng);
                self.add_m(&n_, rel(rel(pos, e, 1), s, 6), rot, 1)
            }
            (W, N) => {
                let n_ = side_name(self.rng);
                self.add(&n_, rel(rel(pos, e, 7), s, 6), rot + 2)
            }
            (W, S) => {
                let n_ = side_name(self.rng);
                self.add_m(&n_, rel(pos, e, 7), rot, 2)
            }
            (S, E) => {
                let n_ = side_name(self.rng);
                self.add_m(&n_, rel(pos, e, 1), rot + 1, 1)
            }
            (S, W) => {
                let n_ = side_name(self.rng);
                self.add(&n_, rel(pos, e, 7), rot + 1)
            }
            (N, W) => {
                let n_ = side_name(self.rng);
                self.add_m(&n_, rel(rel(pos, e, 7), s, 6), rot + 1, 2)
            }
            (N, E) => {
                let n_ = side_name(self.rng);
                self.add(&n_, rel(rel(pos, e, 1), s, 6), rot + 3)
            }
            (S, N) => {
                let n_ = front_name(self.rng);
                self.add(&n_, rel(rel(pos, e, 1), n, 8), rot)
            }
            (N, S) => {
                let n_ = front_name(self.rng);
                self.add(&n_, rel(rel(pos, e, 7), s, 14), rot + 2)
            }
            (W, E) => {
                let n_ = front_name(self.rng);
                self.add(&n_, rel(pos, e, 15), rot + 1)
            }
            (E, W) => {
                let n_ = front_name(self.rng);
                self.add(&n_, rel(rel(pos, w, 7), s, 6), rot + 3)
            }
            (Up, E) => {
                let n_ = secret(self.rng);
                self.add(&n_, rel(pos, e, 15), rot + 1)
            }
            (Up, S) => {
                let n_ = secret(self.rng);
                self.add(&n_, rel(pos, e, 1), rot)
            }
            _ => {}
        }
    }

    fn room_2x2(&mut self, pos: (i32, i32, i32), rot: u8, front: M4, side: M4, floor: usize) {
        use M4::*;
        let (mut i, mut j, mut r1, mut mirror) = (0, 0, rot, 0u8);
        match (side, front) {
            (E, S) => i = -7,
            (E, N) => {
                i = -7;
                j = 6;
                mirror = 1;
            }
            (N, E) => {
                i = 1;
                j = 14;
                r1 = rot + 3;
            }
            (N, W) => {
                i = 7;
                j = 14;
                r1 = rot + 3;
                mirror = 1;
            }
            (S, W) => {
                i = 7;
                j = -8;
                r1 = rot + 1;
            }
            (S, E) => {
                i = 1;
                j = -8;
                r1 = rot + 1;
                mirror = 1;
            }
            (W, N) => {
                i = 15;
                j = 6;
                r1 = rot + 2;
            }
            (W, S) => {
                i = 15;
                mirror = 2;
            }
            _ => {}
        }
        let p = rel(rel(pos, E.turn(rot), i), S.turn(rot), j);
        let name = if floor == 0 { format!("2x2_a{}", self.rng.below(4) + 1) } else { format!("2x2_b{}", self.rng.below(5) + 1) };
        self.add_m(&name, p, r1, mirror);
    }
}

/// Construit le manoir : plan d'étages, pièces, coffres, socle de pierre.
fn mansion(b: &mut Builder, t: &Templates, rng: &mut Rng, x: i32, y: i32, z: i32) -> bool {
    if t.get("mansion/entrance").is_none() {
        return false;
    }
    let rot = rng.below(4) as u8;
    let grid = MansionGrid::new(rng);
    let mut placer = Placer { rng, start: (0, 0), pieces: Vec::new() };
    placer.create((x, y, z), rot, &grid);
    let pieces = std::mem::take(&mut placer.pieces);
    let rng = placer.rng;
    let air = b.p.air;
    let mut lowest: HashMap<(i32, i32), i32> = HashMap::new();
    let (mut min, mut max) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
    for piece in &pieces {
        // « indoors_wall_1 » s'appelle « indoors_wall » dans les fichiers
        let path = format!("mansion/{}", piece.name);
        let Some(tpl) = t.get(&path).or_else(|| t.get(path.trim_end_matches("_1"))) else { continue };
        // Position d'un point du plan : miroir, puis rotation autour du coin du plan (le jeu)
        let place = |(lx, ly, lz): (i32, i32, i32)| {
            let (mut px, mut pz) = (lx, lz);
            match piece.mirror {
                1 => pz = -pz,
                2 => px = -px,
                _ => {}
            }
            let (rx, rz) = match piece.rot & 3 {
                0 => (px, pz),
                1 => (-pz, px),
                2 => (-px, -pz),
                _ => (pz, -px),
            };
            (piece.pos.0 + rx, piece.pos.1 + ly, piece.pos.2 + rz)
        };
        for (local, state) in tpl.blocks_mirrored(piece.mirror, piece.rot) {
            let at = place(local);
            b.put(at, state);
            if state != air {
                let e = lowest.entry((at.0, at.2)).or_insert(at.1);
                *e = (*e).min(at.1);
                min = (min.0.min(at.0), min.1.min(at.2));
                max = (max.0.max(at.0), max.1.max(at.2));
            }
        }
        // Coffres : repère « Chest », « ChestWest »… à leur place, tournés avec la pièce
        for (local, m) in tpl.markers_unturned() {
            if let Some(dir) = m.strip_prefix("Chest") {
                let facing = match dir {
                    "West" => D::W,
                    "East" => D::E,
                    "South" => D::S,
                    "North" => D::N,
                    _ => D::S,
                };
                let at = place(local);
                b.chest(at, if dir.is_empty() { facing } else { facing.turn(piece.rot) }, Loot::WoodlandMansion);
            }
        }
    }
    let _ = rng;
    // Socle : sous le rez-de-chaussée, de la pierre jusqu'au sol (le jeu fait de même)
    let cobble = b.id("cobblestone");
    for ((cx, cz), low) in lowest {
        if low > y + 1 {
            continue;
        }
        let g = b.top(cx, cz);
        for yy in (g + 1).max(low - 24)..low {
            b.put((cx, yy, cz), cobble);
        }
    }
    if min.0 <= max.0 {
        b.no_trees.push((min.0 - 3, min.1 - 3, max.0 + 3, max.1 + 3));
        for cx in min.0..=max.0 {
            for cz in min.1..=max.1 {
                b.clear.push((cx, cz, y));
            }
        }
    }
    true
}

// --- Plans officiels ---------------------------------------------------------------------

/// Construit une structure avec les plans du jeu. Faux si cette sorte n'en a pas (ou si le
/// lieu ne convient pas : rien n'est alors construit, comme avec les plans faits à la main).
fn official(b: &mut Builder, t: &Templates, rng: &mut Rng, kind: Kind, sx: i32, sz: i32) -> bool {
    let (x, z) = (sx * 16 + 8, sz * 16 + 8);
    match kind {
        Kind::Village => {
            let Some(style) = Style::for_biome(b.biome(x, z)) else { return true };
            let theme = match style.theme {
                Theme::Plains => "plains",
                Theme::Desert => "desert",
                Theme::Savanna => "savanna",
                Theme::Taiga => "taiga",
                Theme::Snowy => "snowy",
            };
            // Pas de village dans l'eau
            if b.top(x, z) <= SEA_LEVEL {
                return true;
            }
            let Some(start) = pick_plan(&t.pool(&format!("village/{}/town_centers", theme)), rng) else { return false };
            let rot = rng.below(4) as u8;
            let y = ground(b, x, z);
            let terminators = format!("village/{}/terminators", theme);
            let rules = Rules {
                depth: 6,
                reach: 80,
                follows_ground: &|pool| pool.ends_with("/streets") || pool.ends_with("/terminators"),
                fallback: &|pool| pool.ends_with("/streets").then(|| terminators.clone()),
                empty_chance: &|pool| {
                    if pool.ends_with("/houses") {
                        0.15
                    } else if pool.ends_with("/decor") {
                        0.7
                    } else if pool.ends_with("/streets") || pool.ends_with("/town_centers") || pool.ends_with("/well_bottoms") {
                        0.0
                    } else {
                        // Animaux, chats, villageois, arbres : pas de blocs à poser
                        1.0
                    }
                },
                max_drop: Some((4, 6)),
            };
            let layout = t.assemble(start, rot, (x, z), y, &rules, rng, &mut |x, z| (ground(b, x, z), b.top(x, z)));
            let under = if style.theme == Theme::Desert { "sandstone" } else { "dirt" };
            place_layout(b, rng, &layout, Placing { under: Some(under), air: true, planks: Some(style.planks), clear_trees: true, archaeology: false, carve: false, integrity: 1.0, markers: no_markers, slopes: true });
            true
        }
        Kind::PillagerOutpost => {
            let Some((x, y, z)) = placed(b.v, kind, sx, sz) else { return true };
            let tower = if rng.below(4) == 0 { "pillageroutpost/watchtower_overgrown" } else { "pillageroutpost/watchtower" };
            let Some(tower) = t.get(tower) else { return false };
            let rules = Rules { depth: 0, reach: 40, follows_ground: &|_| false, fallback: &|_| None, empty_chance: &|_| 1.0, max_drop: None };
            let mut layout = t.assemble(tower, rng.below(4) as u8, (x, z), y - 1, &rules, rng, &mut |x, z| (ground(b, x, z), b.top(x, z)));
            // Tentes, cages, rondins et cibles autour de la tour, posés sur le sol
            let features = t.pool("pillager_outpost/features");
            for _ in 0..(3 + rng.below(3)) {
                let Some(f) = pick_plan(&features, rng) else { break };
                let angle = rng.unit() * std::f64::consts::TAU;
                let dist = 13.0 + rng.unit() * 8.0;
                let (fx, fz) = (x + (angle.cos() * dist) as i32, z + (angle.sin() * dist) as i32);
                let fy = ground(b, fx, fz) + 1;
                let piece = t.assemble(f, rng.below(4) as u8, (fx, fz), fy, &rules, rng, &mut |x, z| (ground(b, x, z), b.top(x, z)));
                if piece.pieces.iter().all(|p| layout.pieces.iter().all(|q| !overlap(&p.bbox, &q.bbox))) {
                    layout.pieces.extend(piece.pieces);
                }
            }
            place_layout(b, rng, &layout, Placing { under: Some("dirt"), air: true, planks: None, clear_trees: true, archaeology: false, carve: false, integrity: 1.0, markers: no_markers, slopes: true });
            true
        }
        Kind::AncientCity => {
            if placed(b.v, kind, sx, sz).is_none() {
                return true;
            }
            let Some(start) = pick_plan(&t.pool("ancient_city/city_center"), rng) else { return false };
            let rot = rng.below(4) as u8;
            // Le point d'ancrage du centre est à y = -27, comme dans le jeu
            let anchor = start.anchor_y("minecraft:city_anchor").unwrap_or(24);
            let rules = Rules { depth: 7, reach: 116, follows_ground: &|_| false, fallback: &|_| None, empty_chance: &|_| 0.0, max_drop: None };
            let layout = t.assemble(start, rot, (x, z), -27 - anchor, &rules, rng, &mut |x, z| (ground(b, x, z), b.top(x, z)));
            place_layout(b, rng, &layout, Placing { under: None, air: true, planks: None, clear_trees: false, archaeology: false, carve: true, integrity: 1.0, markers: no_markers, slopes: true });
            true
        }
        Kind::TrailRuins => {
            if placed(b.v, kind, sx, sz).is_none() {
                return true;
            }
            let Some(start) = pick_plan(&t.pool("trail_ruins/tower"), rng) else { return false };
            let rot = rng.below(4) as u8;
            // Enfouies : le bas de la tour est 15 blocs sous la surface
            let y = ground(b, x, z) - 15;
            let rules = Rules { depth: 7, reach: 80, follows_ground: &|_| false, fallback: &|_| None, empty_chance: &|_| 0.0, max_drop: None };
            let layout = t.assemble(start, rot, (x, z), y, &rules, rng, &mut |x, z| (ground(b, x, z), b.top(x, z)));
            place_layout(b, rng, &layout, Placing { under: None, air: false, planks: None, clear_trees: false, archaeology: true, carve: false, integrity: 1.0, markers: no_markers, slopes: true });
            true
        }
        Kind::Shipwreck => {
            let Some((x, _, z)) = placed(b.v, kind, sx, sz) else { return true };
            let beached = matches!(b.biome(x, z), B::Beach | B::SnowyBeach);
            // Échouée : jamais à l'envers
            let list: Vec<_> = t.pool("shipwreck").into_iter().filter(|p| !beached || !p.path.contains("upsidedown")).collect();
            let Some(ship) = pick_plan(&list, rng) else { return false };
            let rot = rng.below(4) as u8;
            let (sx_, _, sz_) = ship.size();
            let (w, d) = if rot & 1 == 1 { (sz_, sx_) } else { (sx_, sz_) };
            // Hauteur du fond sous la coque : moyenne au large, plus basse échouée (à demi
            // enfouie), comme le jeu
            let mut floors = Vec::new();
            for dx in [-w / 2, 0, w / 2 - 1] {
                for dz in [-d / 2, 0, d / 2 - 1] {
                    floors.push(b.top(x + dx, z + dz));
                }
            }
            let y = if beached {
                floors.iter().min().copied().unwrap_or(SEA_LEVEL) + 1 - ship.size().1 / 2 - rng.below(3) as i32
            } else {
                floors.iter().sum::<i32>() / floors.len() as i32 + 1
            };
            let layout = single(t, ship, rot, (x, z), y, rng, b);
            place_layout(b, rng, &layout, Placing { under: None, air: false, planks: None, clear_trees: false, archaeology: false, carve: false, integrity: 1.0, markers: shipwreck_markers, slopes: true });
            true
        }
        Kind::OceanRuin => {
            let Some((x, _, z)) = placed(b.v, kind, sx, sz) else { return true };
            let warm = matches!(b.biome(x, z), B::WarmOcean | B::LukewarmOcean | B::DeepLukewarmOcean);
            let all = t.pool("ruin");
            let name = |p: &str| p.rsplit('/').next().unwrap_or("").to_string();
            let of = |big: bool| -> Vec<_> {
                all.iter()
                    .filter(|p| {
                        let n = name(&p.path);
                        n.starts_with("big_") == big && n.contains("warm") == warm
                    })
                    .cloned()
                    .collect()
            };
            let big = rng.below(10) < 3;
            let Some(main) = pick_plan(&of(big), rng) else { return false };
            let mut layout = single(t, main, rng.below(4) as u8, (x, z), ruin_floor(b, x, z), rng, b);
            let integrity = if big { 0.9 } else { 0.8 };
            // Autour d'une grande ruine, souvent quelques petites (un hameau englouti)
            if big && rng.below(10) < 9 {
                let small = of(false);
                for _ in 0..(4 + rng.below(5)) {
                    let Some(r) = pick_plan(&small, rng) else { break };
                    let angle = rng.unit() * std::f64::consts::TAU;
                    let dist = 14.0 + rng.unit() * 14.0;
                    let (rx, rz) = (x + (angle.cos() * dist) as i32, z + (angle.sin() * dist) as i32);
                    let piece = single(t, r, rng.below(4) as u8, (rx, rz), ruin_floor(b, rx, rz), rng, b);
                    if piece.pieces.iter().all(|p| layout.pieces.iter().all(|q| !overlap(&p.bbox, &q.bbox))) {
                        layout.pieces.extend(piece.pieces);
                    }
                }
            }
            place_layout(b, rng, &layout, Placing { under: None, air: false, planks: None, clear_trees: false, archaeology: true, carve: false, integrity, markers: ruin_markers, slopes: true });
            true
        }
        Kind::Igloo => {
            let Some((x, y, z)) = placed(b.v, kind, sx, sz) else { return true };
            igloo(b, t, rng, x, y - 1, z)
        }
        Kind::RuinedPortal => {
            let Some((x, y, z)) = placed(b.v, kind, sx, sz) else { return true };
            let all = t.pool("ruined_portal");
            let giant = rng.below(20) == 0;
            let list: Vec<_> = all.iter().filter(|p| p.path.contains("giant") == giant).cloned().collect();
            let Some(portal) = pick_plan(&list, rng) else { return false };
            // Souvent un peu enfoui
            let y = y - rng.below(3) as i32;
            let layout = single(t, portal, rng.below(4) as u8, (x, z), y, rng, b);
            ruined_portal_ground(b, rng, &layout);
            place_layout(b, rng, &layout, Placing { under: None, air: false, planks: None, clear_trees: true, archaeology: false, carve: false, integrity: 1.0, markers: no_markers, slopes: true });
            weather_portal(b, rng, &layout);
            true
        }
        Kind::Mansion => {
            let Some((x, y, z)) = placed(b.v, kind, sx, sz) else { return true };
            mansion(b, t, rng, x, y, z)
        }
        Kind::Fossil => {
            let Some((x, y, z)) = placed(b.v, kind, sx, sz) else { return true };
            let Some(f) = pick_plan(&t.pool("fossils"), rng) else { return false };
            let layout = single(t, f, rng.below(4) as u8, (x, z), y - rng.below(6) as i32, rng, b);
            place_layout(b, rng, &layout, Placing { under: None, air: false, planks: None, clear_trees: false, archaeology: false, carve: false, integrity: 0.9, markers: no_markers, slopes: true });
            true
        }
        _ => false,
    }
}

/// Une pièce seule (sans jigsaw), centrée sur (x, z), le bas à `y`.
fn single(t: &Templates, piece: std::sync::Arc<super::templates::Template>, rot: u8, (x, z): (i32, i32), y: i32, rng: &mut Rng, b: &mut Builder) -> Layout {
    let rules = Rules { depth: 0, reach: 64, follows_ground: &|_| false, fallback: &|_| None, empty_chance: &|_| 1.0, max_drop: None };
    t.assemble(piece, rot, (x, z), y, &rules, rng, &mut |x, z| (ground(b, x, z), b.top(x, z)))
}

/// Fond de l'océan sous une ruine : le plus bas autour du centre (la ruine s'y enfonce).
fn ruin_floor(b: &mut Builder, x: i32, z: i32) -> i32 {
    [(0, 0), (-4, -4), (4, -4), (-4, 4), (4, 4)].iter().map(|(dx, dz)| b.top(x + dx, z + dz)).min().unwrap_or(SEA_LEVEL - 10)
}

/// Repères des épaves : le coffre est sous le repère.
fn shipwreck_markers(m: &str) -> Option<(i32, Loot, bool)> {
    match m {
        "supplyChest" => Some((-1, Loot::ShipwreckSupply, false)),
        "treasureChest" => Some((-1, Loot::ShipwreckTreasure, false)),
        "mapChest" => Some((-1, Loot::ShipwreckMap, false)),
        _ => None,
    }
}

/// Repères des ruines océaniques : un coffre à poser à leur place.
fn ruin_markers(m: &str) -> Option<(i32, Loot, bool)> {
    (m == "chest").then_some((0, Loot::OceanRuin, true))
}

/// Repère du laboratoire de l'igloo : le coffre est sous le repère.
fn igloo_markers(m: &str) -> Option<(i32, Loot, bool)> {
    (m == "chest").then_some((-1, Loot::Igloo, false))
}

/// Igloo : la hutte de neige, et une fois sur deux une trappe, une échelle de 4 à 11
/// tronçons et le laboratoire caché dessous (décalages du jeu).
fn igloo(b: &mut Builder, t: &Templates, rng: &mut Rng, x: i32, y: i32, z: i32) -> bool {
    let cellar = rng.below(2) == 0;
    let top = if cellar { "igloo/igloo_top_trapdoor" } else { "igloo/igloo_top_no_trapdoor" };
    let (Some(top), Some(ladder), Some(lab)) = (t.get(top), t.get("igloo/igloo_middle"), t.get("igloo/igloo_bottom")) else { return false };
    let rot = rng.below(4) as u8;
    // (plan, coin dans le repère de la hutte non tournée)
    let mut parts = vec![(top.clone(), (0, 0, 0))];
    if cellar {
        let depth = 4 + rng.below(8) as i32;
        for j in 0..depth - 1 {
            parts.push((ladder.clone(), (2, -3 - 3 * j, 4)));
        }
        parts.push((lab, (0, -3 - 3 * depth, -2)));
    }
    // Le tout tourne d'un bloc autour du coin de la hutte, placée au centre de l'emplacement
    let turn = |(px, pz): (i32, i32)| match rot & 3 {
        0 => (px, pz),
        1 => (-pz, px),
        2 => (-px, -pz),
        _ => (pz, -px),
    };
    let (ox, oz) = (x - 3, z - 4);
    let mut layout = Layout::default();
    for (tpl, (dx, dy, dz)) in parts {
        let (sx, sy, sz) = tpl.size();
        let corners = [(0, 0), (sx - 1, 0), (0, sz - 1), (sx - 1, sz - 1)].map(turn);
        let (mx, mz) = (corners.iter().map(|c| c.0).min().unwrap_or(0), corners.iter().map(|c| c.1).min().unwrap_or(0));
        let (rx, rz) = turn((dx, dz));
        let at = (ox + rx + mx, y + dy, oz + rz + mz);
        let (w, d) = if rot & 1 == 1 { (sz, sx) } else { (sx, sz) };
        let bbox = super::templates::Box3 { min: at, max: (at.0 + w - 1, at.1 + sy - 1, at.2 + d - 1) };
        layout.pieces.push(super::templates::Piece { template: tpl, rot, at, follows_ground: false, bbox });
    }
    place_layout(b, rng, &layout, Placing { under: Some("dirt"), air: true, planks: None, clear_trees: true, archaeology: false, carve: false, integrity: 1.0, markers: igloo_markers, slopes: false });
    true
}

/// Sol d'un portail en ruine : une plaque de netherrack et de magma autour, qui s'effiloche
/// vers l'extérieur.
fn ruined_portal_ground(b: &mut Builder, rng: &mut Rng, layout: &Layout) {
    let Some(p) = layout.pieces.first() else { return };
    let (netherrack, magma) = (b.id("netherrack"), b.id("magma"));
    let (lo, hi) = (p.bbox.min, p.bbox.max);
    let (cx, cz) = ((lo.0 + hi.0) as f64 / 2.0, (lo.2 + hi.2) as f64 / 2.0);
    let radius = ((hi.0 - lo.0).max(hi.2 - lo.2) as f64 / 2.0 + 3.0).max(5.0);
    for x in lo.0 - 4..=hi.0 + 4 {
        for z in lo.2 - 4..=hi.2 + 4 {
            let d = ((x as f64 - cx).powi(2) + (z as f64 - cz).powi(2)).sqrt() / radius;
            if d > 1.0 || rng.unit() < d * d {
                continue;
            }
            let g = b.top(x, z);
            b.put((x, g, z), if rng.below(12) == 0 { magma } else { netherrack });
        }
    }
}

/// Portail en ruine : une partie de l'obsidienne devient de l'obsidienne pleureuse.
fn weather_portal(b: &mut Builder, rng: &mut Rng, layout: &Layout) {
    let (obsidian, crying) = (b.id("obsidian"), b.id("crying_obsidian"));
    for p in &layout.pieces {
        for (rel, state) in p.template.blocks(p.rot) {
            if state == obsidian && rng.below(7) == 0 {
                b.put((p.at.0 + rel.0, p.at.1 + rel.1, p.at.2 + rel.2), crying);
            }
        }
    }
}

/// Hauteur du sol, l'eau comptant comme sol (une rue passe sur l'eau en pont de planches).
fn ground(b: &mut Builder, x: i32, z: i32) -> i32 {
    b.top(x, z).max(SEA_LEVEL)
}

fn overlap(a: &super::templates::Box3, c: &super::templates::Box3) -> bool {
    a.min.0 <= c.max.0 && a.max.0 >= c.min.0 && a.min.2 <= c.max.2 && a.max.2 >= c.min.2
}

/// Façon de poser les pièces d'un assemblage.
struct Placing {
    /// Remblai sous les pièces rigides, jusqu'au sol
    under: Option<&'static str>,
    /// Poser l'air des plans (qui creuse le terrain) ; sinon les pièces restent enfouies
    air: bool,
    /// Planches qui remplacent les chemins posés sur l'eau
    planks: Option<&'static str>,
    /// Abattre les arbres qui poussent dans la structure
    clear_trees: bool,
    /// Ruines de piste : une partie du gravier devient du gravier suspect
    archaeology: bool,
    /// Cité antique : le volume de chaque pièce est d'abord vidé (la grande caverne)
    carve: bool,
    /// Part des blocs posés (les ruines sont incomplètes) ; les coffres restent toujours
    integrity: f64,
    /// Repères des plans : (décalage vertical du coffre, butin, coffre à poser) ou rien
    markers: fn(&str) -> Option<(i32, Loot, bool)>,
    /// Raccorder le terrain en talus autour des pièces (avec `under`)
    slopes: bool,
}

fn no_markers(_: &str) -> Option<(i32, Loot, bool)> {
    None
}

/// Pose les pièces d'un assemblage dans le chantier.
fn place_layout(b: &mut Builder, rng: &mut Rng, layout: &Layout, how: Placing) {
    let air = b.p.air;
    let path = b.id("grass_path");
    let gravel = b.p.gravel;
    let suspicious = b.id("suspicious_gravel");
    let suspicious_sand = b.id("suspicious_sand");
    let (grass, dirt) = (b.p.grass, b.p.dirt);
    let sand = b.p.sand;
    let planks = how.planks.map(|n| b.id(n));
    // Cité antique : la caverne. Le volume de chaque pièce est vidé, et ses bords rongés
    // irrégulièrement sur quelques blocs, pour des parois de grotte plutôt que des murs droits
    let mut floors: Vec<(i32, i32, i32)> = Vec::new();
    if how.carve {
        let seed = b.v.seed();
        for piece in &layout.pieces {
            let (lo, hi) = (piece.bbox.min, piece.bbox.max);
            const M: i32 = 5;
            for x in lo.0 - M..=hi.0 + M {
                for z in lo.2 - M..=hi.2 + M {
                    let dx = (lo.0 - x).max(x - hi.0).max(0);
                    let dz = (lo.2 - z).max(z - hi.2).max(0);
                    let dh = ((dx * dx + dz * dz) as f64).sqrt();
                    let mut lowest = None;
                    for y in lo.1..=hi.1 + M {
                        let dv = (y - hi.1).max(0) as f64;
                        let d = (dh * dh + dv * dv).sqrt();
                        // Profondeur rongée : de 1 à 5 blocs selon un bruit lisse
                        if d == 0.0 || d < 1.0 + 4.0 * soft_noise(seed, x, y, z) {
                            b.put((x, y, z), air);
                            lowest.get_or_insert(y);
                        }
                    }
                    // Sol naturel de la caverne, autour des pièces
                    if dh > 0.0 && lowest == Some(lo.1) {
                        floors.push((x, lo.1 - 1, z));
                    }
                }
            }
        }
    }
    for piece in &layout.pieces {
        let (ax, ay, az) = piece.at;
        let mut columns: HashMap<(i32, i32), i32> = HashMap::new();
        // Coffres et repères : jamais retirés par l'usure
        let mut keep: std::collections::HashSet<(i32, i32, i32)> = piece.template.chests(piece.rot).map(|(p, _)| p).collect();
        for ((rx, ry, rz), m) in piece.template.markers(piece.rot) {
            if let Some((dy, loot, put_chest)) = (how.markers)(m) {
                let pos = (ax + rx, ay + ry + dy, az + rz);
                keep.insert((rx, ry + dy, rz));
                if put_chest {
                    b.chest(pos, D::N, loot);
                } else {
                    b.chests.push((pos, loot));
                }
            }
        }
        for ((rx, ry, rz), state) in piece.template.blocks(piece.rot) {
            let (x, z) = (ax + rx, az + rz);
            if state == air && !how.air {
                continue;
            }
            let y = if piece.follows_ground {
                let top = b.top(x, z);
                let g = top.max(SEA_LEVEL);
                // Chemin sur l'eau : un pont de planches
                let s = if top < SEA_LEVEL && (state == path || state == grass || state == dirt) && ry == 0 { planks.unwrap_or(state) } else { state };
                b.put((x, g + ry, z), s);
                columns.entry((x, z)).or_insert(g + 1);
                continue;
            } else {
                ay + ry
            };
            if how.integrity < 1.0 && state != air && !keep.contains(&(rx, ry, rz)) && rng.unit() > how.integrity {
                continue;
            }
            let s = if how.archaeology && state == gravel && rng.below(6) == 0 {
                suspicious
            } else if how.archaeology && state == sand && rng.below(6) == 0 {
                suspicious_sand
            } else {
                state
            };
            b.put((x, y, z), s);
            if ry == 0 && state != air {
                if let Some(under) = how.under {
                    // Remblai jusqu'au sol (jamais suspendu dans le vide)
                    b.foundation((x, y, z), under);
                }
            }
            columns.entry((x, z)).or_insert(ay);
        }
        // Pièce rigide sur une pente : le terrain est raccordé en talus (une pente de
        // 2 blocs de haut par bloc d'écart) plutôt que coupé à la verticale
        if let (Some(under), false, true) = (how.under, piece.follows_ground, how.slopes) {
            earthworks(b, piece, under);
        }
        for ((rx, ry, rz), color) in piece.template.beds(piece.rot) {
            if !piece.follows_ground {
                b.beds.push(((ax + rx, ay + ry, az + rz), color));
            }
        }
        for ((rx, ry, rz), loot) in piece.template.chests(piece.rot) {
            if !piece.follows_ground {
                b.chests.push(((ax + rx, ay + ry, az + rz), loot_of(loot)));
            }
        }
        if how.clear_trees {
            for ((x, z), from) in columns {
                b.clear.push((x, z, from));
            }
            // Pas d'arbre dont le pied est sur la pièce ou tout près (sa couronne la toucherait)
            const M: i32 = 3;
            b.no_trees.push((piece.bbox.min.0 - M, piece.bbox.min.2 - M, piece.bbox.max.0 + M, piece.bbox.max.2 + M));
        }
    }
    for &((x, y, z), state, follows) in &layout.finals {
        if state == air && !how.air {
            continue;
        }
        let (y, state) = if follows {
            // Bout de chemin d'une rue posée sur l'eau : en planches aussi
            let top = b.top(x, z);
            let s = if top < SEA_LEVEL && y == 0 && (state == path || state == grass || state == dirt) { planks.unwrap_or(state) } else { state };
            (top.max(SEA_LEVEL) + y, s)
        } else {
            (y, state)
        };
        b.put((x, y, z), state);
    }
    // Terre mise à nu : là où une pièce a creusé le terrain (l'air de son plan), la terre du
    // sous-sol restait apparente en plaques autour des maisons. Elle se couvre d'herbe.
    if how.under.map(|u| u != "sandstone").unwrap_or(false) && how.air {
        let dug: Vec<(i32, i32, i32)> = b.blocks.iter().filter(|(_, s)| **s == air).map(|(p, _)| *p).collect();
        for (x, y, z) in dug {
            if b.blocks.contains_key(&(x, y - 1, z)) {
                continue;
            }
            let top = b.top(x, z);
            if top >= SEA_LEVEL && y - 1 < top && y - 1 >= top - 3 {
                let cover = b.cover(x, z, grass);
                b.put((x, y - 1, z), cover);
            }
        }
        // … et la terre que les plans posent eux-mêmes à l'air libre (sous un point d'attache,
        // au pied d'un mur) : dans le jeu l'herbe la gagne vite ; ici elle ne pousse pas seule
        let bare: Vec<(i32, i32, i32)> = b.blocks.iter().filter(|(_, s)| **s == dirt).map(|(p, _)| *p).collect();
        for (x, y, z) in bare {
            let open = match b.blocks.get(&(x, y + 1, z)) {
                Some(s) => *s == air,
                None => y + 1 > b.top(x, z),
            };
            if open && y >= SEA_LEVEL {
                let cover = b.cover(x, z, grass);
                b.put((x, y, z), cover);
            }
        }
    }
    if how.carve {
        sculk_patches(b, rng, layout, floors);
    }
}

/// Talus autour d'une pièce rigide : le terrain plus haut qu'elle est entaillé en pente
/// douce, le terrain plus bas est remblayé en pente jusqu'à son plancher. Les colonnes déjà
/// occupées par la structure (rues, autres maisons) ne sont pas touchées.
fn earthworks(b: &mut Builder, piece: &super::templates::Piece, under: &str) {
    const M: i32 = 6;
    let (lo, hi) = (piece.bbox.min, piece.bbox.max);
    let floor = lo.1;
    let fill = b.id(under);
    let desert = under == "sandstone";
    let top_block = if desert { b.p.sand } else { b.p.grass };
    let air = b.p.air;
    for x in lo.0 - M..=hi.0 + M {
        for z in lo.2 - M..=hi.2 + M {
            let dx = (lo.0 - x).max(x - hi.0).max(0);
            let dz = (lo.2 - z).max(z - hi.2).max(0);
            let d = dx.max(dz);
            let g = b.top(x, z);
            if d == 0 {
                // Au-dessus de la pièce : dégagé
                for y in hi.1 + 1..=g.min(hi.1 + 12) {
                    b.air((x, y, z));
                }
                // Sous la pièce, là où son plan ne pose rien au niveau du sol et où le
                // terrain est plus bas : comblé de plain-pied (sinon un trou le long du mur)
                if g < floor && floor - g <= 8 && !b.blocks.contains_key(&(x, floor, z)) {
                    let cover = b.cover(x, z, top_block);
                    let fill = if cover == b.p.sand || cover == b.p.stone { cover } else { fill };
                    for y in g + 1..floor {
                        b.put((x, y, z), fill);
                    }
                    b.put((x, floor, z), cover);
                }
                continue;
            }
            let taken = |b: &Builder, y: i32| b.blocks.get(&(x, y, z)).map(|s| *s != air).unwrap_or(false);
            // Entaille : de plain-pied au ras de la pièce, puis un bloc de plus par bloc d'écart
            // (une pente douce ; un rebord dès le premier bloc laissait un fossé le long des murs)
            let cut = floor + d - 1;
            // (couvert comme le sol d'ici : du sable sur une plage, pas de l'herbe)
            let top_block = b.cover(x, z, top_block);
            let fill = if top_block == b.p.sand || top_block == b.p.stone { top_block } else { fill };
            if g > cut && !(cut..=g).any(|y| taken(b, y)) {
                for y in cut + 1..=g.min(cut + 14) {
                    b.air((x, y, z));
                }
                b.put((x, cut, z), top_block);
            }
            // Remblai : de plain-pied au ras de la pièce, puis un bloc de moins par bloc d'écart
            let bank = floor - (d - 1);
            // (jamais par-dessus une rue ou une autre pièce posée au niveau du sol)
            if g < bank && !(g..=bank).any(|y| taken(b, y)) {
                if bank - g > 8 {
                    continue; // trop haut : pas de talus, plutôt qu'un bloc de terre suspendu
                }
                for y in g + 1..bank {
                    b.put((x, y, z), fill);
                }
                b.put((x, bank, z), top_block);
            }
        }
    }
}

/// Bruit lisse (0 à 1), grille de 5 blocs : bords de caverne irréguliers mais continus.
fn soft_noise(seed: u64, x: i32, y: i32, z: i32) -> f64 {
    const S: i32 = 5;
    let (gx, gy, gz) = (x.div_euclid(S), y.div_euclid(S), z.div_euclid(S));
    let f = |v: i32| {
        let t = v.rem_euclid(S) as f64 / S as f64;
        t * t * (3.0 - 2.0 * t)
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    let at = |i: i32, j: i32, k: i32| (mix(seed, (gx + i) as i64, (gy + j) as i64 * 1_000_003 + (gz + k) as i64, 4_242) % 1000) as f64 / 1000.0;
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let plane = |j: i32| lerp(lerp(at(0, j, 0), at(1, j, 0), fx), lerp(at(0, j, 1), at(1, j, 1), fx), fz);
    lerp(plane(0), plane(1), fy)
}

/// Sculk de la cité antique : des plaques sur les sols (ceux des pièces et celui de la
/// caverne), avec des capteurs, des hurleurs qui appellent le Warden, et quelques
/// catalyseurs, comme les plaques de sculk que le jeu ajoute autour des cités.
fn sculk_patches(b: &mut Builder, rng: &mut Rng, layout: &Layout, mut floors: Vec<(i32, i32, i32)>) {
    let air = b.p.air;
    // Sols des pièces : bloc de roche avec de l'air au-dessus
    let rock: Vec<u32> = ["deepslate", "cobbled_deepslate", "polished_deepslate", "deepslate_bricks", "deepslate_tiles", "cracked_deepslate_bricks", "cracked_deepslate_tiles", "smooth_basalt", "chiseled_deepslate"]
        .iter()
        .filter_map(|n| b.p.named.get(*n).copied())
        .chain([b.p.deepslate])
        .collect();
    for (&(x, y, z), s) in &b.blocks {
        if rock.contains(s) && b.blocks.get(&(x, y + 1, z)) == Some(&air) {
            floors.push((x, y, z));
        }
    }
    if floors.is_empty() {
        return;
    }
    floors.sort_unstable();
    floors.dedup();
    let floor_set: std::collections::HashSet<(i32, i32, i32)> = floors.iter().copied().collect();
    let (sculk, sensor, shrieker, catalyst) = (b.id("sculk"), b.id("sculk_sensor"), b.id("sculk_shrieker[can_summon=1]"), b.id("sculk_catalyst"));
    let patches = 12 + layout.pieces.len() as u32 * 2;
    for n in 0..patches {
        let (cx, cy, cz) = floors[rng.below(floors.len() as u32) as usize];
        let radius = 2 + rng.below(5) as i32;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx * dx + dz * dz > radius * radius || rng.below(5) == 0 {
                    continue;
                }
                // Le sol voisin peut être un peu plus haut ou plus bas
                for dy in -2..=2 {
                    let at = (cx + dx, cy + dy, cz + dz);
                    if !floor_set.contains(&at) {
                        continue;
                    }
                    b.put(at, sculk);
                    let above = (at.0, at.1 + 1, at.2);
                    match rng.below(70) {
                        0..=2 => b.put(above, sensor),
                        3 => b.put(above, shrieker),
                        _ => {}
                    }
                    break;
                }
            }
        }
        // Un catalyseur au cœur d'une plaque sur quatre
        if n % 4 == 0 {
            b.put((cx, cy + 1, cz), catalyst);
        }
    }
}

/// Table de butin du jeu -> butin de Basalt.
fn loot_of(table: &str) -> Loot {
    if table.contains("pillager_outpost") {
        Loot::PillagerOutpost
    } else if table.contains("ancient_city") {
        Loot::AncientCity
    } else if table.contains("ruined_portal") {
        Loot::RuinedPortal
    } else {
        Loot::Village
    }
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
    /// Direction d'après son numéro (0 nord, 1 est, 2 sud, 3 ouest).
    fn from(n: u8) -> D {
        [D::N, D::E, D::S, D::W][(n & 3) as usize]
    }
    /// Tourne de quarts de tour dans le sens des aiguilles d'une montre.
    fn turn(self, quarter: u8) -> D {
        D::from(self as u8 + quarter)
    }
    /// Pas d'un bloc dans cette direction, en (x, z).
    fn step(self) -> (i32, i32) {
        match self {
            D::N => (0, -1),
            D::E => (1, 0),
            D::S => (0, 1),
            D::W => (-1, 0),
        }
    }
    /// Nom utilisé par minecraft:cardinal_direction.
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
    /// Position dans le monde d'un point du repère local.
    fn at(&self, lx: i32, ly: i32, lz: i32) -> (i32, i32, i32) {
        let (dx, dz) = match self.rot & 3 {
            0 => (lx, lz),
            1 => (-lz, lx),
            2 => (-lx, -lz),
            _ => (lz, -lx),
        };
        (self.x + dx, self.y + ly, self.z + dz)
    }
    /// Direction locale -> direction dans le monde.
    fn dir(&self, local: D) -> D {
        local.turn(self.rot)
    }
}

struct Builder<'a> {
    v: &'a Vanilla,
    p: &'a Palette,
    blocks: HashMap<(i32, i32, i32), u32>,
    chests: Vec<((i32, i32, i32), Loot)>,
    /// Colonnes où les arbres du terrain sont abattus, à partir d'une hauteur
    clear: Vec<(i32, i32, i32)>,
    /// Lits colorés
    beds: Vec<((i32, i32, i32), u8)>,
    /// Zones sans arbres
    no_trees: Vec<(i32, i32, i32, i32)>,
    tops: HashMap<(i32, i32), i32>,
    /// Barrières posées : reliées à leurs voisines à la fin
    fences: HashMap<(i32, i32, i32), &'static str>,
}

impl<'a> Builder<'a> {
    /// Nouveau chantier vide.
    fn new(v: &'a Vanilla, p: &'a Palette) -> Builder<'a> {
        Builder { v, p, blocks: HashMap::new(), chests: Vec::new(), clear: Vec::new(), beds: Vec::new(), no_trees: Vec::new(), tops: HashMap::new(), fences: HashMap::new() }
    }

    /// État d'un bloc d'après son nom ; erreur en test si le nom est absent de block_specs().
    fn id(&self, name: &str) -> u32 {
        match self.p.named.get(name) {
            Some(s) => *s,
            None => {
                debug_assert!(false, "bloc de structure non déclaré : {}", name);
                self.p.stone
            }
        }
    }

    /// Pose un état, si la hauteur est dans le monde.
    fn put(&mut self, (x, y, z): (i32, i32, i32), state: u32) {
        if (MIN_Y + 1..=MAX_Y).contains(&y) {
            self.blocks.insert((x, y, z), state);
        }
    }

    /// Pose un bloc par son nom (les barrières sont notées pour être reliées à la fin).
    fn set(&mut self, pos: (i32, i32, i32), name: &str) {
        let s = self.id(name);
        self.put(pos, s);
        match FENCES.iter().find(|f| **f == name) {
            Some(f) => {
                self.fences.insert(pos, *f);
            }
            None => {
                self.fences.remove(&pos);
            }
        }
    }

    /// Vide une case (l'air remplace le terrain).
    fn air(&mut self, pos: (i32, i32, i32)) {
        let a = self.p.air;
        self.put(pos, a);
        self.fences.remove(&pos);
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

    /// Bloc qui couvre naturellement le sol de cette colonne : sable des plages et des
    /// déserts, pierre des côtes rocheuses, `default` (l'herbe) ailleurs.
    fn cover(&self, x: i32, z: i32, default: u32) -> u32 {
        match self.biome(x, z) {
            B::Desert | B::Beach | B::SnowyBeach | B::River | B::FrozenRiver => self.p.sand,
            B::StonyShore | B::StonyPeaks => self.p.stone,
            b if b.ocean() => self.p.sand,
            _ => default,
        }
    }

    /// Biome de surface (rapide : sans calculer la hauteur du sol).
    fn biome(&self, x: i32, z: i32) -> B {
        self.v.surface_biome_at(x, z)
    }

    /// Coffre tourné vers `front`, rempli à la première ouverture.
    fn chest(&mut self, pos: (i32, i32, i32), front: D, loot: Loot) {
        self.set(pos, &format!("chest[minecraft:cardinal_direction={}]", front.name()));
        self.chests.push((pos, loot));
    }

    /// Termine le chantier : relie les barrières puis range les blocs par chunk.
    fn finish(mut self) -> Plan {
        // Chaque barrière se relie aux barrières et aux blocs pleins voisins
        let fences: Vec<((i32, i32, i32), &'static str)> = self.fences.iter().map(|(p, f)| (*p, *f)).collect();
        let air = self.p.air;
        for ((x, y, z), fence) in fences {
            let mut bits = 0u8;
            for (bit, (dx, dz)) in [(1u8, (1, 0)), (2, (0, -1)), (4, (0, 1)), (8, (-1, 0))] {
                let n = (x + dx, y, z + dz);
                let solid = self.blocks.get(&n).map(|s| *s != air && !self.p.aquatic.contains(s)).unwrap_or(false);
                if self.fences.contains_key(&n) || solid {
                    bits |= bit;
                }
            }
            let s = self.id(&fence_spec(fence, bits));
            self.put((x, y, z), s);
        }
        let mut plan = Plan::default();
        for ((x, y, z), s) in self.blocks {
            plan.blocks.entry((x >> 4, z >> 4)).or_default().push(((x & 15) as u8, y as i16, (z & 15) as u8, s));
        }
        plan.no_trees = self.no_trees;
        for ((x, y, z), color) in self.beds {
            plan.beds.entry((x >> 4, z >> 4)).or_default().push(((x & 15) as u8, y as i16, (z & 15) as u8, color));
        }
        for (x, z, y) in self.clear {
            plan.clear.entry((x >> 4, z >> 4)).or_default().push(((x & 15) as u8, (z & 15) as u8, y as i16));
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
const FENCES: [&str; 4] = ["oak_fence", "spruce_fence", "acacia_fence", "dark_oak_fence"];

/// Barrière reliée à ses voisines : bits 1 est, 2 nord, 4 sud, 8 ouest.
fn fence_spec(fence: &str, bits: u8) -> String {
    format!(
        "{}[minecraft:connection_east={},minecraft:connection_north={},minecraft:connection_south={},minecraft:connection_west={}]",
        fence,
        bits & 1,
        (bits >> 1) & 1,
        (bits >> 2) & 1,
        (bits >> 3) & 1
    )
}
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
    // Portails en ruine (plans officiels)
    out.extend(["obsidian", "crying_obsidian", "netherrack", "magma"].iter().map(|s| s.to_string()));
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
    for d in [D::N, D::E, D::S, D::W] {
        out.push(format!("vault[minecraft:cardinal_direction={}]", d.name()));
        out.push(format!("deepslate_brick_stairs[weirdo_direction={},upside_down_bit=0]", d.stairs()));
    }
    for d in [D::N, D::E, D::S, D::W] {
        for block in ["blast_furnace", "smoker", "lectern", "stonecutter_block", "anvil"] {
            out.push(format!("{}[minecraft:cardinal_direction={}]", block, d.name()));
        }
        out.push(format!("loom[direction={}]", legacy(d)));
        out.push(format!("grindstone[attachment=standing,direction={}]", legacy(d)));
        out.push(format!("bell[attachment=standing,direction={}]", legacy(d)));
        for head in 0..2 {
            out.push(format!("bed[direction={},head_piece_bit={}]", legacy(d), head));
        }
    }
    for log in ["stripped_oak_log", "stripped_acacia_log", "stripped_spruce_log"] {
        for axis in ["x", "z"] {
            out.push(format!("{}[pillar_axis={}]", log, axis));
        }
    }
    for count in 0..4 {
        out.push(format!("candle[candles={},lit=0]", count));
    }
    // Barrières : chaque combinaison de liaisons avec les voisins
    for fence in FENCES {
        for bits in 0..16u8 {
            out.push(fence_spec(fence, bits));
        }
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
            // Structures récentes
            "dark_oak_log",
            "dark_oak_planks",
            "birch_planks",
            "dark_oak_log[pillar_axis=x]",
            "dark_oak_slab[minecraft:vertical_half=bottom]",
            "carved_pumpkin[minecraft:cardinal_direction=south]",
            "mud_bricks",
            "packed_mud",
            "hardened_clay",
            "brown_terracotta",
            "red_terracotta",
            "yellow_terracotta",
            "light_gray_terracotta",
            "light_blue_terracotta",
            "cyan_terracotta",
            "purple_terracotta",
            "suspicious_gravel",
            "tuff_bricks",
            "chiseled_tuff",
            "chiseled_tuff_bricks",
            "polished_tuff",
            "waxed_cut_copper",
            "waxed_copper",
            "waxed_copper_grate",
            "waxed_copper_bulb[lit=1]",
            "chiseled_copper",
            "trial_spawner",
            "decorated_pot",
            "reinforced_deepslate",
            "deepslate_bricks",
            "cracked_deepslate_bricks",
            "deepslate_tiles",
            "cracked_deepslate_tiles",
            "polished_deepslate",
            "chiseled_deepslate",
            "cobbled_deepslate",
            "deepslate_brick_wall[wall_post_bit=1]",
            "deepslate_tile_slab[minecraft:vertical_half=bottom]",
            "soul_lantern[hanging=0]",
            "soul_lantern[hanging=1]",
            "sculk",
            "sculk_sensor",
            "sculk_shrieker[can_summon=1]",
            "sculk_catalyst",
            "gray_wool",
            "light_blue_wool",
            // Villages actuels
            "grass_block",
            "poppy",
            "dandelion",
            "stripped_oak_log",
            "stripped_acacia_log",
            "stripped_spruce_log",
            "snow",
            "packed_ice",
            "blue_ice",
            "lime_terracotta",
            "light_gray_wool",
            "cactus",
            "brewing_stand",
            "red_carpet",
            "yellow_carpet",
            "pumpkin[minecraft:cardinal_direction=south]",
            "sweet_berry_bush[growth=3]",
            "spruce_slab[minecraft:vertical_half=bottom]",
            "acacia_slab[minecraft:vertical_half=bottom]",
            "suspicious_sand",
            "sand",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    out
}

// --- Villages ---------------------------------------------------------------------------
//
// Villages du jeu actuel (depuis la 1.14) : une place centrale avec sa cloche, des rues qui
// suivent le relief, des maisons aux matériaux du biome (plaines, désert, savane, taïga,
// neige), des maisons de métier avec leur bloc (forgeron, bibliothécaire, cartographe…),
// des fermes, des enclos et des lampadaires.

#[derive(Clone, Copy, PartialEq, Debug)]
enum Theme {
    Plains,
    Desert,
    Savanna,
    Taiga,
    Snowy,
}

/// Matériaux d'un village, selon le biome.
struct Style {
    theme: Theme,
    planks: &'static str,
    log: &'static str,
    stripped: &'static str,
    /// Préfixe des escaliers et des dalles du toit
    wood: &'static str,
    door: &'static str,
    fence: &'static str,
    /// Soubassement des murs et socle
    base: &'static str,
    /// Murs (entre les poteaux)
    wall: &'static str,
    floor: &'static str,
    path: &'static str,
    /// Couleur d'accent (laine des lits, tapis, terre cuite)
    accent: &'static str,
    flat_roof: bool,
    snow: bool,
}

impl Style {
    /// Matériaux du village selon le biome ; None si le biome n'a pas de village.
    fn for_biome(b: B) -> Option<Style> {
        let plains = Style {
            theme: Theme::Plains,
            planks: "oak_planks",
            log: "oak_log",
            stripped: "stripped_oak_log",
            wood: "oak",
            door: "wooden_door",
            fence: "oak_fence",
            base: "cobblestone",
            wall: "oak_planks",
            floor: "oak_planks",
            path: "grass_path",
            accent: "yellow_wool",
            flat_roof: false,
            snow: false,
        };
        Some(match b {
            B::Plains | B::SunflowerPlains | B::Meadow => plains,
            B::Savanna | B::SavannaPlateau => Style {
                theme: Theme::Savanna,
                planks: "acacia_planks",
                log: "acacia_log",
                stripped: "stripped_acacia_log",
                wood: "acacia",
                door: "acacia_door",
                fence: "acacia_fence",
                base: "orange_terracotta",
                wall: "acacia_planks",
                floor: "orange_terracotta",
                accent: "orange_terracotta",
                ..plains
            },
            B::Taiga => Style {
                theme: Theme::Taiga,
                planks: "spruce_planks",
                log: "spruce_log",
                stripped: "stripped_spruce_log",
                wood: "spruce",
                door: "spruce_door",
                fence: "spruce_fence",
                base: "mossy_cobblestone",
                wall: "spruce_planks",
                floor: "spruce_planks",
                accent: "light_gray_wool",
                ..plains
            },
            B::SnowyPlains => Style {
                theme: Theme::Snowy,
                planks: "spruce_planks",
                log: "stripped_spruce_log",
                stripped: "stripped_spruce_log",
                wood: "spruce",
                door: "spruce_door",
                fence: "spruce_fence",
                base: "stone_bricks",
                wall: "snow",
                floor: "spruce_planks",
                accent: "white_wool",
                snow: true,
                ..plains
            },
            B::Desert => Style {
                theme: Theme::Desert,
                planks: "cut_sandstone",
                log: "smooth_sandstone",
                stripped: "smooth_sandstone",
                wood: "sandstone",
                door: "wooden_door",
                fence: "oak_fence",
                base: "sandstone",
                wall: "cut_sandstone",
                floor: "smooth_sandstone",
                path: "smooth_sandstone",
                accent: "lime_terracotta",
                flat_roof: true,
                snow: false,
            },
            _ => return None,
        })
    }
}

/// Métiers : le bloc qui fait d'une maison celle d'un artisan.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Job {
    Armorer,
    Butcher,
    Cartographer,
    Cleric,
    Fisherman,
    Fletcher,
    Leatherworker,
    Librarian,
    Mason,
    Shepherd,
    Toolsmith,
    Weaponsmith,
}

const JOBS: [Job; 12] = [
    Job::Armorer,
    Job::Butcher,
    Job::Cartographer,
    Job::Cleric,
    Job::Fisherman,
    Job::Fletcher,
    Job::Leatherworker,
    Job::Librarian,
    Job::Mason,
    Job::Shepherd,
    Job::Toolsmith,
    Job::Weaponsmith,
];

impl Job {
    /// Bloc de métier, tourné vers `front` quand il a une orientation.
    fn block(self, front: D) -> String {
        match self {
            Job::Armorer => format!("blast_furnace[minecraft:cardinal_direction={}]", front.name()),
            Job::Butcher => format!("smoker[minecraft:cardinal_direction={}]", front.name()),
            Job::Cartographer => "cartography_table".into(),
            Job::Cleric => "brewing_stand".into(),
            Job::Fisherman => "barrel[facing_direction=1]".into(),
            Job::Fletcher => "fletching_table".into(),
            Job::Leatherworker => "cauldron[cauldron_liquid=water,fill_level=6]".into(),
            Job::Librarian => format!("lectern[minecraft:cardinal_direction={}]", front.name()),
            Job::Mason => format!("stonecutter_block[minecraft:cardinal_direction={}]", front.name()),
            Job::Shepherd => format!("loom[direction={}]", legacy(front)),
            Job::Toolsmith => "smithing_table".into(),
            Job::Weaponsmith => format!("grindstone[attachment=standing,direction={}]", legacy(front)),
        }
    }
}

/// Orientation « direction » des anciens blocs (lit, métier à tisser, cloche…) : 0 sud,
/// 1 ouest, 2 nord, 3 est.
fn legacy(d: D) -> u8 {
    match d {
        D::S => 0,
        D::W => 1,
        D::N => 2,
        D::E => 3,
    }
}

/// Sorte de bâtiment, avec sa taille (largeur, profondeur).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Building {
    SmallHouse,
    MediumHouse,
    BigHouse,
    Workshop(Job),
    Library,
    Farm,
    BigFarm,
    Pen,
    Lamp,
    Decoration,
}

impl Building {
    /// Emprise du bâtiment, façade comprise.
    fn size(self) -> (i32, i32) {
        match self {
            Building::SmallHouse => (5, 5),
            Building::MediumHouse => (7, 6),
            Building::BigHouse => (9, 7),
            Building::Workshop(_) => (7, 7),
            Building::Library => (9, 9),
            Building::Farm => (7, 9),
            Building::BigFarm => (13, 9),
            Building::Pen => (7, 7),
            Building::Lamp => (1, 1),
            Building::Decoration => (3, 3),
        }
    }
}

/// Rayon maximal d'un village autour de sa place.
const VILLAGE_RADIUS: i32 = 72;

/// Village complet : place, rues, bâtiments le long des rues.
fn village(b: &mut Builder, rng: &mut Rng, cx: i32, cz: i32, style: &Style) {
    let y0 = b.top(cx, cz);
    if y0 <= SEA_LEVEL || y0 > SEA_LEVEL + 60 {
        return;
    }
    let mut taken: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    // Place centrale (7×7) avec sa cloche
    meeting_point(b, rng, Frame { x: cx - 3, y: y0, z: cz - 3, rot: 0 }, style);
    for x in cx - 4..=cx + 4 {
        for z in cz - 4..=cz + 4 {
            taken.insert((x, z));
        }
    }
    // Métiers : chacun une fois avant qu'un deuxième n'apparaisse
    let mut jobs: Vec<Job> = JOBS.to_vec();
    for i in (1..jobs.len()).rev() {
        let j = rng.below(i as u32 + 1) as usize;
        jobs.swap(i, j);
    }
    // Rues : trois à cinq depuis la place, puis des rues secondaires
    let mut streets: Vec<(i32, i32, D, i32, u8)> = Vec::new();
    let mut sides = [D::N, D::E, D::S, D::W];
    let first = rng.below(4) as usize;
    sides.rotate_left(first);
    for (k, d) in sides.iter().enumerate() {
        if k == 3 && rng.below(3) == 0 {
            continue;
        }
        let (dx, dz) = d.step();
        streets.push((cx + dx * 4, cz + dz * 4, *d, 18 + rng.below(22) as i32, 0));
    }
    let mut buildings = 0;
    let mut job_index = 0;
    while let Some((sx, sz, dir, len, depth)) = streets.pop() {
        let (dx, dz) = dir.step();
        let (px, pz) = dir.turn(1).step(); // perpendiculaire
        let mut next_plot = 3 + rng.below(3) as i32;
        for i in 0..len {
            let (x, z) = (sx + dx * i, sz + dz * i);
            if (x - cx).abs() > VILLAGE_RADIUS || (z - cz).abs() > VILLAGE_RADIUS {
                break;
            }
            // Pas de rue dans l'eau, ni trop haut ou trop bas par rapport à la place
            let ground = b.top(x, z);
            if ground < SEA_LEVEL || (ground - y0).abs() > 14 {
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
                    // La rue suit le relief : chemin sur le sol, rien au-dessus
                    let path = if style.theme == Theme::Taiga && rng.below(6) == 0 { "coarse_dirt" } else { style.path };
                    b.set((rx, t, rz), path);
                    b.air((rx, t + 1, rz));
                    b.air((rx, t + 2, rz));
                }
            }
            if i == next_plot {
                next_plot = i + 6 + rng.below(4) as i32;
                for side in [1i32, -1] {
                    if buildings >= 28 {
                        break;
                    }
                    let mut kind = pick_building(rng);
                    if let Building::Workshop(_) = kind {
                        kind = Building::Workshop(jobs[job_index % jobs.len()]);
                    }
                    // Façade tournée vers la rue
                    let towards = if side == 1 { dir.turn(3) } else { dir.turn(1) };
                    if try_building(b, &mut taken, kind, style, rng, (x + px * side * 2, z + pz * side * 2), towards, y0) {
                        buildings += 1;
                        if let Building::Workshop(_) = kind {
                            job_index += 1;
                        }
                    }
                }
                // Rue secondaire
                if depth < 2 && rng.below(3) == 0 {
                    let side = if rng.below(2) == 0 { dir.turn(1) } else { dir.turn(3) };
                    let (ox, oz) = side.step();
                    streets.push((x + ox * 2, z + oz * 2, side, 10 + rng.below(16) as i32, depth + 1));
                }
            } else if i % 10 == 5 && rng.below(2) == 0 {
                let side = if rng.below(2) == 0 { 1 } else { -1 };
                let towards = if side == 1 { dir.turn(3) } else { dir.turn(1) };
                let kind = if rng.below(3) == 0 { Building::Decoration } else { Building::Lamp };
                try_building(b, &mut taken, kind, style, rng, (x + px * side * 2, z + pz * side * 2), towards, y0);
            }
        }
    }
}

/// Tirage de la sorte de bâtiment (le métier est choisi ensuite).
fn pick_building(rng: &mut Rng) -> Building {
    match rng.below(100) {
        0..=19 => Building::SmallHouse,
        20..=31 => Building::MediumHouse,
        32..=39 => Building::BigHouse,
        40..=66 => Building::Workshop(Job::Armorer),
        67..=71 => Building::Library,
        72..=82 => Building::Farm,
        83..=88 => Building::BigFarm,
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
    // Emprise (avec une case de marge, sauf côté façade qui donne sur la rue) libre
    let mut cells = Vec::new();
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
    if lo < SEA_LEVEL || hi - lo > 5 || (b.top(door.0, door.1) - y0).abs() > 12 {
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
        Building::SmallHouse => house(b, f, style, rng, 5, 5, None),
        Building::MediumHouse => house(b, f, style, rng, 7, 6, None),
        Building::BigHouse => house(b, f, style, rng, 9, 7, None),
        Building::Workshop(job) => house(b, f, style, rng, 7, 7, Some(job)),
        Building::Library => library(b, f, style, rng),
        Building::Farm => farm(b, f, style, rng, 7),
        Building::BigFarm => farm(b, f, style, rng, 13),
        Building::Pen => pen(b, f, style, rng),
        Building::Lamp => lamp(b, f, style),
        Building::Decoration => decoration(b, f, style, rng),
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

/// Cloche posée au sol, tournée vers la façade.
fn bell(f: &Frame) -> String {
    format!("bell[attachment=standing,direction={}]", legacy(f.dir(D::S)))
}

/// Place centrale (7×7) : la cloche, et un décor propre au biome.
fn meeting_point(b: &mut Builder, rng: &mut Rng, f: Frame, s: &Style) {
    prepare(b, f, 7, 7, 7, s.base);
    for lx in 0..7 {
        for lz in 0..7 {
            b.set(f.at(lx, 0, lz), s.path);
        }
    }
    match s.theme {
        // Plaines : la cloche sur un poteau, entourée de massifs de fleurs
        Theme::Plains => {
            b.set(f.at(3, 0, 3), "cobblestone");
            b.set(f.at(3, 1, 3), "oak_log");
            b.set(f.at(3, 2, 3), &bell(&f));
            for (lx, lz) in [(1, 1), (5, 1), (1, 5), (5, 5)] {
                b.set(f.at(lx, 0, lz), "grass_block");
                b.set(f.at(lx, 1, lz), if rng.below(2) == 0 { "poppy" } else { "dandelion" });
            }
            for (lx, lz) in [(0, 3), (6, 3)] {
                b.set(f.at(lx, 1, lz), "oak_fence");
                b.set(f.at(lx, 2, lz), "lantern[hanging=0]");
            }
        }
        // Désert : un puits de grès, la cloche à côté
        Theme::Desert => {
            for lx in 1..=5 {
                for lz in 1..=5 {
                    let edge = lx == 1 || lx == 5 || lz == 1 || lz == 5;
                    b.set(f.at(lx, 0, lz), if edge { "cut_sandstone" } else { "water" });
                    if !edge {
                        b.set(f.at(lx, -1, lz), "water");
                        b.set(f.at(lx, -2, lz), "sandstone");
                    } else {
                        b.set(f.at(lx, 1, lz), "sandstone_slab[minecraft:vertical_half=bottom]");
                    }
                }
            }
            b.set(f.at(0, 1, 3), "cut_sandstone");
            b.set(f.at(0, 2, 3), &bell(&f));
            b.set(f.at(6, 1, 3), "cut_sandstone");
            b.set(f.at(6, 2, 3), "torch[torch_facing_direction=top]");
        }
        // Savane : estrade de terre cuite et d'acacia sous la cloche
        Theme::Savanna => {
            for lx in 1..=5 {
                for lz in 1..=5 {
                    b.set(f.at(lx, 0, lz), if (lx + lz) % 2 == 0 { "orange_terracotta" } else { "yellow_terracotta" });
                }
            }
            for (lx, lz) in [(1, 1), (5, 1), (1, 5), (5, 5)] {
                b.fill(f.at(lx, 1, lz), f.at(lx, 3, lz), "acacia_fence");
            }
            for lx in 1..=5 {
                for lz in 1..=5 {
                    b.set(f.at(lx, 4, lz), "acacia_slab[minecraft:vertical_half=bottom]");
                }
            }
            b.set(f.at(3, 1, 3), "acacia_log");
            b.set(f.at(3, 2, 3), &bell(&f));
        }
        // Taïga : feu de camp, bûches pour s'asseoir, cloche sous un abri
        Theme::Taiga => {
            b.set(f.at(3, 1, 3), "campfire[extinguished=0]");
            for (lx, lz, axis) in [(3, 1, "x"), (3, 5, "x"), (1, 3, "z"), (5, 3, "z")] {
                b.set(f.at(lx, 1, lz), &format!("spruce_log[pillar_axis={}]", if (f.rot % 2 == 0) == (axis == "x") { "x" } else { "z" }));
            }
            b.fill(f.at(0, 1, 0), f.at(0, 3, 0), "spruce_log");
            b.set(f.at(0, 4, 0), "spruce_planks");
            b.set(f.at(0, 5, 0), &bell(&f));
            b.set(f.at(6, 1, 6), "pumpkin[minecraft:cardinal_direction=south]");
            b.set(f.at(6, 1, 0), "sweet_berry_bush[growth=3]");
        }
        // Neige : dalle de glace tassée, cloche sous un toit d'épicéa, lanternes
        Theme::Snowy => {
            for lx in 1..=5 {
                for lz in 1..=5 {
                    b.set(f.at(lx, 0, lz), if lx == 3 || lz == 3 { "blue_ice" } else { "packed_ice" });
                }
            }
            for (lx, lz) in [(2, 2), (4, 2), (2, 4), (4, 4)] {
                b.fill(f.at(lx, 1, lz), f.at(lx, 3, lz), "stripped_spruce_log");
            }
            for lx in 2..=4 {
                for lz in 2..=4 {
                    b.set(f.at(lx, 4, lz), "spruce_slab[minecraft:vertical_half=bottom]");
                    b.set(f.at(lx, 5, lz), "snow_layer[height=0]");
                }
            }
            b.set(f.at(3, 3, 3), "lantern[hanging=1]");
            b.set(f.at(3, 1, 3), "stone_bricks");
            b.set(f.at(3, 2, 3), &bell(&f));
        }
    }
}

/// Maison : socle, poteaux d'angle, murs du biome, fenêtres, porte, lit, toit. Avec un
/// métier, son bloc est posé près de la porte (et la maison sert d'atelier).
fn house(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng, w: i32, d: i32, job: Option<Job>) {
    let wall_top = if w >= 9 { 4 } else { 3 };
    prepare(b, f, w, d, wall_top + (w + 1) / 2 + 3, s.base);
    for lx in 0..w {
        for lz in 0..d {
            let corner = (lx == 0 || lx == w - 1) && (lz == 0 || lz == d - 1);
            let edge = lx == 0 || lx == w - 1 || lz == 0 || lz == d - 1;
            // Plancher (bordure du socle, intérieur dans la matière du biome)
            b.set(f.at(lx, 0, lz), if edge { s.base } else if s.theme == Theme::Desert && (lx + lz) % 4 == 0 { s.accent } else { s.floor });
            for ly in 1..=wall_top {
                if corner {
                    b.set(f.at(lx, ly, lz), s.log);
                } else if edge {
                    let window = ly == 2 && ((lx == 0 || lx == w - 1) && lz % 2 == 0 && lz > 0 && lz < d - 1 || (lz == 0 && lx % 2 == 1));
                    if window {
                        let along_x = lz == 0 || lz == d - 1;
                        let pane = match (f.rot % 2 == 0) == along_x {
                            true => "glass_pane[minecraft:connection_east=1,minecraft:connection_west=1]",
                            false => "glass_pane[minecraft:connection_north=1,minecraft:connection_south=1]",
                        };
                        b.set(f.at(lx, ly, lz), pane);
                    } else if ly == 1 && matches!(s.theme, Theme::Plains | Theme::Taiga | Theme::Snowy) {
                        b.set(f.at(lx, ly, lz), s.base);
                    } else if s.theme == Theme::Savanna && ly == wall_top {
                        b.set(f.at(lx, ly, lz), s.stripped);
                    } else if s.theme == Theme::Desert && ly == wall_top {
                        b.set(f.at(lx, ly, lz), s.accent);
                    } else {
                        b.set(f.at(lx, ly, lz), s.wall);
                    }
                }
            }
        }
    }
    // Porte au milieu de la façade (au désert, une simple ouverture une fois sur deux)
    let out = f.dir(D::S);
    if s.theme == Theme::Desert && rng.below(2) == 0 {
        b.air(f.at(w / 2, 1, d - 1));
        b.air(f.at(w / 2, 2, d - 1));
    } else {
        b.set(f.at(w / 2, 1, d - 1), &format!("{}[minecraft:cardinal_direction={},upper_block_bit=0]", s.door, out.door()));
        b.set(f.at(w / 2, 2, d - 1), &format!("{}[minecraft:cardinal_direction={},upper_block_bit=1]", s.door, out.door()));
    }
    let step = f.at(w / 2, 0, d);
    if b.top(step.0, step.2) < step.1 {
        b.set(step, s.base);
    }
    // Lit contre le mur du fond (tête au nord local), et un second dans les grandes maisons
    let head = f.dir(D::N);
    let beds: &[i32] = if w >= 9 { &[1, w - 2] } else if w >= 7 && job.is_none() { &[1, 2] } else { &[1] };
    for lx in beds {
        b.set(f.at(*lx, 1, 1), &format!("bed[direction={},head_piece_bit=1]", legacy(head)));
        b.set(f.at(*lx, 1, 2), &format!("bed[direction={},head_piece_bit=0]", legacy(head)));
    }
    // Mobilier
    let inside = f.dir(D::N);
    match job {
        Some(job) => {
            b.set(f.at(w - 2, 1, d - 2), &job.block(out));
            match job {
                Job::Armorer | Job::Toolsmith | Job::Weaponsmith => {
                    b.set(f.at(w - 2, 1, 1), &format!("anvil[minecraft:cardinal_direction={}]", f.dir(D::E).name()));
                    b.chest(f.at(w - 2, 1, 2), f.dir(D::W), Loot::Village);
                }
                Job::Cartographer => b.set(f.at(w - 2, 1, 1), "bookshelf"),
                Job::Fletcher | Job::Fisherman | Job::Butcher | Job::Leatherworker => {
                    b.set(f.at(w - 2, 1, 1), "barrel[facing_direction=1]");
                    b.set(f.at(w - 3, 1, 1), "hay_block[pillar_axis=y]");
                }
                Job::Shepherd => {
                    b.set(f.at(w - 2, 1, 1), "white_wool");
                    b.set(f.at(w - 3, 1, 1), "light_gray_wool");
                }
                Job::Librarian | Job::Cleric | Job::Mason => b.set(f.at(w - 2, 1, 1), "bookshelf"),
            }
        }
        None => {
            b.set(f.at(w - 2, 1, 1), "crafting_table");
            if w >= 7 {
                b.set(f.at(w - 2, 1, 2), &format!("furnace[minecraft:cardinal_direction={}]", f.dir(D::W).name()));
            }
            if rng.below(3) == 0 || w >= 9 {
                b.chest(f.at(w - 2, 1, d - 2), inside, Loot::Village);
            }
        }
    }
    // Tapis au milieu, lanterne au plafond, pot de fleurs
    if w >= 7 {
        let carpet = match s.theme {
            Theme::Desert => "red_carpet",
            Theme::Savanna => "yellow_carpet",
            _ => "white_carpet",
        };
        b.set(f.at(w / 2, 1, d / 2), carpet);
    }
    b.set(f.at(w / 2, wall_top, d / 2 - 1), "lantern[hanging=1]");
    b.set(f.at(1, 1, d - 2), "flower_pot");
    roof(b, f, s, w, d, wall_top + 1);
}

/// Bibliothèque : grande salle tapissée d'étagères, pupitre au centre.
fn library(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng) {
    house(b, f, s, rng, 9, 9, Some(Job::Librarian));
    for lx in 1..=7 {
        if lx != 4 {
            b.set(f.at(lx, 1, 1), "bookshelf");
            b.set(f.at(lx, 2, 1), "bookshelf");
        }
    }
    for lz in 3..=5 {
        b.set(f.at(1, 1, lz), "bookshelf");
        b.set(f.at(7, 1, lz), "bookshelf");
    }
    b.set(f.at(4, 1, 4), &format!("lectern[minecraft:cardinal_direction={}]", f.dir(D::S).name()));
    b.set(f.at(4, 1, 1), &format!("bed[direction={},head_piece_bit=1]", legacy(f.dir(D::N))));
    b.set(f.at(4, 1, 2), &format!("bed[direction={},head_piece_bit=0]", legacy(f.dir(D::N))));
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
                b.set(f.at(xl, ly, lz), &format!("{}_slab[minecraft:vertical_half=bottom]", s.wood));
            } else {
                b.set(f.at(xl, ly, lz), &format!("{}_stairs[weirdo_direction={},upside_down_bit=0]", s.wood, left.stairs()));
                b.set(f.at(xr, ly, lz), &format!("{}_stairs[weirdo_direction={},upside_down_bit=0]", s.wood, right.stairs()));
            }
            if s.snow {
                b.set(f.at(xl, ly + 1, lz), "snow_layer[height=0]");
                b.set(f.at(xr, ly + 1, lz), "snow_layer[height=0]");
            }
        }
        // Pignons, avec une poutre au faîte
        for lx in (xl + 1)..xr {
            let gable = if s.theme == Theme::Snowy { "spruce_planks" } else { s.planks };
            b.set(f.at(lx, ly, 0), gable);
            b.set(f.at(lx, ly, d - 1), gable);
        }
    }
}

/// Champ : rangs de cultures arrosés par des rigoles, bordure de bûches, composteur
/// (le bloc du fermier).
fn farm(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng, w: i32) {
    let d = 9;
    prepare(b, f, w, d, 3, "dirt");
    let crops = match s.theme {
        Theme::Taiga | Theme::Snowy => ["potatoes[growth=7]", "carrots[growth=7]", "wheat[growth=7]"],
        Theme::Desert => ["wheat[growth=7]", "potatoes[growth=7]", "beetroot[growth=7]"],
        _ => ["wheat[growth=7]", "carrots[growth=7]", "beetroot[growth=7]"],
    };
    let border = if s.theme == Theme::Desert { "sandstone".to_string() } else { format!("{}[pillar_axis={}]", s.log, if f.rot % 2 == 0 { "x" } else { "z" }) };
    let channels: Vec<i32> = if w >= 13 { vec![3, 9] } else { vec![3] };
    for (plot, lx0) in [(0, 1), (1, 7)] {
        if plot == 1 && w < 13 {
            break;
        }
        let crop = crops[rng.below(crops.len() as u32) as usize];
        for lx in lx0..lx0 + 5 {
            for lz in 1..d - 1 {
                if channels.contains(&lx) {
                    b.set(f.at(lx, 0, lz), "water");
                } else {
                    b.set(f.at(lx, 0, lz), "farmland[moisturized_amount=7]");
                    b.set(f.at(lx, 1, lz), if rng.below(5) == 0 { "wheat[growth=4]" } else { crop });
                }
            }
        }
    }
    for lx in 0..w {
        for lz in 0..d {
            if lx == 0 || lx == w - 1 || lz == 0 || lz == d - 1 || (w >= 13 && lx == 6) {
                b.set(f.at(lx, 0, lz), &border);
            }
        }
    }
    b.set(f.at(0, 1, d - 1), "composter");
    if rng.below(2) == 0 {
        b.set(f.at(w - 1, 1, d - 1), "hay_block[pillar_axis=y]");
    }
}

/// Enclos : barrières, foin, abreuvoir.
fn pen(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng) {
    prepare(b, f, 7, 7, 3, "dirt");
    let fence = if s.theme == Theme::Desert { "oak_fence" } else { s.fence };
    for lx in 0..7 {
        for lz in 0..7 {
            b.set(f.at(lx, 0, lz), if rng.below(4) == 0 { "coarse_dirt" } else { "grass_block" });
            let edge = lx == 0 || lx == 6 || lz == 0 || lz == 6;
            if edge && !(lz == 6 && lx == 3) {
                b.set(f.at(lx, 1, lz), fence);
            }
        }
    }
    b.set(f.at(1, 1, 1), "hay_block[pillar_axis=y]");
    b.set(f.at(2, 1, 1), "hay_block[pillar_axis=x]");
    b.set(f.at(1, 2, 1), "hay_block[pillar_axis=y]");
    b.set(f.at(5, 1, 1), "cauldron[cauldron_liquid=water,fill_level=6]");
}

/// Lampadaire, selon le biome.
fn lamp(b: &mut Builder, f: Frame, s: &Style) {
    prepare(b, f, 1, 1, 4, s.base);
    b.set(f.at(0, 0, 0), s.base);
    match s.theme {
        Theme::Desert => {
            b.set(f.at(0, 1, 0), "cut_sandstone");
            b.set(f.at(0, 2, 0), "orange_terracotta");
            b.set(f.at(0, 3, 0), "torch[torch_facing_direction=top]");
        }
        Theme::Snowy => {
            b.fill(f.at(0, 1, 0), f.at(0, 2, 0), "stripped_spruce_log");
            b.set(f.at(0, 3, 0), "lantern[hanging=0]");
        }
        _ => {
            b.set(f.at(0, 1, 0), s.fence);
            b.set(f.at(0, 2, 0), s.fence);
            b.set(f.at(0, 3, 0), "lantern[hanging=0]");
        }
    }
}

/// Petit décor de rue : bottes de foin, citrouilles, baies, fleurs.
fn decoration(b: &mut Builder, f: Frame, s: &Style, rng: &mut Rng) {
    for lx in 0..3 {
        for lz in 0..3 {
            let p = f.at(lx, 0, lz);
            let top = b.top(p.0, p.2);
            if top < p.1 - 1 || top > p.1 + 1 || rng.below(3) == 0 {
                continue;
            }
            let item = match (s.theme, rng.below(3)) {
                (Theme::Taiga, 0) => "sweet_berry_bush[growth=3]",
                (Theme::Taiga | Theme::Snowy, _) => "pumpkin[minecraft:cardinal_direction=south]",
                (Theme::Desert, 0) => "cactus",
                (Theme::Desert, _) => "hay_block[pillar_axis=y]",
                (_, 0) => "poppy",
                (_, 1) => "dandelion",
                _ => "hay_block[pillar_axis=y]",
            };
            b.set((p.0, top + 1, p.2), item);
        }
    }
}

// --- Temples ----------------------------------------------------------------------------

/// Pyramide du désert (21×21), comme dans le jeu : salle centrale au sol de terre cuite,
/// deux tours en façade, sous le centre une chambre secrète à quatre coffres piégée de TNT
/// sous une plaque de pression, et (depuis la 1.20) une pièce enfouie de sable suspect.
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
    // Archéologie (1.20) : sous le sol de la salle, une pièce enfouie remplie de sable où se
    // cachent quelques blocs de sable suspect
    for lx in 4..=16 {
        for lz in 2..=6 {
            for ly in -5..=-2 {
                let edge = lx == 4 || lx == 16 || lz == 2 || lz == 6 || ly == -5 || ly == -2;
                b.set(f.at(lx, ly, lz), if edge { "cut_sandstone" } else { "sand" });
            }
        }
    }
    let mut placed = 0;
    let mut k = (x0 as i64 * 31 + z0 as i64 * 17).rem_euclid(9973) as i32;
    while placed < 4 {
        k = (k * 1103 + 12345) % 9973;
        let (lx, lz, ly) = (5 + k % 11, 3 + (k / 11) % 3, -4 + (k / 33) % 2);
        b.set(f.at(lx, ly, lz), "suspicious_sand");
        placed += 1;
    }
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

// --- Structures récentes ----------------------------------------------------------------
//
// Avant-postes de pillards (1.14), cités antiques (1.19), ruines de piste (1.20) et chambres
// des épreuves (1.21). Comme les autres, reconstruites à la main d'après celles du jeu.

/// Biomes des avant-postes de pillards.
fn outpost_biome(b: B) -> bool {
    matches!(
        b,
        B::Plains | B::SunflowerPlains | B::Desert | B::Savanna | B::SavannaPlateau | B::SnowyPlains | B::Taiga | B::SnowyTaiga | B::Grove | B::Meadow | B::CherryGrove | B::SnowySlopes
    )
}

/// Biomes des ruines de piste.
fn trail_ruins_biome(b: B) -> bool {
    matches!(b, B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga | B::OldGrowthBirchForest | B::Jungle)
}

impl<'a> Builder<'a> {
    /// Remplit un pavé (bornes incluses).
    fn fill(&mut self, a: (i32, i32, i32), c: (i32, i32, i32), name: &str) {
        for x in a.0.min(c.0)..=a.0.max(c.0) {
            for y in a.1.min(c.1)..=a.1.max(c.1) {
                for z in a.2.min(c.2)..=a.2.max(c.2) {
                    self.set((x, y, z), name);
                }
            }
        }
    }

    /// Vide un pavé (bornes incluses).
    fn fill_air(&mut self, a: (i32, i32, i32), c: (i32, i32, i32)) {
        for x in a.0.min(c.0)..=a.0.max(c.0) {
            for y in a.1.min(c.1)..=a.1.max(c.1) {
                for z in a.2.min(c.2)..=a.2.max(c.2) {
                    self.air((x, y, z));
                }
            }
        }
    }

    /// Conteneur (coffre, tonneau) dont le butin est tiré au premier clic.
    fn container(&mut self, pos: (i32, i32, i32), spec: &str, loot: Loot) {
        self.set(pos, spec);
        self.chests.push((pos, loot));
    }
}

/// Un élément au hasard dans une liste.
fn pick<'s>(rng: &mut Rng, list: &[&'s str]) -> &'s str {
    list[rng.below(list.len() as u32) as usize]
}

// Avant-poste de pillards : tour de bouleau et de chêne noir sur socle de pierre, terrasse
// à balustrade sous un toit, coffre au sommet ; autour, une cage, des mannequins, du bois.
fn pillager_outpost(b: &mut Builder, rng: &mut Rng, x0: i32, y0: i32, z0: i32) {
    for dx in -1..=7 {
        for dz in -1..=7 {
            b.foundation((x0 + dx, y0, z0 + dz), "cobblestone");
        }
    }
    b.fill_air((x0 - 2, y0 + 1, z0 - 2), (x0 + 8, y0 + 21, z0 + 8));
    b.fill((x0 - 1, y0, z0 - 1), (x0 + 7, y0, z0 + 7), "cobblestone");
    for y in y0 + 1..=y0 + 15 {
        for i in 0..=6 {
            for (x, z) in [(x0 + i, z0), (x0 + i, z0 + 6), (x0, z0 + i), (x0 + 6, z0 + i)] {
                let corner = (x == x0 || x == x0 + 6) && (z == z0 || z == z0 + 6);
                b.set((x, y, z), if corner { "dark_oak_log" } else if y <= y0 + 1 { "cobblestone" } else { "birch_planks" });
            }
        }
    }
    // Fenêtres sur chaque face, à chaque étage, et la porte au sud
    for base in [y0 + 2, y0 + 7, y0 + 12] {
        for y in base..=base + 1 {
            for (x, z) in [(x0 + 3, z0 + 6), (x0, z0 + 3), (x0 + 6, z0 + 3)] {
                b.air((x, y, z));
            }
            if base != y0 + 2 {
                b.air((x0 + 3, y, z0));
            }
        }
    }
    b.air((x0 + 3, y0 + 1, z0));
    b.air((x0 + 3, y0 + 2, z0));
    // Planchers, avec la trémie de l'échelle
    for y in [y0 + 5, y0 + 10] {
        b.fill((x0 + 1, y, z0 + 1), (x0 + 5, y, z0 + 5), "dark_oak_planks");
        b.air((x0 + 1, y, z0 + 5));
    }
    for y in y0 + 1..=y0 + 15 {
        b.set((x0 + 1, y, z0 + 5), &format!("ladder[facing_direction={}]", D::S.facing()));
    }
    // Terrasse
    b.fill((x0 - 1, y0 + 15, z0 - 1), (x0 + 7, y0 + 15, z0 + 7), "dark_oak_planks");
    b.set((x0 + 1, y0 + 15, z0 + 5), &format!("ladder[facing_direction={}]", D::S.facing()));
    for i in -1..=7 {
        for (x, z) in [(x0 + i, z0 - 1), (x0 + i, z0 + 7), (x0 - 1, z0 + i), (x0 + 7, z0 + i)] {
            let corner = (x == x0 - 1 || x == x0 + 7) && (z == z0 - 1 || z == z0 + 7);
            if corner {
                b.fill((x, y0 + 16, z), (x, y0 + 18, z), "dark_oak_log");
            } else {
                b.set((x, y0 + 16, z), "dark_oak_fence");
            }
        }
    }
    // Toit : rebord en dalles, centre en planches, sommet en dalles
    for dx in -1..=7 {
        for dz in -1..=7 {
            let rim = dx == -1 || dx == 7 || dz == -1 || dz == 7;
            b.set((x0 + dx, y0 + 19, z0 + dz), if rim { "dark_oak_slab[minecraft:vertical_half=bottom]" } else { "dark_oak_planks" });
        }
    }
    b.fill((x0 + 1, y0 + 20, z0 + 1), (x0 + 5, y0 + 20, z0 + 5), "dark_oak_slab[minecraft:vertical_half=bottom]");
    b.container((x0 + 4, y0 + 16, z0 + 3), &format!("chest[minecraft:cardinal_direction={}]", D::S.name()), Loot::PillagerOutpost);
    b.set((x0 + 5, y0 + 16, z0 + 1), "crafting_table");
    // Alentours : deux ou trois installations
    let features = 2 + rng.below(2) as i32;
    for k in 0..features {
        let angle = (k as f64 + rng.below(100) as f64 / 100.0) * std::f64::consts::TAU / features as f64;
        let d = 11.0 + rng.below(4) as f64;
        let (fx, fz) = (x0 + 3 + (angle.cos() * d) as i32, z0 + 3 + (angle.sin() * d) as i32);
        let gy = b.top(fx, fz) + 1;
        match rng.below(4) {
            // Cage de chêne noir
            0 => {
                for dx in 0..5 {
                    for dz in 0..5 {
                        b.foundation((fx + dx, gy, fz + dz), "dirt");
                        b.set((fx + dx, gy - 1, fz + dz), "dark_oak_planks");
                        let edge = dx == 0 || dx == 4 || dz == 0 || dz == 4;
                        let corner = (dx == 0 || dx == 4) && (dz == 0 || dz == 4);
                        for y in gy..=gy + 2 {
                            if corner {
                                b.set((fx + dx, y, fz + dz), "dark_oak_log");
                            } else if edge {
                                b.set((fx + dx, y, fz + dz), "dark_oak_fence");
                            } else {
                                b.air((fx + dx, y, fz + dz));
                            }
                        }
                        b.set((fx + dx, gy + 3, fz + dz), "dark_oak_slab[minecraft:vertical_half=bottom]");
                    }
                }
            }
            // Mannequins d'entraînement : piquet, botte de foin, citrouille
            1 => {
                for (dx, dz) in [(0, 0), (3, 1), (1, 3)] {
                    let (x, z) = (fx + dx, fz + dz);
                    let y = b.top(x, z) + 1;
                    b.set((x, y, z), "dark_oak_fence");
                    b.set((x, y + 1, z), "hay_block[pillar_axis=y]");
                    b.set((x, y + 2, z), &format!("carved_pumpkin[minecraft:cardinal_direction={}]", D::S.name()));
                }
            }
            // Tas de bûches
            2 => {
                for dz in 0..3 {
                    for dx in 0..4 {
                        let (x, z) = (fx + dx, fz + dz);
                        let y = b.top(x, z) + 1;
                        b.set((x, y, z), "dark_oak_log[pillar_axis=x]");
                        if dz == 1 && dx < 3 {
                            b.set((x, y + 1, z), "dark_oak_log[pillar_axis=x]");
                        }
                    }
                }
            }
            // Tente
            _ => {
                for dx in 0..5 {
                    let x = fx + dx;
                    let y = b.top(x, fz + 2) + 1;
                    for (dz, h) in [(0, 0), (1, 1), (2, 2), (3, 1), (4, 0)] {
                        b.set((x, y + h, fz + dz), "white_wool");
                        for yy in y..y + h {
                            if dx != 0 && dx != 4 {
                                b.air((x, yy, fz + dz));
                            }
                        }
                    }
                }
                let y = b.top(fx + 2, fz - 2) + 1;
                b.set((fx + 2, y, fz - 2), "campfire[extinguished=0]");
            }
        }
    }
}

// Ruines de piste : salles de briques de boue et de terre cuite enfouies sous le sol, reliées
// par des routes de terre cuite colorée, remplies de gravier où se cache du gravier suspect.
// Seul le haut d'une tour dépasse.
fn trail_ruins(b: &mut Builder, rng: &mut Rng, cx: i32, cz: i32) {
    const FLOOR: [&str; 6] = ["mud_bricks", "hardened_clay", "brown_terracotta", "red_terracotta", "yellow_terracotta", "light_gray_terracotta"];
    const WALL: [&str; 5] = ["mud_bricks", "mud_bricks", "packed_mud", "hardened_clay", "brown_terracotta"];
    const ROAD: [&str; 5] = ["red_terracotta", "yellow_terracotta", "light_blue_terracotta", "cyan_terracotta", "purple_terracotta"];
    let top = b.top(cx, cz);
    let floor = top - 9 - rng.below(3) as i32;
    // Salles : la première au centre, les autres en chaîne
    let mut rooms: Vec<(i32, i32, i32, i32)> = vec![(cx - 4, cz - 4, 9, 9)];
    let count = 3 + rng.below(3);
    for _ in 1..count {
        let (px, pz, pw, pd) = rooms[rng.below(rooms.len() as u32) as usize];
        let (w, d) = (5 + rng.below(5) as i32, 5 + rng.below(5) as i32);
        let gap = 4 + rng.below(5) as i32;
        let (x, z) = match rng.below(4) {
            0 => (px + pw + gap, pz + rng.below(pd as u32) as i32 - d / 2),
            1 => (px - gap - w, pz + rng.below(pd as u32) as i32 - d / 2),
            2 => (px + rng.below(pw as u32) as i32 - w / 2, pz + pd + gap),
            _ => (px + rng.below(pw as u32) as i32 - w / 2, pz - gap - d),
        };
        if rooms.iter().any(|(ox, oz, ow, od)| x < ox + ow + 1 && x + w + 1 > *ox && z < oz + od + 1 && z + d + 1 > *oz) {
            continue;
        }
        // Route depuis la salle d'origine : trois de large, en terre cuite colorée
        let colour = pick(rng, &ROAD);
        let (ax, az) = (px + pw / 2, pz + pd / 2);
        let (bx, bz) = (x + w / 2, z + d / 2);
        let steps = (bx - ax).abs().max((bz - az).abs()).max(1);
        for s in 0..=steps {
            let (rx, rz) = (ax + (bx - ax) * s / steps, az + (bz - az) * s / steps);
            for o in -1..=1 {
                let (qx, qz) = if (bx - ax).abs() > (bz - az).abs() { (rx, rz + o) } else { (rx + o, rz) };
                b.set((qx, floor, qz), if rng.below(5) == 0 { "gravel" } else { colour });
                for y in floor + 1..=floor + 3 {
                    b.set((qx, y, qz), if rng.below(3) == 0 { "dirt" } else { "gravel" });
                }
            }
        }
        rooms.push((x, z, w, d));
    }
    let tower = rng.below(rooms.len() as u32) as usize;
    for (i, (x0, z0, w, d)) in rooms.iter().copied().enumerate() {
        let height = if i == tower { (top - floor + 2).max(6) } else { 4 + rng.below(3) as i32 };
        let wall = pick(rng, &WALL);
        for dx in 0..w {
            for dz in 0..d {
                let (x, z) = (x0 + dx, z0 + dz);
                b.set((x, floor, z), pick(rng, &FLOOR));
                let edge = dx == 0 || dx == w - 1 || dz == 0 || dz == d - 1;
                for y in floor + 1..=floor + height {
                    if edge {
                        // Murs usés : des briques manquent, plus souvent vers le haut
                        let broken = rng.below(100) < 4 + ((y - floor) * 3) as u32;
                        b.set((x, y, z), if broken { "gravel" } else if rng.below(6) == 0 { pick(rng, &FLOOR) } else { wall });
                    } else if y <= floor + 2 && rng.below(100) < 7 {
                        b.set((x, y, z), "suspicious_gravel");
                    } else if y <= floor + 4 {
                        b.set((x, y, z), if rng.below(4) == 0 { "dirt" } else { "gravel" });
                    }
                }
            }
        }
        // Ouverture vers la route
        let (mx, mz) = (x0 + w / 2, z0 + d / 2);
        for (x, z) in [(mx, z0), (mx, z0 + d - 1), (x0, mz), (x0 + w - 1, mz)] {
            for y in floor + 1..=floor + 2 {
                b.set((x, y, z), "gravel");
            }
        }
    }
}

// Chambres des épreuves : sous terre, en tuf et en cuivre. Un hall central, des couloirs
// éclairés d'ampoules de cuivre, et au bout des salles : générateurs d'épreuves, coffres-forts
// (« vaults »), réserves.
fn trial_chambers(b: &mut Builder, rng: &mut Rng, cx: i32, y0: i32, cz: i32) {
    // Hall central (17 × 17, 9 de haut)
    room_shell(b, (cx - 9, y0, cz - 9), (cx + 9, y0 + 10, cz + 9));
    for x in cx - 8..=cx + 8 {
        for z in cz - 8..=cz + 8 {
            let ring = (x - cx).abs().max((z - cz).abs());
            let name = match ring {
                0..=1 => "chiseled_tuff",
                3 | 6 => "waxed_cut_copper",
                _ => "polished_tuff",
            };
            b.set((x, y0, z), name);
        }
    }
    // Piliers et lumières
    for (sx, sz) in [(-5, -5), (5, -5), (-5, 5), (5, 5)] {
        for y in y0 + 1..=y0 + 9 {
            b.set((cx + sx, y, cz + sz), if y % 3 == 0 { "waxed_copper" } else { "chiseled_tuff_bricks" });
        }
    }
    for (lx, lz) in [(-3, -3), (3, -3), (-3, 3), (3, 3), (0, -6), (0, 6), (-6, 0), (6, 0)] {
        b.set((cx + lx, y0 + 10, cz + lz), "waxed_copper_bulb[lit=1]");
    }
    b.set((cx, y0 + 1, cz), "chiseled_tuff_bricks");
    b.set((cx, y0 + 2, cz), "trial_spawner");
    for (px, pz) in [(-7, -7), (7, -7), (-7, 7), (7, 7)] {
        b.set((cx + px, y0 + 1, cz + pz), "decorated_pot");
    }
    // Quatre couloirs ; au bout, une salle de chaque sorte (l'ordre change)
    let mut kinds = [0u8, 1, 2, 3];
    for i in (1..4).rev() {
        let j = rng.below(i as u32 + 1) as usize;
        kinds.swap(i, j);
    }
    for (side, kind) in [D::N, D::E, D::S, D::W].into_iter().zip(kinds) {
        let (dx, dz) = side.step();
        let len = 12 + rng.below(10) as i32;
        let (px, pz) = (-dz, dx);
        // Couloir : 3 de large, 4 de haut à l'intérieur
        for s in 9..9 + len {
            let (x, z) = (cx + dx * s, cz + dz * s);
            for o in -2..=2 {
                let (qx, qz) = (x + px * o, z + pz * o);
                for y in y0..=y0 + 5 {
                    let wall = o.abs() == 2 || y == y0 || y == y0 + 5;
                    if !wall {
                        b.air((qx, y, qz));
                        continue;
                    }
                    let name = if y == y0 {
                        if o == 0 && s % 4 == 0 { "waxed_cut_copper" } else { "polished_tuff" }
                    } else if y == y0 + 5 {
                        if o == 0 && s % 3 == 0 { "waxed_copper_grate" } else { "tuff_bricks" }
                    } else if y == y0 + 1 {
                        "waxed_cut_copper"
                    } else {
                        "tuff_bricks"
                    };
                    b.set((qx, y, qz), name);
                }
            }
            if s % 6 == 0 {
                b.set((x + px * 2, y0 + 3, z + pz * 2), "waxed_copper_bulb[lit=1]");
                b.set((x - px * 2, y0 + 3, z - pz * 2), "waxed_copper_bulb[lit=1]");
            }
        }
        // Ouverture dans le mur du hall
        for o in -1..=1 {
            for y in y0 + 1..=y0 + 4 {
                b.air((cx + dx * 9 + px * o, y, cz + dz * 9 + pz * o));
            }
        }
        // Un coffre dans le couloir, parfois
        if rng.below(2) == 0 {
            let s = 9 + 2 + rng.below((len - 4) as u32) as i32;
            b.container((cx + dx * s + px, y0 + 1, cz + dz * s + pz), &format!("chest[minecraft:cardinal_direction={}]", side.turn(2).name()), Loot::TrialCorridor);
        }
        // La salle (13 × 13, 8 de haut)
        let (rx, rz) = (cx + dx * (9 + len + 6), cz + dz * (9 + len + 6));
        room_shell(b, (rx - 7, y0, rz - 7), (rx + 7, y0 + 9, rz + 7));
        for o in -1..=1 {
            for y in y0 + 1..=y0 + 4 {
                b.air((rx - dx * 7 + px * o, y, rz - dz * 7 + pz * o));
            }
        }
        for (lx, lz) in [(-3, -3), (3, -3), (-3, 3), (3, 3)] {
            b.set((rx + lx, y0 + 9, rz + lz), "waxed_copper_bulb[lit=1]");
        }
        let facing = side.turn(2);
        match kind {
            // Salle des générateurs
            0 => {
                for o in [-3, 3] {
                    let (x, z) = (rx + px * o, rz + pz * o);
                    b.set((x, y0 + 1, z), "chiseled_tuff");
                    b.set((x, y0 + 2, z), "trial_spawner");
                }
                for (ox, oz) in [(-6, -6), (6, -6), (-6, 6), (6, 6)] {
                    b.set((rx + ox, y0 + 1, rz + oz), "decorated_pot");
                }
                b.container((rx + dx * 5, y0 + 1, rz + dz * 5), &format!("chest[minecraft:cardinal_direction={}]", facing.name()), Loot::TrialCorridor);
            }
            // Salle des coffres-forts : estrade et deux « vaults » tournés vers l'entrée
            1 => {
                let (bx, bz) = (rx + dx * 4, rz + dz * 4);
                for o in -3..=3 {
                    for k in -1..=2 {
                        b.set((bx + px * o + dx * k, y0 + 1, bz + pz * o + dz * k), "polished_tuff");
                    }
                }
                for o in [-2, 2] {
                    b.set((bx + px * o, y0 + 2, bz + pz * o), &format!("vault[minecraft:cardinal_direction={}]", facing.name()));
                }
                b.set((bx, y0 + 2, bz), "chiseled_copper");
                for (ox, oz) in [(-6, -6), (6, -6), (-6, 6), (6, 6)] {
                    b.set((rx + ox, y0 + 1, rz + oz), "decorated_pot");
                }
            }
            // Réserve : tonneaux, coffres, foin
            2 => {
                for o in -5..=5 {
                    let (x, z) = (rx + dx * 6 + px * o, rz + dz * 6 + pz * o);
                    if o % 2 == 0 {
                        b.container((x, y0 + 1, z), "barrel[facing_direction=1]", Loot::TrialSupply);
                    } else {
                        b.set((x, y0 + 1, z), "hay_block[pillar_axis=y]");
                    }
                }
                for o in [-5, 5] {
                    b.container((rx + px * o * 6 / 5 + dx, y0 + 1, rz + pz * o * 6 / 5 + dz), &format!("chest[minecraft:cardinal_direction={}]", facing.name()), Loot::TrialSupply);
                }
                b.set((rx - px * 6, y0 + 1, rz - pz * 6), "crafting_table");
                b.set((rx, y0 + 1, rz), "decorated_pot");
            }
            // Salle de combat à deux niveaux : balcon et générateur au centre
            _ => {
                for o in -6..=6 {
                    for k in 4..=6 {
                        b.set((rx + px * o + dx * k, y0 + 4, rz + pz * o + dz * k), "tuff_bricks");
                    }
                    b.set((rx + px * o + dx * 4, y0 + 5, rz + pz * o + dz * 4), "waxed_copper_grate");
                }
                for k in 1..=3 {
                    b.set((rx + px * 6 + dx * k, y0 + k, rz + pz * 6 + dz * k), "tuff_bricks");
                }
                b.set((rx, y0 + 1, rz), "chiseled_tuff_bricks");
                b.set((rx, y0 + 2, rz), "trial_spawner");
                b.container((rx + dx * 6, y0 + 5, rz + dz * 6), &format!("chest[minecraft:cardinal_direction={}]", facing.name()), Loot::TrialSupply);
            }
        }
    }
}

/// Salle de tuf : murs, sol et plafond, intérieur vidé.
fn room_shell(b: &mut Builder, a: (i32, i32, i32), c: (i32, i32, i32)) {
    for x in a.0..=c.0 {
        for z in a.2..=c.2 {
            for y in a.1..=c.1 {
                let edge = x == a.0 || x == c.0 || z == a.2 || z == c.2;
                if y == a.1 {
                    b.set((x, y, z), "polished_tuff");
                } else if y == c.1 {
                    b.set((x, y, z), "tuff_bricks");
                } else if edge {
                    let name = if y == a.1 + 1 {
                        "waxed_cut_copper"
                    } else if (x + z + y) % 7 == 0 {
                        "chiseled_tuff_bricks"
                    } else {
                        "tuff_bricks"
                    };
                    b.set((x, y, z), name);
                } else {
                    b.air((x, y, z));
                }
            }
        }
    }
}

// Cité antique : une grande caverne au fond du deep dark (y = -51), sol de tuiles d'ardoise
// des abîmes, le grand cadre de deepslate renforcé, des maisons, des colonnes à lanternes
// d'âmes, et du sculk partout (capteurs, hurleurs).
fn ancient_city(b: &mut Builder, rng: &mut Rng, cx: i32, cz: i32) {
    let fy = -51;
    let (hw, hd) = (40, 32);
    // Caverne en dôme
    for dx in -hw..=hw {
        for dz in -hd..=hd {
            let r = (dx as f64 / hw as f64).powi(2) + (dz as f64 / hd as f64).powi(2);
            if r > 1.0 {
                continue;
            }
            let h = (8.0 + 14.0 * (1.0 - r).sqrt()) as i32;
            let (x, z) = (cx + dx, cz + dz);
            let floor = match rng.below(10) {
                0 => "cracked_deepslate_tiles",
                1 | 2 => "polished_deepslate",
                _ => "deepslate_tiles",
            };
            b.set((x, fy, z), floor);
            b.set((x, fy - 1, z), "cobbled_deepslate");
            for y in fy + 1..=fy + h {
                b.air((x, y, z));
            }
        }
    }
    // Le grand cadre, au nord
    let (px, pz) = (cx, cz - 20);
    b.fill((px - 15, fy + 1, pz - 4), (px + 15, fy + 2, pz + 4), "deepslate_bricks");
    for x in px - 15..=px + 15 {
        b.set((x, fy + 2, pz + 5), &format!("deepslate_brick_stairs[weirdo_direction={},upside_down_bit=0]", D::N.stairs()));
    }
    for x in px - 9..=px + 9 {
        for y in fy + 3..=fy + 15 {
            let edge = x == px - 9 || x == px + 9 || y == fy + 3 || y == fy + 15;
            if edge {
                b.set((x, y, pz), "reinforced_deepslate");
            }
        }
    }
    for side in [-1, 1] {
        let tx = px + side * 12;
        b.fill((tx - 1, fy + 3, pz - 2), (tx + 1, fy + 17, pz + 2), "deepslate_bricks");
        b.fill((tx - 1, fy + 18, pz - 2), (tx + 1, fy + 18, pz + 2), "chiseled_deepslate");
        b.set((tx, fy + 19, pz), "soul_lantern[hanging=0]");
        for y in (fy + 5..=fy + 15).step_by(5) {
            b.set((tx, y, pz + 3), "soul_lantern[hanging=0]");
        }
    }
    // Grande allée vers le sud, bordée de colonnes
    for z in pz + 6..=cz + hd - 4 {
        for dx in -2..=2 {
            b.set((cx + dx, fy, z), if dx.abs() == 2 { "polished_deepslate" } else { "deepslate_bricks" });
        }
        if (z - pz) % 7 == 0 {
            for side in [-4, 4] {
                for y in fy + 1..=fy + 4 {
                    b.set((cx + side, y, z), "deepslate_brick_wall[wall_post_bit=1]");
                }
                b.set((cx + side, fy + 5, z), "soul_lantern[hanging=0]");
            }
        }
    }
    // Maisons, de part et d'autre de l'allée
    let mut taken: Vec<(i32, i32, i32, i32)> = vec![(px - 16, pz - 5, 33, 11), (cx - 5, pz + 6, 11, hd + 30)];
    for _ in 0..40 {
        if taken.len() >= 16 {
            break;
        }
        let (w, d) = (5 + rng.below(5) as i32, 5 + rng.below(5) as i32);
        let x0 = cx - hw + 3 + rng.below((2 * hw - 6 - w) as u32) as i32;
        let z0 = cz - hd + 3 + rng.below((2 * hd - 6 - d) as u32) as i32;
        // Dans la caverne, et à l'écart des autres
        let inside = |x: i32, z: i32| ((x - cx) as f64 / hw as f64).powi(2) + ((z - cz) as f64 / hd as f64).powi(2) < 0.8;
        if !(inside(x0, z0) && inside(x0 + w, z0) && inside(x0, z0 + d) && inside(x0 + w, z0 + d)) {
            continue;
        }
        if taken.iter().any(|(ox, oz, ow, od)| x0 < ox + ow + 2 && x0 + w + 2 > *ox && z0 < oz + od + 2 && z0 + d + 2 > *oz) {
            continue;
        }
        taken.push((x0, z0, w, d));
        city_house(b, rng, x0, fy, z0, w, d, cz);
    }
    // Sculk : des plaques sur le sol, avec capteurs et hurleurs
    for _ in 0..34 {
        let (sx, sz) = (cx - hw + rng.below((2 * hw) as u32) as i32, cz - hd + rng.below((2 * hd) as u32) as i32);
        let radius = 2 + rng.below(4) as i32;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx * dx + dz * dz > radius * radius || rng.below(4) == 0 {
                    continue;
                }
                let (x, z) = (sx + dx, sz + dz);
                let r = ((x - cx) as f64 / hw as f64).powi(2) + ((z - cz) as f64 / hd as f64).powi(2);
                if r > 0.95 || taken.iter().any(|(ox, oz, ow, od)| x >= *ox && x < ox + ow && z >= *oz && z < oz + od) {
                    continue;
                }
                b.set((x, fy, z), "sculk");
                match rng.below(60) {
                    0..=2 => b.set((x, fy + 1, z), "sculk_sensor"),
                    3 => b.set((x, fy + 1, z), "sculk_shrieker[can_summon=1]"),
                    _ => {}
                }
            }
        }
    }
    b.set((cx + 7, fy + 1, cz + 5), "sculk_catalyst");
    b.set((cx - 7, fy + 1, cz - 3), "sculk_catalyst");
}

/// Maison de la cité antique : murs de briques d'ardoise (parfois fissurées), toit de dalles,
/// laine grise au sol, bougies, lanterne d'âme, et souvent un coffre.
#[allow(clippy::too_many_arguments)]
fn city_house(b: &mut Builder, rng: &mut Rng, x0: i32, fy: i32, z0: i32, w: i32, d: i32, cz: i32) {
    let h = 4 + rng.below(3) as i32;
    for dx in 0..w {
        for dz in 0..d {
            let (x, z) = (x0 + dx, z0 + dz);
            let edge = dx == 0 || dx == w - 1 || dz == 0 || dz == d - 1;
            b.set((x, fy, z), if edge { "deepslate_bricks" } else if rng.below(3) == 0 { "light_blue_wool" } else { "gray_wool" });
            for y in fy + 1..=fy + h {
                if edge {
                    b.set((x, y, z), if rng.below(7) == 0 { "cracked_deepslate_bricks" } else { "deepslate_bricks" });
                } else {
                    b.air((x, y, z));
                }
            }
            b.set((x, fy + h + 1, z), if edge { "deepslate_tile_slab[minecraft:vertical_half=bottom]" } else { "deepslate_tiles" });
        }
    }
    // Porte du côté de l'allée centrale, fenêtres
    let door_z = if z0 + d / 2 < cz { z0 + d - 1 } else { z0 };
    for y in fy + 1..=fy + 2 {
        b.air((x0 + w / 2, y, door_z));
    }
    for (x, z) in [(x0, z0 + d / 2), (x0 + w - 1, z0 + d / 2)] {
        b.air((x, fy + 2, z));
    }
    // Intérieur
    for _ in 0..3 {
        let (x, z) = (x0 + 1 + rng.below((w - 2) as u32) as i32, z0 + 1 + rng.below((d - 2) as u32) as i32);
        b.set((x, fy + 1, z), &format!("candle[candles={},lit=0]", rng.below(4)));
    }
    b.set((x0 + 1, fy + h, z0 + 1), "soul_lantern[hanging=1]");
    if rng.below(3) != 0 {
        let front = if door_z == z0 { D::N } else { D::S };
        let z = if door_z == z0 { z0 + d - 2 } else { z0 + 1 };
        b.container((x0 + w - 2, fy + 1, z), &format!("chest[minecraft:cardinal_direction={}]", front.name()), Loot::AncientCity);
    }
    if rng.below(2) == 0 {
        b.set((x0 + 1, fy + 1, z0 + d - 2), "sculk_shrieker[can_summon=1]");
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
    PillagerOutpost,
    TrailRuins,
    TrialChambers,
    AncientCity,
    Shipwreck,
    OceanRuin,
    Igloo,
    RuinedPortal,
    Mansion,
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
            "pillager_outpost" | "outpost" => Target::PillagerOutpost,
            "trail_ruins" => Target::TrailRuins,
            "trial_chambers" | "trial_chamber" => Target::TrialChambers,
            "ancient_city" => Target::AncientCity,
            "shipwreck" => Target::Shipwreck,
            "ocean_ruin" | "ocean_ruins" => Target::OceanRuin,
            "igloo" => Target::Igloo,
            "ruined_portal" => Target::RuinedPortal,
            "mansion" | "woodland_mansion" => Target::Mansion,
            _ => return None,
        })
    }

    /// « se trouve » ou « se trouvent », selon le nom.
    pub fn verb(self) -> &'static str {
        match self {
            Target::TrailRuins | Target::TrialChambers | Target::OceanRuin => "se trouvent",
            _ => "se trouve",
        }
    }

    /// Début de la réponse de `/locate`.
    pub fn label(self) -> &'static str {
        match self {
            Target::Village => "Le village le plus proche",
            Target::DesertPyramid => "La pyramide du désert la plus proche",
            Target::JungleTemple => "Le temple de la jungle le plus proche",
            Target::Temple => "Le temple le plus proche",
            Target::Mineshaft => "La mine abandonnée la plus proche",
            Target::PillagerOutpost => "L'avant-poste de pillards le plus proche",
            Target::TrailRuins => "Les ruines de piste les plus proches",
            Target::TrialChambers => "Les chambres des épreuves les plus proches",
            Target::AncientCity => "La cité antique la plus proche",
            Target::Shipwreck => "L'épave la plus proche",
            Target::OceanRuin => "Les ruines océaniques les plus proches",
            Target::Igloo => "L'igloo le plus proche",
            Target::RuinedPortal => "Le portail en ruine le plus proche",
            Target::Mansion => "Le manoir le plus proche",
        }
    }
}

/// Structures du jeu que la commande connaît mais que Basalt ne génère pas encore.
pub const NOT_GENERATED: [&str; 10] = [
    "bastion_remnant", "buried_treasure", "end_city", "fortress", "mansion", "monument", "ocean_ruins", "ruined_portal", "shipwreck", "stronghold",
];

/// Cases d'une couronne carrée de rayon `r` autour de l'origine.
fn ring(r: i32) -> impl Iterator<Item = (i32, i32)> {
    (-r..=r).flat_map(move |dx| (-r..=r).map(move |dz| (dx, dz))).filter(move |(dx, dz)| dx.abs().max(dz.abs()) == r)
}

/// La structure de cette sorte est-elle bien générée à cet emplacement de départ ?
/// Renvoie sa position (bloc) — mêmes règles que `build`.
fn placed(v: &Vanilla, kind: Kind, sx: i32, sz: i32) -> Option<(i32, i32, i32)> {
    let (x, z) = (sx * 16 + 8, sz * 16 + 8);
    // Le biome d'abord (rapide), la hauteur du sol seulement s'il convient
    let biome = v.surface_biome_at(x, z);
    match kind {
        Kind::Village => {
            if Style::for_biome(biome).is_none() {
                return None;
            }
            let top = v.surface(x, z).0;
            // Pas de village au bord d'une falaise : la place du centre doit être à peu près plane
            let around: Vec<i32> = [(-8, -8), (8, -8), (-8, 8), (8, 8)].iter().map(|(dx, dz)| v.surface(x + dx, z + dz).0).collect();
            let flat = around.iter().all(|h| (h - top).abs() <= 6);
            (flat && top > SEA_LEVEL && top <= SEA_LEVEL + 60).then_some((x, top + 1, z))
        }
        Kind::DesertPyramid => {
            if biome != B::Desert {
                return None;
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL).then_some((x, top + 1, z))
        }
        Kind::JungleTemple => {
            if !matches!(biome, B::Jungle | B::BambooJungle) {
                return None;
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL).then_some((x, top + 1, z))
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
        Kind::PillagerOutpost => {
            // Un emplacement sur cinq, comme le jeu, et jamais à moins de 10 chunks d'un village
            if mix(v.seed(), sx as i64, sz as i64, 165_745_297) % 5 != 0 || !outpost_biome(biome) {
                return None;
            }
            let (rx0, rz0) = (sx.div_euclid(34), sz.div_euclid(34));
            for rx in rx0 - 1..=rx0 + 1 {
                for rz in rz0 - 1..=rz0 + 1 {
                    let (vx, vz) = grid_start(v.seed(), rx, rz, 34, 8, 10_387_312);
                    if (vx - sx).abs() <= 10 && (vz - sz).abs() <= 10 && Style::for_biome(v.surface_biome_at(vx * 16 + 8, vz * 16 + 8)).is_some() {
                        return None;
                    }
                }
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL && top < SEA_LEVEL + 90).then_some((x, top + 1, z))
        }
        Kind::TrailRuins => {
            if !trail_ruins_biome(biome) {
                return None;
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL).then_some((x, top - 9, z))
        }
        Kind::TrialChambers => {
            if v.deep_dark_at(x, z) {
                return None;
            }
            let mut rng = Rng::new(mix(v.seed(), sx as i64, sz as i64, 94_251_328));
            let y = -40 + rng.below(21) as i32;
            // Assez de roche au-dessus
            (v.surface(x, z).0 > y + 30).then_some((x, y, z))
        }
        Kind::AncientCity => (v.deep_dark_at(x, z) && v.surface(x, z).0 > -10).then_some((x, -50, z)),
        // Épave : au fond de l'océan, ou échouée sur une plage
        Kind::Shipwreck => {
            if !biome.ocean() && !matches!(biome, B::Beach | B::SnowyBeach) {
                return None;
            }
            let top = v.surface(x, z).0;
            (top < SEA_LEVEL + 3).then_some((x, top + 1, z))
        }
        Kind::OceanRuin => {
            if !biome.ocean() {
                return None;
            }
            let top = v.surface(x, z).0;
            (top < SEA_LEVEL - 3).then_some((x, top + 1, z))
        }
        Kind::Igloo => {
            if !matches!(biome, B::SnowyPlains | B::SnowyTaiga | B::SnowySlopes) {
                return None;
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL).then_some((x, top + 1, z))
        }
        Kind::RuinedPortal => {
            if biome.ocean() || matches!(biome, B::River | B::FrozenRiver) {
                return None;
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL).then_some((x, top + 1, z))
        }
        // Manoir : forêt sombre (ou jardin pâle), sur le point le plus bas de son emprise
        Kind::Mansion => {
            if !matches!(biome, B::DarkForest | B::PaleGarden) {
                return None;
            }
            let low = [(-16, -16), (16, -16), (-16, 16), (16, 16), (0, 0)].iter().map(|(dx, dz)| v.surface(x + dx, z + dz).0).min().unwrap_or(0);
            (low > SEA_LEVEL).then_some((x, low + 1, z))
        }
        Kind::Fossil => {
            if !matches!(biome, B::Desert | B::Swamp | B::MangroveSwamp) {
                return None;
            }
            let top = v.surface(x, z).0;
            (top > SEA_LEVEL).then_some((x, top - 15, z))
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
        Target::PillagerOutpost => &[Kind::PillagerOutpost],
        Target::TrailRuins => &[Kind::TrailRuins],
        Target::TrialChambers => &[Kind::TrialChambers],
        Target::AncientCity => &[Kind::AncientCity],
        Target::Shipwreck => &[Kind::Shipwreck],
        Target::OceanRuin => &[Kind::OceanRuin],
        Target::Igloo => &[Kind::Igloo],
        Target::RuinedPortal => &[Kind::RuinedPortal],
        Target::Mansion => &[Kind::Mansion],
    };
    // Structures qui n'existent qu'avec les plans officiels
    if matches!(what, Target::Shipwreck | Target::OceanRuin | Target::Igloo | Target::RuinedPortal | Target::Mansion) && !v.structures.official.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let dist = |p: &(i32, i32, i32)| ((p.0 - x) as i64).pow(2) + ((p.2 - z) as i64).pow(2);
    let mut best: Option<(i32, i32, i32)> = None;
    for kind in kinds {
        // (espacement de la grille en chunks, séparation, sel ; 1 pour les mines : chaque chunk)
        let (spacing, separation, salt) = GRID.iter().find(|g| g.0 == *kind).map(|g| (g.1, g.2, g.3)).unwrap_or((1, 0, 0));
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
        Loot::PillagerOutpost => (
            &[
                ("crossbow", 1, 1, 2),
                ("wheat", 3, 5, 7),
                ("potato", 2, 5, 5),
                ("carrot", 3, 5, 5),
                ("dark_oak_log", 2, 3, 10),
                ("experience_bottle", 1, 1, 7),
                ("string", 1, 6, 4),
                ("arrow", 2, 7, 4),
                ("tripwire_hook", 1, 3, 3),
                ("iron_ingot", 1, 3, 3),
                ("goat_horn", 1, 1, 1),
            ],
            (2, 5),
        ),
        Loot::AncientCity => (
            &[
                ("enchanted_golden_apple", 1, 2, 1),
                ("music_disc_otherside", 1, 1, 1),
                ("compass", 1, 1, 2),
                ("sculk_catalyst", 1, 2, 2),
                ("name_tag", 1, 1, 2),
                ("diamond_hoe", 1, 1, 2),
                ("lead", 1, 1, 2),
                ("diamond_horse_armor", 1, 1, 2),
                ("saddle", 1, 1, 2),
                ("music_disc_13", 1, 1, 2),
                ("music_disc_cat", 1, 1, 2),
                ("diamond_leggings", 1, 1, 2),
                ("echo_shard", 1, 3, 4),
                ("disc_fragment_5", 1, 3, 4),
                ("experience_bottle", 1, 3, 3),
                ("golden_apple", 1, 2, 3),
                ("iron_leggings", 1, 1, 3),
                ("sculk_sensor", 1, 3, 3),
                ("candle", 1, 4, 3),
                ("amethyst_shard", 1, 15, 3),
                ("soul_torch", 1, 15, 3),
                ("sculk", 4, 10, 3),
                ("coal", 6, 15, 7),
                ("bone", 1, 15, 5),
            ],
            (5, 10),
        ),
        Loot::TrialCorridor => (
            &[
                ("iron_axe", 1, 1, 1),
                ("honeycomb", 1, 8, 1),
                ("stone_axe", 1, 1, 2),
                ("stone_pickaxe", 1, 1, 2),
                ("ender_pearl", 1, 2, 2),
                ("bamboo_hanging_sign", 1, 4, 2),
                ("bamboo_planks", 3, 6, 2),
                ("scaffolding", 2, 10, 2),
                ("torch", 3, 6, 2),
                ("tuff", 8, 20, 3),
                ("wind_charge", 1, 3, 2),
                ("emerald", 1, 3, 1),
            ],
            (1, 3),
        ),
        Loot::ShipwreckSupply => (
            &[
                ("paper", 1, 12, 8),
                ("potato", 2, 6, 7),
                ("poisonous_potato", 2, 6, 7),
                ("carrot", 4, 8, 7),
                ("wheat", 8, 21, 7),
                ("coal", 2, 8, 6),
                ("rotten_flesh", 5, 24, 5),
                ("pumpkin", 1, 3, 2),
                ("bamboo", 1, 3, 2),
                ("gunpowder", 1, 5, 3),
                ("tnt", 1, 2, 1),
                ("moss_block", 1, 4, 2),
            ],
            (3, 10),
        ),
        Loot::ShipwreckTreasure => (
            &[
                ("iron_ingot", 1, 5, 90),
                ("gold_ingot", 1, 5, 10),
                ("emerald", 1, 5, 40),
                ("diamond", 1, 1, 5),
                ("experience_bottle", 1, 1, 5),
                ("iron_nugget", 1, 10, 50),
                ("gold_nugget", 1, 10, 10),
                ("lapis_lazuli", 1, 10, 20),
            ],
            (3, 6),
        ),
        Loot::ShipwreckMap => (
            &[("paper", 1, 10, 20), ("feather", 1, 5, 10), ("book", 1, 5, 5), ("clock", 1, 1, 1), ("compass", 1, 1, 1)],
            (3, 6),
        ),
        Loot::OceanRuin => (
            &[
                ("coal", 1, 4, 10),
                ("stone_axe", 1, 1, 2),
                ("rotten_flesh", 1, 1, 5),
                ("emerald", 1, 1, 1),
                ("wheat", 2, 3, 10),
                ("gold_nugget", 1, 3, 5),
                ("golden_apple", 1, 1, 1),
                ("fishing_rod", 1, 1, 2),
            ],
            (2, 8),
        ),
        Loot::Igloo => (
            &[
                ("apple", 1, 3, 15),
                ("coal", 1, 4, 15),
                ("gold_nugget", 1, 3, 10),
                ("stone_axe", 1, 1, 2),
                ("rotten_flesh", 1, 1, 10),
                ("emerald", 1, 1, 1),
                ("wheat", 2, 3, 10),
                ("golden_apple", 1, 1, 1),
            ],
            (2, 8),
        ),
        Loot::RuinedPortal => (
            &[
                ("obsidian", 1, 2, 40),
                ("flint", 1, 4, 40),
                ("iron_nugget", 9, 18, 40),
                ("flint_and_steel", 1, 1, 40),
                ("fire_charge", 1, 1, 40),
                ("golden_apple", 1, 1, 15),
                ("gold_nugget", 4, 24, 15),
                ("golden_sword", 1, 1, 15),
                ("golden_axe", 1, 1, 15),
                ("golden_pickaxe", 1, 1, 15),
                ("golden_helmet", 1, 1, 15),
                ("golden_boots", 1, 1, 15),
                ("glistering_melon_slice", 4, 12, 5),
                ("golden_carrot", 4, 12, 5),
                ("gold_ingot", 2, 8, 5),
                ("clock", 1, 1, 5),
                ("bell", 1, 1, 1),
                ("enchanted_golden_apple", 1, 1, 1),
                ("gold_block", 1, 2, 1),
            ],
            (4, 8),
        ),
        Loot::WoodlandMansion => (
            &[
                ("lead", 1, 1, 20),
                ("golden_apple", 1, 1, 15),
                ("music_disc_13", 1, 1, 15),
                ("music_disc_cat", 1, 1, 15),
                ("name_tag", 1, 1, 20),
                ("chainmail_chestplate", 1, 1, 10),
                ("diamond_hoe", 1, 1, 15),
                ("diamond_chestplate", 1, 1, 5),
                ("book", 1, 1, 10),
                ("iron_ingot", 1, 4, 10),
                ("bread", 1, 1, 20),
                ("wheat", 1, 4, 20),
                ("bucket", 1, 1, 10),
                ("redstone", 1, 4, 15),
                ("coal", 1, 4, 15),
                ("melon_seeds", 2, 4, 10),
                ("pumpkin_seeds", 2, 4, 10),
                ("beetroot_seeds", 2, 4, 10),
                ("bone", 1, 8, 10),
                ("gunpowder", 1, 8, 10),
                ("rotten_flesh", 1, 8, 10),
                ("string", 1, 8, 10),
            ],
            (2, 7),
        ),
        Loot::TrialSupply => (
            &[
                ("arrow", 4, 14, 2),
                ("baked_potato", 2, 4, 2),
                ("glow_berries", 2, 10, 2),
                ("acacia_planks", 3, 6, 1),
                ("moss_block", 2, 5, 1),
                ("bone_meal", 2, 5, 1),
                ("tuff", 5, 10, 1),
                ("torch", 3, 6, 1),
                ("potion", 1, 1, 1),
                ("milk_bucket", 1, 1, 1),
                ("bread", 2, 4, 2),
                ("wind_charge", 1, 2, 1),
            ],
            (3, 5),
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

    /// Les structures récentes : trouvées par `/locate`, construites avec ce qui les
    /// caractérise (générateurs et coffres-forts, cadre de deepslate renforcé, gravier
    /// suspect, coffre au sommet de la tour).
    #[test]
    fn recent_structures_are_found_and_built() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let Ok(data) = crate::data::GameData::load(&dir) else { return };
        let p = Palette::new(&|n| data.state_from_spec(n));
        let v = Vanilla::new(12345);
        for (what, kind, signature, chests) in [
            (Target::PillagerOutpost, Kind::PillagerOutpost, "dark_oak_fence", 1),
            (Target::TrailRuins, Kind::TrailRuins, "suspicious_gravel", 0),
            (Target::TrialChambers, Kind::TrialChambers, "vault", 3),
            (Target::AncientCity, Kind::AncientCity, "reinforced_deepslate", 2),
        ] {
            let started = std::time::Instant::now();
            let (x, y, z) = locate(&v, what, 0, 0).unwrap_or_else(|| panic!("{:?} introuvable", what));
            let found_in = started.elapsed();
            let plan = build(&v, &p, (kind, (x - 8) >> 4, (z - 8) >> 4));
            let names: Vec<&str> = plan.blocks.values().flatten().filter_map(|b| data.block_name(b.3)).collect();
            let has = |n: &str| names.iter().any(|b| b.trim_start_matches("minecraft:") == n);
            let count = plan.chests.values().map(|c| c.len()).sum::<usize>();
            eprintln!("{:?} en {} {} {} ({} blocs, {} coffres, trouvé en {:?})", what, x, y, z, names.len(), count, found_in);
            assert!(has(signature), "{:?} sans {}", what, signature);
            if what == Target::TrialChambers {
                // Le monde lui-même contient la structure : générateur au centre du hall
                let world = super::super::World::with_generator(super::super::generator::Generator::new(super::super::generator::Kind::Vanilla, 12345, Palette::new(&|n| data.state_from_spec(n))));
                assert_eq!(data.block_name(world.get(x, y + 2, z)), Some("minecraft:trial_spawner"));
            }
            assert!(count >= chests, "{:?} : {} coffres", what, count);
        }
    }

    /// Avec les plans officiels (s'ils sont présents) : chaque structure est assemblée de
    /// plusieurs pièces, avec ses blocs typiques et ses coffres.
    #[test]
    fn official_plans_are_assembled() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let Ok(data) = crate::data::GameData::load(&dir) else { return };
        let path = std::env::var("BASALT_STRUCTURES").unwrap_or_else(|_| "/tmp/vs/structures".into());
        let Ok(t) = Templates::load(std::path::Path::new(&path), &|n| data.state_from_spec(n)) else {
            eprintln!("plans absents : test ignoré");
            return;
        };
        let p = Palette::new(&|n| data.state_from_spec(n)).with_templates(Some(std::sync::Arc::new(t)));
        let v = Vanilla::new(12345);
        for (what, kind, signature, chests) in [
            (Target::Village, Kind::Village, "bell", 1),
            (Target::PillagerOutpost, Kind::PillagerOutpost, "dark_oak_fence", 1),
            (Target::TrailRuins, Kind::TrailRuins, "suspicious_gravel", 0),
            (Target::AncientCity, Kind::AncientCity, "sculk_shrieker", 2),
            (Target::Shipwreck, Kind::Shipwreck, "chest", 1),
            (Target::OceanRuin, Kind::OceanRuin, "chest", 1),
            (Target::Igloo, Kind::Igloo, "snow", 0),
            (Target::RuinedPortal, Kind::RuinedPortal, "netherrack", 0),
            (Target::Mansion, Kind::Mansion, "dark_oak_planks", 3),
        ] {
            // Le monde sait qu'il a les plans (sinon `locate` ignore ces structures)
            near(&v, &p, &v.structures, 0, 0);
            let (x, _, z) = locate(&v, what, 0, 0).unwrap_or_else(|| panic!("{:?} introuvable", what));
            let plan = build(&v, &p, (kind, (x - 8) >> 4, (z - 8) >> 4));
            let names: Vec<&str> = plan.blocks.values().flatten().filter_map(|b| data.block_name(b.3)).collect();
            let has = |n: &str| names.iter().any(|b| b.trim_start_matches("minecraft:") == n);
            let count = plan.chests.values().map(|c| c.len()).sum::<usize>();
            eprintln!("{:?} : {} blocs, {} coffres", what, names.len(), count);
            assert!(has(signature), "{:?} sans {}", what, signature);
            assert!(count >= chests, "{:?} : {} coffres", what, count);
            // Plus de jigsaw ni de bloc de structure dans le monde
            assert!(!has("jigsaw") && !has("structure_block"));
        }
        // Un fossile, dans le premier chunk de désert ou de marais qui en a un
        let fossil = (0..4000).map(|i| (i % 64 - 32, i / 64 - 32)).find_map(|(cx, cz)| {
            (mix(v.seed(), cx as i64, cz as i64, 77_002) % 64 == 0 && placed(&v, Kind::Fossil, cx, cz).is_some()).then(|| build(&v, &p, (Kind::Fossil, cx, cz)))
        });
        if let Some(plan) = fossil {
            assert!(plan.blocks.values().flatten().any(|b| data.block_name(b.3) == Some("minecraft:bone_block")), "fossile sans os");
        }
        // Le monde généré contient bien le village, sans erreur
        let (x, y, z) = locate(&v, Target::Village, 0, 0).unwrap();
        let world = super::super::World::with_generator(super::super::generator::Generator::new(super::super::generator::Kind::Vanilla, 12345, p.clone()));
        assert_ne!(world.get(x, y - 1, z), p.air);
        // Les lits gardent leur couleur : envoyée avec le chunk
        let plan = build(&v, &p, (Kind::Village, (x - 8) >> 4, (z - 8) >> 4));
        let (&(cx, cz), beds) = plan.beds.iter().find(|(_, b)| !b.is_empty()).expect("des lits");
        assert!(plan.beds.values().flatten().any(|b| b.3 != 14), "pas que des lits rouges");
        let (bx, by, bz, _) = beds[0];
        world.get(cx * 16 + bx as i32, by as i32, cz * 16 + bz as i32);
        let (_, payload) = world.chunk_payload(cx, cz);
        assert!(payload.windows(3).any(|w| w == b"Bed"), "couleur des lits absente du chunk");
        // Épave : ses escaliers et dalles gardent l'eau de la mer
        let (x, _, z) = locate(&v, Target::Shipwreck, 0, 0).unwrap();
        let plan = build(&v, &p, (Kind::Shipwreck, (x - 8) >> 4, (z - 8) >> 4));
        let wet = plan.blocks.iter().flat_map(|((cx, cz), list)| list.iter().map(move |b| (cx * 16 + b.0 as i32, b.1 as i32, cz * 16 + b.2 as i32))).filter(|&(x, y, z)| {
            world.get(x, y, z);
            world.is_wet(x, y, z)
        });
        assert!(wet.count() > 10, "épave sans eau dans ses blocs");
    }

    /// Tout le butin des structures existe dans le jeu.
    #[test]
    fn loot_items_exist() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let Ok(data) = crate::data::GameData::load(&dir) else { return };
        for kind in [Loot::Village, Loot::DesertPyramid, Loot::JungleTemple, Loot::Mineshaft, Loot::PillagerOutpost, Loot::AncientCity, Loot::TrialCorridor, Loot::TrialSupply, Loot::ShipwreckSupply, Loot::ShipwreckTreasure, Loot::ShipwreckMap, Loot::OceanRuin, Loot::Igloo, Loot::RuinedPortal, Loot::WoodlandMansion] {
            for (name, ..) in table(kind).0 {
                assert!(data.item(&format!("minecraft:{}", name)).is_some() || data.item(name).is_some(), "objet inconnu : {}", name);
            }
        }
    }

    /// Palette de test, avec les plans officiels si `OFFICIAL` est défini (et les plans
    /// présents, voir `templates`).
    fn test_palette(data: &crate::data::GameData) -> Palette {
        let p = Palette::new(&|n| data.state_from_spec(n));
        if std::env::var("OFFICIAL").is_err() {
            return p;
        }
        let path = std::env::var("BASALT_STRUCTURES").unwrap_or_else(|_| "/tmp/vs/structures".into());
        let t = Templates::load(std::path::Path::new(&path), &|n| data.state_from_spec(n)).ok().map(std::sync::Arc::new);
        p.with_templates(t)
    }

    /// Vue de dessus du monde généré autour d'un village de taïga (arbres compris), dans
    /// /tmp/world-village.txt (x z y nom). `OFFICIAL=1 cargo test world_around_village -- --ignored`
    #[test]
    #[ignore]
    fn world_around_village() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let p = test_palette(&data);
        let v = Vanilla::new(12345);
        let mut found = None;
        'search: for r in 0..40i32 {
            for (dx, dz) in ring(r) {
                let (sx, sz) = grid_start(v.seed(), dx, dz, 34, 8, 10_387_312);
                if matches!(v.surface_biome_at(sx * 16 + 8, sz * 16 + 8), B::Taiga | B::SnowyTaiga) && placed(&v, Kind::Village, sx, sz).is_some() {
                    found = Some((sx * 16 + 8, sz * 16 + 8));
                    break 'search;
                }
            }
        }
        let (x, z) = found.unwrap();
        eprintln!("village de taïga en {} {}", x, z);
        let mut out = String::new();
        for cx in (x >> 4) - 5..=(x >> 4) + 5 {
            for cz in (z >> 4) - 5..=(z >> 4) + 5 {
                let col = v.generate(&p, cx, cz);
                for lx in 0..16 {
                    for lz in 0..16 {
                        let mut y = MAX_Y;
                        while y > MIN_Y && col.get(lx, y, lz) == p.air {
                            y -= 1;
                        }
                        let n = data.block_name(col.get(lx, y, lz)).unwrap_or("?").trim_start_matches("minecraft:");
                        out.push_str(&format!("{} {} {} {}\n", cx * 16 + lx as i32, y, cz * 16 + lz as i32, n));
                    }
                }
            }
        }
        std::fs::write("/tmp/world-village.txt", out).unwrap();
    }

    /// Écrit les blocs d'une structure récente dans /tmp/plan-<sorte>.txt (x y z nom), pour
    /// les dessiner. `cargo test plan_files -- --ignored`
    #[test]
    #[ignore]
    fn plan_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let p = test_palette(&data);
        let v = Vanilla::new(12345);
        for (what, kind, label) in [
            (Target::PillagerOutpost, Kind::PillagerOutpost, "outpost"),
            (Target::TrailRuins, Kind::TrailRuins, "trail"),
            (Target::TrialChambers, Kind::TrialChambers, "trial"),
            (Target::AncientCity, Kind::AncientCity, "city"),
            (Target::Shipwreck, Kind::Shipwreck, "shipwreck"),
            (Target::OceanRuin, Kind::OceanRuin, "oceanruin"),
            (Target::Igloo, Kind::Igloo, "igloo"),
            (Target::RuinedPortal, Kind::RuinedPortal, "portal"),
            (Target::Mansion, Kind::Mansion, "mansion"),
        ] {
            near(&v, &p, &v.structures, 0, 0);
            let (x, _, z) = locate(&v, what, 0, 0).unwrap();
            let started = std::time::Instant::now();
            let plan = build(&v, &p, (kind, (x - 8) >> 4, (z - 8) >> 4));
            eprintln!("{} : {} blocs en {:?}", label, plan.blocks.values().map(|b| b.len()).sum::<usize>(), started.elapsed());
            let mut out = String::new();
            for ((cx, cz), list) in &plan.blocks {
                for (bx, y, bz, s) in list {
                    let n = data.block_name(*s).unwrap_or("?").trim_start_matches("minecraft:");
                    out.push_str(&format!("{} {} {} {}\n", cx * 16 + *bx as i32, y, cz * 16 + *bz as i32, n));
                }
            }
            std::fs::write(format!("/tmp/plan-{}.txt", label), out).unwrap();
        }
    }

    /// Écrit un village de chaque biome (et une pyramide) dans /tmp/village-<biome>.txt.
    /// `cargo test village_files -- --ignored`
    #[test]
    #[ignore]
    fn village_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let p = test_palette(&data);
        let v = Vanilla::new(std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(12345));
        let mut done: Vec<String> = Vec::new();
        let write = |plan: &Plan, label: &str| {
            let mut out = String::new();
            for ((cx, cz), list) in &plan.blocks {
                for (bx, y, bz, s) in list {
                    let n = data.block_name(*s).unwrap_or("?").trim_start_matches("minecraft:");
                    out.push_str(&format!("{} {} {} {}\n", cx * 16 + *bx as i32, y, cz * 16 + *bz as i32, n));
                }
            }
            std::fs::write(format!("/tmp/village-{}.txt", label), out).unwrap();
        };
        'search: for r in 0..60i32 {
            for (dx, dz) in ring(r) {
                let (sx, sz) = grid_start(v.seed(), dx, dz, 34, 8, 10_387_312);
                let biome = v.surface_biome_at(sx * 16 + 8, sz * 16 + 8);
                let Some(style) = Style::for_biome(biome) else { continue };
                let label = format!("{:?}", style.theme).to_lowercase();
                if done.contains(&label) {
                    continue;
                }
                let plan = build(&v, &p, (Kind::Village, sx, sz));
                if plan.blocks.values().map(|b| b.len()).sum::<usize>() < 3000 {
                    continue;
                }
                eprintln!("village {} en {} {}", label, sx * 16 + 8, sz * 16 + 8);
                write(&plan, &label);
                done.push(label);
                if done.len() == 5 {
                    break 'search;
                }
            }
        }
        let (x, _, z) = locate(&v, Target::DesertPyramid, 0, 0).unwrap();
        write(&build(&v, &p, (Kind::DesertPyramid, (x - 8) >> 4, (z - 8) >> 4)), "pyramid");
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
