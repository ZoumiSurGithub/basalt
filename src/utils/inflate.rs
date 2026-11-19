//! Décompression DEFLATE brute (RFC 1951), utilisée pour les paquets Bedrock compressés.

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn bits(&mut self, n: u32) -> Result<u32, &'static str> {
        while self.cnt < n {
            let b = *self.data.get(self.pos).ok_or("flux compressé tronqué")?;
            self.pos += 1;
            self.buf |= (b as u32) << self.cnt;
            self.cnt += 8;
        }
        let v = self.buf & ((1u32 << n) - 1);
        self.buf >>= n;
        self.cnt -= n;
        Ok(v)
    }
}

struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut offs = [0u16; 16];
        for i in 1..15 {
            offs[i + 1] = offs[i] + count[i];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huffman { count, symbol }
    }

    fn decode(&self, b: &mut Bits) -> Result<u16, &'static str> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return self
                    .symbol
                    .get((index + code - first) as usize)
                    .copied()
                    .ok_or("code de Huffman invalide");
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("code de Huffman invalide")
    }
}

const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn codes(
    b: &mut Bits,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
    limit: usize,
) -> Result<(), &'static str> {
    loop {
        let sym = lit.decode(b)? as usize;
        if sym < 256 {
            if out.len() >= limit {
                return Err("données décompressées trop volumineuses");
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return Err("longueur invalide");
            }
            let len = LBASE[s] as usize + b.bits(LEXT[s] as u32)? as usize;
            let d = dist.decode(b)? as usize;
            if d >= 30 {
                return Err("distance invalide");
            }
            let back = DBASE[d] as usize + b.bits(DEXT[d] as u32)? as usize;
            if back > out.len() {
                return Err("distance hors limites");
            }
            if out.len() + len > limit {
                return Err("données décompressées trop volumineuses");
            }
            let start = out.len() - back;
            for i in 0..len {
                let v = out[start + i];
                out.push(v);
            }
        }
    }
}

/// Décompresse `data` ; échoue si la sortie dépasse `limit` octets.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    let mut b = Bits { data, pos: 0, buf: 0, cnt: 0 };
    let mut out = Vec::new();
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.buf = 0;
                b.cnt = 0;
                let hdr = data.get(b.pos..b.pos + 4).ok_or("flux compressé tronqué")?;
                let len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                let nlen = u16::from_le_bytes([hdr[2], hdr[3]]) as usize;
                if len != (!nlen & 0xffff) {
                    return Err("bloc stocké invalide");
                }
                b.pos += 4;
                let raw = data.get(b.pos..b.pos + len).ok_or("flux compressé tronqué")?;
                if out.len() + len > limit {
                    return Err("données décompressées trop volumineuses");
                }
                out.extend_from_slice(raw);
                b.pos += len;
            }
            1 => {
                let mut l = [8u8; 288];
                for v in l.iter_mut().take(256).skip(144) {
                    *v = 9;
                }
                for v in l.iter_mut().take(280).skip(256) {
                    *v = 7;
                }
                codes(&mut b, &mut out, &Huffman::new(&l), &Huffman::new(&[5u8; 30]), limit)?;
            }
            2 => {
                let nlen = b.bits(5)? as usize + 257;
                let ndist = b.bits(5)? as usize + 1;
                let ncode = b.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("en-tête dynamique invalide");
                }
                let mut cl = [0u8; 19];
                for &o in ORDER.iter().take(ncode) {
                    cl[o] = b.bits(3)? as u8;
                }
                let lencode = Huffman::new(&cl);
                let mut lengths = vec![0u8; nlen + ndist];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = lencode.decode(&mut b)?;
                    if sym < 16 {
                        lengths[i] = sym as u8;
                        i += 1;
                    } else {
                        let (val, rep) = match sym {
                            16 => {
                                if i == 0 {
                                    return Err("répétition invalide");
                                }
                                (lengths[i - 1], 3 + b.bits(2)? as usize)
                            }
                            17 => (0, 3 + b.bits(3)? as usize),
                            _ => (0, 11 + b.bits(7)? as usize),
                        };
                        if i + rep > nlen + ndist {
                            return Err("répétition invalide");
                        }
                        for _ in 0..rep {
                            lengths[i] = val;
                            i += 1;
                        }
                    }
                }
                let lit = Huffman::new(&lengths[..nlen]);
                let dist = Huffman::new(&lengths[nlen..]);
                codes(&mut b, &mut out, &lit, &dist, limit)?;
            }
            _ => return Err("type de bloc invalide"),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inflate;

    #[test]
    fn stored_block() {
        let data = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&data, 100).unwrap(), b"abc");
    }

    #[test]
    fn limit_is_enforced() {
        let data = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert!(inflate(&data, 2).is_err());
    }

    #[test]
    fn garbage_does_not_panic() {
        for seed in 0..2000u32 {
            let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
            let data: Vec<u8> = (0..64)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let _ = inflate(&data, 1 << 16);
        }
    }
}

#[cfg(test)]
mod reference {
    use super::inflate;

    /// Flux produits par zlib (blocs à codes fixes puis dynamiques).
    #[test]
    fn fixed_huffman() {
        let data = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01];
        assert_eq!(inflate(&data, 100).unwrap(), b"hello hello hello hello");
    }

    #[test]
    fn dynamic_huffman() {
        let data = include_bytes!("../../tests/dyn.bin");
        let expected = include_bytes!("../../tests/dyn.txt");
        assert_eq!(inflate(data, 1 << 20).unwrap(), expected.to_vec());
    }
}
