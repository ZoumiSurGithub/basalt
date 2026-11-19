//! NBT, le format de données de Minecraft.
//!
//! - lecture et écriture du format des fichiers (grand-boutiste, celui des `.nbt` et `.dat`
//!   de PocketMine, lisible par les éditeurs NBT) ;
//! - écriture du format « réseau » de Bedrock (entiers à taille variable) ;
//! - `save_file` / `load_file` : fichiers `.nbt` compressés en gzip, utilisés pour toutes les
//!   sauvegardes du serveur (joueurs, monde, coffres et fours).

use crate::bedrock::W;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    Str(String),
    List(u8, Vec<Tag>),
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::Str(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Tag::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(_, items) => Some(items),
            _ => None,
        }
    }
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
    /// Garde-fou contre les fichiers malformés : nombre total d'éléments lus
    budget: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn i16(&mut self) -> Option<i16> {
        let b = self.take(2)?;
        Some(i16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Option<i32> {
        let b = self.take(4)?;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i64(&mut self) -> Option<i64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Some(i64::from_be_bytes(a))
    }
    fn string(&mut self) -> Option<String> {
        let len = self.i16()? as u16 as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
    fn len(&mut self) -> Option<usize> {
        let n = self.i32()?;
        if n < 0 || n as usize > self.d.len() {
            return None;
        }
        Some(n as usize)
    }
    fn value(&mut self, id: u8, depth: u32) -> Option<Tag> {
        if depth > 64 || self.budget == 0 {
            return None;
        }
        self.budget -= 1;
        Some(match id {
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.to_vec())
            }
            8 => Tag::Str(self.string()?),
            9 => {
                let elem = self.u8()?;
                let n = self.len()?;
                let mut items = Vec::with_capacity(n.min(65_536));
                for _ in 0..n {
                    items.push(self.value(elem, depth + 1)?);
                }
                Tag::List(elem, items)
            }
            10 => {
                let mut entries = Vec::new();
                loop {
                    let t = self.u8()?;
                    if t == 0 {
                        break;
                    }
                    let key = self.string()?;
                    entries.push((key, self.value(t, depth + 1)?));
                }
                Tag::Compound(entries)
            }
            11 => {
                let n = self.len()?;
                let mut v = Vec::with_capacity(n.min(65_536));
                for _ in 0..n {
                    v.push(self.i32()?);
                }
                Tag::IntArray(v)
            }
            12 => {
                let n = self.len()?;
                let mut v = Vec::with_capacity(n.min(65_536));
                for _ in 0..n {
                    v.push(self.i64()?);
                }
                Tag::LongArray(v)
            }
            _ => return None,
        })
    }
}

/// Lit un document NBT grand-boutiste (racine nommée).
pub fn read_be(data: &[u8]) -> Option<Tag> {
    let mut r = Reader { d: data, pos: 0, budget: 4_000_000 };
    let id = r.u8()?;
    r.string()?;
    r.value(id, 0)
}

fn write_value(w: &mut W, tag: &Tag) {
    match tag {
        Tag::Byte(v) => w.u8(*v as u8),
        Tag::Short(v) => w.i16(*v),
        Tag::Int(v) => w.vari32(*v),
        Tag::Long(v) => w.vari64(*v),
        Tag::Float(v) => w.f32(*v),
        Tag::Double(v) => w.0.extend_from_slice(&v.to_le_bytes()),
        Tag::ByteArray(v) => {
            w.vari32(v.len() as i32);
            w.0.extend_from_slice(v);
        }
        Tag::Str(s) => w.string(s),
        Tag::List(elem, items) => {
            w.u8(*elem);
            w.vari32(items.len() as i32);
            for item in items {
                write_value(w, item);
            }
        }
        Tag::Compound(entries) => {
            for (key, value) in entries {
                w.u8(value.id());
                w.string(key);
                write_value(w, value);
            }
            w.u8(0);
        }
        Tag::IntArray(v) => {
            w.vari32(v.len() as i32);
            for x in v {
                w.vari32(*x);
            }
        }
        Tag::LongArray(v) => {
            w.vari32(v.len() as i32);
            for x in v {
                w.vari64(*x);
            }
        }
    }
}

/// Écrit un document NBT au format réseau (racine sans nom).
pub fn write_network(tag: &Tag) -> Vec<u8> {
    let mut w = W::default();
    w.u8(tag.id());
    w.string("");
    write_value(&mut w, tag);
    w.0
}

impl Tag {
    /// Section : `Tag::compound(vec![("Vie", Tag::Float(20.0))])`.
    pub fn compound(entries: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// Liste de sections.
    pub fn list_of(items: Vec<Tag>) -> Tag {
        let elem = items.first().map(|t| t.id()).unwrap_or(0);
        Tag::List(elem, items)
    }

    pub fn string(text: &str) -> Tag {
        Tag::Str(text.to_string())
    }

    pub fn as_byte(&self) -> Option<i8> {
        match self {
            Tag::Byte(v) => Some(*v),
            _ => None,
        }
    }

    /// Nombre entier, quel que soit sa taille dans le fichier.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Tag::Byte(v) => Some(*v as i64),
            Tag::Short(v) => Some(*v as i64),
            Tag::Int(v) => Some(*v as i64),
            Tag::Long(v) => Some(*v),
            _ => None,
        }
    }

    /// Nombre à virgule (ou entier converti).
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Tag::Float(v) => Some(*v),
            Tag::Double(v) => Some(*v as f32),
            other => other.as_i64().map(|v| v as f32),
        }
    }

    pub fn as_int_array(&self) -> Option<&[i32]> {
        match self {
            Tag::IntArray(v) => Some(v),
            _ => None,
        }
    }

    /// Texte d'une clé d'une section.
    pub fn str_of(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|t| t.as_str())
    }
}

