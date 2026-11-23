//! basalt-bots : des joueurs simulés qui se connectent au serveur comme de vrais jeux
//! (RakNet, login, terrain reçu, paquets de mouvement) et se promènent comme des joueurs :
//! ils marchent, courent, sautent les marches, contournent les arbres et les murs, évitent
//! les falaises, l'eau et la lave, s'arrêtent pour regarder autour d'eux.
//!
//!   basalt-bots <nombre> [adresse[:port]] [rayon]
//!
//! - nombre : joueurs simulés (Bot…), connectés à raison de 4 par seconde ;
//! - adresse : 127.0.0.1:19132 par défaut ;
//! - rayon : distance (en blocs) jusqu'où ils s'éloignent de leur point d'arrivée (150).
//!
//! Côté serveur, dans server.properties : local-bots=true. Les bots doivent tourner sur la
//! même machine que le serveur (adresse 127.x.x.x) : ils n'ont pas de compte Xbox.
//! Les données du jeu sont lues dans ./data (ou BASALT_DATA=chemin).
//!
//! Outil privé, non publié.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use basalt::bedrock::{read_varu32, write_varu32};
use basalt::data::GameData;
use basalt::raknet::MAGIC;

const PROTOCOL: i32 = 2225;
const EYE: f64 = 1.62;
const HALF_WIDTH: f64 = 0.3;
const HEIGHT: f64 = 1.8;
const WALK: f64 = 4.317 / 20.0;
const SPRINT: f64 = 5.612 / 20.0;
const MIN_Y: i32 = -64;
const SECTIONS: usize = 24;

// ---------------------------------------------------------------------------------------
// Petits outils
// ---------------------------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
}

fn read_vari32(d: &[u8], pos: &mut usize) -> Option<i32> {
    let v = read_varu32(d, pos)?;
    Some(((v >> 1) as i32) ^ -((v & 1) as i32))
}

