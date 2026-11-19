//! Édition du monde en volume : remplir, remplacer, copier, coller, annuler. C'est le moteur
//! du plugin EasyEdit ; il travaille directement sur le monde, côté serveur, pour qu'une
//! opération sur des dizaines de milliers de blocs ne coûte rien au plugin.
//!
//! Chaque joueur a sa `EditSession` : son presse-papiers et son historique pour annuler.

use std::collections::HashSet;

use super::World;

pub type Pos = (i32, i32, i32);

/// Nombre maximal de blocs touchés par une seule opération.
pub const MAX_BLOCKS: usize = 250_000;
/// Nombre d'opérations que l'on peut annuler.
const MAX_HISTORY: usize = 20;

/// Volume rectangulaire, coins compris.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub min: Pos,
    pub max: Pos,
}

impl Region {
    pub fn new(a: Pos, b: Pos) -> Region {
        Region { min: (a.0.min(b.0), a.1.min(b.1), a.2.min(b.2)), max: (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)) }
    }

    pub fn size(&self) -> (i64, i64, i64) {
        ((self.max.0 as i64 - self.min.0 as i64 + 1), (self.max.1 as i64 - self.min.1 as i64 + 1), (self.max.2 as i64 - self.min.2 as i64 + 1))
    }

    pub fn volume(&self) -> i64 {
        let (x, y, z) = self.size();
        x.saturating_mul(y).saturating_mul(z)
    }

    fn check(&self) -> Result<(), String> {
        if self.volume() > MAX_BLOCKS as i64 {
            return Err(format!("sélection trop grande : {} blocs (maximum {})", self.volume(), MAX_BLOCKS));
        }
        Ok(())
    }

    fn positions(&self) -> impl Iterator<Item = Pos> + '_ {
        let r = *self;
        (r.min.1..=r.max.1).flat_map(move |y| (r.min.0..=r.max.0).flat_map(move |x| (r.min.2..=r.max.2).map(move |z| (x, y, z))))
    }

    fn on_wall(&self, p: Pos) -> bool {
        p.0 == self.min.0 || p.0 == self.max.0 || p.2 == self.min.2 || p.2 == self.max.2
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Change {
    pos: Pos,
    old: u32,
    new: u32,
}

/// Blocs copiés, notés par leur décalage depuis l'endroit où se tenait le joueur.
#[derive(Debug, Clone, Default)]
struct Clipboard {
    blocks: Vec<(Pos, u32)>,
}

/// Résultat d'une opération : nombre de blocs changés et chunks à renvoyer aux joueurs.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub changed: usize,
    pub chunks: Vec<(i32, i32)>,
    /// Blocs changés, quand ils sont assez peu nombreux pour être envoyés un par un
    pub blocks: Vec<(Pos, u32)>,
}

#[derive(Default)]
pub struct EditSession {
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
    clipboard: Option<Clipboard>,
    /// Graine du tirage au sort des motifs à plusieurs blocs
    seed: u64,
}

impl EditSession {
    pub fn new() -> EditSession {
        EditSession::default()
    }

    /// Bloc tiré au sort dans un motif (`stone,dirt` : moitié pierre, moitié terre).
    fn pick(&mut self, pattern: &[u32]) -> u32 {
        if pattern.len() == 1 {
            return pattern[0];
        }
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        pattern[((self.seed >> 33) as usize) % pattern.len()]
    }

