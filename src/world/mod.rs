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
/// Chunks générés gardés en mémoire (environ 40 Ko chacun). Au-delà, les moins récemment
/// utilisés sont oubliés : les régénérer ne prend qu'une milliseconde environ.
const CACHE_CHUNKS: usize = 2048;

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
}

impl Terrain {
    fn new(generator: Generator) -> Arc<Terrain> {
        Arc::new(Terrain {
            generator,
            chunks: Mutex::new(HashMap::new()),
            clock: AtomicU64::new(0),
            pending: Mutex::new(HashSet::new()),
            jobs: Mutex::new(None),
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
        if chunks.len() > CACHE_CHUNKS {
            // Les moins récemment utilisés sortent (ils seront régénérés au besoin)
            let mut stamps: Vec<u64> = chunks.values().map(|(_, s)| *s).collect();
            stamps.sort_unstable();
            let cut = stamps[stamps.len() / 4];
            chunks.retain(|_, (_, s)| *s > cut);
        }
    }

    /// Chunk généré (calculé sur place s'il n'est pas prêt).
    fn column(&self, key: (i32, i32)) -> Arc<Column> {
        if let Some(c) = self.cached(key) {
            return c;
        }
        let column = Arc::new(self.generator.generate(key.0, key.1));
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
                        let column = Arc::new(terrain.generator.generate(key.0, key.1));
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
        if block == self.base(x, y, z) {
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

    /// Le chunk est-il généré (prêt à être envoyé sans attendre) ?
    pub fn is_ready(&self, cx: i32, cz: i32) -> bool {
        self.terrain.ready((cx, cz))
    }

    /// Enregistre les blocs modifiés dans `level.nbt`. Chaque bloc occupe quatre entiers
    /// (x, y, z, identifiant) ; l'identifiant est stable (dérivé du nom et de l'état), donc
    /// valable d'une version du jeu à l'autre.
    pub fn save(&mut self, dir: &std::path::Path) -> std::io::Result<()> {
        let doc = self.snapshot();
        Self::write_snapshot(dir, &doc)
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
        matches!(crate::nbt::load_file(&dir.join("level.nbt")), Ok(Some(doc)) if doc.get("Generator").and_then(|t| t.as_str()).map(Kind::was_legacy).unwrap_or(false))
    }

    /// Nom du générateur d'un plugin enregistré pour ce monde (`plugin:<nom>`), s'il y en a un.
    pub fn stored_plugin(dir: &std::path::Path) -> Option<String> {
        let doc = crate::nbt::load_file(&dir.join("level.nbt")).ok()??;
        let text = doc.get("Generator")?.as_str()?.to_string();
        text.strip_prefix("plugin:").map(|s| s.to_string())
    }

    /// Recharge les blocs enregistrés. Renvoie leur nombre (0 si aucun fichier).
    pub fn load(&mut self, dir: &std::path::Path) -> Result<usize, String> {
        match crate::nbt::load_file(&dir.join("level.nbt"))? {
            Some(doc) => {
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
                self.dirty = false;
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
        *self.versions.entry(chunk).or_insert(0) += 1;
    }
}

impl WorldView {
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
        let mut sections = column.sections.iter().rposition(|s| !matches!(s, Section::Uniform(b) if *b == air)).map(|i| i + 1).unwrap_or(1);
        if let Some(edits) = edits {
            for ((_, y, _), block) in edits.iter() {
                if *block != air {
                    sections = sections.max(((*y as i32 - MIN_Y) / 16 + 1) as usize);
                }
            }
        }
        let mut w = W::default();
        for s in 0..sections {
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
        for s in 0..24usize {
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
        (sections as u32, w.0)
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
