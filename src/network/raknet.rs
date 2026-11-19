//! Couche RakNet : découverte du serveur, poignée de main, fiabilité,
//! ordonnancement et fragmentation au-dessus d'UDP.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const MAGIC: [u8; 16] = [
    0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78,
];
pub const RAKNET_PROTOCOL: u8 = 11;

const MIN_MTU: usize = 576;
const MAX_MTU: usize = 1492;
const RESEND_AFTER: Duration = Duration::from_millis(1000);
/// Taille demandée pour les tampons d'émission et de réception du socket.
const SOCKET_BUFFER: i32 = 8 * 1024 * 1024;
/// Datagrammes lus au plus par passage de la boucle principale.
const MAX_READS_PER_POLL: usize = 8192;
/// Durée maximale d'un passage de lecture.
const MAX_POLL_TIME: Duration = Duration::from_millis(8);

/// Compteurs de trafic depuis le démarrage.
#[derive(Default, Clone, Copy)]
pub struct Traffic {
    pub datagrams_in: u64,
    pub datagrams_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

// --- Limites anti-abus -------------------------------------------------------
/// Fragments maximum pour un paquet découpé.
const MAX_SPLIT_PARTS: u32 = 2400;
/// Taille maximale d'un paquet avant le login (le login contient le skin).
pub const MAX_PACKET_LOGIN: usize = 3 * 1024 * 1024;
/// Taille maximale d'un paquet une fois le joueur identifié.
pub const MAX_PACKET_GAME: usize = 512 * 1024;
/// Datagrammes acceptés par seconde et par session ; au-delà, déconnexion.
const MAX_DATAGRAMS_PER_SEC: u32 = 1500;
/// Accusés de réception acceptés par seconde et par session. Comptés à part : pendant
/// l'envoi des grosses listes (recettes, objets), le serveur émet plus d'un millier de
/// datagrammes en une seconde, et certains clients accusent réception de chacun séparément.
const MAX_ACKS_PER_SEC: u32 = 8000;
/// Paquets hors connexion (ping, ouverture) traités par seconde et par adresse IP.
const MAX_OFFLINE_PER_IP: u16 = 30;
/// Idem, toutes adresses confondues.
const MAX_OFFLINE_GLOBAL: u32 = 3000;
/// Nouvelles sessions créées par seconde, toutes adresses confondues.
const MAX_NEW_SESSIONS_PER_SEC: u32 = 40;
/// Mémoire maximale des paquets reçus en avance (en attente d'ordre).
const MAX_ORDER_BYTES: usize = 2 * 1024 * 1024;
const MAX_ORDER_PACKETS: usize = 256;
/// Sans accusé de réception pendant ce délai, le client est considéré comme factice.
const ACK_DEADLINE: Duration = Duration::from_secs(12);
/// Un NACK n'est honoré que pour un envoi assez ancien (évite de servir d'amplificateur).
const NACK_MIN_AGE: Duration = Duration::from_millis(200);

/// Lecteur d'octets borné : toute lecture hors limites renvoie `None`.
pub struct Reader<'a> {
    pub d: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Reader { d, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.d.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    pub fn u24le(&mut self) -> Option<u32> {
        self.take(3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn i64(&mut self) -> Option<i64> {
        self.take(8).map(|b| {
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            i64::from_be_bytes(a)
        })
    }
    pub fn left(&self) -> usize {
        self.d.len() - self.pos
    }
}

fn put_u24le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes()[..3]);
}

pub fn put_addr(buf: &mut Vec<u8>, a: &SocketAddr) {
    match a.ip() {
        IpAddr::V4(ip) => {
            buf.push(4);
            for b in ip.octets().iter() {
                buf.push(!b);
            }
            buf.extend_from_slice(&a.port().to_be_bytes());
        }
        IpAddr::V6(ip) => {
            buf.push(6);
            buf.extend_from_slice(&23u16.to_le_bytes());
            buf.extend_from_slice(&a.port().to_be_bytes());
            buf.extend_from_slice(&[0; 4]);
            buf.extend_from_slice(&ip.octets());
            buf.extend_from_slice(&[0; 4]);
        }
    }
}

fn skip_addr(r: &mut Reader) -> Option<()> {
    match r.u8()? {
        4 => r.take(6).map(|_| ()),
        6 => r.take(28).map(|_| ()),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Connecting,
    Connected,
    Closed,
}

struct Split {
    parts: Vec<Option<Vec<u8>>>,
    got: usize,
    size: usize,
}

pub enum RakEvent {
    Connected(SocketAddr),
    Packet(SocketAddr, Vec<u8>),
    /// Session terminée ; le texte précise la raison si le serveur l'a fermée pour abus.
    Disconnected(SocketAddr, Option<&'static str>),
}

pub struct Session {
    pub addr: SocketAddr,
    pub guid: i64,
    pub mtu: usize,
    pub state: State,
    last_recv: Instant,
    close_at: Option<Instant>,
    announced: bool,
    /// Raison d'une fermeture pour abus
    fault: Option<&'static str>,
    max_packet: usize,
    max_splits: usize,
    rate_window: Instant,
    rate_count: u32,
    rate_acks: u32,
    waiting_since: Option<Instant>,
    /// Datagrammes et octets émis depuis le dernier relevé
    sent: (u64, u64),
    // Réception
    ack_queue: Vec<u32>,
    seen: HashSet<u32>,
    seen_low: u32,
    order_next: u32,
    order_buf: BTreeMap<u32, Vec<u8>>,
    order_bytes: usize,
    splits: HashMap<u16, Split>,
    inbox: Vec<Vec<u8>>,
    // Émission
    seq: u32,
    reliable_idx: u32,
    order_idx: u32,
    split_id: u16,
    pending: VecDeque<Vec<u8>>,
    unacked: BTreeMap<u32, (Instant, Vec<Vec<u8>>)>,
}

impl Session {
    fn new(addr: SocketAddr, guid: i64, mtu: usize, now: Instant) -> Session {
        Session {
            addr,
            guid,
            mtu,
            state: State::Connecting,
            last_recv: now,
            close_at: None,
            announced: false,
            fault: None,
            max_packet: MAX_PACKET_LOGIN,
            max_splits: 2,
            rate_window: now,
            rate_count: 0,
            rate_acks: 0,
            waiting_since: None,
            sent: (0, 0),
            ack_queue: Vec::new(),
            seen: HashSet::new(),
            seen_low: 0,
            order_next: 0,
            order_buf: BTreeMap::new(),
            order_bytes: 0,
            splits: HashMap::new(),
            inbox: Vec::new(),
            seq: 0,
            reliable_idx: 0,
            order_idx: 0,
            split_id: 0,
            pending: VecDeque::new(),
            unacked: BTreeMap::new(),
        }
    }

    fn frame(&mut self, body: &[u8], order: u32, split: Option<(u32, u16, u32)>) -> Vec<u8> {
        let mut f = Vec::with_capacity(body.len() + 20);
        f.push((3 << 5) | if split.is_some() { 0x10 } else { 0 });
        f.extend_from_slice(&((body.len() * 8) as u16).to_be_bytes());
        put_u24le(&mut f, self.reliable_idx);
        self.reliable_idx = (self.reliable_idx + 1) & 0xff_ffff;
        put_u24le(&mut f, order);
        f.push(0);
        if let Some((count, id, index)) = split {
            f.extend_from_slice(&count.to_be_bytes());
            f.extend_from_slice(&id.to_be_bytes());
            f.extend_from_slice(&index.to_be_bytes());
        }
        f.extend_from_slice(body);
        f
    }

    /// Met en file un paquet fiable et ordonné (fragmenté si nécessaire).
    pub fn send(&mut self, payload: &[u8]) {
        if self.state == State::Closed || payload.is_empty() {
            return;
        }
        let max = self.mtu - 60;
        let order = self.order_idx;
        self.order_idx = (self.order_idx + 1) & 0xff_ffff;
        if payload.len() <= max {
            let f = self.frame(payload, order, None);
            self.pending.push_back(f);
        } else {
            let count = ((payload.len() + max - 1) / max) as u32;
            let id = self.split_id;
            self.split_id = self.split_id.wrapping_add(1);
            for (i, chunk) in payload.chunks(max).enumerate() {
                let f = self.frame(chunk, order, Some((count, id, i as u32)));
                self.pending.push_back(f);
            }
        }
    }

    fn on_datagram(&mut self, d: &[u8], now: Instant, start: Instant) {
        self.last_recv = now;
        if now.duration_since(self.rate_window) >= Duration::from_secs(1) {
            self.rate_window = now;
            self.rate_count = 0;
            self.rate_acks = 0;
        }
        let flags = d[0];
        // Un accusé de réception (ou une demande de renvoi) coûte très peu à traiter
        let is_ack = flags & 0x60 != 0;
        if is_ack {
            self.rate_acks += 1;
        } else {
            self.rate_count += 1;
        }
        if self.rate_count > MAX_DATAGRAMS_PER_SEC || self.rate_acks > MAX_ACKS_PER_SEC {
            self.abort("trop de datagrammes par seconde");
            return;
        }
        let mut r = Reader::new(&d[1..]);
        if flags & 0x40 != 0 {
            self.on_ack(&mut r, false, now);
        } else if flags & 0x20 != 0 {
            self.on_ack(&mut r, true, now);
        } else {
            let seq = match r.u24le() {
                Some(s) => s,
                None => return,
            };
            if self.ack_queue.len() < 4096 {
                self.ack_queue.push(seq);
            }
            while r.left() > 0 {
                if self.read_frame(&mut r, start).is_none() {
                    break;
                }
            }
        }
    }

    fn abort(&mut self, reason: &'static str) {
        if self.fault.is_none() {
            self.fault = Some(reason);
        }
        self.state = State::Closed;
        // Libère tout de suite la mémoire retenue par la session
        self.pending.clear();
        self.unacked.clear();
        self.splits.clear();
        self.order_buf.clear();
        self.inbox.clear();
    }

    fn on_ack(&mut self, r: &mut Reader, nack: bool, now: Instant) -> Option<()> {
        let count = r.u16()?;
        let mut budget = 8192u32;
        for _ in 0..count {
            let single = r.u8()? != 0;
            let a = r.u24le()?;
            let b = if single { a } else { r.u24le()? };
            if b < a {
                return None;
            }
            for seq in a..=b {
                if budget == 0 {
                    return None;
                }
                budget -= 1;
                if nack {
                    let old_enough = self
                        .unacked
                        .get(&seq)
                        .map(|(t, _)| now.duration_since(*t) >= NACK_MIN_AGE)
                        .unwrap_or(false);
                    if !old_enough {
                        continue;
                    }
                    if let Some((_, frames)) = self.unacked.remove(&seq) {
                        for f in frames.into_iter().rev() {
                            self.pending.push_front(f);
                        }
                    }
                } else if self.unacked.remove(&seq).is_some() {
                    self.waiting_since = Some(now);
                }
            }
        }
        Some(())
    }

    fn read_frame(&mut self, r: &mut Reader, start: Instant) -> Option<()> {
        let flags = r.u8()?;
        let rel = flags >> 5;
        let is_split = flags & 0x10 != 0;
        let len = (r.u16()? as usize + 7) / 8;
        if len == 0 {
            return None;
        }
        let ridx = if matches!(rel, 2 | 3 | 4 | 6 | 7) { Some(r.u24le()?) } else { None };
        if matches!(rel, 1 | 4) {
            r.u24le()?;
        }
        let ord = if matches!(rel, 1 | 3 | 4 | 7) {
            let i = r.u24le()?;
            r.u8()?;
            Some(i)
        } else {
            None
        };
        let split = if is_split { Some((r.u32()?, r.u16()?, r.u32()?)) } else { None };
        let body = r.take(len)?;

        if let Some(i) = ridx {
            if i < self.seen_low || i > self.seen_low + 8192 || !self.seen.insert(i) {
                return Some(()); // doublon ou hors fenêtre
            }
            while self.seen.remove(&self.seen_low) {
                self.seen_low += 1;
            }
        }

        let whole = match split {
            None => body.to_vec(),
            Some((count, id, index)) => {
                if count == 0 || count > MAX_SPLIT_PARTS || index >= count {
                    return Some(());
                }
                if !self.splits.contains_key(&id) && self.splits.len() >= self.max_splits {
                    return Some(());
                }
                let s = self.splits.entry(id).or_insert_with(|| Split {
                    parts: vec![None; count as usize],
                    got: 0,
                    size: 0,
                });
                if s.parts.len() != count as usize || s.parts[index as usize].is_some() {
                    return Some(());
                }
                s.size += body.len();
                if s.size > self.max_packet {
                    self.abort("paquet fragmenté trop volumineux");
                    return None;
                }
                s.parts[index as usize] = Some(body.to_vec());
                s.got += 1;
                if s.got < s.parts.len() {
                    return Some(());
                }
                let s = self.splits.remove(&id)?;
                let mut all = Vec::with_capacity(s.size);
                for p in s.parts.into_iter().flatten() {
                    all.extend_from_slice(&p);
                }
                all
            }
        };

        match ord {
            Some(i) if matches!(rel, 3 | 7) => {
                if i == self.order_next {
                    self.order_next += 1;
                    self.deliver(whole, start);
                    while let Some(next) = self.order_buf.remove(&self.order_next) {
                        self.order_next += 1;
                        self.order_bytes -= next.len();
                        self.deliver(next, start);
                    }
                } else if i > self.order_next
                    && i < self.order_next + 2048
                    && self.order_buf.len() < MAX_ORDER_PACKETS
                    && self.order_bytes + whole.len() <= MAX_ORDER_BYTES
                    && !self.order_buf.contains_key(&i)
                {
                    self.order_bytes += whole.len();
                    self.order_buf.insert(i, whole);
                }
            }
            _ => self.deliver(whole, start),
        }
        Some(())
    }

    fn deliver(&mut self, p: Vec<u8>, start: Instant) {
        let now_ms = start.elapsed().as_millis() as i64;
        match p[0] {
            0x00 => {
                // Ping connecté -> pong
                if let Some(t) = Reader::new(&p[1..]).i64() {
                    let mut out = vec![0x03];
                    out.extend_from_slice(&t.to_be_bytes());
                    out.extend_from_slice(&now_ms.to_be_bytes());
                    self.send(&out);
                }
            }
            0x09 if self.state == State::Connecting => {
                let mut r = Reader::new(&p[1..]);
                let time = match (r.i64(), r.i64()) {
                    (Some(_), Some(t)) => t,
                    _ => return,
                };
                let mut out = vec![0x10];
                put_addr(&mut out, &self.addr);
                out.extend_from_slice(&0u16.to_be_bytes());
                let none: SocketAddr = ([0, 0, 0, 0], 0).into();
                for _ in 0..20 {
                    put_addr(&mut out, &none);
                }
                out.extend_from_slice(&time.to_be_bytes());
                out.extend_from_slice(&now_ms.to_be_bytes());
                self.send(&out);
            }
            0x13 if self.state == State::Connecting => self.state = State::Connected,
            0x15 => self.state = State::Closed,
            0xfe if self.state == State::Connected => {
                if self.inbox.len() < 256 {
                    self.inbox.push(p);
                }
            }
            _ => {}
        }
    }

    fn flush(&mut self, sock: &UdpSocket, now: Instant) {
        // Accusés de réception
        if !self.ack_queue.is_empty() {
            self.ack_queue.sort_unstable();
            self.ack_queue.dedup();
            let mut recs = Vec::new();
            let mut count = 0u16;
            let mut i = 0;
            while i < self.ack_queue.len() {
                let a = self.ack_queue[i];
                let mut b = a;
                while i + 1 < self.ack_queue.len() && self.ack_queue[i + 1] == b + 1 {
                    b += 1;
                    i += 1;
                }
                i += 1;
                recs.push(if a == b { 1 } else { 0 });
                put_u24le(&mut recs, a);
                if a != b {
                    put_u24le(&mut recs, b);
                }
                count += 1;
                if recs.len() > self.mtu - 60 {
                    break;
                }
            }
            self.ack_queue.clear();
            let mut out = vec![0xc0];
            out.extend_from_slice(&count.to_be_bytes());
            out.extend_from_slice(&recs);
            let _ = sock.send_to(&out, self.addr);
            self.sent = (self.sent.0 + 1, self.sent.1 + out.len() as u64);
        }

        // Réémission de ce qui n'a pas été acquitté
        let late: Vec<u32> = self
            .unacked
            .iter()
            .filter(|(_, (t, _))| now.duration_since(*t) >= RESEND_AFTER)
            .map(|(s, _)| *s)
            .collect();
        for s in late.into_iter().rev() {
            if let Some((_, frames)) = self.unacked.remove(&s) {
                for f in frames.into_iter().rev() {
                    self.pending.push_front(f);
                }
            }
        }
        if self.unacked.len() > 4096 {
            self.abort("trop de paquets non acquittés");
            return;
        }
        if self.unacked.is_empty() {
            self.waiting_since = None;
        } else if let Some(t) = self.waiting_since {
            if now.duration_since(t) > ACK_DEADLINE {
                self.abort("aucun accusé de réception");
                return;
            }
        }

        // Envoi des trames en attente
        let room = self.mtu - 28 - 4;
        let mut budget = 24; // datagrammes par passage, pour ne pas saturer le lien
        while !self.pending.is_empty() && budget > 0 {
            budget -= 1;
            let mut out = vec![0x84];
            put_u24le(&mut out, self.seq);
            let mut frames = Vec::new();
            while let Some(f) = self.pending.front() {
                if !frames.is_empty() && out.len() + f.len() > room {
                    break;
                }
                let f = self.pending.pop_front().unwrap();
                out.extend_from_slice(&f);
                frames.push(f);
            }
            let _ = sock.send_to(&out, self.addr);
            self.sent = (self.sent.0 + 1, self.sent.1 + out.len() as u64);
            self.unacked.insert(self.seq, (now, frames));
            if self.waiting_since.is_none() {
                self.waiting_since = Some(now);
            }
            self.seq = (self.seq + 1) & 0xff_ffff;
        }
    }
}

pub struct RakServer {
    pub socket: UdpSocket,
    /// Second socket pour l'IPv6, comme le serveur officiel (port 19133 par défaut).
    socket6: Option<UdpSocket>,
    /// Copies des sockets utilisées pour la lecture.
    readers: Vec<UdpSocket>,
    pub guid: i64,
    pub sessions: HashMap<SocketAddr, Session>,
    /// Texte renvoyé aux clients qui interrogent le serveur (liste des serveurs).
    pub pong: String,
    pub max_sessions: usize,
    pub timeout: Duration,
    /// Sessions simultanées autorisées par adresse IP.
    pub max_per_ip: usize,
    /// Nouvelles sessions acceptées par seconde, toutes adresses confondues.
    pub max_new_sessions: u32,
    /// Journalise les étapes de la connexion réseau (diagnostic).
    pub debug: bool,
    last_ping_log: Option<Instant>,
    raw_logged: u64,
    recv_errors: u64,
    pub traffic: Traffic,
    start: Instant,
    // Compteurs anti-abus, remis à zéro périodiquement
    window: Instant,
    offline_per_ip: HashMap<IpAddr, u16>,
    offline_total: u32,
    new_sessions: u32,
    opens_window: Instant,
    opens_per_ip: HashMap<IpAddr, u32>,
}

/// Agrandit les tampons du socket : avec des centaines de joueurs, le tampon par
/// défaut du système déborde et des datagrammes sont perdus avant d'être lus.
#[cfg(unix)]
fn grow_buffers(sock: &UdpSocket) {
    use std::os::raw::{c_int, c_void};
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn setsockopt(fd: c_int, level: c_int, name: c_int, value: *const c_void, len: u32) -> c_int;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const OPTS: (c_int, c_int, c_int) = (1, 8, 7); // SOL_SOCKET, SO_RCVBUF, SO_SNDBUF
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const OPTS: (c_int, c_int, c_int) = (0xffff, 0x1002, 0x1001);
    let size: c_int = SOCKET_BUFFER;
    for name in [OPTS.1, OPTS.2].iter() {
        unsafe {
            setsockopt(sock.as_raw_fd(), OPTS.0, *name, &size as *const c_int as *const c_void, 4);
        }
    }
}

#[cfg(windows)]
fn grow_buffers(sock: &UdpSocket) {
    use std::os::raw::{c_char, c_int};
    use std::os::windows::io::AsRawSocket;
    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(s: usize, level: c_int, name: c_int, value: *const c_char, len: c_int) -> c_int;
    }
    let size: c_int = SOCKET_BUFFER;
    for name in [0x1002, 0x1001].iter() {
        unsafe {
            setsockopt(sock.as_raw_socket() as usize, 0xffff, *name, &size as *const c_int as *const c_char, 4);
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn grow_buffers(_: &UdpSocket) {}

fn random_guid() -> i64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.finish() as i64
}

impl RakServer {
    pub fn bind(addr: &str, max_sessions: usize, timeout: Duration) -> std::io::Result<RakServer> {
        let socket = UdpSocket::bind(addr)?;
        // Socket non bloquant : la boucle principale ne dépend jamais du réseau pour avancer.
        socket.set_nonblocking(true)?;
        grow_buffers(&socket);
        let _ = socket.set_broadcast(true);
        let readers = vec![socket.try_clone()?];
        Ok(RakServer {
            socket,
            socket6: None,
            readers,
            guid: random_guid(),
            sessions: HashMap::new(),
            pong: String::new(),
            max_sessions,
            timeout,
            max_per_ip: 5,
            max_new_sessions: MAX_NEW_SESSIONS_PER_SEC,
            debug: false,
            last_ping_log: None,
            raw_logged: 0,
            recv_errors: 0,
            traffic: Traffic::default(),
            start: Instant::now(),
            window: Instant::now(),
            offline_per_ip: HashMap::new(),
            offline_total: 0,
            new_sessions: 0,
            opens_window: Instant::now(),
            opens_per_ip: HashMap::new(),
        })
    }

    /// Écoute aussi en IPv6. Un échec n'est pas bloquant : le serveur reste joignable en IPv4.
    pub fn listen_v6(&mut self, port: u16) {
        let open = || -> std::io::Result<UdpSocket> {
            let s = UdpSocket::bind(("::", port))?;
            s.set_nonblocking(true)?;
            grow_buffers(&s);
            Ok(s)
        };
        match open().and_then(|s| s.try_clone().map(|c| (s, c))) {
            Ok((s, clone)) => {
                self.readers.push(clone);
                self.socket6 = Some(s);
            }
            Err(e) => warn!("Écoute IPv6 impossible sur le port {} ({}) : IPv4 seulement", port, e),
        }
    }

    pub fn has_v6(&self) -> bool {
        self.socket6.is_some()
    }

    fn send_raw<A: std::borrow::Borrow<SocketAddr>>(&self, data: &[u8], addr: A) {
        let addr = addr.borrow();
        let sock = match (&self.socket6, addr.is_ipv6()) {
            (Some(s6), true) => s6,
            _ => &self.socket,
        };
        let _ = sock.send_to(data, addr);
    }

    /// Lit les datagrammes déjà arrivés, sans attendre, puis fait vivre les sessions.
    /// La lecture est bornée en nombre et en durée pour que les ticks du serveur
    /// restent réguliers même sous un trafic continu. Renvoie le nombre de datagrammes lus.
    pub fn poll(&mut self, events: &mut Vec<RakEvent>) -> usize {
        let mut buf = [0u8; 2048];
        let began = Instant::now();
        let mut read = 0;
        let mut errors = 0;
        let readers = std::mem::take(&mut self.readers);
        for sock in &readers {
        for i in 0..MAX_READS_PER_POLL {
            if i % 64 == 63 && began.elapsed() > MAX_POLL_TIME {
                break;
            }
            let received = sock.recv_from(&mut buf);
            if received.is_ok() {
                read += 1;
                errors = 0;
            }
            match received {
                Ok((n, addr)) if n > 0 => {
                    self.traffic.datagrams_in += 1;
                    self.traffic.bytes_in += n as u64;
                    if self.debug && !self.sessions.contains_key(&addr) {
                        // Trace brute, avant toute analyse : prouve qu'un datagramme est arrivé
                        self.raw_logged += 1;
                        if self.raw_logged <= 40 || self.raw_logged % 500 == 0 {
                            info!(
                                "[réseau] datagramme n°{} reçu de {} : {} octets, type 0x{:02x}",
                                self.raw_logged, addr, n, buf[0]
                            );
                        }
                    }
                    // Filet de sécurité : un bug déclenché par un datagramme ne doit
                    // jamais arrêter le serveur, seulement la session fautive.
                    let data = &buf[..n];
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.on_raw(data, addr, events)
                    }));
                    if res.is_err() {
                        error!("Erreur interne en traitant un datagramme de {} : session fermée", addr);
                        if let Some(s) = self.sessions.get_mut(&addr) {
                            s.abort("erreur interne");
                        }
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
                Err(e) => {
                    // datagramme trop grand, port injoignable… : ignoré
                    if self.debug {
                        self.recv_errors += 1;
                        if self.recv_errors <= 20 || self.recv_errors % 1000 == 0 {
                            warn!("[réseau] erreur de lecture n°{} : {} ({:?})", self.recv_errors, e, e.kind());
                        }
                    }
                    errors += 1;
                    if errors > 32 {
                        break;
                    }
                }
            }
        }
        }
        self.readers = readers;
        let now = Instant::now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.offline_per_ip.clear();
            self.offline_total = 0;
            self.new_sessions = 0;
        }
        if now.duration_since(self.opens_window) >= Duration::from_secs(10) {
            self.opens_window = now;
            self.opens_per_ip.clear();
        }
        let timeout = self.timeout;
        let mut dead = Vec::new();
        for (addr, s) in self.sessions.iter_mut() {
            if s.state == State::Connected && !s.announced {
                s.announced = true;
                events.push(RakEvent::Connected(*addr));
            }
            for p in s.inbox.drain(..) {
                events.push(RakEvent::Packet(*addr, p));
            }
            let limit = if s.state == State::Connected { timeout } else { Duration::from_secs(5) };
            let expired = now.duration_since(s.last_recv) > limit
                || s.close_at.map(|t| now >= t).unwrap_or(false);
            if expired || s.state == State::Closed {
                dead.push(*addr);
            }
        }
        for addr in dead {
            self.drop_session(&addr, events);
        }
        read
    }

    fn drop_session(&mut self, addr: &SocketAddr, events: &mut Vec<RakEvent>) {
        if let Some(s) = self.sessions.remove(addr) {
            if s.state != State::Closed || s.fault.is_some() {
                // Notification de déconnexion (trame non fiable)
                let mut out = vec![0x84];
                put_u24le(&mut out, s.seq);
                out.extend_from_slice(&[0x00, 0x00, 0x08, 0x15]);
                self.send_raw(&out, addr);
            }
            if s.announced {
                events.push(RakEvent::Disconnected(*addr, s.fault));
            } else if let Some(reason) = s.fault {
                warn!("{} : connexion fermée ({})", addr, reason);
            } else if self.debug {
                info!("[réseau] {} : session abandonnée avant la fin de la poignée de main RakNet", addr);
            }
        }
    }

    fn on_raw(&mut self, d: &[u8], addr: SocketAddr, events: &mut Vec<RakEvent>) {
        if d[0] & 0x80 != 0 {
            if let Some(s) = self.sessions.get_mut(&addr) {
                s.on_datagram(d, Instant::now(), self.start);
            }
            return;
        }
        // Quota de paquets hors connexion : par adresse IP, puis global
        self.offline_total += 1;
        if self.offline_total > MAX_OFFLINE_GLOBAL {
            return;
        }
        if self.offline_per_ip.len() >= 8192 && !self.offline_per_ip.contains_key(&addr.ip()) {
            return;
        }
        let n = self.offline_per_ip.entry(addr.ip()).or_insert(0);
        *n = n.saturating_add(1);
        if *n > MAX_OFFLINE_PER_IP {
            return;
        }
        let mut r = Reader::new(&d[1..]);
        match d[0] {
            // Ping hors connexion : c'est ce qui fait apparaître le serveur dans la liste
            0x01 | 0x02 => {
                let time = match r.i64() {
                    Some(t) => t,
                    None => return,
                };
                if r.take(16) != Some(&MAGIC[..]) {
                    return;
                }
                if self.debug {
                    let now = Instant::now();
                    if self.last_ping_log.map(|t| now.duration_since(t) > Duration::from_secs(5)).unwrap_or(true) {
                        self.last_ping_log = Some(now);
                        info!("[réseau] ping reçu de {} (le serveur est joignable)", addr);
                    }
                }
                let mut out = vec![0x1c];
                out.extend_from_slice(&time.to_be_bytes());
                out.extend_from_slice(&self.guid.to_be_bytes());
                out.extend_from_slice(&MAGIC);
                out.extend_from_slice(&(self.pong.len() as u16).to_be_bytes());
                out.extend_from_slice(self.pong.as_bytes());
                self.send_raw(&out, addr);
            }
            // Demande d'ouverture de connexion n°1
            0x05 => {
                if r.take(16) != Some(&MAGIC[..]) {
                    return;
                }
                let proto = r.u8().unwrap_or(0);
                if self.debug {
                    info!("[réseau] {} : demande d'ouverture 1 (RakNet {}, {} octets)", addr, proto, d.len());
                }
                if proto != RAKNET_PROTOCOL {
                    let mut out = vec![0x19, RAKNET_PROTOCOL];
                    out.extend_from_slice(&MAGIC);
                    out.extend_from_slice(&self.guid.to_be_bytes());
                    self.send_raw(&out, addr);
                    return;
                }
                let mtu = (d.len() + 28).max(MIN_MTU).min(MAX_MTU);
                let mut out = vec![0x06];
                out.extend_from_slice(&MAGIC);
                out.extend_from_slice(&self.guid.to_be_bytes());
                out.push(0);
                out.extend_from_slice(&(mtu as u16).to_be_bytes());
                self.send_raw(&out, addr);
            }
            // Demande d'ouverture de connexion n°2 : la session est créée ici
            0x07 => {
                if r.take(16) != Some(&MAGIC[..]) || skip_addr(&mut r).is_none() {
                    return;
                }
                let (mtu, guid) = match (r.u16(), r.i64()) {
                    (Some(m), Some(g)) => ((m as usize).max(MIN_MTU).min(MAX_MTU), g),
                    _ => return,
                };
                if self.debug {
                    info!("[réseau] {} : demande d'ouverture 2 (MTU {})", addr, mtu);
                }
                if self.sessions.contains_key(&addr) {
                    self.drop_session(&addr, events);
                }
                // Limites : débit global d'ouvertures, puis par adresse IP
                self.new_sessions += 1;
                if self.new_sessions > self.max_new_sessions {
                    return;
                }
                let opens = self.opens_per_ip.entry(addr.ip()).or_insert(0);
                *opens += 1;
                if *opens as usize > self.max_per_ip * 3 {
                    return;
                }
                let same_ip = self.sessions.keys().filter(|a| a.ip() == addr.ip()).count();
                if self.sessions.len() >= self.max_sessions || same_ip >= self.max_per_ip {
                    warn!("{} : connexion refusée (serveur plein ou trop de connexions depuis cette adresse)", addr);
                    let mut out = vec![0x14];
                    out.extend_from_slice(&MAGIC);
                    out.extend_from_slice(&self.guid.to_be_bytes());
                    self.send_raw(&out, addr);
                    return;
                }
                self.sessions.insert(addr, Session::new(addr, guid, mtu, Instant::now()));
                let mut out = vec![0x08];
                out.extend_from_slice(&MAGIC);
                out.extend_from_slice(&self.guid.to_be_bytes());
                put_addr(&mut out, &addr);
                out.extend_from_slice(&(mtu as u16).to_be_bytes());
                out.push(0);
                self.send_raw(&out, addr);
            }
            _ => {}
        }
    }

    pub fn send(&mut self, addr: &SocketAddr, payload: &[u8]) {
        if let Some(s) = self.sessions.get_mut(addr) {
            s.send(payload);
        }
    }

    /// Une fois le joueur identifié, ses paquets n'ont plus besoin d'être aussi gros.
    pub fn set_game_limits(&mut self, addr: &SocketAddr) {
        if let Some(s) = self.sessions.get_mut(addr) {
            s.max_packet = MAX_PACKET_GAME;
            s.max_splits = 4;
        }
    }

    /// Ferme immédiatement une session pour abus.
    pub fn abort(&mut self, addr: &SocketAddr, reason: &'static str) {
        if let Some(s) = self.sessions.get_mut(addr) {
            s.abort(reason);
        }
    }

    /// Ferme la session après un court délai, le temps que les derniers paquets partent.
    pub fn close(&mut self, addr: &SocketAddr, delay: Duration) {
        if let Some(s) = self.sessions.get_mut(addr) {
            if s.close_at.is_none() {
                s.close_at = Some(Instant::now() + delay);
            }
        }
    }

    pub fn flush(&mut self) {
        let now = Instant::now();
        let (s4, s6) = (&self.socket, &self.socket6);
        for s in self.sessions.values_mut() {
            let sock = match (s6, s.addr.is_ipv6()) {
                (Some(x), true) => x,
                _ => s4,
            };
            s.flush(sock, now);
            self.traffic.datagrams_out += s.sent.0;
            self.traffic.bytes_out += s.sent.1;
            s.sent = (0, 0);
        }
    }

    /// Prévient tous les clients puis vide la table des sessions.
    pub fn shutdown(&mut self) {
        self.flush();
        let addrs: Vec<SocketAddr> = self.sessions.keys().copied().collect();
        let mut sink = Vec::new();
        for a in addrs {
            self.drop_session(&a, &mut sink);
        }
    }
}
