//! Transport de l'outil de capture : Basalt joue ici le rôle d'un joueur et se connecte
//! au serveur officiel par RakNet (UDP), l'ancien transport du jeu.
//!
//! Le serveur officiel n'accepte une liaison NetherNet que d'un joueur muni d'un vrai
//! compte ; en RakNet, avec `online-mode=false`, il accepte un joueur qui signe lui-même
//! son identité. Il faut donc régler `transport=raknet` dans son `server.properties` le
//! temps de la capture.
//!
//! Tout est écrit ici sans bibliothèque externe, ce qui permet de tester l'outil contre
//! Basalt lui-même.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use crate::crypto::Crypt;
use crate::crypto::{sha256, EcKey, EcSecret};
use crate::json::{base64_decode, base64_encode};
use crate::raknet::{put_addr, MAGIC};
use crate::tools::capture::{build_login_as, Capture, Stage};

const GUID: i64 = 0x4261_7361_6c74_0001;

/// Identifiant RakNet de cette session. Il doit différer d'une session à l'autre : le serveur
/// officiel refuse une seconde connexion portant un identifiant déjà en ligne (cas du témoin
/// et du joueur de capture, lancés depuis la même machine).
fn guid_of(sock: &UdpSocket) -> i64 {
    let port = sock.local_addr().map(|a| a.port()).unwrap_or(0) as i64;
    GUID ^ (port << 16) ^ ((std::process::id() as i64) << 32)
}

/// Client RakNet minimal : envoi fiable et ordonné, réception réordonnée et dédoublonnée.
struct RakClient {
    sock: UdpSocket,
    server: SocketAddr,
    mtu: usize,
    seq: u32,
    reliable: u32,
    order: u32,
    split_id: u16,
    /// Trames envoyées non acquittées : numéro de datagramme → (heure, trame)
    unacked: HashMap<u32, (Instant, Vec<u8>)>,
    seen: HashSet<u32>,
    parts: HashMap<u16, Vec<Option<Vec<u8>>>>,
    waiting: HashMap<u32, Vec<u8>>,
    next_order: u32,
    inbox: VecDeque<Vec<u8>>,
    started: Instant,
    last_ping: Instant,
}

