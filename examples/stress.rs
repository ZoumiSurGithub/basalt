//! Outil de charge : simule N joueurs qui se connectent, entrent en jeu, puis
//! marchent en envoyant 20 paquets de déplacement par seconde chacun, et mesure comment le serveur tient.
//!
//!   cargo build --release
//!   cargo run --release --example stress -- 1000 [secondes] [arrivées_par_seconde]
//!
//! Linux uniquement : chaque joueur simulé utilise sa propre adresse 127.x.y.z, et
//! la consommation du serveur est lue dans /proc. Le serveur testé est le binaire
//! target/release/basalt, lancé dans un dossier temporaire sans authentification.
//!
//! Ce que la mesure couvre : la couche réseau (réception, accusés, sessions) et la
//! séquence d'entrée en jeu. Elle ne dit rien d'un futur gameplay (entités, blocs,
//! joueurs qui se voient), qui coûtera davantage.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, UdpSocket};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use basalt::bedrock::{read_varu32, write_varu32};
use basalt::raknet::MAGIC;

#[derive(PartialEq, Clone, Copy)]
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
    sock: UdpSocket,
    stage: Stage,
    seq: u32,
    ridx: u32,
    oidx: u32,
    compressed: bool,
    last_step: Instant,
    last_input: Instant,
    last_heard: Instant,
    joined_after: Option<Duration>,
    started: Option<Instant>,
    walked: f32,
    chunks: u64,
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
        let _ = self.sock.send(&d);
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

    fn open1(&mut self) {
        let mut p = vec![0x05];
        p.extend_from_slice(&MAGIC);
        p.push(11);
        p.resize(1372, 0);
        let _ = self.sock.send(&p);
    }

    fn open2(&mut self) {
        let mut p = vec![0x07];
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&[4, 0x80, 0xff, 0xff, 0xfe, 0x4a, 0xbc]);
        p.extend_from_slice(&1400u16.to_be_bytes());
        p.extend_from_slice(&7i64.to_be_bytes());
        let _ = self.sock.send(&p);
    }

    fn login(&mut self, index: usize) {
        let payload = format!(r#"{{"extraData":{{"displayName":"Bot{}","XUID":"{}"}}}}"#, index, index);
        let jwt = format!("e30.{}.sig", b64(payload.as_bytes()));
        let chain = format!(r#"{{"chain":["{}"]}}"#, jwt);
        let client = format!("e30.{}.sig", b64(br#"{"GameVersion":"1.26.65"}"#));
        let mut data = Vec::new();
        data.extend_from_slice(&(chain.len() as u32).to_le_bytes());
        data.extend_from_slice(chain.as_bytes());
        data.extend_from_slice(&(client.len() as u32).to_le_bytes());
        data.extend_from_slice(client.as_bytes());
        let mut body = 2225i32.to_be_bytes().to_vec();
        write_varu32(&mut body, data.len() as u32);
        body.extend_from_slice(&data);
        self.game(1, &body);
    }

    fn on_packet(&mut self, index: usize, p: &[u8], now: Instant) {
        match p[0] {
            0x10 if self.stage == Stage::Connecting => {
                self.frame(&[0x13, 0, 0, 0]);
                self.game(0xc1, &2225i32.to_be_bytes());
                self.stage = Stage::Joining;
            }
            0xfe => {
                let mut pos = 1;
                let inflated;
                let mut p = p;
                if self.compressed {
                    pos = 2;
                    // Lot compressé par le serveur (« zlib », algorithme 0)
                    if p.get(1) == Some(&0x00) {
                        inflated = match basalt::inflate::inflate(&p[2..], 8 << 20) {
                            Ok(v) => v,
                            Err(_) => return,
                        };
                        p = &inflated;
                        pos = 0;
                    }
                }
                while pos < p.len() {
                    let len = match read_varu32(p, &mut pos) {
                        Some(l) => l as usize,
                        None => return,
                    };
                    let end = (pos + len).min(p.len());
                    let mut q = pos;
                    let id = read_varu32(&p[..end], &mut q).unwrap_or(0);
                    let body = &p[q..end];
                    pos = end;
                    match id {
                        0x8f => {
                            self.compressed = true;
                            self.login(index);
                        }
                        6 => self.game(8, &[3, 0, 0]),
                        7 => self.game(8, &[4, 0, 0]),
                        11 => self.game(69, &[8, 16]),
                        2 if body == [0, 0, 0, 3] => {
                            self.game(113, &[2]);
                            self.stage = Stage::InGame;
                            self.joined_after = self.started.map(|s| now.duration_since(s));
                        }
                        58 if self.stage == Stage::InGame => self.chunks += 1,
                        5 => self.stage = Stage::Lost,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn on_datagram(&mut self, index: usize, d: &[u8], now: Instant) {
        self.last_heard = now;
        match d[0] {
            0x06 if self.stage == Stage::Open1 => {
                self.open2();
                self.stage = Stage::Open2;
                self.last_step = now;
            }
            0x08 if self.stage == Stage::Open2 => {
                let mut p = vec![0x09];
                p.extend_from_slice(&7i64.to_be_bytes());
                p.extend_from_slice(&1i64.to_be_bytes());
                p.push(0);
                self.frame(&p);
                self.stage = Stage::Connecting;
                self.last_step = now;
            }
            f if f & 0x80 != 0 && f & 0x60 == 0 && d.len() > 4 => {
                let mut ack = vec![0xc0, 0, 1, 1];
                ack.extend_from_slice(&d[1..4]);
                let _ = self.sock.send(&ack);
                let mut pos = 4;
                while pos + 3 <= d.len() {
                    let flags = d[pos];
                    let len = (u16::from_be_bytes([d[pos + 1], d[pos + 2]]) as usize + 7) / 8;
                    let rel = flags >> 5;
                    pos += 3;
                    if rel == 0 {
                        if d.get(pos) == Some(&0x15) {
                            self.stage = Stage::Lost;
                        }
                        return;
                    }
                    pos += 7;
                    let split = flags & 0x10 != 0;
                    if split {
                        pos += 10; // les gros lots fragmentés sont acquittés mais pas relus
                    }
                    if pos + len > d.len() || len == 0 {
                        return;
                    }
                    if !split {
                        self.on_packet(index, &d[pos..pos + len], now);
                    }
                    pos += len;
                }
            }
            _ => {}
        }
    }
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if c.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if c.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

/// Temps processeur consommé par un processus, en secondes (Linux).
fn cpu_seconds(pid: u32) -> f64 {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).unwrap_or_default();
    let after = stat.rsplit(')').next().unwrap_or("");
    let f: Vec<&str> = after.split_whitespace().collect();
    let ticks: f64 = f.get(11).and_then(|v| v.parse().ok()).unwrap_or(0.0) + f.get(12).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    ticks / 100.0
}

fn memory_mb(pid: u32) -> f64 {
    let status = std::fs::read_to_string(format!("/proc/{}/status", pid)).unwrap_or_default();
    status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(1000);
    let hold: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(15);
    let rate: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(60);
    let port = 20000 + (std::process::id() % 10000) as u16;

    let dir = std::env::temp_dir().join(format!("basalt-stress-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("server.properties"),
        format!(
            "server-ip=127.0.0.1\nserver-port={}\nmax-players={}\nxbox-auth=false\nview-distance=4\nplugins-dir=plugins\nnetwork-compression={}\n",
            port,
            count + 10,
            std::env::var("COMPRESSION").unwrap_or_else(|_| "true".into())
        ),
    )
    .unwrap();
    let exe = std::env::current_dir().unwrap().join("target/release/basalt");
    let mut server = Command::new(&exe)
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("lancez d'abord : cargo build --release");
    let pid = server.id();
    let mut stdin = server.stdin.take().unwrap();
    let stdout = server.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        BufReader::new(stdout).lines().filter_map(|l| l.ok()).collect::<Vec<String>>()
    });
    std::thread::sleep(Duration::from_millis(500));
    let target: SocketAddr = ([127, 0, 0, 1], port).into();

    let now = Instant::now();
    let mut bots: Vec<Bot> = (0..count)
        .map(|i| {
            // Une adresse source par joueur, comme sur Internet
            let ip = [127, 1 + (i / 62500) as u8, (i / 250 % 250) as u8, (i % 250) as u8 + 1];
            let sock = UdpSocket::bind(SocketAddr::from((ip, 0))).unwrap();
            sock.connect(target).unwrap();
            sock.set_nonblocking(true).unwrap();
            Bot {
                sock,
                stage: Stage::Waiting,
                seq: 0,
                ridx: 0,
                oidx: 0,
                compressed: false,
                last_step: now,
                last_input: now,
                last_heard: now,
                joined_after: None,
                started: None,
                walked: 0.0,
                chunks: 0,
            }
        })
        .collect();

    println!("{} joueurs simulés, {} arrivées par seconde, puis {} s de jeu", count, rate, hold);
    let begin = Instant::now();
    let mut buf = [0u8; 2048];
    let mut steady: Option<(Instant, f64)> = None;
    let mut inputs_sent = 0u64;
    let mut received_bytes = 0u64;
    let mut last_report = Instant::now();
    loop {
        let now = Instant::now();
        let allowed = ((now.duration_since(begin).as_secs_f64() * rate as f64) as usize + 1).min(count);
        for (i, b) in bots.iter_mut().enumerate() {
            if b.stage == Stage::Waiting && i < allowed {
                b.stage = Stage::Open1;
                b.started = Some(now);
                b.last_step = now;
                b.last_heard = now;
                b.open1();
            }
            // Le serveur limite les ouvertures par seconde : on réessaie comme un vrai client
            if (b.stage == Stage::Open1 || b.stage == Stage::Open2) && now.duration_since(b.last_step) > Duration::from_millis(700) {
                b.last_step = now;
                if b.stage == Stage::Open1 {
                    b.open1();
                } else {
                    b.open2();
                }
            }
            while let Ok(n) = b.sock.recv(&mut buf) {
                received_bytes += n as u64;
                if n > 0 {
                    b.on_datagram(i, &buf[..n], now);
                }
            }
            if b.stage == Stage::InGame && now.duration_since(b.last_input) >= Duration::from_millis(50) {
                b.last_input = now;
                // Paquet de déplacement : chaque joueur marche tout droit dans sa direction
                b.walked += 0.25;
                let angle = i as f32 * 0.37;
                let mut input = vec![0u8; 8];
                for v in [0.5 + b.walked * angle.cos(), -58.38, 0.5 + b.walked * angle.sin()] {
                    input.extend_from_slice(&v.to_le_bytes());
                }
                input.resize(48, 0);
                b.game(144, &input);
                inputs_sent += 1;
            }
            if matches!(b.stage, Stage::Connecting | Stage::Joining | Stage::InGame)
                && now.duration_since(b.last_heard) > Duration::from_secs(8)
            {
                b.stage = Stage::Lost;
            }
        }
        let in_game = bots.iter().filter(|b| b.stage == Stage::InGame).count();
        let lost = bots.iter().filter(|b| b.stage == Stage::Lost).count();
        if now.duration_since(last_report) > Duration::from_secs(5) {
            last_report = now;
            println!("  {:>4.0} s : {} en jeu, {} perdus", now.duration_since(begin).as_secs_f64(), in_game, lost);
        }
        if steady.is_none() && in_game + lost == count {
            println!("Tous les joueurs sont arrivés en {:.1} s", now.duration_since(begin).as_secs_f64());
            steady = Some((now, cpu_seconds(pid)));
            inputs_sent = 0;
        }
        if let Some((since, cpu0)) = steady {
            let elapsed = now.duration_since(since);
            if elapsed >= Duration::from_secs(hold) {
                let cpu = (cpu_seconds(pid) - cpu0) / elapsed.as_secs_f64() * 100.0;
                let mut joins: Vec<f64> = bots.iter().filter_map(|b| b.joined_after).map(|d| d.as_secs_f64()).collect();
                joins.sort_by(|a, b| a.partial_cmp(b).unwrap());
                println!();
                println!("=== Résultat pour {} joueurs ===", count);
                println!("En jeu à la fin        : {}", in_game);
                println!("Perdus                 : {}", lost);
                if !joins.is_empty() {
                    println!(
                        "Temps d'entrée en jeu  : médiane {:.2} s, pire {:.2} s",
                        joins[joins.len() / 2],
                        joins[joins.len() - 1]
                    );
                }
                println!("Paquets joueurs        : {:.0} par seconde", inputs_sent as f64 / elapsed.as_secs_f64());
                let chunks: u64 = bots.iter().map(|b| b.chunks).sum();
                println!("Chunks reçus en marche : {:.0} par seconde", chunks as f64 / elapsed.as_secs_f64());
                println!("Processeur du serveur  : {:.0} % d'un cœur", cpu);
                println!("Mémoire du serveur     : {:.0} Mo", memory_mb(pid));
                println!("Données reçues         : {:.1} Mo au total ({:.0} Ko par joueur)", received_bytes as f64 / 1048576.0, received_bytes as f64 / 1024.0 / count as f64);
                break;
            }
        }
        if now.duration_since(begin) > Duration::from_secs(600) {
            println!("Abandon : délai dépassé");
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    let _ = writeln!(stdin, "status");
    std::thread::sleep(Duration::from_millis(300));
    let _ = writeln!(stdin, "stop");
    let _ = server.wait();
    let lines = reader.join().unwrap_or_default();
    println!("--- Vu par le serveur ---");
    for l in lines.iter().filter(|l| l.contains("Charge") || l.contains("Trafic") || l.contains("Réseau") || l.contains("Joueurs :") || l.contains("surcharge") || l.contains("sécurité")).take(12) {
        println!("{}", l);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
