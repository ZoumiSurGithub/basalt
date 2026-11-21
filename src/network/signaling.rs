//! Signalisation HTTP de NetherNet.
//!
//! Depuis Bedrock 1.26.60, le jeu ne cherche plus un serveur par un ping UDP :
//! il ouvre une connexion TCP sur le port du serveur et envoie des requêtes HTTP.
//!
//!   GET  /v1/join          -> informations du serveur en JSON (nom, joueurs…) ;
//!                             c'est ce qui affiche le serveur dans la liste du jeu.
//!   POST /v1/join/{id}     -> offre SDP du joueur ; la réponse SDP ouvre la
//!                             liaison WebRTC qui transporte ensuite le jeu.
//!
//! Ce module est un petit serveur HTTP non bloquant, sans dépendance, pensé pour
//! du trafic hostile : nombre de connexions, taille des requêtes et durées bornés.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

const MAX_CONNECTIONS: usize = 128;
const MAX_HEADER: usize = 8 * 1024;
const MAX_BODY: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Requête HTTP complète, prête à être traitée par le serveur.
pub struct Request {
    pub addr: SocketAddr,
    pub method: String,
    /// Chemin sans la partie « ?… »
    pub path: String,
    pub query: String,
    pub body: Vec<u8>,
    /// Adresse du serveur sur laquelle le joueur s'est connecté
    pub local_ip: Option<std::net::IpAddr>,
    stream: TcpStream,
}

impl Request {
    /// Valeur d'un paramètre de l'adresse (« ?protocol=2225 »).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.split('&').find_map(|kv| {
            let mut it = kv.splitn(2, '=');
            if it.next() == Some(name) {
                it.next()
            } else {
                None
            }
        })
    }

    /// Envoie la réponse et ferme la connexion.
    pub fn respond(mut self, status: u16, content_type: &str, body: &[u8]) {
        let reason = match status {
            200 => "OK",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            413 => "Payload Too Large",
            503 => "Service Unavailable",
            _ => "Error",
        };
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
            status,
            reason,
            content_type,
            body.len()
        );
        // Réponses courtes : une écriture bloquante bornée dans le temps suffit.
        let _ = self.stream.set_nonblocking(false);
        let _ = self.stream.set_write_timeout(Some(Duration::from_millis(500)));
        let _ = self.stream.write_all(head.as_bytes());
        let _ = self.stream.write_all(body);
        let _ = self.stream.flush();
    }
}

struct Conn {
    stream: TcpStream,
    addr: SocketAddr,
    since: Instant,
    buf: Vec<u8>,
}

pub struct Signaling {
    listeners: Vec<TcpListener>,
    conns: Vec<Conn>,
    pub debug: bool,
}

/// Échappe un texte pour l'insérer dans une chaîne JSON.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Valeur de l'attribut `a=identity` d'une description SDP.
pub fn sdp_identity(sdp: &str) -> Option<&str> {
    sdp.lines().find_map(|l| l.trim_end().strip_prefix("a=identity:"))
}

/// Empreintes DTLS d'une description SDP : (algorithme, empreinte), dans l'ordre du texte.
pub fn sdp_fingerprints(sdp: &str) -> Vec<(String, String)> {
    sdp.lines()
        .filter_map(|l| l.trim_end().strip_prefix("a=fingerprint:"))
        .filter_map(|v| {
            let mut it = v.splitn(2, ' ');
            match (it.next(), it.next()) {
                (Some(a), Some(d)) if !a.is_empty() && !d.is_empty() => Some((a.to_string(), d.trim().to_string())),
                _ => None,
            }
        })
        .collect()
}

/// La même description sans la ligne d'identité (la pile WebRTC n'en a pas besoin).
pub fn sdp_without_identity(sdp: &str) -> String {
    sdp.split_inclusive('\n').filter(|l| !l.starts_with("a=identity:")).collect()
}