    /// Écrit des blocs dans le monde et note l'opération pour pouvoir l'annuler.
    fn commit(&mut self, world: &mut World, wanted: Vec<(Pos, u32)>) -> Result<Outcome, String> {
        if wanted.len() > MAX_BLOCKS {
            return Err(format!("opération trop grande : {} blocs (maximum {})", wanted.len(), MAX_BLOCKS));
        }
        let mut changes = Vec::new();
        for (pos, new) in wanted {
            if !World::in_bounds(pos.1) {
                continue;
            }
            let old = world.get(pos.0, pos.1, pos.2);
            if old != new && world.set(pos.0, pos.1, pos.2, new) {
                changes.push(Change { pos, old, new });
            }
        }
        let outcome = Self::outcome(&changes, false);
        if !changes.is_empty() {
            self.undo.push(changes);
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        Ok(outcome)
    }

    fn outcome(changes: &[Change], reverse: bool) -> Outcome {
        let chunks: HashSet<(i32, i32)> = changes.iter().map(|c| (c.pos.0.div_euclid(16), c.pos.2.div_euclid(16))).collect();
        let mut chunks: Vec<(i32, i32)> = chunks.into_iter().collect();
        chunks.sort();
        let blocks = if changes.len() <= 512 { changes.iter().map(|c| (c.pos, if reverse { c.old } else { c.new })).collect() } else { Vec::new() };
        Outcome { changed: changes.len(), chunks, blocks }
    }

    /// Remplit la sélection.
    pub fn set(&mut self, world: &mut World, region: Region, pattern: &[u32]) -> Result<Outcome, String> {
        region.check()?;
        let wanted = region.positions().map(|p| (p, self.pick(pattern))).collect();
        self.commit(world, wanted)
    }

    /// Remplace un bloc par un autre dans la sélection.
    pub fn replace(&mut self, world: &mut World, region: Region, from: u32, pattern: &[u32]) -> Result<Outcome, String> {
        region.check()?;
        let targets: Vec<Pos> = region.positions().filter(|p| world.get(p.0, p.1, p.2) == from).collect();
        let wanted = targets.into_iter().map(|p| (p, self.pick(pattern))).collect();
        self.commit(world, wanted)
    }

    /// Les quatre murs de la sélection.
    pub fn walls(&mut self, world: &mut World, region: Region, pattern: &[u32]) -> Result<Outcome, String> {
        region.check()?;
        let wanted = region.positions().filter(|p| region.on_wall(*p)).map(|p| (p, self.pick(pattern))).collect();
        self.commit(world, wanted)
    }

    /// Sphère (pleine ou creuse) autour d'un point.
    pub fn sphere(&mut self, world: &mut World, center: Pos, radius: i32, pattern: &[u32], hollow: bool) -> Result<Outcome, String> {
        let radius = radius.clamp(1, 64);
        let (outer, inner) = (((radius as f32) + 0.5).powi(2), ((radius as f32) - 0.5).powi(2));
        let mut wanted = Vec::new();
        for y in -radius..=radius {
            for x in -radius..=radius {
                for z in -radius..=radius {
                    let d = (x * x + y * y + z * z) as f32;
                    if d <= outer && (!hollow || d >= inner) {
                        wanted.push(((center.0 + x, center.1 + y, center.2 + z), self.pick(pattern)));
                    }
                }
            }
        }
        self.commit(world, wanted)
    }

    /// Cylindre vertical (plein ou creux) posé sur un point.
    pub fn cylinder(&mut self, world: &mut World, base: Pos, radius: i32, height: i32, pattern: &[u32], hollow: bool) -> Result<Outcome, String> {
        let (radius, height) = (radius.clamp(1, 64), height.clamp(1, 384));
        let (outer, inner) = (((radius as f32) + 0.5).powi(2), ((radius as f32) - 0.5).powi(2));
        let mut wanted = Vec::new();
        for y in 0..height {
            for x in -radius..=radius {
                for z in -radius..=radius {
                    let d = (x * x + z * z) as f32;
                    if d <= outer && (!hollow || d >= inner) {
                        wanted.push(((base.0 + x, base.1 + y, base.2 + z), self.pick(pattern)));
                    }
                }
            }
        }
        self.commit(world, wanted)
    }

    /// Copie la sélection ; `origin` est l'endroit où se tient le joueur, repris au collage.
    pub fn copy(&mut self, world: &World, region: Region, origin: Pos) -> Result<usize, String> {
        region.check()?;
        let blocks: Vec<(Pos, u32)> = region.positions().map(|p| ((p.0 - origin.0, p.1 - origin.1, p.2 - origin.2), world.get(p.0, p.1, p.2))).collect();
        let count = blocks.len();
        self.clipboard = Some(Clipboard { blocks });
        Ok(count)
    }

    /// Copie la sélection puis la vide.
    pub fn cut(&mut self, world: &mut World, region: Region, origin: Pos) -> Result<Outcome, String> {
        self.copy(world, region, origin)?;
        let air = world.air;
        self.set(world, region, &[air])
    }

    /// Colle le presse-papiers par rapport à l'endroit où se tient le joueur. Avec
    /// `skip_air`, les vides de la copie n'effacent pas ce qui existe.
    pub fn paste(&mut self, world: &mut World, origin: Pos, skip_air: bool) -> Result<Outcome, String> {
        let clip = self.clipboard.clone().ok_or("rien n'a été copié : utilisez d'abord copy ou cut")?;
        let air = world.air;
        let wanted = clip.blocks.iter().filter(|(_, b)| !(skip_air && *b == air)).map(|(o, b)| ((origin.0 + o.0, origin.1 + o.1, origin.2 + o.2), *b)).collect();
        self.commit(world, wanted)
    }

    /// Répète la sélection `times` fois dans une direction.
    pub fn stack(&mut self, world: &mut World, region: Region, direction: Pos, times: i32) -> Result<Outcome, String> {
        region.check()?;
        let times = times.clamp(1, 64) as i64;
        if region.volume() * times > MAX_BLOCKS as i64 {
            return Err(format!("empilement trop grand : {} blocs (maximum {})", region.volume() * times, MAX_BLOCKS));
        }
        let (sx, sy, sz) = region.size();
        let source: Vec<(Pos, u32)> = region.positions().map(|p| (p, world.get(p.0, p.1, p.2))).collect();
        let mut wanted = Vec::with_capacity(source.len() * times as usize);
        for n in 1..=times {
            let shift = ((direction.0 as i64 * sx * n) as i32, (direction.1 as i64 * sy * n) as i32, (direction.2 as i64 * sz * n) as i32);
            wanted.extend(source.iter().map(|(p, b)| ((p.0 + shift.0, p.1 + shift.1, p.2 + shift.2), *b)));
        }
        self.commit(world, wanted)
    }

    /// Déplace la sélection de `distance` blocs dans une direction.
    pub fn shift(&mut self, world: &mut World, region: Region, direction: Pos, distance: i32) -> Result<Outcome, String> {
        region.check()?;
        let distance = distance.clamp(1, 512);
        let offset = (direction.0 * distance, direction.1 * distance, direction.2 * distance);
        let source: Vec<(Pos, u32)> = region.positions().map(|p| (p, world.get(p.0, p.1, p.2))).collect();
        let air = world.air;
        let moved: HashSet<Pos> = source.iter().map(|(p, _)| (p.0 + offset.0, p.1 + offset.1, p.2 + offset.2)).collect();
        // D'abord vider l'ancien emplacement (sauf là où la sélection déplacée arrive), puis écrire
        let mut wanted: Vec<(Pos, u32)> = source.iter().filter(|(p, _)| !moved.contains(p)).map(|(p, _)| (*p, air)).collect();
        wanted.extend(source.iter().map(|(p, b)| ((p.0 + offset.0, p.1 + offset.1, p.2 + offset.2), *b)));
        self.commit(world, wanted)
    }

    /// Compte les blocs de la sélection (un bloc précis, ou tout ce qui n'est pas de l'air).
    pub fn count(world: &World, region: Region, block: Option<u32>) -> Result<usize, String> {
        region.check()?;
        Ok(region.positions().filter(|p| {
            let b = world.get(p.0, p.1, p.2);
            match block {
                Some(wanted) => b == wanted,
                None => b != world.air,
            }
        }).count())
    }

    /// Annule la dernière opération.
    pub fn undo(&mut self, world: &mut World) -> Result<Outcome, String> {
        let changes = self.undo.pop().ok_or("rien à annuler")?;
        for c in changes.iter().rev() {
            world.set(c.pos.0, c.pos.1, c.pos.2, c.old);
        }
        let outcome = Self::outcome(&changes, true);
        self.redo.push(changes);
        Ok(outcome)
    }

    /// Refait la dernière opération annulée.
    pub fn redo(&mut self, world: &mut World) -> Result<Outcome, String> {
        let changes = self.redo.pop().ok_or("rien à refaire")?;
        for c in &changes {
            world.set(c.pos.0, c.pos.1, c.pos.2, c.new);
        }
        let outcome = Self::outcome(&changes, false);
        self.undo.push(changes);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bedrock::block_hash;

    fn count_of(w: &World, r: Region, b: u32) -> usize {
        EditSession::count(w, r, Some(b)).unwrap()
    }

    #[test]
    fn fill_replace_undo_and_redo() {
        let mut w = World::new();
        let mut s = EditSession::new();
        let (stone, glass, air) = (block_hash("minecraft:stone"), block_hash("minecraft:glass"), w.air);
        let r = Region::new((2, 10, 2), (-1, 12, 0));
        assert_eq!((r.min, r.max, r.volume()), ((-1, 10, 0), (2, 12, 2), 36));
        let done = s.set(&mut w, r, &[stone]).unwrap();
        assert_eq!((done.changed, done.blocks.len()), (36, 36));
        assert_eq!(done.chunks, vec![(-1, 0), (0, 0)]);
        assert_eq!(count_of(&w, r, stone), 36);
        // Refaire la même chose ne change rien et n'encombre pas l'historique
        assert_eq!(s.set(&mut w, r, &[stone]).unwrap().changed, 0);
        assert_eq!(s.replace(&mut w, r, stone, &[glass]).unwrap().changed, 36);
        assert_eq!(s.replace(&mut w, r, stone, &[glass]).unwrap().changed, 0);
        assert_eq!(s.undo(&mut w).unwrap().changed, 36);
        assert_eq!(count_of(&w, r, stone), 36);
        assert_eq!(s.undo(&mut w).unwrap().changed, 36);
        assert_eq!(count_of(&w, r, air), 36);
        assert!(s.undo(&mut w).is_err());
        assert_eq!(s.redo(&mut w).unwrap().changed, 36);
        assert_eq!(s.redo(&mut w).unwrap().changed, 36);
        assert_eq!(count_of(&w, r, glass), 36);
        assert!(s.redo(&mut w).is_err());
        // Motif à plusieurs blocs : les deux apparaissent
        s.set(&mut w, r, &[stone, glass]).unwrap();
        assert!(count_of(&w, r, stone) > 5 && count_of(&w, r, glass) > 5);
        // Garde-fous
        assert!(s.set(&mut w, Region::new((0, 0, 0), (200, 100, 200)), &[stone]).unwrap_err().contains("trop grande"));
        let high = s.set(&mut w, Region::new((0, 318, 0), (0, 330, 0)), &[stone]).unwrap();
        assert!(high.changed <= 13, "au-dessus du monde, rien n'est écrit");
    }

    #[test]
    fn shapes_copy_paste_stack_and_move() {
        let mut w = World::new();
        let mut s = EditSession::new();
        let (stone, glass, air) = (block_hash("minecraft:stone"), block_hash("minecraft:glass"), w.air);
        // Murs d'une boîte 5×3×5 : le tour sans l'intérieur
        let box_ = Region::new((0, 20, 0), (4, 22, 4));
        assert_eq!(s.walls(&mut w, box_, &[stone]).unwrap().changed, 16 * 3);
        assert_eq!(w.get(2, 21, 2), air);
        // Sphère creuse : vide au centre, et moins de blocs que la pleine
        let hollow = s.sphere(&mut w, (40, 40, 40), 5, &[glass], true).unwrap().changed;
        assert_eq!(w.get(40, 40, 40), air);
        assert_eq!(w.get(45, 40, 40), glass);
        s.undo(&mut w).unwrap();
        let full = s.sphere(&mut w, (40, 40, 40), 5, &[glass], false).unwrap().changed;
        assert!(hollow < full && full > 600 && full < 800, "{} / {}", hollow, full);
        assert_eq!(s.cylinder(&mut w, (80, 30, 80), 3, 4, &[stone], false).unwrap().changed, 37 * 4);
        // Copier-coller par rapport au joueur
        assert_eq!(s.copy(&w, box_, (0, 20, 0)).unwrap(), 75);
        assert_eq!(s.paste(&mut w, (100, 50, 100), true).unwrap().changed, 48);
        assert_eq!(w.get(104, 52, 104), stone);
        assert!(EditSession::new().paste(&mut w, (0, 0, 0), false).is_err());
        // Empiler deux fois vers le haut
        assert_eq!(s.stack(&mut w, box_, (0, 1, 0), 2).unwrap().changed, 96);
        assert_eq!(w.get(0, 28, 0), stone);
        // Déplacer : l'ancien emplacement est vidé
        let line = Region::new((0, 60, 0), (2, 60, 0));
        s.set(&mut w, line, &[stone]).unwrap();
        s.shift(&mut w, line, (1, 0, 0), 2).unwrap();
        assert_eq!([w.get(0, 60, 0), w.get(1, 60, 0), w.get(2, 60, 0), w.get(3, 60, 0), w.get(4, 60, 0)], [air, air, stone, stone, stone]);
        s.undo(&mut w).unwrap();
        assert_eq!([w.get(0, 60, 0), w.get(2, 60, 0), w.get(4, 60, 0)], [stone, stone, air]);
        // Couper : copie puis vide
        assert_eq!(s.cut(&mut w, line, (0, 60, 0)).unwrap().changed, 3);
        assert_eq!(w.get(1, 60, 0), air);
        assert_eq!(s.paste(&mut w, (0, 70, 0), false).unwrap().changed, 3);
    }
}
