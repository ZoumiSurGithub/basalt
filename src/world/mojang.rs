//! Le monde enregistré comme le fait le jeu (Bedrock) : un fichier `level.dat`, un fichier
//! `levelname.txt` et une base LevelDB dans `db/`, où chaque chunk est rangé sous-chunk par
//! sous-chunk, avec sa palette de blocs.
//!
//! Basalt n'y écrit que les chunks que quelqu'un a modifiés : le reste du terrain se
//! régénère à partir de la graine, comme le jeu régénère les chunks qu'il n'a jamais vus.
//! Un monde venu du jeu se lit de la même façon : ses chunks enregistrés sont servis tels
//! quels, les autres sont générés par Basalt.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::generator::{Column, Kind, Section, SECTIONS};
use super::leveldb::Db;
use super::{MAX_Y, MIN_Y};
use crate::nbt::{self, Tag};

/// Version des chunks écrits (celle du jeu depuis la 1.21.40).
const CHUNK_VERSION: u8 = 41;
/// Version du format de `level.dat`.
const STORAGE_VERSION: i32 = 10;

const TAG_DATA_3D: u8 = 0x2b;
const TAG_VERSION: u8 = 0x2c;
const TAG_SUB_CHUNK: u8 = 0x2f;
const TAG_BLOCK_ENTITIES: u8 = 0x31;
/// Marque des entités écrites par Basalt : elle tient la place du compteur de sessions du jeu
/// dans leur clé (quatre octets) et, en négatif, dans la moitié haute de leur `UniqueID`.
pub const ACTOR_MARK: u32 = 0x0042_4153;

/// `UniqueID` d'une entité de Basalt d'après son numéro (comme le jeu : compteur de sessions
/// en négatif dans la moitié haute, numéro dans la moitié basse).
pub fn actor_unique_id(number: u32) -> i64 {
    (-(ACTOR_MARK as i64) << 32) | number as i64
}

fn digp_key(cx: i32, cz: i32) -> Vec<u8> {
    let mut k = b"digp".to_vec();
    k.extend_from_slice(&cx.to_le_bytes());
    k.extend_from_slice(&cz.to_le_bytes());
    k
}

/// Entités d'un chunk (mondes d'avant les clés « actorprefix »)
const TAG_ENTITIES: u8 = 0x32;
const TAG_FINALIZED: u8 = 0x36;
const TAG_OLD_VERSION: u8 = 0x76;

/// Indice du sous-chunk le plus bas (y = -64).
const LOWEST: i32 = MIN_Y >> 4;

