//! Le monde : un terrain généré (voir `generator`), plus les modifications des joueurs.
//!
//! Le terrain de base est une fonction de la graine : il se régénère à l'identique et n'est
//! jamais enregistré. Les chunks générés sont gardés dans un cache (les plus anciens en
//! sortent quand il est plein) et générés à l'avance par des fils de travail, autour des
//! joueurs. Seuls les blocs modifiés sont gardés à part et enregistrés dans `level.nbt`,
//! avec le type de terrain et la graine.
//!
//! Les plugins lisent le monde depuis leur propre fil, par une « vue » (`WorldView`) : une
//! copie instantanée qui ne coûte rien à prendre. Le stockage des modifications est partagé,
//! et n'est recopié (chunk par chunk) que si le serveur modifie un bloc pendant qu'une vue
//! est encore utilisée ; le terrain généré est partagé tel quel.

pub mod generator;
pub mod leveldb;
pub mod mojang;
pub mod light;
pub mod liquid;
pub mod structures;
pub mod templates;
pub mod vanilla;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::bedrock::W;
#[cfg(test)]
use crate::bedrock::block_hash;
use generator::{Column, Generator, Kind, Palette, Section};

/// Hauteur minimale du monde.
pub const MIN_Y: i32 = -64;
/// Hauteur maximale où l'on peut construire.
pub const MAX_Y: i32 = 319;
/// Blocs modifiés tolérés au total : au-delà, les nouvelles modifications sont refusées.
const MAX_EDITS: usize = 2_000_000;
/// Chunks générés gardés en mémoire (environ 40 Ko chacun : 320 Mo au plus). Au-delà, les
/// moins récemment utilisés sont oubliés et seront régénérés au besoin. (2 048 ne suffisait
/// pas à 500 joueurs dispersés : les chunks sortaient puis revenaient sans cesse.)
const CACHE_CHUNKS: usize = 8192;

/// Blocs modifiés d'un chunk : (x local, y, z local) -> identifiant réseau du bloc.
type ChunkEdits = HashMap<(u8, i16, u8), u32>;
/// Blocs posés dans l'eau qui la gardent (dalles, escaliers, clôtures…), par chunk
type WetMap = HashMap<(i32, i32), Arc<HashSet<(u8, i16, u8)>>>;

/// Chunks générés, partagés entre la boucle du serveur, les fils de génération et les plugins.
struct Terrain {
    generator: Generator,
    chunks: Mutex<HashMap<(i32, i32), (Arc<Column>, u64)>>,
    clock: AtomicU64,
    /// Chunks demandés aux fils de génération et pas encore prêts
    pending: Mutex<HashSet<(i32, i32)>>,
    jobs: Mutex<Option<Sender<(i32, i32)>>>,
    /// Base du monde : un chunk qui y est enregistré est servi tel quel, au lieu d'être généré
    store: std::sync::OnceLock<Arc<mojang::Store>>,
    /// Chunks à garder en mémoire : au moins `CACHE_CHUNKS`, davantage quand les joueurs en
    /// voient plus (voir `World::keep_chunks`)
    limit: std::sync::atomic::AtomicUsize,
}

impl Terrain {
    fn new(generator: Generator) -> Arc<Terrain> {
        Arc::new(Terrain {
            generator,
            chunks: Mutex::new(HashMap::new()),
            clock: AtomicU64::new(0),
            pending: Mutex::new(HashSet::new()),
            jobs: Mutex::new(None),
            store: std::sync::OnceLock::new(),
            limit: std::sync::atomic::AtomicUsize::new(CACHE_CHUNKS),
        })
    }

    fn cached(&self, key: (i32, i32)) -> Option<Arc<Column>> {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut chunks = self.chunks.lock().unwrap_or_else(|e| e.into_inner());
        chunks.get_mut(&key).map(|(c, s)| {
            *s = stamp;
            c.clone()
        })
    }

    fn insert(&self, key: (i32, i32), column: Arc<Column>) {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut chunks = self.chunks.lock().unwrap_or_else(|e| e.into_inner());
        chunks.insert(key, (column, stamp));
        if chunks.len() > self.limit.load(Ordering::Relaxed).max(CACHE_CHUNKS) {
            // Les moins récemment utilisés sortent (ils seront régénérés au besoin)
            let mut stamps: Vec<u64> = chunks.values().map(|(_, s)| *s).collect();
            stamps.sort_unstable();
            let cut = stamps[stamps.len() / 4];
            chunks.retain(|_, (_, s)| *s > cut);
        }
    }

    /// Fabrique un chunk : celui de la base s'il y est enregistré, sinon celui du générateur.
    /// Dans un monde créé par Basalt, un chunk enregistré garde ce que le générateur sait de
    /// lui (coffres et lits des structures) ; dans un monde venu du jeu, il est pris tel quel.
    fn produce(&self, key: (i32, i32)) -> Column {
        let Some(store) = self.store.get() else { return self.generator.generate(key.0, key.1) };
        match store.load(key.0, key.1) {
            Some(stored) => {
                let air = self.generator.blocks.air;
                let mut column = if store.native {
                    self.generator.generate(key.0, key.1)
                } else {
                    Column::from_blocks(&vec![air; 256 * (MAX_Y - MIN_Y + 1) as usize], air, generator::Biome::Plains)
                };
                stored.apply(&mut column, air);
                column
            }
            None => self.generator.generate(key.0, key.1),
        }
    }

    /// Chunk généré (calculé sur place s'il n'est pas prêt).
    fn column(&self, key: (i32, i32)) -> Arc<Column> {
        if let Some(c) = self.cached(key) {
            return c;
        }
        let column = {
            // (avec l'étape qui l'a demandé : c'est elle qu'il faut corriger)
            let _t = crate::timings::scope_with(|| match crate::timings::current() {
                Some(by) => format!("Chunk sur place ← {}", by),
                None => format!("Chunk sur place ← fil {}", std::thread::current().name().unwrap_or("sans nom")),
            });
            Arc::new(self.produce(key))
        };
        self.insert(key, column.clone());
        column
    }

    fn ready(&self, key: (i32, i32)) -> bool {
        self.chunks.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&key)
    }

    /// Demande un chunk aux fils de génération (sans attendre).
    fn request(self: &Arc<Self>, key: (i32, i32)) {
        if self.ready(key) {
            return;
        }
        if !self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(key) {
            return; // déjà demandé
        }
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.is_none() {
            *jobs = Some(Self::start_workers(self.clone()));
        }
        if let Some(tx) = jobs.as_ref() {
            let _ = tx.send(key);
        }
    }

    fn start_workers(terrain: Arc<Terrain>) -> Sender<(i32, i32)> {
        let (tx, rx) = mpsc::channel::<(i32, i32)>();
        let rx = Arc::new(Mutex::new(rx));
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).clamp(1, 4);
        for i in 0..threads {
            let (terrain, rx): (Arc<Terrain>, Arc<Mutex<Receiver<(i32, i32)>>>) = (terrain.clone(), rx.clone());
            let _ = std::thread::Builder::new().name(format!("terrain-{}", i)).spawn(move || {
                crate::utils::lower_thread_priority();
                loop {
                    let key = match rx.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                        Ok(k) => k,
                        Err(_) => return,
                    };
                    if !terrain.ready(key) {
                        let column = Arc::new(terrain.produce(key));
                        terrain.insert(key, column);
                    }
                    terrain.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                }
            });
        }
        tx
    }
}

