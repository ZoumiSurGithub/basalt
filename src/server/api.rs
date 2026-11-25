//! Accès à distance : une petite API HTTP pour une application de gestion.
//!
//! Éteinte par défaut (`api-port=0`). Allumée, elle répond en JSON à qui présente un jeton
//! (`Authorization: Bearer <jeton>`). Il y en a deux, pour séparer les droits :
//! - `api-token.txt` : **lecture** (`GET` : état, joueurs, objets, blocs) ;
//! - `api-admin-token.txt` : lecture et **actions** (`POST` : désactiver un objet ou un
//!   bloc, recharger les plugins, lancer une commande de console).
//!
//! Elle n'écoute que sur la machine elle-même (`api-bind=127.0.0.1`) tant qu'on ne lui dit
//! pas autre chose : les échanges ne sont **pas chiffrés**. Pour y accéder d'ailleurs, il
//! faut la placer derrière un tunnel SSH ou un serveur HTTPS (nginx, Caddy).
//!
//! Un fil à part reçoit les connexions ; chaque question est posée au serveur par un canal
//! et sa réponse revient au tick suivant : l'API ne touche jamais au monde elle-même.

use super::*;
use std::io::{Read, Write};
use crate::world::World;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicUsize;

/// Une question posée au serveur : ce qui est demandé, et où rendre la réponse (code HTTP,
/// corps, type du corps).
pub(super) struct ApiRequest {
    /// `GET` (lecture) ou `POST` (action, déjà autorisée par le jeton d'administration)
    pub method: String,
    pub path: String,
    /// Paramètres de l'adresse (ce qui suit « ? »), tels quels
    pub query: String,
    /// Corps de la requête (JSON), vide pour une lecture
    pub body: String,
    /// Le jeton présenté est celui d'administration
    pub admin: bool,
    pub reply: Sender<(u16, Vec<u8>, &'static str)>,
}

/// Connexions traitées à la fois ; les suivantes sont refusées
const MAX_CONNECTIONS: usize = 16;
/// Taille maximale d'une requête (ligne et en-têtes)
const MAX_REQUEST: usize = 8 * 1024;
/// Taille maximale du corps d'une action
const MAX_BODY: usize = 16 * 1024;
const JSON: &str = "application/json; charset=utf-8";
const PNG: &str = "image/png";
const BIN: &str = "application/octet-stream";

/// Le tableau de bord : trois fichiers servis tels quels, sans jeton (ils ne contiennent
/// aucun secret ; c'est la page qui demande le jeton, puis interroge l'API avec).
const PANEL: [(&str, &str, &str); 5] = [
    ("/", "text/html; charset=utf-8", include_str!("panel/index.html")),
    ("/panel.css", "text/css; charset=utf-8", include_str!("panel/panel.css")),
    ("/panel.js", "text/javascript; charset=utf-8", include_str!("panel/panel.js")),
    ("/logo.svg", "image/svg+xml", include_str!("../../docs/images/logo.svg")),
    ("/relief.js", "text/javascript; charset=utf-8", include_str!("panel/relief.js")),
];

/// Lit le jeton d'accès, ou en crée un (32 octets au hasard, en hexadécimal) à la première fois.
pub(super) fn load_token(root: &std::path::Path, file: &str) -> std::io::Result<String> {
    let path = root.join(file);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let token = text.trim().to_string();
        if token.len() >= 32 && token.bytes().all(|b| b.is_ascii_graphic()) {
            return Ok(token);
        }
    }
    let mut bytes = [0u8; 32];
    if !crate::security::crypto::os_random(&mut bytes) {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "pas de source de hasard pour créer le jeton"));
    }
    let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    std::fs::write(&path, format!("{}\n", token))?;
    // (lisible par son seul propriétaire, là où le système le permet)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}

/// Comparaison en temps constant : sa durée ne dit rien de l'endroit où deux jetons diffèrent.
pub(super) fn same_token(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (*a.get(i).unwrap_or(&0) ^ *b.get(i).unwrap_or(&0)) as usize;
    }
    diff == 0
}

/// Ce que demande une requête HTTP : (méthode, adresse sans ses paramètres, jeton présenté,
/// taille annoncée du corps, paramètres).
pub(super) fn parse_request(head: &str) -> Option<(String, String, Option<String>, usize, String)> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return None;
    }
    let path = target.split(['?', '#']).next().unwrap_or("/").trim_end_matches('/');
    let headers: Vec<(&str, &str)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim(), v.trim())).collect();
    let token = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("authorization")).and_then(|(_, v)| (v.len() > 7 && v.is_char_boundary(7) && v[..7].eq_ignore_ascii_case("bearer ")).then(|| v[7..].trim().to_string()));
    let length = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    let query = target.split('#').next().unwrap_or("").split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();
    Some((method.to_string(), if path.is_empty() { "/".to_string() } else { path.to_string() }, token, length, query))
}

