//! Couleur moyenne d'une image PNG ou TGA : de quoi donner à chaque bloc de la carte la
//! couleur de sa texture. Ne lit que ce que les packs de ressources contiennent d'ordinaire
//! (8 bits par couleur, sans entrelacement).

use super::inflate::inflate;

/// Plus grande image lue, en points
const MAX_PIXELS: usize = 1 << 20;

/// Une image décodée : quatre octets par point (rouge, vert, bleu, opacité), du haut vers le bas.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// Reçoit les points d'une image, dans l'ordre du fichier.
#[derive(Default)]
struct Mean {
    rgba: Vec<u8>,
}

impl Mean {
    fn add(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.rgba.extend_from_slice(&[r, g, b, a]);
    }

    fn done(self, w: usize, h: usize) -> Option<Image> {
        (self.rgba.len() == w * h * 4).then_some(Image { w, h, rgba: self.rgba })
    }
}

/// Décode une image PNG ou TGA, ou `None` si elle n'est pas lisible.
pub fn decode(data: &[u8]) -> Option<Image> {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 13, 10, 26, 10]) {
        png(data)
    } else {
        tga(data)
    }
}

/// Couleur moyenne d'une image, ou `None` si elle n'est pas lisible ou entièrement
/// transparente. Les points transparents ne comptent pas.
pub fn average(data: &[u8]) -> Option<[u8; 3]> {
    let image = decode(data)?;
    let (mut sum, mut weight) = ([0u64; 3], 0u64);
    for p in image.rgba.chunks_exact(4).filter(|p| p[3] >= 16) {
        for c in 0..3 {
            sum[c] += p[c] as u64 * p[3] as u64;
        }
        weight += p[3] as u64;
    }
    (weight > 0).then(|| [(sum[0] / weight) as u8, (sum[1] / weight) as u8, (sum[2] / weight) as u8])
}

/// Écrit une image en PNG (couleurs et opacité sur 8 bits).
pub fn to_png(image: &Image) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
            }
        }
        !c
    }
    let mut raw = Vec::with_capacity((image.w * 4 + 1) * image.h);
    for row in image.rgba.chunks(image.w * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    let mut packed = vec![0x78, 0x9c];
    packed.extend_from_slice(&super::deflate::deflate(&raw, super::deflate::Level::Best));
    packed.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut head = Vec::new();
    head.extend_from_slice(&(image.w as u32).to_be_bytes());
    head.extend_from_slice(&(image.h as u32).to_be_bytes());
    head.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    for (name, body) in [(b"IHDR", head), (b"IDAT", packed), (b"IEND", Vec::new())] {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut checked = name.to_vec();
        checked.extend_from_slice(&body);
        out.extend_from_slice(&checked);
        out.extend_from_slice(&crc(&checked).to_be_bytes());
    }
    out
}

