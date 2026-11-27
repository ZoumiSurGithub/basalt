//! Lecture et écriture d'une base LevelDB, le format dans lequel le jeu range ses mondes
//! (dossier `db/`). Écrit ici de zéro, sans bibliothèque : journal (`.log`), tables triées
//! (`.ldb`), fichier MANIFEST et fichier CURRENT, avec les compressions qu'utilise Mojang
//! (zlib, deflate brut) et celle des anciens mondes (snappy, en lecture).
//!
//! Le serveur n'a besoin que de peu : lire une clé, en écrire, en supprimer. Les écritures
//! attendent en mémoire ; `flush` les range dans une nouvelle table de niveau 0 puis réécrit
//! le MANIFEST. Quand les tables de niveau 0 s'accumulent, elles sont fondues en une seule.
//! Une base écrite par le jeu est relue telle quelle, son journal compris.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const BLOCK_SIZE: usize = 32 * 1024;
const TABLE_MAGIC: u64 = 0xdb47_7524_8b80_fb57;
const COMPARATOR: &str = "leveldb.BytewiseComparator";
/// Taille visée d'un bloc de données dans une table
const DATA_BLOCK: usize = 16 * 1024;
/// Tables de niveau 0 tolérées avant de les fondre en une seule
const MAX_LEVEL0: usize = 10;
/// Blocs décodés gardés en mémoire (octets), pour ne pas relire le disque à chaque clé
const CACHE_BYTES: usize = 32 * 1024 * 1024;
/// Garde-fou : taille maximale d'un bloc décompressé
const MAX_BLOCK: usize = 256 * 1024 * 1024;

const NONE: u8 = 0;
const SNAPPY: u8 = 1;
const ZLIB: u8 = 2;
const ZLIB_RAW: u8 = 4;

const TYPE_DELETION: u8 = 0;
const TYPE_VALUE: u8 = 1;

// --------------------------------------------------------------------------------------
// Outils : CRC-32C, entiers à longueur variable
// --------------------------------------------------------------------------------------

fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0x82f6_3b78 } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

