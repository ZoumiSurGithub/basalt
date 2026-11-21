//! Compression DEFLATE brute (RFC 1951), celle que le jeu appelle « zlib ».
//!
//! Recherche des répétitions (LZ77, fenêtre de 32 Ko, table de hachage et chaînes de
//! longueur bornée), puis codage de Huffman calculé pour chaque bloc. Réglée pour la
//! vitesse : les chunks et les gros paquets du jeu, très répétitifs, se compriment de 5 à
//! 20 fois en une fraction de milliseconde.

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 4;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
/// Symboles par bloc (un bloc a ses propres codes de Huffman)
const BLOCK_SYMBOLS: usize = 32 * 1024;

/// Effort de recherche : nombre de positions examinées par chaîne.
#[derive(Clone, Copy)]
pub enum Level {
    /// Paquets envoyés depuis la boucle du jeu
    Fast,
    /// Chunks et paquets mis en cache (compressés une fois, envoyés souvent)
    Best,
}

impl Level {
    fn chain(self) -> usize {
        match self {
            Level::Fast => 4,
            Level::Best => 48,
        }
    }
}

// Longueurs : code 257 + i pour les longueurs à partir de LEN_BASE[i], avec LEN_EXTRA[i] bits
const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// Ordre des longueurs du code des longueurs de codes, dans l'en-tête d'un bloc
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Écriture bit à bit, bit de poids faible en premier.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    #[inline]
    fn put(&mut self, value: u32, bits: u32) {
        self.acc |= (value as u64) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Un symbole LZ77 : un octet, ou une copie (longueur, distance).
#[derive(Clone, Copy)]
enum Sym {
    Lit(u8),
    Copy(u16, u16),
}

fn len_code(len: usize) -> usize {
    // Dernier code dont la base ne dépasse pas la longueur
    if len == 258 {
        return 28;
    }
    let mut i = 27;
    while LEN_BASE[i] as usize > len {
        i -= 1;
    }
    i
}

fn dist_code(dist: usize) -> usize {
    let mut i = 29;
    while DIST_BASE[i] as usize > dist {
        i -= 1;
    }
    i
}

/// Longueurs des codes de Huffman pour ces fréquences, sans dépasser `limit` bits.
fn code_lengths(freq: &[u32], limit: u8) -> Vec<u8> {
    let mut freq: Vec<u64> = freq.iter().map(|&f| f as u64).collect();
    loop {
        let lens = huffman_lengths(&freq);
        if lens.iter().all(|&l| l <= limit) {
            return lens;
        }
        // Trop long : on aplatit les fréquences et on recommence
        for f in freq.iter_mut() {
            if *f > 0 {
                *f = (*f >> 1) + 1;
            }
        }
    }
}

fn huffman_lengths(freq: &[u64]) -> Vec<u8> {
    let n = freq.len();
    let mut lens = vec![0u8; n];
    let used: Vec<usize> = (0..n).filter(|&i| freq[i] > 0).collect();
    match used.len() {
        0 => return lens,
        1 => {
            lens[used[0]] = 1;
            return lens;
        }
        _ => {}
    }
    // Nœuds : (poids, parent) ; feuilles d'abord
    let mut weight: Vec<u64> = used.iter().map(|&i| freq[i]).collect();
    let mut parent: Vec<usize> = vec![usize::MAX; used.len()];
    let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<(u64, usize)>> = (0..used.len()).map(|i| std::cmp::Reverse((weight[i], i))).collect();
    while heap.len() > 1 {
        let std::cmp::Reverse((wa, a)) = heap.pop().unwrap_or(std::cmp::Reverse((0, 0)));
        let std::cmp::Reverse((wb, b)) = heap.pop().unwrap_or(std::cmp::Reverse((0, 0)));
        let id = weight.len();
        weight.push(wa + wb);
        parent.push(usize::MAX);
        parent[a] = id;
        parent[b] = id;
        heap.push(std::cmp::Reverse((wa + wb, id)));
    }
    // Profondeur de chaque nœud (les parents sont créés après leurs enfants)
    let mut depth = vec![0u8; weight.len()];
    for i in (0..weight.len()).rev() {
        if parent[i] != usize::MAX {
            depth[i] = depth[parent[i]].saturating_add(1);
        }
    }
    for (k, &sym) in used.iter().enumerate() {
        lens[sym] = depth[k];
    }
    lens
}

/// Codes canoniques (déjà inversés pour l'écriture bit de poids faible en premier).
fn codes(lens: &[u8]) -> Vec<u32> {
    let mut count = [0u32; 16];
    for &l in lens {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for bits in 1..16 {
        code = (code + count[bits - 1]) << 1;
        next[bits] = code;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                return 0;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            // Inversion des `l` bits
            c.reverse_bits() >> (32 - l as u32)
        })
        .collect()
}

/// Compresse `data` en DEFLATE brut.
pub fn deflate(data: &[u8], level: Level) -> Vec<u8> {
    let mut bits = Bits { out: Vec::with_capacity(data.len() / 3 + 64), acc: 0, n: 0 };
    if data.is_empty() {
        // Un bloc fixe vide, final
        bits.put(1, 1);
        bits.put(1, 2);
        bits.put(0, 7);
        return bits.finish();
    }
    let chain = level.chain();
    let mut head = vec![u32::MAX; 1 << HASH_BITS];
    let mut prev = vec![u32::MAX; WINDOW];
    let hash = |p: usize| -> usize {
        let v = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
        (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    };
    let mut syms: Vec<Sym> = Vec::with_capacity(BLOCK_SYMBOLS.min(data.len()) + 1);
    let mut pos = 0usize;
    let insert = |p: usize, head: &mut Vec<u32>, prev: &mut Vec<u32>| {
        if p + MIN_MATCH <= data.len() {
            let h = hash(p);
            prev[p % WINDOW] = head[h];
            head[h] = p as u32;
        }
    };
    while pos < data.len() {
        // Plus longue répétition dans la fenêtre
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if pos + MIN_MATCH <= data.len() {
            let h = hash(pos);
            let mut cand = head[h];
            let max = (data.len() - pos).min(MAX_MATCH);
            let mut tries = chain;
            while cand != u32::MAX && tries > 0 {
                let c = cand as usize;
                if pos - c > WINDOW - 1 {
                    break;
                }
                if data[c + best_len.min(max - 1)] == data[pos + best_len.min(max - 1)] {
                    let mut l = 0;
                    while l < max && data[c + l] == data[pos + l] {
                        l += 1;
                    }
                    if l > best_len {
                        best_len = l;
                        best_dist = pos - c;
                        if l == max {
                            break;
                        }
                    }
                }
                let next = prev[c % WINDOW];
                if next == u32::MAX || next as usize >= c {
                    break;
                }
                cand = next;
                tries -= 1;
            }
        }
        if best_len >= MIN_MATCH {
            syms.push(Sym::Copy(best_len as u16, best_dist as u16));
            // Les positions recouvertes entrent aussi dans la table (moins pour les très longues)
            let step_end = pos + best_len;
            let limit = if best_len > 32 { pos + 32 } else { step_end };
            for p in pos..limit {
                insert(p, &mut head, &mut prev);
            }
            pos = step_end;
        } else {
            syms.push(Sym::Lit(data[pos]));
            insert(pos, &mut head, &mut prev);
            pos += 1;
        }
        if syms.len() >= BLOCK_SYMBOLS {
            write_block(&mut bits, &syms, false);
            syms.clear();
        }
    }
    write_block(&mut bits, &syms, true);
    bits.finish()
}

/// Écrit un bloc à codes de Huffman calculés pour ses symboles.
fn write_block(bits: &mut Bits, syms: &[Sym], last: bool) {
    let mut lit_freq = [0u32; 286];
    let mut dist_freq = [0u32; 30];
    for s in syms {
        match *s {
            Sym::Lit(b) => lit_freq[b as usize] += 1,
            Sym::Copy(l, d) => {
                lit_freq[257 + len_code(l as usize)] += 1;
                dist_freq[dist_code(d as usize)] += 1;
            }
        }
    }
    lit_freq[256] = 1; // fin de bloc
    if dist_freq.iter().all(|&f| f == 0) {
        dist_freq[0] = 1; // au moins un code de distance
    }
    let lit_lens = code_lengths(&lit_freq, 15);
    let dist_lens = code_lengths(&dist_freq, 15);
    let hlit = (257..=286).rev().find(|&n| lit_lens[n - 1] != 0).unwrap_or(257).max(257);
    let hdist = (1..=30).rev().find(|&n| dist_lens[n - 1] != 0).unwrap_or(1).max(1);
    // Longueurs des deux codes, compressées en série (codes 16, 17, 18)
    let mut all: Vec<u8> = lit_lens[..hlit].to_vec();
    all.extend_from_slice(&dist_lens[..hdist]);
    let mut cl_syms: Vec<(u8, u8)> = Vec::new(); // (symbole, bits supplémentaires)
    let mut i = 0;
    while i < all.len() {
        let v = all[i];
        let mut run = 1;
        while i + run < all.len() && all[i + run] == v {
            run += 1;
        }
        if v == 0 && run >= 3 {
            let mut left = run;
            while left >= 11 {
                let n = left.min(138);
                cl_syms.push((18, (n - 11) as u8));
                left -= n;
            }
            if left >= 3 {
                cl_syms.push((17, (left - 3) as u8));
                left = 0;
            }
            for _ in 0..left {
                cl_syms.push((0, 0));
            }
        } else if v != 0 && run >= 4 {
            cl_syms.push((v, 0));
            let mut left = run - 1;
            while left >= 3 {
                let n = left.min(6);
                cl_syms.push((16, (n - 3) as u8));
                left -= n;
            }
            for _ in 0..left {
                cl_syms.push((v, 0));
            }
        } else {
            for _ in 0..run {
                cl_syms.push((v, 0));
            }
        }
        i += run;
    }
    let mut cl_freq = [0u32; 19];
    for (s, _) in &cl_syms {
        cl_freq[*s as usize] += 1;
    }
    let cl_lens = code_lengths(&cl_freq, 7);
    let hclen = (4..=19).rev().find(|&n| cl_lens[CL_ORDER[n - 1]] != 0).unwrap_or(4);
    let (lit_codes, dist_codes, cl_codes) = (codes(&lit_lens), codes(&dist_lens), codes(&cl_lens));

    bits.put(last as u32, 1);
    bits.put(2, 2); // codes de Huffman calculés
    bits.put((hlit - 257) as u32, 5);
    bits.put((hdist - 1) as u32, 5);
    bits.put((hclen - 4) as u32, 4);
    for &k in CL_ORDER.iter().take(hclen) {
        bits.put(cl_lens[k] as u32, 3);
    }
    for (s, extra) in &cl_syms {
        bits.put(cl_codes[*s as usize], cl_lens[*s as usize] as u32);
        match s {
            16 => bits.put(*extra as u32, 2),
            17 => bits.put(*extra as u32, 3),
            18 => bits.put(*extra as u32, 7),
            _ => {}
        }
    }
    for s in syms {
        match *s {
            Sym::Lit(b) => bits.put(lit_codes[b as usize], lit_lens[b as usize] as u32),
            Sym::Copy(l, d) => {
                let lc = len_code(l as usize);
                bits.put(lit_codes[257 + lc], lit_lens[257 + lc] as u32);
                if LEN_EXTRA[lc] > 0 {
                    bits.put(l as u32 - LEN_BASE[lc] as u32, LEN_EXTRA[lc] as u32);
                }
                let dc = dist_code(d as usize);
                bits.put(dist_codes[dc], dist_lens[dc] as u32);
                if DIST_EXTRA[dc] > 0 {
                    bits.put(d as u32 - DIST_BASE[dc] as u32, DIST_EXTRA[dc] as u32);
                }
            }
        }
    }
    bits.put(lit_codes[256], lit_lens[256] as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::inflate::inflate;

    fn round(data: &[u8]) {
        for level in [Level::Fast, Level::Best] {
            let z = deflate(data, level);
            let back = inflate(&z, data.len() + 16).expect("flux illisible");
            assert_eq!(back, data, "aller-retour faux ({} octets)", data.len());
        }
    }

    #[test]
    fn round_trips() {
        round(b"");
        round(b"a");
        round(b"abcabcabcabcabcabcabcabcabcabc");
        round(&vec![0u8; 100_000]);
        let mut r = 12345u64;
        let random: Vec<u8> = (0..200_000)
            .map(|_| {
                r = r.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (r >> 56) as u8
            })
            .collect();
        round(&random);
        // Peu de symboles différents, longues répétitions : comme un chunk
        let text: Vec<u8> = (0..300_000u32).map(|i| b"minecraft:stone"[(i % 15) as usize] ^ ((i / 997) as u8 & 3)).collect();
        round(&text);
        // Toutes les longueurs de copie
        let mut mixed = Vec::new();
        for n in 1..300 {
            mixed.extend(std::iter::repeat((n % 7) as u8).take(n));
            mixed.extend_from_slice(&(n as u32).to_le_bytes());
        }
        round(&mixed);
    }

    #[test]
    fn compresses_repetitive_data() {
        let data: Vec<u8> = (0..65_536u32).map(|i| (i % 64) as u8).collect();
        let z = deflate(&data, Level::Fast);
        assert!(z.len() < data.len() / 50, "{} octets", z.len());
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    /// Vitesse et gain sur de vraies données ; écrit des échantillons dans /tmp pour une
    /// vérification par un autre décompresseur (`cargo test deflate_bench -- --ignored`).
    #[test]
    #[ignore]
    fn deflate_bench() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let data = crate::data::GameData::load(&root.join("data")).unwrap();
        let p = crate::world::generator::Palette::new(&|n| data.state_from_spec(n));
        let mut w = crate::world::World::with_generator(crate::world::generator::Generator::new(crate::world::generator::Kind::Vanilla, 12345, p));
        let mut samples: Vec<(String, Vec<u8>)> = Vec::new();
        for (i, (cx, cz)) in [(0, 0), (5, 3), (-20, 7), (46, -17)].iter().enumerate() {
            let (_, payload) = w.chunk_payload(*cx, *cz);
            samples.push((format!("chunk{}", i), payload));
        }
        let _ = &mut w;
        if let Ok(rd) = std::fs::read_dir(root.join("data/2225/packets")) {
            for e in rd.flatten() {
                if let Ok(b) = std::fs::read(e.path()) {
                    if b.len() > 10_000 {
                        samples.push((e.file_name().to_string_lossy().into_owned(), b));
                    }
                }
            }
        }
        for (name, d) in &samples {
            for (lv, label) in [(Level::Fast, "rapide"), (Level::Best, "fort")] {
                let t = std::time::Instant::now();
                let mut z = Vec::new();
                for _ in 0..5 {
                    z = deflate(d, lv);
                }
                let el = t.elapsed() / 5;
                eprintln!("{:28} {:8} -> {:7} octets ({:4.1} x) en {:?} [{}]", name, d.len(), z.len(), d.len() as f64 / z.len() as f64, el, label);
                std::fs::write(format!("/tmp/deflate-{}-{}.bin", name, label), &z).unwrap();
                std::fs::write(format!("/tmp/deflate-{}.raw", name), d).unwrap();
            }
        }
    }
}
