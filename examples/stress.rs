//! Outil de charge : simule N joueurs qui se connectent par NetherNet (le transport du vrai
//! jeu), entrent en jeu, puis marchent en envoyant 20 paquets de déplacement par seconde
//! chacun, et mesure comment le serveur tient.
//!
//!   cargo build --release
//!   cargo run --release --example stress -- 1000 [secondes] [arrivées_par_seconde]
//!
//! Le serveur testé est le binaire target/release/basalt, lancé dans un dossier temporaire,
//! sans authentification, avec `local-bots=true` (joueurs simulés admis depuis cette machine).
//! Sa consommation est lue dans /proc : hors de Linux, ces deux lignes restent à zéro.
//!
//! Ce que la mesure couvre : la liaison chiffrée (WebRTC), les sessions et la séquence
//! d'entrée en jeu. Elle ne dit rien d'un vrai gameplay (combat, blocs, entités), qui coûte
//! davantage.

#[cfg(not(feature = "nethernet"))]
fn main() {
    eprintln!("L'outil de charge a besoin du transport NetherNet : compilez avec les fonctions par défaut.");
}

#[cfg(feature = "nethernet")]
fn main() {
    stress::run();
}

#[cfg(feature = "nethernet")]
mod stress {
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use basalt::bedrock::{read_varu32, write_varu32};
use basalt::nethernet_client::NetherLink;

#[derive(PartialEq, Clone, Copy)]
enum Stage {
    Waiting,
    Connecting,
    Joining,
    InGame,
    Lost,
}

struct Bot {
    link: Option<NetherLink>,
    stage: Stage,
    compressed: bool,
    last_input: Instant,
    last_heard: Instant,
    joined_after: Option<Duration>,
    started: Option<Instant>,
    walked: f32,
    chunks: u64,
}

impl Bot {
    /// Envoie un paquet de jeu : un lot à lui seul, sans l'octet 0xfe (NetherNet s'en passe).
    fn game(&mut self, id: u32, body: &[u8]) {
        let mut inner = Vec::new();
        write_varu32(&mut inner, id);
        inner.extend_from_slice(body);
        let mut p = Vec::new();
        if self.compressed {
            p.push(0xff);
        }
        write_varu32(&mut p, inner.len() as u32);
        p.extend_from_slice(&inner);
        let sent = self.link.as_mut().map(|l| l.send(&p).is_ok()).unwrap_or(false);
        if !sent {
            self.stage = Stage::Lost;
        }
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

    /// Un lot reçu du serveur (sans octet 0xfe).
    fn on_message(&mut self, index: usize, message: &[u8], now: Instant) {
        self.last_heard = now;
        let mut pos = 0;
        let inflated;
        let mut p = message;
        if self.compressed {
            pos = 1;
            // Lot compressé par le serveur (« zlib », algorithme 0)
            if p.first() == Some(&0x00) {
                inflated = match basalt::inflate::inflate(&p[1..], 8 << 20) {
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
            let id = read_varu32(&p[..end], &mut q).unwrap_or(0) & 0x3ff;
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

pub fn run() {
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
            "server-ip=127.0.0.1\nserver-port={}\nnethernet-udp-port={}\nmax-players={}\nxbox-auth=false\nlocal-bots=true\nview-distance=4\nplugins-dir=plugins\nnetwork-compression={}\n",
            port,
            port + 10000,
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
        .map(|_| Bot { link: None, stage: Stage::Waiting, compressed: false, last_input: now, last_heard: now, joined_after: None, started: None, walked: 0.0, chunks: 0 })
        .collect();

    println!("{} joueurs simulés, {} arrivées par seconde, puis {} s de jeu", count, rate, hold);
    let begin = Instant::now();
    let mut refused: Option<String> = None;
    let mut steady: Option<(Instant, f64)> = None;
    let mut inputs_sent = 0u64;
    let mut received_bytes = 0u64;
    let mut last_report = Instant::now();
    loop {
        let now = Instant::now();
        let allowed = ((now.duration_since(begin).as_secs_f64() * rate as f64) as usize + 1).min(count);
        for (i, b) in bots.iter_mut().enumerate() {
            if b.stage == Stage::Waiting && i < allowed {
                b.started = Some(now);
                b.last_heard = now;
                // Demande d'entrée par la signalisation ; la liaison s'ouvre ensuite
                match NetherLink::connect(target, 7_000_000_000 + i as u64) {
                    Ok(link) => {
                        b.link = Some(link);
                        b.stage = Stage::Connecting;
                    }
                    Err(why) => {
                        b.stage = Stage::Lost;
                        refused.get_or_insert(why);
                    }
                }
            }
            let polled = b.link.as_mut().map(|l| (l.poll(), l.is_open(), l.is_closed()));
            if let Some((messages, open, closed)) = polled {
                if open && b.stage == Stage::Connecting {
                    b.game(0xc1, &2225i32.to_be_bytes());
                    b.stage = Stage::Joining;
                    b.last_heard = now;
                }
                match messages {
                    Ok(list) => {
                        for m in list {
                            received_bytes += m.len() as u64;
                            b.on_message(i, &m, now);
                        }
                    }
                    Err(why) => {
                        refused.get_or_insert(why);
                    }
                }
                if closed && b.stage != Stage::Lost {
                    b.stage = Stage::Lost;
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
            if let Some(why) = refused.take() {
                println!("         dernière erreur : {}", why);
            }
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
}
