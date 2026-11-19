//! Transport NetherNet : liaison WebRTC (ICE, DTLS, SCTP) portée par la
//! bibliothèque `str0m`, sur un socket UDP dédié.
//!
//! Déroulement pour un joueur :
//!   1. son offre SDP arrive par HTTP (voir `signaling.rs`) ; `accept` crée la
//!      session WebRTC et renvoie la réponse, signée par la clé du serveur ;
//!   2. le jeu ouvre la liaison UDP et deux canaux de données ; seul
//!      `ReliableDataChannel` transporte les paquets du jeu ;
//!   3. chaque message est découpé en fragments (voir `framing.rs`).
//!
//! `str0m` ne touche pas au réseau : ce module lit le socket, lui passe les
//! datagrammes, et envoie ce qu'elle demande d'émettre. Tout tourne dans la
//! boucle principale du serveur, sans fil d'exécution supplémentaire.
//!
//! Vers le reste du serveur, ce transport émet les mêmes événements que RakNet
//! (`RakEvent`), avec des paquets préfixés de l'octet 0xfe, pour que la couche
//! de jeu n'ait pas à distinguer les deux.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant};

use str0m::change::SdpOffer;
use str0m::channel::ChannelId;
use str0m::net::{DatagramRecv, Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

use crate::framing::{self, Reassembler};
use crate::identity::ServerIdentity;
use crate::raknet::{RakEvent, MAX_PACKET_GAME, MAX_PACKET_LOGIN};
use crate::signaling::{sdp_fingerprints, sdp_insert_identity};

/// Canal qui transporte les paquets du jeu.
const RELIABLE_CHANNEL: &str = "ReliableDataChannel";
/// Délai accordé au jeu pour ouvrir la liaison après la réponse.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Sessions en cours de négociation acceptées en même temps.
const MAX_PENDING: usize = 64;
/// Datagrammes lus au plus par passage.
const MAX_READS: usize = 4096;
/// Données en attente d'envoi tolérées par joueur avant de couper la liaison.
const MAX_OUTBOUND: usize = 16 * 1024 * 1024;

struct Peer {
    key: SocketAddr,
    rtc: Rtc,
    /// Adresses locales annoncées au joueur (candidats ICE) ; la première est la principale
    locals: Vec<SocketAddr>,
    debug: bool,
    /// Dernière correspondance adresse du joueur -> candidat local (évite de la recalculer)
    route: Option<(SocketAddr, SocketAddr)>,
    /// Compteurs pour le diagnostic
    received: u64,
    sent: u64,
    messages_in: u64,
    messages_out: u64,
    refused_writes: u64,
    stats_at: Instant,
    /// Fragments en attente d'écriture : la pile WebRTC refuse d'en prendre plus quand son
    /// tampon est plein, il faut alors patienter au lieu de perdre des données.
    outbound: VecDeque<Vec<u8>>,
    outbound_bytes: usize,
    reliable: Option<ChannelId>,
    /// La couche de jeu a été prévenue de la connexion
    announced: bool,
    created: Instant,
    /// Prochain instant où la session veut être réveillée
    timeout: Instant,
    inbound: Reassembler,
    /// Clé publique du joueur prouvée dans l'offre, si elle a été vérifiée
    cpk: Option<String>,
    close_at: Option<Instant>,
    /// Session terminée ; le texte précise la raison si le serveur l'a fermée pour abus
    dead: Option<Option<&'static str>>,
    /// Des données ont été écrites depuis le dernier passage
    dirty: bool,
}

pub struct NetherNet {
    socket: UdpSocket,
    port: u16,
    public_ip: Option<IpAddr>,
    identity: ServerIdentity,
    peers: HashMap<SocketAddr, Peer>,
    /// Adresse UDP du joueur -> clé de sa session
    by_remote: HashMap<SocketAddr, SocketAddr>,
    pub max_peers: usize,
    pub debug: bool,
}

/// Adresse locale que le système utiliserait pour joindre `remote` (aucun paquet n'est émis).
fn route_local_ip(remote: SocketAddr) -> Option<IpAddr> {
    let bind = if remote.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let probe = UdpSocket::bind(bind).ok()?;
    probe.connect(remote).ok()?;
    let ip = probe.local_addr().ok()?.ip();
    if ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

/// Adresses (IP et port) des candidats annoncés dans une description SDP.
fn sdp_candidates(sdp: &str) -> Vec<SocketAddr> {
    sdp.lines()
        .filter_map(|l| l.trim_end().strip_prefix("a=candidate:"))
        .filter_map(|v| {
            let f: Vec<&str> = v.split_whitespace().collect();
            if f.len() >= 6 && f[2].eq_ignore_ascii_case("udp") {
                Some(SocketAddr::new(f[4].parse().ok()?, f[5].parse().ok()?))
            } else {
                None
            }
        })
        .take(16)
        .collect()
}

impl Peer {
    /// Candidat local par lequel un datagramme venu de `source` est arrivé.
    fn local_for(&self, source: SocketAddr) -> SocketAddr {
        if let Some(ip) = route_local_ip(source) {
            if let Some(l) = self.locals.iter().find(|l| l.ip() == ip) {
                return *l;
            }
        }
        if let Some(l) = self.locals.iter().find(|l| l.ip() == source.ip()) {
            return *l;
        }
        self.locals[0]
    }

    /// Écrit sur le canal fiable autant de fragments en attente que la pile en accepte.
    fn pump(&mut self) {
        let id = match self.reliable {
            Some(id) => id,
            None => return,
        };
        while let Some(frame) = self.outbound.front() {
            let outcome = match self.rtc.channel(id) {
                // Le type exact du résultat a changé selon les versions de la bibliothèque :
                // on le lit sous forme de texte pour rester compatible.
                Some(mut channel) => format!("{:?}", channel.write(true, frame)),
                None => return,
            };
            if outcome == "Ok(false)" || outcome == "Ok(0)" {
                self.refused_writes += 1;
                return; // tampon plein : on réessaiera au prochain passage
            }
            if !outcome.starts_with("Ok") {
                if self.debug {
                    info!("[nethernet] {} : écriture refusée ({})", self.key, outcome);
                }
                self.dead = Some(Some("écriture impossible sur la liaison WebRTC"));
                return;
            }
            self.outbound_bytes -= frame.len();
            self.outbound.pop_front();
            self.dirty = true;
        }
    }

    /// Exécute ce que la session demande : émissions réseau et événements.
    fn drain(&mut self, socket: &UdpSocket, events: &mut Vec<RakEvent>) {
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(at)) => {
                    self.timeout = at;
                    return;
                }
                Ok(Output::Transmit(t)) => {
                    self.sent += 1;
                    if self.debug && self.sent <= 3 {
                        info!("[nethernet] {} : envoi n°{} vers {} ({} octets)", self.key, self.sent, t.destination, t.contents.len());
                    }
                    if let Err(e) = socket.send_to(&t.contents, t.destination) {
                        if self.debug && self.sent <= 20 {
                            info!("[nethernet] {} : envoi vers {} impossible ({})", self.key, t.destination, e);
                        }
                    }
                }
                Ok(Output::Event(event)) => self.on_event(event, events),
                #[allow(unreachable_patterns)]
                Ok(_) => {}
                Err(_) => {
                    self.dead = Some(Some("erreur de la liaison WebRTC"));
                    return;
                }
            }
        }
    }

    fn on_event(&mut self, event: Event, events: &mut Vec<RakEvent>) {
        match event {
            Event::ChannelOpen(id, label) => {
                if self.debug {
                    info!("[nethernet] {} : canal « {} » ouvert", self.key, label);
                }
                if label == RELIABLE_CHANNEL {
                    self.reliable = Some(id);
                    if !self.announced {
                        self.announced = true;
                        events.push(RakEvent::Connected(self.key));
                    }
                }
            }
            Event::ChannelData(data) => {
                if Some(data.id) != self.reliable {
                    return; // le canal non fiable n'est pas utilisé par le serveur
                }
                match self.inbound.push(&data.data) {
                    Ok(Some(message)) => {
                        self.messages_in += 1;
                        let mut packet = Vec::with_capacity(message.len() + 1);
                        packet.push(0xfe);
                        packet.extend_from_slice(&message);
                        events.push(RakEvent::Packet(self.key, packet));
                    }
                    Ok(None) => {}
                    Err(why) => self.dead = Some(Some(why)),
                }
            }
            Event::ChannelClose(id) => {
                if self.debug {
                    info!("[nethernet] {} : canal fermé par le jeu (fiable : {})", self.key, Some(id) == self.reliable);
                }
                if Some(id) == self.reliable {
                    self.dead = Some(None);
                }
            }
            Event::IceConnectionStateChange(state) => {
                if self.debug {
                    info!("[nethernet] {} : état ICE {:?}", self.key, state);
                }
                if state == IceConnectionState::Disconnected {
                    self.dead = Some(None);
                }
            }
            Event::Connected => {
                if self.debug {
                    info!("[nethernet] {} : liaison chiffrée établie (ICE + DTLS)", self.key);
                }
            }
            _ => {}
        }
    }
}