/// CRC-32C (Castagnoli) de plusieurs morceaux à la suite.
fn crc32c(parts: &[&[u8]]) -> u32 {
    let t = crc_table();
    let mut c = 0xffff_ffffu32;
    for part in parts {
        for b in part.iter() {
            c = t[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
        }
    }
    !c
}

/// Le CRC tel que LevelDB l'enregistre (« masqué », pour qu'un CRC de CRC reste fiable).
fn mask(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(0xa282_ead8)
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn varint(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *d.get(*pos)?;
        *pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn sized<'a>(d: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let n = varint(d, pos)? as usize;
    let s = d.get(*pos..pos.checked_add(n)?)?;
    *pos += n;
    Some(s)
}

fn put_sized(out: &mut Vec<u8>, s: &[u8]) {
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s);
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

/// Clé interne : la clé, puis (numéro de séquence << 8 | type) sur 8 octets.
fn internal(key: &[u8], seq: u64, kind: u8) -> Vec<u8> {
    let mut k = Vec::with_capacity(key.len() + 8);
    k.extend_from_slice(key);
    k.extend_from_slice(&((seq << 8) | kind as u64).to_le_bytes());
    k
}

/// Sépare une clé interne : (clé, séquence, type).
fn split(ikey: &[u8]) -> Option<(&[u8], u64, u8)> {
    if ikey.len() < 8 {
        return None;
    }
    let (key, tail) = ikey.split_at(ikey.len() - 8);
    let tag = u64::from_le_bytes(tail.try_into().ok()?);
    Some((key, tag >> 8, tag as u8))
}

/// Ordre des clés internes : par clé, puis de la plus récente à la plus ancienne.
fn cmp_internal(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    match (split(a), split(b)) {
        (Some((ka, sa, ta)), Some((kb, sb, tb))) => ka.cmp(kb).then(((sb << 8) | tb as u64).cmp(&((sa << 8) | ta as u64))),
        _ => a.cmp(b),
    }
}

// --------------------------------------------------------------------------------------
// Snappy (lecture seule : les mondes d'avant la 1.2 du jeu)
// --------------------------------------------------------------------------------------

fn unsnappy(d: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0;
    let total = varint(d, &mut pos)? as usize;
    if total > MAX_BLOCK {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(total);
    while pos < d.len() {
        let tag = d[pos];
        pos += 1;
        let (len, offset) = match tag & 3 {
            0 => {
                let mut len = (tag >> 2) as usize;
                if len >= 60 {
                    let extra = len - 59;
                    let bytes = d.get(pos..pos + extra)?;
                    pos += extra;
                    len = bytes.iter().rev().fold(0usize, |acc, b| (acc << 8) | *b as usize);
                }
                let len = len + 1;
                out.extend_from_slice(d.get(pos..pos.checked_add(len)?)?);
                pos += len;
                continue;
            }
            1 => {
                let len = 4 + ((tag >> 2) & 7) as usize;
                let offset = (((tag >> 5) as usize) << 8) | *d.get(pos)? as usize;
                pos += 1;
                (len, offset)
            }
            2 => {
                let offset = u16::from_le_bytes(d.get(pos..pos + 2)?.try_into().ok()?) as usize;
                pos += 2;
                ((tag >> 2) as usize + 1, offset)
            }
            _ => {
                let offset = u32_at(d, pos)? as usize;
                pos += 4;
                ((tag >> 2) as usize + 1, offset)
            }
        };
        if offset == 0 || offset > out.len() || out.len() + len > MAX_BLOCK {
            return None;
        }
        let start = out.len() - offset;
        for i in 0..len {
            out.push(out[start + i]);
        }
    }
    (out.len() == total).then_some(out)
}

// --------------------------------------------------------------------------------------
// Journal : enregistrements découpés en blocs de 32 Ko (fichiers .log et MANIFEST)
// --------------------------------------------------------------------------------------

fn log_records(data: &[u8]) -> Vec<Vec<u8>> {
    let mut records = Vec::new();
    let mut current: Option<Vec<u8>> = None;
    for block in data.chunks(BLOCK_SIZE) {
        let mut pos = 0;
        while pos + 7 <= block.len() {
            let len = u16::from_le_bytes([block[pos + 4], block[pos + 5]]) as usize;
            let kind = block[pos + 6];
            if kind == 0 && len == 0 {
                break; // fin de bloc remplie de zéros
            }
            let Some(body) = block.get(pos + 7..pos + 7 + len) else { break };
            let crc = u32::from_le_bytes([block[pos], block[pos + 1], block[pos + 2], block[pos + 3]]);
            pos += 7 + len;
            if mask(crc32c(&[&[kind], body])) != crc {
                // Enregistrement abîmé (coupure pendant une écriture) : ce qui suit est perdu
                return records;
            }
            match kind {
                1 => {
                    records.push(body.to_vec());
                    current = None;
                }
                2 => current = Some(body.to_vec()),
                3 => {
                    if let Some(c) = current.as_mut() {
                        c.extend_from_slice(body);
                    }
                }
                4 => {
                    if let Some(mut c) = current.take() {
                        c.extend_from_slice(body);
                        records.push(c);
                    }
                }
                _ => {}
            }
        }
    }
    records
}

fn write_log(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        let mut left: &[u8] = record;
        let mut first = true;
        loop {
            let room = BLOCK_SIZE - out.len() % BLOCK_SIZE;
            if room < 7 {
                out.resize(out.len() + room, 0);
                continue;
            }
            let take = left.len().min(room - 7);
            let last = take == left.len();
            let kind = match (first, last) {
                (true, true) => 1u8,
                (true, false) => 2,
                (false, false) => 3,
                (false, true) => 4,
            };
            let body = &left[..take];
            out.extend_from_slice(&mask(crc32c(&[&[kind], body])).to_le_bytes());
            out.extend_from_slice(&(take as u16).to_le_bytes());
            out.push(kind);
            out.extend_from_slice(body);
            left = &left[take..];
            first = false;
            if last {
                break;
            }
        }
    }
    out
}

// --------------------------------------------------------------------------------------
// Tables
// --------------------------------------------------------------------------------------

/// Les entrées d'un bloc, dans l'ordre : (clé complète, valeur).
fn block_entries(block: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    let count = u32_at(block, block.len().checked_sub(4)?)? as usize;
    let end = block.len().checked_sub(4 + count.checked_mul(4)?)?;
    let mut entries = Vec::new();
    let mut pos = 0;
    let mut key: Vec<u8> = Vec::new();
    while pos < end {
        let shared = varint(block, &mut pos)? as usize;
        let fresh = varint(block, &mut pos)? as usize;
        let value_len = varint(block, &mut pos)? as usize;
        if shared > key.len() {
            return None;
        }
        key.truncate(shared);
        key.extend_from_slice(block.get(pos..pos.checked_add(fresh)?)?);
        pos += fresh;
        let value = block.get(pos..pos.checked_add(value_len)?)?;
        pos += value_len;
        if pos > end {
            return None;
        }
        entries.push((key.clone(), value.to_vec()));
    }
    Some(entries)
}

/// Fabrique un bloc. Chaque entrée est un point de reprise (clés écrites en entier) : c'est
/// permis par le format, et les clés du jeu sont courtes.
fn build_block(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut restarts = Vec::with_capacity(entries.len().max(1));
    for (key, value) in entries {
        restarts.push(out.len() as u32);
        put_varint(&mut out, 0);
        put_varint(&mut out, key.len() as u64);
        put_varint(&mut out, value.len() as u64);
        out.extend_from_slice(key);
        out.extend_from_slice(value);
    }
    if restarts.is_empty() {
        restarts.push(0);
    }
    for r in &restarts {
        out.extend_from_slice(&r.to_le_bytes());
    }
    out.extend_from_slice(&(restarts.len() as u32).to_le_bytes());
    out
}

/// Une table ouverte : son index (dernière clé de chaque bloc, position, taille) est en mémoire.
struct Table {
    level: u32,
    number: u64,
    size: u64,
    smallest: Vec<u8>,
    largest: Vec<u8>,
    path: PathBuf,
    index: Vec<(Vec<u8>, u64, u64)>,
}

type Block = Arc<Vec<(Vec<u8>, Vec<u8>)>>;

/// Lit le bloc à cette position d'un fichier de table : vérifie sa somme, le décompresse.
fn read_block(file: &mut std::fs::File, offset: u64, size: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    if size as usize > MAX_BLOCK {
        return Err("bloc trop grand".into());
    }
    let mut raw = vec![0u8; size as usize + 5];
    file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    file.read_exact(&mut raw).map_err(|e| e.to_string())?;
    let (body, trailer) = raw.split_at(size as usize);
    let crc = u32::from_le_bytes([trailer[1], trailer[2], trailer[3], trailer[4]]);
    if mask(crc32c(&[body, &trailer[..1]])) != crc {
        return Err("bloc abîmé (somme de contrôle)".into());
    }
    match trailer[0] {
        NONE => Ok(body.to_vec()),
        SNAPPY => unsnappy(body).ok_or_else(|| "bloc snappy illisible".to_string()),
        ZLIB => crate::utils::inflate::inflate(body.get(2..).unwrap_or(&[]), MAX_BLOCK).map_err(|e| e.to_string()),
        ZLIB_RAW => crate::utils::inflate::inflate(body, MAX_BLOCK).map_err(|e| e.to_string()),
        other => Err(format!("compression inconnue ({})", other)),
    }
}

fn handle(d: &[u8], pos: &mut usize) -> Option<(u64, u64)> {
    Some((varint(d, pos)?, varint(d, pos)?))
}

impl Table {
    fn open(path: PathBuf, level: u32, number: u64, smallest: Vec<u8>, largest: Vec<u8>) -> Result<Table, String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&path).map_err(|e| format!("{} : {}", path.display(), e))?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        if size < 48 {
            return Err(format!("{} : table tronquée", path.display()));
        }
        let mut footer = [0u8; 48];
        file.seek(SeekFrom::Start(size - 48)).map_err(|e| e.to_string())?;
        file.read_exact(&mut footer).map_err(|e| e.to_string())?;
        if u64_at(&footer, 40) != Some(TABLE_MAGIC) {
            return Err(format!("{} : ce n'est pas une table LevelDB", path.display()));
        }
        let mut pos = 0;
        let _meta = handle(&footer, &mut pos).ok_or("pied de table illisible")?;
        let (offset, len) = handle(&footer, &mut pos).ok_or("pied de table illisible")?;
        let block = read_block(&mut file, offset, len)?;
        let mut index = Vec::new();
        for (key, value) in block_entries(&block).ok_or("index de table illisible")? {
            let mut p = 0;
            let (o, s) = handle(&value, &mut p).ok_or("index de table illisible")?;
            index.push((key, o, s));
        }
        Ok(Table { level, number, size, smallest, largest, path, index })
    }
}