/// Valeur d'un paramètre d'adresse (`q=zou%20mi&limit=20`), décodée.
pub(super) fn param(query: &str, key: &str) -> Option<String> {
    let raw = query.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == key)?.1;
    let bytes = raw.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() => {
                out.push((hex(bytes[i + 1]).unwrap() * 16 + hex(bytes[i + 2]).unwrap()) as u8);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Les deux jetons, partagés avec le fil de l'API : la console peut les renouveler à chaud.
pub(super) type Tokens = Arc<std::sync::RwLock<(String, String)>>;

/// Essais de jeton ratés avant qu'une adresse soit mise à l'écart, et pour combien de temps
const MAX_FAILURES: u32 = 8;
const LOCKOUT: Duration = Duration::from_secs(600);

/// Garde-fou contre les essais de jetons en série : qui s'est trompé, combien de fois.
#[derive(Default)]
pub(super) struct Guard {
    /// adresse → (essais ratés, premier essai, écartée jusqu'à)
    seen: HashMap<std::net::IpAddr, (u32, Option<Instant>, Option<Instant>)>,
}

impl Guard {
    /// Cette adresse est-elle écartée en ce moment ?
    pub(super) fn locked(&mut self, ip: std::net::IpAddr, now: Instant) -> bool {
        match self.seen.get(&ip) {
            Some((_, _, Some(until))) if *until > now => true,
            Some((_, _, Some(_))) => {
                self.seen.remove(&ip);
                false
            }
            _ => false,
        }
    }

    /// Note un jeton refusé. Vrai si l'adresse vient d'être écartée.
    pub(super) fn failed(&mut self, ip: std::net::IpAddr, now: Instant) -> bool {
        // (la table ne grossit pas sans fin sous une pluie d'adresses)
        if self.seen.len() > 10_000 {
            self.seen.retain(|_, v| v.2.map(|until| until > now).unwrap_or(false));
        }
        let entry = self.seen.entry(ip).or_insert((0, None, None));
        // Les essais ratés s'oublient au bout de dix minutes
        if entry.1.map(|first| now.duration_since(first) > LOCKOUT).unwrap_or(true) {
            *entry = (0, Some(now), None);
        }
        entry.0 += 1;
        if entry.0 >= MAX_FAILURES {
            entry.2 = Some(now + LOCKOUT);
            return true;
        }
        false
    }

    /// Un bon jeton efface l'ardoise de cette adresse.
    pub(super) fn passed(&mut self, ip: std::net::IpAddr) {
        self.seen.remove(&ip);
    }
}

/// Valeur d'un en-tête de la requête, sans égard à la casse de son nom.
pub(super) fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.split("\r\n").skip(1).filter_map(|l| l.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

/// L'adresse de celui qui parle vraiment. Derrière un tunnel ou un serveur HTTPS installé
/// sur cette machine, toutes les connexions viennent de la machine elle-même : c'est alors
/// l'en-tête posé par ce relais qui dit qui est au bout. Cet en-tête n'est cru que venant
/// de la machine elle-même, sans quoi n'importe qui pourrait se faire passer pour un autre.
pub(super) fn client_ip(peer: std::net::IpAddr, head: &str) -> std::net::IpAddr {
    if !peer.is_loopback() {
        return peer;
    }
    header(head, "cf-connecting-ip")
        .or_else(|| header(head, "x-forwarded-for").and_then(|list| list.rsplit(',').next()))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(peer)
}

/// Ouvre l'écoute et lance le fil de l'API. Renvoie l'adresse réellement écoutée.
pub(super) fn start(bind: &str, port: u16, tokens: Tokens, questions: Sender<ApiRequest>) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind((bind, port))?;
    let addr = listener.local_addr()?;
    let busy = Arc::new(AtomicUsize::new(0));
    let guard = Arc::new(std::sync::Mutex::new(Guard::default()));
    std::thread::Builder::new().name("api".into()).spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if busy.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                let _ = respond(stream, 503, "{\"error\":\"trop de connexions\"}");
                continue;
            }
            busy.fetch_add(1, Ordering::Relaxed);
            let (busy, tokens, questions, guard) = (busy.clone(), tokens.clone(), questions.clone(), guard.clone());
            let spawned = std::thread::Builder::new().name("api-client".into()).spawn(move || {
                serve(stream, &tokens, &questions, &guard);
                busy.fetch_sub(1, Ordering::Relaxed);
            });
            // (si le fil n'a pas pu démarrer, la connexion est simplement fermée)
            let _ = spawned;
        }
    })?;
    Ok(addr)
}

/// Sert une connexion : une requête, une réponse, puis elle est fermée.
fn serve(mut stream: TcpStream, tokens: &Tokens, questions: &Sender<ApiRequest>, guard: &std::sync::Mutex<Guard>) {
    let peer = stream.peer_addr().map(|a| a.ip()).unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    let end = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut buf) {
            Ok(n) if n > 0 && data.len() + n <= MAX_REQUEST => data.extend_from_slice(&buf[..n]),
            _ => return,
        }
    };
    let text = String::from_utf8_lossy(&data[..end]).into_owned();
    let Some((method, path, presented, length, query)) = parse_request(&text) else {
        let _ = respond(stream, 400, "{\"error\":\"requête illisible\"}");
        return;
    };
    if method == "GET" {
        if let Some((_, kind, page)) = PANEL.iter().find(|(p, _, _)| *p == path) {
            let _ = respond_as(stream, 200, kind, page);
            return;
        }
        // (les navigateurs la demandent d'eux-mêmes : c'est le logo)
        if path == "/favicon.ico" {
            let _ = respond_as(stream, 200, PANEL[3].1, PANEL[3].2);
            return;
        }
    }
    // Une adresse qui a présenté trop de mauvais jetons est écartée dix minutes
    let ip = client_ip(peer, &text);
    let now = Instant::now();
    if guard.lock().map(|mut g| g.locked(ip, now)).unwrap_or(false) {
        let _ = respond(stream, 429, "{\"error\":\"trop de jetons refusés depuis cette adresse : réessayez dans dix minutes\"}");
        return;
    }
    // Les deux comparaisons sont toujours faites : la durée ne dit pas quel jeton a été essayé
    let presented = presented.unwrap_or_default();
    let (is_reader, is_admin) = match tokens.read() {
        Ok(t) => (same_token(presented.as_bytes(), t.0.as_bytes()), same_token(presented.as_bytes(), t.1.as_bytes())),
        Err(_) => (false, false),
    };
    if !is_reader && !is_admin {
        // (une requête sans jeton est une page qui se charge, pas un essai)
        if !presented.is_empty() {
            let locked = guard.lock().map(|mut g| g.failed(ip, now)).unwrap_or(false);
            if locked {
                warn!("API : {} mauvais jetons depuis {} — adresse écartée dix minutes", MAX_FAILURES, ip);
            }
        }
        // Un mauvais jeton attend une demi-seconde : les essais en rafale ne mènent nulle part
        std::thread::sleep(Duration::from_millis(500));
        let _ = respond(stream, 401, "{\"error\":\"jeton manquant ou invalide (Authorization: Bearer <jeton>)\"}");
        return;
    }
    if let Ok(mut g) = guard.lock() {
        g.passed(ip);
    }
    if method != "GET" && method != "POST" {
        let _ = respond(stream, 405, "{\"error\":\"méthodes acceptées : GET (lecture) et POST (action)\"}");
        return;
    }
    if method == "POST" && !is_admin {
        let _ = respond(stream, 403, "{\"error\":\"ce jeton ne donne que la lecture ; les actions demandent celui de api-admin-token.txt\"}");
        return;
    }
    if length > MAX_BODY {
        let _ = respond(stream, 400, "{\"error\":\"corps trop long\"}");
        return;
    }
    let mut body = data[end..].to_vec();
    while body.len() < length {
        match stream.read(&mut buf) {
            Ok(n) if n > 0 => body.extend_from_slice(&buf[..n]),
            _ => return,
        }
    }
    body.truncate(length);
    let Ok(body) = String::from_utf8(body) else {
        let _ = respond(stream, 400, "{\"error\":\"corps illisible (UTF-8 attendu)\"}");
        return;
    };
    let (reply, answer) = mpsc::channel();
    if questions.send(ApiRequest { method, path, query, body, admin: is_admin, reply }).is_err() {
        let _ = respond(stream, 503, "{\"error\":\"serveur à l'arrêt\"}");
        return;
    }
    match answer.recv_timeout(Duration::from_secs(3)) {
        Ok((code, body, kind)) => {
            let _ = respond_bytes(stream, code, kind, &body, kind == PNG);
        }
        Err(_) => {
            let _ = respond(stream, 503, "{\"error\":\"le serveur n'a pas répondu à temps\"}");
        }
    }
}