/// Vue du monde pour un autre fil (les plugins) : lecture seule, figée au moment où elle a
/// été prise.
#[derive(Clone)]
pub struct WorldView {
    edits: Arc<HashMap<(i32, i32), Arc<ChunkEdits>>>,
    wet: Arc<WetMap>,
    terrain: Arc<Terrain>,
    air: u32,
}

impl WorldView {
    /// Le dessus d'un chunk déjà en mémoire : pour chacune de ses 256 colonnes (z * 16 + x),
    /// le bloc le plus haut qui n'est pas de l'air et sa hauteur. `None` si le chunk n'est ni
    /// généré ni modifié : rien n'est généré pour répondre.
    pub fn surface(&self, cx: i32, cz: i32) -> Option<Vec<(u32, i16)>> {
        let edits = self.edits.get(&(cx, cz));
        let column = match self.terrain.cached((cx, cz)) {
            Some(c) => c,
            None if edits.is_some() => self.terrain.column((cx, cz)),
            None => return None,
        };
        let mut out = Vec::with_capacity(256);
        for z in 0..16usize {
            for x in 0..16usize {
                let mut top = (self.air, (MIN_Y - 1) as i16);
                for y in (MIN_Y..=MAX_Y).rev() {
                    let block = match edits.and_then(|e| e.get(&(x as u8, y as i16, z as u8))) {
                        Some(b) => *b,
                        None => column.get(x, y, z),
                    };
                    if block != self.air {
                        top = (block, y as i16);
                        break;
                    }
                }
                out.push(top);
            }
        }
        Some(out)
    }

    /// Les blocs d'un chunk déjà en mémoire, du creux le plus bas de sa surface (moins
    /// `below` blocs) jusqu'à son sommet : (y du bas, hauteur, blocs rangés par colonne —
    /// `(z * 16 + x) * hauteur + (y - bas)`). `None` si le chunk n'est ni généré ni modifié.
    pub fn volume(&self, cx: i32, cz: i32, below: i32) -> Option<(i32, usize, Vec<u32>)> {
        let tops = self.surface(cx, cz)?;
        let lowest = tops.iter().map(|t| t.1 as i32).min().unwrap_or(MIN_Y);
        let highest = tops.iter().map(|t| t.1 as i32).max().unwrap_or(MIN_Y);
        let base = (lowest - below).max(MIN_Y);
        let height = (highest - base + 1).max(1) as usize;
        let edits = self.edits.get(&(cx, cz));
        let column = self.terrain.column((cx, cz));
        let mut out = vec![self.air; 256 * height];
        for z in 0..16usize {
            for x in 0..16usize {
                let top = tops[z * 16 + x].1 as i32;
                for y in base..=top {
                    out[(z * 16 + x) * height + (y - base) as usize] = match edits.and_then(|e| e.get(&(x as u8, y as i16, z as u8))) {
                        Some(b) => *b,
                        None => column.get(x, y, z),
                    };
                }
            }
        }
        Some((base, height, out))
    }

    /// Le générateur vanilla, pour les recherches (`/locate`) faites sur un autre fil.
    pub fn vanilla(&self) -> Option<&vanilla::Vanilla> {
        self.terrain.generator.vanilla()
    }

    /// Bloc présent à une position (identifiant réseau du bloc et de son état).
    pub fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        if !World::in_bounds(y) {
            return self.air;
        }
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        match self.edits.get(&(x >> 4, z >> 4)).and_then(|c| c.get(&key)) {
            Some(b) => *b,
            None => self.terrain.column((x >> 4, z >> 4)).get((x & 15) as usize, y, (z & 15) as usize),
        }
    }
}

/// Un enregistrement préparé par `World::plan_save`, à écrire (sur un autre fil si l'on veut).
pub enum SavePlan {
    /// Ancien format : tout le monde dans `level.nbt`
    Legacy(crate::nbt::Tag),
    /// Format du jeu : les chunks modifiés dans `db/`, et `level.dat`
    Db { store: Arc<mojang::Store>, batch: mojang::Batch, info: mojang::LevelInfo, legacy: bool, chunks: usize, pending: Vec<SaveChunk>, palette: Palette },
}

/// Un chunk à encoder pour la base du monde, au moment de l'écrire (sur le fil des écritures,
/// pas pendant le tick : un joueur qui court fait naître des centaines de chunks à
/// enregistrer, et les encoder figeait le serveur une seconde et plus).
pub struct SaveChunk {
    key: (i32, i32),
    column: Arc<Column>,
    edits: Option<Arc<ChunkEdits>>,
    wet: Option<Arc<HashSet<(u8, i16, u8)>>>,
}

impl SaveChunk {
    fn encode(&self, store: &mojang::Store, p: &Palette, out: &mut mojang::Batch) {
        let (column, edits, wet) = (&self.column, self.edits.as_deref(), self.wet.as_deref());
        let block = |x: usize, y: i32, z: usize| edits.and_then(|e| e.get(&(x as u8, y as i16, z as u8))).copied().unwrap_or_else(|| column.get(x, y, z));
        let holds_water = |x: usize, y: i32, z: usize| {
            let key = (x as u8, y as i16, z as u8);
            if wet.map(|w| w.contains(&key)).unwrap_or(false) || (column.wet.contains(&key) && !edits.map(|e| e.contains_key(&key)).unwrap_or(false)) {
                return true;
            }
            // Plantes aquatiques : toujours dans l'eau ; coraux : s'ils touchent de l'eau
            let b = block(x, y, z);
            p.aquatic.contains(&b)
                || (p.soaked.contains(&b)
                    && [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0), (0, -1, 0)].iter().any(|(dx, dy, dz)| {
                        let (nx, ny, nz) = (x as i32 + dx, y + dy, z as i32 + dz);
                        (0..16).contains(&nx) && (0..16).contains(&nz) && World::in_bounds(ny) && {
                            let n = block(nx as usize, ny, nz as usize);
                            n == p.water || p.aquatic.contains(&n)
                        }
                    }))
        };
        let biome = |s: usize, x: usize, y: usize, z: usize| match &column.biome_ids {
            Some(ids) => ids.get(s * 64 + (x / 4) * 16 + (z / 4) * 4 + y / 4).copied().unwrap_or(1) as u32,
            None => column.biome.id(),
        };
        store.encode(self.key.0, self.key.1, &block, &holds_water, &biome, out);
    }
}

impl SavePlan {
    /// Nombre de chunks que cet enregistrement écrit (0 pour l'ancien format).
    pub fn chunks(&self) -> usize {
        match self {
            SavePlan::Legacy(_) => 0,
            SavePlan::Db { chunks, .. } => *chunks,
        }
    }

