//! Côté joueur de NetherNet : une liaison vers un serveur Basalt, pour les joueurs simulés
//! (outil de bots, outil de charge, tests). Signalisation HTTP (`POST /v1/join/{id}` avec
//! l'offre WebRTC), puis liaison WebRTC dont le canal fiable transporte les messages du jeu.
//!
//! Un message est un lot de paquets Bedrock **sans** l'octet 0xfe de tête, comme les échange
//! le vrai jeu sur ce transport. Rien ne bloque après `connect` : `poll` fait vivre la liaison
//! et rend les messages arrivés.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use str0m::change::SdpAnswer;
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

use crate::framing::{self, Reassembler};
use crate::signaling::sdp_without_identity;

/// Taille maximale d'un message reçu du serveur (le plus gros : la liste des objets).
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// Une liaison NetherNet vers un serveur, vue du joueur.
pub struct NetherLink {
    socket: UdpSocket,
    local: SocketAddr,
    rtc: Rtc,
    reliable: ChannelId,
    inbound: Reassembler,
    /// Messages demandés avant l'ouverture du canal : envoyés dès qu'il s'ouvre
    waiting: VecDeque<Vec<u8>>,
    open: bool,
    closed: bool,
    next_timeout: Instant,
}

/// Requête HTTP simple vers la signalisation ; renvoie (code, corps).
fn http(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| format!("connexion TCP à {} : {}", addr, e))?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    let head = format!("{} {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", method, path, addr, body.len());
    stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body)).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > 1_000_000 {
                    return Err("réponse HTTP trop longue".into());
                }
                // Réponse complète dès que le corps annoncé est arrivé
                if let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
                    let wanted = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
                    if matches!(wanted, Some(len) if raw.len() >= split + 4 + len) {
                        break;
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("réponse HTTP incomplète")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("réponse HTTP illisible")?;
    Ok((status, raw[split + 4..].to_vec()))
}

/// Adresse locale par laquelle on joint `remote`.
fn local_ip_for(remote: SocketAddr) -> Option<IpAddr> {
    let probe = UdpSocket::bind(if remote.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).ok()?;
    probe.connect(remote).ok()?;
    probe.local_addr().ok().map(|a| a.ip())
}

impl NetherLink {
    /// Demande à entrer sur le serveur dont la signalisation écoute en `signal` (le port du
    /// serveur, en TCP). `id` distingue les joueurs : un nombre différent pour chacun.
    /// Bloque le temps de l'échange HTTP ; la liaison s'ouvre ensuite au fil des `poll`.
    pub fn connect(signal: SocketAddr, id: u64) -> Result<NetherLink, String> {
        let ip = local_ip_for(signal).ok_or("aucune adresse locale utilisable")?;
        let socket = UdpSocket::bind(SocketAddr::new(ip, 0)).map_err(|e| format!("prise UDP : {}", e))?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let local = socket.local_addr().map_err(|e| e.to_string())?;
        let start = Instant::now();
        let mut rtc = Rtc::builder().build(start);
        let candidate = Candidate::host(local, "udp").map_err(|e| format!("candidat : {:?}", e))?;
        let _ = rtc.add_local_candidate(candidate);
        let mut api = rtc.sdp_api();
        let reliable = api.add_channel("ReliableDataChannel".to_string());
        let _unreliable = api.add_channel("UnreliableDataChannel".to_string());
        let (offer, pending) = api.apply().ok_or("offre WebRTC vide")?;
        let offer_sdp = offer.to_sdp_string();
        let (status, body) = http(signal, "POST", &format!("/v1/join/{}", id), offer_sdp.as_bytes())?;
        let text = String::from_utf8_lossy(&body).to_string();
        let at = match text.find("v=0") {
            Some(at) if status == 200 || status == 201 => at,
            _ => return Err(format!("offre refusée par le serveur : HTTP {} « {} »", status, text.chars().take(120).collect::<String>())),
        };
        let answer = SdpAnswer::from_sdp_string(&sdp_without_identity(&text[at..])).map_err(|e| format!("réponse SDP illisible : {:?}", e))?;
        rtc.sdp_api().accept_answer(pending, answer).map_err(|e| format!("réponse SDP refusée : {:?}", e))?;
        Ok(NetherLink { socket, local, rtc, reliable, inbound: Reassembler::new(MAX_MESSAGE), waiting: VecDeque::new(), open: false, closed: false, next_timeout: start })
    }