fn respond(stream: TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    respond_as(stream, code, "application/json; charset=utf-8", body)
}

fn respond_as(stream: TcpStream, code: u16, kind: &str, body: &str) -> std::io::Result<()> {
    respond_bytes(stream, code, kind, body.as_bytes(), false)
}

/// `keep` : le navigateur peut garder la réponse une heure (les images ; jamais le reste).
fn respond_bytes(mut stream: TcpStream, code: u16, kind: &str, body: &[u8], keep: bool) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        429 => "Too Many Requests",
        _ => "Service Unavailable",
    };
    // La page ne charge rien d'ailleurs que d'ici, ne s'affiche dans aucun cadre, et ne
    // laisse pas fuir son adresse
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
        code,
        reason,
        kind,
        body.len(),
        if keep && code == 200 { "private, max-age=3600" } else { "no-store" }
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

impl Server {
    /// Répond aux questions arrivées par l'API depuis le dernier tick.
    pub(super) fn api_tick(&mut self) {
        let Some(rx) = self.api_rx.as_ref() else { return };
        let waiting: Vec<ApiRequest> = rx.try_iter().take(32).collect();
        for q in waiting {
            if q.method == "GET" && q.path == "/v1/texture" {
                let _ = q.reply.send(match self.api_texture(&q.query) {
                    Some(png) => (200, png.to_vec(), PNG),
                    None => (404, b"{\"error\":\"pas d'image pour ce nom\"}".to_vec(), JSON),
                });
                continue;
            }
            if q.method == "GET" && q.path == "/v1/map" {
                // La tuile est dessinée hors du fil du serveur, sur une vue du monde
                let coord = |key: &str| param(&q.query, key).and_then(|v| v.parse::<i32>().ok()).filter(|c| c.abs() <= 1_875_000);
                match (coord("cx"), coord("cz"), self.data.clone()) {
                    (Some(cx), Some(cz), Some(data)) => {
                        let view = self.world.view();
                        let reply = q.reply.clone();
                        let colours = self.map_colours();
                        let spawned = std::thread::Builder::new().name("carte".into()).spawn(move || {
                            let _ = reply.send((200, map::tile(&view, &data, &map::ready(&colours).colours, cx & !(map::TILE - 1), cz & !(map::TILE - 1)), BIN));
                        });
                        if spawned.is_err() {
                            let _ = q.reply.send((503, b"{\"error\":\"serveur occup\xc3\xa9\"}".to_vec(), JSON));
                        }
                    }
                    _ => {
                        let _ = q.reply.send((400, b"{\"error\":\"cx et cz attendus (chunks)\"}".to_vec(), JSON));
                    }
                }
                continue;
            }
            if q.method == "GET" && q.path == "/v1/world/chunks" {
                // Les chunks de la vue en relief : « cx.cz.version » séparés par des virgules.
                // Le fil du serveur ne fait que relever les versions ; un chunk que le
                // demandeur a déjà dans cette version n'est pas renvoyé.
                let wanted: Vec<(i32, i32, i64)> = param(&q.query, "list").unwrap_or_default().split(',').take(128).filter_map(|item| {
                    let mut parts = item.split('.');
                    let (cx, cz, v) = (parts.next()?.parse::<i32>().ok()?, parts.next()?.parse::<i32>().ok()?, parts.next()?.parse::<i64>().ok()?);
                    (cx.abs() <= 1_875_000 && cz.abs() <= 1_875_000).then_some((cx, cz, v))
                }).collect();
                let Some(data) = self.data.clone() else { continue };
                let todo: Vec<(i32, i32, u32, bool)> = wanted.iter().map(|&(cx, cz, v)| {
                    let version = self.world.chunk_version(cx, cz) as u32;
                    (cx, cz, version, v == version as i64)
                }).collect();
                let (view, air, reply) = (self.world.view(), self.world.air, q.reply.clone());
                let colours = self.map_colours();
                let spawned = std::thread::Builder::new().name("relief".into()).spawn(move || {
                    let mut out = Vec::new();
                    let real = map::ready(&colours);
                    for (cx, cz, version, same) in todo {
                        // (état : 0 pas en mémoire, 1 inchangé, 2 suivi de ses octets)
                        let body = if same { None } else { map::chunk3d(&view, &data, &real, air, cx, cz) };
                        let state: u8 = if same { 1 } else if body.is_some() { 2 } else { 0 };
                        out.extend_from_slice(&cx.to_le_bytes());
                        out.extend_from_slice(&cz.to_le_bytes());
                        out.extend_from_slice(&version.to_le_bytes());
                        out.push(state);
                        let body = body.unwrap_or_default();
                        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                        out.extend_from_slice(&body);
                    }
                    let _ = reply.send((200, out, BIN));
                });
                if spawned.is_err() {
                    let _ = q.reply.send((503, b"{\"error\":\"serveur occup\xc3\xa9\"}".to_vec(), JSON));
                }
                continue;
            }
            if q.method == "GET" && q.path == "/v1/world/atlas" {
                // La planche des textures des blocs, en PNG : préparée une fois, hors du fil du serveur
                let (colours, reply) = (self.map_colours(), q.reply.clone());
                let _ = std::thread::Builder::new().name("planche".into()).spawn(move || {
                    let look = map::ready(&colours);
                    let _ = reply.send(if look.atlas.is_empty() { (404, b"{\"error\":\"aucune texture de bloc dans les packs\"}".to_vec(), JSON) } else { (200, look.atlas.clone(), PNG) });
                });
                continue;
            }
            if q.method == "GET" && q.path == "/v1/world/skin" {
                let _ = q.reply.send(match self.textures().steve() {
                    Some(png) => (200, png.to_vec(), PNG),
                    None => (404, b"{\"error\":\"pas de pack du jeu\"}".to_vec(), JSON),
                });
                continue;
            }
            let (code, body) = if q.method == "POST" {
                self.api_action(&q.path, &q.body)
            } else if q.path == "/v1/me" {
                (200, format!("{{\"admin\":{}}}", q.admin))
            } else {
                self.api_answer(&q.path, &q.query)
            };
            let _ = q.reply.send((code, body.into_bytes(), JSON));
        }
    }