fn write_be_name(out: &mut Vec<u8>, name: &str) {
    let bytes = &name.as_bytes()[..name.len().min(u16::MAX as usize)];
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn write_be_value(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            out.extend_from_slice(v);
        }
        Tag::Str(s) => write_be_name(out, s),
        Tag::List(elem, items) => {
            out.push(if items.is_empty() { *elem } else { items[0].id() });
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for item in items {
                write_be_value(out, item);
            }
        }
        Tag::Compound(entries) => {
            for (key, value) in entries {
                out.push(value.id());
                write_be_name(out, key);
                write_be_value(out, value);
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
}

/// Écrit un document NBT au format des fichiers (grand-boutiste, racine sans nom).
pub fn write_be(tag: &Tag) -> Vec<u8> {
    let mut out = vec![tag.id()];
    write_be_name(&mut out, "");
    write_be_value(&mut out, tag);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Enveloppe gzip (blocs non compressés) : le fichier est lisible par tout outil gzip et par
/// les éditeurs NBT, sans que le serveur ait besoin d'un compresseur.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        out.push(if chunks.peek().is_none() { 1 } else { 0 });
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// Taille maximale d'un fichier de sauvegarde une fois décompressé.
const MAX_FILE: usize = 256 * 1024 * 1024;

/// Enregistre un document dans un fichier `.nbt` (gzip). Le fichier est écrit à côté puis
/// renommé : une coupure de courant ne laisse jamais un fichier à moitié écrit.
pub fn save_file(path: &std::path::Path, tag: &Tag) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, gzip(&write_be(tag)))?;
    std::fs::rename(&tmp, path)
}

/// Lit un fichier `.nbt`, compressé ou non. `Ok(None)` si le fichier n'existe pas.
pub fn load_file(path: &std::path::Path) -> Result<Option<Tag>, String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) { crate::data::gunzip(&bytes, MAX_FILE).map_err(|e| e.to_string())? } else { bytes };
    read_be(&raw).map(Some).ok_or_else(|| "fichier NBT illisible".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_big_endian_and_writes_network() {
        // { "idlist": [ { "rid": 257, "id": "a" } ] }
        let mut d = vec![10, 0, 0, 9, 0, 6];
        d.extend_from_slice(b"idlist");
        d.extend_from_slice(&[10, 0, 0, 0, 1]);
        d.extend_from_slice(&[3, 0, 3]);
        d.extend_from_slice(b"rid");
        d.extend_from_slice(&257i32.to_be_bytes());
        d.extend_from_slice(&[8, 0, 2]);
        d.extend_from_slice(b"id");
        d.extend_from_slice(&[0, 1, b'a', 0, 0]);
        let tag = read_be(&d).unwrap();
        let entry = &tag.get("idlist").unwrap().as_list().unwrap()[0];
        assert_eq!(entry.get("rid").unwrap().as_int(), Some(257));
        assert_eq!(entry.get("id").unwrap().as_str(), Some("a"));
        let net = write_network(&tag);
        // racine, liste « idlist » d'un objet, entier 257 en zigzag (0x82 0x04)
        assert_eq!(&net[..3], &[10, 0, 9]);
        assert!(net.windows(2).any(|w| w == [0x82, 0x04]));
        assert_eq!(*net.last().unwrap(), 0);
        // fichiers tronqués ou absurdes : refusés sans planter
        assert!(read_be(&d[..d.len() - 3]).is_none());
        assert!(read_be(&[9, 0, 0, 10, 0x7f, 0xff, 0xff, 0xff]).is_none());
        assert!(read_be(&[]).is_none());
    }

    #[test]
    fn files_round_trip_through_gzip() {
        let doc = Tag::compound(vec![
            ("Name", Tag::string("Enzo")),
            ("Health", Tag::Float(17.5)),
            ("XpLevel", Tag::Int(12)),
            ("LastPlayed", Tag::Long(1_795_000_000)),
            ("Flag", Tag::Byte(1)),
            ("Pos", Tag::List(5, vec![Tag::Float(1.5), Tag::Float(-58.0), Tag::Float(7.0)])),
            ("Inventory", Tag::list_of(vec![Tag::compound(vec![("Slot", Tag::Byte(3)), ("Name", Tag::string("minecraft:stick")), ("Count", Tag::Byte(64))])])),
            ("Vide", Tag::list_of(vec![])),
            ("Blocks", Tag::IntArray(vec![1, -2, 3, i32::MIN])),
            ("Gros", Tag::ByteArray(vec![7; 200_000])),
        ]);
        // Format des fichiers : relu à l'identique
        assert_eq!(read_be(&write_be(&doc)), Some(doc.clone()));
        // Valeur de contrôle connue de gzip
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let packed = gzip(&write_be(&doc));
        assert_eq!(crate::data::gunzip(&packed, 1 << 24).unwrap(), write_be(&doc));

        let dir = std::env::temp_dir().join(format!("basalt-nbt-test-{}", std::process::id()));
        let path = dir.join("essai.nbt");
        assert_eq!(load_file(&path).unwrap(), None);
        save_file(&path, &doc).unwrap();
        let back = load_file(&path).unwrap().unwrap();
        assert_eq!(back, doc);
        assert_eq!((back.str_of("Name"), back.get("Health").and_then(|t| t.as_f32()), back.get("XpLevel").and_then(|t| t.as_i64())), (Some("Enzo"), Some(17.5), Some(12)));
        // Un fichier non compressé est accepté aussi ; un fichier abîmé est signalé
        std::fs::write(&path, write_be(&doc)).unwrap();
        assert_eq!(load_file(&path).unwrap().unwrap(), doc);
        std::fs::write(&path, b"n'importe quoi").unwrap();
        assert!(load_file(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