    /// Le canal du jeu est-il ouvert (les messages partent) ?
    pub fn is_open(&self) -> bool {
        self.open && !self.closed
    }

    /// La liaison est-elle terminée (fermée par le serveur, ou perdue) ?
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Envoie un message (lot de paquets sans l'octet 0xfe). Avant l'ouverture du canal, il
    /// est gardé et partira à l'ouverture.
    pub fn send(&mut self, message: &[u8]) -> Result<(), String> {
        if self.closed {
            return Err("liaison fermée".into());
        }
        if !self.open {
            self.waiting.push_back(message.to_vec());
            return Ok(());
        }
        self.write(message)
    }

    fn write(&mut self, message: &[u8]) -> Result<(), String> {
        for frame in framing::split(message).map_err(|e| e.to_string())? {
            let mut ch = self.rtc.channel(self.reliable).ok_or("canal fermé")?;
            let outcome = format!("{:?}", ch.write(true, &frame));
            if !outcome.starts_with("Ok") {
                return Err(format!("écriture refusée : {}", outcome));
            }
        }
        Ok(())
    }

    /// Ce que la liaison veut faire maintenant : datagrammes à envoyer, événements. Les
    /// messages complets sont ajoutés à `messages`.
    fn drain(&mut self, messages: &mut Vec<Vec<u8>>) -> Result<(), String> {
        loop {
            match self.rtc.poll_output().map_err(|e| format!("liaison WebRTC : {:?}", e))? {
                Output::Timeout(at) => {
                    self.next_timeout = at;
                    return Ok(());
                }
                Output::Transmit(t) => {
                    let _ = self.socket.send_to(&t.contents, t.destination);
                }
                Output::Event(event) => match event {
                    Event::ChannelOpen(id, label) => {
                        if id == self.reliable || label == "ReliableDataChannel" {
                            self.reliable = id;
                            self.open = true;
                            while let Some(m) = self.waiting.pop_front() {
                                self.write(&m)?;
                            }
                        }
                    }
                    Event::ChannelData(data) => {
                        if data.id == self.reliable {
                            if let Some(message) = self.inbound.push(&data.data).map_err(|e| format!("message du serveur illisible : {}", e))? {
                                messages.push(message);
                            }
                        }
                    }
                    Event::ChannelClose(id) => {
                        if id == self.reliable {
                            self.closed = true;
                        }
                    }
                    _ => {}
                },
                #[allow(unreachable_patterns)]
                _ => {}
            }
        }
    }

    /// Fait vivre la liaison sans attendre : lit ce qui est arrivé, envoie ce qui doit
    /// partir, et rend les messages complets reçus du serveur. À appeler souvent (au moins
    /// vingt fois par seconde). Une erreur signale une liaison perdue.
    pub fn poll(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut messages = Vec::new();
        if self.closed {
            return Ok(messages);
        }
        let outcome = self.step(&mut messages);
        if outcome.is_err() || !self.rtc.is_alive() {
            self.closed = true;
        }
        outcome.map(|_| messages)
    }

    fn step(&mut self, messages: &mut Vec<Vec<u8>>) -> Result<(), String> {
        self.drain(messages)?;
        let mut buf = [0u8; 2048];
        // Borné : un flot continu ne doit pas retenir l'appelant
        for _ in 0..256 {
            match self.socket.recv_from(&mut buf) {
                Ok((n, source)) => {
                    if let Ok(contents) = (&buf[..n]).try_into() {
                        let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: self.local, contents });
                        if self.rtc.accepts(&input) {
                            self.rtc.handle_input(input).map_err(|e| format!("liaison WebRTC : {:?}", e))?;
                            self.drain(messages)?;
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
                // (sous Windows : « connexion réinitialisée » après un envoi non reçu ; sans gravité)
                Err(_) => continue,
            }
        }
        let now = Instant::now();
        if now >= self.next_timeout {
            self.rtc.handle_input(Input::Timeout(now)).map_err(|e| format!("liaison WebRTC : {:?}", e))?;
            self.drain(messages)?;
        }
        Ok(())
    }
}

impl Drop for NetherLink {
    fn drop(&mut self) {
        self.rtc.disconnect();
        // Les derniers datagrammes (fermeture) partent si possible
        for _ in 0..8 {
            match self.rtc.poll_output() {
                Ok(Output::Transmit(t)) => {
                    let _ = self.socket.send_to(&t.contents, t.destination);
                }
                _ => break,
            }
        }
    }
}