impl NetherNet {
    pub const AVAILABLE: bool = true;

    /// Ouvre le socket UDP de la liaison WebRTC et charge la clé du serveur.
    pub fn bind(ip: &str, port: u16, public_ip: &str, domain: &str, cpk_as_jwk: bool) -> Result<NetherNet, String> {
        let socket = UdpSocket::bind(format!("{}:{}", ip, port)).map_err(|e| format!("port UDP {} : {}", port, e))?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = socket.local_addr().map(|a| a.port()).unwrap_or(port);
        let public_ip = match public_ip.trim() {
            "" => None,
            text => Some(text.parse::<IpAddr>().map_err(|_| format!("public-ip invalide : {}", text))?),
        };
        let identity = ServerIdentity::load_or_generate(Path::new("server-identity.key"), domain, cpk_as_jwk)?;
        Ok(NetherNet {
            socket,
            port,
            public_ip,
            identity,
            peers: HashMap::new(),
            by_remote: HashMap::new(),
            max_peers: 64,
            debug: false,
        })
    }

    pub fn key_fingerprint(&self) -> &str {
        self.identity.fingerprint()
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Accepte l'offre SDP d'un joueur (sans sa ligne d'identité, déjà vérifiée) et
    /// renvoie la réponse signée. `local_ip` est l'adresse du serveur que le joueur
    /// a utilisée pour la signalisation : elle lui est donc joignable.
    pub fn accept(
        &mut self,
        key: SocketAddr,
        local_ip: IpAddr,
        offer: &str,
        cpk: Option<String>,
        now_unix: u64,
    ) -> Result<String, (u16, &'static str)> {
        let pending = self.peers.values().filter(|p| !p.announced).count();
        if self.peers.len() >= self.max_peers || pending >= MAX_PENDING {
            return Err((503, "server busy"));
        }
        if let Some(old) = self.peers.get_mut(&key) {
            old.dead = Some(None); // même origine : l'ancienne tentative est abandonnée
            return Err((503, "retry"));
        }
        let now = Instant::now();
        let _t = crate::timings::scope("NetherNet — nouvelle connexion (préparation)");
        let mut rtc = Rtc::builder().build(now);
        // Candidats : l'adresse par laquelle le joueur a joint le serveur, plus celle que le
        // système utiliserait pour joindre chacune des adresses annoncées par le joueur
        // (indispensable quand le jeu tourne sur la même machine et annonce son adresse locale).
        let mut ips = vec![self.public_ip.unwrap_or(local_ip)];
        if self.public_ip.is_none() {
            for remote in sdp_candidates(offer) {
                if let Some(ip) = route_local_ip(remote) {
                    if !ips.contains(&ip) && ips.len() < 4 {
                        ips.push(ip);
                    }
                }
            }
        }
        let mut locals = Vec::new();
        for ip in ips {
            let addr = SocketAddr::new(ip, self.port);
            if let Ok(candidate) = Candidate::host(addr, "udp") {
                let _ = rtc.add_local_candidate(candidate);
                locals.push(addr);
            }
        }
        if locals.is_empty() {
            return Err((503, "no usable address"));
        }

        let offer = SdpOffer::from_sdp_string(offer).map_err(|_| (400, "invalid SDP offer"))?;
        let answer = rtc.sdp_api().accept_offer(offer).map_err(|_| (400, "SDP offer rejected"))?;
        let sdp = answer.to_sdp_string();

        // Le jeu refuse toute réponse qui ne porte pas l'identité signée du serveur.
        let fingerprints = sdp_fingerprints(&sdp);
        if fingerprints.is_empty() {
            return Err((503, "no DTLS fingerprint"));
        }
        let signed = sdp_insert_identity(&sdp, &self.identity.assertion(&fingerprints, now_unix));
        if self.debug {
            info!("[nethernet] {} : réponse envoyée, candidats {:?}", key, locals);
        }
        self.peers.insert(
            key,
            Peer {
                key,
                rtc,
                locals,
                debug: self.debug,
                route: None,
                received: 0,
                sent: 0,
                messages_in: 0,
                messages_out: 0,
                refused_writes: 0,
                stats_at: now,
                outbound: VecDeque::new(),
                outbound_bytes: 0,
                reliable: None,
                announced: false,
                created: now,
                timeout: now,
                inbound: Reassembler::new(MAX_PACKET_LOGIN),
                cpk,
                close_at: None,
                dead: None,
                dirty: true,
            },
        );
        Ok(signed)
    }

    /// Lit les datagrammes arrivés et fait vivre les sessions. Renvoie le nombre lu.
    pub fn poll(&mut self, events: &mut Vec<RakEvent>) -> usize {
        let now = Instant::now();
        let mut buf = [0u8; 2048];
        let mut read = 0;
        for _ in 0..MAX_READS {
            match self.socket.recv_from(&mut buf) {
                Ok((n, source)) if n > 0 => {
                    read += 1;
                    self.on_datagram(&buf[..n], source, now, events);
                }
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
                Err(_) => {}
            }
        }
        for peer in self.peers.values_mut() {
            if peer.dead.is_some() {
                continue;
            }
            if now >= peer.timeout {
                let _t = crate::timings::scope("NetherNet — minuteries des liaisons");
                if peer.rtc.handle_input(Input::Timeout(now)).is_err() {
                    peer.dead = Some(Some("erreur de la liaison WebRTC"));
                    continue;
                }
                peer.drain(&self.socket, events);
            }
            if peer.debug && now.duration_since(peer.stats_at) >= Duration::from_secs(5) {
                peer.stats_at = now;
                info!(
                    "[nethernet] {} : datagrammes reçus {} / envoyés {}, messages reçus {} / envoyés {}, en attente {} ({} octets), écritures refusées {}, réassemblage en cours {} octets",
                    peer.key,
                    peer.received,
                    peer.sent,
                    peer.messages_in,
                    peer.messages_out,
                    peer.outbound.len(),
                    peer.outbound_bytes,
                    peer.refused_writes,
                    peer.inbound.pending()
                );
            }
            if !peer.announced && now.duration_since(peer.created) > CONNECT_TIMEOUT {
                peer.dead = Some(None);
            }
            if peer.close_at.map(|t| now >= t).unwrap_or(false) || !peer.rtc.is_alive() {
                if peer.debug && peer.dead.is_none() {
                    info!(
                        "[nethernet] {} : fin de session (fermeture demandée par le serveur : {}, liaison encore active : {})",
                        peer.key,
                        peer.close_at.is_some(),
                        peer.rtc.is_alive()
                    );
                }
                peer.dead = Some(peer.dead.unwrap_or(None));
            }
        }
        self.reap(events);
        read
    }

    fn on_datagram(&mut self, data: &[u8], source: SocketAddr, now: Instant, events: &mut Vec<RakEvent>) {
        // Session déjà associée à cette adresse, sinon celle qui reconnaît le paquet (STUN)
        let key = match self.by_remote.get(&source) {
            Some(k) => Some(*k),
            None => {
                let mut found = None;
                'search: for (k, peer) in self.peers.iter() {
                    for local in peer.locals.iter() {
                        let contents: DatagramRecv = match data.try_into() {
                            Ok(c) => c,
                            Err(_) => {
                                if self.debug {
                                    info!("[nethernet] datagramme inconnu de {} ({} octets), ignoré", source, data.len());
                                }
                                return;
                            }
                        };
                        let probe =
                            Input::Receive(now, Receive { proto: Protocol::Udp, source, destination: *local, contents });
                        if peer.rtc.accepts(&probe) {
                            found = Some(*k);
                            break 'search;
                        }
                    }
                }
                if found.is_none() && self.debug {
                    info!(
                        "[nethernet] datagramme de {} ({} octets, type 0x{:02x}) sans session correspondante",
                        source,
                        data.len(),
                        data[0]
                    );
                }
                if let Some(k) = found {
                    if self.by_remote.len() < 65_536 {
                        self.by_remote.insert(source, k);
                    }
                }
                found
            }
        };
        let peer = match key.and_then(|k| self.peers.get_mut(&k)) {
            Some(p) if p.dead.is_none() => p,
            _ => return,
        };
        let contents: DatagramRecv = match data.try_into() {
            Ok(c) => c,
            Err(_) => return,
        };
        peer.received += 1;
        if peer.debug && peer.received <= 3 {
            info!("[nethernet] {} : datagramme n°{} reçu de {} ({} octets)", peer.key, peer.received, source, data.len());
        }
        let destination = match peer.route {
            Some((from, local)) if from == source => local,
            _ => {
                let local = peer.local_for(source);
                peer.route = Some((source, local));
                local
            }
        };
        let input = Input::Receive(now, Receive { proto: Protocol::Udp, source, destination, contents });
        {
            let _t = crate::timings::scope("NetherNet — datagramme reçu (déchiffrement, négociation)");
            if peer.rtc.handle_input(input).is_err() {
                peer.dead = Some(Some("datagramme WebRTC invalide"));
                return;
            }
        }
        let _t = crate::timings::scope("NetherNet — réponses et messages du jeu");
        peer.drain(&self.socket, events);
    }

    /// Émet ce qui a été écrit depuis le dernier passage.
    pub fn flush(&mut self, events: &mut Vec<RakEvent>) {
        for peer in self.peers.values_mut() {
            if peer.dead.is_some() {
                continue;
            }
            if !peer.outbound.is_empty() {
                peer.pump();
            }
            if peer.dirty {
                peer.dirty = false;
                peer.drain(&self.socket, events);
            }
        }
    }

    /// Retire les sessions terminées et prévient la couche de jeu.
    fn reap(&mut self, events: &mut Vec<RakEvent>) {
        let gone: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.dead.is_some()).map(|(k, _)| *k).collect();
        if gone.is_empty() {
            return;
        }
        let _t = crate::timings::scope("NetherNet — fin de session");
        for key in &gone {
            if let Some(mut peer) = self.peers.remove(key) {
                peer.rtc.disconnect();
                let reason = peer.dead.unwrap_or(None);
                if peer.announced {
                    events.push(RakEvent::Disconnected(*key, reason));
                } else if let Some(why) = reason {
                    warn!("{} : connexion NetherNet fermée ({})", key, why);
                } else if self.debug {
                    info!(
                        "[nethernet] {} : négociation abandonnée ({} datagrammes reçus, {} envoyés)",
                        key, peer.received, peer.sent
                    );
                }
            }
        }
        let peers = &self.peers;
        self.by_remote.retain(|_, k| peers.contains_key(k));
    }