    pub fn write(self, dir: &std::path::Path) -> std::io::Result<()> {
        match self {
            SavePlan::Legacy(doc) => World::write_snapshot(dir, &doc),
            SavePlan::Db { store, mut batch, info, legacy, pending, palette, .. } => {
                for chunk in &pending {
                    chunk.encode(&store, &palette, &mut batch);
                }
                if !batch.is_empty() {
                    store.write(batch)?;
                }
                mojang::write_level(dir, &info)?;
                if legacy {
                    // L'ancien fichier est gardé de côté, au cas où
                    let _ = std::fs::rename(dir.join("level.nbt"), dir.join("level.nbt.ancien"));
                }
                // (les coffres, fours et panneaux sont dans la base eux aussi)
                if dir.join("tiles.nbt").exists() {
                    let _ = std::fs::rename(dir.join("tiles.nbt"), dir.join("tiles.nbt.ancien"));
                }
                Ok(())
            }
        }
    }
}

/// Le monde (comme `World` dans PocketMine).
pub struct World {
    /// (chunk x, chunk z) -> (x local, y, z local) -> identifiant réseau du bloc
    edits: Arc<HashMap<(i32, i32), Arc<ChunkEdits>>>,
    /// Blocs posés dans l'eau qui contiennent encore cette eau (seconde couche)
    wet: Arc<WetMap>,
    count: usize,
    terrain: Arc<Terrain>,
    /// Version de chaque chunk modifié (voir `chunk_version`)
    versions: HashMap<(i32, i32), u64>,
    /// Modifié depuis le dernier enregistrement
    pub dirty: bool,
    /// Base du monde au format du jeu (absente sans les données du jeu : ancien `level.nbt`)
    store: Option<Arc<mojang::Store>>,
    /// Chunks modifiés depuis le dernier enregistrement dans la base
    unsaved: HashSet<(i32, i32)>,
    /// Un ancien `level.nbt` vient d'être relu : il sera retiré une fois la base écrite
    legacy: bool,
    /// Chunks qui ont des blocs à données dans la base, et l'empreinte de ce qui y est écrit
    tile_chunks: HashMap<(i32, i32), u32>,
    /// Blocs à données du jeu que Basalt ne gère pas (bannières, têtes, générateurs…) : gardés
    /// tels quels, pour être réécrits avec les autres
    foreign_tiles: HashMap<(i32, i32), Vec<u8>>,
    /// Données attachées aux blocs : contenu des coffres, état des fours, texte des panneaux
    pub tiles: crate::tile::TileMap,
    pub air: u32,
    #[cfg(test)]
    stone: u32,
    #[cfg(test)]
    grass: u32,
}

impl Default for World {
    fn default() -> World {
        World::new()
    }
}

impl World {
    /// Monde plat (blocs sans données du jeu) : le terrain des tests et des anciens mondes.
    pub fn new() -> World {
        World::with_generator(Generator::new(Kind::Flat, 0, Palette::new(&|_| None)))
    }

    pub fn with_generator(generator: Generator) -> World {
        let air = generator.blocks.air;
        World {
            edits: Arc::new(HashMap::new()),
            wet: Arc::new(HashMap::new()),
            count: 0,
            terrain: Terrain::new(generator),
            versions: HashMap::new(),
            dirty: false,
            store: None,
            unsaved: HashSet::new(),
            legacy: false,
            tile_chunks: HashMap::new(),
            foreign_tiles: HashMap::new(),
            tiles: crate::tile::TileMap::new(),
            air,
            #[cfg(test)]
            stone: block_hash("minecraft:stone"),
            #[cfg(test)]
            grass: block_hash("minecraft:grass_block"),
        }
    }

    pub fn generator(&self) -> &Generator {
        &self.terrain.generator
    }

