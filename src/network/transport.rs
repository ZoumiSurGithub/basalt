//! Ce que les transports ont en commun (événements, compteurs, limites), et le **transport de
//! test** : une liaison TCP toute simple, limitée à cette machine, pour les tests de bout en
//! bout et les outils qui n'ont pas besoin de NetherNet.
//!
//! Les joueurs, eux, n'entrent que par NetherNet (`nethernet.rs`). Le transport de test ne
//! s'active que par programme (`Config::test_transport`) : aucun réglage de
//! `server.properties` ne l'ouvre, et il n'écoute que sur 127.0.0.1.
//!
//! Format : chaque message est précédé de sa longueur (4 octets, petit-boutiste). Un message
//! du jeu est un lot de paquets Bedrock avec son octet 0xfe de tête. Le message d'un seul
//! octet 0x01 demande l'état du serveur, rendu dans un message qui commence par 0x1c.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Événements réseau remontés au serveur, quel que soit le transport.
pub enum NetEvent {
    Connected(SocketAddr),
    /// Un lot de paquets, avec son octet 0xfe de tête
    Packet(SocketAddr, Vec<u8>),
    /// Session terminée ; le texte précise la raison si le serveur l'a fermée pour abus.
    Disconnected(SocketAddr, Option<&'static str>),
}

/// Compteurs de trafic depuis le démarrage.
#[derive(Default, Clone, Copy)]
pub struct Traffic {
    pub datagrams_in: u64,
    pub datagrams_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// Taille maximale d'un paquet avant le login (le login contient le skin).
pub const MAX_PACKET_LOGIN: usize = 3 * 1024 * 1024;
/// Taille maximale d'un paquet une fois le joueur identifié.
pub const MAX_PACKET_GAME: usize = 512 * 1024;
/// Données en attente d'envoi pour une liaison de test ; au-delà, elle est fermée.
const MAX_BACKLOG: usize = 64 * 1024 * 1024;

struct Conn {
    stream: TcpStream,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    max_packet: usize,
    close_at: Option<Instant>,
    aborted: Option<&'static str>,
}

/// Côté serveur du transport de test.
pub struct LocalServer {
    listener: Option<TcpListener>,
    conns: HashMap<SocketAddr, Conn>,
    max_sessions: usize,
    pub traffic: Traffic,
    /// État du serveur rendu à qui le demande (nom, version, joueurs)
    pub status: String,
}

impl LocalServer {
    /// Transport fermé : c'est le cas normal, hors des tests.
    pub fn off() -> LocalServer {
        LocalServer { listener: None, conns: HashMap::new(), max_sessions: 0, traffic: Traffic::default(), status: String::new() }
    }

    /// Ouvre le transport de test sur un port libre de cette machine.
    pub fn bind_local(max_sessions: usize) -> std::io::Result<LocalServer> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        Ok(LocalServer { listener: Some(listener), conns: HashMap::new(), max_sessions, traffic: Traffic::default(), status: String::new() })
    }

    /// Adresse d'écoute, si le transport est ouvert.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    /// Nombre de liaisons ouvertes.
    pub fn len(&self) -> usize {
        self.conns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conns.is_empty()
    }