/// Insère l'identité du serveur au niveau « session », juste avant la première section média.
pub fn sdp_insert_identity(sdp: &str, identity: &str) -> String {
    let line = format!("a=identity:{}\r\n", identity);
    match sdp.find("\nm=") {
        Some(i) => format!("{}{}{}", &sdp[..i + 1], line, &sdp[i + 1..]),
        None => format!("{}{}", sdp, line),
    }
}

/// Position d'une suite d'octets dans une autre.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

enum Parsed {
    /// Il manque encore des octets.
    Incomplete,
    /// Requête invalide : code HTTP à renvoyer (0 = fermer sans répondre).
    Invalid(u16),
    Complete { method: String, path: String, query: String, body: Vec<u8> },
}

/// Lit une requête HTTP (signalisation NetherNet) : incomplète, invalide ou complète.
fn parse(buf: &[u8]) -> Parsed {
    // Une tentative HTTPS commence par un enregistrement TLS (0x16) : le serveur ne
    // parle pas TLS, on ferme et le jeu réessaie aussitôt en HTTP simple.
    if buf.first() == Some(&0x16) {
        return Parsed::Invalid(0);
    }
    let end = match find(buf, b"\r\n\r\n") {
        Some(i) => i,
        None => {
            return if buf.len() > MAX_HEADER { Parsed::Invalid(413) } else { Parsed::Incomplete };
        }
    };
    let head = match std::str::from_utf8(&buf[..end]) {
        Ok(h) => h,
        Err(_) => return Parsed::Invalid(400),
    };
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = match (first.next(), first.next()) {
        (Some(m), Some(t)) if !m.is_empty() && t.starts_with('/') => (m, t),
        _ => return Parsed::Invalid(400),
    };
    let mut length = 0usize;
    for line in lines {
        let mut kv = line.splitn(2, ':');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            if k.eq_ignore_ascii_case("content-length") {
                length = match v.trim().parse() {
                    Ok(n) => n,
                    Err(_) => return Parsed::Invalid(400),
                };
            } else if k.eq_ignore_ascii_case("transfer-encoding") {
                return Parsed::Invalid(400); // corps découpé non pris en charge
            }
        }
    }
    if length > MAX_BODY {
        return Parsed::Invalid(413);
    }
    let body = &buf[end + 4..];
    if body.len() < length {
        return Parsed::Incomplete;
    }
    let (path, query) = match target.find('?') {
        Some(i) => (&target[..i], &target[i + 1..]),
        None => (target, ""),
    };
    Parsed::Complete {
        method: method.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        body: body[..length].to_vec(),
    }
}

impl Signaling {
    /// Écoute en TCP sur le port du serveur, en IPv4 et si possible en IPv6.
    pub fn bind(ip: &str, port: u16) -> std::io::Result<Signaling> {
        let open = |addr: String| -> std::io::Result<TcpListener> {
            let l = TcpListener::bind(addr)?;
            l.set_nonblocking(true)?;
            Ok(l)
        };
        let mut listeners = vec![open(format!("{}:{}", ip, port))?];
        if ip == "0.0.0.0" {
            if let Ok(l) = open(format!("[::]:{}", port)) {
                listeners.push(l);
            }
        }
        Ok(Signaling { listeners, conns: Vec::new(), debug: false })
    }