/// Entier sur 3 octets, petit-boutiste.
fn u24(d: &[u8], pos: usize) -> Option<u32> {
    let b = d.get(pos..pos + 3)?;
    Some(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
}

impl RakClient {
    /// Attend un datagramme du serveur.
    fn recv_raw(&self, wait: Duration) -> Option<Vec<u8>> {
        let _ = self.sock.set_read_timeout(Some(wait.max(Duration::from_millis(1))));
        let mut buf = [0u8; 4096];
        match self.sock.recv(&mut buf) {
            Ok(n) if n > 0 => Some(buf[..n].to_vec()),
            Ok(_) => None,
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => None,
            Err(_) => None,
        }
    }

    /// Demande au serveur de se présenter ; renvoie son annonce (texte « MCPE;… »).
    fn ping(sock: &UdpSocket) -> Option<String> {
        let mut p = vec![0x01];
        p.extend_from_slice(&1i64.to_be_bytes());
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&guid_of(sock).to_be_bytes());
        let _ = sock.set_read_timeout(Some(Duration::from_millis(700)));
        let mut buf = [0u8; 4096];
        for _ in 0..4 {
            let _ = sock.send(&p);
            if let Ok(n) = sock.recv(&mut buf) {
                if n >= 35 && buf[0] == 0x1c {
                    let len = u16::from_be_bytes([buf[33], buf[34]]) as usize;
                    return Some(String::from_utf8_lossy(buf.get(35..35 + len)?).to_string());
                }
            }
        }
        None
    }

    /// Ouvre la session (échange hors connexion, puis demande de connexion).
    fn connect(sock: UdpSocket, server: SocketAddr) -> Result<RakClient, String> {
        let guid = guid_of(&sock);
        let _ = sock.set_read_timeout(Some(Duration::from_millis(800)));
        let mut buf = [0u8; 4096];
        // 1. Taille des datagrammes : on propose du plus grand au plus petit
        let mut reply1 = None;
        'sizes: for size in [1400usize, 1200, 576] {
            let mut p = vec![0x05];
            p.extend_from_slice(&MAGIC);
            p.push(11);
            p.resize(size - 28, 0);
            for _ in 0..2 {
                let _ = sock.send(&p);
                if let Ok(n) = sock.recv(&mut buf) {
                    if n >= 28 && buf[0] == 0x06 {
                        reply1 = Some(buf[..n].to_vec());
                        break 'sizes;
                    }
                    if buf[0] == 0x19 {
                        return Err("le serveur officiel refuse cette version de RakNet".into());
                    }
                }
            }
        }
        let r = reply1.ok_or("le serveur officiel n'a pas répondu à l'ouverture de session RakNet")?;
        let secure = r[25] != 0;
        let (cookie, mtu_at) = if secure { (r.get(26..30).map(|c| c.to_vec()), 30) } else { (None, 26) };
        let mtu = r.get(mtu_at..mtu_at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize).unwrap_or(1200).clamp(400, 1400);

        // 2. Seconde demande, avec le jeton anti-usurpation si le serveur en a donné un
        let mut p = vec![0x07];
        p.extend_from_slice(&MAGIC);
        if let Some(c) = &cookie {
            p.extend_from_slice(c);
            p.push(0);
        }
        put_addr(&mut p, &server);
        p.extend_from_slice(&(mtu as u16).to_be_bytes());
        p.extend_from_slice(&guid_of(&sock).to_be_bytes());
        let mut ok = false;
        for _ in 0..3 {
            let _ = sock.send(&p);
            if let Ok(n) = sock.recv(&mut buf) {
                if n >= 25 && buf[0] == 0x08 {
                    ok = true;
                    break;
                }
            }
        }
        if !ok {
            return Err("le serveur officiel n'a pas confirmé la session RakNet".into());
        }
        let now = Instant::now();
        let mut c = RakClient {
            sock,
            server,
            mtu,
            seq: 0,
            reliable: 0,
            order: 0,
            split_id: 0,
            unacked: HashMap::new(),
            seen: HashSet::new(),
            parts: HashMap::new(),
            waiting: HashMap::new(),
            next_order: 0,
            inbox: VecDeque::new(),
            started: now,
            last_ping: now,
        };
        // 3. Demande de connexion
        let mut p = vec![0x09];
        p.extend_from_slice(&guid.to_be_bytes());
        p.extend_from_slice(&0i64.to_be_bytes());
        p.push(0);
        c.send(&p);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(m) = c.next(Duration::from_millis(50)) {
                if m.first() == Some(&0x10) {
                    break;
                }
                if m.first() == Some(&0x15) {
                    return Err("le serveur officiel a fermé la session RakNet".into());
                }
            }
            if Instant::now() > deadline {
                return Err("le serveur officiel n'a pas accepté la connexion RakNet".into());
            }
        }
        let mut p = vec![0x13];
        put_addr(&mut p, &c.server);
        let none: SocketAddr = "0.0.0.0:0".parse().map_err(|_| "adresse")?;
        for _ in 0..20 {
            put_addr(&mut p, &none);
        }
        p.extend_from_slice(&0i64.to_be_bytes());
        p.extend_from_slice(&c.millis().to_be_bytes());
        c.send(&p);
        Ok(c)
    }

    /// Millisecondes depuis le début de la capture.
    fn millis(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }

    /// Envoie une frame dans un datagramme numéroté.
    fn datagram(&mut self, frame: Vec<u8>) {
        let mut d = Vec::with_capacity(frame.len() + 4);
        d.push(0x84);
        d.extend_from_slice(&self.seq.to_le_bytes()[..3]);
        d.extend_from_slice(&frame);
        let _ = self.sock.send(&d);
        self.unacked.insert(self.seq, (Instant::now(), frame));
        self.seq += 1;
    }

    /// Envoi fiable et ordonné, découpé au besoin.
    fn send(&mut self, payload: &[u8]) {
        let order = self.order;
        self.order += 1;
        let chunk = self.mtu.saturating_sub(100).max(200);
        let pieces: Vec<&[u8]> = if payload.len() <= chunk { vec![payload] } else { payload.chunks(chunk).collect() };
        let count = pieces.len() as u32;
        let split_id = self.split_id;
        if count > 1 {
            self.split_id = self.split_id.wrapping_add(1);
        }
        for (i, piece) in pieces.iter().enumerate() {
            let mut f = Vec::with_capacity(piece.len() + 24);
            f.push(0x60 | if count > 1 { 0x10 } else { 0 });
            f.extend_from_slice(&((piece.len() * 8) as u16).to_be_bytes());
            f.extend_from_slice(&self.reliable.to_le_bytes()[..3]);
            self.reliable += 1;
            f.extend_from_slice(&order.to_le_bytes()[..3]);
            f.push(0);
            if count > 1 {
                f.extend_from_slice(&count.to_be_bytes());
                f.extend_from_slice(&split_id.to_be_bytes());
                f.extend_from_slice(&(i as u32).to_be_bytes());
            }
            f.extend_from_slice(piece);
            self.datagram(f);
            // Ne pas noyer le serveur : on écoute entre deux gros envois
            if count > 8 && i % 8 == 7 {
                self.pump(Duration::from_millis(2));
            }
        }
    }

    /// ACK ou NACK du serveur : renvoie ce qu'il n'a pas reçu.
    fn on_ack(&mut self, d: &[u8], nack: bool) -> Option<()> {
        let count = u16::from_be_bytes([*d.get(1)?, *d.get(2)?]) as usize;
        let mut pos = 3;
        let mut again = Vec::new();
        for _ in 0..count.min(4096) {
            let single = *d.get(pos)? != 0;
            let first = u24(d, pos + 1)?;
            let last = if single { first } else { u24(d, pos + 4)? };
            pos += if single { 4 } else { 7 };
            for n in first..=last.min(first.saturating_add(8192)) {
                if let Some((_, frame)) = self.unacked.remove(&n) {
                    if nack {
                        again.push(frame);
                    }
                }
            }
        }
        for frame in again {
            self.datagram(frame);
        }
        Some(())
    }

    /// Remet les paquets dans l'ordre d'envoi avant de les traiter.
    fn deliver_ordered(&mut self, index: u32, payload: Vec<u8>) {
        if index < self.next_order {
            return;
        }
        self.waiting.insert(index, payload);
        while let Some(p) = self.waiting.remove(&self.next_order) {
            self.inbox.push_back(p);
            self.next_order += 1;
        }
    }

    /// Frames d'un datagramme, morceaux de paquets découpés compris.
    fn on_frames(&mut self, d: &[u8]) -> Option<()> {
        let mut pos = 4;
        while pos < d.len() {
            let flags = *d.get(pos)?;
            let reliability = flags >> 5;
            let split = flags & 0x10 != 0;
            let len = (u16::from_be_bytes([*d.get(pos + 1)?, *d.get(pos + 2)?]) as usize + 7) / 8;
            pos += 3;
            let mut reliable_index = None;
            if matches!(reliability, 2 | 3 | 4 | 6 | 7) {
                reliable_index = Some(u24(d, pos)?);
                pos += 3;
            }
            if matches!(reliability, 1 | 4) {
                pos += 3;
            }
            let mut order_index = None;
            if matches!(reliability, 1 | 3 | 4 | 7) {
                let index = u24(d, pos)?;
                pos += 4;
                if matches!(reliability, 3 | 7) {
                    order_index = Some(index);
                }
            }
            let mut part = None;
            if split {
                let count = u32::from_be_bytes(d.get(pos..pos + 4)?.try_into().ok()?) as usize;
                let id = u16::from_be_bytes([*d.get(pos + 4)?, *d.get(pos + 5)?]);
                let index = u32::from_be_bytes(d.get(pos + 6..pos + 10)?.try_into().ok()?) as usize;
                pos += 10;
                part = Some((count, id, index));
            }
            let body = d.get(pos..pos + len)?.to_vec();
            pos += len;
            if let Some(r) = reliable_index {
                if !self.seen.insert(r) {
                    continue;
                }
            }
            let whole = match part {
                None => body,
                Some((count, id, index)) => {
                    if count == 0 || count > 100_000 || index >= count {
                        continue;
                    }
                    let slot = self.parts.entry(id).or_insert_with(|| vec![None; count]);
                    if slot.len() != count {
                        continue;
                    }
                    slot[index] = Some(body);
                    if slot.iter().any(|p| p.is_none()) {
                        continue;
                    }
                    self.parts.remove(&id).map(|v| v.into_iter().flatten().flatten().collect()).unwrap_or_default()
                }
            };
            match order_index {
                Some(i) => self.deliver_ordered(i, whole),
                None => self.inbox.push_back(whole),
            }
        }
        Some(())
    }

    /// Reçoit au plus un datagramme et fait l'entretien (acquittements, renvois, ping).
    fn pump(&mut self, wait: Duration) {
        if let Some(d) = self.recv_raw(wait) {
            let kind = d[0];
            if kind & 0x80 != 0 {
                if kind & 0x40 != 0 {
                    self.on_ack(&d, false);
                } else if kind & 0x20 != 0 {
                    self.on_ack(&d, true);
                } else if d.len() >= 4 {
                    let mut ack = vec![0xc0, 0, 1, 1];
                    ack.extend_from_slice(&d[1..4]);
                    let _ = self.sock.send(&ack);
                    self.on_frames(&d);
                }
            }
        }
        let now = Instant::now();
        let late: Vec<u32> = self
            .unacked
            .iter()
            .filter(|(_, (at, _))| now.duration_since(*at) > Duration::from_millis(1500))
            .map(|(n, _)| *n)
            .collect();
        for n in late {
            if let Some((_, frame)) = self.unacked.remove(&n) {
                self.datagram(frame);
            }
        }
        if now.duration_since(self.last_ping) > Duration::from_secs(2) {
            self.last_ping = now;
            let mut p = vec![0x00];
            p.extend_from_slice(&self.millis().to_be_bytes());
            self.send(&p);
        }
    }

    /// Prochain message complet du serveur (les pings sont traités ici).
    fn next(&mut self, wait: Duration) -> Option<Vec<u8>> {
        if self.inbox.is_empty() {
            self.pump(wait);
        }
        while let Some(m) = self.inbox.pop_front() {
            match m.first() {
                Some(0x00) if m.len() >= 9 => {
                    let mut p = vec![0x03];
                    p.extend_from_slice(&m[1..9]);
                    p.extend_from_slice(&self.millis().to_be_bytes());
                    self.send(&p);
                }
                Some(0x03) => {}
                Some(_) => return Some(m),
                None => {}
            }
        }
        None
    }
}