    /// Ce qui reste quand ce bloc disparaît : de l'eau pour une plante aquatique ou un corail
    /// immergé (le bloc contenait de l'eau), de l'air sinon.
    pub fn left_behind(&self, x: i32, y: i32, z: i32) -> u32 {
        let p = &self.terrain.generator.blocks;
        let b = self.get(x, y, z);
        let wet = self.is_wet(x, y, z)
            || p.aquatic.contains(&b)
            || (p.soaked.contains(&b)
                && [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0)].iter().any(|(dx, dy, dz)| {
                    let n = self.get(x + dx, y + dy, z + dz);
                    n == p.water || p.aquatic.contains(&n)
                }));
        if wet {
            p.water
        } else {
            self.air
        }
    }

    /// Butin d'un coffre de structure (village, temple, mine) encore à sa place d'origine.
    pub fn structure_chest(&self, (x, y, z): (i32, i32, i32)) -> Option<structures::Loot> {
        if !Self::in_bounds(y) {
            return None;
        }
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        if self.edits.get(&(x >> 4, z >> 4)).map(|c| c.contains_key(&key)).unwrap_or(false) {
            return None; // remplacé par un joueur
        }
        let column = self.terrain.column((x >> 4, z >> 4));
        column.chests.iter().find(|c| (c.0, c.1, c.2) == key).map(|c| c.3)
    }

    /// Le bloc posé ici contient-il de l'eau (posé dans l'eau) ?
    pub fn is_wet(&self, x: i32, y: i32, z: i32) -> bool {
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        if self.wet.get(&(x >> 4, z >> 4)).map(|c| c.contains(&key)).unwrap_or(false) {
            return true;
        }
        // Bloc d'une structure posé dans l'eau, que personne n'a remplacé
        let chunk = (x >> 4, z >> 4);
        !self.edits.get(&chunk).map(|c| c.contains_key(&key)).unwrap_or(false) && self.terrain.ready(chunk) && self.terrain.column(chunk).wet.contains(&key)
    }

    /// Le bloc garde (ou perd) l'eau dans laquelle il a été posé.
    pub fn set_wet(&mut self, x: i32, y: i32, z: i32, wet: bool) {
        if !Self::in_bounds(y) || self.is_wet(x, y, z) == wet {
            return;
        }
        let chunk = (x >> 4, z >> 4);
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        let all = Arc::make_mut(&mut self.wet);
        if wet {
            Arc::make_mut(all.entry(chunk).or_default()).insert(key);
        } else if let Some(c) = all.get_mut(&chunk) {
            Arc::make_mut(c).remove(&key);
            if c.is_empty() {
                all.remove(&chunk);
            }
        }
        self.dirty = true;
        self.bump(chunk);
    }

    /// Bloc du terrain d'origine à cette position.
    fn base(&self, x: i32, y: i32, z: i32) -> u32 {
        self.terrain.column((x >> 4, z >> 4)).get((x & 15) as usize, y, (z & 15) as usize)
    }

    pub fn in_bounds(y: i32) -> bool {
        (MIN_Y..=MAX_Y).contains(&y)
    }

    /// Bloc présent à une position.
    pub fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        if !Self::in_bounds(y) {
            return self.air;
        }
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        match self.edits.get(&(x >> 4, z >> 4)).and_then(|c| c.get(&key)) {
            Some(b) => *b,
            None => self.base(x, y, z),
        }
    }

    /// Hauteur du bloc plein le plus haut d'une colonne (feuilles et plantes exclues si
    /// `solid` le dit), ou `MIN_Y - 1` si la colonne est vide.
    pub fn highest(&self, x: i32, z: i32, solid: &dyn Fn(u32) -> bool) -> i32 {
        for y in (MIN_Y..=MAX_Y).rev() {
            if solid(self.get(x, y, z)) {
                return y;
            }
        }
        MIN_Y - 1
    }

    /// Change un bloc. Renvoie faux si la position est hors du monde ou si la limite est atteinte.
    pub fn set(&mut self, x: i32, y: i32, z: i32, block: u32) -> bool {
        if !Self::in_bounds(y) {
            return false;
        }
        if block == self.air || block == self.terrain.generator.blocks.water {
            self.set_wet(x, y, z, false);
        }
        let chunk = (x >> 4, z >> 4);
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        // (avec une base, le terrain d'un chunk enregistré contient déjà les modifications : une
        // modification n'est alors jamais retirée, elle remplace ce qui est enregistré)
        if self.store.is_none() && block == self.base(x, y, z) {
            // retour à l'état d'origine : rien à garder
            if self.edits.get(&chunk).map(|c| c.contains_key(&key)).unwrap_or(false) {
                let all = Arc::make_mut(&mut self.edits);
                if let Some(c) = all.get_mut(&chunk) {
                    let c = Arc::make_mut(c);
                    c.remove(&key);
                    self.count -= 1;
                    self.dirty = true;
                    *self.versions.entry(chunk).or_insert(0) += 1;
                    if c.is_empty() {
                        all.remove(&chunk);
                    }
                }
            }
            return true;
        }
        self.store(chunk, key, block)
    }

    /// Garde une modification telle quelle (sans la comparer au terrain).
    fn store(&mut self, chunk: (i32, i32), key: (u8, i16, u8), block: u32) -> bool {
        let known = self.edits.get(&chunk).map(|c| c.contains_key(&key)).unwrap_or(false);
        if !known {
            if self.count >= MAX_EDITS {
                return false;
            }
            self.count += 1;
        }
        if self.edits.get(&chunk).and_then(|c| c.get(&key)) == Some(&block) {
            return true;
        }
        let all = Arc::make_mut(&mut self.edits);
        Arc::make_mut(all.entry(chunk).or_default()).insert(key, block);
        self.dirty = true;
        self.bump(chunk);
        true
    }

    /// Vue du monde pour un autre fil (plugins) : copie instantanée, sans recopie des blocs.
    pub fn view(&self) -> WorldView {
        WorldView { edits: self.edits.clone(), wet: self.wet.clone(), terrain: self.terrain.clone(), air: self.air }
    }

    /// Nombre de blocs qui diffèrent du terrain d'origine.
    pub fn edit_count(&self) -> usize {
        self.count
    }

    /// Demande la génération d'un chunk en arrière-plan (sans attendre).
    pub fn prepare(&self, cx: i32, cz: i32) {
        self.terrain.request((cx, cz));
    }

    /// Cloches posées par une structure dans ce chunk déjà généré (le cœur d'un village).
    pub fn structure_bells(&self, cx: i32, cz: i32) -> Vec<(i32, i32, i32)> {
        self.terrain.cached((cx, cz)).map(|c| c.bells.iter().map(|&(x, y, z)| (cx * 16 + x as i32, y as i32, cz * 16 + z as i32)).collect()).unwrap_or_default()
    }

    /// Lits posés par une structure dans ce chunk déjà généré (un village) : position dans le
    /// monde. Vide si le chunk n'est pas prêt.
    pub fn structure_beds(&self, cx: i32, cz: i32) -> Vec<(i32, i32, i32)> {
        self.terrain.cached((cx, cz)).map(|c| c.beds.iter().map(|&(x, y, z, _)| (cx * 16 + x as i32, y as i32, cz * 16 + z as i32)).collect()).unwrap_or_default()
    }

    /// Le bloc, seulement si son chunk est déjà en mémoire : jamais de génération sur place.
    /// Pour les vérifications faites à chaque mouvement d'un joueur, qui ne doivent pas
    /// figer la boucle du jeu le temps de générer un chunk.
    pub fn get_loaded(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        if !World::in_bounds(y) {
            return Some(self.air);
        }
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        if let Some(b) = self.edits.get(&(x >> 4, z >> 4)).and_then(|c| c.get(&key)) {
            return Some(*b);
        }
        self.terrain.cached((x >> 4, z >> 4)).map(|c| c.get((x & 15) as usize, y, (z & 15) as usize))
    }

    /// Chunks que les joueurs ont autour d'eux : le cache en garde au moins autant (plus une
    /// marge), jusqu'à 65 536 (2,5 Go environ). Avec 500 joueurs dispersés, 8 192 chunks ne
    /// suffisaient pas : ceux qu'un joueur voyait sortaient du cache et étaient régénérés sur
    /// place (70 ms chacun) dès qu'un objet, une créature ou une pousse les touchait.
    pub fn keep_chunks(&self, wanted: usize) {
        self.terrain.limit.store((wanted + wanted / 4).clamp(CACHE_CHUNKS, 65_536), Ordering::Relaxed);
    }

    /// Le chunk est-il généré (prêt à être envoyé sans attendre) ?
    pub fn is_ready(&self, cx: i32, cz: i32) -> bool {
        self.terrain.ready((cx, cz))
    }

    /// Rattache le monde à sa base (`db/`, au format du jeu). À faire avant de s'en servir.
    pub fn attach_store(&mut self, store: Arc<mojang::Store>) {
        let _ = self.terrain.store.set(store.clone());
        self.store = Some(store);
    }

    /// Note qu'un chunk a des blocs à données dans la base (lus à l'ouverture du monde).
    pub fn note_tile_chunk(&mut self, chunk: (i32, i32)) {
        self.tile_chunks.entry(chunk).or_insert(0);
    }

    /// Garde tel quel un bloc à données du jeu que Basalt ne sait pas gérer.
    pub fn keep_foreign_tile(&mut self, chunk: (i32, i32), entry: &crate::nbt::Tag) {
        self.foreign_tiles.entry(chunk).or_default().extend_from_slice(&crate::nbt::write_le(entry));
    }

    /// Ajoute à un enregistrement les blocs à données (coffres, fours, panneaux…) : seuls les
    /// chunks dont le contenu a changé sont réécrits, et ceux qui n'en ont plus sont effacés.
    pub fn plan_tiles(&mut self, mut chunks: HashMap<(i32, i32), Vec<u8>>, plan: &mut SavePlan) {
        let SavePlan::Db { batch, .. } = plan else { return };
        for (chunk, raw) in &self.foreign_tiles {
            chunks.entry(*chunk).or_default().extend_from_slice(raw);
        }
        let gone: Vec<(i32, i32)> = self.tile_chunks.keys().filter(|c| !chunks.contains_key(c)).copied().collect();
        for chunk in gone {
            self.tile_chunks.remove(&chunk);
            mojang::Store::encode_block_entities(chunk.0, chunk.1, None, batch);
        }
        for (chunk, data) in chunks {
            // (zéro est réservé à « lu dans la base, contenu pas encore comparé »)
            let print = mojang::fnv1a(&data).max(1);
            if self.tile_chunks.insert(chunk, print) != Some(print) {
                mojang::Store::encode_block_entities(chunk.0, chunk.1, Some(data), batch);
            }
        }
    }

    /// La base du monde, si elle est ouverte.
    pub fn db_store(&self) -> Option<&Arc<mojang::Store>> {
        self.store.as_ref()
    }

    /// Enregistre le monde tout de suite (création d'un monde, tests). Le serveur, lui,
    /// prépare l'enregistrement avec `plan_save` et l'écrit sur un autre fil.
    pub fn save(&mut self, dir: &std::path::Path) -> std::io::Result<()> {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.plan_save(mojang::LevelInfo { name, ..Default::default() }).write(dir)
    }

    /// Prépare l'enregistrement (la partie rapide, faite dans la boucle principale) ; le monde
    /// est ensuite considéré comme enregistré. Avec une base : les chunks modifiés depuis la
    /// dernière fois, entiers, et `level.dat`. Sans : l'ancien `level.nbt`.
    pub fn plan_save(&mut self, mut info: mojang::LevelInfo) -> SavePlan {
        let Some(store) = self.store.clone() else { return SavePlan::Legacy(self.snapshot()) };
        self.dirty = false;
        info.generator = self.generator().stored_name();
        info.seed = self.generator().seed;
        let batch = mojang::Batch::new();
        let unsaved = std::mem::take(&mut self.unsaved);
        let pending: Vec<SaveChunk> = unsaved
            .into_iter()
            .map(|chunk| SaveChunk { key: chunk, column: self.terrain.column(chunk), edits: self.edits.get(&chunk).cloned(), wet: self.wet.get(&chunk).cloned() })
            .collect();
        let chunks = pending.len();
        let palette = self.terrain.generator.blocks.clone();
        SavePlan::Db { store, batch, info, legacy: std::mem::take(&mut self.legacy), chunks, pending, palette }
    }

    /// Document à enregistrer (la partie rapide de l'enregistrement, faite dans la boucle
    /// principale) ; le monde est ensuite considéré comme enregistré.
    pub fn snapshot(&mut self) -> crate::nbt::Tag {
        use crate::nbt::Tag;
        self.dirty = false;
        let mut blocks = Vec::with_capacity(self.count * 4);
        for ((cx, cz), edits) in self.edits.iter() {
            for ((x, y, z), block) in edits.iter() {
                blocks.extend_from_slice(&[cx * 16 + *x as i32, *y as i32, cz * 16 + *z as i32, *block as i32]);
            }
        }
        let mut wet = Vec::new();
        for ((cx, cz), cells) in self.wet.iter() {
            for (x, y, z) in cells.iter() {
                wet.extend_from_slice(&[cx * 16 + *x as i32, *y as i32, cz * 16 + *z as i32]);
            }
        }
        let g = self.generator();
        Tag::compound(vec![
            ("Version", Tag::Int(1)),
            ("Generator", Tag::string(&g.stored_name())),
            ("Seed", Tag::Long(g.seed as i64)),
            ("Blocks", Tag::IntArray(blocks)),
            ("Waterlogged", Tag::IntArray(wet)),
        ])
    }

    /// Écrit un document préparé par `snapshot` (compression et disque : la partie lente,
    /// qui peut se faire sur un autre fil).
    pub fn write_snapshot(dir: &std::path::Path, doc: &crate::nbt::Tag) -> std::io::Result<()> {
        crate::nbt::save_file(&dir.join("level.nbt"), doc)?;
        // L'ancien format n'a plus lieu d'être une fois le nouveau fichier écrit
        let _ = std::fs::remove_file(dir.join("blocs.bin"));
        Ok(())
    }

    /// Type de terrain et graine d'un monde déjà créé, s'il y en a un dans ce dossier. Un
    /// monde enregistré avant la génération de terrain est plat.
    pub fn stored_generator(dir: &std::path::Path) -> Result<Option<(Kind, u64)>, String> {
        if let Some(level) = mojang::read_level(dir)? {
            let (kind, seed, _) = mojang::level_generator(&level);
            return Ok(Some((kind, seed)));
        }
        match crate::nbt::load_file(&dir.join("level.nbt"))? {
            Some(doc) => {
                let kind = doc.get("Generator").and_then(|t| t.as_str()).map(Kind::parse).unwrap_or(Kind::Flat);
                let seed = doc.get("Seed").and_then(|t| t.as_i64()).unwrap_or(0) as u64;
                Ok(Some((kind, seed)))
            }
            None if dir.join("blocs.bin").exists() => Ok(Some((Kind::Flat, 0))),
            None => Ok(None),
        }
    }

    /// Vrai si ce monde a été créé avec l'ancien terrain propre à Basalt, retiré depuis : il
    /// est repris avec le terrain du jeu.
    pub fn stored_was_legacy(dir: &std::path::Path) -> bool {
        !dir.join("level.dat").exists() && matches!(crate::nbt::load_file(&dir.join("level.nbt")), Ok(Some(doc)) if doc.get("Generator").and_then(|t| t.as_str()).map(Kind::was_legacy).unwrap_or(false))
    }

    /// Nom du générateur d'un plugin enregistré pour ce monde (`plugin:<nom>`), s'il y en a un.
    pub fn stored_plugin(dir: &std::path::Path) -> Option<String> {
        if let Ok(Some(level)) = mojang::read_level(dir) {
            return mojang::level_generator(&level).2;
        }
        let doc = crate::nbt::load_file(&dir.join("level.nbt")).ok()??;
        let text = doc.get("Generator")?.as_str()?.to_string();
        text.strip_prefix("plugin:").map(|s| s.to_string())
    }

    /// Recharge les blocs enregistrés. Renvoie leur nombre (0 si aucun fichier). Avec une
    /// base, il n'y a rien à recharger : les chunks enregistrés sont lus à la demande ; seul
    /// un ancien `level.nbt` est repris, pour passer dans la base au prochain enregistrement.
    pub fn load(&mut self, dir: &std::path::Path) -> Result<usize, String> {
        match crate::nbt::load_file(&dir.join("level.nbt"))? {
            Some(doc) => {
                self.legacy = self.store.is_some();
                let blocks = doc.get("Blocks").and_then(|t| t.as_int_array()).ok_or("level.nbt : liste « Blocks » absente")?;
                if blocks.len() % 4 != 0 {
                    return Err("level.nbt : liste « Blocks » tronquée".into());
                }
                for b in blocks.chunks_exact(4) {
                    if Self::in_bounds(b[1]) {
                        self.store((b[0] >> 4, b[2] >> 4), ((b[0] & 15) as u8, b[1] as i16, (b[2] & 15) as u8), b[3] as u32);
                    }
                }
                if let Some(wet) = doc.get("Waterlogged").and_then(|t| t.as_int_array()) {
                    for w in wet.chunks_exact(3) {
                        self.set_wet(w[0], w[1], w[2], true);
                    }
                }
                self.dirty = self.legacy;
                Ok(self.count)
            }
            None => self.load_legacy(dir),
        }
    }

    /// Ancien format (`blocs.bin`) : relu une dernière fois, puis remplacé par `level.nbt`
    /// au prochain enregistrement.
    fn load_legacy(&mut self, dir: &std::path::Path) -> Result<usize, String> {
        let data = match std::fs::read(dir.join("blocs.bin")) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.to_string()),
        };
        if data.len() < 16 || &data[..8] != b"BASALTW1" {
            return Err("fichier du monde non reconnu".into());
        }
        let count = u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8])) as usize;
        if data.len() != 16 + count.saturating_mul(16) {
            return Err("fichier du monde tronqué".into());
        }
        for e in data[16..].chunks_exact(16) {
            let v = |i: usize| i32::from_le_bytes([e[i], e[i + 1], e[i + 2], e[i + 3]]);
            self.set(v(0), v(4), v(8), v(12) as u32);
        }
        // Reste « modifié » : le prochain enregistrement écrit level.nbt
        self.dirty = self.count > 0;
        Ok(self.count)
    }

    pub fn is_edited(&self, cx: i32, cz: i32) -> bool {
        self.edits.contains_key(&(cx, cz))
    }

    /// Contenu d'un chunk à envoyer : (nombre de sous-chunks, données). Les sous-chunks d'air
    /// au-dessus du dernier bloc ne sont pas envoyés.
    pub fn chunk_payload(&self, cx: i32, cz: i32) -> (u32, Vec<u8>) {
        encode_chunk(&self.terrain, &self.edits, &self.wet, self.air, cx, cz)
    }

    /// Numéro de version d'un chunk : il change à chaque bloc modifié dedans. Sert à savoir si
    /// un chunk encodé ailleurs (sur un autre fil) est encore à jour.
    pub fn chunk_version(&self, cx: i32, cz: i32) -> u64 {
        self.versions.get(&(cx, cz)).copied().unwrap_or(0)
    }

    fn bump(&mut self, chunk: (i32, i32)) {
        if self.store.is_some() {
            self.unsaved.insert(chunk);
        }
        *self.versions.entry(chunk).or_insert(0) += 1;
    }
}