    /// Accepte les connexions, lit ce qui est arrivé et renvoie les requêtes complètes.
    pub fn poll(&mut self) -> Vec<Request> {
        for l in &self.listeners {
            for _ in 0..16 {
                match l.accept() {
                    Ok((stream, addr)) => {
                        if self.conns.len() >= MAX_CONNECTIONS || stream.set_nonblocking(true).is_err() {
                            continue; // saturé : la connexion est fermée aussitôt
                        }
                        self.conns.push(Conn { stream, addr, since: Instant::now(), buf: Vec::new() });
                    }
                    Err(_) => break,
                }
            }
        }
        let now = Instant::now();
        let mut ready = Vec::new();
        let mut i = 0;
        while i < self.conns.len() {
            let mut chunk = [0u8; 4096];
            let mut closed = false;
            loop {
                let c = &mut self.conns[i];
                match c.stream.read(&mut chunk) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        c.buf.extend_from_slice(&chunk[..n]);
                        if c.buf.len() > MAX_HEADER + MAX_BODY {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let parsed = parse(&self.conns[i].buf);
            let expired = now.duration_since(self.conns[i].since) > REQUEST_TIMEOUT;
            match parsed {
                Parsed::Incomplete if !closed && !expired => i += 1,
                Parsed::Incomplete => {
                    self.conns.remove(i);
                }
                Parsed::Invalid(status) => {
                    let c = self.conns.remove(i);
                    if status != 0 {
                        let req = Request {
                            addr: c.addr,
                            local_ip: None,
                            method: String::new(),
                            path: String::new(),
                            query: String::new(),
                            body: Vec::new(),
                            stream: c.stream,
                        };
                        req.respond(status, "text/plain", b"");
                    } else if self.debug {
                        info!("[signalisation] {} : tentative HTTPS ignorée (le jeu réessaie en HTTP)", c.addr);
                    }
                }
                Parsed::Complete { method, path, query, body } => {
                    let c = self.conns.remove(i);
                    let local_ip = c.stream.local_addr().ok().map(|a| a.ip());
                    ready.push(Request { addr: c.addr, method, path, query, body, local_ip, stream: c.stream });
                }
            }
        }
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_and_post() {
        match parse(b"GET /v1/join?version=1.26.65&protocol=2225 HTTP/1.1\r\nHost: x\r\n\r\n") {
            Parsed::Complete { method, path, query, body } => {
                assert_eq!((method.as_str(), path.as_str()), ("GET", "/v1/join"));
                assert_eq!(query, "version=1.26.65&protocol=2225");
                assert!(body.is_empty());
            }
            _ => panic!("requête complète attendue"),
        }
        let post = b"POST /v1/join/42 HTTP/1.1\r\nContent-Type: application/sdp\r\nContent-Length: 5\r\n\r\nv=0\r\n";
        match parse(post) {
            Parsed::Complete { method, path, body, .. } => {
                assert_eq!((method.as_str(), path.as_str(), body.as_slice()), ("POST", "/v1/join/42", &b"v=0\r\n"[..]));
            }
            _ => panic!("requête complète attendue"),
        }
        assert!(matches!(parse(&post[..post.len() - 2]), Parsed::Incomplete));
    }

    #[test]
    fn rejects_abuse() {
        assert!(matches!(parse(&[0x16, 0x03, 0x01, 0x00]), Parsed::Invalid(0)));
        assert!(matches!(parse(b"GARBAGE\r\n\r\n"), Parsed::Invalid(400)));
        assert!(matches!(parse(b"POST /v1/join/1 HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n"), Parsed::Invalid(413)));
        assert!(matches!(parse(b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"), Parsed::Invalid(400)));
        assert!(matches!(parse(&vec![b'a'; MAX_HEADER + 1]), Parsed::Invalid(413)));
        assert!(matches!(parse(b"GET /v1/join HTTP/1.1\r\n"), Parsed::Incomplete));
    }

    #[test]
    fn sdp_helpers() {
        let offer = "v=0\r\ns=-\r\na=identity:QUJD\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=fingerprint:sha-256 AA:BB\r\na=mid:0\r\n";
        assert_eq!(sdp_identity(offer), Some("QUJD"));
        assert_eq!(sdp_fingerprints(offer), vec![("sha-256".to_string(), "AA:BB".to_string())]);
        let clean = sdp_without_identity(offer);
        assert!(!clean.contains("identity") && clean.contains("a=mid:0\r\n") && clean.starts_with("v=0\r\ns=-\r\nm="));
        let signed = sdp_insert_identity(&clean, "WFla");
        assert_eq!(signed, offer.replace("QUJD", "WFla"));
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }
}