/// Écrit une table à partir d'entrées triées (clés internes). Renvoie sa taille.
fn write_table(path: &Path, entries: &[(Vec<u8>, Vec<u8>)], compress: bool) -> std::io::Result<u64> {
    let mut out: Vec<u8> = Vec::new();
    let mut index: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let emit = |out: &mut Vec<u8>, block: Vec<u8>, may_compress: bool| -> Vec<u8> {
        let packed = if may_compress { Some(crate::utils::deflate::deflate(&block, crate::utils::deflate::Level::Fast)) } else { None };
        let (body, kind) = match packed {
            Some(p) if p.len() < block.len() - block.len() / 8 => (p, ZLIB_RAW),
            _ => (block, NONE),
        };
        let mut h = Vec::new();
        put_varint(&mut h, out.len() as u64);
        put_varint(&mut h, body.len() as u64);
        out.extend_from_slice(&body);
        out.push(kind);
        out.extend_from_slice(&mask(crc32c(&[&body, &[kind]])).to_le_bytes());
        h
    };
    let mut start = 0;
    while start < entries.len() {
        let mut end = start;
        let mut bytes = 0;
        while end < entries.len() && (bytes < DATA_BLOCK || end == start) {
            bytes += entries[end].0.len() + entries[end].1.len() + 8;
            end += 1;
        }
        let h = emit(&mut out, build_block(&entries[start..end]), compress);
        index.push((entries[end - 1].0.clone(), h));
        start = end;
    }
    let meta = emit(&mut out, build_block(&[]), false);
    let idx = emit(&mut out, build_block(&index), false);
    let mut footer = Vec::with_capacity(48);
    footer.extend_from_slice(&meta);
    footer.extend_from_slice(&idx);
    footer.resize(40, 0);
    footer.extend_from_slice(&TABLE_MAGIC.to_le_bytes());
    out.extend_from_slice(&footer);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &out)?;
    std::fs::rename(&tmp, path)?;
    Ok(out.len() as u64)
}