impl WorldView {
    /// Dimension annoncée au jeu pour ce monde (0 surface, 1 Nether, 2 End).
    pub fn dimension(&self) -> i32 {
        self.terrain.generator.kind.dimension()
    }

    /// Contenu d'un chunk à envoyer, encodé depuis un autre fil (même résultat que `World`).
    pub fn chunk_payload(&self, cx: i32, cz: i32) -> (u32, Vec<u8>) {
        encode_chunk(&self.terrain, &self.edits, &self.wet, self.air, cx, cz)
    }
}

/// Contenu d'un chunk : terrain généré et blocs modifiés.
fn encode_chunk(terrain: &Terrain, all_edits: &HashMap<(i32, i32), Arc<ChunkEdits>>, all_wet: &WetMap, air: u32, cx: i32, cz: i32) -> (u32, Vec<u8>) {
    {
        let column = terrain.column((cx, cz));
        let edits = all_edits.get(&(cx, cz));
        let wet = all_wet.get(&(cx, cz));
        // Sous-chunks à envoyer : jusqu'au plus haut qui n'est pas que de l'air
        // (seulement les tranches que cette dimension connaît : le Nether et l'End commencent à 0)
        let span = terrain.generator.kind.sections();
        let mut sections = column.sections.iter().rposition(|s| !matches!(s, Section::Uniform(b) if *b == air)).map(|i| i + 1).unwrap_or(1);
        if let Some(edits) = edits {
            for ((_, y, _), block) in edits.iter() {
                if *block != air {
                    sections = sections.max(((*y as i32 - MIN_Y) / 16 + 1) as usize);
                }
            }
        }
        let sections = sections.clamp(span.start + 1, span.end);
        let mut w = W::default();
        for s in span.start..sections {
            let y_index = ((MIN_Y >> 4) + s as i32) as i8;
            let base_y = MIN_Y + s as i32 * 16;
            let in_section = |y: i16| (y as i32) >= base_y && (y as i32) < base_y + 16;
            let touched = edits.map(|e| e.keys().any(|(_, y, _)| in_section(*y))).unwrap_or(false)
                || wet.map(|w| w.iter().any(|(_, y, _)| in_section(*y))).unwrap_or(false)
                || column.wet.iter().any(|(_, y, _)| in_section(*y));
            let p = &terrain.generator.blocks;
            match (&column.sections[s], touched) {
                (Section::Mixed { palette, index }, false) if !palette.iter().any(|b| p.aquatic.contains(b) || p.soaked.contains(b)) => {
                    let wide: Vec<u16> = index.iter().map(|i| *i as u16).collect();
                    encode_sub_chunk(&mut w, y_index, palette, &wide, None);
                }
                (section, _) => {
                    let mut palette: Vec<u32> = Vec::new();
                    let mut indices = vec![0u16; 4096];
                    for x in 0..16usize {
                        for z in 0..16usize {
                            for dy in 0..16usize {
                                let y = base_y + dy as i32;
                                let block = edits.and_then(|e| e.get(&(x as u8, y as i16, z as u8))).copied().unwrap_or_else(|| section.get(x, dy, z));
                                let idx = match palette.iter().position(|p| *p == block) {
                                    Some(i) => i,
                                    None => {
                                        palette.push(block);
                                        palette.len() - 1
                                    }
                                };
                                indices[(x << 8) | (z << 4) | dy] = idx as u16;
                            }
                        }
                    }
                    let mut water = waterlogged(p, &palette, &indices);
                    // Blocs posés dans l'eau par les joueurs
                    for (x, y, z) in wet.iter().flat_map(|w| w.iter()).filter(|(_, y, _)| in_section(*y)) {
                        water.get_or_insert_with(|| vec![false; 4096])[((*x as usize) << 8) | ((*z as usize) << 4) | (*y as i32 - base_y) as usize] = true;
                    }
                    // Blocs des structures posés dans l'eau, tant que personne n'y a touché
                    for (x, y, z) in column.wet.iter().filter(|(_, y, _)| in_section(*y)) {
                        if edits.map(|e| e.contains_key(&(*x, *y, *z))).unwrap_or(false) {
                            continue;
                        }
                        water.get_or_insert_with(|| vec![false; 4096])[((*x as usize) << 8) | ((*z as usize) << 4) | (*y as i32 - base_y) as usize] = true;
                    }
                    encode_sub_chunk(&mut w, y_index, &palette, &indices, water.as_ref().map(|wl| (wl, p.air, p.water)));
                }
            }
        }
        let biome = column.biome.id();
        for s in span.clone() {
            match column.biome_ids.as_ref().map(|ids| &ids[s * 64..s * 64 + 64]) {
                Some(quarts) if quarts.iter().any(|b| *b != quarts[0]) => encode_biomes(&mut w, quarts),
                Some(quarts) => {
                    w.u8(1); // biome unique pour la section
                    w.vari32(quarts[0] as i32);
                }
                None => {
                    w.u8(1);
                    w.vari32(biome as i32);
                }
            }
        }
        w.u8(0); // blocs de bordure
        // Données des blocs (à la suite, au format NBT réseau) : la couleur des lits des
        // structures, tant que le joueur n'a pas changé le bloc
        for &(x, y, z, color) in &column.beds {
            if edits.map(|e| e.contains_key(&(x, y, z))).unwrap_or(false) {
                continue;
            }
            let tag = crate::nbt::Tag::compound(vec![
                ("id", crate::nbt::Tag::string("Bed")),
                ("x", crate::nbt::Tag::Int(cx * 16 + x as i32)),
                ("y", crate::nbt::Tag::Int(y as i32)),
                ("z", crate::nbt::Tag::Int(cz * 16 + z as i32)),
                ("color", crate::nbt::Tag::Byte(color as i8)),
                ("isMovable", crate::nbt::Tag::Byte(1)),
            ]);
            w.0.extend_from_slice(&crate::nbt::write_network(&tag));
        }
        // Les cloches : le jeu ne les dessine et ne les fait sonner qu'avec leurs données
        for &(x, y, z) in &column.bells {
            if edits.map(|e| e.contains_key(&(x, y, z))).unwrap_or(false) {
                continue;
            }
            let tag = crate::nbt::Tag::compound(vec![
                ("id", crate::nbt::Tag::string("Bell")),
                ("x", crate::nbt::Tag::Int(cx * 16 + x as i32)),
                ("y", crate::nbt::Tag::Int(y as i32)),
                ("z", crate::nbt::Tag::Int(cz * 16 + z as i32)),
                ("Ringing", crate::nbt::Tag::Byte(0)),
                ("Direction", crate::nbt::Tag::Int(0)),
                ("Ticks", crate::nbt::Tag::Int(0)),
                ("isMovable", crate::nbt::Tag::Byte(1)),
            ]);
            w.0.extend_from_slice(&crate::nbt::write_network(&tag));
        }
        ((sections - span.start) as u32, w.0)
    }
}