pub(crate) fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for b in bytes {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Identifiant réseau d'un bloc décrit par son nom et ses états : le hachage que calcule le jeu.
fn hash_of(name: &str, states: &Tag) -> u32 {
    let mut sorted = match states {
        Tag::Compound(entries) => entries.clone(),
        _ => Vec::new(),
    };
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    fnv1a(&nbt::write_le(&Tag::Compound(vec![("name".to_string(), Tag::string(name)), ("states".to_string(), Tag::Compound(sorted))])))
}

/// Correspondance entre les identifiants de blocs de Basalt et leur description dans un
/// monde enregistré (`{name, states, version}`).
pub struct BlockTable {
    entries: HashMap<u32, Vec<u8>>,
    defaults: HashMap<String, u32>,
    /// États de chaque bloc : (identifiant, propriétés en texte)
    states: HashMap<String, Vec<(u32, Vec<(String, String)>)>>,
    version: i32,
    pub air: u32,
    pub water: u32,
}

impl BlockTable {
    /// Construit la table à partir de la liste `blocks` de `block_palette.nbt`.
    pub fn from_palette(list: &[Tag]) -> BlockTable {
        let mut t = BlockTable { entries: HashMap::with_capacity(list.len()), defaults: HashMap::new(), states: HashMap::new(), version: 0, air: 0, water: 0 };
        for entry in list {
            let (Some(name), Some(id)) = (entry.get("name").and_then(|t| t.as_str()), entry.get("network_id").and_then(|t| t.as_int())) else { continue };
            let states = entry.get("states").cloned().unwrap_or(Tag::Compound(Vec::new()));
            let version = entry.get("version").and_then(|t| t.as_int()).unwrap_or(0);
            t.version = t.version.max(version);
            t.states.entry(name.to_string()).or_default().push((id as u32, props_of(&states)));
            t.entries.insert(id as u32, Self::describe(name, states, version));
            t.defaults.entry(name.to_string()).or_insert(id as u32);
        }
        t.air = t.defaults.get("minecraft:air").copied().unwrap_or_else(|| crate::bedrock::block_hash("minecraft:air"));
        t.water = t.defaults.get("minecraft:water").copied().unwrap_or_else(|| crate::bedrock::block_hash("minecraft:water"));
        t
    }

    fn describe(name: &str, states: Tag, version: i32) -> Vec<u8> {
        nbt::write_le(&Tag::Compound(vec![("name".to_string(), Tag::string(name)), ("states".to_string(), states), ("version".to_string(), Tag::Int(version))]))
    }

    /// Description d'un bloc, telle qu'écrite dans la palette d'un sous-chunk.
    fn entry(&self, id: u32) -> Vec<u8> {
        if let Some(e) = self.entries.get(&id) {
            return e.clone();
        }
        // Bloc créé par un plugin : son nom, et sa propriété d'état s'il en a une
        for (block, _) in crate::block::custom::all() {
            if let Some(variant) = block.states().iter().position(|s| *s == id) {
                let states = match block.variants.property() {
                    Some(p) => vec![(p.to_string(), Tag::Int(variant as i32))],
                    None => Vec::new(),
                };
                return Self::describe(&block.name, Tag::Compound(states), self.version);
            }
        }
        Self::describe("minecraft:info_update", Tag::Compound(Vec::new()), self.version)
    }

    /// Identifiant du bloc décrit par cette entrée de palette. Un état inconnu (bloc d'une
    /// très vieille version) donne l'état par défaut du bloc ; un bloc inconnu donne `None`.
    fn id_of(&self, entry: &Tag) -> Option<u32> {
        let name = entry.get("name")?.as_str()?;
        let empty = Tag::Compound(Vec::new());
        let id = hash_of(name, entry.get("states").unwrap_or(&empty));
        if self.entries.contains_key(&id) || !name.starts_with("minecraft:") {
            return Some(id);
        }
        let old = props_of(entry.get("states").unwrap_or(&empty));
        // Les très vieux noms étaient écrits sans séparation (`seaLantern`, `tripWire`)
        let mut base = String::new();
        for c in name.trim_start_matches("minecraft:").chars() {
            if c.is_ascii_uppercase() {
                base.push('_');
            }
            base.push(c.to_ascii_lowercase());
        }
        let double = base.starts_with("double_") && base.contains("slab");
        let base = match base.trim_end_matches(|c: char| c.is_ascii_digit()) {
            "stonebrick" => "stone_bricks",
            "stained_hardened_clay" | "hardened_clay" => "terracotta",
            "grass" => "grass_block",
            b if b.contains("slab") => "slab",
            b => b,
        }
        .to_string();
        // Le bloc a été renommé depuis : avant, une propriété portait sa couleur, son bois ou sa
        // variante (`wool` + `color=red`, `planks` + `wood_type=oak`, `stone` + `stone_type=granite`)
        for (key, value) in &old {
            if value.parse::<i64>().is_ok() || !(key.contains("type") || key.contains("color") || key.contains("variant")) {
                continue;
            }
            let value = match value.as_str() {
                "silver" => "light_gray",
                "wood" if base == "slab" => "petrified_oak",
                "prismarine_rough" => "prismarine",
                "prismarine_dark" => "dark_prismarine",
                "default" | "normal" | "stone" => continue,
                v => v,
            };
            let mut candidates = vec![format!("minecraft:{}_{}", value, base), format!("minecraft:{}", value), format!("minecraft:{}_{}", base, value)];
            if double {
                candidates.insert(0, format!("minecraft:{}_double_slab", value));
            }
            for candidate in candidates {
                if let Some(states) = self.states.get(&candidate) {
                    return Some(closest(states, &old));
                }
            }
        }
        // Le bloc existe encore (sous ce nom ou son nom d'aujourd'hui), avec d'autres états :
        // celui qui garde le plus de propriétés
        [name.to_string(), format!("minecraft:{}", base)].iter().find_map(|n| self.states.get(n)).map(|states| closest(states, &old))
    }
}

/// Propriétés d'un bloc, en texte (pour comparer deux descriptions sans se soucier des types).
fn props_of(states: &Tag) -> Vec<(String, String)> {
    match states {
        Tag::Compound(entries) => entries
            .iter()
            .map(|(k, v)| {
                let text = match v {
                    Tag::Str(s) => s.clone(),
                    other => other.as_i64().map(|n| n.to_string()).unwrap_or_default(),
                };
                (k.clone(), text)
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Parmi les états d'un bloc, celui qui partage le plus de propriétés avec `wanted` (le
/// premier, l'état par défaut, à égalité).
fn closest(states: &[(u32, Vec<(String, String)>)], wanted: &[(String, String)]) -> u32 {
    let score = |props: &Vec<(String, String)>| props.iter().filter(|p| wanted.contains(p)).count();
    let mut best = (states[0].0, score(&states[0].1));
    for (id, props) in &states[1..] {
        let s = score(props);
        if s > best.1 {
            best = (*id, s);
        }
    }
    best.0
}

// --------------------------------------------------------------------------------------
// Sous-chunks
// --------------------------------------------------------------------------------------

/// Écrit une couche de blocs : indices compactés, puis palette.
fn write_layer(out: &mut Vec<u8>, palette: &[Vec<u8>], indices: &[u16]) {
    let bits = [1u32, 2, 3, 4, 5, 6, 8, 16].into_iter().find(|b| (1usize << b) >= palette.len()).unwrap_or(16);
    out.push((bits << 1) as u8);
    let per_word = 32 / bits;
    let mut word = 0u32;
    let mut filled = 0;
    for i in indices {
        word |= (*i as u32) << (filled * bits);
        filled += 1;
        if filled == per_word {
            out.extend_from_slice(&word.to_le_bytes());
            word = 0;
            filled = 0;
        }
    }
    if filled > 0 {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(&(palette.len() as i32).to_le_bytes());
    for entry in palette {
        out.extend_from_slice(entry);
    }
}

/// Lit des indices compactés : (bits par valeur, 4096 indices). `pos` avance.
fn read_indices(data: &[u8], pos: &mut usize) -> Option<(u32, Vec<u16>)> {
    let header = *data.get(*pos)?;
    *pos += 1;
    let bits = (header >> 1) as u32;
    if bits == 0 {
        return Some((0, vec![0; 4096]));
    }
    if bits > 16 {
        return None;
    }
    let per_word = (32 / bits) as usize;
    let words = 4096usize.div_ceil(per_word);
    let raw = data.get(*pos..pos.checked_add(words * 4)?)?;
    *pos += words * 4;
    let mask = (1u32 << bits) - 1;
    let mut indices = Vec::with_capacity(4096);
    for w in raw.chunks_exact(4) {
        let word = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        for k in 0..per_word {
            if indices.len() < 4096 {
                indices.push(((word >> (k as u32 * bits)) & mask) as u16);
            }
        }
    }
    Some((bits, indices))
}

/// Lit une couche de blocs : 4096 identifiants (x << 8 | z << 4 | y). Les blocs inconnus
/// sont comptés dans `unknown` et remplacés par de l'air.
fn read_layer(data: &[u8], pos: &mut usize, table: &BlockTable, unknown: &mut Vec<String>) -> Option<Vec<u32>> {
    let persistent = data.get(*pos)? & 1 == 0;
    let (bits, indices) = read_indices(data, pos)?;
    if !persistent {
        return None; // palette d'identifiants de session : rien à en tirer hors du jeu
    }
    let count = if bits == 0 {
        1
    } else {
        let n = i32::from_le_bytes(data.get(*pos..*pos + 4)?.try_into().ok()?);
        *pos += 4;
        if !(0..=65_536).contains(&n) {
            return None;
        }
        n as usize
    };
    let mut palette = Vec::with_capacity(count);
    for _ in 0..count {
        let entry = nbt::read_le(data, pos)?;
        palette.push(match table.id_of(&entry) {
            Some(id) => id,
            None => {
                unknown.push(entry.str_of("name").unwrap_or("?").to_string());
                table.air
            }
        });
    }
    Some(indices.into_iter().map(|i| palette.get(i as usize).copied().unwrap_or(table.air)).collect())
}

/// Un sous-chunk lu : ses blocs, et les cases de sa seconde couche qui contiennent de l'eau.
struct SubChunk {
    blocks: Vec<u32>,
    wet: Vec<u16>,
}

fn read_sub_chunk(data: &[u8], table: &BlockTable, unknown: &mut Vec<String>) -> Option<SubChunk> {
    let mut pos = 1;
    let layers = match *data.first()? {
        1 => 1,
        8 => {
            pos += 1;
            *data.get(1)? as usize
        }
        9 => {
            pos += 2;
            *data.get(1)? as usize
        }
        // (les formats d'avant la 1.2.13 du jeu, par identifiants numériques, ne sont pas lus)
        _ => return None,
    };
    if layers == 0 {
        return Some(SubChunk { blocks: vec![table.air; 4096], wet: Vec::new() });
    }
    let blocks = read_layer(data, &mut pos, table, unknown)?;
    let mut wet = Vec::new();
    if layers >= 2 {
        if let Some(second) = read_layer(data, &mut pos, table, unknown) {
            wet = second.iter().enumerate().filter(|(_, b)| **b != table.air).map(|(i, _)| i as u16).collect();
        }
    }
    Some(SubChunk { blocks, wet })
}

/// Écrit un sous-chunk (version 9) : ses blocs, et une seconde couche d'eau s'il en faut une.
fn write_sub_chunk(index: i8, blocks: &[u32], water: Option<&[bool]>, table: &BlockTable) -> Vec<u8> {
    let mut palette: Vec<u32> = Vec::new();
    let mut slots: HashMap<u32, u16> = HashMap::new();
    let indices: Vec<u16> = blocks
        .iter()
        .map(|b| {
            *slots.entry(*b).or_insert_with(|| {
                palette.push(*b);
                (palette.len() - 1) as u16
            })
        })
        .collect();
    let mut out = vec![9, if water.is_some() { 2 } else { 1 }, index as u8];
    let entries: Vec<Vec<u8>> = palette.iter().map(|b| table.entry(*b)).collect();
    write_layer(&mut out, &entries, &indices);
    if let Some(water) = water {
        let indices: Vec<u16> = water.iter().map(|w| *w as u16).collect();
        write_layer(&mut out, &[table.entry(table.air), table.entry(table.water)], &indices);
    }
    out
}

/// Clé d'une donnée de chunk dans la base (monde normal : pas de numéro de dimension).
fn chunk_key(cx: i32, cz: i32, tag: u8, sub: Option<i8>) -> Vec<u8> {
    let mut k = Vec::with_capacity(10);
    k.extend_from_slice(&cx.to_le_bytes());
    k.extend_from_slice(&cz.to_le_bytes());
    k.push(tag);
    if let Some(s) = sub {
        k.push(s as u8);
    }
    k
}

/// Hauteurs et biomes d'un chunk (« Data3D ») : 256 hauteurs, puis une palette de biomes par
/// sous-chunk.
fn write_data_3d(heights: &[i16; 256], biomes: &dyn Fn(usize, usize, usize, usize) -> u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(512 + SECTIONS * 16);
    for h in heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    for s in 0..SECTIONS {
        let mut palette: Vec<u32> = Vec::new();
        let mut indices = vec![0u16; 4096];
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..16 {
                    let b = biomes(s, x, y, z);
                    let i = palette.iter().position(|p| *p == b).unwrap_or_else(|| {
                        palette.push(b);
                        palette.len() - 1
                    });
                    indices[(x << 8) | (z << 4) | y] = i as u16;
                }
            }
        }
        if palette.len() == 1 {
            out.push(1); // zéro bit par case : un seul biome
            out.extend_from_slice(&(palette[0] as i32).to_le_bytes());
            continue;
        }
        let bits = [1u32, 2, 3, 4, 5, 6, 8, 16].into_iter().find(|b| (1usize << b) >= palette.len()).unwrap_or(16);
        out.push(((bits << 1) | 1) as u8);
        let per_word = 32 / bits;
        for chunk in indices.chunks(per_word as usize) {
            let mut word = 0u32;
            for (k, i) in chunk.iter().enumerate() {
                word |= (*i as u32) << (k as u32 * bits);
            }
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&(palette.len() as i32).to_le_bytes());
        for b in &palette {
            out.extend_from_slice(&(*b as i32).to_le_bytes());
        }
    }
    out
}

/// Biomes d'un « Data3D », ramenés à une valeur par case de 4×4×4 (voir `Column::biome_ids`).
fn read_data_3d(data: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 512;
    let mut out = Vec::with_capacity(SECTIONS * 64);
    let mut previous: Option<Vec<u8>> = None;
    for _ in 0..SECTIONS {
        let cells = match data.get(pos) {
            // 0xff : comme le sous-chunk du dessous
            Some(0xff) => {
                pos += 1;
                previous.clone()?
            }
            Some(_) => {
                let (bits, indices) = read_indices(data, &mut pos)?;
                let count = if bits == 0 {
                    1
                } else {
                    let n = i32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
                    pos += 4;
                    n.clamp(0, 4096) as usize
                };
                let mut palette = Vec::with_capacity(count);
                for _ in 0..count {
                    let id = i32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
                    pos += 4;
                    palette.push(if (0..=255).contains(&id) { id as u8 } else { 1 });
                }
                let mut cells = vec![1u8; 64];
                for x in 0..4 {
                    for z in 0..4 {
                        for y in 0..4 {
                            let i = indices[((x * 4 + 1) << 8) | ((z * 4 + 1) << 4) | (y * 4 + 1)] as usize;
                            cells[x * 16 + z * 4 + y] = palette.get(i).copied().unwrap_or(1);
                        }
                    }
                }
                cells
            }
            // (les mondes d'avant la 1.18 s'arrêtent plus tôt : le reste garde le dernier biome)
            None => previous.clone()?,
        };
        out.extend_from_slice(&cells);
        previous = Some(cells);
    }
    Some(out)
}

// --------------------------------------------------------------------------------------
// Le monde sur le disque
// --------------------------------------------------------------------------------------

/// Ce qu'un chunk enregistré apporte au terrain.
pub struct Stored {
    pub sections: Vec<Option<Section>>,
    pub wet: Vec<(u8, i16, u8)>,
    pub biome_ids: Option<Vec<u8>>,
}

/// Écritures à faire dans la base : (clé, valeur) ou (clé, rien) pour une suppression.
pub type Batch = Vec<(Vec<u8>, Option<Vec<u8>>)>;

/// La base d'un monde, partagée entre la boucle du serveur, les fils de génération (qui
/// lisent les chunks enregistrés) et le fil d'écriture.
pub struct Store {
    db: Mutex<Db>,
    pub table: Arc<BlockTable>,
    /// Monde créé par Basalt : les chunks enregistrés gardent ce que le générateur sait
    /// d'eux (coffres des structures, lits des villages). Faux pour un monde venu du jeu.
    pub native: bool,
    unknown: Mutex<std::collections::BTreeSet<String>>,
    overflow: std::sync::atomic::AtomicUsize,
}

impl Store {
    pub fn open(dir: &Path, table: Arc<BlockTable>, native: bool) -> Result<Arc<Store>, String> {
        let db = Db::open(&dir.join("db"))?;
        Ok(Arc::new(Store { db: Mutex::new(db), table, native, unknown: Default::default(), overflow: Default::default() }))
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Blocs d'un type inconnu rencontrés à la lecture (remplacés par de l'air), et
    /// sous-chunks de plus de 256 blocs différents (simplifiés).
    pub fn losses(&self) -> (Vec<String>, usize) {
        let names = self.unknown.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
        (names, self.overflow.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Le chunk enregistré à cet endroit, s'il y en a un.
    pub fn load(&self, cx: i32, cz: i32) -> Option<Stored> {
        let mut db = self.db();
        if db.get(&chunk_key(cx, cz, TAG_VERSION, None)).is_none() && db.get(&chunk_key(cx, cz, TAG_OLD_VERSION, None)).is_none() {
            return None;
        }
        let mut unknown = Vec::new();
        let (mut sections, mut wet) = (Vec::with_capacity(SECTIONS), Vec::new());
        for s in 0..SECTIONS {
            let index = (LOWEST + s as i32) as i8;
            let sub = db.get(&chunk_key(cx, cz, TAG_SUB_CHUNK, Some(index))).and_then(|data| read_sub_chunk(&data, &self.table, &mut unknown));
            sections.push(sub.map(|sub| {
                let base_y = MIN_Y + s as i32 * 16;
                wet.extend(sub.wet.iter().map(|i| ((i >> 8) as u8, (base_y + (i & 15) as i32) as i16, ((i >> 4) & 15) as u8)));
                let (section, lost) = section_of(&sub.blocks);
                if lost {
                    self.overflow.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                section
            }));
        }
        let stored = Stored { sections, wet, biome_ids: db.get(&chunk_key(cx, cz, TAG_DATA_3D, None)).and_then(|d| read_data_3d(&d)) };
        if !unknown.is_empty() {
            let mut names = self.unknown.lock().unwrap_or_else(|e| e.into_inner());
            for name in unknown {
                if names.len() < 64 {
                    names.insert(name);
                }
            }
        }
        Some(stored)
    }

    /// Prépare l'écriture d'un chunk : `block(x, y, z)` donne ses blocs (y absolu), `wet`
    /// dit si la case contient aussi de l'eau, `biome(sous-chunk, x, y, z)` son biome.
    pub fn encode(&self, cx: i32, cz: i32, block: &dyn Fn(usize, i32, usize) -> u32, wet: &dyn Fn(usize, i32, usize) -> bool, biome: &dyn Fn(usize, usize, usize, usize) -> u32, out: &mut Batch) {
        let air = self.table.air;
        let mut heights = [0i16; 256];
        for s in 0..SECTIONS {
            let base_y = MIN_Y + s as i32 * 16;
            let mut blocks = vec![air; 4096];
            let mut water = vec![false; 4096];
            let (mut empty, mut any_water) = (true, false);
            for x in 0..16 {
                for z in 0..16 {
                    for y in 0..16 {
                        let b = block(x, base_y + y as i32, z);
                        if b != air {
                            empty = false;
                            blocks[(x << 8) | (z << 4) | y] = b;
                            heights[z * 16 + x] = (base_y + y as i32 - MIN_Y + 1) as i16;
                            if b != self.table.water && wet(x, base_y + y as i32, z) {
                                water[(x << 8) | (z << 4) | y] = true;
                                any_water = true;
                            }
                        }
                    }
                }
            }
            let index = (LOWEST + s as i32) as i8;
            let key = chunk_key(cx, cz, TAG_SUB_CHUNK, Some(index));
            // (un sous-chunk d'air n'est pas enregistré, comme dans le jeu)
            out.push((key, (!empty).then(|| write_sub_chunk(index, &blocks, any_water.then_some(&water[..]), &self.table))));
        }
        out.push((chunk_key(cx, cz, TAG_DATA_3D, None), Some(write_data_3d(&heights, biome))));
        out.push((chunk_key(cx, cz, TAG_VERSION, None), Some(vec![CHUNK_VERSION])));
        out.push((chunk_key(cx, cz, TAG_FINALIZED, None), Some(2i32.to_le_bytes().to_vec())));
    }

    /// Tous les blocs à données enregistrés (coffres, fours, panneaux…), chunk par chunk.
    /// Parcourt toute la base : à faire une fois, à l'ouverture du monde.
    pub fn block_entities(&self) -> Vec<((i32, i32), Vec<Tag>)> {
        let mut db = self.db();
        let mut out = Vec::new();
        for key in db.keys() {
            if key.len() != 9 || key[8] != TAG_BLOCK_ENTITIES {
                continue;
            }
            let chunk = (i32::from_le_bytes([key[0], key[1], key[2], key[3]]), i32::from_le_bytes([key[4], key[5], key[6], key[7]]));
            let Some(data) = db.get(&key) else { continue };
            let (mut pos, mut entries) = (0, Vec::new());
            while pos < data.len() {
                match nbt::read_le(&data, &mut pos) {
                    Some(tag) => entries.push(tag),
                    None => break,
                }
            }
            out.push((chunk, entries));
        }
        out
    }

    /// Les créatures et autres entités enregistrées par le jeu : une clé « actorprefix » par
    /// entité (mondes récents), ou la liste d'un chunk sous l'étiquette 0x32 (mondes plus
    /// anciens). Parcourt toute la base : à faire une fois, à l'import d'un monde.
    pub fn actors(&self) -> Vec<Tag> {
        let mut db = self.db();
        let mut out = Vec::new();
        for key in db.keys() {
            let modern = key.starts_with(b"actorprefix");
            let legacy = key.len() == 9 && key[8] == TAG_ENTITIES;
            if !modern && !legacy {
                continue;
            }
            let Some(data) = db.get(&key) else { continue };
            let mut pos = 0;
            while pos < data.len() {
                match nbt::read_le(&data, &mut pos) {
                    Some(tag) => out.push(tag),
                    None => break,
                }
            }
        }
        out
    }

    /// Plus grand numéro d'entité déjà écrit par Basalt dans cette base, plus un. Les entités
    /// de Basalt portent toutes la même marque (`ACTOR_MARK`) à la place du compteur de
    /// sessions du jeu : elles ne se confondent pas avec celles que le jeu aurait écrites.
    pub fn next_actor(&self) -> u32 {
        let mut prefix = b"actorprefix".to_vec();
        prefix.extend_from_slice(&ACTOR_MARK.to_be_bytes());
        let mut db = self.db();
        db.keys().iter().filter(|k| k.len() == prefix.len() + 4 && k.starts_with(&prefix)).map(|k| u32::from_be_bytes([k[15], k[16], k[17], k[18]])).max().map(|n| n.wrapping_add(1)).unwrap_or(1)
    }

    /// Ce chunk a-t-il des entités de Basalt dans la base ?
    pub fn has_actors(&self, cx: i32, cz: i32) -> bool {
        let mark = ACTOR_MARK.to_be_bytes();
        self.db().get(&digp_key(cx, cz)).map(|v| v.chunks_exact(8).any(|id| id[..4] == mark)).unwrap_or(false)
    }

    /// Écrit les entités d'un chunk comme le fait le jeu : une clé « actorprefix » par entité,
    /// et la liste de leurs clés sous « digp » + position du chunk. Les entités que Basalt y
    /// avait écrites sont remplacées ; celles d'une autre origine (le jeu lui-même) restent.
    pub fn write_actors(&self, cx: i32, cz: i32, actors: &[(u32, Vec<u8>)]) -> std::io::Result<()> {
        let mark = ACTOR_MARK.to_be_bytes();
        let list = digp_key(cx, cz);
        let old = self.db().get(&list).unwrap_or_default();
        let mut batch = Batch::new();
        let mut ids: Vec<u8> = Vec::new();
        for id in old.chunks_exact(8) {
            if id[..4] == mark {
                let mut key = b"actorprefix".to_vec();
                key.extend_from_slice(id);
                batch.push((key, None));
            } else {
                ids.extend_from_slice(id);
            }
        }
        for (number, nbt) in actors {
            let mut id = mark.to_vec();
            id.extend_from_slice(&number.to_be_bytes());
            let mut key = b"actorprefix".to_vec();
            key.extend_from_slice(&id);
            ids.extend_from_slice(&id);
            batch.push((key, Some(nbt.clone())));
        }
        batch.push((list, (!ids.is_empty()).then_some(ids)));
        self.write(batch)
    }

    /// Prépare l'écriture des blocs à données d'un chunk (`None` : il n'en a plus).
    pub fn encode_block_entities(cx: i32, cz: i32, data: Option<Vec<u8>>, out: &mut Batch) {
        out.push((chunk_key(cx, cz, TAG_BLOCK_ENTITIES, None), data));
    }

    /// Écrit un lot dans la base (sur le disque avant de rendre la main).
    pub fn write(&self, batch: Batch) -> std::io::Result<()> {
        let mut db = self.db();
        for (key, value) in batch {
            match value {
                Some(v) => db.put(&key, v),
                None => db.delete(&key),
            }
        }
        db.flush()
    }
}

/// Section d'un chunk à partir de ses 4096 blocs. Vrai en second si elle compte plus de 256
/// blocs différents : les derniers sont alors confondus avec le 256e.
fn section_of(blocks: &[u32]) -> (Section, bool) {
    let mut palette: Vec<u32> = Vec::new();
    let mut slots: HashMap<u32, u8> = HashMap::new();
    let mut index = Box::new([0u8; 4096]);
    let mut lost = false;
    for (i, b) in blocks.iter().enumerate().take(4096) {
        index[i] = match slots.get(b) {
            Some(s) => *s,
            None if palette.len() < 256 => {
                palette.push(*b);
                slots.insert(*b, (palette.len() - 1) as u8);
                (palette.len() - 1) as u8
            }
            None => {
                lost = true;
                255
            }
        };
    }
    if palette.len() == 1 {
        (Section::Uniform(palette[0]), false)
    } else {
        (Section::Mixed { palette, index }, lost)
    }
}

impl Stored {
    /// Pose ce chunk enregistré sur une colonne (générée, ou vide pour un monde venu du jeu).
    pub fn apply(self, column: &mut Column, air: u32) {
        for (s, section) in self.sections.into_iter().enumerate() {
            if let Some(slot) = column.sections.get_mut(s) {
                *slot = section.unwrap_or(Section::Uniform(air));
            }
        }
        column.wet = self.wet;
        if self.biome_ids.is_some() {
            column.biome_ids = self.biome_ids;
        }
    }
}

// --------------------------------------------------------------------------------------
// level.dat
// --------------------------------------------------------------------------------------

/// Ce que le serveur sait du monde au moment de l'enregistrer.
#[derive(Clone, Debug, Default)]
pub struct LevelInfo {
    pub name: String,
    /// Terrain tel que Basalt le nomme (`vanilla`, `flat`, `void`, `plugin:<nom>`)
    pub generator: String,
    pub seed: u64,
    pub spawn: (i32, i32, i32),
    pub time: i64,
    pub raining: bool,
    pub thundering: bool,
    /// Ticks avant le prochain changement de temps
    pub weather_time: i32,
    /// Règles du jeu de ce monde : l'heure avance, la météo change d'elle-même
    pub daylight_cycle: bool,
    pub weather_cycle: bool,
    pub protocol: i32,
    pub game_version: String,
}

fn level_path(dir: &Path) -> PathBuf {
    dir.join("level.dat")
}

/// Lit `level.dat` (8 octets d'en-tête, puis du NBT petit-boutiste). `Ok(None)` s'il n'y en a pas.
pub fn read_level(dir: &Path) -> Result<Option<Tag>, String> {
    let bytes = match std::fs::read(level_path(dir)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut pos = 8;
    nbt::read_le(&bytes, &mut pos).map(Some).ok_or_else(|| "level.dat illisible".to_string())
}

/// Copie les chunks d'une autre dimension d'un monde du jeu (1 : le Nether, 2 : l'End) dans la
/// base d'un monde à part, où ils deviennent des chunks ordinaires. Dans la base du jeu, leur
/// clé porte la dimension après les coordonnées ; elle est retirée. Rend le nombre de chunks
/// copiés et la position de l'un d'eux. La base d'origine n'est pas modifiée.
pub fn split_dimension(src: &Path, dimension: i32, dst: &Path) -> Result<(usize, Option<(i32, i32)>), String> {
    let mut from = super::leveldb::Db::open(&src.join("db"))?;
    let wanted: Vec<Vec<u8>> = from.keys().into_iter().filter(|k| (k.len() == 13 || k.len() == 14) && k[8..12] == dimension.to_le_bytes()).collect();
    if wanted.is_empty() {
        return Ok((0, None));
    }
    let mut to = super::leveldb::Db::open(&dst.join("db"))?;
    let (mut chunks, mut one) = (0, None);
    for key in wanted {
        let Some(value) = from.get(&key) else { continue };
        let mut plain = key[..8].to_vec();
        plain.extend_from_slice(&key[12..]);
        if key.len() == 13 && (key[12] == TAG_VERSION || key[12] == TAG_OLD_VERSION) {
            chunks += 1;
            one = Some((i32::from_le_bytes([key[0], key[1], key[2], key[3]]), i32::from_le_bytes([key[4], key[5], key[6], key[7]])));
        }
        to.put(&plain, value);
    }
    to.flush().map_err(|e| e.to_string())?;
    Ok((chunks, one))
}

/// Terrain et graine d'après `level.dat` : ceux qu'a notés Basalt, sinon ceux du jeu.
pub fn level_generator(level: &Tag) -> (Kind, u64, Option<String>) {
    let seed = level.get("RandomSeed").and_then(|t| t.as_i64()).unwrap_or(0) as u64;
    if let Some(name) = level.str_of("BasaltGenerator") {
        return (Kind::parse(name), seed, name.strip_prefix("plugin:").map(|s| s.to_string()));
    }
    let kind = match level.get("Generator").and_then(|t| t.as_i64()) {
        Some(1) => Kind::Vanilla,
        // Monde plat du jeu : ses couches ne sont pas forcément celles de Basalt, mais un
        // monde plat sans aucune couche est un monde vide
        Some(2) if !level.str_of("FlatWorldLayers").map(|l| l.contains("minecraft:air") && !l.contains("grass")).unwrap_or(false) => Kind::Flat,
        _ => Kind::Void,
    };
    (kind, seed, None)
}

/// Écrit `level.dat` et `levelname.txt`. Ce que le jeu y avait mis et que Basalt ne connaît
/// pas est gardé tel quel.
pub fn write_level(dir: &Path, info: &LevelInfo) -> std::io::Result<()> {
    let mut entries: Vec<(String, Tag)> = match read_level(dir) {
        Ok(Some(Tag::Compound(e))) => e,
        _ => Vec::new(),
    };
    let fresh = entries.is_empty();
    let mut set = |key: &str, value: Tag, always: bool| match entries.iter_mut().find(|(k, _)| k == key) {
        Some(slot) if always => slot.1 = value,
        Some(_) => {}
        None => entries.push((key.to_string(), value)),
    };
    let version: Vec<Tag> = info.game_version.split('.').filter_map(|p| p.parse::<i32>().ok()).chain(std::iter::repeat(0)).take(5).map(Tag::Int).collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let kind = Kind::parse(&info.generator);
    set("StorageVersion", Tag::Int(STORAGE_VERSION), false);
    set("NetworkVersion", Tag::Int(info.protocol), true);
    set("lastOpenedWithVersion", Tag::List(3, version.clone()), true);
    set("MinimumCompatibleClientVersion", Tag::List(3, version), false);
    set("LevelName", Tag::string(&info.name), true);
    set("LastPlayed", Tag::Long(now), true);
    set("Time", Tag::Long(info.time), true);
    set("currentTick", Tag::Long(info.time), true);
    set("SpawnX", Tag::Int(info.spawn.0), true);
    set("SpawnY", Tag::Int(info.spawn.1), true);
    set("SpawnZ", Tag::Int(info.spawn.2), true);
    set("rainLevel", Tag::Float(if info.raining { 1.0 } else { 0.0 }), true);
    set("lightningLevel", Tag::Float(if info.thundering { 1.0 } else { 0.0 }), true);
    set("rainTime", Tag::Int(info.weather_time), true);
    set("lightningTime", Tag::Int(info.weather_time), true);
    set("dodaylightcycle", Tag::Byte(info.daylight_cycle as i8), true);
    set("doweathercycle", Tag::Byte(info.weather_cycle as i8), true);
    set("RandomSeed", Tag::Long(info.seed as i64), false);
    set("Generator", Tag::Int(if kind == Kind::Vanilla { 1 } else { 2 }), false);
    if fresh || entries.iter().any(|(k, _)| k == "BasaltGenerator") {
        // (seuls les mondes créés par Basalt portent cette marque)
        let mut set = |key: &str, value: Tag| match entries.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => entries.push((key.to_string(), value)),
        };
        set("BasaltGenerator", Tag::string(&info.generator));
        if fresh {
            let layers = match kind {
                Kind::Void => r#"{"block_name":"minecraft:air","count":1}"#.to_string(),
                _ => r#"{"block_name":"minecraft:stone","count":1},{"block_name":"minecraft:dirt","count":2},{"block_name":"minecraft:grass_block","count":1}"#.to_string(),
            };
            set("FlatWorldLayers", Tag::string(&format!(r#"{{"biome_id":1,"block_layers":[{}],"encoding_version":6,"structure_options":null,"world_version":"version.post_1_18"}}"#, layers)));
            set("GameType", Tag::Int(0));
            set("Difficulty", Tag::Int(2));
            set("commandsEnabled", Tag::Byte(1));
            set("hasBeenLoadedInCreative", Tag::Byte(1));
            set("spawnMobs", Tag::Byte(1));
            set("Dimension", Tag::Int(0));
            set("ForceGameType", Tag::Byte(0));
        }
    }
    let body = nbt::write_le(&Tag::Compound(entries));
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&STORAGE_VERSION.to_le_bytes());
    out.extend_from_slice(&(body.len() as i32).to_le_bytes());
    out.extend_from_slice(&body);
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("level.dat.tmp");
    std::fs::write(&tmp, &out)?;
    // (le jeu garde lui aussi la version précédente à côté)
    if level_path(dir).exists() {
        let _ = std::fs::copy(level_path(dir), dir.join("level.dat_old"));
    }
    std::fs::rename(&tmp, level_path(dir))?;
    std::fs::write(dir.join("levelname.txt"), info.name.as_bytes())
}

/// Hauteur maximale utilisée ici (évite un avertissement quand rien d'autre ne s'en sert).
pub const TOP: i32 = MAX_Y;

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Option<Arc<BlockTable>> {
        let data = crate::data::GameData::load(Path::new("data")).ok()?;
        Some(data.table.clone())
    }

    #[test]
    fn block_ids_are_the_hash_of_their_description() {
        let Some(t) = table() else { return };
        assert!(t.entries.len() > 10_000);
        // Chaque bloc du jeu se relit : la description écrite redonne le même identifiant
        for (id, entry) in &t.entries {
            let mut pos = 0;
            let tag = nbt::read_le(entry, &mut pos).unwrap();
            assert_eq!(t.id_of(&tag), Some(*id), "{:?}", tag);
        }
        // Un état inconnu donne l'état par défaut, un bloc inconnu rien
        let odd = Tag::compound(vec![("name", Tag::string("minecraft:stone")), ("states", Tag::compound(vec![("vieux", Tag::Int(3))]))]);
        assert_eq!(t.id_of(&odd), t.defaults.get("minecraft:stone").copied());
        // Blocs renommés depuis : la couleur ou le bois était une propriété
        let old = |name: &str, key: &str, value: &str| t.id_of(&Tag::compound(vec![("name", Tag::string(name)), ("states", Tag::compound(vec![(key, Tag::string(value))]))]));
        assert_eq!(old("minecraft:wool", "color", "red"), t.defaults.get("minecraft:red_wool").copied());
        assert_eq!(old("minecraft:planks", "wood_type", "spruce"), t.defaults.get("minecraft:spruce_planks").copied());
        assert_eq!(old("minecraft:stained_glass", "color", "silver"), t.defaults.get("minecraft:light_gray_stained_glass").copied());
        assert_eq!(old("minecraft:stone", "stone_type", "granite"), t.defaults.get("minecraft:granite").copied());
        assert_eq!(t.id_of(&Tag::compound(vec![("name", Tag::string("minecraft:nexistepas"))])), None);
    }

    #[test]
    fn sub_chunks_are_written_and_read_back() {
        let Some(t) = table() else { return };
        let ids: Vec<u32> = t.entries.keys().copied().take(300).collect();
        for kinds in [1usize, 2, 5, 17, 40, 300] {
            let blocks: Vec<u32> = (0..4096).map(|i| ids[(i * 7 + i / 16) % kinds]).collect();
            let water: Vec<bool> = (0..4096).map(|i| i % 5 == 0).collect();
            let data = write_sub_chunk(-3, &blocks, Some(&water), &t);
            assert_eq!(&data[..3], &[9, 2, (-3i8) as u8]);
            let mut unknown = Vec::new();
            let sub = read_sub_chunk(&data, &t, &mut unknown).unwrap();
            assert_eq!(sub.blocks, blocks, "{} blocs différents", kinds);
            assert_eq!(sub.wet, (0..4096u16).filter(|i| i % 5 == 0).collect::<Vec<_>>());
            assert!(unknown.is_empty());
        }
    }

    #[test]
    fn a_chunk_goes_to_the_database_and_comes_back() {
        let Some(t) = table() else { return };
        let dir = std::env::temp_dir().join(format!("basalt-mojang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stone = t.defaults["minecraft:stone"];
        let stairs = t.defaults["minecraft:oak_stairs"];
        let block = |x: usize, y: i32, z: usize| match y {
            -64..=-10 => stone,
            70 if x == 3 && z == 9 => stairs,
            _ => t.air,
        };
        let wet = |x: usize, y: i32, z: usize| y == 70 && x == 3 && z == 9;
        let biome = |s: usize, x: usize, _y: usize, _z: usize| if s > 10 && x >= 8 { 4u32 } else { 1 };
        {
            let store = Store::open(&dir, t.clone(), true).unwrap();
            assert!(store.load(-2, 5).is_none());
            let mut batch = Batch::new();
            store.encode(-2, 5, &block, &wet, &biome, &mut batch);
            store.write(batch).unwrap();
        }
        let store = Store::open(&dir, t.clone(), true).unwrap();
        assert!(store.load(5, -2).is_none(), "seul le chunk écrit est là");
        let stored = store.load(-2, 5).unwrap();
        assert_eq!(stored.wet, vec![(3, 70, 9)]);
        let mut column = Column::from_blocks(&vec![t.air; 256 * (MAX_Y - MIN_Y + 1) as usize], t.air, super::super::generator::Biome::Plains);
        let ids = stored.biome_ids.clone().unwrap();
        stored.apply(&mut column, t.air);
        for (x, y, z) in [(0, -64, 0), (15, -10, 15), (3, 70, 9), (3, 71, 9), (0, -9, 0), (7, 319, 7)] {
            assert_eq!(column.get(x, y, z), block(x, y, z), "bloc en {} {} {}", x, y, z);
        }
        // Biomes : une valeur par case de 4×4×4, sous-chunk par sous-chunk
        assert_eq!((ids[0], ids[11 * 64], ids[11 * 64 + 2 * 16]), (1, 1, 4));
        assert_eq!(store.losses(), (Vec::new(), 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_dat_keeps_what_the_game_wrote() {
        let dir = std::env::temp_dir().join(format!("basalt-leveldat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let info = LevelInfo { name: "lobby".into(), generator: "void".into(), seed: 42, spawn: (1, 64, -3), time: 6000, raining: true, thundering: false, weather_time: 900, daylight_cycle: true, weather_cycle: false, protocol: 2225, game_version: "1.26.60".into() };
        write_level(&dir, &info).unwrap();
        let level = read_level(&dir).unwrap().unwrap();
        assert_eq!(level_generator(&level), (Kind::Void, 42, None));
        assert_eq!((level.get("SpawnY").and_then(|t| t.as_int()), level.str_of("LevelName")), (Some(64), Some("lobby")));
        assert_eq!(std::fs::read_to_string(dir.join("levelname.txt")).unwrap(), "lobby");
        assert_eq!((level.get("rainTime"), level.get("dodaylightcycle"), level.get("doweathercycle")), (Some(&Tag::Int(900)), Some(&Tag::Byte(1)), Some(&Tag::Byte(0))));
        // Un monde du jeu : sa graine et son type restent, ce que Basalt ignore aussi
        let game = Tag::compound(vec![("RandomSeed", Tag::Long(-77)), ("Generator", Tag::Int(1)), ("educationFeaturesEnabled", Tag::Byte(1))]);
        let body = nbt::write_le(&game);
        let mut raw = 10i32.to_le_bytes().to_vec();
        raw.extend_from_slice(&(body.len() as i32).to_le_bytes());
        raw.extend_from_slice(&body);
        std::fs::write(dir.join("level.dat"), raw).unwrap();
        write_level(&dir, &LevelInfo { time: 99, ..info }).unwrap();
        let level = read_level(&dir).unwrap().unwrap();
        assert_eq!(level_generator(&level), (Kind::Vanilla, -77i64 as u64, None));
        assert_eq!(level.get("educationFeaturesEnabled"), Some(&Tag::Byte(1)));
        assert_eq!(level.get("Time").and_then(|t| t.as_i64()), Some(99));
        assert!(level.get("BasaltGenerator").is_none(), "un monde du jeu n'est pas marqué comme créé par Basalt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Les chunks du Nether d'un monde du jeu passent dans la base d'un monde à part, où ils se
    /// lisent comme n'importe quel chunk ; ceux de la surface et de l'End n'y vont pas.
    #[test]
    fn another_dimension_becomes_a_world_of_its_own() {
        let Some(t) = table() else { return };
        let root = std::env::temp_dir().join(format!("basalt-mojang-dim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (src, dst) = (root.join("monde"), root.join("monde_nether"));
        let rack = t.defaults["minecraft:netherrack"];
        let block = |_x: usize, y: i32, _z: usize| if (0..=40).contains(&y) { rack } else { t.air };
        {
            // Un chunk écrit comme le fait Basalt, puis rangé sous les clés du jeu : surface,
            // Nether (dimension 1) et End (dimension 2)
            let store = Store::open(&src, t.clone(), false).unwrap();
            let mut batch = Batch::new();
            store.encode(3, -1, &block, &|_, _, _| false, &|_, _, _, _| 8, &mut batch);
            let mut all = Batch::new();
            for (key, value) in batch {
                for dim in [1i32, 2] {
                    let mut k = key[..8].to_vec();
                    k.extend_from_slice(&dim.to_le_bytes());
                    k.extend_from_slice(&key[8..]);
                    all.push((k, value.clone()));
                }
                all.push((key, value));
            }
            store.write(all).unwrap();
        }
        assert_eq!(split_dimension(&src, 1, &dst).unwrap(), (1, Some((3, -1))));
        assert_eq!(split_dimension(&src, 3, &root.join("rien")).unwrap(), (0, None));
        assert!(!root.join("rien").exists(), "rien n'est créé pour une dimension absente");
        let store = Store::open(&dst, t.clone(), true).unwrap();
        let stored = store.load(3, -1).expect("le chunk du Nether est là");
        let mut column = Column::from_blocks(&vec![t.air; 256 * (MAX_Y - MIN_Y + 1) as usize], t.air, super::super::generator::Biome::Plains);
        stored.apply(&mut column, t.air);
        assert_eq!((column.get(5, 40, 5), column.get(5, 41, 5), column.get(5, 0, 5)), (rack, t.air, rack));
        // (et rien d'autre : ni clé à 13 ou 14 octets, ni chunk voisin)
        assert!(store.db().keys().iter().all(|k| k.len() == 9 || k.len() == 10));
        assert!(store.load(0, 0).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Lecture d'un vrai monde du jeu (`BASALT_MOJANG_WORLD=<dossier>`) : affiche ce qui s'y trouve.
    #[test]
    #[ignore]
    fn reads_a_world_saved_by_the_game() {
        let Ok(dir) = std::env::var("BASALT_MOJANG_WORLD") else { return };
        let data = crate::data::GameData::load(Path::new("data")).unwrap();
        let dir = PathBuf::from(dir);
        let level = read_level(&dir).unwrap().unwrap();
        println!("level.dat : {:?}, nom {:?}, apparition {:?} {:?} {:?}", level_generator(&level), level.str_of("LevelName"), level.get("SpawnX"), level.get("SpawnY"), level.get("SpawnZ"));
        let store = Store::open(&dir, data.table.clone(), false).unwrap();
        let keys = store.db().keys();
        let mut tags: std::collections::BTreeMap<u8, usize> = Default::default();
        let mut chunks = Vec::new();
        for k in &keys {
            if k.len() == 9 || k.len() == 10 {
                *tags.entry(k[8]).or_default() += 1;
                if k.len() == 9 && (k[8] == TAG_VERSION || k[8] == TAG_OLD_VERSION) {
                    chunks.push((i32::from_le_bytes(k[0..4].try_into().unwrap()), i32::from_le_bytes(k[4..8].try_into().unwrap())));
                }
            }
        }
        println!("{} clés, par sorte : {:?}, {} chunks", keys.len(), tags, chunks.len());
        let actors = store.actors();
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for a in &actors {
            *kinds.entry(a.str_of("identifier").unwrap_or("?").to_string()).or_default() += 1;
        }
        println!("{} entités : {:?}", actors.len(), kinds);
        for a in actors.iter().take(2) {
            if let Tag::Compound(entries) = a {
                let keys: Vec<String> = entries.iter().map(|(k, v)| format!("{}={}", k, match v { Tag::Byte(b) => format!("b{}", b), Tag::Int(i) => format!("i{}", i), Tag::Long(l) => format!("l{}", l), Tag::Float(f) => format!("f{}", f), Tag::Short(h) => format!("s{}", h), Tag::List(t, l) => format!("list{}x{}", t, l.len()), Tag::Compound(c) => format!("{{{}}}", c.len()), _ => "?".into() })).collect();
                println!("ACTEUR {}", keys.join(" "));
                println!("ATTR {:?}", a.get("Attributes").map(|t| format!("{:?}", t).chars().take(700).collect::<String>()));
                println!("DEFS {:?}", a.get("definitions"));
            }
        }
        for k in keys.iter().filter(|k| k.starts_with(b"digp")).take(3) {
            println!("DIGP clé {:?} valeur {:?}", k, store.db().get(k).map(|v| v.len()));
        }
        for k in keys.iter().filter(|k| k.starts_with(b"actorprefix")).take(2) {
            println!("ACTORKEY {:?}", k);
        }
        if let Some(a) = actors.first() {
            println!("première : Pos {:?} Rotation {:?} IsBaby {:?} Variant {:?} Color {:?} IsTamed {:?} Health? {:?}", a.get("Pos"), a.get("Rotation"), a.get("IsBaby"), a.get("Variant"), a.get("Color"), a.get("IsTamed"), a.get("Attributes").map(|_| "oui"));
        }
        let other: std::collections::BTreeSet<String> = keys.iter().filter(|k| k.len() != 9 && k.len() != 10 && k.len() != 13 && k.len() != 14).map(|k| String::from_utf8_lossy(&k[..k.len().min(11)]).to_string()).collect();
        println!("autres clés : {:?}", other);
        let mut counts: HashMap<u32, usize> = HashMap::new();
        let (mut wet, mut biomes) = (0, 0);
        for (cx, cz) in &chunks {
            let stored = store.load(*cx, *cz).expect("chunk annoncé par sa clé de version");
            let mut column = Column::from_blocks(&vec![data.table.air; 256 * (MAX_Y - MIN_Y + 1) as usize], data.table.air, super::super::generator::Biome::Plains);
            wet += stored.wet.len();
            biomes += stored.biome_ids.is_some() as usize;
            stored.apply(&mut column, data.table.air);
            for x in 0..16 {
                for z in 0..16 {
                    for y in MIN_Y..=MAX_Y {
                        *counts.entry(column.get(x, y, z)).or_default() += 1;
                    }
                }
            }
        }
        let mut top: Vec<(usize, String)> = counts.iter().map(|(id, n)| (*n, data.block_name(*id).unwrap_or("?").to_string())).collect();
        top.sort_by(|a, b| b.cmp(a));
        println!("blocs : {:?}", &top[..top.len().min(14)]);
        println!("cases avec de l'eau en plus : {}, chunks avec biomes : {}, pertes (inconnus, palettes pleines) : {:?}", wet, biomes, store.losses());
        assert!(!chunks.is_empty());
    }
}

#[cfg(test)]
mod actor_tests {
    use super::*;

    /// Les entités d'un chunk s'écrivent sous les clés du jeu et se relisent ; une nouvelle
    /// écriture remplace celles de Basalt et laisse celles d'une autre origine.
    #[test]
    fn actors_are_stored_the_way_the_game_stores_them() {
        let Ok(data) = crate::data::GameData::load(Path::new("data")) else { return };
        let dir = std::env::temp_dir().join(format!("basalt-mojang-acteurs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir, data.table.clone(), true).unwrap();
        assert_eq!(store.next_actor(), 1);
        let cow = |n: u32| nbt::write_le(&Tag::compound(vec![("identifier", Tag::string("minecraft:cow")), ("UniqueID", Tag::Long(actor_unique_id(n)))]));
        // Une entité écrite par le jeu lui-même dans ce chunk (compteur de sessions 3, numéro 96)
        let foreign = [0u8, 0, 0, 3, 0, 0, 0, 96];
        let mut key = b"actorprefix".to_vec();
        key.extend_from_slice(&foreign);
        store.write(vec![(key.clone(), Some(nbt::write_le(&Tag::compound(vec![("identifier", Tag::string("minecraft:sheep"))])))), (digp_key(2, -7), Some(foreign.to_vec()))]).unwrap();
        assert!(!store.has_actors(2, -7), "celle du jeu n'est pas comptée comme une entité de Basalt");
        store.write_actors(2, -7, &[(1, cow(1)), (2, cow(2))]).unwrap();
        assert!(store.has_actors(2, -7) && !store.has_actors(2, -6));
        assert_eq!(store.next_actor(), 3);
        // La liste du chunk : l'entité du jeu, puis les deux de Basalt, chacune sur huit octets
        let list = store.db().get(&digp_key(2, -7)).unwrap();
        assert_eq!(list.len(), 24);
        assert_eq!(&list[..8], &foreign);
        assert_eq!(&list[8..16], &[0x00, 0x42, 0x41, 0x53, 0, 0, 0, 1]);
        // (même rapport entre la clé et l'identifiant que dans les mondes du jeu : moitié haute
        // en négatif, moitié basse telle quelle)
        assert_eq!(actor_unique_id(1), -(0x0042_4153i64 << 32) + 1);
        let kinds = |s: &Store| {
            let mut k: Vec<String> = s.actors().iter().map(|a| a.str_of("identifier").unwrap_or("?").to_string()).collect();
            k.sort();
            k
        };
        assert_eq!(kinds(&store), ["minecraft:cow", "minecraft:cow", "minecraft:sheep"]);
        // Réécriture : une seule vache désormais ; puis plus aucune
        store.write_actors(2, -7, &[(3, cow(3))]).unwrap();
        assert_eq!(kinds(&store), ["minecraft:cow", "minecraft:sheep"]);
        store.write_actors(2, -7, &[]).unwrap();
        assert_eq!(kinds(&store), ["minecraft:sheep"]);
        assert_eq!(store.db().get(&digp_key(2, -7)).unwrap(), foreign.to_vec());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