    /// Les images, préparées à la première demande.
    fn textures(&mut self) -> &mut textures::Textures {
        if self.textures.is_none() {
            self.textures = Some(textures::Textures::load(&self.packs, &self.cfg.root));
        }
        self.textures.as_mut().unwrap()
    }

    /// Les couleurs des blocs pour la carte. Au premier appel, un fil à part lit les textures
    /// des packs et en tire la couleur de chaque bloc ; rien n'est lu sur le fil du serveur.
    fn map_colours(&mut self) -> map::Colours {
        if !self.map_colours_started {
            self.map_colours_started = true;
            let (shared, packs, root) = (self.map_colours.clone(), self.packs.clone(), self.cfg.root.clone());
            let names = self.data.as_ref().map(|d| d.block_name_list()).unwrap_or_default();
            let work = move || {
                let found = textures::Textures::load(&packs, &root).look(&names);
                if !found.colours.is_empty() {
                    info!("Carte : la couleur de {} blocs et les textures de {} viennent des packs", found.colours.len(), found.faces.len());
                }
                if let Ok(mut slot) = shared.lock() {
                    *slot = Some(Arc::new(found));
                }
            };
            if std::thread::Builder::new().name("couleurs".into()).spawn(work).is_err() {
                if let Ok(mut slot) = self.map_colours.lock() {
                    *slot = Some(Default::default());
                }
            }
        }
        self.map_colours.clone()
    }

    /// L'image d'un objet ou d'un bloc (`kind=item|block&name=…`), si un pack la fournit.
    fn api_texture(&mut self, query: &str) -> Option<Arc<Vec<u8>>> {
        let (kind, name) = (param(query, "kind")?, param(query, "name")?);
        if (kind != "item" && kind != "block") || name.is_empty() || name.len() > 128 {
            return None;
        }
        let full = if name.contains(':') { name.clone() } else { format!("minecraft:{}", name) };
        // Ce que la fiche d'un plugin a choisi pour son objet ou son bloc
        let icon = crate::item::custom::all().into_iter().find(|(d, _)| d.name == full).map(|(d, _)| d.icon).filter(|i| !i.is_empty());
        let face = crate::block::custom::find(&full).map(|b| b.texture.clone()).filter(|f| !f.is_empty());
        self.textures().image(&kind, &name, icon.as_deref(), face.as_deref())
    }

    /// Le bloc d'une position (`x`, `y`, `z`), ou le plus haut de la colonne sans `y`. Ne
    /// répond que pour du terrain en mémoire.
    fn api_block_at(&self, query: &str) -> (u16, String) {
        let coord = |key: &str| param(query, key).and_then(|v| v.parse::<i32>().ok()).filter(|c| c.abs() <= 30_000_000);
        let (Some(x), Some(z)) = (coord("x"), coord("z")) else { return (400, "{\"error\":\"x et z attendus\"}".to_string()) };
        if !self.world.is_ready(x >> 4, z >> 4) {
            return (409, "{\"error\":\"zone non chargée : un joueur doit être à proximité\"}".to_string());
        }
        let air = self.world.air;
        let y = match param(query, "y").and_then(|v| v.parse::<i32>().ok()) {
            Some(y) if World::in_bounds(y) => y,
            Some(_) => return (400, "{\"error\":\"y de -64 à 319\"}".to_string()),
            None => self.world.highest(x, z, &|s| s != air),
        };
        let state = self.world.get(x, y, z);
        let name = self.data.as_ref().and_then(|d| d.block_name(state)).unwrap_or("air").trim_start_matches("minecraft:").to_string();
        (200, Json::Obj(vec![("x".to_string(), Json::Num(x as f64)), ("y".to_string(), Json::Num(y as f64)), ("z".to_string(), Json::Num(z as f64)), ("block".to_string(), Json::Str(name))]).to_json())
    }