/// Biomes d'un sous-chunk, bloc par bloc (x << 8 | z << 4 | y), d'après les cases de 4×4×4
/// blocs (`quarts` : x * 16 + z * 4 + y).
fn encode_biomes(w: &mut W, quarts: &[u8]) {
    let mut palette: Vec<u8> = Vec::new();
    for b in quarts {
        if !palette.contains(b) {
            palette.push(*b);
        }
    }
    let bits: usize = [1usize, 2, 3, 4, 5, 6, 8].iter().copied().find(|b| (1usize << b) >= palette.len()).unwrap_or(8);
    w.u8(((bits as u8) << 1) | 1);
    let per_word = 32 / bits;
    let (mut word, mut filled) = (0u32, 0);
    for x in 0..16usize {
        for z in 0..16usize {
            for y in 0..16usize {
                let b = quarts[(x / 4) * 16 + (z / 4) * 4 + y / 4];
                let idx = palette.iter().position(|p| *p == b).unwrap_or(0) as u32;
                word |= idx << (bits * filled);
                filled += 1;
                if filled == per_word {
                    w.u32(word);
                    word = 0;
                    filled = 0;
                }
            }
        }
    }
    if filled > 0 {
        w.u32(word);
    }
    w.vari32(palette.len() as i32);
    for b in palette {
        w.vari32(b as i32);
    }
}