/// Calcule la clé de chiffrement à partir du jeton envoyé par le serveur.
pub fn session_key(jwt: &str, secret: &EcSecret) -> Result<[u8; 32], String> {
    let mut parts = jwt.split('.');
    let header = parts.next().and_then(base64_decode).ok_or("jeton de chiffrement illisible")?;
    let payload = parts.next().and_then(base64_decode).ok_or("jeton de chiffrement illisible")?;
    let header = crate::json::parse(&String::from_utf8_lossy(&header)).map_err(|e| e.to_string())?;
    let payload = crate::json::parse(&String::from_utf8_lossy(&payload)).map_err(|e| e.to_string())?;
    let x5u = header.get("x5u").and_then(|v| v.as_str()).ok_or("jeton de chiffrement sans clé publique")?;
    let salt = payload.get("salt").and_then(|v| v.as_str()).and_then(base64_decode).ok_or("jeton de chiffrement sans sel")?;
    let server_key = EcKey::from_spki(&base64_decode(x5u).ok_or("clé du serveur illisible")?).map_err(|e| e.to_string())?;
    let shared = secret.shared(&server_key).ok_or("secret partagé invalide")?;
    let mut material = salt;
    material.extend_from_slice(&shared);
    Ok(sha256(&material))
}

/// Lance la capture. `data_dir` reçoit les paquets, `payload` est le modèle de données client.
pub fn run(host: &str, port: u16, data_dir: &Path, payload: &Path) -> Result<String, String> {
    run_as(host, port, Some(data_dir), payload, "BasaltCapture", Duration::from_secs(3), ("", ""))
}

