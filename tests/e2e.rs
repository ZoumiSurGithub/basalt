//! Test de bout en bout : un faux client parle au serveur en UDP
//! (ping, poignée de main RakNet, réglages réseau, login, plugin Lua).

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use basalt::bedrock::{read_varu32, write_varu32};
use basalt::config::Config;
use basalt::lua::PluginManager;
use basalt::raknet::MAGIC;
use basalt::server::Server;

const PLUGIN: &str = r#"
server.on("login", function(p)
  if p.name == "Banni" then return "Tu es banni, " .. p.name end
end)
server.on("join", function(name) server.set_motd("Bienvenue " .. name) end)
server.after(1, function() server.set_motd("Motd Lua") end)
"#;

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

struct Client {
    sock: UdpSocket,
    seq: u32,
    ridx: u32,
    oidx: u32,
    inbox: std::collections::VecDeque<Vec<u8>>,
    parts: std::collections::HashMap<u16, Vec<Option<Vec<u8>>>>,
    games: std::collections::VecDeque<(u32, Vec<u8>)>,
    /// Datagrammes reçus pas encore acquittés (acquittés par lots, comme un vrai client)
    to_ack: Vec<[u8; 3]>,
}

impl Client {
    fn new(server: SocketAddr) -> Client {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.connect(server).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        Client { sock, seq: 0, ridx: 0, oidx: 0, inbox: Default::default(), parts: Default::default(), games: Default::default(), to_ack: Vec::new() }
    }

    fn recv_raw(&self) -> Vec<u8> {
        let mut buf = [0u8; 2048];
        let n = self.sock.recv(&mut buf).expect("pas de réponse du serveur");
        buf[..n].to_vec()
    }