    /// Une action demandée par l'API (jeton d'administration déjà vérifié) : (code HTTP, JSON).
    /// Chacune est écrite au journal.
    fn api_action(&mut self, path: &str, body: &str) -> (u16, String) {
        let bad = |why: &str| (400, Json::Obj(vec![("error".to_string(), Json::Str(why.to_string()))]).to_json());
        let fields = if body.trim().is_empty() { Json::Obj(Vec::new()) } else {
            match crate::utils::json::parse(body) {
                Ok(v) => v,
                Err(_) => return bad("le corps doit être un objet JSON"),
            }
        };
        let field = |key: &str| fields.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let number = |key: &str| fields.get(key).and_then(|v| v.as_f64()).filter(|n| n.is_finite());
        // (une action peut changer ce que les listes gardées en réserve décrivent)
        self.api_cache.clear();
        match path {
            "/v1/players/action" => {
                let (Some(name), Some(action)) = (field("name"), field("action")) else { return bad("champs « name » et « action » attendus") };
                let Some(addr) = self.addr_of(&name) else {
                    return (404, Json::Obj(vec![("error".to_string(), Json::Str(format!("{} n'est pas en jeu", crate::log::clean(&name, 64))))]).to_json());
                };
                let shown = crate::log::clean(&name, 64);
                let done: Result<String, String> = match action.as_str() {
                    "teleport" => match (number("x"), number("y"), number("z")) {
                        (Some(x), Some(y), Some(z)) if x.abs() <= 30_000_000.0 && z.abs() <= 30_000_000.0 && (-64.0..=320.0).contains(&y) => {
                            self.teleport(&addr, (x as f32, y as f32 + EYE, z as f32));
                            Ok(format!("téléporté en {} {} {}", x.floor(), y.floor(), z.floor()))
                        }
                        _ => Err("x, y et z attendus (y de -64 à 320)".into()),
                    },
                    "teleport_to" => match field("target").and_then(|t| self.addr_of(&t)).and_then(|a| self.clients.get(&a)).map(|c| c.pos) {
                        Some(pos) => {
                            self.teleport(&addr, pos);
                            Ok("téléporté auprès de l'autre joueur".into())
                        }
                        None => Err("« target » : un joueur en jeu attendu".into()),
                    },
                    "gamemode" => match field("mode").and_then(|m| GameMode::parse(&m)) {
                        Some(mode) => {
                            self.set_gamemode(&addr, mode);
                            Ok("mode de jeu changé".into())
                        }
                        None => Err("« mode » attendu : survival, creative, adventure ou spectator".into()),
                    },
                    "kick" => {
                        let reason = field("reason").filter(|r| r.len() <= 200 && !r.chars().any(|c| c.is_control())).unwrap_or_else(|| "Expulsé par un administrateur".to_string());
                        self.kick(&addr, &reason);
                        Ok("expulsé".into())
                    }
                    "kill" => {
                        self.hurt(&addr, 1.0e6, Cause::Command);
                        Ok("tué".into())
                    }
                    "heal" => {
                        self.heal(&addr, 1.0e6);
                        if let Some(d) = self.clients.get_mut(&addr).and_then(|c| c.data.as_mut()) {
                            d.food = 20.0;
                        }
                        self.sync_attributes(&addr);
                        self.extinguish(&addr);
                        Ok("soigné et nourri".into())
                    }
                    "clear_effects" => {
                        let n = self.clear_effects(&addr, None);
                        Ok(format!("{} effet(s) retiré(s)", n))
                    }
                    "effect" => match (field("effect").and_then(|e| effects::effect_id(&e)), number("seconds").unwrap_or(30.0), number("level").unwrap_or(1.0)) {
                        (Some(effect), seconds, level) if (1.0..=1_000_000.0).contains(&seconds) && (1.0..=256.0).contains(&level) => {
                            self.give_effect(&addr, effect, level as i32 - 1, seconds as u64 * 20, true);
                            Ok(format!("effet {} donné", effects::effect_name(effect)))
                        }
                        _ => Err("« effect » (speed, regeneration…), « seconds » (1 et plus) et « level » (1 à 256) attendus".into()),
                    },
                    "give" => match (field("item").and_then(|i| self.item_id(&i)), number("count").unwrap_or(1.0)) {
                        (Some(item), count) if (1.0..=6400.0).contains(&count) => {
                            self.add_item(&addr, item, count as u16);
                            Ok("objet donné".into())
                        }
                        _ => Err("« item » (un objet du jeu) et « count » (1 à 6400) attendus".into()),
                    },
                    "clear_inventory" => {
                        if let Some(c) = self.clients.get_mut(&addr) {
                            for slot in c.inv.main.slots.iter_mut().chain(c.inv.armor.slots.iter_mut()).chain(c.inv.offhand.slots.iter_mut()) {
                                *slot = inventory::ItemStack::EMPTY;
                            }
                            c.inv_touched = true;
                        }
                        self.send_inventory(&addr);
                        Ok("inventaire vidé".into())
                    }
                    "message" => match field("text").filter(|t| t.len() <= 256 && !t.chars().any(|c| c.is_control())) {
                        Some(t) => {
                            self.reply(Some(addr), &format!("§e[Administration] §f{}", t));
                            Ok("message envoyé".into())
                        }
                        None => Err("« text » attendu (256 caractères au plus)".into()),
                    },
                    _ => Err("action inconnue : teleport, teleport_to, gamemode, kick, kill, heal, effect, clear_effects, give, clear_inventory, message".into()),
                };
                match done {
                    Ok(what) => {
                        info!("API : {} — {}", shown, what);
                        (200, Json::Obj(vec![("ok".to_string(), Json::Bool(true)), ("done".to_string(), Json::Str(what))]).to_json())
                    }
                    Err(why) => bad(&why),
                }
            }
            "/v1/disable" | "/v1/enable" => {
                let Some(name) = field("name") else { return bad("champ « name » attendu : {\"name\": \"tnt\"}") };
                let name = name.trim_start_matches("minecraft:").to_ascii_lowercase();
                let known = self.item_id(&name).is_some() || self.data.as_ref().map(|d| d.default_state(&name).is_some() || d.default_state(&format!("minecraft:{}", name)).is_some()).unwrap_or(false);
                if !known {
                    return (404, Json::Obj(vec![("error".to_string(), Json::Str(format!("ni objet ni bloc de ce nom : {}", crate::log::clean(&name, 64))))]).to_json());
                }
                let off = path == "/v1/disable";
                let changed = if off { self.disabled.insert(name.clone()) } else { self.disabled.remove(&name) };
                if changed {
                    self.save_disabled();
                    info!("API : {} {}", if off { "désactive" } else { "réactive" }, name);
                }
                (200, Json::Obj(vec![("ok".to_string(), Json::Bool(true)), ("name".to_string(), Json::Str(name)), ("disabled".to_string(), Json::Bool(off)), ("changed".to_string(), Json::Bool(changed))]).to_json())
            }
            "/v1/world/block" | "/v1/world/fill" => {
                let whole = |key: &str| number(key).filter(|n| n.abs() <= 30_000_000.0).map(|n| n.floor() as i32);
                let Some(name) = field("block") else { return bad("champ « block » attendu : {\"block\": \"stone\"}") };
                let name = name.trim_start_matches("minecraft:").to_ascii_lowercase();
                let Some(state) = self.data.as_ref().and_then(|d| d.default_state(&name).or_else(|| d.default_state(&format!("minecraft:{}", name)))) else {
                    return (404, Json::Obj(vec![("error".to_string(), Json::Str(format!("pas de bloc de ce nom : {}", crate::log::clean(&name, 64))))]).to_json());
                };
                let (Some(x), Some(y), Some(z)) = (whole("x"), whole("y"), whole("z")) else { return bad("x, y et z attendus") };
                let (x2, y2, z2) = if path == "/v1/world/fill" {
                    match (whole("x2"), whole("y2"), whole("z2")) {
                        (Some(a), Some(b), Some(c)) => (a, b, c),
                        _ => return bad("x2, y2 et z2 attendus (le coin opposé)"),
                    }
                } else {
                    (x, y, z)
                };
                let (lo, hi) = ((x.min(x2), y.min(y2), z.min(z2)), (x.max(x2), y.max(y2), z.max(z2)));
                if !World::in_bounds(lo.1) || !World::in_bounds(hi.1) {
                    return bad("y de -64 à 319");
                }
                let volume = (hi.0 - lo.0 + 1) as i64 * (hi.1 - lo.1 + 1) as i64 * (hi.2 - lo.2 + 1) as i64;
                if volume > 32_768 {
                    return bad("32 768 blocs au plus par demande");
                }
                // (on ne touche qu'à du terrain en mémoire : rien n'est généré sur le fil du serveur)
                for cx in (lo.0 >> 4)..=(hi.0 >> 4) {
                    for cz in (lo.2 >> 4)..=(hi.2 >> 4) {
                        if !self.world.is_ready(cx, cz) {
                            return (409, "{\"error\":\"zone non chargée : un joueur doit être à proximité\"}".to_string());
                        }
                    }
                }
                let mut list = Vec::with_capacity(volume as usize);
                for bx in lo.0..=hi.0 {
                    for by in lo.1..=hi.1 {
                        for bz in lo.2..=hi.2 {
                            if self.world.get(bx, by, bz) != state {
                                list.push((bx, by, bz, state));
                            }
                        }
                    }
                }
                let changed = list.len();
                self.set_blocks_from_plugin(list);
                info!("API : {} bloc(s) changés en {} de {} {} {} à {} {} {}", changed, name, lo.0, lo.1, lo.2, hi.0, hi.1, hi.2);
                (200, Json::Obj(vec![("ok".to_string(), Json::Bool(true)), ("block".to_string(), Json::Str(name)), ("changed".to_string(), Json::Num(changed as f64))]).to_json())
            }
            "/v1/scan" => {
                if self.start_scan() {
                    info!("API : analyse de la carte lancée");
                    (200, "{\"ok\":true,\"started\":true}".to_string())
                } else {
                    bad("une analyse est déjà en cours")
                }
            }
            "/v1/packs/encrypt" => {
                let Some(file) = field("file") else { return bad("champ « file » attendu : {\"file\": \"monpack.mcpack\"}") };
                match self.encrypt_pack_file(&file) {
                    Ok(()) => {
                        info!("API : chiffrement du pack {}", crate::log::clean(&file, 128));
                        (200, "{\"ok\":true,\"started\":true}".to_string())
                    }
                    Err(e) => bad(&e),
                }
            }
            "/v1/plugins/reload" => {
                info!("API : rechargement des plugins");
                self.command("reload");
                (200, "{\"ok\":true}".to_string())
            }
            "/v1/command" => {
                let Some(line) = field("command") else { return bad("champ « command » attendu : {\"command\": \"say Bonjour\"}") };
                if line.len() > 256 || line.chars().any(|c| c.is_control()) {
                    return bad("commande trop longue (256 caractères au plus) ou contenant des caractères de contrôle");
                }
                let line = line.trim_start_matches('/').to_string();
                info!("API : commande « {} »", crate::log::clean(&line, 256));
                self.command(&line);
                (200, "{\"ok\":true}".to_string())
            }
            _ => (404, "{\"error\":\"action inconnue ; voir /v1\"}".into()),
        }
    }