/// Une session de capture. `name` : pseudo du faux joueur. `linger` : temps passé en jeu
/// après l'apparition (le serveur envoie alors les autres joueurs). `data_dir` absent : rien
/// n'est enregistré — c'est le « témoin », second joueur dont la seule présence fait envoyer
/// au premier le paquet d'apparition d'un joueur. `account` : identifiants de compte (`mid`,
/// `xid`) inscrits dans le jeton ; vides par défaut.
pub fn run_as(host: &str, port: u16, data_dir: Option<&Path>, payload: &Path, name: &str, linger: Duration, account: (&str, &str)) -> Result<String, String> {
    let server: SocketAddr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("adresse {} : {}", host, e))?
        .next()
        .ok_or("adresse introuvable")?;
    let client_payload = std::fs::read_to_string(payload).map_err(|_| {
        format!(
            "modèle introuvable ({}). Connectez-vous une fois à Basalt avec le jeu, debug-packets=true, puis relancez la capture.",
            payload.display()
        )
    })?;
    let bind = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let sock = UdpSocket::bind(bind).map_err(|e| e.to_string())?;
    sock.connect(server).map_err(|e| e.to_string())?;

    // 1. Le serveur officiel se présente
    let motd = RakClient::ping(&sock).ok_or_else(|| {
        format!(
            "aucune réponse RakNet de {}. Dans le server.properties du serveur officiel, mettez transport=raknet (et online-mode=false), relancez-le, puis relancez la capture. Il affichera une erreur « TRANSPORT TYPE » : c'est attendu, elle concerne les vrais joueurs.",
            server
        )
    })?;
    let fields: Vec<&str> = motd.split(';').collect();
    let protocol: i32 = fields.get(2).and_then(|p| p.parse().ok()).unwrap_or(2225);
    info!("Serveur officiel trouvé : version {} (protocole {})", fields.get(3).unwrap_or(&"?"), protocol);

    // 2. Session RakNet
    let mut rak = RakClient::connect(sock, server)?;
    info!("Session RakNet ouverte (datagrammes de {} octets)", rak.mtu);

    // 3. Dialogue du jeu
    let secret = EcSecret::generate().ok_or("aucune source d'aléa disponible sur ce système")?;
    let spki = base64_encode(&secret.spki(), false, true);
    let now_unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let sign = |m: &[u8]| secret.sign_sha384(m).unwrap_or_else(|| vec![0u8; 96]);
    // Le témoin se présente avec d'autres identifiants d'appareil que le joueur de capture
    // Le témoin se présente avec d'autres identifiants d'appareil ; `account.0 == "installation"`
    // demande seulement de remplir l'identifiant d'installation s'il est vide dans le modèle.
    let witness = name != "BasaltCapture";
    let fill = account.0 == "installation";
    let account = if fill { ("", account.1) } else { account };
    let payload_b64 = if witness || fill {
        let (varied, changed, keys) = super::capture::vary_client_payload_with(client_payload.trim(), witness, if witness { 2 } else { 1 });
        // Noms de clés uniquement, jamais leurs valeurs
        info!("{} : identifiants modifiés [{}]", name, changed.join(", "));
        if witness {
            info!("Clés du modèle : {}", keys.join(", "));
        }
        varied
    } else {
        client_payload.trim().to_string()
    };
    let login = build_login_as(protocol, name, &spki, &payload_b64, &sign, now_unix, account);
    let mut capture = Capture::new(protocol, login);
    let mut crypt: Option<Crypt> = None;

    let send = |rak: &mut RakClient, crypt: &mut Option<Crypt>, message: &[u8]| {
        let mut p = vec![0xfe];
        match crypt {
            Some(c) => p.extend_from_slice(&c.seal(message)),
            None => p.extend_from_slice(message),
        }
        rak.send(&p);
    };
    let hello = capture.hello();
    send(&mut rak, &mut crypt, &hello);

    let deadline = Instant::now() + Duration::from_secs(60) + linger;
    let mut last_packet = Instant::now();
    let mut announced = Stage::Settings;
    let mut closed = false;
    // Fin de la session : `linger` après l'apparition (ou après un long silence en jeu)
    let mut end: Option<Instant> = None;
    while Instant::now() < deadline && capture.stage != Stage::Failed {
        let in_game = capture.complete || (capture.records.iter().any(|(id, _)| *id == 11) && last_packet.elapsed() > Duration::from_secs(4));
        if in_game && end.is_none() {
            end = Some(Instant::now() + linger);
        }
        if end.map(|e| Instant::now() >= e).unwrap_or(false) {
            break;
        }
        let m = match rak.next(Duration::from_millis(20)) {
            Some(m) => m,
            None => continue,
        };
        match m[0] {
            0x15 => {
                closed = true;
                break;
            }
            0xfe => {
                let plain = match &mut crypt {
                    Some(c) => c.open(&m[1..])?,
                    None => m[1..].to_vec(),
                };
                last_packet = Instant::now();
                let replies = capture.on_message(&plain);
                if let Some(jwt) = capture.handshake.take() {
                    crypt = Some(Crypt::new(session_key(&jwt, &secret)?));
                    info!("Chiffrement convenu avec le serveur officiel");
                    let reply = capture.handshake_reply();
                    send(&mut rak, &mut crypt, &reply);
                }
                for reply in replies {
                    send(&mut rak, &mut crypt, &reply);
                }
                if capture.stage != announced {
                    announced = capture.stage;
                    match announced {
                        Stage::Login => info!("Réglages réseau reçus : login envoyé"),
                        Stage::Recording => info!("Login accepté par le serveur officiel : enregistrement en cours"),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    // Départ propre, puis on laisse le temps aux derniers acquittements
    rak.send(&[0x15]);
    rak.pump(Duration::from_millis(100));

    if let Some(why) = &capture.refusal {
        return Err(why.clone());
    }
    // Numéros des paquets reçus et leur nombre : sert à repérer celui d'une commande (tp…)
    let seen: Vec<String> = capture.seen.iter().map(|(id, n)| format!("{}×{}", id, n)).collect();
    info!("{} : paquets reçus par numéro : {}", name, seen.join(" "));
    let found = capture.found();
    if found.is_empty() {
        return Err(format!(
            "aucun paquet utile reçu ({} paquets au total, étape {:?}{}). Vérifiez online-mode=false dans le server.properties du serveur officiel.",
            capture.records.len(),
            capture.stage,
            if closed { ", session fermée par le serveur" } else { "" }
        ));
    }
    let data_dir = match data_dir {
        Some(d) => d,
        None => return Ok(format!("témoin « {} » : resté en jeu, rien à enregistrer", name)),
    };
    let dir = capture.save(data_dir).map_err(|e| format!("écriture des fichiers : {}", e))?;
    let list: Vec<String> = found.iter().map(|(name, size)| format!("{} ({} octets)", name, size)).collect();
    Ok(format!("Capture réussie pour le protocole {} : {} — rangés dans {}", protocol, list.join(", "), dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_derive_the_same_key_and_understand_each_other() {
        let client = EcSecret::generate().unwrap();
        let server = EcSecret::generate().unwrap();
        let salt = [9u8; 16];
        let header = format!("{{\"x5u\":\"{}\",\"alg\":\"ES384\"}}", base64_encode(&server.spki(), false, true));
        let payload = format!("{{\"salt\":\"{}\"}}", base64_encode(&salt, false, true));
        let jwt = format!("{}.{}.sig", base64_encode(header.as_bytes(), true, false), base64_encode(payload.as_bytes(), true, false));
        let key = session_key(&jwt, &client).unwrap();
        // Côté serveur : même calcul avec la clé publique du joueur
        let mut material = salt.to_vec();
        material.extend_from_slice(&server.shared(client.public()).unwrap());
        assert_eq!(key, sha256(&material));

        let (mut a, mut b) = (Crypt::new(key), Crypt::new(key));
        for message in [&b"premier"[..], &[0u8; 1000][..], &b"x"[..]] {
            let sealed = a.seal(message);
            assert_ne!(&sealed[..message.len()], message);
            assert_eq!(b.open(&sealed).unwrap(), message);
        }
        // Un message altéré ou rejoué est refusé
        let mut bad = a.seal(b"abc");
        bad[0] ^= 1;
        assert!(b.open(&bad).is_err());
    }
}