    fn ping(&self) -> String {
        let mut p = vec![0x01];
        p.extend_from_slice(&7i64.to_be_bytes());
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&1i64.to_be_bytes());
        // Le serveur limite les pings par adresse : on réessaie après la fenêtre d'une seconde
        self.sock.set_read_timeout(Some(Duration::from_millis(1200))).unwrap();
        let mut buf = [0u8; 2048];
        let mut r = Vec::new();
        'retry: for _ in 0..4 {
            self.sock.send(&p).unwrap();
            while let Ok(n) = self.sock.recv(&mut buf) {
                if buf[0] == 0x1c {
                    r = buf[..n].to_vec();
                    break 'retry;
                }
            }
        }
        self.sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        assert!(!r.is_empty(), "le serveur ne répond plus aux pings");
        assert_eq!(&r[1..9], &7i64.to_be_bytes());
        let len = u16::from_be_bytes([r[33], r[34]]) as usize;
        String::from_utf8(r[35..35 + len].to_vec()).unwrap()
    }

    fn handshake(&mut self) {
        let mut p = vec![0x05];
        p.extend_from_slice(&MAGIC);
        p.push(11);
        p.resize(1200, 0);
        self.sock.send(&p).unwrap();
        assert_eq!(self.recv_raw()[0], 0x06);

        let mut p = vec![0x07];
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&[4, 0x80, 0xff, 0xff, 0xfe, 0x4a, 0xbc]);
        p.extend_from_slice(&1228u16.to_be_bytes());
        p.extend_from_slice(&99i64.to_be_bytes());
        self.sock.send(&p).unwrap();
        assert_eq!(self.recv_raw()[0], 0x08);

        let mut p = vec![0x09];
        p.extend_from_slice(&99i64.to_be_bytes());
        p.extend_from_slice(&1234i64.to_be_bytes());
        p.push(0);
        self.send(&p);
        let r = self.recv();
        assert_eq!(r[0], 0x10, "ConnectionRequestAccepted attendu");
        self.send(&[0x13, 0, 0, 0]);
    }

    fn frame(&mut self, body: &[u8], order: u32, split: Option<(u32, u32)>) {
        let mut d = vec![0x84];
        d.extend_from_slice(&self.seq.to_le_bytes()[..3]);
        self.seq += 1;
        d.push(0x60 | if split.is_some() { 0x10 } else { 0 });
        d.extend_from_slice(&((body.len() * 8) as u16).to_be_bytes());
        d.extend_from_slice(&self.ridx.to_le_bytes()[..3]);
        self.ridx += 1;
        d.extend_from_slice(&order.to_le_bytes()[..3]);
        d.push(0);
        if let Some((count, index)) = split {
            d.extend_from_slice(&count.to_be_bytes());
            d.extend_from_slice(&5u16.to_be_bytes());
            d.extend_from_slice(&index.to_be_bytes());
        }
        d.extend_from_slice(body);
        self.sock.send(&d).unwrap();
    }

    /// Envoi fiable et ordonné, fragmenté au besoin (fragments envoyés dans le désordre).
    fn send(&mut self, payload: &[u8]) {
        let order = self.oidx;
        self.oidx += 1;
        if payload.len() <= 1000 {
            self.frame(payload, order, None);
        } else {
            let chunks: Vec<&[u8]> = payload.chunks(1000).collect();
            let count = chunks.len() as u32;
            for (i, c) in chunks.iter().enumerate().rev() {
                self.frame(c, order, Some((count, i as u32)));
            }
        }
    }

    /// Attend le prochain paquet utile (acquitte, réassemble les fragments).
    fn recv(&mut self) -> Vec<u8> {
        loop {
            if let Some(p) = self.inbox.pop_front() {
                return p;
            }
            // S'il reste des datagrammes déjà arrivés, on les lit d'abord ; sinon tout ce qui
            // a été reçu est acquitté avant d'attendre la suite
            let mut buf = [0u8; 2048];
            self.sock.set_nonblocking(true).unwrap();
            let ready = self.sock.recv(&mut buf).ok();
            self.sock.set_nonblocking(false).unwrap();
            let r = match ready {
                Some(n) => buf[..n].to_vec(),
                None => {
                    self.flush_acks();
                    self.recv_raw()
                }
            };
            self.feed(&r);
        }
    }

    /// Envoie en un seul datagramme les accusés de réception en attente.
    fn flush_acks(&mut self) {
        if self.to_ack.is_empty() {
            return;
        }
        let mut ack = vec![0xc0];
        ack.extend_from_slice(&(self.to_ack.len() as u16).to_be_bytes());
        for seq in self.to_ack.drain(..) {
            ack.push(1); // un seul numéro
            ack.extend_from_slice(&seq);
        }
        self.sock.send(&ack).unwrap();
    }

    /// Traite un datagramme reçu : acquitte et range les paquets complets.
    fn feed(&mut self, r: &[u8]) {
        {
            if r[0] & 0x40 != 0 || r[0] & 0x20 != 0 {
                return;
            }
            // Un accusé par datagramme ferait dépasser la limite de débit du serveur pendant
            // l'envoi des grosses listes (recettes, objets) : on les regroupe
            self.to_ack.push([r[1], r[2], r[3]]);
            if self.to_ack.len() >= 32 {
                self.flush_acks();
            }
            let mut pos = 4;
            while pos < r.len() {
                let flags = r[pos];
                assert_eq!(flags >> 5, 3, "trame fiable ordonnée attendue (position {} sur {}, début {:02x?})", pos, r.len(), &r[..r.len().min(24)]);
                let len = (u16::from_be_bytes([r[pos + 1], r[pos + 2]]) as usize) / 8;
                pos += 10;
                if flags & 0x10 != 0 {
                    let count = u32::from_be_bytes([r[pos], r[pos + 1], r[pos + 2], r[pos + 3]]) as usize;
                    let id = u16::from_be_bytes([r[pos + 4], r[pos + 5]]);
                    let index = u32::from_be_bytes([r[pos + 6], r[pos + 7], r[pos + 8], r[pos + 9]]) as usize;
                    pos += 10;
                    let slot = self.parts.entry(id).or_insert_with(|| vec![None; count]);
                    slot[index] = Some(r[pos..pos + len].to_vec());
                    if slot.iter().all(|p| p.is_some()) {
                        let whole: Vec<u8> = self.parts.remove(&id).unwrap().into_iter().flatten().flatten().collect();
                        self.inbox.push_back(whole);
                    }
                } else {
                    self.inbox.push_back(r[pos..pos + len].to_vec());
                }
                pos += len;
            }
        }
    }

    fn game(&mut self, id: u32, body: &[u8], header: Option<u8>) {
        let mut inner = Vec::new();
        write_varu32(&mut inner, id);
        inner.extend_from_slice(body);
        let mut batch = Vec::new();
        write_varu32(&mut batch, inner.len() as u32);
        batch.extend_from_slice(&inner);
        let mut p = vec![0xfe];
        match header {
            None => p.extend_from_slice(&batch),
            Some(0xff) => {
                p.push(0xff);
                p.extend_from_slice(&batch);
            }
            Some(_) => {
                // « Compression » zlib en blocs stockés
                p.push(0x00);
                let n = batch.chunks(60000).count();
                for (i, c) in batch.chunks(60000).enumerate() {
                    p.push(if i + 1 == n { 1 } else { 0 });
                    p.extend_from_slice(&(c.len() as u16).to_le_bytes());
                    p.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
                    p.extend_from_slice(c);
                }
            }
        }
        self.send(&p);
    }

    /// Lit un paquet de jeu : (identifiant, corps). Un lot peut en contenir plusieurs.
    /// Comme `recv_game`, en ignorant les paquets d'état initial du joueur (vie, inventaire…).
    fn recv_after_state(&mut self) -> (u32, Vec<u8>) {
        loop {
            let (id, body) = self.recv_game(true);
            if !basalt::bedrock::EXTRA_IDS.contains(&id) {
                return (id, body);
            }
        }
    }

    fn recv_game(&mut self, header: bool) -> (u32, Vec<u8>) {
        loop {
            if let Some(p) = self.games.pop_front() {
                return p;
            }
            let r = self.recv();
            self.unpack(&r, header);
        }
    }

    fn unpack(&mut self, r: &[u8], header: bool) {
        assert_eq!(r[0], 0xfe);
        let mut pos = 1;
        if header {
            assert_eq!(r[1], 0xff);
            pos = 2;
        }
        while pos < r.len() {
            let len = read_varu32(r, &mut pos).unwrap() as usize;
            let end = pos + len;
            let id = read_varu32(r, &mut pos).unwrap();
            self.games.push_back((id, r[pos..end].to_vec()));
            pos = end;
        }
    }

    /// Ramasse tous les paquets de jeu jusqu'à ce que le serveur se taise pendant `quiet`.
    fn drain_games(&mut self, quiet: Duration) -> Vec<(u32, Vec<u8>)> {
        self.sock.set_read_timeout(Some(quiet)).unwrap();
        let mut buf = [0u8; 2048];
        while let Ok(n) = self.sock.recv(&mut buf) {
            let d = buf[..n].to_vec();
            self.feed(&d);
        }
        self.flush_acks();
        self.sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        while let Some(p) = self.inbox.pop_front() {
            self.unpack(&p, true);
        }
        self.games.drain(..).collect()
    }

    /// Attend le prochain paquet de jeu ayant cet identifiant.
    fn wait_for(&mut self, id: u32) -> Vec<u8> {
        for _ in 0..2000 {
            let (got, body) = self.recv_game(true);
            if got == id {
                return body;
            }
        }
        panic!("paquet {} jamais reçu", id);
    }

    /// Paquet de déplacement du joueur vers (x, z), au niveau du sol.
    fn walk_to(&mut self, x: f32, z: f32) {
        let mut b = vec![0u8; 8];
        for v in [x, -58.38f32, z] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.resize(48, 0);
        self.game(144, &b, Some(0xff));
    }

    /// Connexion complète jusqu'à l'apparition dans le monde (sans authentification).
    fn join(addr: SocketAddr, name: &str) -> Client {
        let mut c = Client::new(addr);
        c.handshake();
        c.game(0xc1, &2225i32.to_be_bytes(), None);
        assert_eq!(c.recv_game(false).0, 0x8f);
        c.login(2225, name, false);
        c.wait_for(6);
        c.game(8, &[3, 0, 0], Some(0xff));
        c.wait_for(7);
        c.game(8, &[4, 0, 0], Some(0xff));
        c.wait_for(11);
        c.game(69, &[8, 16], Some(0xff));
        loop {
            if c.wait_for(2) == [0, 0, 0, 3] {
                break;
            }
        }
        c.game(113, &[2], Some(0xff));
        c
    }

    fn login(&mut self, protocol: i32, name: &str, compressed: bool) {
        let payload = format!(r#"{{"extraData":{{"displayName":"{}","XUID":"42"}}}}"#, name);
        let jwt = format!("e30.{}.sig", b64(payload.as_bytes()));
        let cert = format!(r#"{{"chain":["{}"]}}"#, jwt).replace('"', "\\\"");
        let chain = format!(r#"{{"AuthenticationType":0,"Certificate":"{}","Token":""}}"#, cert);
        let skin = "A".repeat(6000); // force la fragmentation
        let client = format!(
            "e30.{}.sig",
            b64(format!(r#"{{"GameVersion":"1.26.40","SkinData":"{}"}}"#, skin).as_bytes())
        );
        self.login_raw(protocol, &chain, &client, compressed);
    }

    /// Envoie un login avec un JSON d'authentification et des données client fournis.
    fn login_raw(&mut self, protocol: i32, chain: &str, client: &str, compressed: bool) {
        let mut data = Vec::new();
        data.extend_from_slice(&(chain.len() as u32).to_le_bytes());
        data.extend_from_slice(chain.as_bytes());
        data.extend_from_slice(&(client.len() as u32).to_le_bytes());
        data.extend_from_slice(client.as_bytes());
        let mut body = protocol.to_be_bytes().to_vec();
        write_varu32(&mut body, data.len() as u32);
        body.extend_from_slice(&data);
        self.game(0x01, &body, Some(if compressed { 0 } else { 0xff }));

    }

    /// Attend la notification de fermeture de session envoyée par le serveur.
    fn expect_closed(&mut self) {
        let mut buf = [0u8; 2048];
        for _ in 0..5000 {
            match self.sock.recv(&mut buf) {
                Ok(n) if n >= 8 && buf[0] == 0x84 && buf[4] == 0x00 && buf[7] == 0x15 => return,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        panic!("le serveur n'a pas fermé la session");
    }

    fn expect_disconnect(&mut self) -> String {
        let (id, b) = self.recv_game(true);
        assert_eq!(id, 0x05, "paquet Disconnect attendu");
        let mut pos = 2;
        let len = read_varu32(&b, &mut pos).unwrap() as usize;
        String::from_utf8(b[pos..pos + len].to_vec()).unwrap()
    }
}

fn start(dir: &PathBuf) -> (SocketAddr, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    start_with(dir, 50)
}

fn start_with(dir: &PathBuf, max_per_ip: usize) -> (SocketAddr, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    start_opts(dir, max_per_ip, false)
}

fn start_opts(dir: &PathBuf, max_per_ip: usize, xbox_auth: bool) -> (SocketAddr, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let (tx, rx) = mpsc::channel();
    let dir = dir.clone();
    let handle = std::thread::spawn(move || {
        let mut cfg = Config::default();
        cfg.ip = "127.0.0.1".into();
        cfg.port = 0;
        cfg.plugins_dir = dir.to_string_lossy().into_owned();
        cfg.world_dir = dir.join("world").to_string_lossy().into_owned();
        cfg.root = dir.to_path_buf();
        // Les tests activent la liste blanche en déposant ce fichier témoin
        cfg.white_list = dir.join("liste-blanche-active").exists();
        cfg.view_distance = 3;
        cfg.max_per_ip = max_per_ip;
        cfg.xbox_auth = xbox_auth;
        cfg.nethernet_udp_port = 0;
        cfg.gamemode = "Creative".into();
        cfg.debug = std::env::var("BASALT_TEST_DEBUG").is_ok();
        cfg.auth_keys_file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/auth/keys.json").into();
        let mut server = Server::new(cfg).unwrap();
        tx.send(server.local_addr().unwrap()).unwrap();
        let (_ctx, crx) = mpsc::channel::<String>();
        server.run(&stop2, &crx);
    });
    (rx.recv().unwrap(), stop, handle)
}

#[test]
fn full_connection_flow() {
    let dir = std::env::temp_dir().join(format!("basalt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.lua"), PLUGIN).unwrap();
    let lua = PluginManager::new(&dir).available();
    let (addr, stop, handle) = start(&dir);
    // Le serveur charge ses données (objets, recettes) avant son premier tick
    std::thread::sleep(Duration::from_millis(1500));

    // 1. Le serveur répond aux pings (liste des serveurs)
    let c = Client::new(addr);
    let pong = c.ping();
    assert!(pong.starts_with("MCPE;"), "{}", pong);
    assert!(pong.contains(";2225;1.26.65;0;20;"), "{}", pong);
    if lua {
        assert!(pong.contains("Motd Lua"), "la minuterie Lua n'a pas changé le nom : {}", pong);
    }

    // 2. Mauvaise version du jeu : statut « client trop ancien »
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &100i32.to_be_bytes(), None);
    let (id, body) = c.recv_game(false);
    assert_eq!((id, body), (0x02, vec![0, 0, 0, 1]));

    // 3. Connexion normale, login non compressé et fragmenté
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &2225i32.to_be_bytes(), None);
    let (id, body) = c.recv_game(false);
    assert_eq!(id, 0x8f, "NetworkSettings attendu");
    assert_eq!(body.len(), 10);
    c.login(2225, "Steve", false);
    assert_eq!(c.recv_game(true), (2, vec![0, 0, 0, 0]), "PlayStatus LoginSuccess attendu");
    assert_eq!(c.recv_game(true).0, 6, "ResourcePacksInfo attendu");
    c.game(8, &[3, 0, 0], Some(0xff));
    assert_eq!(c.recv_game(true).0, 7, "ResourcePackStack attendu");
    c.game(8, &[4, 0, 0], Some(0xff));
    let (id, jigsaw) = c.recv_game(true);
    assert_eq!(id, 313, "JigsawStructureData attendu avant StartGame");
    assert!(jigsaw.windows(7).any(|w| w == b"jigsaws"), "listes de règles attendues");
    assert_eq!(c.recv_game(true), (337, vec![0, 0, 0, 0]), "VoxelShapes attendu avant StartGame");
    let (id, start) = c.recv_game(true);
    assert_eq!(id, 11, "StartGame attendu");
    assert!(start.len() > 150, "StartGame trop court : {}", start.len());
    assert_eq!(c.recv_game(true).0, 162);
    c.wait_for(122); // liste volumineuse : elle arrive en plusieurs morceaux

    // 4. Demande de terrain : rayon, zone publiée, chunks, puis apparition
    c.game(69, &[40, 16], Some(0xff)); // rayon demandé : 20 (zigzag), le serveur plafonne à 3
    assert_eq!(c.recv_after_state(), (70, vec![6]));
    assert_eq!(c.recv_game(true).0, 121);
    let mut chunks = 0;
    let mut first_chunk_len = 0;
    loop {
        // Ce client de test ne remet pas les messages dans l'ordre : les listes volumineuses
        // (recettes, créatif), découpées en morceaux, peuvent se terminer au milieu des chunks.
        let (id, body) = c.recv_after_state();
        if id == 58 {
            if chunks == 0 {
                // premier chunk : (0, 0), dimension 0, 1 sous-chunk, sans cache
                // (la suite de l'en-tête dépend de la variante en cours de mise au point avec le vrai jeu)
                assert_eq!(&body[..4], &[0, 0, 0, 1]);
                first_chunk_len = body.len();
            }
            chunks += 1;
        } else {
            assert_eq!((id, body), (2, vec![0, 0, 0, 3]), "PlayStatus PlayerSpawn attendu");
            break;
        }
    }
    assert_eq!(chunks, 29, "nombre de chunks dans un rayon de 3");
    c.game(113, &[2], Some(0xff));
    std::thread::sleep(Duration::from_millis(200));
    if lua {
        assert!(c.ping().contains("Bienvenue Steve"), "événement join non reçu par le plugin");
    }
    assert!(c.ping().contains(";1;20;"), "le joueur devrait être compté");

    // 4 bis. Le joueur marche jusqu'au chunk (2, 0) : le terrain manquant suit
    let mut x = 0.5f32;
    while x < 40.5 {
        x += 0.5;
        c.walk_to(x, 0.5);
    }
    let inside = |cx: i32, x: i32, z: i32| (x - cx) * (x - cx) + z * z <= 9;
    let (mut at_least, mut at_most) = (0, 0);
    for x in -10..10 {
        for z in -10..10 {
            if inside(2, x, z) && !inside(0, x, z) {
                at_least += 1;
            }
            if (inside(2, x, z) || inside(1, x, z)) && !inside(0, x, z) {
                at_most += 1;
            }
        }
    }
    // Positions aberrantes ignorées, et aucun chunk renvoyé sans changement de chunk
    c.walk_to(f32::NAN, 0.5);
    c.walk_to(1.0e12, 0.5);
    c.walk_to(41.0, 3.0);
    let (mut zones, mut chunks) = (0, 0);
    for (id, _) in c.drain_games(Duration::from_millis(500)) {
        match id {
            121 => zones += 1,
            58 => chunks += 1,
            other => panic!("paquet inattendu pendant la marche : {}", other),
        }
    }
    assert_eq!(zones, 2, "une zone publiée par chunk franchi");
    assert!(chunks >= at_least && chunks <= at_most, "{} chunks reçus, attendu entre {} et {}", chunks, at_least, at_most);

    // 5. Même pseudo une seconde fois : refusé
    let mut d = Client::new(addr);
    d.handshake();
    d.game(0xc1, &2225i32.to_be_bytes(), None);
    d.recv_game(false);
    d.login(2225, "steve", false);
    assert_eq!(d.expect_disconnect(), "Un joueur porte déjà ce pseudo.");

    // 5 bis. Autres versions : la suite de paquets s'adapte au protocole du client
    for (proto, jigsaw, chunk_flag) in [(2168, false, false), (2193, true, false), (2259, true, true)] {
        let mut v = Client::new(addr);
        v.handshake();
        v.game(0xc1, &(proto as i32).to_be_bytes(), None);
        let (id, body) = v.recv_game(false);
        // le profil SCTP n'existe qu'à partir de la préversion 1.26.70
        assert_eq!((id, body.len()), (0x8f, if proto >= 2251 { 11 } else { 10 }));
        v.login(proto, &format!("Joueur{}", proto), false);
        assert_eq!(v.recv_game(true).0, 2);
        assert_eq!(v.recv_game(true).0, 6);
        v.game(8, &[3, 0, 0], Some(0xff));
        assert_eq!(v.recv_game(true).0, 7);
        v.game(8, &[4, 0, 0], Some(0xff));
        let (mut id, mut body) = v.recv_game(true);
        assert_eq!(id == 313, jigsaw, "JigsawStructureData pour le protocole {}", proto);
        if jigsaw {
            let next = v.recv_game(true);
            id = next.0;
            body = next.1;
        }
        if id == 337 {
            let next = v.recv_game(true);
            id = next.0;
            body = next.1;
        }
        assert_eq!(id, 11);
        // la version de migration de l'éditeur ajoute un octet à partir de 1.26.60
        assert_eq!(body.len(), start.len() - if proto >= 2216 { 0 } else { 1 }, "protocole {}", proto);
        assert_eq!(v.recv_game(true).0, 162);
        assert_eq!(v.recv_game(true).0, 122);
        v.game(69, &[4, 16], Some(0xff));
        assert_eq!(v.recv_after_state(), (70, vec![4]));
        assert_eq!(v.recv_game(true).0, 121);
        let (id, chunk) = v.recv_game(true);
        assert_eq!(id, 58);
        // Les versions récentes ajoutent des octets à l'en-tête (variante en cours de mise au point)
        if chunk_flag {
            assert_eq!(chunk.len(), first_chunk_len, "protocole {}", proto);
        } else {
            assert!(chunk.len() < first_chunk_len, "protocole {}", proto);
        }
    }

    // 6. Login compressé + refus décidé par le plugin Lua
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &2225i32.to_be_bytes(), None);
    c.recv_game(false);
    c.login(2225, "Banni", true);
    if lua {
        assert_eq!(c.expect_disconnect(), "Tu es banni, Banni");
    } else {
        assert_eq!(c.recv_game(true).0, 2);
    }

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn garbage_datagrams_do_not_crash() {
    let dir = std::env::temp_dir().join(format!("basalt-fuzz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start(&dir);
    let mut c = Client::new(addr);
    c.handshake();
    let mut x = 0x1234_5678u32;
    for i in 0..20000 {
        let len = 1 + (i % 90);
        let mut d: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 8) as u8
            })
            .collect();
        // Une partie des paquets vise le chemin « session connectée »
        match i % 4 {
            0 => d[0] = 0x84,
            1 => d[0] = 0xc0,
            2 => d[0] = [0x01, 0x05, 0x07, 0xa0][(i / 4) % 4],
            _ => {}
        }
        c.sock.send(&d).unwrap();
        if i % 200 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    // Le flot dépasse le débit autorisé : la session fautive est fermée…
    c.expect_closed();
    // …et le serveur répond toujours aux autres
    let fresh = Client::new(addr);
    assert!(fresh.ping().starts_with("MCPE;"));
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Flux DEFLATE minuscule qui se décompresse en `1 + 258 * matches` octets nuls.
fn deflate_bomb(matches: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut n) = (0u32, 0u32);
    let mut put = |value: u32, bits: u32, msb_first: bool, out: &mut Vec<u8>| {
        for i in 0..bits {
            let bit = if msb_first { (value >> (bits - 1 - i)) & 1 } else { (value >> i) & 1 };
            acc |= bit << n;
            n += 1;
            if n == 8 {
                out.push(acc as u8);
                acc = 0;
                n = 0;
            }
        }
    };
    put(1, 1, false, &mut out); // dernier bloc
    put(1, 2, false, &mut out); // codes de Huffman fixes
    put(0x30, 8, true, &mut out); // littéral 0
    for _ in 0..matches {
        put(0xc5, 8, true, &mut out); // longueur 258
        put(0, 5, true, &mut out); // distance 1
    }
    put(0, 7, true, &mut out); // fin de bloc
    put(0, 7, false, &mut out); // remplissage
    out
}

#[test]
fn decompression_bomb_is_rejected() {
    // 13 Ko compressés -> 2 Mo : accepté seulement si la limite le permet
    let bomb = deflate_bomb(8000);
    assert!(bomb.len() < 14_000);
    assert_eq!(basalt::inflate::inflate(&bomb, 4 << 20).unwrap().len(), 1 + 258 * 8000);
    assert!(basalt::inflate::inflate(&bomb, 1 << 20).is_err());

    let dir = std::env::temp_dir().join(format!("basalt-bomb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start(&dir);
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &2225i32.to_be_bytes(), None);
    c.recv_game(false);
    // 26 Ko compressés qui en vaudraient 4 Mo : au-delà de la limite d'un login
    let mut p = vec![0xfe, 0x00];
    p.extend_from_slice(&deflate_bomb(16000));
    let started = std::time::Instant::now();
    c.send(&p);
    c.expect_closed();
    assert!(started.elapsed() < Duration::from_secs(2), "le rejet doit être immédiat");
    assert!(Client::new(addr).ping().starts_with("MCPE;"));
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packet_flood_closes_only_the_offender() {
    let dir = std::env::temp_dir().join(format!("basalt-flood-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start(&dir);
    let mut victim = Client::new(addr);
    victim.handshake();
    victim.game(0xc1, &2225i32.to_be_bytes(), None);
    assert_eq!(victim.recv_game(false).0, 0x8f);

    let mut flooder = Client::new(addr);
    flooder.handshake();
    for i in 0..4000u32 {
        // Lots de jeu valides mais inutiles, en rafale
        flooder.game(0xc1, &i.to_be_bytes(), None);
    }
    flooder.expect_closed();
    // Laisse le serveur vider la rafale restante : ce faux client ne sait pas
    // réémettre un paquet perdu, contrairement à un vrai client.
    std::thread::sleep(Duration::from_millis(500));

    // Le joueur honnête poursuit sa connexion normalement
    victim.login(2225, "Honnete", false);
    assert_eq!(victim.recv_game(true), (2, vec![0, 0, 0, 0]));
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn connections_per_ip_are_limited() {
    let dir = std::env::temp_dir().join(format!("basalt-ip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start_with(&dir, 2);
    let mut a = Client::new(addr);
    a.handshake();
    let mut b = Client::new(addr);
    b.handshake();
    // Troisième connexion depuis la même adresse : refusée (« aucune place libre »)
    let c = Client::new(addr);
    let mut p = vec![0x07];
    p.extend_from_slice(&MAGIC);
    p.extend_from_slice(&[4, 0x80, 0xff, 0xff, 0xfe, 0x4a, 0xbc]);
    p.extend_from_slice(&1228u16.to_be_bytes());
    p.extend_from_slice(&77i64.to_be_bytes());
    c.sock.send(&p).unwrap();
    assert_eq!(c.recv_raw()[0], 0x14);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn xbox_authentication_flow() {
    let fixtures: std::collections::HashMap<&str, &str> = include_str!("auth/fixtures.txt")
        .lines()
        .filter_map(|l| l.find(' ').map(|i| (&l[..i], &l[i + 1..])))
        .collect();
    let dir = std::env::temp_dir().join(format!("basalt-auth-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start_opts(&dir, 50, true);
    // Le client annonce « Imposteur » dans une ancienne chaîne auto-signée ; seul le jeton compte.
    let request = |token: &str| {
        let cert = format!(r#"{{"chain":["{}"]}}"#, fixtures["self_signed_chain"]).replace('"', "\\\"");
        format!(r#"{{"AuthenticationType":0,"Certificate":"{}","Token":"{}"}}"#, cert, token)
    };
    let connect = || {
        let mut c = Client::new(addr);
        c.handshake();
        c.game(0xc1, &2225i32.to_be_bytes(), None);
        c.recv_game(false);
        c
    };

    // Jeton valide : accepté, sous le pseudo certifié par le jeton
    let mut c = connect();
    c.login_raw(2225, &request(fixtures["token_valid"]), fixtures["client_valid"], false);
    assert_eq!(c.recv_game(true), (2, vec![0, 0, 0, 0]), "login accepté attendu");
    std::thread::sleep(Duration::from_millis(100));
    assert!(c.ping().contains(";1;20;"));

    // Sans jeton (joueur non connecté à Xbox)
    let mut c = connect();
    c.login_raw(2225, &request(""), fixtures["client_valid"], false);
    assert_eq!(c.expect_disconnect(), "Connectez-vous à votre compte Xbox pour rejoindre ce serveur.");

    // Jeton falsifié, jeton expiré, données signées par une autre clé
    for (token, client) in [
        ("token_tampered", "client_valid"),
        ("token_expired", "client_valid"),
        ("token_other_signer", "client_valid"),
        ("token_valid", "client_other_key"),
    ] {
        let mut c = connect();
        c.login_raw(2225, &request(fixtures[token]), fixtures[client], false);
        assert_eq!(c.expect_disconnect(), "Échec de la vérification du compte Xbox.", "{} / {}", token, client);
    }
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn players_see_each_other_without_packets_the_game_refuses() {
    let dir = std::env::temp_dir().join(format!("basalt-players-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start(&dir);

    let mut alpha = Client::join(addr, "Alpha");
    let mut bravo = Client::join(addr, "Bravo");

    // Chacun voit apparaître l'autre, avec son nom
    let seen_by_alpha = alpha.wait_for(12);
    let seen_by_bravo = bravo.wait_for(12);
    assert!(seen_by_alpha.windows(5).any(|w| w == b"Bravo"));
    assert!(seen_by_bravo.windows(5).any(|w| w == b"Alpha"));
    let alpha_id = seen_by_bravo[16 + 1 + 5]; // après l'UUID et le nom : identifiant d'exécution

    // Alpha marche, puis fait un bond de 50 blocs. Un vrai jeu 1.26.63 a refusé le paquet de
    // déplacement (19) tel que le serveur l'écrivait et s'est déconnecté : tant que son
    // format n'est pas connu pour ce protocole, il ne doit plus partir, ni vers Bravo
    // (déplacements d'Alpha), ni vers Alpha (correction).
    for step in 1..=4 {
        alpha.walk_to(0.5 + step as f32 * 0.5, 0.5);
        std::thread::sleep(Duration::from_millis(60));
    }
    alpha.walk_to(52.5, 0.5);
    std::thread::sleep(Duration::from_millis(200));

    // Alpha se déconnecte : il disparaît chez Bravo, qui n'a reçu aucun paquet 19 entre-temps
    alpha.send(&[0x15]);
    loop {
        let (id, body) = bravo.recv_game(true);
        assert_ne!(id, 19, "le paquet de déplacement ne doit pas être envoyé à un jeu 1.26.60+");
        if id == 14 {
            assert_eq!(body, vec![alpha_id * 2], "identifiant d'Alpha (zigzag)");
            break;
        }
    }

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn capture_tool_talks_raknet_to_basalt_itself() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-capture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start(&dir);
    let payload = dir.join("client-data.b64");
    // modèle de données client : un vrai jeu en fournit un bien plus gros (skin)
    let skin = "A".repeat(40_000);
    std::fs::write(&payload, b64(format!(r#"{{"GameVersion":"1.26.65","SkinData":"{}"}}"#, skin).as_bytes())).unwrap();
    let out = dir.join("data");
    // Un témoin entre d'abord et reste en jeu : la capture reçoit alors, en plus du reste,
    // le paquet qui fait apparaître un autre joueur
    let witness_payload = payload.clone();
    let port = addr.port();
    let witness = std::thread::spawn(move || {
        basalt::tools::capture_rak::run_as("127.0.0.1", port, None, &witness_payload, "BasaltTemoin", Duration::from_secs(15), ("", ""))
    });
    std::thread::sleep(Duration::from_millis(3000));
    // Le joueur de la capture reste 8 s en jeu : sur une machine lente (intégration continue),
    // le témoin a le temps de lui apparaître
    let summary = basalt::tools::capture_rak::run_as("127.0.0.1", addr.port(), Some(&out), &payload, "BasaltCapture", Duration::from_secs(8), ("", ""))
        .expect("capture");
    // Le témoin est attendu avant de conclure : en cas d'échec, son propre résultat dit pourquoi
    let witness_result = witness.join().unwrap();
    assert!(summary.contains("start_game"), "{}", summary);
    assert!(
        summary.contains("add_player"),
        "le témoin doit apparaître chez le joueur de la capture : {} — témoin : {:?}",
        summary,
        witness_result
    );
    let saved = out.join("2225").join("packets");
    assert!(std::fs::metadata(saved.join("start_game.bin")).unwrap().len() > 100);
    assert!(std::fs::metadata(saved.join("add_player.bin")).unwrap().len() > 40);
    assert!(witness_result.expect("témoin").contains("BasaltTemoin"));
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn whitelist_keeps_strangers_out() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-whitelist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("liste-blanche-active"), "").unwrap();
    std::fs::write(dir.join("white-list.txt"), "# joueurs invités\nAlice\n").unwrap();
    // Ancien emplacement des opérateurs : il doit être repris et déplacé à la racine
    std::fs::write(dir.join("world").join("ops.txt"), "bob\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(dir.join("ops.txt").exists() && !dir.join("world").join("ops.txt").exists());

    let refused = |name: &str| {
        let mut c = Client::new(addr);
        c.handshake();
        c.game(0xc1, &2225i32.to_be_bytes(), None);
        c.recv_game(false);
        c.login(2225, name, true);
        c.expect_disconnect()
    };
    // Un inconnu est refusé, quelle que soit la casse de son pseudo
    assert!(refused("Mallory").contains("liste blanche"));
    // Une inscrite et un opérateur entrent
    let _alice = Client::join(addr, "ALICE");
    let _bob = Client::join(addr, "Bob");

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Les données d'un joueur vivent au niveau du serveur (`players/`), pas dans un monde ; un
/// fichier laissé dans le dossier du monde par une ancienne version est repris et déplacé.
#[test]
fn player_data_belongs_to_the_server_not_to_a_world() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-playerdata-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Ancienne disposition : world/players/steve.nbt
    let world = dir.join("world");
    let mut old = basalt::player::PlayerData::new("Steve", 1_700_000_000);
    old.display_name = "Ancien".to_string();
    old.save(&world).unwrap();
    let old_file = basalt::player::PlayerData::path(&world, "Steve");
    assert!(old_file.exists());

    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let steve = Client::join(addr, "Steve");
    let alex = Client::join(addr, "Alex");
    std::thread::sleep(Duration::from_millis(300));
    drop(steve);
    drop(alex);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();

    let new_file = basalt::player::PlayerData::path(&dir, "Steve");
    assert!(new_file.exists(), "repris dans players/ à côté de server.properties");
    assert!(!old_file.exists() && !world.join("players").exists(), "l'ancien fichier est retiré du monde");
    let kept = basalt::player::PlayerData::load(&dir, "Steve").unwrap().unwrap();
    assert_eq!(kept.display_name, "Ancien", "les données sont conservées");
    // Un nouveau joueur est enregistré directement au bon endroit
    assert!(basalt::player::PlayerData::path(&dir, "Alex").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un coffre s'ouvre, reçoit un objet, et le garde pour la prochaine ouverture.
#[test]
fn a_chest_opens_and_keeps_what_is_put_in_it() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-chest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    // Clic droit relevé avec le vrai jeu (1.26.65) ; bloc visé et position du joueur adaptés
    let click: Vec<u8> = {
        let hex = "0000020100010000001afe01000000cbced3b4020a0000000000000000000000000000000000000001\
                   027502\
                   0100001afe01000000cbced3b4020a000000000000000000000000003f1c8569c20000003f8009653f0000803f0031153ed9f6edc5030100";
        (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect()
    };
    let close = |c: &mut Client| {
        c.game(47, &[2, 0, 0], Some(0xff));
        assert_eq!(c.wait_for(47), vec![2, 0, 0]);
    };

    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/setblock 1 -59 1 chest");
    steve.wait_for(21);
    command(&mut steve, "/give @s diamond 3");
    steve.wait_for(9);
    steve.wait_for(9);

    // Ouverture : l'écran (fenêtre 2, type coffre, position), puis ses 27 emplacements vides
    steve.game(30, &click, Some(0xff));
    assert_eq!(steve.wait_for(46), vec![2, 0, 0x02, 0x75, 0x02, 0x01]);
    let empty = loop {
        let content = steve.wait_for(49);
        if content[0] == 2 {
            break content;
        }
    };
    assert_eq!(empty[1], 27);

    // Les 3 diamants passent de la barre (conteneur 28, case 0) au coffre (conteneur 7, case 5)
    let mut request = vec![1, 0x05, 1, 1, 1, 3];
    request.extend_from_slice(&[28, 0, 0, 0, 0, 0, 0]);
    request.extend_from_slice(&[7, 0, 5, 0, 0, 0, 0]);
    request.extend_from_slice(&[0, 0, 0, 0, 0]);
    steve.game(147, &request, Some(0xff));
    let answer = steve.wait_for(148);
    assert_eq!(&answer[..4], &[1, 0, 0x05, 1], "demande acceptée : {:02x?}", answer);
    close(&mut steve);

    // Réouverture : le coffre n'est plus vide
    steve.game(30, &click, Some(0xff));
    steve.wait_for(46);
    let filled = loop {
        let content = steve.wait_for(49);
        if content[0] == 2 {
            break content;
        }
    };
    assert!(filled.len() > empty.len(), "le coffre doit contenir les diamants");

    // Commandes propres au serveur, réservées aux opérateurs
    let expect_text = |c: &mut Client, needle: &str| {
        for _ in 0..20 {
            if String::from_utf8_lossy(&c.wait_for(9)).contains(needle) {
                return;
            }
        }
        panic!("message « {} » jamais reçu", needle);
    };
    command(&mut steve, "/status");
    expect_text(&mut steve, "ticks/s");
    expect_text(&mut steve, "Joueurs : §f1/");
    command(&mut steve, "/version");
    expect_text(&mut steve, "Basalt 0.1.0");
    // Mesures de temps : rien sans les avoir allumées, puis un résumé et un fichier de rapport
    command(&mut steve, "/timings on");
    expect_text(&mut steve, "Mesures allumées");
    std::thread::sleep(Duration::from_millis(400));
    command(&mut steve, "/timings report");
    expect_text(&mut steve, "Mesures sur");
    expect_text(&mut steve, "Rapport complet");
    // Le rapport est écrit en arrière-plan : on lui laisse quelques secondes
    let mut text = String::new();
    for _ in 0..100 {
        let reports: Vec<_> = std::fs::read_dir(dir.join("timings")).map(|d| d.flatten().collect()).unwrap_or_default();
        if let Some(r) = reports.first() {
            assert_eq!(reports.len(), 1);
            text = std::fs::read_to_string(r.path()).unwrap_or_default();
            if text.contains("Tick complet") {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(text.contains("Tick complet") && text.contains("Réseau — lecture RakNet") && text.contains("Tick — joueurs vus par les autres"), "{}", text);
    command(&mut steve, "/timings off");
    expect_text(&mut steve, "Mesures éteintes");

    // Reprendre plus que ce qu'il contient est refusé
    let mut request = vec![1, 0x07, 1, 1, 0, 9];
    request.extend_from_slice(&[7, 0, 5, 0, 0, 0, 0]);
    request.extend_from_slice(&[28, 0, 1, 0, 0, 0, 0]);
    request.extend_from_slice(&[0, 0, 0, 0, 0]);
    steve.game(147, &request, Some(0xff));
    assert_eq!(&steve.wait_for(148)[..3], &[1, 1, 0x07]);

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vanilla_commands_work_like_in_the_game() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-vanilla-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let expect_text = |c: &mut Client, needle: &str| {
        let mut seen = Vec::new();
        for _ in 0..20 {
            let text = String::from_utf8_lossy(&c.wait_for(9)).to_string();
            if text.contains(needle) {
                return;
            }
            seen.push(text);
        }
        panic!("message « {} » jamais reçu ; reçus : {:?}", needle, seen);
    };

    // Un simple joueur a les commandes de tout le monde, pas celles des opérateurs
    let mut guest = Client::join(addr, "Alex");
    command(&mut guest, "/gamemode creative");
    expect_text(&mut guest, "permission");
    command(&mut guest, "/list");
    expect_text(&mut guest, "joueurs en ligne");
    command(&mut guest, "/status");
    expect_text(&mut guest, "permission");

    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/setblock ~ ~3 ~ glass");
    steve.wait_for(21);
    expect_text(&mut steve, "Bloc placé");
    command(&mut steve, "/fill ~ ~5 ~ ~1 ~6 ~1 minecraft:stone");
    expect_text(&mut steve, "8 blocs remplis");
    command(&mut steve, "/setblock ~ ~ ~ pas_un_bloc");
    expect_text(&mut steve, "bloc inconnu");
    command(&mut steve, "/time set night");
    assert_eq!(steve.wait_for(10), vec![0x90, 0xcb, 0x01], "13000 en entier zigzag");
    expect_text(&mut steve, "Heure définie sur 13000");
    command(&mut steve, "/gamemode creative");
    steve.wait_for(62);
    expect_text(&mut steve, "Créatif");
    command(&mut steve, "/give @s diamond 3");
    expect_text(&mut steve, "Steve a reçu diamond * 3");
    command(&mut steve, "/give Personne diamond");
    expect_text(&mut steve, "Aucune cible");
    command(&mut steve, "/tell Alex \"bonjour\"");
    expect_text(&mut guest, "Steve vous chuchote : bonjour");

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn easyedit_commands_typed_in_game_change_the_world() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-easyedit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let plugin = dir.join("EasyEdit");
    std::fs::create_dir_all(&plugin).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins").join("EasyEdit");
    for entry in std::fs::read_dir(&source).unwrap().flatten() {
        std::fs::copy(entry.path(), plugin.join(entry.file_name())).unwrap();
    }
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    if !PluginManager::new(&dir).available() {
        return;
    }
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(1500));

    // Le jeu envoie la commande tapée dans un paquet qui commence par son texte
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let text_of = |body: Vec<u8>| String::from_utf8_lossy(&body).to_string();
    // Attend le message contenant `needle` (d'autres messages, comme les arrivées, passent avant)
    let expect_text = |c: &mut Client, needle: &str| {
        let mut seen = Vec::new();
        for _ in 0..20 {
            let text = text_of(c.wait_for(9));
            if text.contains(needle) {
                return;
            }
            seen.push(text);
        }
        panic!("message « {} » jamais reçu ; reçus : {:?}", needle, seen);
    };

    // Un simple joueur n'a pas la permission
    let mut guest = Client::join(addr, "Alex");
    command(&mut guest, "//set stone");
    expect_text(&mut guest, "permission");
    command(&mut guest, "/inconnue");
    expect_text(&mut guest, "Commande inconnue");

    // L'opérateur choisit deux coins là où il se tient, puis remplit : un bloc change
    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "//pos1");
    expect_text(&mut steve, "Premier coin");
    command(&mut steve, "/pos2");
    expect_text(&mut steve, "1 blocs");
    command(&mut steve, "//set glass");
    let mut seen_block = false;
    let mut message = String::new();
    for _ in 0..200 {
        let (id, body) = steve.recv_game(true);
        if id == 21 {
            seen_block = true;
        }
        if id == 9 && text_of(body.clone()).contains("modifi") {
            message = text_of(body);
            break;
        }
    }
    assert!(seen_block, "le bloc changé doit être envoyé au jeu");
    assert!(message.contains("1 blocs modifi"), "{}", message);
    command(&mut steve, "//set pas_un_bloc");
    expect_text(&mut steve, "bloc inconnu");
    command(&mut steve, "//undo");
    expect_text(&mut steve, "1 blocs modifi");
    command(&mut steve, "//undo");
    expect_text(&mut steve, "rien à annuler");

    // Même chose tapée dans le chat avec un point : c'est la voie qui marche avec le vrai jeu,
    // qui n'envoie pas les commandes en « / » qu'il ne connaît pas
    let chat = |c: &mut Client, text: &str| {
        let mut body = vec![0u8, 1, 1];
        body.push(5);
        body.extend_from_slice(b"Steve");
        body.push(text.len() as u8);
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(9, &body, Some(0xff));
    };
    // Un message de chat qui ressemble à une commande reste un message : seules les
    // commandes du jeu (« / ») sont exécutées
    chat(&mut steve, ".set stone");
    expect_text(&mut steve, ".set stone");

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