    pub fn has(&self, key: &SocketAddr) -> bool {
        self.peers.contains_key(key)
    }

    /// Clé publique que le joueur a prouvée dans son offre, si elle a été vérifiée.
    pub fn bound_cpk(&self, key: &SocketAddr) -> Option<String> {
        self.peers.get(key).and_then(|p| p.cpk.clone())
    }

    /// Envoie un paquet de jeu (sans l'octet 0xfe) sur le canal fiable.
    pub fn send(&mut self, key: &SocketAddr, message: &[u8]) {
        let peer = match self.peers.get_mut(key) {
            Some(p) if p.dead.is_none() => p,
            _ => return,
        };
        let id = match peer.reliable {
            Some(id) => id,
            None => return,
        };
        let frames = match framing::split(message) {
            Ok(f) => f,
            Err(why) => {
                peer.dead = Some(Some(why));
                return;
            }
        };
        let _ = id;
        peer.messages_out += 1;
        for frame in frames {
            peer.outbound_bytes += frame.len();
            peer.outbound.push_back(frame);
        }
        if peer.outbound_bytes > MAX_OUTBOUND {
            peer.dead = Some(Some("le joueur ne lit plus les données envoyées"));
            return;
        }
        peer.pump();
        peer.dirty = true;
    }

    /// Ferme la session après un court délai, le temps que les derniers paquets partent.
    pub fn close(&mut self, key: &SocketAddr, delay: Duration) {
        if let Some(p) = self.peers.get_mut(key) {
            if p.close_at.is_none() {
                p.close_at = Some(Instant::now() + delay);
            }
        }
    }

    /// Ferme immédiatement une session pour abus.
    pub fn abort(&mut self, key: &SocketAddr, reason: &'static str) {
        if let Some(p) = self.peers.get_mut(key) {
            if p.dead.is_none() {
                p.dead = Some(Some(reason));
            }
        }
    }

    /// Une fois le joueur identifié, ses paquets n'ont plus besoin d'être aussi gros.
    pub fn set_game_limits(&mut self, key: &SocketAddr) {
        if let Some(p) = self.peers.get_mut(key) {
            p.inbound.limit = MAX_PACKET_GAME;
        }
    }

    pub fn shutdown(&mut self) {
        let mut sink = Vec::new();
        self.flush(&mut sink);
        for p in self.peers.values_mut() {
            p.rtc.disconnect();
        }
        self.peers.clear();
        self.by_remote.clear();
    }
}