    /// La réponse à une adresse de l'API : (code HTTP, JSON).
    fn api_answer(&mut self, path: &str, query: &str) -> (u16, String) {
        // Les longues listes ne changent qu'avec les plugins et les désactivations : elles
        // sont composées une fois, puis resservies telles quelles
        if let Some(ready) = self.api_cache.get(path) {
            return (200, ready.clone());
        }
        let obj = |pairs: Vec<(&str, Json)>| Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        let num = |n: f64| Json::Num(n);
        let text = |s: &str| Json::Str(s.to_string());
        let round = |v: f32| Json::Num((v as f64 * 100.0).round() / 100.0);
        let body = match path {
            "/v1" => obj(vec![
                ("name", text("Basalt")),
                ("version", text(env!("CARGO_PKG_VERSION"))),
                ("endpoints", Json::Arr(["GET /v1/me", "GET /v1/status", "GET /v1/players", "GET /v1/items", "GET /v1/blocks", "GET /v1/scan", "POST /v1/scan", "GET /v1/map?cx=&cz=", "GET /v1/world/chunks?list=cx.cz.v,…", "GET /v1/entities?x=&z=&r=", "GET /v1/world/atlas", "GET /v1/world/skin", "GET /v1/world/block?x=&z=", "POST /v1/world/block", "POST /v1/world/fill", "GET /v1/disabled", "GET /v1/textures", "GET /v1/texture?kind=item&name=…", "GET /v1/packs", "POST /v1/packs/encrypt", "POST /v1/players/action", "POST /v1/disable", "POST /v1/enable", "POST /v1/plugins/reload", "POST /v1/command"].iter().map(|e| text(e)).collect())),
            ]),
            "/v1/status" => {
                let tps = if self.perf.last.0 <= 0.0 { 20.0 } else { self.perf.last.0.min(20.0) };
                obj(vec![
                    ("name", text("Basalt")),
                    ("version", text(env!("CARGO_PKG_VERSION"))),
                    ("uptime_seconds", num(self.started.elapsed().as_secs() as f64)),
                    ("ticks_per_second", round(tps)),
                    ("worst_tick_delay_ms", num(self.perf.last.1.as_millis() as f64)),
                    ("players_online", num(self.players().len() as f64)),
                    ("players_max", num(self.cfg.max_players as f64)),
                    ("memory_mb", memory_mb().map(|m| num(m as f64)).unwrap_or(Json::Null)),
                    ("world_time", num(self.world_time() as f64)),
                    ("world_edits", num(self.world.edit_count() as f64)),
                    ("creatures", num(self.mobs.len() as f64)),
                    ("plugins", Json::Arr(self.plugins.catalog().plugins.iter().map(|(p, _)| text(&p.name)).collect())),
                ])
            }
            "/v1/scan" => return (200, self.scan_status()),
            "/v1/world/block" => return self.api_block_at(query),
            "/v1/entities" => {
                // Joueurs et créatures autour d'un point (96 blocs par défaut, 160 au plus),
                // les plus proches d'abord, 400 au plus : de quoi dessiner leurs boîtes
                let centre = |key: &str| param(query, key).and_then(|v| v.parse::<f32>().ok()).filter(|c| c.is_finite());
                let (Some(x), Some(z)) = (centre("x"), centre("z")) else { return (400, "{\"error\":\"x et z attendus\"}".to_string()) };
                let reach = centre("r").unwrap_or(96.0).clamp(8.0, 160.0);
                let near = |p: (f32, f32, f32)| (p.0 - x).abs() <= reach && (p.2 - z).abs() <= reach;
                let mut found: Vec<(f32, Json)> = Vec::new();
                let entry = |kind: &str, name: &str, player: bool, p: (f32, f32, f32), (w, h): (f32, f32), yaw: f32| {
                    obj(vec![("kind", text(kind)), ("name", text(name)), ("player", Json::Bool(player)), ("x", round(p.0)), ("y", round(p.1)), ("z", round(p.2)), ("w", round(w)), ("h", round(h)), ("yaw", round(if yaw.is_finite() { yaw } else { 0.0 }))])
                };
                for c in self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && near(c.pos)) {
                    let Some(name) = c.name.as_deref() else { continue };
                    found.push(((c.pos.0 - x).powi(2) + (c.pos.2 - z).powi(2), entry("player", name, true, (c.pos.0, c.pos.1 - EYE, c.pos.2), (0.6, 1.8), c.yaw)));
                }
                for m in self.mobs.values().filter(|m| near(m.pos)) {
                    let label = m.label();
                    found.push(((m.pos.0 - x).powi(2) + (m.pos.2 - z).powi(2), entry(&label, &label, false, m.pos, m.size(), m.yaw)));
                }
                let total = found.len();
                found.sort_by(|a, b| a.0.total_cmp(&b.0));
                found.truncate(400);
                obj(vec![("total", Json::Num(total as f64)), ("entities", Json::Arr(found.into_iter().map(|f| f.1).collect()))])
            }
            "/v1/textures" => {
                let vanilla = self.textures().has_vanilla();
                obj(vec![("vanilla", Json::Bool(vanilla)), ("folder", text("vanilla_resource_pack"))])
            }
            "/v1/disabled" => {
                let mut names: Vec<&String> = self.disabled.iter().collect();
                names.sort_unstable();
                Json::Arr(names.into_iter().map(|n| text(n)).collect())
            }
            "/v1/packs" => {
                // Les packs envoyés aux joueurs, puis les fichiers du dossier (chiffrés ou non)
                let loaded = Json::Arr(self.packs.iter().map(|p| obj(vec![("name", text(&p.name)), ("uuid", text(&p.uuid)), ("version", text(&p.version)), ("size", num(p.data.len() as f64)), ("encrypted", Json::Bool(!p.key.is_empty()))])).collect());
                let dir = self.cfg.root.join("resource_packs");
                let mut files: Vec<(String, u64, bool)> = std::fs::read_dir(&dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_file())
                            .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.metadata().ok()?.len())))
                            .filter(|(n, _)| n.ends_with(".mcpack") || n.ends_with(".zip"))
                            .map(|(n, size)| {
                                let has_key = dir.join(format!("{}.key", n)).is_file();
                                (n, size, has_key)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                let files = Json::Arr(files.into_iter().map(|(n, size, key)| obj(vec![("file", text(&n)), ("size", num(size as f64)), ("has_key", Json::Bool(key))])).collect());
                obj(vec![("force", Json::Bool(self.force_packs)), ("loaded", loaded), ("files", files)])
            }
            "/v1/players" => {
                // Recherche par nom, puis une page de la liste (50 par défaut, 200 au plus) :
                // avec mille joueurs, on ne renvoie pas tout à chaque rafraîchissement
                let wanted = param(query, "q").unwrap_or_default().to_lowercase();
                let offset = param(query, "offset").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                let limit = param(query, "limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(50).clamp(1, 200);
                let online = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && c.name.is_some()).count();
                let mut found: Vec<&Client> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).filter(|c| c.name.as_ref().map(|n| wanted.is_empty() || n.to_lowercase().contains(&wanted)).unwrap_or(false)).collect();
                found.sort_by_cached_key(|c| c.name.as_ref().map(|n| n.to_lowercase()).unwrap_or_default());
                let total = found.len();
                let page = found
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|c| {
                        let name = c.name.as_deref().unwrap_or("");
                        let d = c.data.as_ref();
                        obj(vec![
                            ("name", text(name)),
                            ("gamemode", text(&format!("{:?}", c.mode).to_lowercase())),
                            ("x", round(c.pos.0)),
                            ("y", round(c.pos.1 - EYE)),
                            ("z", round(c.pos.2)),
                            ("health", d.map(|d| round(d.health)).unwrap_or(Json::Null)),
                            ("max_health", d.map(|d| round(d.max_health)).unwrap_or(Json::Null)),
                            ("food", d.map(|d| round(d.food)).unwrap_or(Json::Null)),
                            ("dead", Json::Bool(c.dead.is_some())),
                            ("operator", Json::Bool(self.ops.is_op(name))),
                            ("effects", Json::Arr(c.effects.iter().filter(|e| e.2 > self.tick).map(|e| text(effects::effect_name(e.0))).collect())),
                        ])
                    })
                    .collect();
                obj(vec![("online", num(online as f64)), ("total", num(total as f64)), ("offset", num(offset as f64)), ("players", Json::Arr(page))])
            }
            "/v1/items" => {
                let Some(d) = self.data.as_ref() else { return (503, "{\"error\":\"données du jeu pas encore chargées\"}".into()) };
                Json::Arr(
                    d.items
                        .iter()
                        .map(|def| {
                            let item = VanillaItems::get(&def.name);
                            obj(vec![
                                ("name", text(&def.name)),
                                ("id", num(def.id as f64)),
                                ("custom", Json::Bool(crate::item::custom::get(&def.name).is_some())),
                                ("disabled", Json::Bool(self.disabled.contains(def.name.trim_start_matches("minecraft:")))),
                                ("max_stack", num(item.max_stack_size() as f64)),
                                ("durability", num(item.max_durability() as f64)),
                                ("attack_damage", round(item.attack_damage())),
                                ("armor_points", num(item.armor_points() as f64)),
                                ("food", Json::Bool(item.food().is_some())),
                                ("block", Json::Bool(d.block_for_item(def.id).is_some())),
                            ])
                        })
                        .collect(),
                )
            }
            "/v1/blocks" => {
                let Some(d) = self.data.as_ref() else { return (503, "{\"error\":\"données du jeu pas encore chargées\"}".into()) };
                let mut names = d.block_list();
                names.sort_unstable();
                Json::Arr(
                    names
                        .into_iter()
                        .map(|(name, states, first)| {
                            let props = d.block_props(name);
                            obj(vec![
                                ("name", text(name)),
                                ("custom", Json::Bool(crate::block::custom::find(name).is_some())),
                                ("disabled", Json::Bool(self.disabled.contains(name.trim_start_matches("minecraft:")))),
                                ("states", num(states as f64)),
                                ("hardness", d.hardness(first).map(round).unwrap_or(Json::Null)),
                                ("blast_resistance", props.map(|p| round(p.blast_resistance)).unwrap_or(Json::Null)),
                                ("light", props.map(|p| num(p.light as f64)).unwrap_or(Json::Null)),
                                ("opaque", props.map(|p| Json::Bool(p.opacity >= 1.0)).unwrap_or(Json::Null)),
                            ])
                        })
                        .collect(),
                )
            }
            _ => return (404, "{\"error\":\"adresse inconnue ; voir /v1\"}".into()),
        };
        let json = body.to_json();
        if path == "/v1/items" || path == "/v1/blocks" {
            self.api_cache.insert(path.to_string(), json.clone());
        }
        (200, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_read_and_tokens_compared() {
        let head = "GET /v1/players/?x=1 HTTP/1.1\r\nHost: a\r\nauthorization:  Bearer  abc123 \r\n\r\n";
        assert_eq!(parse_request(head), Some(("GET".into(), "/v1/players".into(), Some("abc123".into()), 0, "x=1".into())));
        assert_eq!(parse_request("POST / HTTP/1.1\r\ncontent-length: 12\r\n\r\n"), Some(("POST".into(), "/".into(), None, 12, String::new())));
        // Un autre schéma d'autorisation n'est pas un jeton
        assert_eq!(parse_request("GET /v1 HTTP/1.1\r\nAuthorization: Basic abc\r\n\r\n").unwrap().2, None);
        assert_eq!(parse_request("n'importe quoi"), None);
        assert_eq!(parse_request("GET http://ailleurs/ HTTP/1.1\r\n\r\n"), None);
        assert_eq!(param("q=zou%20mi&limit=20", "q"), Some("zou mi".to_string()));
        assert_eq!((param("q=a+b", "q"), param("limit=20", "q"), param("q=%zz", "q")), (Some("a b".to_string()), None, Some("%zz".to_string())));
        // Qui parle vraiment : l'en-tête d'un relais n'est cru que venant de cette machine
        let relayed = "GET / HTTP/1.1\r\nX-Forwarded-For: 10.0.0.1, 203.0.113.9\r\n\r\n";
        let (local, far): (std::net::IpAddr, std::net::IpAddr) = ("127.0.0.1".parse().unwrap(), "198.51.100.7".parse().unwrap());
        assert_eq!(client_ip(local, relayed).to_string(), "203.0.113.9");
        assert_eq!(client_ip(far, relayed), far);
        assert_eq!(client_ip(local, "GET / HTTP/1.1\r\nCF-Connecting-IP: 203.0.113.50\r\nX-Forwarded-For: 1.2.3.4\r\n\r\n").to_string(), "203.0.113.50");
        assert_eq!(client_ip(local, "GET / HTTP/1.1\r\n\r\n"), local);
        assert!(same_token(b"secret", b"secret"));
        assert!(!same_token(b"secret", b"secreT") && !same_token(b"secret", b"secret2") && !same_token(b"", b"secret"));
    }

    #[test]
    fn repeated_bad_tokens_lock_an_address_out() {
        let mut g = Guard::default();
        let (bad, good): (std::net::IpAddr, std::net::IpAddr) = ("203.0.113.9".parse().unwrap(), "203.0.113.10".parse().unwrap());
        let start = Instant::now();
        for _ in 0..MAX_FAILURES - 1 {
            assert!(!g.failed(bad, start));
        }
        assert!(!g.locked(bad, start));
        assert!(g.failed(bad, start), "le huitième essai écarte l'adresse");
        assert!(g.locked(bad, start) && !g.locked(good, start));
        // Dix minutes plus tard, elle peut réessayer
        assert!(!g.locked(bad, start + LOCKOUT + Duration::from_secs(1)));
        // Un bon jeton efface l'ardoise ; des erreurs espacées ne s'additionnent pas
        for _ in 0..5 {
            g.failed(good, start);
        }
        g.passed(good);
        for _ in 0..MAX_FAILURES - 1 {
            assert!(!g.failed(good, start));
        }
        let later = start + LOCKOUT + Duration::from_secs(5);
        assert!(!g.failed(good, later), "les essais d'il y a plus de dix minutes sont oubliés");
    }

    #[test]
    fn a_token_is_created_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("basalt-api-jeton-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = load_token(&dir, "api-token.txt").unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(load_token(&dir, "api-token.txt").unwrap(), first, "le jeton ne change pas d'un démarrage à l'autre");
        assert_ne!(load_token(&dir, "api-admin-token.txt").unwrap(), first, "le jeton d'administration est un autre jeton");
        // Un jeton trop court (fichier abîmé ou vidé) est remplacé
        std::fs::write(dir.join("api-token.txt"), "court").unwrap();
        assert_ne!(load_token(&dir, "api-token.txt").unwrap(), "court");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