/// Encode un sous-chunk au format 9 : une couche de blocs avec sa palette.
/// Blocs d'un sous-chunk qui contiennent aussi de l'eau (seconde couche) : plantes
/// aquatiques, et coraux ou cornichons de mer qui touchent de l'eau.
fn waterlogged(p: &crate::world::generator::Palette, palette: &[u32], indices: &[u16]) -> Option<Vec<bool>> {
    if !palette.iter().any(|b| p.aquatic.contains(b) || p.soaked.contains(b)) {
        return None;
    }
    let block = |x: usize, z: usize, y: usize| palette[indices[(x << 8) | (z << 4) | y] as usize];
    let mut out = vec![false; 4096];
    for x in 0..16usize {
        for z in 0..16usize {
            for y in 0..16usize {
                let b = block(x, z, y);
                out[(x << 8) | (z << 4) | y] = if p.aquatic.contains(&b) {
                    true
                } else if p.soaked.contains(&b) {
                    let near = [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0), (0, -1, 0)];
                    near.iter().any(|(dx, dz, dy)| {
                        let (nx, nz, ny) = (x as i32 + dx, z as i32 + dz, y as i32 + dy);
                        (0..16).contains(&nx) && (0..16).contains(&nz) && (0..16).contains(&ny) && {
                            let n = block(nx as usize, nz as usize, ny as usize);
                            n == p.water || p.aquatic.contains(&n)
                        }
                    })
                } else {
                    false
                };
            }
        }
    }
    Some(out)
}

fn encode_sub_chunk(w: &mut W, y_index: i8, palette: &[u32], indices: &[u16], water: Option<(&Vec<bool>, u32, u32)>) {
    w.u8(9);
    w.u8(if water.is_some() { 2 } else { 1 });
    w.u8(y_index as u8);
    write_storage(w, palette, indices);
    if let Some((cells, air, water)) = water {
        let second: Vec<u16> = cells.iter().map(|c| *c as u16).collect();
        write_storage(w, &[air, water], &second);
    }
}