// --------------------------------------------------------------------------------------
// La base
// --------------------------------------------------------------------------------------

pub struct Db {
    dir: PathBuf,
    tables: Vec<Table>,
    /// Écritures en attente : clé -> valeur (ou suppression)
    mem: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    next_file: u64,
    last_seq: u64,
    manifest: u64,
    /// Journaux du jeu relus à l'ouverture : supprimés une fois leur contenu rangé dans une table
    old_logs: Vec<PathBuf>,
    cache: HashMap<(u64, u64), Block>,
    cache_bytes: usize,
    /// Compresse les blocs écrits (deflate brut, comme le jeu)
    pub compress: bool,
}

impl Db {
    /// Y a-t-il une base dans ce dossier ?
    pub fn exists(dir: &Path) -> bool {
        dir.join("CURRENT").exists()
    }

    /// Ouvre la base de ce dossier, créée si elle n'existe pas.
    pub fn open(dir: &Path) -> Result<Db, String> {
        let mut db = Db { dir: dir.to_path_buf(), tables: Vec::new(), mem: BTreeMap::new(), next_file: 2, last_seq: 0, manifest: 0, old_logs: Vec::new(), cache: HashMap::new(), cache_bytes: 0, compress: true };
        let current = match std::fs::read_to_string(dir.join("CURRENT")) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                db.write_manifest().map_err(|e| e.to_string())?;
                return Ok(db);
            }
            Err(e) => return Err(e.to_string()),
        };
        let name = current.trim();
        if !name.starts_with("MANIFEST-") || name.contains(['/', '\\']) {
            return Err("fichier CURRENT illisible".into());
        }
        db.manifest = name["MANIFEST-".len()..].parse().unwrap_or(0);
        let data = std::fs::read(dir.join(name)).map_err(|e| format!("{} : {}", name, e))?;
        // Le MANIFEST est une suite de modifications : fichiers ajoutés, fichiers retirés
        let mut files: BTreeMap<u64, (u32, Vec<u8>, Vec<u8>)> = BTreeMap::new();
        let (mut log_number, mut prev_log) = (0u64, 0u64);
        for record in log_records(&data) {
            let mut pos = 0;
            while pos < record.len() {
                let bad = || "MANIFEST illisible".to_string();
                match varint(&record, &mut pos).ok_or_else(bad)? {
                    1 => {
                        let name = sized(&record, &mut pos).ok_or_else(bad)?;
                        if name != COMPARATOR.as_bytes() {
                            return Err(format!("ordre de tri inconnu : {}", String::from_utf8_lossy(name)));
                        }
                    }
                    2 => log_number = varint(&record, &mut pos).ok_or_else(bad)?,
                    9 => prev_log = varint(&record, &mut pos).ok_or_else(bad)?,
                    3 => db.next_file = varint(&record, &mut pos).ok_or_else(bad)?,
                    4 => db.last_seq = varint(&record, &mut pos).ok_or_else(bad)?,
                    5 => {
                        varint(&record, &mut pos).ok_or_else(bad)?;
                        sized(&record, &mut pos).ok_or_else(bad)?;
                    }
                    6 => {
                        varint(&record, &mut pos).ok_or_else(bad)?;
                        let number = varint(&record, &mut pos).ok_or_else(bad)?;
                        files.remove(&number);
                    }
                    7 => {
                        let level = varint(&record, &mut pos).ok_or_else(bad)? as u32;
                        let number = varint(&record, &mut pos).ok_or_else(bad)?;
                        varint(&record, &mut pos).ok_or_else(bad)?;
                        let smallest = sized(&record, &mut pos).ok_or_else(bad)?.to_vec();
                        let largest = sized(&record, &mut pos).ok_or_else(bad)?.to_vec();
                        files.insert(number, (level, smallest, largest));
                    }
                    _ => return Err(bad()),
                }
            }
        }
        for (number, (level, smallest, largest)) in files {
            let path = [format!("{:06}.ldb", number), format!("{:06}.sst", number)].iter().map(|n| dir.join(n)).find(|p| p.exists()).ok_or_else(|| format!("table {:06} absente", number))?;
            db.tables.push(Table::open(path, level, number, smallest, largest)?);
            db.next_file = db.next_file.max(number + 1);
        }
        // Journaux : les dernières écritures du jeu, pas encore rangées dans une table
        let mut logs: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let number = path.file_name()?.to_str()?.strip_suffix(".log")?.parse::<u64>().ok()?;
                Some((number, path))
            })
            .collect();
        logs.sort();
        for (number, path) in logs {
            db.next_file = db.next_file.max(number + 1);
            if number >= log_number || (prev_log != 0 && number == prev_log) {
                let data = std::fs::read(&path).map_err(|e| e.to_string())?;
                for batch in log_records(&data) {
                    db.replay(&batch);
                }
            }
            db.old_logs.push(path);
        }
        Ok(db)
    }

    /// Rejoue un lot d'écritures du journal.
    fn replay(&mut self, batch: &[u8]) {
        let (Some(seq), Some(count)) = (u64_at(batch, 0), u32_at(batch, 8)) else { return };
        let mut pos = 12;
        for _ in 0..count {
            let Some(kind) = batch.get(pos).copied() else { return };
            pos += 1;
            let Some(key) = sized(batch, &mut pos) else { return };
            match kind {
                TYPE_VALUE => {
                    let Some(value) = sized(batch, &mut pos) else { return };
                    self.mem.insert(key.to_vec(), Some(value.to_vec()));
                }
                _ => {
                    self.mem.insert(key.to_vec(), None);
                }
            }
        }
        self.last_seq = self.last_seq.max(seq + count as u64);
    }

    fn block(&mut self, table: usize, offset: u64, size: u64) -> Result<Block, String> {
        let key = (self.tables[table].number, offset);
        if let Some(b) = self.cache.get(&key) {
            return Ok(b.clone());
        }
        let mut file = std::fs::File::open(&self.tables[table].path).map_err(|e| e.to_string())?;
        let raw = read_block(&mut file, offset, size)?;
        let entries = Arc::new(block_entries(&raw).ok_or("bloc de table illisible")?);
        if self.cache_bytes + raw.len() > CACHE_BYTES {
            self.cache.clear();
            self.cache_bytes = 0;
        }
        self.cache_bytes += raw.len();
        self.cache.insert(key, entries.clone());
        Ok(entries)
    }

    /// Cherche une clé dans une table : `Some(None)` si elle y est marquée supprimée.
    fn get_in(&mut self, table: usize, key: &[u8]) -> Option<Option<Vec<u8>>> {
        let target = internal(key, u64::MAX >> 8, TYPE_VALUE);
        let at = self.tables[table].index.partition_point(|(last, _, _)| cmp_internal(last, &target) == std::cmp::Ordering::Less);
        let (_, offset, size) = self.tables[table].index.get(at)?.clone();
        let block = self.block(table, offset, size).ok()?;
        let i = block.partition_point(|(k, _)| cmp_internal(k, &target) == std::cmp::Ordering::Less);
        let (found, value) = block.get(i)?;
        let (user, _, kind) = split(found)?;
        if user != key {
            return None;
        }
        Some((kind == TYPE_VALUE).then(|| value.clone()))
    }

    /// Valeur d'une clé (la plus récente), ou rien.
    pub fn get(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(v) = self.mem.get(key) {
            return v.clone();
        }
        // Niveau 0 : les tables se chevauchent, la plus récente gagne. Ensuite, une table au
        // plus par niveau peut contenir la clé.
        let mut order: Vec<usize> = (0..self.tables.len()).filter(|i| self.tables[*i].covers(key)).collect();
        order.sort_by_key(|i| (self.tables[*i].level, std::cmp::Reverse(self.tables[*i].number)));
        for table in order {
            if let Some(found) = self.get_in(table, key) {
                return found;
            }
        }
        None
    }

    pub fn put(&mut self, key: &[u8], value: Vec<u8>) {
        self.mem.insert(key.to_vec(), Some(value));
    }

    pub fn delete(&mut self, key: &[u8]) {
        self.mem.insert(key.to_vec(), None);
    }

    /// Des écritures attendent-elles d'être rangées sur le disque ?
    pub fn pending(&self) -> bool {
        !self.mem.is_empty()
    }

    /// Toutes les clés présentes (lent : relit toute la base ; pour les outils et les tests).
    pub fn keys(&mut self) -> Vec<Vec<u8>> {
        let mut all: BTreeMap<Vec<u8>, (u64, bool)> = BTreeMap::new();
        for t in 0..self.tables.len() {
            for b in 0..self.tables[t].index.len() {
                let (_, offset, size) = self.tables[t].index[b].clone();
                let Ok(block) = self.block(t, offset, size) else { continue };
                for (ikey, _) in block.iter() {
                    if let Some((user, seq, kind)) = split(ikey) {
                        let newer = all.get(user).map(|(s, _)| seq >= *s).unwrap_or(true);
                        if newer {
                            all.insert(user.to_vec(), (seq, kind == TYPE_VALUE));
                        }
                    }
                }
            }
        }
        for (key, value) in &self.mem {
            all.insert(key.clone(), (u64::MAX, value.is_some()));
        }
        all.into_iter().filter(|(_, (_, live))| *live).map(|(k, _)| k).collect()
    }

    /// Range les écritures en attente dans une nouvelle table, puis réécrit le MANIFEST.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if self.mem.is_empty() {
            return Ok(());
        }
        let mem = std::mem::take(&mut self.mem);
        let mut entries = Vec::with_capacity(mem.len());
        for (key, value) in mem {
            self.last_seq += 1;
            match value {
                Some(v) => entries.push((internal(&key, self.last_seq, TYPE_VALUE), v)),
                None => entries.push((internal(&key, self.last_seq, TYPE_DELETION), Vec::new())),
            }
        }
        self.add_table(entries)?;
        if self.tables.iter().filter(|t| t.level == 0).count() > MAX_LEVEL0 {
            self.merge_level0()?;
        }
        self.write_manifest()?;
        for log in std::mem::take(&mut self.old_logs) {
            let _ = std::fs::remove_file(log);
        }
        Ok(())
    }

    fn add_table(&mut self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> std::io::Result<()> {
        let number = self.next_file;
        self.next_file += 1;
        let path = self.dir.join(format!("{:06}.ldb", number));
        write_table(&path, &entries, self.compress)?;
        let (smallest, largest) = (entries[0].0.clone(), entries[entries.len() - 1].0.clone());
        let table = Table::open(path, 0, number, smallest, largest).map_err(std::io::Error::other)?;
        self.tables.push(table);
        Ok(())
    }

    /// Fond toutes les tables de niveau 0 en une seule (la version la plus récente de chaque
    /// clé est gardée ; les suppressions aussi, car un niveau plus profond peut encore avoir
    /// la clé).
    fn merge_level0(&mut self) -> std::io::Result<()> {
        let mut newest: BTreeMap<Vec<u8>, (u64, u8, Vec<u8>)> = BTreeMap::new();
        let level0: Vec<usize> = (0..self.tables.len()).filter(|i| self.tables[*i].level == 0).collect();
        for t in &level0 {
            for b in 0..self.tables[*t].index.len() {
                let (_, offset, size) = self.tables[*t].index[b].clone();
                let block = self.block(*t, offset, size).map_err(std::io::Error::other)?;
                for (ikey, value) in block.iter() {
                    let Some((user, seq, kind)) = split(ikey) else { continue };
                    if newest.get(user).map(|(s, _, _)| seq > *s).unwrap_or(true) {
                        newest.insert(user.to_vec(), (seq, kind, value.clone()));
                    }
                }
            }
        }
        let entries: Vec<(Vec<u8>, Vec<u8>)> = newest.into_iter().map(|(key, (seq, kind, value))| (internal(&key, seq, kind), value)).collect();
        let old: Vec<PathBuf> = level0.iter().map(|t| self.tables[*t].path.clone()).collect();
        self.tables.retain(|t| t.level != 0);
        self.cache.clear();
        self.cache_bytes = 0;
        if !entries.is_empty() {
            self.add_table(entries)?;
        }
        self.write_manifest()?;
        for path in old {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    /// Écrit un MANIFEST complet (l'état entier de la base), puis fait pointer CURRENT dessus.
    fn write_manifest(&mut self) -> std::io::Result<()> {
        let number = self.next_file;
        self.next_file += 1;
        let mut edit = Vec::new();
        put_varint(&mut edit, 1);
        put_sized(&mut edit, COMPARATOR.as_bytes());
        // (aucun journal : tout ce qui est écrit l'est dans des tables)
        put_varint(&mut edit, 2);
        put_varint(&mut edit, self.next_file);
        put_varint(&mut edit, 3);
        put_varint(&mut edit, self.next_file + 1);
        put_varint(&mut edit, 4);
        put_varint(&mut edit, self.last_seq);
        for t in &self.tables {
            put_varint(&mut edit, 7);
            put_varint(&mut edit, t.level as u64);
            put_varint(&mut edit, t.number);
            put_varint(&mut edit, t.size);
            put_sized(&mut edit, &t.smallest);
            put_sized(&mut edit, &t.largest);
        }
        self.next_file += 1;
        let name = format!("MANIFEST-{:06}", number);
        std::fs::write(self.dir.join(&name), write_log(&[edit]))?;
        let tmp = self.dir.join("CURRENT.tmp");
        std::fs::write(&tmp, format!("{}\n", name))?;
        std::fs::rename(&tmp, self.dir.join("CURRENT"))?;
        if self.manifest != 0 && self.manifest != number {
            let _ = std::fs::remove_file(self.dir.join(format!("MANIFEST-{:06}", self.manifest)));
        }
        self.manifest = number;
        Ok(())
    }
}

impl Table {
    /// La clé peut-elle se trouver dans cette table ?
    fn covers(&self, key: &[u8]) -> bool {
        let low = split(&self.smallest).map(|(k, _, _)| k).unwrap_or(&[]);
        let high = split(&self.largest).map(|(k, _, _)| k).unwrap_or(&[]);
        key >= low && key <= high
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("basalt-ldb-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn checksums_match_the_reference_values() {
        // Valeurs de référence du CRC-32C (RFC 3720)
        assert_eq!(crc32c(&[&[0u8; 32]]), 0x8a91_36aa);
        assert_eq!(crc32c(&[&[0xffu8; 32]]), 0x62a8_ab43);
        assert_eq!(crc32c(&[b"1234", b"56789"]), 0xe306_9283);
    }

    #[test]
    fn snappy_blocks_are_read() {
        // « aaaaaaaaaa » : un littéral « a », puis une copie de 9 octets à distance 1
        let packed = [10u8, 0, b'a', (9 - 4) << 2 | 1, 1];
        assert_eq!(unsnappy(&packed).unwrap(), b"aaaaaaaaaa");
        assert!(unsnappy(&[10, 0, b'a']).is_none(), "longueur annoncée non atteinte");
    }

    #[test]
    fn long_records_are_split_across_blocks_and_read_back() {
        let records = vec![vec![1u8; 10], vec![2u8; 70_000], Vec::new(), vec![3u8; BLOCK_SIZE - 7]];
        let log = write_log(&records);
        assert_eq!(log_records(&log), records);
        // Une fin de fichier abîmée ne fait perdre que ce qui la suit
        let mut cut = log.clone();
        let n = cut.len();
        cut[n - 3] ^= 0xff;
        assert_eq!(log_records(&cut), records[..3].to_vec());
    }

    #[test]
    fn values_survive_flushes_reopening_and_merges() {
        for compress in [true, false] {
            let dir = temp(if compress { "zip" } else { "brut" });
            let mut db = Db::open(&dir).unwrap();
            db.compress = compress;
            let key = |i: u32| format!("cle-{:05}", i).into_bytes();
            let value = |i: u32, round: u32| vec![(i % 251) as u8 ^ round as u8; 40 + (i % 900) as usize];
            // Plusieurs séries d'écritures : chaque `flush` ajoute une table, jusqu'à la fusion
            for round in 0..14 {
                for i in (0..3000).filter(|i| i % 14 >= round % 14 || round == 0) {
                    db.put(&key(i), value(i, round));
                }
                db.delete(&key(round));
                db.flush().unwrap();
            }
            assert!(db.tables.iter().filter(|t| t.level == 0).count() <= MAX_LEVEL0, "les tables de niveau 0 sont fondues");
            drop(db);
            let mut db = Db::open(&dir).unwrap();
            for i in 0..3000u32 {
                let last = (0..14).filter(|round| i % 14 >= round % 14 || *round == 0).max().unwrap();
                let expected = if i < 14 { None } else { Some(value(i, last)) };
                assert_eq!(db.get(&key(i)), expected, "clé {} (compression {})", i, compress);
            }
            assert_eq!(db.get(b"absente"), None);
            assert_eq!(db.keys().len(), 3000 - 14);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_log_left_by_the_game_is_replayed() {
        let dir = temp("journal");
        let mut db = Db::open(&dir).unwrap();
        db.put(b"ancienne", b"table".to_vec());
        db.put(b"effacee", b"x".to_vec());
        db.flush().unwrap();
        drop(db);
        // Le jeu a écrit après coup dans son journal : une valeur, une suppression
        let mut batch = Vec::new();
        batch.extend_from_slice(&100u64.to_le_bytes());
        batch.extend_from_slice(&2u32.to_le_bytes());
        batch.push(TYPE_VALUE);
        put_sized(&mut batch, b"recente");
        put_sized(&mut batch, b"journal");
        batch.push(TYPE_DELETION);
        put_sized(&mut batch, b"effacee");
        std::fs::write(dir.join("000099.log"), write_log(&[batch])).unwrap();
        let mut db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"recente"), Some(b"journal".to_vec()));
        assert_eq!(db.get(b"ancienne"), Some(b"table".to_vec()));
        assert_eq!(db.get(b"effacee"), None);
        // Rangé dans une table, le journal n'a plus lieu d'être
        db.flush().unwrap();
        assert!(!dir.join("000099.log").exists());
        drop(db);
        let mut db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"recente"), Some(b"journal".to_vec()));
        assert_eq!(db.get(b"effacee"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Vérification croisée avec le LevelDB de référence (outil C++ à part, voir
    /// `tests/leveldb/README.md`) : `BASALT_LDB_WRITE` reçoit une base écrite par Basalt,
    /// `BASALT_LDB_READ` désigne une base écrite par la référence.
    #[test]
    #[ignore]
    fn cross_check_with_the_reference_implementation() {
        let key = |i: u32| format!("k{:06}", i).into_bytes();
        let value = |i: u32| format!("valeur-{}-{}", i, "x".repeat((i % 300) as usize)).into_bytes();
        if let Ok(dir) = std::env::var("BASALT_LDB_WRITE") {
            let dir = PathBuf::from(dir);
            let _ = std::fs::remove_dir_all(&dir);
            let mut db = Db::open(&dir).unwrap();
            db.compress = false;
            for round in 0..3u32 {
                for i in (0..5000).filter(|i| i % 3 == round) {
                    db.put(&key(i), value(i));
                }
                db.flush().unwrap();
            }
            for i in 0..50 {
                db.delete(&key(i * 100));
            }
            db.flush().unwrap();
        }
        if let Ok(dir) = std::env::var("BASALT_LDB_READ") {
            let mut db = Db::open(Path::new(&dir)).unwrap();
            for i in 0..20_000u32 {
                let expected = if i % 7 == 0 { None } else { Some(value(i)) };
                assert_eq!(db.get(&key(i)), expected, "clé {}", i);
            }
            assert_eq!(db.keys().len(), (0..20_000).filter(|i| i % 7 != 0).count());
            // Et Basalt y ajoute sa part, que la référence devra relire
            db.compress = false;
            db.put(b"basalt", b"etait ici".to_vec());
            db.delete(&key(1));
            db.flush().unwrap();
        }
    }
}