fn png(data: &[u8]) -> Option<Image> {
    let (mut at, mut head, mut palette, mut alpha, mut packed) = (8, None, Vec::new(), Vec::new(), Vec::new());
    while at + 8 <= data.len() {
        let len = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let body = data.get(at + 8..at + 8 + len)?;
        match &data[at + 4..at + 8] {
            b"IHDR" if len >= 13 => head = Some((u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize, u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize, body[8], body[9], body[12])),
            b"PLTE" => palette = body.to_vec(),
            b"tRNS" => alpha = body.to_vec(),
            b"IDAT" => packed.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let (w, h, depth, kind, interlace) = head?;
    if w == 0 || h == 0 || w.checked_mul(h)? > MAX_PIXELS || interlace != 0 {
        return None;
    }
    let channels = match kind {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    // (moins de 8 bits : seulement pour le gris et la palette ; 16 bits : pas lu)
    if !(depth == 8 || (depth < 8 && channels == 1 && [1, 2, 4].contains(&depth))) {
        return None;
    }
    let row = (w * channels * depth as usize + 7) / 8;
    let raw = inflate(packed.get(2..)?, (row + 1) * h + 64).ok()?;
    if raw.len() < (row + 1) * h {
        return None;
    }
    let step = if depth == 8 { channels } else { 1 };
    let mut mean = Mean::default();
    let (mut before, mut line) = (vec![0u8; row], vec![0u8; row]);
    for y in 0..h {
        let filter = raw[y * (row + 1)];
        line.copy_from_slice(&raw[y * (row + 1) + 1..(y + 1) * (row + 1)]);
        for i in 0..row {
            let (left, up, corner) = (if i >= step { line[i - step] } else { 0 }, before[i], if i >= step { before[i - step] } else { 0 });
            line[i] = line[i].wrapping_add(match filter {
                1 => left,
                2 => up,
                3 => ((left as u16 + up as u16) / 2) as u8,
                4 => {
                    let p = left as i32 + up as i32 - corner as i32;
                    let (pa, pb, pc) = ((p - left as i32).abs(), (p - up as i32).abs(), (p - corner as i32).abs());
                    if pa <= pb && pa <= pc { left } else if pb <= pc { up } else { corner }
                }
                _ => 0,
            });
        }
        for x in 0..w {
            let sample = |n: usize| -> u8 {
                if depth == 8 {
                    line[x * channels + n]
                } else {
                    let bit = x * depth as usize;
                    (line[bit / 8] >> (8 - depth as usize - bit % 8)) & ((1u8 << depth) - 1)
                }
            };
            match kind {
                0 => {
                    let v = if depth == 8 { sample(0) } else { (sample(0) as u16 * 255 / ((1u16 << depth) - 1)) as u8 };
                    mean.add(v, v, v, 255)
                }
                2 => mean.add(sample(0), sample(1), sample(2), 255),
                3 => {
                    let i = sample(0) as usize;
                    if let Some(c) = palette.get(i * 3..i * 3 + 3) {
                        mean.add(c[0], c[1], c[2], alpha.get(i).copied().unwrap_or(255));
                    }
                }
                4 => mean.add(sample(0), sample(0), sample(0), sample(1)),
                _ => mean.add(sample(0), sample(1), sample(2), sample(3)),
            }
        }
        std::mem::swap(&mut before, &mut line);
    }
    mean.done(w, h)
}

/// TGA en vraies couleurs, brut (type 2) ou compressé par plages (type 10), 24 ou 32 bits.
fn tga(data: &[u8]) -> Option<Image> {
    if data.len() < 18 || data[1] != 0 || !(data[2] == 2 || data[2] == 10) {
        return None;
    }
    let (w, h, bytes) = (u16::from_le_bytes([data[12], data[13]]) as usize, u16::from_le_bytes([data[14], data[15]]) as usize, (data[16] / 8) as usize);
    if w == 0 || h == 0 || w * h > MAX_PIXELS || !(bytes == 3 || bytes == 4) {
        return None;
    }
    let mut mean = Mean::default();
    let mut at = 18 + data[0] as usize;
    let mut left = w * h;
    let mut point = |p: &[u8]| mean.add(p[2], p[1], p[0], if bytes == 4 { p[3] } else { 255 });
    while left > 0 {
        if data[2] == 2 {
            point(data.get(at..at + bytes)?);
            at += bytes;
            left -= 1;
        } else {
            let head = *data.get(at)?;
            let count = ((head & 127) as usize + 1).min(left);
            at += 1;
            if head & 128 != 0 {
                let p = data.get(at..at + bytes)?;
                for _ in 0..count {
                    point(p);
                }
                at += bytes;
            } else {
                for _ in 0..count {
                    point(data.get(at..at + bytes)?);
                    at += bytes;
                }
            }
            left -= count;
        }
    }
    drop(point);
    let mut image = mean.done(w, h)?;
    // (un TGA commence par la ligne du bas, sauf mention contraire)
    if data[17] & 0x20 == 0 {
        let rows: Vec<&[u8]> = image.rgba.chunks(w * 4).rev().collect();
        image.rgba = rows.concat();
    }
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un PNG sans compression (blocs « stockés ») : de quoi essayer chaque format.
    fn png_of(w: u32, h: u32, depth: u8, kind: u8, rows: &[Vec<u8>], extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
        let mut chunk = |name: &[u8; 4], body: &[u8]| {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(body);
            out.extend_from_slice(&[0, 0, 0, 0]);
        };
        let mut head = Vec::new();
        head.extend_from_slice(&w.to_be_bytes());
        head.extend_from_slice(&h.to_be_bytes());
        head.extend_from_slice(&[depth, kind, 0, 0, 0]);
        chunk(b"IHDR", &head);
        for (name, body) in extra {
            chunk(name, body);
        }
        let raw: Vec<u8> = rows.concat();
        let mut z = vec![0x78, 0x01, 1];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(&raw);
        chunk(b"IDAT", &z);
        chunk(b"IEND", &[]);
        out
    }

    #[test]
    fn pngs_average_to_their_visible_colour() {
        // Couleurs avec transparence : le point transparent ne compte pas
        let rgba = png_of(2, 1, 8, 6, &[vec![0, 200, 100, 50, 255, 9, 9, 9, 0]], &[]);
        assert_eq!(average(&rgba), Some([200, 100, 50]));
        // Les filtres : « gauche » sur la première ligne, « haut » sur la seconde
        let filtered = png_of(2, 2, 8, 2, &[vec![1, 10, 20, 30, 10, 20, 30], vec![2, 0, 0, 0, 0, 0, 0]], &[]);
        assert_eq!(average(&filtered), Some([15, 30, 45]));
        // Une palette de deux couleurs sur un bit, la seconde transparente
        let indexed = png_of(8, 1, 1, 3, &[vec![0, 0b0000_1111]], &[(b"PLTE", vec![40, 80, 120, 255, 255, 255]), (b"tRNS", vec![255, 0])]);
        assert_eq!(average(&indexed), Some([40, 80, 120]));
        // Les filtres « Paeth » et « moyenne », sur une image tirée au hasard
        let mixed = png_of(3, 3, 8, 2, &[vec![4, 120, 155, 52, 82, 90, 27, 100, 45, 187], vec![3, 145, 71, 4, 166, 244, 87, 9, 201, 59], vec![4, 160, 121, 103, 30, 86, 199, 45, 94, 106]], &[]);
        assert_eq!(average(&mixed), Some([131, 120, 94]));
        assert_eq!(average(b"pas une image"), None);
    }

    #[test]
    fn tgas_average_too() {
        let mut raw = vec![0u8; 18];
        raw[2] = 2;
        raw[12] = 2;
        raw[14] = 1;
        raw[16] = 32;
        raw.extend_from_slice(&[30, 20, 10, 255, 0, 0, 0, 0]);
        assert_eq!(average(&raw), Some([10, 20, 30]));
        let mut packed = vec![0u8; 18];
        packed[2] = 10;
        packed[12] = 4;
        packed[14] = 1;
        packed[16] = 24;
        packed.extend_from_slice(&[0x83, 90, 60, 30]);
        assert_eq!(average(&packed), Some([30, 60, 90]));
        // Deux lignes : celle du bas est écrite la première
        let mut two = vec![0u8; 18];
        two[2] = 2;
        two[12] = 1;
        two[14] = 2;
        two[16] = 24;
        two.extend_from_slice(&[1, 1, 1, 9, 9, 9]);
        assert_eq!(decode(&two).unwrap().rgba, vec![9, 9, 9, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn a_written_png_reads_back_the_same() {
        let image = Image { w: 3, h: 2, rgba: (0..24).map(|i| (i * 11) as u8).collect() };
        let back = decode(&to_png(&image)).unwrap();
        assert_eq!((back.w, back.h, back.rgba), (3, 2, image.rgba));
    }
}