/// Une couche de blocs : indices empaquetés dans des mots de 32 bits, puis la palette.
fn write_storage(w: &mut W, palette: &[u32], indices: &[u16]) {
    let bits: usize = [1usize, 2, 3, 4, 5, 6, 8, 16]
        .iter()
        .copied()
        .find(|b| (1usize << b) >= palette.len())
        .unwrap_or(16);
    w.u8(((bits as u8) << 1) | 1);
    let per_word = 32 / bits;
    let mut word: u32 = 0;
    let mut filled = 0;
    for idx in indices {
        word |= (*idx as u32) << (bits * filled);
        filled += 1;
        if filled == per_word {
            w.u32(word);
            word = 0;
            filled = 0;
        }
    }
    if filled > 0 {
        w.u32(word);
    }
    w.vari32(palette.len() as i32);
    for block in palette {
        w.vari32(*block as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une herbe marine est envoyée avec de l'eau dans le même bloc (seconde couche), et
    /// la casser laisse de l'eau, pas un trou d'air.
    #[test]
    fn sea_plants_carry_their_water() {
        let mut world = World::new();
        let p = &world.generator().blocks;
        let (seagrass, water) = (p.named["seagrass"], p.water);
        // Section du bas (y de -64 à -49) : un seul stockage tant qu'il n'y a pas de plante
        assert_eq!(&world.chunk_payload(0, 0).1[..2], &[9, 1]);
        assert!(world.set(2, -60, 3, seagrass));
        let (_, bytes) = world.chunk_payload(0, 0);
        assert_eq!(&bytes[..2], &[9, 2]);
        assert_eq!(world.left_behind(2, -60, 3), water);
        assert_eq!(world.left_behind(5, -61, 5), world.air);
    }

    /// Une dalle posée dans l'eau la garde : envoyée avec sa seconde couche, conservée à
    /// l'enregistrement, et rendue en eau quand on la casse.
    #[test]
    fn blocks_placed_in_water_keep_it() {
        let dir = std::env::temp_dir().join(format!("basalt-wet-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut world = World::new();
        let slab = block_hash("minecraft:oak_slab");
        let water = world.generator().blocks.water;
        assert!(world.set(4, -60, 4, slab));
        world.set_wet(4, -60, 4, true);
        assert_eq!(&world.chunk_payload(0, 0).1[..2], &[9, 2]);
        world.save(&dir).unwrap();
        let mut again = World::new();
        again.load(&dir).unwrap();
        assert!(again.is_wet(4, -60, 4));
        assert_eq!(again.left_behind(4, -60, 4), water);
        // Remplacé par de l'eau (cassé) : plus rien dans la seconde couche
        assert!(again.set(4, -60, 4, water));
        assert!(!again.is_wet(4, -60, 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_are_tracked_and_reverted() {
        let mut world = World::new();
        assert_eq!(world.get(3, -61, 4), world.grass);
        assert!(world.set(3, -61, 4, world.air));
        assert_eq!(world.get(3, -61, 4), world.air);
        assert!(world.is_edited(0, 0));
        // remettre le bloc d'origine efface la modification
        assert!(world.set(3, -61, 4, world.grass));
        assert!(!world.is_edited(0, 0));
        assert!(!world.set(0, 400, 0, world.stone));
    }

    #[test]
    fn flat_chunk_payload_has_one_section_until_someone_builds() {
        let mut world = World::new();
        let (sections, payload) = world.chunk_payload(4, -2);
        assert_eq!(sections, 1);
        // Sous-chunk au format 9, une couche, hauteur -4 ; 2 bits par bloc (4 blocs différents)
        assert_eq!(&payload[..4], &[9, 1, (-4i8) as u8, (2 << 1) | 1]);
        // Un bloc posé plus haut ajoute un sous-chunk
        assert!(world.set(-1, -40, -1, world.stone));
        let (sections, again) = world.chunk_payload(-1, -1);
        assert_eq!(sections, 2);
        assert_eq!(again[..4 + 1024], payload[..4 + 1024], "premier sous-chunk inchangé");
    }

    #[test]
    fn generated_world_keeps_edits_and_its_seed() {
        use generator::{Generator, Kind, Palette};
        let dir = std::env::temp_dir().join(format!("basalt-world-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = World::with_generator(Generator::new(Kind::Vanilla, 777, Palette::new(&|_| None)));
        let top = w.generator().surface(5, 5);
        let ground = w.get(5, top, 5);
        assert_ne!(ground, w.air);
        // Creuser puis reboucher : plus rien à garder
        assert!(w.set(5, top, 5, w.air));
        assert_eq!(w.edit_count(), 1);
        assert!(w.set(5, top, 5, ground));
        assert_eq!(w.edit_count(), 0);
        assert!(w.set(5, top + 1, 5, w.stone));
        w.save(&dir).unwrap();
        assert_eq!(World::stored_generator(&dir).unwrap(), Some((Kind::Vanilla, 777)));
        let mut again = World::with_generator(Generator::new(Kind::Vanilla, 777, Palette::new(&|_| None)));
        assert_eq!(again.load(&dir).unwrap(), 1);
        assert_eq!(again.get(5, top + 1, 5), w.stone);
        assert_eq!(again.get(5, top, 5), ground);
        // Les fils de génération préparent un chunk demandé
        again.prepare(40, 40);
        let start = std::time::Instant::now();
        while !again.is_ready(40, 40) && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(again.is_ready(40, 40));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn negative_coordinates_use_floor_division() {
        let mut world = World::new();
        assert!(world.set(-1, -61, -17, world.air));
        assert!(world.is_edited(-1, -2));
        assert_eq!(world.get(-1, -61, -17), world.air);
        assert_eq!(world.get(-1, -61, -16), world.grass);
    }

    #[test]
    fn world_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("basalt-world-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = World::new();
        assert_eq!(w.load(&dir).unwrap(), 0);
        let stone = block_hash("minecraft:stone");
        w.set(-17, 10, 33, stone);
        w.set(5, -61, 5, w.air);
        assert!(w.dirty);
        w.save(&dir).unwrap();
        assert!(!w.dirty);
        let mut again = World::new();
        assert_eq!(again.load(&dir).unwrap(), 2);
        assert_eq!(again.get(-17, 10, 33), stone);
        assert_eq!(again.get(5, -61, 5), again.air);
        assert_eq!(again.get(6, -61, 5), w.get(6, -61, 5));
        // Ancien format : relu, puis remplacé
        let mut old = Vec::from(&b"BASALTW1"[..]);
        old.extend_from_slice(&1u64.to_le_bytes());
        for v in [3i32, 4, 5, stone as i32] {
            old.extend_from_slice(&v.to_le_bytes());
        }
        let legacy = dir.join("ancien");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("blocs.bin"), &old).unwrap();
        let mut migrated = World::new();
        assert_eq!(migrated.load(&legacy).unwrap(), 1);
        assert!(migrated.dirty);
        migrated.save(&legacy).unwrap();
        assert!(legacy.join("level.nbt").exists() && !legacy.join("blocs.bin").exists());
        assert_eq!(World::new().load(&legacy).unwrap(), 1);
        std::fs::write(dir.join("level.nbt"), b"n'importe quoi").unwrap();
        assert!(World::new().load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}


#[cfg(test)]
mod encode_speed {
    use super::*;
    /// Temps d'encodage et de compression d'un chunk (`cargo test encode_speed -- --ignored`).
    #[test]
    #[ignore]
    fn encode_speed() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let p = Palette::new(&|n| data.state_from_spec(n));
        let w = World::with_generator(Generator::new(Kind::Vanilla, 12345, p));
        for cx in 0..10 {
            for cz in 0..10 {
                w.terrain.column((cx, cz));
            }
        }
        let start = std::time::Instant::now();
        let mut total = 0;
        for cx in 0..10 {
            for cz in 0..10 {
                total += w.chunk_payload(cx, cz).1.len();
            }
        }
        let enc = start.elapsed();
        let start = std::time::Instant::now();
        let mut z = 0;
        for cx in 0..10 {
            for cz in 0..10 {
                let (_, payload) = w.chunk_payload(cx, cz);
                z += crate::network::protocol::bedrock::encode_packet_with(58, &payload, true, crate::utils::deflate::Level::Best).len();
            }
        }
        eprintln!("100 chunks : encodage {:?} ({} Ko), encodage + compression {:?} ({} Ko)", enc, total / 1024, start.elapsed(), z / 1024);
    }
}

#[cfg(test)]
mod dimension_chunk_tests {
    use super::*;

    /// Un chunk du Nether part avec huit tranches au plus, numérotées depuis 0, et huit
    /// palettes de biomes ; un chunk de l'End, seize.
    #[test]
    fn nether_and_end_chunks_have_the_shape_of_their_dimension() {
        for (kind, most, biome) in [(Kind::Nether, 8usize, None), (Kind::End, 16, Some(crate::world::generator::END_BIOME))] {
            let world = World::with_generator(Generator::new(kind, 5, Palette::new(&|_| None)));
            world.prepare(0, 0);
            let (count, payload) = world.chunk_payload(0, 0);
            assert!(count >= 1 && count as usize <= most, "{:?} : {} tranches", kind, count);
            // (première tranche : version 9, nombre de couches, puis son numéro : 0)
            assert_eq!((payload[0], payload[2]), (9, 0), "{:?}", kind);
            assert_eq!(kind.sections().len(), most);
            if let Some(biome) = biome {
                // la fin du contenu : seize biomes uniques (1, numéro en zigzag), puis l'octet de bordure
                let tail = &payload[payload.len() - 1 - 2 * most..];
                assert!(tail.chunks(2).take(most).all(|p| p == [1, biome * 2]), "{:?}", tail);
                assert_eq!(*tail.last().unwrap(), 0);
            }
        }
    }
}
