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

/// Emballe un paquet dans un lot prêt à être envoyé par RakNet.
pub fn encode_packet(id: u32, body: &[u8], compression: bool) -> Vec<u8> {
    let mut inner = Vec::with_capacity(body.len() + 4);
    write_varu32(&mut inner, id);
    inner.extend_from_slice(body);
    let mut out = Vec::with_capacity(inner.len() + 8);
    out.push(0xfe);
    if compression {
        out.push(0xff); // lot non compressé
    }
    write_varu32(&mut out, inner.len() as u32);
    out.extend_from_slice(&inner);
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
    let mut out = vec![0xfe];
    if compression {
        out.push(0xff);
    }
    for (id, body) in packets {
        let mut head = Vec::with_capacity(4);
        write_varu32(&mut head, *id);
        write_varu32(&mut out, (head.len() + body.len()) as u32);
        out.extend_from_slice(&head);
        out.extend_from_slice(body);
    }
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