    /// Accepte les nouvelles liaisons, lit ce qui est arrivé et rend les événements. Renvoie
    /// le nombre de messages lus.
    pub fn poll(&mut self, events: &mut Vec<NetEvent>) -> usize {
        if let Some(listener) = &self.listener {
            loop {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        // Cette machine seulement, et pas au-delà du nombre de places
                        if !addr.ip().is_loopback() || self.conns.len() >= self.max_sessions || stream.set_nonblocking(true).is_err() {
                            continue;
                        }
                        let _ = stream.set_nodelay(true);
                        self.conns.insert(addr, Conn { stream, inbuf: Vec::new(), outbuf: Vec::new(), max_packet: MAX_PACKET_LOGIN, close_at: None, aborted: None });
                        events.push(NetEvent::Connected(addr));
                    }
                    Err(_) => break,
                }
            }
        }
        let now = Instant::now();
        let mut read = 0;
        let mut gone: Vec<(SocketAddr, Option<&'static str>)> = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        for (addr, c) in self.conns.iter_mut() {
            if let Some(reason) = c.aborted {
                gone.push((*addr, Some(reason)));
                continue;
            }
            if matches!(c.close_at, Some(at) if at <= now) {
                gone.push((*addr, None));
                continue;
            }
            let mut closed = false;
            loop {
                match c.stream.read(&mut chunk) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        c.inbuf.extend_from_slice(&chunk[..n]);
                        self.traffic.bytes_in += n as u64;
                        if c.inbuf.len() > c.max_packet + 4 {
                            break; // de quoi juger le message en attente
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            }
            // Messages complets
            let mut at = 0;
            let mut fault = None;
            while c.inbuf.len() - at >= 4 {
                let len = u32::from_le_bytes([c.inbuf[at], c.inbuf[at + 1], c.inbuf[at + 2], c.inbuf[at + 3]]) as usize;
                if len == 0 || len > c.max_packet {
                    fault = Some("paquet trop gros");
                    break;
                }
                if c.inbuf.len() - at - 4 < len {
                    break;
                }
                let message = c.inbuf[at + 4..at + 4 + len].to_vec();
                at += 4 + len;
                read += 1;
                self.traffic.datagrams_in += 1;
                if message == [0x01] {
                    // Demande d'état : réponse immédiate, sans passer par le serveur
                    let mut reply = vec![0x1c];
                    reply.extend_from_slice(self.status.as_bytes());
                    c.outbuf.extend_from_slice(&(reply.len() as u32).to_le_bytes());
                    c.outbuf.extend_from_slice(&reply);
                } else {
                    events.push(NetEvent::Packet(*addr, message));
                }
            }
            c.inbuf.drain(..at);
            if let Some(reason) = fault {
                gone.push((*addr, Some(reason)));
            } else if closed {
                gone.push((*addr, None));
            }
        }
        for (addr, reason) in gone {
            if let Some(mut c) = self.conns.remove(&addr) {
                // Les derniers messages (le motif de l'exclusion…) partent si possible
                let _ = c.stream.set_nonblocking(false);
                let _ = c.stream.set_write_timeout(Some(Duration::from_millis(200)));
                let _ = c.stream.write_all(&c.outbuf);
                let _ = c.stream.shutdown(std::net::Shutdown::Both);
            }
            events.push(NetEvent::Disconnected(addr, reason));
        }
        read
    }

    /// Met un message en attente d'envoi (il part au prochain `flush`).
    pub fn send(&mut self, addr: &SocketAddr, payload: &[u8]) {
        if let Some(c) = self.conns.get_mut(addr) {
            c.outbuf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            c.outbuf.extend_from_slice(payload);
            self.traffic.datagrams_out += 1;
            self.traffic.bytes_out += payload.len() as u64;
            if c.outbuf.len() > MAX_BACKLOG {
                c.aborted = Some("le joueur ne lit plus ce qu'on lui envoie");
            }
        }
    }

    /// Une fois le joueur identifié, ses paquets n'ont plus besoin d'être aussi gros.
    pub fn set_game_limits(&mut self, addr: &SocketAddr) {
        if let Some(c) = self.conns.get_mut(addr) {
            c.max_packet = MAX_PACKET_GAME;
        }
    }

    /// Ferme une liaison pour abus (l'événement de fin porte la raison).
    pub fn abort(&mut self, addr: &SocketAddr, reason: &'static str) {
        if let Some(c) = self.conns.get_mut(addr) {
            c.aborted.get_or_insert(reason);
        }
    }

    /// Ferme la liaison après un court délai, le temps que les derniers messages partent.
    pub fn close(&mut self, addr: &SocketAddr, delay: Duration) {
        if let Some(c) = self.conns.get_mut(addr) {
            if c.close_at.is_none() {
                c.close_at = Some(Instant::now() + delay);
            }
        }
    }