fn read_varu64(d: &[u8], pos: &mut usize) -> Option<u64> {
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

fn write_varu64(out: &mut Vec<u8>, mut v: u64) {
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

fn f32_at(d: &[u8], at: usize) -> Option<f32> {
    d.get(at..at + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    s
}

/// Écart d'angle ramené entre -180 et 180 degrés.
fn angle_diff(a: f64, b: f64) -> f64 {
    (a - b + 540.0).rem_euclid(360.0) - 180.0
}

/// Direction (dx, dz) d'un regard (degrés, 0 = vers +z, comme le jeu).
fn dir(yaw: f64) -> (f64, f64) {
    let r = yaw.to_radians();
    (-r.sin(), r.cos())
}

fn yaw_to(dx: f64, dz: f64) -> f64 {
    (-dx).atan2(dz).to_degrees()
}

// ---------------------------------------------------------------------------------------
// Le terrain, tel que les bots l'ont reçu (partagé par tous)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cell {
    /// Traversable : air, herbes, fleurs, torches…
    Air,
    /// Bloc plein
    Solid,
    /// Barrière, muret : trop haut pour être sauté
    Tall,
    Water,
    /// Lave, feu, cactus, magma… : à éviter absolument
    Danger,
    /// Chunk pas encore reçu
    Unknown,
}

enum Section {
    Uniform(u32),
    Mixed { palette: Vec<u32>, index: Box<[u16; 4096]> },
}

impl Section {
    fn get(&self, x: usize, y: usize, z: usize) -> u32 {
        match self {
            Section::Uniform(b) => *b,
            Section::Mixed { palette, index } => palette.get(index[(x << 8) | (z << 4) | y] as usize).copied().unwrap_or(0),
        }
    }
}

struct Terrain {
    data: GameData,
    air: u32,
    chunks: HashMap<(i32, i32), Vec<Section>>,
    kinds: HashMap<u32, Cell>,
}

fn classify(name: &str) -> Cell {
    let n = name.trim_start_matches("minecraft:");
    if matches!(n, "air" | "cave_air" | "void_air" | "structure_void" | "light_block") {
        return Cell::Air;
    }
    if n == "water" || n == "flowing_water" || n == "bubble_column" {
        return Cell::Water;
    }
    if n.contains("lava") || n.contains("fire") || n == "cactus" || n == "magma" || n == "sweet_berry_bush" || n == "powder_snow" || n == "web" || n.contains("campfire") || n == "wither_rose" {
        return Cell::Danger;
    }
    const PASS: [&str; 30] = [
        "flower", "sapling", "short_grass", "tall_grass", "fern", "deadbush", "dead_bush", "torch", "rail", "mushroom", "petals", "leaf_litter", "dry_grass", "vine", "snow_layer",
        "carpet", "seagrass", "kelp", "button", "pressure_plate", "standing_sign", "wall_sign", "redstone_wire", "tripwire", "ladder", "tulip", "sugar_cane", "lily_pad", "pickle", "bush",
    ];
    if PASS.iter().any(|p| n.contains(p))
        || matches!(n, "dandelion" | "poppy" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wheat" | "carrots" | "potatoes" | "beetroot" | "wildflowers" | "lever" | "glow_lichen" | "hanging_roots" | "spore_blossom" | "cave_vines" | "cave_vines_body_with_berries" | "cave_vines_head_with_berries" | "pink_petals")
    {
        // Les champignons géants sont des blocs pleins
        if n.contains("mushroom_block") || n.contains("mushroom_stem") {
            return Cell::Solid;
        }
        return Cell::Air;
    }
    if (n.contains("fence") && !n.contains("gate")) || n.ends_with("_wall") || n == "cobblestone_wall" || n.contains("border_block") {
        return Cell::Tall;
    }
    Cell::Solid
}

impl Terrain {
    fn kind(&mut self, state: u32) -> Cell {
        if let Some(k) = self.kinds.get(&state) {
            return *k;
        }
        let k = match self.data.block_name(state) {
            Some(n) => classify(n),
            None => Cell::Solid,
        };
        self.kinds.insert(state, k);
        k
    }

    fn cell(&mut self, x: i32, y: i32, z: i32) -> Cell {
        if y < MIN_Y {
            return Cell::Solid;
        }
        let s = ((y - MIN_Y) >> 4) as usize;
        let state = match self.chunks.get(&(x >> 4, z >> 4)) {
            None => return Cell::Unknown,
            Some(secs) => match secs.get(s) {
                None => return Cell::Air,
                Some(sec) => sec.get((x & 15) as usize, ((y - MIN_Y) & 15) as usize, (z & 15) as usize),
            },
        };
        self.kind(state)
    }

    fn known(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x >> 4, z >> 4))
    }

    fn set(&mut self, x: i32, y: i32, z: i32, block: u32) {
        if y < MIN_Y {
            return;
        }
        let s = ((y - MIN_Y) >> 4) as usize;
        let air = self.air;
        let secs = match self.chunks.get_mut(&(x >> 4, z >> 4)) {
            Some(c) => c,
            None => return,
        };
        while secs.len() <= s && secs.len() < SECTIONS {
            secs.push(Section::Uniform(air));
        }
        let sec = match secs.get_mut(s) {
            Some(sec) => sec,
            None => return,
        };
        if let Section::Uniform(b) = *sec {
            if b == block {
                return;
            }
            *sec = Section::Mixed { palette: vec![b], index: Box::new([0; 4096]) };
        }
        if let Section::Mixed { palette, index } = sec {
            let i = match palette.iter().position(|p| *p == block) {
                Some(i) => i,
                None => {
                    palette.push(block);
                    palette.len() - 1
                }
            };
            index[(((x & 15) as usize) << 8) | (((z & 15) as usize) << 4) | ((y - MIN_Y) & 15) as usize] = i as u16;
        }
    }

    /// LevelChunk (58) : position, puis le contenu des sous-chunks.
    fn load_chunk(&mut self, body: &[u8]) -> bool {
        let mut pos = 0;
        let (cx, cz) = match (read_vari32(body, &mut pos), read_vari32(body, &mut pos)) {
            (Some(x), Some(z)) => (x, z),
            _ => return false,
        };
        let _dim = read_vari32(body, &mut pos);
        let _count = read_varu32(body, &mut pos);
        // Plusieurs écritures existent selon les versions : on cherche la longueur qui tombe
        // juste sur la fin du paquet
        let mut payload = None;
        for skip in 0..=3 {
            let mut q = pos + skip;
            if let Some(len) = read_varu32(body, &mut q) {
                let end = q + len as usize;
                if (end == body.len() || end + 1 == body.len()) && matches!(body.get(q), Some(8) | Some(9)) {
                    payload = Some(&body[q..end]);
                    break;
                }
            }
        }
        let d = match payload {
            Some(d) => d,
            None => return false,
        };
        let mut secs = Vec::new();
        let mut p = 0;
        for i in 0..SECTIONS {
            let version = match d.get(p) {
                Some(v @ (8 | 9)) => *v,
                _ => break,
            };
            let storages = match d.get(p + 1) {
                Some(s @ 1..=2) => *s,
                _ => break,
            };
            if version == 9 && d.get(p + 2).map(|y| *y as i8 as i32) != Some((MIN_Y >> 4) + i as i32) {
                break;
            }
            p += if version == 9 { 3 } else { 2 };
            let mut first = None;
            for layer in 0..storages {
                let (sec, next) = match read_storage(d, p) {
                    Some(r) => r,
                    None => return false,
                };
                p = next;
                if layer == 0 {
                    first = Some(sec);
                }
            }
            secs.push(first.unwrap_or(Section::Uniform(self.air)));
        }
        self.chunks.insert((cx, cz), secs);
        true
    }
}

fn read_storage(d: &[u8], mut p: usize) -> Option<(Section, usize)> {
    let header = *d.get(p)?;
    p += 1;
    let bits = (header >> 1) as usize;
    if bits == 0 {
        let b = read_vari32(d, &mut p)? as u32;
        return Some((Section::Uniform(b), p));
    }
    if bits > 16 {
        return None;
    }
    let per_word = 32 / bits;
    let words = 4096usize.div_ceil(per_word);
    let raw = d.get(p..p + words * 4)?;
    p += words * 4;
    let len = read_vari32(d, &mut p)?.max(0) as usize;
    let mut palette = Vec::with_capacity(len.min(4096));
    for _ in 0..len.min(4096) {
        palette.push(read_vari32(d, &mut p)? as u32);
    }
    if palette.len() == 1 {
        return Some((Section::Uniform(palette[0]), p));
    }
    let mut index = Box::new([0u16; 4096]);
    let mask = (1u32 << bits) - 1;
    for (i, slot) in index.iter_mut().enumerate() {
        let w = i / per_word;
        let word = u32::from_le_bytes([raw[w * 4], raw[w * 4 + 1], raw[w * 4 + 2], raw[w * 4 + 3]]);
        *slot = ((word >> ((i % per_word) * bits)) & mask) as u16;
    }
    Some((Section::Mixed { palette, index }, p))
}

// ---------------------------------------------------------------------------------------
// Collisions et déplacement (comme le jeu, simplifié)
// ---------------------------------------------------------------------------------------

/// La boîte du joueur (pieds en x, y, z) touche-t-elle un bloc ?
fn collides(t: &mut Terrain, x: f64, y: f64, z: f64) -> bool {
    let (x0, x1) = ((x - HALF_WIDTH).floor() as i32, (x + HALF_WIDTH - 1e-6).floor() as i32);
    let (z0, z1) = ((z - HALF_WIDTH).floor() as i32, (z + HALF_WIDTH - 1e-6).floor() as i32);
    let (y0, y1) = (y.floor() as i32 - 1, (y + HEIGHT - 1e-6).floor() as i32);
    for bx in x0..=x1 {
        for bz in z0..=z1 {
            for by in y0..=y1 {
                let top = match t.cell(bx, by, bz) {
                    Cell::Solid | Cell::Unknown => by as f64 + 1.0,
                    Cell::Tall => by as f64 + 1.5,
                    _ => continue,
                };
                if y < top - 1e-6 && y + HEIGHT > by as f64 + 1e-6 {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------------------
// Un joueur simulé
// ---------------------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy, Debug)]
enum Stage {
    Waiting,
    Open1,
    Open2,
    Connecting,
    Joining,
    InGame,
    Lost,
}

struct Bot {
    name: String,
    sock: UdpSocket,
    /// Adresse du serveur (les envois la visent ; voir la réception dans `main`)
    server: SocketAddr,
    stage: Stage,
    seq: u32,
    ridx: u32,
    oidx: u32,
    compressed: bool,
    acks: Vec<[u8; 3]>,
    parts: HashMap<u16, Vec<Option<Vec<u8>>>>,
    last_step: Instant,
    last_heard: Instant,
    runtime_id: u64,
    // Corps
    pos: (f64, f64, f64),
    vel: (f64, f64, f64),
    on_ground: bool,
    yaw: f64,
    pitch: f64,
    head_yaw: f64,
    placed: bool,
    dead: bool,
    // Esprit
    rng: Rng,
    home: (f64, f64),
    target: Option<(f64, f64)>,
    idle: u32,
    look_at: (f64, f64),
    sprinting: bool,
    sneaking: u32,
    steer: f64,
    progress: ((f64, f64), u32),
    jump: bool,
    moving: bool,
    /// Mesures (BOTS_DEBUG) : ticks coincé dans un bloc, demi-tours, distance parcourue
    stats: (u64, u64, f64),
}

impl Bot {
    fn frame(&mut self, body: &[u8]) {
        let mut d = vec![0x84];
        d.extend_from_slice(&self.seq.to_le_bytes()[..3]);
        self.seq += 1;
        d.push(0x60);
        d.extend_from_slice(&((body.len() * 8) as u16).to_be_bytes());
        d.extend_from_slice(&self.ridx.to_le_bytes()[..3]);
        self.ridx += 1;
        d.extend_from_slice(&self.oidx.to_le_bytes()[..3]);
        self.oidx += 1;
        d.push(0);
        d.extend_from_slice(body);
        let _ = self.sock.send_to(&d, self.server);
    }

    fn game(&mut self, id: u32, body: &[u8]) {
        let mut inner = Vec::new();
        write_varu32(&mut inner, id);
        inner.extend_from_slice(body);
        let mut p = vec![0xfe];
        if self.compressed {
            p.push(0xff);
        }
        write_varu32(&mut p, inner.len() as u32);
        p.extend_from_slice(&inner);
        self.frame(&p);
    }

    fn flush_acks(&mut self) {
        while !self.acks.is_empty() {
            let n = self.acks.len().min(200);
            let mut a = vec![0xc0];
            a.extend_from_slice(&(n as u16).to_be_bytes());
            for s in self.acks.drain(..n) {
                a.push(1);
                a.extend_from_slice(&s);
            }
            let _ = self.sock.send_to(&a, self.server);
        }
    }

    fn open1(&mut self) {
        let mut p = vec![0x05];
        p.extend_from_slice(&MAGIC);
        p.push(11);
        p.resize(1372, 0);
        let _ = self.sock.send_to(&p, self.server);
    }

    fn open2(&mut self) {
        let mut p = vec![0x07];
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&[4, 0x80, 0xff, 0xff, 0xfe, 0x4a, 0xbc]);
        p.extend_from_slice(&1400u16.to_be_bytes());
        p.extend_from_slice(&(self.rng.next() as i64).to_be_bytes());
        let _ = self.sock.send_to(&p, self.server);
    }

    fn login(&mut self) {
        let payload = format!(r#"{{"extraData":{{"displayName":"{}","XUID":""}}}}"#, self.name);
        let jwt = format!("e30.{}.sig", b64(payload.as_bytes()));
        let chain = format!(r#"{{"chain":["{}"]}}"#, jwt);
        let client = format!("e30.{}.sig", b64(br#"{"GameVersion":"1.26.65","DeviceOS":7}"#));
        let mut data = Vec::new();
        data.extend_from_slice(&(chain.len() as u32).to_le_bytes());
        data.extend_from_slice(chain.as_bytes());
        data.extend_from_slice(&(client.len() as u32).to_le_bytes());
        data.extend_from_slice(client.as_bytes());
        let mut body = PROTOCOL.to_be_bytes().to_vec();
        write_varu32(&mut body, data.len() as u32);
        body.extend_from_slice(&data);
        self.game(1, &body);
    }

    fn on_datagram(&mut self, d: &[u8], t: &mut Terrain, now: Instant) {
        self.last_heard = now;
        match d[0] {
            0x06 if self.stage == Stage::Open1 => {
                self.open2();
                self.stage = Stage::Open2;
                self.last_step = now;
            }
            0x08 if self.stage == Stage::Open2 => {
                let mut p = vec![0x09];
                p.extend_from_slice(&(self.rng.next() as i64).to_be_bytes());
                p.extend_from_slice(&1i64.to_be_bytes());
                p.push(0);
                self.frame(&p);
                self.stage = Stage::Connecting;
                self.last_step = now;
            }
            f if f & 0x80 != 0 && f & 0x40 == 0 && f & 0x20 == 0 && d.len() > 4 => {
                self.acks.push([d[1], d[2], d[3]]);
                let mut pos = 4;
                while pos + 3 <= d.len() {
                    let flags = d[pos];
                    let len = (u16::from_be_bytes([d[pos + 1], d[pos + 2]]) as usize).div_ceil(8);
                    let rel = flags >> 5;
                    pos += 3;
                    if matches!(rel, 2 | 3 | 4 | 6 | 7) {
                        pos += 3;
                    }
                    if matches!(rel, 1 | 4) {
                        pos += 3;
                    }
                    if matches!(rel, 1 | 3 | 4 | 7) {
                        pos += 4;
                    }
                    let split = if flags & 0x10 != 0 {
                        let h = match d.get(pos..pos + 10) {
                            Some(h) => h,
                            None => return,
                        };
                        pos += 10;
                        Some((u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize, u16::from_be_bytes([h[4], h[5]]), u32::from_be_bytes([h[6], h[7], h[8], h[9]]) as usize))
                    } else {
                        None
                    };
                    let body = match d.get(pos..pos + len) {
                        Some(b) => b.to_vec(),
                        None => return,
                    };
                    pos += len;
                    match split {
                        Some((count, id, index)) if count > 0 && count < 4096 && index < count => {
                            let slot = self.parts.entry(id).or_insert_with(|| vec![None; count]);
                            if slot.len() == count {
                                slot[index] = Some(body);
                                if slot.iter().all(|p| p.is_some()) {
                                    let whole: Vec<u8> = self.parts.remove(&id).unwrap_or_default().into_iter().flatten().flatten().collect();
                                    self.on_packet(&whole, t, now);
                                }
                            }
                        }
                        Some(_) => {}
                        None => self.on_packet(&body, t, now),
                    }
                }
            }
            _ => {}
        }
    }

    fn on_packet(&mut self, p: &[u8], t: &mut Terrain, now: Instant) {
        match p.first() {
            Some(0x10) if self.stage == Stage::Connecting => {
                self.frame(&[0x13, 0, 0, 0]);
                self.game(0xc1, &PROTOCOL.to_be_bytes());
                self.stage = Stage::Joining;
                self.last_step = now;
            }
            Some(0x15) => self.stage = Stage::Lost,
            Some(0xfe) => {
                let inflated;
                let mut p = p;
                let mut pos = 1;
                if self.compressed {
                    match p.get(1) {
                        Some(0x00) => {
                            inflated = match basalt::inflate::inflate(&p[2..], 16 << 20) {
                                Ok(v) => v,
                                Err(_) => return,
                            };
                            p = &inflated;
                            pos = 0;
                        }
                        _ => pos = 2,
                    }
                }
                while pos < p.len() {
                    let len = match read_varu32(p, &mut pos) {
                        Some(l) => l as usize,
                        None => return,
                    };
                    let end = (pos + len).min(p.len());
                    let mut q = pos;
                    let id = read_varu32(&p[..end], &mut q).unwrap_or(0) & 0x3ff;
                    let body = &p[q..end];
                    pos = end;
                    self.on_game(id, body, t);
                }
            }
            _ => {}
        }
    }

    fn on_game(&mut self, id: u32, body: &[u8], t: &mut Terrain) {
        match id {
            0x8f => {
                self.compressed = true;
                self.login();
            }
            6 => self.game(8, &[3, 0, 0]),
            7 => self.game(8, &[4, 0, 0]),
            // StartGame : identifiants, mode de jeu, puis position (hauteur des yeux)
            11 => {
                let mut pos = 0;
                let _ = read_varu64(body, &mut pos);
                if let Some(rid) = read_varu64(body, &mut pos) {
                    self.runtime_id = rid;
                }
                let _ = read_vari32(body, &mut pos);
                if let (Some(x), Some(y), Some(z)) = (f32_at(body, pos), f32_at(body, pos + 4), f32_at(body, pos + 8)) {
                    if x.is_finite() && y.is_finite() && z.is_finite() && x.abs() < 3.0e7 && z.abs() < 3.0e7 {
                        self.place(x as f64, y as f64 - EYE, z as f64);
                    }
                }
                self.game(69, &[8, 16]); // rayon de 4 chunks : de quoi voir où l'on va
            }
            2 if body == [0, 0, 0, 3] => {
                let mut b = Vec::new();
                write_varu64(&mut b, self.runtime_id);
                self.game(113, &b);
                self.stage = Stage::InGame;
            }
            58 => {
                t.load_chunk(body);
            }
            // UpdateBlock : un bloc change (couche 0 seulement)
            21 => {
                let mut pos = 0;
                if let (Some(x), Some(y), Some(z), Some(block)) = (read_vari32(body, &mut pos), read_vari32(body, &mut pos), read_vari32(body, &mut pos), read_varu32(body, &mut pos)) {
                    let _flags = read_varu32(body, &mut pos);
                    if read_varu32(body, &mut pos).unwrap_or(0) == 0 {
                        t.set(x, y, z, block);
                    }
                }
            }
            // MovePlayer : le serveur nous replace
            19 => {
                let mut pos = 0;
                if read_varu64(body, &mut pos) == Some(self.runtime_id) {
                    if let (Some(x), Some(y), Some(z)) = (f32_at(body, pos), f32_at(body, pos + 4), f32_at(body, pos + 8)) {
                        self.place(x as f64, y as f64 - EYE, z as f64);
                    }
                }
            }
            // Respawn : 0 = mort (écran de mort), 1 = de retour
            45 if body.len() > 12 => match body[12] {
                0 => {
                    self.dead = true;
                    let mut ready = body.to_vec();
                    ready[12] = 2;
                    self.game(45, &ready);
                }
                1 => {
                    self.dead = false;
                    if let (Some(x), Some(y), Some(z)) = (f32_at(body, 0), f32_at(body, 4), f32_at(body, 8)) {
                        self.place(x as f64, y as f64 - EYE, z as f64);
                    }
                }
                _ => {}
            },
            5 => self.stage = Stage::Lost,
            _ => {}
        }
    }

    fn place(&mut self, x: f64, y: f64, z: f64) {
        self.pos = (x, y, z);
        self.vel = (0.0, 0.0, 0.0);
        if !self.placed {
            self.home = (x, z);
            self.placed = true;
        }
    }

    // -----------------------------------------------------------------------------------
    // Décision : où aller, comment
    // -----------------------------------------------------------------------------------

    /// Peut-on avancer dans cette direction ? Renvoie un score (plus haut = mieux), ou None.
    fn probe(&self, t: &mut Terrain, heading: f64) -> Option<f64> {
        let (dx, dz) = dir(heading);
        // Le corps fait 0,6 bloc de large : on sonde l'axe et les deux bords
        let (sx, sz) = (dz, -dx);
        let mut fy = (self.pos.1 + 0.01).floor() as i32;
        let walkable = |c: Cell| matches!(c, Cell::Air | Cell::Water);
        // Place pour sauter au-dessus de sa tête, là où il se trouve
        let (cx, cz) = (self.pos.0.floor() as i32, self.pos.2.floor() as i32);
        let room_above = walkable(t.cell(cx, fy + 2, cz));
        let mut score = 0.0;
        for (i, d) in [0.7, 1.4, 2.2, 3.0].iter().enumerate() {
            let blocked = if i >= 2 { Some(score - 1.0) } else { None };
            let mut step = false;
            for lateral in [-0.32, 0.0, 0.32] {
                let (x, z) = ((self.pos.0 + dx * d + sx * lateral).floor() as i32, (self.pos.2 + dz * d + sz * lateral).floor() as i32);
                if !t.known(x, z) {
                    return None;
                }
                let (feet, head) = (t.cell(x, fy, z), t.cell(x, fy + 1, z));
                if feet == Cell::Danger || head == Cell::Danger {
                    return None;
                }
                if walkable(feet) && walkable(head) {
                    continue;
                }
                // Une marche : bloc plein aux pieds, de la place au-dessus (ici et là-bas)
                if feet == Cell::Solid && walkable(head) && walkable(t.cell(x, fy + 2, z)) && room_above {
                    step = true;
                    continue;
                }
                // Mur, tronc, feuillage
                return blocked;
            }
            if step {
                fy += 1;
                score += 0.6;
                continue;
            }
            // Le sol, sous l'axe : pas de chute de plus de 3 blocs, ni lave
            let (x, z) = ((self.pos.0 + dx * d).floor() as i32, (self.pos.2 + dz * d).floor() as i32);
            let mut ground = None;
            for k in 1..=4 {
                match t.cell(x, fy - k, z) {
                    Cell::Air => continue,
                    c => {
                        ground = Some((k, c));
                        break;
                    }
                }
            }
            match ground {
                None | Some((_, Cell::Danger)) => return blocked,
                Some((k, Cell::Water)) => {
                    score -= 3.0;
                    fy -= k - 1;
                }
                Some((k, _)) => {
                    fy -= k - 1;
                    if k > 1 {
                        score -= 0.3 * (k - 1) as f64;
                    }
                }
            }
            if t.cell(x, fy, z) == Cell::Water {
                score -= 2.0;
            }
            score += 1.0;
        }
        Some(score)
    }

    fn new_target(&mut self, around: f64) {
        let far = ((self.pos.0 - self.home.0).powi(2) + (self.pos.2 - self.home.1).powi(2)).sqrt();
        let base = if far > around { yaw_to(self.home.0 - self.pos.0, self.home.1 - self.pos.2) } else { self.yaw };
        let heading = base + self.rng.range(-100.0, 100.0);
        let dist = self.rng.range(8.0, 45.0);
        let (dx, dz) = dir(heading);
        self.target = Some((self.pos.0 + dx * dist, self.pos.2 + dz * dist));
        self.sprinting = self.rng.chance(0.3);
        self.steer = 0.0;
    }

    fn think(&mut self, t: &mut Terrain, radius: f64) {
        self.jump = false;
        self.moving = false;
        if self.sneaking > 0 {
            self.sneaking -= 1;
        }
        // Pause : regarde autour de lui
        if self.idle > 0 {
            self.idle -= 1;
            if self.rng.chance(0.04) {
                self.look_at = (self.yaw + self.rng.range(-120.0, 120.0), self.rng.range(-35.0, 25.0));
            }
            self.turn_toward(self.look_at.0, self.look_at.1, 8.0);
            if self.idle == 0 {
                self.new_target(radius);
            }
            return;
        }
        let (tx, tz) = match self.target {
            Some(tg) => tg,
            None => {
                self.new_target(radius);
                return;
            }
        };
        let (dx, dz) = (tx - self.pos.0, tz - self.pos.2);
        if dx * dx + dz * dz < 2.0 {
            self.target = None;
            if self.rng.chance(0.5) {
                self.idle = self.rng.range(10.0, 80.0) as u32;
                self.look_at = (self.yaw, 0.0);
                if self.rng.chance(0.15) {
                    self.sneaking = self.rng.range(10.0, 40.0) as u32;
                }
            }
            return;
        }
        // Coincé (rien n'avance depuis 2 secondes) : autre direction
        self.progress.1 += 1;
        if self.progress.1 >= 40 {
            let (px, pz) = self.progress.0;
            if (self.pos.0 - px).powi(2) + (self.pos.2 - pz).powi(2) < 1.0 {
                self.stats.1 += 1;
                if std::env::var("BOTS_DEBUG").is_ok() && self.stats.1 <= 3 {
                    let (fx, fz) = dir(self.yaw);
                    let fy = (self.pos.1 + 0.01).floor() as i32;
                    let (ax, az) = ((self.pos.0 + fx * 0.75).floor() as i32, (self.pos.2 + fz * 0.75).floor() as i32);
                    let (cx, cz) = (self.pos.0.floor() as i32, self.pos.2.floor() as i32);
                    let name = |t: &mut Terrain, x: i32, y: i32, z: i32| {
                        let s = t.chunks.get(&(x >> 4, z >> 4)).and_then(|c| c.get(((y - MIN_Y) >> 4) as usize)).map(|s| s.get((x & 15) as usize, ((y - MIN_Y) & 15) as usize, (z & 15) as usize));
                        s.and_then(|s| t.data.block_name(s).map(|n| n.trim_start_matches("minecraft:").to_string())).unwrap_or("?".into())
                    };
                    println!(
                        "COINCÉ {} en {:.2} {:.2} {:.2} yaw {:.0} sol {} steer {:.0} | devant : {} / {} / {} | ici +2 : {} | sous : {}",
                        self.name, self.pos.0, self.pos.1, self.pos.2, self.yaw, self.on_ground, self.steer,
                        name(t, ax, fy, az), name(t, ax, fy + 1, az), name(t, ax, fy + 2, az), name(t, cx, fy + 2, cz), name(t, cx, fy - 1, cz)
                    );
                }
                let back = self.yaw + 180.0 + self.rng.range(-60.0, 60.0);
                let (bx, bz) = dir(back);
                let d = self.rng.range(6.0, 20.0);
                self.target = Some((self.pos.0 + bx * d, self.pos.2 + bz * d));
                self.steer = 0.0;
            }
            self.progress = ((self.pos.0, self.pos.2), 0);
        }
        // Direction : celle de la cible, ou la plus proche qui passe (contourne arbres et murs)
        let wanted = yaw_to(dx, dz);
        let mut best: Option<(f64, f64)> = None;
        let mut candidates = vec![self.steer];
        for k in 0..=9 {
            let a = k as f64 * 18.0;
            candidates.push(a);
            if k > 0 {
                candidates.push(-a);
            }
        }
        for off in candidates {
            if let Some(score) = self.probe(t, wanted + off) {
                // Moins on dévie, mieux c'est ; garder le même côté évite d'hésiter
                let value = score - off.abs() / 30.0 + if (off - self.steer).abs() < 1.0 { 0.8 } else { 0.0 };
                if best.map(|(_, v)| value > v).unwrap_or(true) {
                    best = Some((off, value));
                }
            }
        }
        let heading = match best {
            Some((off, _)) => {
                self.steer = off;
                wanted + off
            }
            None => {
                // Impasse : demi-tour vers une autre cible
                self.target = None;
                self.idle = self.rng.range(5.0, 25.0) as u32;
                self.look_at = (self.yaw + 180.0, 0.0);
                return;
            }
        };
        // La tête suit la direction avec un peu de mollesse, comme à la souris
        let wobble = self.rng.range(-4.0, 4.0);
        if self.rng.chance(0.03) {
            self.look_at.1 = self.rng.range(-10.0, 15.0);
        }
        let pitch = self.look_at.1;
        self.turn_toward(heading + wobble, pitch, 14.0);
        // On avance dans la direction regardée : le virage se fait en arc
        if angle_diff(heading, self.yaw).abs() < 100.0 {
            self.moving = true;
        }
        // Saut : marche devant soi, ou nage
        let (fx, fz) = dir(self.yaw);
        let ahead = ((self.pos.0 + fx * 0.75).floor() as i32, (self.pos.2 + fz * 0.75).floor() as i32);
        let fy = (self.pos.1 + 0.01).floor() as i32;
        let (here_x, here_z) = (self.pos.0.floor() as i32, self.pos.2.floor() as i32);
        if self.on_ground
            && t.cell(ahead.0, fy, ahead.1) == Cell::Solid
            && t.cell(ahead.0, fy + 1, ahead.1) == Cell::Air
            && t.cell(ahead.0, fy + 2, ahead.1) == Cell::Air
            && matches!(t.cell(here_x, fy + 2, here_z), Cell::Air | Cell::Water)
        {
            self.jump = true;
        }
        let x = self.pos.0.floor() as i32;
        let z = self.pos.2.floor() as i32;
        if t.cell(x, fy, z) == Cell::Water || t.cell(x, fy + 1, z) == Cell::Water {
            self.jump = true;
        }
        // Petit saut de temps en temps en courant, comme beaucoup de joueurs
        if self.sprinting && self.on_ground && self.rng.chance(0.015) {
            self.jump = true;
        }
    }

    fn turn_toward(&mut self, yaw: f64, pitch: f64, speed: f64) {
        let d = angle_diff(yaw, self.yaw);
        self.yaw += d.clamp(-speed, speed);
        self.yaw = (self.yaw + 540.0).rem_euclid(360.0) - 180.0;
        self.pitch += (pitch - self.pitch).clamp(-4.0, 4.0);
        let h = angle_diff(self.yaw, self.head_yaw);
        self.head_yaw += h * 0.6;
    }

    // -----------------------------------------------------------------------------------
    // Physique
    // -----------------------------------------------------------------------------------

    fn physics(&mut self, t: &mut Terrain) {
        let (x, y, z) = self.pos;
        if !t.known(x.floor() as i32, z.floor() as i32) {
            return; // terrain pas encore reçu : il attend
        }
        let fy = y.floor() as i32;
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        let in_water = t.cell(bx, fy, bz) == Cell::Water || t.cell(bx, fy + 1, bz) == Cell::Water;
        let mut speed = if self.sneaking > 0 {
            WALK * 0.3
        } else if self.sprinting && !in_water {
            SPRINT
        } else {
            WALK
        };
        if in_water {
            speed *= 0.45;
        }
        let (dx, dz) = dir(self.yaw);
        let (hx, hz) = if self.moving { (dx * speed, dz * speed) } else { (0.0, 0.0) };
        let grip = if self.on_ground { 0.55 } else if in_water { 0.3 } else { 0.15 };
        self.vel.0 += (hx - self.vel.0) * grip;
        self.vel.2 += (hz - self.vel.2) * grip;
        if in_water {
            self.vel.1 = if self.jump { (self.vel.1 + 0.04).min(0.12) } else { (self.vel.1 - 0.02) * 0.8 };
        } else if self.jump && self.on_ground {
            self.vel.1 = 0.42;
        } else {
            self.vel.1 = (self.vel.1 - 0.08) * 0.98;
        }
        // Vertical
        let (mut x, mut y, mut z) = (x, y, z);
        let ny = y + self.vel.1;
        if collides(t, x, ny, z) {
            if self.vel.1 < 0.0 {
                // Atterrit : position exacte par dichotomie
                let (mut free, mut hit) = (y, ny);
                for _ in 0..12 {
                    let mid = (free + hit) / 2.0;
                    if collides(t, x, mid, z) {
                        hit = mid;
                    } else {
                        free = mid;
                    }
                }
                let snapped = (free * 2.0).round() / 2.0;
                y = if (snapped - free).abs() < 0.02 && !collides(t, x, snapped, z) { snapped } else { free };
            }
            self.vel.1 = 0.0;
        } else {
            y = ny;
        }
        // Horizontal, un axe après l'autre (on glisse le long des murs)
        let mut bumped = false;
        let nx = x + self.vel.0;
        if collides(t, nx, y, z) {
            self.vel.0 = 0.0;
            bumped = true;
        } else {
            x = nx;
        }
        let nz = z + self.vel.2;
        if collides(t, x, y, nz) {
            self.vel.2 = 0.0;
            bumped = true;
        } else {
            z = nz;
        }
        // Dans l'eau contre un bord : il se hisse dehors, comme le jeu
        if in_water && bumped && self.moving {
            self.vel.1 = 0.3;
        }
        self.on_ground = collides(t, x, y - 0.01, z);
        if self.on_ground && self.vel.1 < 0.0 {
            self.vel.1 = 0.0;
        }
        self.stats.2 += ((x - self.pos.0).powi(2) + (z - self.pos.2).powi(2)).sqrt();
        self.pos = (x, y.max(-128.0), z);
        if collides(t, x, self.pos.1, z) {
            self.stats.0 += 1;
        }
    }

    fn send_input(&mut self, tick: u64) {
        let mut b = Vec::with_capacity(64);
        for v in [self.pitch as f32, self.yaw as f32, self.pos.0 as f32, (self.pos.1 + EYE) as f32, self.pos.2 as f32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // Manette : (côté, avant)
        let forward = if self.moving { 1.0f32 } else { 0.0 };
        b.extend_from_slice(&0f32.to_le_bytes());
        b.extend_from_slice(&forward.to_le_bytes());
        b.extend_from_slice(&(self.head_yaw as f32).to_le_bytes());
        let mut flags = Vec::new();
        if self.sneaking > 0 {
            flags.push(8);
        }
        if self.sprinting && self.moving && self.sneaking == 0 {
            flags.push(20);
        }
        write_varu32(&mut b, flags.len() as u32);
        for f in flags {
            write_varu32(&mut b, (f << 1) as u32);
        }
        write_varu32(&mut b, 0); // mode d'entrée
        write_varu32(&mut b, 0); // mode de jeu
        write_varu32(&mut b, 0); // modèle d'interaction
        write_varu64(&mut b, tick);
        b.resize(b.len().max(64), 0);
        self.game(144, &b);
    }
}

// ---------------------------------------------------------------------------------------

const NAMES: [&str; 24] = [
    "Alex", "Steve", "Lea", "Hugo", "Emma", "Nolan", "Jade", "Louis", "Chloe", "Noah", "Lina", "Tom", "Zoe", "Enzo", "Ines", "Leo", "Manon", "Theo", "Lucie", "Adam", "Sarah", "Jules", "Maya", "Sacha",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = match args.get(1).and_then(|a| a.parse().ok()) {
        Some(n) if n > 0 => n,
        _ => {
            eprintln!("Usage : basalt-bots <nombre> [adresse[:port]] [rayon]");
            eprintln!("Côté serveur : local-bots=true dans server.properties.");
            std::process::exit(2);
        }
    };
    let target = args.get(2).cloned().unwrap_or_else(|| "127.0.0.1:19132".into());
    let target = if target.contains(':') { target } else { format!("{}:19132", target) };
    let server: SocketAddr = match target.to_socket_addrs().ok().and_then(|mut a| a.next()) {
        Some(a) => a,
        None => {
            eprintln!("Adresse invalide : {}", target);
            std::process::exit(2);
        }
    };
    let radius: f64 = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(150.0);
    let data_dir = std::env::var("BASALT_DATA").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        let here = std::path::PathBuf::from("data");
        if here.is_dir() {
            return here;
        }
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("data"))).unwrap_or(here)
    });
    let data = match GameData::load(&data_dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Données du jeu introuvables dans {} : {} (BASALT_DATA=chemin)", data_dir.display(), e);
            std::process::exit(1);
        }
    };
    let air = (0..200_000u32).find(|s| data.block_name(*s) == Some("minecraft:air")).unwrap_or(0);
    let mut terrain = Terrain { data, air, chunks: HashMap::new(), kinds: HashMap::new() };

    let loopback = server.ip().is_loopback();
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
    let mut bots: Vec<Bot> = Vec::new();
    let now = Instant::now();
    for i in 0..count {
        // Une adresse locale par bot (127.0.x.y) : la limite de connexions par adresse ne joue pas
        let local = if loopback { format!("127.0.{}.{}:0", 1 + i / 250, 2 + i % 250) } else { "0.0.0.0:0".into() };
        let sock = match UdpSocket::bind(&local) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Impossible d'ouvrir {} : {}", local, e);
                std::process::exit(1);
            }
        };
        // Pas de « connect » : sous Windows, le serveur peut répondre depuis une autre adresse
        // locale que celle visée (127.0.x.y au lieu de 127.0.0.1), et une prise connectée
        // jetterait ses réponses. L'origine est vérifiée à la réception.
        sock.set_nonblocking(true).expect("mode non bloquant");
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let name = format!("Bot{}{}", NAMES[i % NAMES.len()], i / NAMES.len() + 1);
        bots.push(Bot {
            name,
            sock,
            server,
            stage: Stage::Waiting,
            seq: 0,
            ridx: 0,
            oidx: 0,
            compressed: false,
            acks: Vec::new(),
            parts: HashMap::new(),
            last_step: now,
            last_heard: now,
            runtime_id: 0,
            pos: (0.5, 100.0, 0.5),
            vel: (0.0, 0.0, 0.0),
            on_ground: false,
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            placed: false,
            dead: false,
            rng: Rng(seed),
            home: (0.0, 0.0),
            target: None,
            idle: 20,
            look_at: (0.0, 0.0),
            sprinting: false,
            sneaking: 0,
            steer: 0.0,
            progress: ((0.0, 0.0), 0),
            jump: false,
            moving: false,
            stats: (0, 0, 0.0),
        });
    }
    println!("{} bots vers {} (rayon {} blocs). Ctrl+C pour arrêter.", count, server, radius);

    let start = Instant::now();
    let mut next_tick = Instant::now();
    let mut tick: u64 = 0;
    let mut last_report = Instant::now();
    let mut buf = vec![0u8; 2048];
    let mut lost_names: VecDeque<String> = VecDeque::new();
    loop {
        let now = Instant::now();
        // Arrivées : 4 par seconde
        let allowed = (start.elapsed().as_secs_f64() * 4.0) as usize + 1;
        for (i, b) in bots.iter_mut().enumerate() {
            if b.stage == Stage::Waiting && i < allowed {
                b.open1();
                b.stage = Stage::Open1;
                b.last_step = now;
                b.last_heard = now;
            }
        }
        // Réception
        for b in bots.iter_mut() {
            if matches!(b.stage, Stage::Waiting | Stage::Lost) {
                continue;
            }
            // Jusqu'à épuisement des datagrammes en attente. Sous Windows, une erreur de
            // « connexion réinitialisée » signale seulement qu'un envoi précédent n'a pas été
            // reçu : on continue de lire (bornée, pour ne jamais tourner en rond).
            let mut errors = 0;
            loop {
                let (n, from) = match b.sock.recv_from(&mut buf) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        errors += 1;
                        if errors > 8 {
                            break;
                        }
                        continue;
                    }
                };
                // Seul le serveur est écouté : même port, même adresse (ou une autre adresse
                // de cette machine quand le serveur est local)
                let ours = from.port() == server.port() && (from.ip() == server.ip() || (loopback && from.ip().is_loopback()));
                if n > 0 && ours {
                    let d = buf[..n].to_vec();
                    b.on_datagram(&d, &mut terrain, now);
                }
            }
            // Étapes sans réponse : on relance
            if matches!(b.stage, Stage::Open1 | Stage::Open2) && now.duration_since(b.last_step) > Duration::from_secs(1) {
                b.stage = Stage::Open1;
                b.open1();
                b.last_step = now;
            }
            if now.duration_since(b.last_heard) > Duration::from_secs(15) {
                b.stage = Stage::Lost;
                lost_names.push_back(b.name.clone());
            }
        }
        // Tick de jeu : 20 par seconde
        if now >= next_tick {
            next_tick += Duration::from_millis(50);
            if now > next_tick + Duration::from_millis(500) {
                next_tick = now;
            }
            tick += 1;
            for b in bots.iter_mut() {
                if b.stage == Stage::InGame && b.placed && !b.dead {
                    b.think(&mut terrain, radius);
                    b.physics(&mut terrain);
                    b.send_input(tick);
                }
                b.flush_acks();
            }
        }
        // Le terrain loin de tous les bots est oublié
        if tick % 200 == 0 && terrain.chunks.len() > 2000 {
            let centers: Vec<(i32, i32)> = bots.iter().filter(|b| b.stage == Stage::InGame).map(|b| ((b.pos.0 as i32) >> 4, (b.pos.2 as i32) >> 4)).collect();
            terrain.chunks.retain(|(cx, cz), _| centers.iter().any(|(x, z)| (cx - x).abs() <= 8 && (cz - z).abs() <= 8));
        }
        if last_report.elapsed() >= Duration::from_secs(10) {
            last_report = Instant::now();
            let in_game = bots.iter().filter(|b| b.stage == Stage::InGame).count();
            let walking = bots.iter().filter(|b| b.stage == Stage::InGame && b.moving).count();
            let lost = bots.iter().filter(|b| b.stage == Stage::Lost).count();
            println!("{} en jeu ({} en marche), {} déconnectés, {} chunks connus", in_game, walking, lost, terrain.chunks.len());
            if std::env::var("BOTS_DEBUG").is_ok() {
                let (inside, stuck, dist) = bots.iter().fold((0, 0, 0.0), |a, b| (a.0 + b.stats.0, a.1 + b.stats.1, a.2 + b.stats.2));
                println!("  dans un bloc : {} ticks, coincés : {} fois, parcouru : {:.0} blocs", inside, stuck, dist);
                for b in bots.iter().filter(|b| b.stage == Stage::InGame).take(5) {
                    println!("  {} en {:.1} {:.1} {:.1} (sol {}, cible {:?}, pause {})", b.name, b.pos.0, b.pos.1, b.pos.2, b.on_ground, b.target.map(|(x, z)| (x as i32, z as i32)), b.idle);
                }
            }
            while let Some(n) = lost_names.pop_front() {
                println!("  {} : plus de réponse du serveur", n);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
