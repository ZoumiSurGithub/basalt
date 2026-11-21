//! Encodage de base : entiers à taille variable, écrivain binaire, lots de paquets (0xfe).

#[allow(unused_imports)]
use super::*;
use crate::inflate::inflate;

pub fn read_varu32(d: &[u8], pos: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for shift in (0..35).step_by(7) {
        let b = *d.get(*pos)?;
        *pos += 1;
        v |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

pub fn write_varu32(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

pub(crate) fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varu32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

/// Découpe un lot (sans l'octet 0xfe) en paquets `(identifiant, corps)`.
/// `max` borne la taille après décompression (protection contre les « bombes »).
pub fn decode_batch(d: &[u8], compression: bool, max: usize) -> Result<Vec<(u32, Vec<u8>)>, &'static str> {
    let owned;
    let data: &[u8] = if compression {
        match d.first() {
            Some(0xff) => &d[1..],
            Some(0x00) => {
                owned = inflate(&d[1..], max)?;
                &owned
            }
            Some(_) => return Err("algorithme de compression non pris en charge"),
            None => return Err("lot vide"),
        }
    } else {
        d
    };
    if data.len() > max {
        return Err("lot trop volumineux");
    }
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let len = read_varu32(data, &mut pos).ok_or("longueur de paquet invalide")? as usize;
        let end = pos.checked_add(len).filter(|e| *e <= data.len()).ok_or("paquet tronqué")?;
        let mut p = pos;
        let header = read_varu32(&data[..end], &mut p).ok_or("en-tête de paquet invalide")?;
        out.push((header & 0x3ff, data[p..end].to_vec()));
        pos = end;
        if out.len() > 128 {
            return Err("trop de paquets dans un lot");
        }
    }
    Ok(out)
}

/// Taille à partir de laquelle un lot est compressé (en dessous, la compression coûte plus
/// qu'elle ne rapporte ; le jeu accepte les deux, lot par lot).
pub const COMPRESS_FROM: usize = 512;

/// Compression de ce qui part (réglage `network-compression`, actif par défaut).
pub static COMPRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn compressing() -> bool {
    COMPRESS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Emballe un paquet dans un lot prêt à être envoyé par RakNet.
pub fn encode_packet(id: u32, body: &[u8], compression: bool) -> Vec<u8> {
    encode_packet_with(id, body, compression, crate::utils::deflate::Level::Fast)
}

/// Même chose, avec un effort de compression choisi (`Best` pour ce qui est compressé hors
/// de la boucle du jeu ou une seule fois : chunks, gros paquets mis en cache).
pub fn encode_packet_with(id: u32, body: &[u8], compression: bool, level: crate::utils::deflate::Level) -> Vec<u8> {
    let mut inner = Vec::with_capacity(body.len() + 4);
    write_varu32(&mut inner, id);
    inner.extend_from_slice(body);
    seal_batch(inner, compression, level)
}

/// Contenu d'un lot (paquets précédés de leur longueur) -> lot à envoyer : compressé
/// (« zlib », algorithme 0) s'il est assez gros, sinon marqué non compressé (0xff).
fn seal_batch(inner: Vec<u8>, compression: bool, level: crate::utils::deflate::Level) -> Vec<u8> {
    let mut framed = Vec::with_capacity(inner.len() + 5);
    write_varu32(&mut framed, inner.len() as u32);
    framed.extend_from_slice(&inner);
    if compression && framed.len() >= COMPRESS_FROM && compressing() {
        let z = crate::utils::deflate::deflate(&framed, level);
        if z.len() < framed.len() {
            let mut out = Vec::with_capacity(z.len() + 2);
            out.push(0xfe);
            out.push(0x00);
            out.extend_from_slice(&z);
            return out;
        }
    }
    let mut out = Vec::with_capacity(framed.len() + 2);
    out.push(0xfe);
    if compression {
        out.push(0xff); // lot non compressé
    }
    out.extend_from_slice(&framed);
    out
}
/// Écrivain binaire pour les paquets Bedrock (entiers fixes en petit-boutiste).
#[derive(Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.0.push(v as u8);
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn varu32(&mut self, v: u32) {
        write_varu32(&mut self.0, v);
    }
    pub fn varu64(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(b);
                return;
            }
            self.0.push(b | 0x80);
        }
    }
    /// Entier signé en « zigzag ».
    pub fn vari32(&mut self, v: i32) {
        self.varu32(((v << 1) ^ (v >> 31)) as u32);
    }
    pub fn vari64(&mut self, v: i64) {
        self.varu64(((v << 1) ^ (v >> 63)) as u64);
    }
    pub fn string(&mut self, s: &str) {
        write_string(&mut self.0, s);
    }
}

pub fn read_vari32(d: &[u8], pos: &mut usize) -> Option<i32> {
    let v = read_varu32(d, pos)?;
    Some(((v >> 1) as i32) ^ -((v & 1) as i32))
}


/// Emballe plusieurs paquets dans un seul lot.
pub fn encode_batch(packets: &[(u32, Vec<u8>)], compression: bool) -> Vec<u8> {
    let mut framed = Vec::with_capacity(packets.iter().map(|p| p.1.len() + 8).sum());
    for (id, body) in packets {
        let mut head = Vec::with_capacity(4);
        write_varu32(&mut head, *id);
        write_varu32(&mut framed, (head.len() + body.len()) as u32);
        framed.extend_from_slice(&head);
        framed.extend_from_slice(body);
    }
    let mut out = Vec::with_capacity(framed.len() + 2);
    out.push(0xfe);
    if compression && framed.len() >= COMPRESS_FROM && compressing() {
        let z = crate::utils::deflate::deflate(&framed, crate::utils::deflate::Level::Fast);
        if z.len() < framed.len() {
            out.push(0x00);
            out.extend_from_slice(&z);
            return out;
        }
    }
    if compression {
        out.push(0xff);
    }
    out.extend_from_slice(&framed);
    out
}

pub(crate) fn read_varu64(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..70).step_by(7) {
        let b = *d.get(*pos)?;
        *pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod compression_tests {
    use super::*;

    /// Un gros paquet part compressé (algorithme 0), un petit en clair (0xff) ; les deux se
    /// relisent à l'identique.
    #[test]
    fn big_batches_are_compressed_small_ones_are_not() {
        let big: Vec<u8> = (0..20_000u32).map(|i| (i % 7) as u8).collect();
        let out = encode_packet(58, &big, true);
        assert_eq!(&out[..2], &[0xfe, 0x00]);
        assert!(out.len() < big.len() / 10);
        assert_eq!(decode_batch(&out[1..], true, 1 << 20).unwrap(), vec![(58, big.clone())]);
        let small = encode_packet(9, b"bonjour", true);
        assert_eq!(&small[..2], &[0xfe, 0xff]);
        assert_eq!(decode_batch(&small[1..], true, 1 << 20).unwrap(), vec![(9, b"bonjour".to_vec())]);
        let batch = encode_batch(&[(1, big.clone()), (2, vec![1, 2, 3])], true);
        assert_eq!(batch[1], 0x00);
        assert_eq!(decode_batch(&batch[1..], true, 1 << 20).unwrap(), vec![(1, big), (2, vec![1, 2, 3])]);
        // Avant la négociation (pas de compression) : ni marqueur ni compression
        assert_eq!(&encode_packet(9, b"x", false)[..2], &[0xfe, 2]);
    }
}