    /// Envoie ce qui attend, sans bloquer : le reste partira au passage suivant.
    pub fn flush(&mut self) {
        for c in self.conns.values_mut() {
            while !c.outbuf.is_empty() {
                match c.stream.write(&c.outbuf) {
                    Ok(0) => break,
                    Ok(n) => {
                        c.outbuf.drain(..n);
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
    }

    /// Ferme toutes les liaisons.
    pub fn shutdown(&mut self) {
        for (_, mut c) in self.conns.drain() {
            let _ = c.stream.set_nonblocking(false);
            let _ = c.stream.set_write_timeout(Some(Duration::from_millis(200)));
            let _ = c.stream.write_all(&c.outbuf);
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Côté joueur du transport de test (tests de bout en bout, outils locaux).
pub struct LocalClient {
    stream: TcpStream,
    pending: Vec<u8>,
}

impl LocalClient {
    pub fn connect(server: SocketAddr) -> std::io::Result<LocalClient> {
        let stream = TcpStream::connect(server)?;
        let _ = stream.set_nodelay(true);
        Ok(LocalClient { stream, pending: Vec::new() })
    }

    /// Envoie un message (un lot avec son octet 0xfe, ou la demande d'état 0x01).
    pub fn send(&mut self, message: &[u8]) -> std::io::Result<()> {
        let mut out = Vec::with_capacity(message.len() + 4);
        out.extend_from_slice(&(message.len() as u32).to_le_bytes());
        out.extend_from_slice(message);
        self.stream.write_all(&out)
    }

    /// Octets bruts, sans mise en forme (pour éprouver le serveur).
    pub fn send_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(bytes)
    }

    /// Prochain message, en attendant au plus `wait`. `Ok(None)` : rien n'est arrivé à temps ;
    /// une erreur `UnexpectedEof` : le serveur a fermé la liaison.
    pub fn recv(&mut self, wait: Duration) -> std::io::Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + wait;
        let mut chunk = [0u8; 16 * 1024];
        loop {
            if self.pending.len() >= 4 {
                let len = u32::from_le_bytes([self.pending[0], self.pending[1], self.pending[2], self.pending[3]]) as usize;
                if self.pending.len() >= 4 + len {
                    let message = self.pending[4..4 + len].to_vec();
                    self.pending.drain(..4 + len);
                    return Ok(Some(message));
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            self.stream.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.pending.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_cross_the_test_transport_both_ways() {
        let mut server = LocalServer::bind_local(4).unwrap();
        server.status = "MCPE;essai".into();
        let addr = server.local_addr().unwrap();
        let mut client = LocalClient::connect(addr).unwrap();
        let mut events = Vec::new();
        let mut peer = None;
        // Un gros message arrive entier, puis la demande d'état reçoit sa réponse
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        client.send(&big).unwrap();
        client.send(&[0x01]).unwrap();
        let mut got = None;
        for _ in 0..200 {
            server.poll(&mut events);
            server.flush();
            for e in events.drain(..) {
                match e {
                    NetEvent::Connected(a) => peer = Some(a),
                    NetEvent::Packet(_, m) => got = Some(m),
                    NetEvent::Disconnected(..) => panic!("liaison perdue"),
                }
            }
            if got.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got.as_deref(), Some(&big[..]));
        assert_eq!(client.recv(Duration::from_secs(2)).unwrap().unwrap(), b"\x1cMCPE;essai".to_vec());
        // Du serveur vers le joueur
        server.send(&peer.unwrap(), &[0xfe, 1, 2, 3]);
        server.flush();
        assert_eq!(client.recv(Duration::from_secs(2)).unwrap().unwrap(), vec![0xfe, 1, 2, 3]);
        // Une longueur absurde ferme la liaison, avec sa raison
        client.send_raw(&[0xff, 0xff, 0xff, 0x7f, 0]).unwrap();
        let mut reason = None;
        for _ in 0..200 {
            server.poll(&mut events);
            for e in events.drain(..) {
                if let NetEvent::Disconnected(_, r) = e {
                    reason = r;
                }
            }
            if reason.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(reason, Some("paquet trop gros"));
        assert!(server.is_empty());
    }
}
