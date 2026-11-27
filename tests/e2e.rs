//! Tests de bout en bout : un faux client parle au serveur par le transport de test (TCP,
//! cette machine seulement) : état du serveur, réglages réseau, login, jeu, plugins Lua.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use basalt::bedrock::{read_varu32, write_varu32};
use basalt::config::Config;
use basalt::lua::PluginManager;
use basalt::transport::LocalClient;
use basalt::server::Server;

const PLUGIN: &str = r#"
server.on("login", function(p)
  if p.name == "Banni" then return "Tu es banni, " .. p.name end
end)
server.on("join", function(name) server.setMotd("Bienvenue " .. name) end)
server.afterSeconds(0.05, function() server.setMotd("Motd Lua") end)
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
    sock: LocalClient,
    inbox: std::collections::VecDeque<Vec<u8>>,
    games: std::collections::VecDeque<(u32, Vec<u8>)>,
}

impl Client {
    fn new(server: SocketAddr) -> Client {
        Client { sock: LocalClient::connect(server).unwrap(), inbox: Default::default(), games: Default::default() }
    }

    /// État du serveur (nom, version, joueurs), comme la liste des serveurs le lirait.
    fn ping(&mut self) -> String {
        self.sock.send(&[0x01]).unwrap();
        loop {
            let m = self.sock.recv(Duration::from_secs(3)).expect("liaison perdue").expect("le serveur ne répond plus");
            if m.first() == Some(&0x1c) {
                return String::from_utf8(m[1..].to_vec()).unwrap();
            }
            // Un lot du jeu arrivé entre-temps : gardé pour la suite
            self.inbox.push_back(m);
        }
    }

    /// La liaison est établie dès la connexion : rien à négocier.
    fn handshake(&mut self) {}

    fn send(&mut self, payload: &[u8]) {
        // (une liaison déjà fermée par le serveur : l'erreur se verra à la lecture)
        let _ = self.sock.send(payload);
    }

    /// Attend le prochain lot du serveur.
    fn recv(&mut self) -> Vec<u8> {
        if let Some(p) = self.inbox.pop_front() {
            return p;
        }
        self.sock.recv(Duration::from_secs(3)).expect("liaison perdue").expect("pas de réponse du serveur")
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

    /// Prochain paquet de jeu, ou rien si le serveur se tait pendant `wait`.
    fn poll_game(&mut self, wait: Duration) -> Option<(u32, Vec<u8>)> {
        loop {
            if let Some(p) = self.games.pop_front() {
                return Some(p);
            }
            if let Some(p) = self.inbox.pop_front() {
                self.unpack(&p, true);
                continue;
            }
            match self.sock.recv(wait) {
                Ok(Some(d)) => self.unpack(&d, true),
                _ => return None,
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
        let inflated;
        let mut r = r;
        if header {
            match r[1] {
                0xff => pos = 2,
                // Lot compressé par le serveur (« zlib », algorithme 0)
                0x00 => {
                    inflated = basalt::inflate::inflate(&r[2..], 8 << 20).expect("lot compressé illisible");
                    r = &inflated;
                    pos = 0;
                }
                other => panic!("marqueur de compression inattendu : {:#x}", other),
            }
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
        while let Ok(Some(d)) = self.sock.recv(quiet) {
            self.inbox.push_back(d);
        }
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

    /// Attend que le serveur ferme la liaison.
    fn expect_closed(&mut self) {
        for _ in 0..5000 {
            match self.sock.recv(Duration::from_secs(3)) {
                Err(_) => return,
                Ok(Some(_)) => {}
                Ok(None) => break,
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
        // Pas d'animaux (leurs paquets gêneraient les tests), sauf fichier témoin
        cfg.spawn_animals = dir.join("animaux").exists();
        cfg.spawn_monsters = dir.join("monstres").exists();
        cfg.api_test = dir.join("api").exists();
        // Déplacements libres (grands pas d'un coup), sur fichier témoin
        cfg.movement_check = !dir.join("deplacements-libres").exists();
        cfg.view_distance = 3;
        cfg.max_per_ip = max_per_ip;
        cfg.xbox_auth = xbox_auth;
        cfg.nethernet_udp_port = 0;
        cfg.test_transport = true;
        cfg.http_signaling = false;
        cfg.gamemode = "Creative".into();
        // Terrain plat : les tests marchent sur le sol à y = -60 (sauf fichier témoin)
        cfg.level_type = if let Ok(kind) = std::fs::read_to_string(dir.join("terrain-type")) {
            kind.trim().to_string()
        } else if dir.join("terrain-normal").exists() {
            "vanilla".into()
        } else {
            "flat".into()
        };
        cfg.level_seed = "42".into();
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
    let mut c = Client::new(addr);
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
    let mut listed = 0;
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
        } else if id == 121 {
            // La zone servie, annoncée avec les chunks qui viennent de partir (comme le
            // serveur officiel) : centre (0, -59, 0), 64 blocs, puis leur nombre
            assert_eq!(&body[..4], &[0, 117, 0, 64]);
            listed += u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
        } else {
            assert_eq!(listed, 29, "tous les chunks envoyés sont annoncés dans la zone servie");
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
    // (sous forte charge, le serveur peut réémettre un datagramme que ce faux client n'a pas
    // acquitté à temps, et ce client ne filtre pas les doublons : les zones sont comptées
    // une fois chacune)
    let (mut zones, mut chunk_set) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    let mut listed = std::collections::HashSet::new();
    for (id, body) in c.drain_games(Duration::from_millis(500)) {
        match id {
            121 => {
                // centre (x, y, z), rayon (64 blocs), nombre de chunks annoncés, puis leurs x et z
                let mut pos = 0;
                let x = basalt::bedrock::read_vari32(&body, &mut pos);
                pos += 3;
                let count = u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
                pos += 4;
                for _ in 0..count {
                    let cx = basalt::bedrock::read_vari32(&body, &mut pos);
                    let cz = basalt::bedrock::read_vari32(&body, &mut pos);
                    listed.insert((cx, cz));
                }
                zones.insert(x);
            }
            58 => {
                // x et z : entiers variables en tête du paquet
                let mut pos = 0;
                let x = basalt::bedrock::read_vari32(&body, &mut pos);
                let z = basalt::bedrock::read_vari32(&body, &mut pos);
                chunk_set.insert((x, z));
            }
            // Le reste (listes volumineuses qui finissent d'arriver, message d'arrivée, mise à
            // jour régulière de la vie…) ne concerne pas le terrain
            _ => {}
        }
    }
    // La zone servie suit la position exacte du joueur (comme le serveur officiel), et chaque
    // chunk envoyé y est annoncé
    assert!(zones.contains(&Some(41)), "la zone suit le joueur jusqu'à son arrivée : {:?}", zones);
    assert_eq!(listed, chunk_set, "les chunks annoncés sont ceux envoyés");
    let chunks = chunk_set.len();
    assert!(chunks >= at_least && chunks <= at_most, "{} chunks reçus, attendu entre {} et {}", chunks, at_least, at_most);

    // 5. Même pseudo une seconde fois : refusé
    let mut d = Client::new(addr);
    d.handshake();
    d.game(0xc1, &2225i32.to_be_bytes(), None);
    d.recv_game(false);
    d.login(2225, "steve", false);
    assert_eq!(d.expect_disconnect(), "Un joueur porte déjà ce pseudo.");

    // 5 bis. Versions d'avant la 1.26.60 : refusées (« jeu trop ancien »)
    for proto in [2168, 2193] {
        let mut v = Client::new(addr);
        v.handshake();
        v.game(0xc1, &(proto as i32).to_be_bytes(), None);
        assert_eq!(v.recv_game(false), (2, 1i32.to_be_bytes().to_vec()), "protocole {}", proto);
    }
    // Autres versions : la suite de paquets s'adapte au protocole du client
    for (proto, jigsaw, chunk_flag) in [(2259, true, true)] {
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
        // Tantôt un message bien formé au contenu absurde (parfois marqué comme un lot du
        // jeu), tantôt des octets bruts qui brouillent jusqu'au découpage des messages
        match i % 4 {
            0 => {
                d[0] = 0xfe;
                c.send(&d);
            }
            1 => c.send(&d),
            _ => {
                let _ = c.sock.send_raw(&d);
            }
        }
        if i % 200 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    // Messages illisibles : la session fautive est fermée…
    c.expect_closed();
    // …et le serveur répond toujours aux autres
    let mut fresh = Client::new(addr);
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
    // Les deux premières sont bien en place
    assert!(a.ping().starts_with("MCPE;") && b.ping().starts_with("MCPE;"));
    // Troisième connexion depuis la même adresse : fermée par le serveur
    let mut c = Client::new(addr);
    c.expect_closed();
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
    // Accepté : état « login réussi », puis les packs de ressources
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

    // Alpha marche, puis fait un bond de 50 blocs. Bravo voit Alpha bouger par
    // MoveActorAbsolute (le paquet 19 en mode normal n'est pas connu pour ce protocole) ;
    // Alpha, lui, est replacé par une téléportation au format relevé (paquet 19).
    for step in 1..=4 {
        alpha.walk_to(0.5 + step as f32 * 0.5, 0.5);
        std::thread::sleep(Duration::from_millis(60));
    }
    alpha.walk_to(52.5, 0.5);
    let corrected = alpha.wait_for(19);
    assert_eq!(corrected[corrected.len() - 13..], [0x02, 0x00, 0x00, 0x01, 0x03, 0, 0, 0, 0x01, 0, 0, 0, 0][..], "téléportation au format du serveur officiel");
    let x = f32::from_le_bytes([corrected[1], corrected[2], corrected[3], corrected[4]]);
    assert!(x < 10.0, "replacé à sa dernière position valide, pas à {}", x);
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
    expect_text(&mut steve, &format!("Basalt {}", env!("CARGO_PKG_VERSION")));
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
    assert!(text.contains("Tick complet") && text.contains("Réseau — lecture du transport de test") && text.contains("Tick — joueurs vus par les autres"), "{}", text);
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
fn a_plugin_rewrites_death_messages_and_cancels_damage() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-degats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("regles.lua"),
        r#"
server.on("damage", function(joueur, degats, cause, auteur)
  if cause == "custom" then return false end      -- les coups des plugins ne passent pas
end)
server.on("death", function(joueur, message)
  return "FIN DE " .. joueur.name
end)
server.command("fin", "tue le joueur", function(args, joueur)
  if joueur then joueur:setHealth(0) end
end)
server.command("frappe", "blesse le joueur", function(args, joueur)
  if joueur then
    joueur:damage(5)
    server.afterSeconds(0.3, function() joueur:sendMessage("vie " .. joueur:getHealth()) end)
  end
end)
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let expect_text = |c: &mut Client, needle: &str| {
        for _ in 0..30 {
            if String::from_utf8_lossy(&c.wait_for(9)).contains(needle) {
                return;
            }
        }
        panic!("message « {} » jamais reçu", needle);
    };
    let mut steve = Client::join(addr, "Steve");
    // Le coup du plugin est annulé par l'écouteur : la vie reste entière
    command(&mut steve, "/frappe");
    expect_text(&mut steve, "vie 20");
    // La mort, elle, a lieu, et son message est celui du plugin
    command(&mut steve, "/fin");
    expect_text(&mut steve, "FIN DE Steve");

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_plugin_inventory_opens_reports_clicks_and_gives_nothing() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-menu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("boutique.lua"),
        r#"
local menu = server.inventory("Boutique", 27)
menu:setItem(14, "minecraft:diamond", 2)
menu:onClick(function(joueur, case, objet, quantite)
  joueur:sendMessage("clic " .. case .. " " .. tostring(objet) .. " " .. quantite)
end)
menu:onClose(function(joueur, contenu)
  joueur:sendMessage("ferme " .. tostring(contenu[14] and contenu[14].count))
end)
server.command("boutique", "ouvre la boutique", function(args, joueur)
  if joueur then menu:open(joueur) end
end)
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let expect_text = |c: &mut Client, needle: &str| {
        for _ in 0..20 {
            if String::from_utf8_lossy(&c.wait_for(9)).contains(needle) {
                return;
            }
        }
        panic!("message « {} » jamais reçu", needle);
    };
    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/boutique");
    // Un faux coffre apparaît chez lui, avec le titre de l'inventaire, puis l'écran s'ouvre
    let fake = steve.wait_for(21);
    let named = steve.wait_for(56);
    assert!(String::from_utf8_lossy(&named).contains("Boutique"), "le coffre porte le titre : {:02x?}", named);
    let opened = steve.wait_for(46);
    assert_eq!(&opened[..2], &[2, 0], "fenêtre 2, écran de coffre");
    let content = loop {
        let content = steve.wait_for(49);
        if content[0] == 2 {
            break content;
        }
    };
    assert_eq!(content[1], 27);
    // Prendre les diamants (conteneur 7, case 13) : refusé, et le clic est rapporté au plugin
    let mut request = vec![1, 0x05, 1, 0, 2];
    request.extend_from_slice(&[7, 0, 13, 0, 0, 0, 0]);
    request.extend_from_slice(&[28, 0, 0, 0, 0, 0, 0]);
    request.extend_from_slice(&[0, 0, 0, 0, 0]);
    steve.game(147, &request, Some(0xff));
    assert_eq!(&steve.wait_for(148)[..3], &[1, 1, 0x05], "rien ne sort d'un inventaire verrouillé");
    expect_text(&mut steve, "clic 14 minecraft:diamond 2");
    // Fermeture : le plugin est prévenu (les diamants y sont toujours), le vrai bloc reparaît
    steve.game(47, &[2, 0, 0], Some(0xff));
    expect_text(&mut steve, "ferme 2");
    command(&mut steve, "/boutique");
    let again = steve.wait_for(21);
    assert_eq!(again, fake, "le faux coffre revient à la même place à la réouverture");
    assert_eq!(&steve.wait_for(46)[..2], &[2, 0]);

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
    expect_text(&mut steve, "inconnu : pas_un_bloc");
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
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins").join("EasyEdit");
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                copy_tree(&path, &to.join(entry.file_name()));
            } else {
                std::fs::copy(&path, to.join(entry.file_name())).unwrap();
            }
        }
    }
    copy_tree(&source, &plugin);
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
    expect_text(&mut steve, "inconnu : pas_un_bloc");
    command(&mut steve, "//undo");
    expect_text(&mut steve, "1 blocs annul");
    command(&mut steve, "//undo");
    expect_text(&mut steve, "Rien à annuler");

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

#[test]
fn players_get_hurt_die_and_respawn() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-death-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let mut alex = Client::join(addr, "Alex");
    steve.walk_to(0.5, 0.5);
    alex.walk_to(1.5, 0.5);
    // Alex voit Steve apparaître : son identifiant suit l'UUID et le pseudo
    let seen = alex.wait_for(12);
    let steve_id = seen[16 + 1 + 5];

    // Steve passe en survie (en créatif, on ne prend pas de coups)
    let mut body = vec![18u8];
    body.extend_from_slice(b"/gamemode survival");
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    std::thread::sleep(Duration::from_millis(300));

    // Alex frappe Steve : Steve rougit (événement 2) et perd de la vie
    let mut attack = vec![0x00, 0x03, 0x00, steve_id, 0x01];
    attack.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
    alex.game(30, &attack, Some(0xff));
    loop {
        let event = steve.wait_for(27);
        if event[1] == 2 {
            break;
        }
    }

    // Les quatre modes du jeu : aventure (2) puis spectateur (6), puis retour en survie
    for (mode, id) in [("adventure", 4u8), ("spectator", 12u8), ("survival", 0u8)] {
        let line = format!("/gamemode {}", mode);
        let mut body = vec![line.len() as u8];
        body.extend_from_slice(line.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        steve.game(77, &body, Some(0xff));
        let set = steve.wait_for(62);
        assert_eq!(set, vec![id], "mode {}", mode);
    }

    // Steve reçoit des pierres : elles tomberont au sol à sa mort
    let give = b"/give Steve stone 5";
    let mut body = vec![give.len() as u8];
    body.extend_from_slice(give);
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    std::thread::sleep(Duration::from_millis(300));

    // /kill : écran de mort (Respawn, état « recherche »), puis réapparition sur demande
    let mut body = vec![5u8];
    body.extend_from_slice(b"/kill");
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    // Ses pierres apparaissent au sol
    let dropped = steve.wait_for(15);
    assert!(dropped.len() > 30, "objet au sol");
    let searching = steve.wait_for(45);
    assert_eq!(searching[12], 0, "état « recherche du point de réapparition »");
    let mut ready = searching.clone();
    ready[12] = 2; // le joueur clique sur « Réapparaître »
    steve.game(45, &ready, Some(0xff));
    let back = steve.wait_for(45);
    assert_eq!(back[12], 1, "état « prêt à réapparaître »");
    // Sa vie est remise au maximum
    let attributes = steve.wait_for(29);
    let health_max = 20.0f32.to_le_bytes();
    assert!(attributes.windows(4).filter(|w| *w == health_max).count() >= 2, "vie pleine après la réapparition");
    // Réapparu à l'endroit de sa mort (le point de départ), il ramasse ses pierres
    let taken = steve.wait_for(17);
    // L'identifiant d'exécution suit l'identifiant unique (entier variable) dans l'apparition
    let runtime_at = dropped.iter().position(|b| b & 0x80 == 0).unwrap() + 1;
    assert_eq!(taken[0], dropped[runtime_at], "c'est bien l'objet tombé qui est ramassé");

    drop(steve);
    drop(alex);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugin_listeners_change_and_cancel_what_happens() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-listeners-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("ecouteur.lua"),
        r#"
local Ecouteur = {}
function Ecouteur:onPlayerJoin(event)
  event:setJoinMessage("Salut " .. event:getPlayer():getName() .. " !")
end
function Ecouteur:onPlayerChat(event)
  if event:getMessage():find("interdit") then event:cancel() return end
  event:setMessage(event:getMessage():upper())
end
server.registerEvents(Ecouteur)
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let expect_text = |c: &mut Client, needle: &str| {
        for _ in 0..200 {
            let body = c.wait_for(9);
            if body.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                return;
            }
        }
        panic!("message « {} » jamais reçu", needle);
    };
    expect_text(&mut steve, "Salut Steve !");
    let chat = |c: &mut Client, text: &str| {
        let mut body = vec![0u8, 1, 1];
        body.push(5);
        body.extend_from_slice(b"Steve");
        body.push(text.len() as u8);
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(9, &body, Some(0xff));
    };
    // Le message annulé n'arrive jamais ; le suivant arrive en majuscules
    chat(&mut steve, "mot interdit");
    chat(&mut steve, "bonjour");
    for _ in 0..200 {
        let body = steve.wait_for(9);
        assert!(!body.windows(8).any(|w| w == b"interdit"), "le message annulé a été diffusé");
        if body.windows(7).any(|w| w == b"BONJOUR") {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Deux objets identiques jetés au même endroit avec la touche Q (paquet du jeu 1.26.63) ne
/// font plus qu'un tas une fois au sol.
#[test]
fn identical_items_on_the_ground_merge() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-merge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    steve.walk_to(0.5, 0.5);
    let give = b"/give Steve stone 5";
    let mut body = vec![give.len() as u8];
    body.extend_from_slice(give);
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    std::thread::sleep(Duration::from_millis(300));
    let stone = basalt::bedrock::GIVABLE_BLOCKS.iter().find(|(n, _)| *n == "stone").unwrap().1;
    let item = |count: u16| {
        let mut v = stone.to_le_bytes().to_vec();
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    };
    let air = [0u8; 8];
    for left in [4u16, 3] {
        // Disposition relevée : demande ancienne, transaction normale, l'objet qui apparaît
        // dans le monde puis la case 0 de la barre qui perd un objet
        let mut q = vec![0x02, 0x01, 0x01, 0x1d, 0x01, 0x00, 0x00, 0x02];
        q.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00]);
        q.extend(item(left + 1));
        q.extend(item(left));
        q.extend_from_slice(&[0x02, 0x00, 0x01, 0x00, 0x00]);
        q.extend_from_slice(&air);
        q.extend(item(1));
        steve.game(30, &q, Some(0xff));
        let shown = steve.wait_for(15);
        assert!(shown.len() > 30, "objet jeté visible");
        std::thread::sleep(Duration::from_millis(100));
    }
    // Une fois au sol, l'un des deux disparaît et la pile de l'autre passe à 2 (événement 69)
    steve.wait_for(14);
    loop {
        let event = steve.wait_for(27);
        if event.len() >= 3 && event[1] == 69 {
            assert_eq!(event[2], 4, "nouvelle taille : 2 (entier zigzag)");
            break;
        }
    }

    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Fabrication : un résultat qui correspond à une vraie recette est accepté, un résultat
/// inventé (tricherie) est refusé et la grille reste intacte.
#[test]
fn crafting_is_checked_against_real_recipes() {
    let data = match basalt::data::GameData::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data")) {
        Ok(d) => d,
        Err(_) => return,
    };
    let dir = std::env::temp_dir().join(format!("basalt-e2e-craft-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    steve.walk_to(0.5, 0.5);
    let give = b"/give Steve oak_planks 4";
    let mut body = vec![give.len() as u8];
    body.extend_from_slice(give);
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    std::thread::sleep(Duration::from_millis(300));

    let slot = |container: u8, slot: u8| vec![container, 0, slot, 0, 0, 0, 0];
    let request = |id: u8, actions: Vec<Vec<u8>>| {
        let mut r = vec![1, id * 2, actions.len() as u8];
        for (i, a) in actions.into_iter().enumerate() {
            r.push(i as u8);
            r.extend(a);
        }
        r.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]); // textes, origine
        r
    };
    let place = |count: u8, from: Vec<u8>, to: Vec<u8>| {
        let mut a = vec![1, count];
        a.extend(from);
        a.extend(to);
        a
    };
    let results = |name: &str, count: u16| {
        let mut a = vec![20, 1, 1, 1, name.len() as u8];
        a.extend_from_slice(name.as_bytes());
        a.push(0);
        a.extend_from_slice(&count.to_le_bytes());
        a.push(0); // bloc
        a.push(0); // données
        a.push(1); // fabrications
        a
    };
    let consume = |from: Vec<u8>| {
        let mut a = vec![5, 1];
        a.extend(from);
        a
    };
    let status = |steve: &mut Client| steve.wait_for(148)[1];

    // Deux planches dans la grille 2×2 de l'inventaire, l'une au-dessus de l'autre
    steve.game(147, &request(1, vec![place(1, slot(12, 0), slot(13, 28)), place(1, slot(12, 0), slot(13, 30))]), Some(0xff));
    assert_eq!(status(&mut steve), 0, "planches posées dans la grille");

    // Tricherie : la grille annonce un bloc de diamant
    let cheat = request(2, vec![vec![12, 1, 1], results("minecraft:diamond_block", 64), consume(slot(13, 28)), consume(slot(13, 30)), place(64, slot(60, 50), slot(12, 5))]);
    steve.game(147, &cheat, Some(0xff));
    assert_eq!(status(&mut steve), 1, "résultat inventé refusé");

    // Fabrication honnête : 4 bâtons
    let honest = request(3, vec![vec![12, 1, 1], results("minecraft:stick", 4), consume(slot(13, 28)), consume(slot(13, 30)), place(4, slot(60, 50), slot(12, 5))]);
    steve.game(147, &honest, Some(0xff));
    assert_eq!(status(&mut steve), 0, "bâtons fabriqués");
    let _ = data.item("stick").unwrap();

    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Monde généré : le joueur apparaît sur la terre ferme, au-dessus du niveau de la mer, et
/// reçoit des chunks en relief (plusieurs sous-chunks), générés en arrière-plan.
#[test]
fn generated_world_spawns_players_on_dry_land() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-terrain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("terrain-normal"), "").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &2225i32.to_be_bytes(), None);
    assert_eq!(c.recv_game(false).0, 0x8f);
    c.login(2225, "Steve", false);
    c.wait_for(6);
    c.game(8, &[3, 0, 0], Some(0xff));
    c.wait_for(7);
    c.game(8, &[4, 0, 0], Some(0xff));
    let start_game = c.wait_for(11);
    let y = f32::from_le_bytes([start_game[7], start_game[8], start_game[9], start_game[10]]);
    assert!(y > 62.0 && y < 250.0, "apparition au-dessus de la mer, pas à {}", y);
    c.game(69, &[6, 16], Some(0xff));
    let (mut chunks, mut tall) = (0, 0);
    loop {
        let (id, body) = c.recv_after_state();
        match id {
            58 => {
                chunks += 1;
                // x, z, dimension (un octet chacun près de l'origine), puis le nombre de sous-chunks
                if body[3] > 1 {
                    tall += 1;
                }
            }
            2 => {
                assert_eq!(body, vec![0, 0, 0, 3]);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(chunks, 29, "chunks dans un rayon de 3");
    assert_eq!(tall, chunks, "terrain en relief : plusieurs sous-chunks par chunk");
    assert!(dir.join("world").join("level.dat").exists(), "graine enregistrée dès la création");
    drop(c);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    // Redémarrage : même monde, même graine
    let world = basalt::world::World::stored_generator(&dir.join("world")).unwrap();
    assert_eq!(world, Some((basalt::world::generator::Kind::Vanilla, 42)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Le jeu oublie les chunks qui sortent de sa zone : quand le joueur revient, le serveur
/// doit les renvoyer (sinon le joueur se retrouve devant un trou, bloqué).
#[test]
fn chunks_are_sent_again_when_a_player_comes_back() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-return-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::join(addr, "Steve");
    // Aller jusqu'au chunk 2, par petits pas : les chunks à gauche sortent de la zone (rayon 3)
    let mut x = 0.5f32;
    // (au rythme d'un joueur en créatif, pour ne pas être replacé par le contrôle des déplacements)
    while x < 40.5 {
        x += 0.5;
        c.walk_to(x, 0.5);
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(500));
    c.drain_games(Duration::from_millis(300));
    // Retour au départ
    let mut back = std::collections::HashSet::new();
    let unzig = |b: u8| ((b >> 1) as i32) ^ -((b & 1) as i32);
    while x > 0.5 {
        x -= 0.5;
        c.walk_to(x, 0.5);
        for (id, body) in c.drain_games(Duration::from_millis(20)) {
            if id == 58 && body[0] < 0x80 && body[1] < 0x80 {
                back.insert((unzig(body[0]), unzig(body[1])));
            }
        }
    }
    for (id, body) in c.drain_games(Duration::from_millis(1500)) {
        // x et z en entiers zigzag sur un octet près de l'origine
        if id == 58 && body[0] < 0x80 && body[1] < 0x80 {
            back.insert((unzig(body[0]), unzig(body[1])));
        }
    }
    // Sortis de la zone (oubliés par le jeu), puis de nouveau dedans au retour
    for chunk in [(-1, 2), (-1, -2), (-2, 1), (-3, 0)] {
        assert!(back.contains(&chunk), "chunk {:?} renvoyé au retour (reçus : {:?})", chunk, back);
    }
    drop(c);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Deux mondes ouverts à la fois : un joueur passe dans l'autre, l'autre joueur ne le voit
/// plus, le tchat traverse les mondes, et il revient dans son monde après un redémarrage.
#[test]
fn players_live_in_several_worlds_at_once() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-mondes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mondes.lua"),
        r#"
server.command("creer", "crée le lobby", function(args, joueur)
  server.worlds.load("lobby", "flat", "7")
end)
server.command("aller", "change de monde", function(args, joueur)
  if joueur then joueur:setWorld(args[1]) end
end)
server.command("ou", "dit où l'on est", function(args, joueur)
  if joueur then joueur:sendMessage("monde " .. joueur:getWorld() .. " parmi " .. table.concat(server.worlds.list(), ",")) end
end)
server.command("vide", "crée un monde vide", function(args, joueur)
  server.worlds.load("vide", "void")
end)
server.command("poser", "pose une pierre dans le lobby", function(args, joueur)
  server.getWorld("lobby"):setBlock(3, -50, 3, "minecraft:stone")
end)
server.command("lire", "lit le même bloc dans les deux mondes", function(args, joueur)
  local la, ici = server.getWorld("lobby"):getBlock(3, -50, 3), server.getWorld():getBlock(3, -50, 3)
  joueur:sendMessage("blocs " .. la:getName() .. " / " .. ici:getName() .. " / " .. tostring(server.getWorld("absent")))
end)
server.command("sol", "dit sur quoi l'on se tient", function(args, joueur)
  local x, y, z = joueur:getPosition()
  local sous = server.getWorld(joueur:getWorld()):getBlock(x, y - 1, z)
  joueur:sendMessage("sol " .. sous:getName() .. " hauteur " .. math.floor(y + 0.5))
end)
server.command("crier", "parle à tous", function(args, joueur)
  server.broadcast("cri de " .. joueur.name)
end)
"#,
    )
    .unwrap();
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let expect_text = |c: &mut Client, needle: &str| {
        for _ in 0..30 {
            if String::from_utf8_lossy(&c.wait_for(9)).contains(needle) {
                return;
            }
        }
        panic!("message « {} » jamais reçu", needle);
    };
    std::fs::write(dir.join("ops.txt"), "alex\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let mut alex = Client::join(addr, "Alex");
    std::thread::sleep(Duration::from_millis(600));
    alex.drain_games(Duration::from_millis(300));
    command(&mut steve, "/ou");
    expect_text(&mut steve, "monde world parmi world");
    command(&mut steve, "/creer");
    std::thread::sleep(Duration::from_millis(400));
    steve.drain_games(Duration::from_millis(200));
    command(&mut steve, "/aller lobby");
    // Il est replacé, et reçoit le terrain du nouveau monde
    let got = steve.drain_games(Duration::from_millis(700));
    assert!(got.iter().any(|(id, _)| *id == 19), "replacé dans le nouveau monde");
    let chunks = got.iter().filter(|(id, _)| *id == 58).count();
    assert!(chunks >= 20, "terrain du nouveau monde envoyé ({} chunks)", chunks);
    command(&mut steve, "/ou");
    expect_text(&mut steve, "monde lobby parmi world,lobby");
    // Alex, resté dans le monde principal, ne le voit plus
    let seen = alex.drain_games(Duration::from_millis(400));
    assert!(seen.iter().any(|(id, _)| *id == 14), "Steve a disparu de l'écran d'Alex");
    // …mais un message à tous traverse les mondes
    command(&mut alex, "/crier");
    expect_text(&mut steve, "cri de Alex");
    // Un monde inconnu : rien ne bouge
    command(&mut steve, "/aller nulle-part");
    command(&mut steve, "/ou");
    expect_text(&mut steve, "monde lobby");
    // Un plugin pose et lit des blocs dans le monde qu'il nomme, sans toucher au principal
    command(&mut alex, "/poser");
    std::thread::sleep(Duration::from_millis(300));
    command(&mut alex, "/lire");
    expect_text(&mut alex, "blocs minecraft:stone / minecraft:air / nil");
    // Une commande du jeu atteint un joueur d'un autre monde : /tp y emmène son auteur
    command(&mut alex, "/tp Steve");
    std::thread::sleep(Duration::from_millis(300));
    command(&mut alex, "/ou");
    expect_text(&mut alex, "monde lobby");
    // Un monde vide accueille les joueurs sur une plateforme, pas dans le vide
    command(&mut steve, "/vide");
    std::thread::sleep(Duration::from_millis(400));
    command(&mut steve, "/aller vide");
    std::thread::sleep(Duration::from_millis(400));
    command(&mut steve, "/sol");
    expect_text(&mut steve, "sol minecraft:stone hauteur 64");
    command(&mut steve, "/aller lobby");
    std::thread::sleep(Duration::from_millis(300));
    drop(steve);
    drop(alex);
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    assert!(dir.join("worlds").join("lobby").join("level.dat").exists() && dir.join("worlds").join("lobby").join("db").join("CURRENT").exists(), "le nouveau monde est enregistré dans worlds/, au format du jeu");
    let kept = basalt::player::PlayerData::load(&dir, "Steve").unwrap().unwrap();
    assert_eq!(kept.world, "lobby", "le monde du joueur est enregistré avec lui");

    // Au redémarrage, le monde est rouvert et le joueur y revient
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/ou");
    expect_text(&mut steve, "monde lobby parmi world,lobby,vide");
    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Le monde est rangé comme le fait le jeu (level.dat, db/) : un ancien monde de Basalt y
/// est converti, ce qu'on y construit se relit avec le format du jeu, et un monde venu du
/// jeu, déposé dans worlds/, s'ouvre tel qu'il est.
#[test]
fn worlds_are_saved_the_way_the_game_saves_them() {
    use basalt::world::generator::{Biome, Column, Generator, Kind, Palette};
    use basalt::world::mojang::{Batch, Store};
    let dir = std::env::temp_dir().join(format!("basalt-e2e-format-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let data = basalt::data::GameData::load(std::path::Path::new("data")).unwrap();
    let state = |name: &str| data.default_state(name).unwrap();
    let air = state("minecraft:air");
    let world_dir = dir.join("world");
    // 1. Un monde d'une version précédente : level.nbt, avec un bloc d'or posé
    {
        let mut old = basalt::world::World::with_generator(Generator::new(Kind::Flat, 0, Palette::new(&|n| data.state_from_spec(n))));
        old.set(5, -50, 5, state("minecraft:gold_block"));
        old.save(&world_dir).unwrap();
        assert!(world_dir.join("level.nbt").exists());
    }
    // 2. Un monde venu du jeu, déposé dans worlds/ : un sol d'émeraude à y = 10 dans un chunk
    let game_dir = dir.join("worlds").join("jeu");
    {
        let store = Store::open(&game_dir, data.table.clone(), false).unwrap();
        let emerald = state("minecraft:emerald_block");
        let mut batch = Batch::new();
        let (chest_block, sign_block) = (state("minecraft:chest"), state("minecraft:standing_sign"));
        let block = |x: usize, y: i32, z: usize| match (x, y, z) {
            (_, 10, _) => emerald,
            (5, 11, 5) => chest_block,
            (6, 11, 5) => sign_block,
            _ => air,
        };
        store.encode(0, 0, &block, &|_, _, _| false, &|_, _, _, _| 1, &mut batch);
        // …et un coffre du jeu posé dessus, avec une épée enchantée usée et des pommes
        let tag = basalt::nbt::Tag::compound;
        let chest = tag(vec![
            ("id", basalt::nbt::Tag::string("Chest")),
            ("x", basalt::nbt::Tag::Int(5)),
            ("y", basalt::nbt::Tag::Int(11)),
            ("z", basalt::nbt::Tag::Int(5)),
            ("Findable", basalt::nbt::Tag::Byte(0)),
            (
                "Items",
                basalt::nbt::Tag::List(
                    10,
                    vec![
                        tag(vec![
                            ("Slot", basalt::nbt::Tag::Byte(0)),
                            ("Name", basalt::nbt::Tag::string("minecraft:diamond_sword")),
                            ("Count", basalt::nbt::Tag::Byte(1)),
                            ("Damage", basalt::nbt::Tag::Short(0)),
                            ("tag", tag(vec![("Damage", basalt::nbt::Tag::Int(30)), ("ench", basalt::nbt::Tag::List(10, vec![tag(vec![("id", basalt::nbt::Tag::Short(9)), ("lvl", basalt::nbt::Tag::Short(4))])]))])),
                        ]),
                        tag(vec![("Slot", basalt::nbt::Tag::Byte(13)), ("Name", basalt::nbt::Tag::string("minecraft:apple")), ("Count", basalt::nbt::Tag::Byte(7)), ("Damage", basalt::nbt::Tag::Short(0))]),
                    ],
                ),
            ),
        ]);
        let sign = tag(vec![
            ("id", basalt::nbt::Tag::string("Sign")),
            ("x", basalt::nbt::Tag::Int(6)),
            ("y", basalt::nbt::Tag::Int(11)),
            ("z", basalt::nbt::Tag::Int(5)),
            ("FrontText", tag(vec![("Text", basalt::nbt::Tag::string("Bienvenue\nici"))])),
        ]);
        let mut both = basalt::nbt::write_le(&chest);
        both.extend_from_slice(&basalt::nbt::write_le(&sign));
        Store::encode_block_entities(0, 0, Some(both), &mut batch);
        store.write(batch).unwrap();
        let level = basalt::nbt::Tag::compound(vec![
            ("Time", basalt::nbt::Tag::Long(18_000)),
            ("dodaylightcycle", basalt::nbt::Tag::Byte(0)),
            ("rainLevel", basalt::nbt::Tag::Float(1.0)),
            ("lightningLevel", basalt::nbt::Tag::Float(0.0)),
            ("rainTime", basalt::nbt::Tag::Int(500_000)),
            ("Generator", basalt::nbt::Tag::Int(2)),
            ("RandomSeed", basalt::nbt::Tag::Long(5)),
            ("SpawnX", basalt::nbt::Tag::Int(3)),
            ("SpawnY", basalt::nbt::Tag::Int(11)),
            ("SpawnZ", basalt::nbt::Tag::Int(3)),
            ("LevelName", basalt::nbt::Tag::string("Carte du jeu")),
        ]);
        let body = basalt::nbt::write_le(&level);
        let mut raw = 10i32.to_le_bytes().to_vec();
        raw.extend_from_slice(&(body.len() as i32).to_le_bytes());
        raw.extend_from_slice(&body);
        std::fs::write(game_dir.join("level.dat"), raw).unwrap();
    }
    std::fs::write(
        dir.join("blocs.lua"),
        r#"
local function nom(monde, x, y, z) return server.getWorld(monde):getBlock(x, y, z):getName() end
server.command("lire", "lit les blocs", function(args, joueur)
  joueur:sendMessage("ici " .. nom(nil, 5, -50, 5) .. " " .. nom(nil, 6, -50, 5) .. " " .. nom(nil, 0, -61, 0))
end)
server.command("poser", "pose un bloc", function(args, joueur)
  server.getWorld():setBlock(6, -50, 5, "minecraft:diamond_block")
  server.getWorld("jeu"):setBlock(3, 11, 3, "minecraft:torch")
end)
server.command("jeu", "lit le monde du jeu", function(args, joueur)
  joueur:sendMessage("jeu " .. nom("jeu", 3, 10, 3) .. " " .. nom("jeu", 3, 11, 3) .. " " .. nom("jeu", 40, -61, 40) .. " " .. joueur:getWorld())
end)
server.command("aller", "va dans le monde du jeu", function(args, joueur) joueur:setWorld("jeu") end)
server.command("ou", "hauteur", function(args, joueur)
  local x, y, z = joueur:getPosition()
  joueur:sendMessage("position " .. math.floor(x) .. " " .. math.floor(y + 0.5) .. " " .. math.floor(z))
end)
"#,
    )
    .unwrap();
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let expect_text = |c: &mut Client, needle: &str| {
        let mut seen = Vec::new();
        for _ in 0..30 {
            let text = String::from_utf8_lossy(&c.wait_for(9)).into_owned();
            if text.contains(needle) {
                return;
            }
            seen.push(text);
        }
        panic!("message « {} » jamais reçu (reçus : {:?})", needle, seen);
    };
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/lire");
    expect_text(&mut steve, "ici minecraft:gold_block minecraft:air minecraft:grass_block");
    // Le monde du jeu : ses blocs enregistrés, et du terrain généré là où il n'a rien
    command(&mut steve, "/jeu");
    expect_text(&mut steve, "jeu minecraft:emerald_block minecraft:air minecraft:grass_block world");
    command(&mut steve, "/poser");
    std::thread::sleep(Duration::from_millis(300));
    // On y arrive à son point d'apparition, celui de level.dat
    command(&mut steve, "/aller");
    std::thread::sleep(Duration::from_millis(400));
    command(&mut steve, "/ou");
    expect_text(&mut steve, "position 3 11 3");
    drop(steve);
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();

    // Sur le disque : le format du jeu, et l'ancien fichier mis de côté
    assert!(world_dir.join("level.dat").exists() && world_dir.join("levelname.txt").exists() && world_dir.join("db").join("CURRENT").exists());
    assert!(!world_dir.join("level.nbt").exists() && world_dir.join("level.nbt.ancien").exists(), "l'ancien fichier est converti puis mis de côté");
    let read = |dir: &std::path::Path, native: bool, at: (usize, i32, usize)| {
        let store = Store::open(dir, data.table.clone(), native).unwrap();
        let stored = store.load(0, 0).expect("chunk enregistré");
        let mut column = Column::from_blocks(&vec![air; 256 * 384], air, Biome::Plains);
        stored.apply(&mut column, air);
        data.block_name(column.get(at.0, at.1, at.2)).unwrap_or("?").to_string()
    };
    assert_eq!(read(&world_dir, true, (5, -50, 5)), "minecraft:gold_block");
    assert_eq!(read(&world_dir, true, (6, -50, 5)), "minecraft:diamond_block");
    assert_eq!(read(&world_dir, true, (0, -61, 0)), "minecraft:grass_block", "le chunk est enregistré entier, terrain compris");
    assert_eq!(read(&game_dir, false, (3, 11, 3)), "minecraft:torch");
    assert_eq!(read(&game_dir, false, (9, 10, 9)), "minecraft:emerald_block", "le reste du chunk du jeu est intact");
    let level = basalt::world::mojang::read_level(&game_dir).unwrap().unwrap();
    assert!(level.get("BasaltGenerator").is_none() && level.str_of("LevelName") == Some("jeu"));
    // L'heure arrêtée du monde du jeu, sa pluie et ses règles sont restées celles du jeu
    let number = |key: &str| level.get(key).and_then(|t| t.as_i64());
    assert_eq!((number("Time"), number("dodaylightcycle"), number("doweathercycle")), (Some(18_000), Some(0), Some(1)));
    assert_eq!(level.get("rainLevel").and_then(|t| t.as_f32()), Some(1.0));
    assert!(number("rainTime").map(|t| (490_000..=500_000).contains(&t)).unwrap_or(false), "la pluie garde sa durée restante : {:?}", number("rainTime"));
    assert!(!game_dir.join("time.txt").exists() && !game_dir.join("weather.txt").exists() && !game_dir.join("tiles.nbt").exists(), "plus de fichiers à part : tout est dans level.dat et db/");
    // Le coffre et le panneau du jeu sont toujours dans la base, au format du jeu ; le coffre
    // que le plugin a rempli dans le monde principal y est aussi
    let tiles = |dir: &std::path::Path| -> Vec<basalt::nbt::Tag> { Store::open(dir, data.table.clone(), true).unwrap().block_entities().into_iter().flat_map(|(_, e)| e).collect() };
    let game_tiles = tiles(&game_dir);
    let chest = game_tiles.iter().find(|t| t.str_of("id") == Some("Chest")).expect("coffre du jeu gardé");
    let items = chest.get("Items").and_then(|t| t.as_list()).unwrap();
    let sword = items.iter().find(|i| i.str_of("Name") == Some("minecraft:diamond_sword")).expect("épée gardée");
    let in_tag = sword.get("tag").unwrap();
    assert_eq!(in_tag.get("Damage").and_then(|t| t.as_i64()), Some(30), "usure gardée");
    let ench = &in_tag.get("ench").and_then(|t| t.as_list()).unwrap()[0];
    assert_eq!((ench.get("id").and_then(|t| t.as_i64()), ench.get("lvl").and_then(|t| t.as_i64())), (Some(9), Some(4)), "enchantement gardé");
    assert!(items.iter().any(|i| i.str_of("Name") == Some("minecraft:apple") && i.get("Count").and_then(|t| t.as_i64()) == Some(7) && i.get("Slot").and_then(|t| t.as_i64()) == Some(13)));
    let sign = game_tiles.iter().find(|t| t.str_of("id") == Some("Sign")).expect("panneau du jeu gardé");
    assert_eq!(sign.get("FrontText").and_then(|f| f.str_of("Text")), Some("Bienvenue\nici"));

    // Au redémarrage, tout est relu depuis la base
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/lire");
    expect_text(&mut steve, "ici minecraft:gold_block minecraft:diamond_block minecraft:grass_block");
    command(&mut steve, "/jeu");
    expect_text(&mut steve, "jeu minecraft:emerald_block minecraft:torch minecraft:grass_block jeu");
    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `/locate` : la réponse (calculée sur un fil à part) arrive dans le chat du joueur.
#[test]
fn locate_answers_in_the_chat() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-locate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("terrain-normal"), "").unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let mut steve = Client::join(addr, "Steve");
    let expect = |c: &mut Client, wanted: &str| {
        for _ in 0..40 {
            let body = c.wait_for(9);
            if String::from_utf8_lossy(&body).contains(wanted) {
                return;
            }
        }
        panic!("réponse attendue : {}", wanted);
    };
    command(&mut steve, "/locate structure village");
    expect(&mut steve, "Le village le plus proche se trouve en");
    command(&mut steve, "/locate biome desert");
    expect(&mut steve, "Le biome desert le plus proche");
    command(&mut steve, "/locate structure trial_chambers");
    expect(&mut steve, "Les chambres des épreuves les plus proches se trouvent en");
    command(&mut steve, "/locate structure stronghold");
    expect(&mut steve, "ne génère pas encore");
    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un monde créé avec le générateur d'un plugin (`level-type=plugin:archipel`) : le joueur
/// apparaît sur une île, et le type de terrain est enregistré.
#[test]
fn a_plugin_generator_shapes_the_world() {
    if !PluginManager::new(std::path::Path::new("plugins")).available() {
        return; // pas de Lua sur cette machine
    }
    let dir = std::env::temp_dir().join(format!("basalt-e2e-plugingen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let gens = dir.join("Archipel").join("generators");
    std::fs::create_dir_all(&gens).unwrap();
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/Archipel/generators/archipel.lua"), gens.join("archipel.lua")).unwrap();
    for f in ["plugin.yml", "main.lua"] {
        std::fs::copy(format!("{}/plugins/Archipel/{}", env!("CARGO_MANIFEST_DIR"), f), dir.join("Archipel").join(f)).unwrap();
    }
    std::fs::write(dir.join("terrain-type"), "plugin:archipel").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &2225i32.to_be_bytes(), None);
    assert_eq!(c.recv_game(false).0, 0x8f);
    c.login(2225, "Steve", false);
    c.wait_for(6);
    c.game(8, &[3, 0, 0], Some(0xff));
    c.wait_for(7);
    c.game(8, &[4, 0, 0], Some(0xff));
    let start_game = c.wait_for(11);
    let y = f32::from_le_bytes([start_game[7], start_game[8], start_game[9], start_game[10]]);
    assert!(y > 63.0 && y < 200.0, "apparition sur une île, pas à {}", y);
    c.game(69, &[6, 16], Some(0xff));
    let mut chunks = 0;
    loop {
        let (id, _) = c.recv_after_state();
        match id {
            58 => chunks += 1,
            2 => break,
            _ => {}
        }
    }
    assert_eq!(chunks, 29, "chunks dans un rayon de 3");
    drop(c);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let level = basalt::world::mojang::read_level(&dir.join("world")).unwrap().unwrap();
    assert_eq!(level.str_of("BasaltGenerator"), Some("plugin:archipel"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// PNJ d'un plugin (`server.spawnNpc`, `server.walkNpc`) : les joueurs les voient
/// apparaître, avancer chacun dans sa direction, puis disparaître.
#[test]
fn plugin_npcs_appear_walk_and_disappear() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-npc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let plugin = dir.join("Pnj");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("plugin.yml"), "name: Pnj\nversion: 1.0.0\nmain: main.lua\napi: 1\n").unwrap();
    std::fs::write(
        plugin.join("main.lua"),
        r#"
server.command("pnj", "test", function(args, joueur)
  if args[1] == "stop" then server.removeNpcs() return end
  server.spawnNpc("Bot1", 0.5, -60, 0.5)
  server.spawnNpc("Bot2", 0.5, -60, 0.5)
  server.walkNpc("Bot1", 2, 0)
  server.walkNpc("Bot2", -2, 0)
end)
"#,
    )
    .unwrap();
    if !PluginManager::new(&dir).available() {
        return;
    }
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(800));
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let mut steve = Client::join(addr, "Steve");
    command(&mut steve, "/pnj");
    // Les deux PNJ apparaissent, sous leur nom
    let mut ids = Vec::new();
    while ids.len() < 2 {
        let body = steve.wait_for(12);
        for name in [&b"Bot1"[..], &b"Bot2"[..]] {
            if body.windows(4).any(|w| w == name) {
                ids.push(body[16 + 1 + 4]);
            }
        }
    }
    // Ils avancent : des déplacements arrivent pour chacun d'eux
    let mut moved = std::collections::HashSet::new();
    let started = std::time::Instant::now();
    while moved.len() < 2 && started.elapsed() < Duration::from_secs(10) {
        let (id, body) = steve.recv_game(true);
        if (id == 18 || id == 19) && ids.contains(&body[0]) {
            moved.insert(body[0]);
        }
    }
    assert_eq!(moved.len(), 2, "les deux PNJ doivent bouger");
    // Retirés : ils disparaissent
    command(&mut steve, "/pnj stop");
    let mut gone = 0;
    let started = std::time::Instant::now();
    while gone < 2 && started.elapsed() < Duration::from_secs(10) {
        let (id, body) = steve.recv_game(true);
        if id == 14 && ids.iter().any(|i| body == vec![*i * 2]) {
            gone += 1;
        }
    }
    assert_eq!(gone, 2, "les PNJ retirés disparaissent");
    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Plugin privé Bots (absent du dépôt public) : `/bots 4` fait apparaître quatre bots.
/// `cargo test --test e2e bots_plugin -- --ignored`
#[test]
#[ignore]
fn bots_plugin() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins").join("Bots");
    if !source.join("plugin.yml").is_file() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("basalt-e2e-bots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Bots")).unwrap();
    for f in ["plugin.yml", "main.lua"] {
        std::fs::copy(source.join(f), dir.join("Bots").join(f)).unwrap();
    }
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(800));
    let mut steve = Client::join(addr, "Steve");
    let text = "/bots 4 4 10";
    let mut body = vec![text.len() as u8];
    body.extend_from_slice(text.as_bytes());
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    let mut names = std::collections::HashSet::new();
    let started = std::time::Instant::now();
    while names.len() < 4 && started.elapsed() < Duration::from_secs(10) {
        let (id, body) = steve.recv_game(true);
        if id == 12 {
            for n in ["Bot1", "Bot2", "Bot3", "Bot4"] {
                if body.windows(4).any(|w| w == n.as_bytes()) {
                    names.insert(n);
                }
            }
        }
    }
    assert_eq!(names.len(), 4, "quatre bots visibles");
    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Coût des PNJ sur un vrai terrain : 50 bots qui partent loin, terrain généré à la volée.
/// `cargo test --test e2e bots_cost -- --ignored --nocapture`
#[test]
#[ignore]
fn bots_cost() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins").join("Bots");
    if !source.join("plugin.yml").is_file() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("basalt-e2e-botscost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Bots")).unwrap();
    for f in ["plugin.yml", "main.lua"] {
        std::fs::copy(source.join(f), dir.join("Bots").join(f)).unwrap();
    }
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    std::fs::write(dir.join("terrain-normal"), "").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(800));
    let mut steve = Client::join(addr, "Steve");
    basalt::timings::enable();
    let text = "/bots 50 10 400";
    let mut body = vec![text.len() as u8];
    body.extend_from_slice(text.as_bytes());
    body.extend_from_slice(&[0, 0, 0]);
    steve.game(77, &body, Some(0xff));
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(100));
    }
    for line in basalt::timings::report(&[]).unwrap_or_default().lines().filter(|l| l.contains("PNJ") || l.contains("Tick complet")) {
        eprintln!("{}", line);
    }
    drop(steve);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Chat : les opérateurs écrivent en couleur (& ou §), les autres non ; les messages partent
/// comme messages du serveur, que le jeu ne passe pas à son filtre de grossièretés.
#[test]
fn operators_write_in_colour() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-couleurs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let mut alex = Client::join(addr, "Alex");
    let chat = |c: &mut Client, who: &str, text: &str| {
        let mut body = vec![0u8, 1, 1];
        body.push(who.len() as u8);
        body.extend_from_slice(who.as_bytes());
        let bytes = text.as_bytes();
        body.push(bytes.len() as u8);
        body.extend_from_slice(bytes);
        body.extend_from_slice(&[0, 0, 0]);
        c.game(9, &body, Some(0xff));
    };
    let expect = |c: &mut Client, needle: &str| {
        for _ in 0..300 {
            let body = c.wait_for(9);
            if body.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                assert_eq!(body[2], 0, "message envoyé comme texte du serveur (type 0)");
                return;
            }
        }
        panic!("message « {} » jamais reçu", needle);
    };
    chat(&mut steve, "Steve", "&aSalut §ctout le monde, R&D");
    expect(&mut steve, "<Steve§r> §aSalut §ctout le monde, R&D");
    chat(&mut alex, "Alex", "&aSalut §ctoi");
    expect(&mut steve, "<Alex§r> &aSalut ctoi");
    drop(steve);
    drop(alex);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Des animaux apparaissent sur l'herbe autour du joueur, et se promènent.
#[test]
fn animals_appear_and_wander() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-animaux-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("animaux"), "").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let started = std::time::Instant::now();
    let mut animal = None;
    let mut moved = false;
    // (Large délai : machine chargée par les autres tests ; la boucle s'arrête dès que c'est vu)
    while (animal.is_none() || !moved) && started.elapsed() < Duration::from_secs(90) {
        if let Some((id, body)) = steve.poll_game(Duration::from_millis(500)) {
            if id == 13 {
                for kind in ["minecraft:cow", "minecraft:pig", "minecraft:sheep", "minecraft:chicken"] {
                    if body.windows(kind.len()).any(|w| w == kind.as_bytes()) {
                        animal = Some(body[0]);
                    }
                }
            }
            // Il bouge tôt ou tard (promenade)
            if id == 18 && animal.is_some() {
                moved = true;
            }
        }
    }
    assert!(animal.is_some(), "un animal doit apparaître");
    assert!(moved, "les animaux se déplacent");
    // Enregistrés avec le monde à l'arrêt du serveur, puis rechargés
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    // Rangés par région (entities/r.<x>.<z>.json), comme le jeu les range avec leur chunk
    let saved: String = std::fs::read_dir(dir.join("world").join("entities"))
        .expect("dossier entities créé")
        .filter_map(|e| e.ok())
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect();
    assert!(saved.contains("minecraft:") && saved.contains("\"populated\""), "{}", &saved[..saved.len().min(200)]);
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut alex = Client::join(addr, "Alex");
    let started = std::time::Instant::now();
    let mut again = false;
    while !again && started.elapsed() < Duration::from_secs(20) {
        if let Some((id, _)) = alex.poll_game(Duration::from_millis(500)) {
            again = id == 13;
        }
    }
    assert!(again, "les animaux enregistrés reviennent");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
}

/// L'ancien fichier unique (`mobs.json`) est converti en régions, et ses animaux reviennent.
#[test]
fn old_mobs_file_is_converted() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-ancien-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("animaux"), "").unwrap();
    std::fs::write(dir.join("deplacements-libres"), "").unwrap();
    // Un lapin (jamais tiré au sort en terrain plat) près du point d'apparition, une autre très loin (région à part)
    std::fs::write(
        dir.join("world").join("mobs.json"),
        r#"{"mobs":[{"kind":"minecraft:rabbit","x":2.5,"y":-60,"z":2.5},{"kind":"minecraft:pig","x":5000.5,"y":-60,"z":5000.5}],"chunks":["0,0"]}"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let started = std::time::Instant::now();
    let mut rabbit = false;
    while !rabbit && started.elapsed() < Duration::from_secs(20) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(500)) {
            rabbit |= body.windows(16).any(|w| w == b"minecraft:rabbit");
        }
    }
    // (Le cochon lointain, lui, reste dans sa région : vérifié dans le fichier à la fin. Des
    // troupeaux neufs peuvent apparaître autour du joueur : on ne compte pas les cochons.)
    assert!(rabbit, "le lapin convertie revient près du joueur");
    // Le joueur s'éloigne : le lapin retourne dans sa région (le jeu la retire) ; il revient : elle aussi
    // (Rend vrai si un lapin est apparue en chemin)
    let walk = |c: &mut Client, from: f32, to: f32| {
        let steps = ((to - from).abs() / 4.0).ceil() as i32;
        let mut seen = false;
        for i in 1..=steps {
            let x = from + (to - from) * i as f32 / steps as f32;
            c.walk_to(x, 0.5);
            if let Some((13, body)) = c.poll_game(Duration::from_millis(50)) {
                seen |= body.windows(16).any(|w| w == b"minecraft:rabbit");
            }
        }
        seen
    };
    // (Loin, le lapin quitte la mémoire : il n'est plus montré ; il ne peut être renvoyé au
    // retour que s'il a été rangé puis ressorti de sa région)
    walk(&mut steve, 0.5, 300.5);
    for _ in 0..6 {
        let _ = steve.poll_game(Duration::from_millis(500));
    }
    let mut back = walk(&mut steve, 300.5, 0.5);
    let started = std::time::Instant::now();
    while !back && started.elapsed() < Duration::from_secs(15) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(500)) {
            back |= body.windows(16).any(|w| w == b"minecraft:rabbit");
        }
    }
    assert!(back, "le lapin revient avec le joueur");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let world = dir.join("world");
    assert!(!world.join("mobs.json").exists() && world.join("mobs.json.ancien").exists());
    let far = std::fs::read_to_string(world.join("entities").join("r.9.9.json")).expect("région du cochon");
    assert!(far.contains("minecraft:pig"), "{}", far);
    let near = std::fs::read_to_string(world.join("entities").join("r.0.0.json")).expect("région de le lapin");
    assert!(near.contains("minecraft:rabbit") && near.contains("\"0,0\""), "{}", near);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `/summon` fait apparaître un animal (même sans animaux naturels), et refuse les inconnus.
#[test]
fn summon_makes_an_animal_appear() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-summon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let wait_for = |c: &mut Client, text: Option<&str>, kind: Option<&[u8]>| {
        let (mut said, mut shown) = (text.is_none(), kind.is_none());
        let started = std::time::Instant::now();
        while !(said && shown) && started.elapsed() < Duration::from_secs(10) {
            if let Some((id, body)) = c.poll_game(Duration::from_millis(300)) {
                said |= id == 9 && text.map(|t| String::from_utf8_lossy(&body).contains(t)).unwrap_or(false);
                shown |= id == 13 && kind.map(|k| body.windows(k.len()).any(|w| w == k)).unwrap_or(false);
            }
        }
        (said, shown)
    };
    command(&mut steve, "/summon minecraft:horse ~2 ~ ~2 minecraft:entity_born");
    assert_eq!(wait_for(&mut steve, Some("Objet invoqué : horse"), Some(b"minecraft:horse")), (true, true));
    command(&mut steve, "/summon mooshroom");
    assert_eq!(wait_for(&mut steve, Some("Objet invoqué : mooshroom"), Some(b"minecraft:mooshroom")), (true, true));
    command(&mut steve, "/summon licorne");
    assert_eq!(wait_for(&mut steve, Some("ne sait pas encore"), None), (true, true));
    // En selle : un clic droit sur un âne adulte, main vide, le lie au joueur (SetActorLink,
    // type 1) ; « descendre » (Interact, action 3) le délie (type 0)
    command(&mut steve, "/summon donkey ~-2 ~ ~-2");
    let mut donkey = None;
    let started = std::time::Instant::now();
    while donkey.is_none() && started.elapsed() < Duration::from_secs(10) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(300)) {
            if body.windows(16).any(|w| w == b"minecraft:donkey") {
                donkey = Some(body[1]);
            }
        }
    }
    let donkey = donkey.expect("l'âne invoqué est montré");
    let mut click = vec![0x00, 0x03, 0x00, donkey, 0x00];
    click.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
    steve.game(30, &click, Some(0xff));
    let link = |c: &mut Client| {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if let Some((41, body)) = c.poll_game(Duration::from_millis(300)) {
                return Some(body);
            }
        }
        None
    };
    let on = link(&mut steve).expect("lien de monture reçu");
    assert_eq!((on[0], on[2]), (donkey * 2, 1), "monture, puis type « cavalier » : {:?}", on);
    steve.game(33, &[3, donkey, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], Some(0xff));
    let off = link(&mut steve).expect("lien retiré à la descente");
    assert_eq!(off[2], 0, "type « retiré » : {:?}", off);
    // Laisse : une vache attachée au joueur (données « tenue par » mises à jour), puis
    // détachée main vide (la laisse tombe par terre)
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/give @s lead");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/summon cow ~1 ~ ~-3");
    let mut cow = None;
    let started = std::time::Instant::now();
    while cow.is_none() && started.elapsed() < Duration::from_secs(10) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(300)) {
            if body.windows(13).any(|w| w == b"minecraft:cow") {
                cow = Some(body[1]);
            }
        }
    }
    let cow = cow.expect("vache montrée");
    // (La laisse est dans le premier emplacement, celui qu'il tient)
    let mut click = vec![0x00, 0x03, 0x00, cow, 0x00];
    click.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
    steve.game(30, &click, Some(0xff));
    let wait = |c: &mut Client, id: u32, first: u8| {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if let Some((got, body)) = c.poll_game(Duration::from_millis(300)) {
                if got == id && body.first() == Some(&first) {
                    return true;
                }
            }
        }
        false
    };
    assert!(wait(&mut steve, 39, cow), "la vache est tenue en laisse (données mises à jour)");
    steve.game(30, &click, Some(0xff));
    assert!(wait(&mut steve, 39, cow), "détachée");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Parti en selle : ses données (objets, position) sont gardées, debout à la place de sa monture.
#[test]
fn leaving_while_riding_keeps_player_data() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-selle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let mut alex = Client::join(addr, "Alex");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/give @s diamond 5");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/summon donkey ~-2 ~ ~-2");
    let mut donkey = None;
    let started = std::time::Instant::now();
    while donkey.is_none() && started.elapsed() < Duration::from_secs(10) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(300)) {
            if body.windows(16).any(|w| w == b"minecraft:donkey") {
                donkey = Some(body[1]);
            }
        }
    }
    let donkey = donkey.expect("âne montré");
    let mut click = vec![0x00, 0x03, 0x00, donkey, 0x00];
    click.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
    steve.game(30, &click, Some(0xff));
    let started = std::time::Instant::now();
    let mut linked = false;
    while !linked && started.elapsed() < Duration::from_secs(5) {
        linked = matches!(steve.poll_game(Duration::from_millis(300)), Some((41, _)));
    }
    assert!(linked, "en selle");
    // Son jeu annonce une position sous la monture (comme en selle) : Alex le voit pourtant
    // assis sur l'âne (posé en y = -60, siège à 2,175 au-dessus)
    steve.walk_to(-1.5, -1.5);
    let started = std::time::Instant::now();
    let mut seated = false;
    while !seated && started.elapsed() < Duration::from_secs(5) {
        steve.walk_to(-1.5, -1.5);
        if let Some((18, body)) = alex.poll_game(Duration::from_millis(100)) {
            let y = f32::from_le_bytes([body[6], body[7], body[8], body[9]]);
            seated = (y - (-60.0 + 2.175)).abs() < 0.05;
        }
    }
    assert!(seated, "Alex voit Steve sur le siège de l'âne");
    // Puis il part (son jeu le fait d'abord descendre, comme en vrai)
    steve.walk_to(3.5, 3.5);
    std::thread::sleep(Duration::from_millis(300));
    steve.game(33, &[3, donkey, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], Some(0xff));
    std::thread::sleep(Duration::from_millis(200));
    steve.send(&[0x15]);
    std::thread::sleep(Duration::from_millis(1500));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let saved = basalt::player::PlayerData::load(&dir, "Steve").expect("fichier lisible").expect("Steve enregistré");
    // Debout à la place de l'âne (invoqué en -1,5 ; -60 ; -1,5, il a pu faire quelques pas
    // avant qu'on monte), pas là où son jeu le disait (3,5 ; 3,5)
    let p = saved.human.pos;
    assert!((p.0 + 1.5).abs() < 2.0 && (p.2 + 1.5).abs() < 2.0 && (p.1 - (-60.0 + 1.62)).abs() < 0.6, "position enregistrée : {:?}", p);
    assert!(saved.items.iter().any(|i| i.2.contains("diamond")), "ses objets sont gardés : {:?}", saved.items);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Deux chevaux apprivoisés nourris d'une carotte dorée ont un poulain.
#[test]
fn tamed_horses_have_a_foal() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-poulain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/give @s golden_carrot 4");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/summon horse ~2 ~ ~2 minecraft:on_tame");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/summon horse ~-2 ~ ~2 minecraft:on_tame");
    let mut horses: Vec<u8> = Vec::new();
    let started = std::time::Instant::now();
    while horses.len() < 2 && started.elapsed() < Duration::from_secs(10) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(300)) {
            if body.windows(15).any(|w| w == b"minecraft:horse") && !horses.contains(&body[1]) {
                horses.push(body[1]);
            }
        }
    }
    assert_eq!(horses.len(), 2, "deux chevaux montrés");
    for h in &horses {
        let mut click = vec![0x00, 0x03, 0x00, *h, 0x00];
        click.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        steve.game(30, &click, Some(0xff));
        std::thread::sleep(Duration::from_millis(200));
    }
    let mut foal = false;
    let started = std::time::Instant::now();
    while !foal && started.elapsed() < Duration::from_secs(40) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(300)) {
            foal |= body.windows(15).any(|w| w == b"minecraft:horse") && !horses.contains(&body[1]);
        }
    }
    assert!(foal, "un poulain est né");
    // Né apprivoisé : enregistré comme tel à l'arrêt
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let saved: String = std::fs::read_dir(dir.join("world").join("entities")).unwrap().filter_map(|e| e.ok()).filter_map(|e| std::fs::read_to_string(e.path()).ok()).collect();
    let tamed = saved.matches("\"tamed\":true").count();
    assert_eq!(tamed, 3, "les deux parents et le poulain sont apprivoisés : {}", saved);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cheval sauvage apprivoisé en le montant (comme ceux qui apparaissent dans la nature), puis
/// nourri d'une carotte dorée avec un autre cheval apprivoisé : un poulain naît.
#[test]
fn horses_tamed_by_riding_have_a_foal() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-dresses-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    // (Le cheval sauvage gambade avec son cavalier : le joueur le rejoint ensuite d'un pas)
    std::fs::write(dir.join("deplacements-libres"), "").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
        // (Les commandes trop rapprochées sont limitées)
        std::thread::sleep(Duration::from_millis(1300));
    };
    let click = |c: &mut Client, h: u8| {
        let mut click = vec![0x00, 0x03, 0x00, h, 0x00];
        click.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        c.game(30, &click, Some(0xff));
    };
    // Positions annoncées des chevaux ; nouveaux chevaux vus ; événements (cœurs, refus)
    let mut seen: std::collections::HashMap<u8, (f32, f32)> = std::collections::HashMap::new();
    let mut horses: Vec<u8> = Vec::new();
    let mut here = (0.5f32, 0.5f32);
    let mut last_step = std::time::Instant::now();
    // Écoute un moment (et continue d'envoyer ses mouvements, sinon le serveur le croit figé) ;
    // rend les événements reçus (cheval, numéro)
    let mut listen = |c: &mut Client, ms: u64, seen: &mut std::collections::HashMap<u8, (f32, f32)>, horses: &mut Vec<u8>, here: (f32, f32)| {
        let mut events = Vec::new();
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_millis(ms) {
            match c.poll_game(Duration::from_millis(50)) {
                Some((13, body)) if body.windows(15).any(|w| w == b"minecraft:horse") => {
                    if !horses.contains(&body[1]) {
                        horses.push(body[1]);
                    }
                }
                Some((18, body)) => {
                    let f = |i: usize| f32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]);
                    seen.insert(body[0], (f(2), f(10)));
                }
                Some((27, body)) if body.len() > 1 => events.push((body[0], body[1])),
                _ => {}
            }
            if last_step.elapsed() > Duration::from_secs(1) {
                c.walk_to(here.0, here.1);
                last_step = std::time::Instant::now();
            }
        }
        events
    };
    command(&mut steve, "/summon horse ~2 ~ ~2");
    listen(&mut steve, 1500, &mut seen, &mut horses, here);
    let wild = *horses.first().expect("cheval montré");
    // Amadoué : dix pommes dorées (patience au maximum), puis monté jusqu'au dressage
    command(&mut steve, "/give @s golden_apple 10");
    for _ in 0..10 {
        click(&mut steve, wild);
        std::thread::sleep(Duration::from_millis(60));
    }
    command(&mut steve, "/clear @s");
    click(&mut steve, wild);
    let started = std::time::Instant::now();
    let mut tamed = false;
    // Dernière nouvelle du dressage : sans rien pendant huit secondes (un essai en prend cinq
    // au plus), c'est que le clic n'a pas mis le joueur en selle (machine chargée, cheval
    // parti se promener) : on le rejoint et on remonte
    let mut news = std::time::Instant::now();
    while !tamed && started.elapsed() < Duration::from_secs(90) {
        if news.elapsed() > Duration::from_secs(8) {
            if let Some((x, z)) = seen.get(&wild) {
                here = (*x + 1.0, *z);
                steve.walk_to(here.0, here.1);
            }
            click(&mut steve, wild);
            news = std::time::Instant::now();
        }
        for (h, event) in listen(&mut steve, 200, &mut seen, &mut horses, here) {
            tamed |= h == wild && event == 7;
            if h == wild && (event == 6 || event == 7) {
                news = std::time::Instant::now();
            }
            if h == wild && event == 6 {
                // Jeté à terre : on le rejoint et on remonte
                if let Some((x, z)) = seen.get(&wild) {
                    here = (*x + 1.0, *z);
                    steve.walk_to(here.0, here.1);
                }
                click(&mut steve, wild);
            }
        }
    }
    assert!(tamed, "cheval apprivoisé en le montant");
    steve.game(33, &[3, wild, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], Some(0xff));
    listen(&mut steve, 1000, &mut seen, &mut horses, here);
    // Un second cheval, apprivoisé, juste à côté du premier
    let (x, z) = seen.get(&wild).copied().unwrap_or((2.5, 2.5));
    here = (x + 1.0, z);
    steve.walk_to(here.0, here.1);
    command(&mut steve, &format!("/summon horse {:.1} -60 {:.1} minecraft:on_tame", x + 1.5, z));
    listen(&mut steve, 1500, &mut seen, &mut horses, here);
    let other = *horses.iter().find(|h| **h != wild).expect("second cheval montré");
    command(&mut steve, "/give @s golden_carrot 4");
    for h in [wild, other] {
        click(&mut steve, h);
        std::thread::sleep(Duration::from_millis(200));
    }
    let started = std::time::Instant::now();
    let mut foal = false;
    while !foal && started.elapsed() < Duration::from_secs(60) {
        listen(&mut steve, 300, &mut seen, &mut horses, here);
        foal = horses.iter().any(|h| *h != wild && *h != other);
    }
    assert!(foal, "un poulain est né");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Une vache attirée par du blé franchit un muret d'un bloc (elle saute la marche).
#[test]
fn animals_climb_one_block() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-marche-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    // Un muret d'un bloc de haut entre la vache et le joueur, qui tient du blé
    command(&mut steve, "/fill -6 -60 4 6 -60 4 stone");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/give @s wheat");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/summon cow 0.5 -60 7.5");
    let mut cow = None;
    let mut crossed = false;
    let started = std::time::Instant::now();
    while !crossed && started.elapsed() < Duration::from_secs(20) {
        if let Some((id, body)) = steve.poll_game(Duration::from_millis(300)) {
            if id == 13 && body.windows(13).any(|w| w == b"minecraft:cow") {
                cow = Some(body[1]);
            }
            if id == 18 && cow.is_some() && body.first() == cow.as_ref() {
                let z = f32::from_le_bytes([body[10], body[11], body[12], body[13]]);
                crossed = z < 4.0;
            }
        }
    }
    assert!(cow.is_some(), "vache montrée");
    assert!(crossed, "la vache a sauté le muret");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Dans l'eau : une vache flotte, le corps dans l'eau et la tête dehors ; un zombie coule et
/// se retrouve au fond (où il finira par se noyer).
#[test]
fn animals_float_and_zombies_sink() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-eau-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    // Un bassin de pierre rempli d'eau sur cinq blocs de profondeur : le fond est à y = -60,
    // la surface de l'eau à y = -54
    command(&mut steve, "/fill 2 -60 2 8 -55 8 stone");
    std::thread::sleep(Duration::from_millis(600));
    command(&mut steve, "/fill 3 -60 3 7 -55 7 water");
    std::thread::sleep(Duration::from_millis(600));
    command(&mut steve, "/summon cow 4.5 -54 4.5");
    command(&mut steve, "/summon zombie 6.5 -54 6.5");
    let (mut cow, mut zombie) = (None, None);
    let (mut cow_y, mut zombie_y) = (Vec::new(), Vec::new());
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(8) {
        if let Some((id, body)) = steve.poll_game(Duration::from_millis(200)) {
            if id == 13 && body.windows(13).any(|w| w == b"minecraft:cow") {
                cow = Some(body[1]);
            }
            if id == 13 && body.windows(16).any(|w| w == b"minecraft:zombie") {
                zombie = Some(body[1]);
            }
            if id == 18 && body.len() >= 14 {
                let y = f32::from_le_bytes([body[6], body[7], body[8], body[9]]);
                if body.first() == cow.as_ref() {
                    cow_y.push(y);
                } else if body.first() == zombie.as_ref() {
                    zombie_y.push(y);
                }
            }
        }
    }
    assert!(cow.is_some() && zombie.is_some(), "vache et zombie montrés");
    let zombie_last = zombie_y.last().copied().unwrap_or(-54.0);
    assert!(zombie_last < -59.5, "le zombie a coulé et reste au fond, tête sous l'eau (y = {})", zombie_last);
    // (surface à -54 ; une vache mesure 1,3 bloc : plus de la moitié sous l'eau, la tête dehors.
    // Elle peut ensuite nager jusqu'au bord et s'y hisser.)
    let lowest = cow_y.iter().cloned().fold(f32::MAX, f32::min);
    let floating = cow_y.iter().filter(|y| (-54.95..=-54.5).contains(*y)).count();
    assert!(lowest > -55.0, "la vache ne coule pas (au plus bas : {})", lowest);
    assert!(floating >= 15, "la vache flotte le corps dans l'eau, pas posée sur la surface ({} positions à cette profondeur : {:?})", floating, cow_y);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Âne apprivoisé : un coffre posé sur son dos, puis, en selle, la touche d'inventaire ouvre
/// le sien (écran de monture, selle et armure plus 15 cases de coffre). Un mouton teint
/// change de couleur.
#[test]
fn donkey_chest_and_dyed_sheep() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-coffre-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    let find = |c: &mut Client, kind: &[u8]| {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            if let Some((13, body)) = c.poll_game(Duration::from_millis(300)) {
                if body.windows(kind.len()).any(|w| w == kind) {
                    return Some(body[1]);
                }
            }
        }
        None
    };
    let wait = |c: &mut Client, id: u32| {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if let Some((got, body)) = c.poll_game(Duration::from_millis(200)) {
                if got == id {
                    return Some(body);
                }
            }
        }
        None
    };
    let click = |c: &mut Client, h: u8| {
        let mut click = vec![0x00, 0x03, 0x00, h, 0x00];
        click.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        c.game(30, &click, Some(0xff));
    };
    // Mouton teint en rouge (couleur 14)
    command(&mut steve, "/summon sheep ~2 ~ ~2");
    let sheep = find(&mut steve, b"minecraft:sheep").expect("mouton montré");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/give @s red_dye");
    std::thread::sleep(Duration::from_millis(800));
    click(&mut steve, sheep);
    let data = wait(&mut steve, 39).expect("données du mouton mises à jour");
    assert_eq!(data[0], sheep);
    assert!(data.windows(4).any(|w| w == [3, 0, 0, 14]), "couleur 14 (rouge) : {:?}", data);
    // Âne apprivoisé, coffre sur le dos
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/clear @s");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/summon donkey ~-2 ~ ~2 minecraft:on_tame");
    let donkey = find(&mut steve, b"minecraft:donkey").expect("âne montré");
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/give @s chest");
    std::thread::sleep(Duration::from_millis(800));
    click(&mut steve, donkey);
    assert!(wait(&mut steve, 39).is_some(), "coffre posé (indicateurs mis à jour)");
    // En selle (main vide), puis la touche d'inventaire
    std::thread::sleep(Duration::from_millis(1200));
    command(&mut steve, "/clear @s");
    std::thread::sleep(Duration::from_millis(800));
    click(&mut steve, donkey);
    assert!(wait(&mut steve, 41).is_some(), "en selle");
    steve.game(33, &[6, donkey, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], Some(0xff));
    // Comme le jeu officiel : le paquet des cases d'équipement ouvre l'écran à lui seul
    let equip = wait(&mut steve, 81).expect("cases d'équipement envoyées");
    assert_eq!((equip[0], equip[1]), (2, 12), "fenêtre 2, écran de monture : {:?}", &equip[..4]);
    assert!(equip.windows(16).any(|w| w == b"minecraft:saddle"), "la selle parmi les objets acceptés");
    // Âne avec coffre : une seule liste, la selle en case 0 puis les 15 cases du coffre
    let content = wait(&mut steve, 49).expect("contenu envoyé");
    assert_eq!((content[0], content[1]), (2, 16), "selle et 15 cases de coffre : {:?}", &content[..4]);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugin_items_are_declared_to_the_game() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-custom-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rubis.lua"),
        r#"
server.items.create("e2e:rubis", { name = "Rubis du test", icon = "emerald", maxStack = 16, category = "nature", group = "Gemmes du test" })
server.items.create("e2e:secret", { name = "Secret", creative = false })
server.blocks.create("e2e:pousse", { stages = { "wheat_stage_0", "wheat_stage_7" }, solid = false, transparent = true, needsFloor = true })
server.blocks.create("e2e:tronc", { texture = "log_oak", rotation = "axis" })
server.blocks.create("e2e:minerai", { name = "Minerai du test", texture = "diamond_block", hardness = 3, light = 7, drops = { ["e2e:rubis"] = 2 } })
server.items.create("e2e:galette", { name = "Galette", icon = "bread", food = 8, saturation = 9.6 })
pcall(server.items.create, "minecraft:faux", { name = "Refusé" })
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    // Connexion pas à pas, pour lire la liste des objets envoyée après StartGame
    let mut c = Client::new(addr);
    c.handshake();
    c.game(0xc1, &2225i32.to_be_bytes(), None);
    assert_eq!(c.recv_game(false).0, 0x8f);
    c.login(2225, "Steve", false);
    c.wait_for(6);
    c.game(8, &[3, 0, 0], Some(0xff));
    c.wait_for(7);
    c.game(8, &[4, 0, 0], Some(0xff));
    // Le bloc du plugin est déclaré dans l'entrée dans le monde, avec son image
    let start_game = c.wait_for(11);
    let declared = |needle: &[u8]| start_game.windows(needle.len()).any(|w| w == needle);
    assert!(declared(b"e2e:minerai") && declared(b"minecraft:material_instances") && declared(b"diamond_block") && declared(b"vanilla_block_data"), "bloc du plugin absent de StartGame");
    // Les blocs à états déclarent leur propriété et une condition par état
    assert!(declared(b"basalt:stage") && declared(b"basalt:axis") && declared(b"q.block_state('basalt:axis') == 2") && declared(b"minecraft:transformation"));
    let registry = c.wait_for(162);
    let has = |needle: &[u8]| registry.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"e2e:minerai"), "l'objet qui pose le bloc manque à la liste");
    // Les objets du jeu sont toujours là, ceux du plugin les suivent avec leurs composants
    assert!(has(b"minecraft:apple"), "la liste du jeu a disparu");
    assert!(has(b"e2e:rubis") && has(b"Rubis du test") && has(b"emerald"), "objet du plugin absent de la liste");
    assert!(has(b"e2e:galette") && has(b"minecraft:food") && has(b"nutrition"));
    assert!(!has(b"minecraft:faux"), "un plugin ne crée rien sous le nom du jeu");
    // Le nombre en tête compte les nouveaux venus
    let mut pos = 0;
    let count = read_varu32(&registry, &mut pos).unwrap();
    assert!(count > 2000, "{} objets annoncés", count);
    // Le menu créatif : celui du jeu, plus le groupe du plugin et ses objets (sauf l'objet caché)
    c.game(69, &[8, 16], Some(0xff));
    let menu = c.wait_for(145);
    let has = |needle: &[u8]| menu.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"itemGroup.name.planks") && has(b"Gemmes du test"), "groupe du plugin absent du menu créatif");
    let mut pos = 0;
    assert_eq!(read_varu32(&menu, &mut pos), Some(124 + 5), "les trois blocs, le groupe nommé et celui de la galette");
    // (le bloc y figure avec son identifiant réseau : le hachage de son nom)
    let hash = basalt::bedrock::block_hash("e2e:minerai");
    let mut w = Vec::new();
    write_varu32(&mut w, hash);
    assert!(menu.windows(w.len()).any(|x| x == &w[..]), "bloc du plugin absent du menu créatif");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugin_creatures_appear_with_their_own_values() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-creature-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("vache.lua"),
        r#"
server.entities.create("e2e:vache_doree", {
  looks = "minecraft:cow", name = "Vache du test", health = 40, scale = 2,
  drops = { ["minecraft:gold_ingot"] = { 1, 3 } }, xp = 5,
})
pcall(server.entities.create, "e2e:sans_modele", { health = 5 })
server.entities.create("e2e:gardien", { looks = "minecraft:iron_golem", ownModel = true, width = 0.9, height = 2 })
local Arrivee = {}
function Arrivee:onPlayerJoin(event)
  local x, y, z = event:getPlayer():getPosition()
  server.entities.spawn("e2e:vache_doree", x + 2, y, z)
  server.entities.spawn("e2e:gardien", x - 2, y, z)
end
server.registerEvents(Arrivee)
-- Les écouteurs voient la créature apparaître, avec sa sorte et sa vie, et peuvent agir sur elle
local Garde = {}
function Garde:onCreatureSpawn(event)
  local creature = event:getEntity()
  if creature:getType() == "e2e:vache_doree" and creature:isCustom() and creature:getLooks() == "minecraft:cow" then
    creature:setNameTag("Vue par le plugin, vie " .. math.floor(creature:getHealth()))
  end
end
function Garde:onCreatureDamage(event)
  local creature = event:getEntity()
  if creature:getType() == "e2e:vache_doree" and event:getDamager() then
    event:setDamage(3)
    creature:setNameTag("Touchée, reste " .. math.floor(creature:getHealth() - event:getDamage()))
  end
end
server.registerEvents(Garde)
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    // Une vache du jeu, à la vie de la fiche, renommée par l'écouteur du plugin
    let (mut seen, mut renamed, mut own) = (false, false, false);
    let mut cow = 0u32;
    for _ in 0..3000 {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        if id == 13 && (has(b"Vache du test") || has(b"Vue par le plugin")) {
            // (identifiant unique, puis identifiant d'exécution : celui qu'on vise)
            let mut pos = 0;
            read_varu32(&body, &mut pos).unwrap();
            cow = read_varu32(&body, &mut pos).unwrap();
            assert!(has(b"minecraft:cow"), "elle garde l'apparence de la vache");
            // Sa vie : 40, celle de la fiche (attribut de santé, en flottants petit-boutistes)
            assert!(has(&40.0f32.to_le_bytes()), "vie de la fiche absente");
            seen = true;
        }
        renamed |= has("Vue par le plugin, vie 40".as_bytes());
        // Celle dont l'apparence vient du pack est annoncée sous son propre nom, pas en golem
        if id == 13 && has(b"e2e:gardien") {
            assert!(!has(b"minecraft:iron_golem"));
            own = true;
        }
        if seen && renamed && own {
            break;
        }
    }
    assert!(seen, "la créature du plugin n'est jamais apparue");
    assert!(renamed, "l'écouteur CreatureSpawnEvent n'a pas renommé la créature");
    assert!(own, "la créature à l'apparence du pack n'est jamais apparue sous son nom");
    // Un coup : l'écouteur ramène les dégâts à 3 et affiche la vie restante au-dessus d'elle,
    // chez un joueur qui la voit déjà
    let mut attack = vec![0x00, 0x03, 0x00];
    write_varu32(&mut attack, cow);
    attack.push(0x01);
    attack.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
    c.game(30, &attack, Some(0xff));
    let mut told = false;
    for _ in 0..3000 {
        let (id, body) = c.recv_game(true);
        if id == 39 && body.windows(18).any(|w| w == "Touchée, reste 37".as_bytes()) {
            told = true;
            break;
        }
    }
    assert!(told, "le nom changé par CreatureDamageEvent n'est pas arrivé au joueur");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_creatures_attack_players_in_survival() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-hostile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("ogre.lua"),
        r#"
server.entities.create("e2e:ogre", { looks = "minecraft:cow", name = "Ogre du test", hostile = true, attackDamage = 4, sight = 12 })
local Arrivee = {}
function Arrivee:onPlayerJoin(event)
  local x, y, z = event:getPlayer():getPosition()
  server.entities.spawn("e2e:ogre", x + 1, y, z)
  event:getPlayer():setGamemode(0)
end
function Arrivee:onPlayerDeath(event)
  server.setMotd(event:getDeathMessage())
end
server.registerEvents(Arrivee)
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    // Personne ne frappe l'ogre : un événement « blessé » (2) ne peut venir que du joueur
    let is_hurt = |id: u32, body: &[u8]| {
        let mut pos = 0;
        id == 27 && read_varu32(body, &mut pos).is_some() && body.get(pos) == Some(&2)
    };
    // Le plugin met le joueur en survie à son arrivée : l'ogre vient le frapper
    let mut hurt = false;
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < until {
        let (id, body) = c.recv_game(true);
        if is_hurt(id, &body) {
            hurt = true;
            break;
        }
    }
    assert!(hurt, "la créature hostile n'a jamais frappé le joueur en survie");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn monsters_appear_at_night_and_zombies_burn_by_day() {
    // La nuit : un monstre apparaît de lui-même autour d'un joueur en survie
    let dir = std::env::temp_dir().join(format!("basalt-e2e-nuit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("monstres"), "").unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "18000").unwrap();
    let survival = r#"
local Arrivee = {}
function Arrivee:onPlayerJoin(event) event:getPlayer():setGamemode(0) end
server.registerEvents(Arrivee)
"#;
    std::fs::write(dir.join("survie.lua"), survival).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let mut monster = false;
    let until = std::time::Instant::now() + Duration::from_secs(40);
    while std::time::Instant::now() < until && !monster {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        monster = id == 13 && (has(b"minecraft:zombie") || has(b"minecraft:spider"));
    }
    assert!(monster, "aucun monstre n'est apparu de la nuit");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    // La nuit encore, mais le sol est éclairé partout : rien n'apparaît
    let dir = std::env::temp_dir().join(format!("basalt-e2e-eclaire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("monstres"), "").unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "18000").unwrap();
    let lit = r#"
local Arrivee = {}
function Arrivee:onPlayerJoin(event)
  local x, y, z = event:getPlayer():getPosition()
  x, y, z = math.floor(x), math.floor(y), math.floor(z)
  local world = server.getWorld()
  for dx = -60, 60, 6 do
    for dz = -60, 60, 6 do
      world:setBlock(x + dx, y, z + dz, "minecraft:glowstone")
    end
  end
  event:getPlayer():setGamemode(0)
end
server.registerEvents(Arrivee)
"#;
    std::fs::write(dir.join("lampes.lua"), lit).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let mut monster = false;
    let until = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < until && !monster {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        monster = id == 13 && (has(b"minecraft:zombie") || has(b"minecraft:spider"));
    }
    assert!(!monster, "un monstre est apparu sur un sol éclairé");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    // En plein jour, sous un grand toit : il y fait assez sombre, un monstre apparaît
    let dir = std::env::temp_dir().join(format!("basalt-e2e-toit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("monstres"), "").unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "6000").unwrap();
    let roof = r#"
local Arrivee = {}
function Arrivee:onPlayerJoin(event)
  local x, y, z = event:getPlayer():getPosition()
  x, y, z = math.floor(x), math.floor(y), math.floor(z)
  local world = server.getWorld()
  local stone, roof = server.blockState("minecraft:stone"), {}
  for dx = -64, 64 do
    for dz = -64, 64 do
      roof[#roof + 1] = x + dx
      roof[#roof + 1] = y + 3
      roof[#roof + 1] = z + dz
      roof[#roof + 1] = stone
    end
  end
  world:setBlocks(roof)
  event:getPlayer():setGamemode(0)
end
server.registerEvents(Arrivee)
"#;
    std::fs::write(dir.join("toit.lua"), roof).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let mut monster = false;
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < until && !monster {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        monster = id == 13 && (has(b"minecraft:zombie") || has(b"minecraft:spider"));
    }
    assert!(monster, "aucun monstre n'est apparu à l'ombre en plein jour");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    // Le jour : rien n'apparaît, et un zombie à ciel ouvert brûle (il encaisse des coups
    // alors que personne ne le frappe : le joueur est en créatif)
    let dir = std::env::temp_dir().join(format!("basalt-e2e-jour-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("monstres"), "").unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "6000").unwrap();
    let summon = r#"
local Arrivee = {}
function Arrivee:onPlayerJoin(event)
  local x, y, z = event:getPlayer():getPosition()
  server.entities.spawn("minecraft:zombie", x + 4, y, z)
end
server.registerEvents(Arrivee)
"#;
    std::fs::write(dir.join("zombie.lua"), summon).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut zombies, mut burnt) = (0, false);
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < until && !burnt {
        let (id, body) = c.recv_game(true);
        if id == 13 && body.windows(16).any(|w| w == b"minecraft:zombie") {
            zombies += 1;
        }
        let mut pos = 0;
        burnt = id == 27 && read_varu32(&body, &mut pos).is_some() && body.get(pos) == Some(&2);
    }
    assert!(burnt, "le zombie n'a pas brûlé au soleil");
    assert_eq!(zombies, 1, "seul le zombie du plugin doit exister en plein jour");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}


/// Un squelette tire des flèches sur un joueur en survie, un plugin en tire aussi et apprend
/// où la sienne tombe ; un creeper explose au contact.
#[test]
fn fire_reaches_tnt_which_falls_and_explodes() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-feu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    // Une TNT en l'air, une flamme contre elle : le feu l'amorce, elle tombe et explose
    let plugin = r#"
local Feu = {}
function Feu:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  x, y, z = math.floor(x), math.floor(y), math.floor(z)
  p:setGamemode(1)
  local world = server.getWorld()
  world:setBlocks({ x + 13, y + 3, z, server.blockState("minecraft:netherrack"), x + 12, y + 4, z, server.blockState("minecraft:tnt") })
  world:setBlocks({ x + 13, y + 4, z, server.blockState("minecraft:fire") })
end
server.registerEvents(Feu)
"#;
    std::fs::write(dir.join("feu.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut primed, mut fell, mut boom) = (false, false, false);
    let until = std::time::Instant::now() + Duration::from_secs(50);
    while std::time::Instant::now() < until && !boom {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        primed |= id == 13 && has(b"minecraft:tnt");
        // (MoveActorAbsolute : la TNT amorcée tombe)
        fell |= primed && id == 18;
        boom |= primed && has(b"explode");
    }
    assert!(primed, "le feu n'a pas amorcé la TNT");
    assert!(fell, "la TNT amorcée n'est pas tombée");
    assert!(boom, "la TNT n'a pas explosé");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skeletons_shoot_plugins_too_and_creepers_explode() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-tirs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "18000").unwrap();
    let plugin = r#"
local Tirs = {}
function Tirs:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  p:setGamemode(0)
  server.entities.spawn("minecraft:skeleton", x + 8, y, z)
  -- Une boule de neige tirée vers le bas, loin du joueur : elle touche le sol
  server.entities.shoot("minecraft:snowball", x - 10, y + 3, z, 0, -1, 0, { speed = 0.5, damage = 0 })
end
function Tirs:onProjectileHit(event)
  if event:getProjectile() == "minecraft:snowball" then server.broadcast("touche " .. event:getTarget()) end
end
server.registerEvents(Tirs)
"#;
    std::fs::write(dir.join("tirs.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut arrow, mut snowball, mut landed) = (false, false, false);
    let until = std::time::Instant::now() + Duration::from_secs(40);
    while std::time::Instant::now() < until && !(arrow && snowball && landed) {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        arrow |= id == 13 && has(b"minecraft:arrow");
        snowball |= id == 13 && has(b"minecraft:snowball");
        landed |= id == 9 && has(b"touche block");
    }
    assert!(snowball, "le projectile du plugin n'est pas apparu");
    assert!(landed, "le plugin n'a pas su que son projectile avait touché le sol");
    assert!(arrow, "le squelette n'a pas tiré");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    // Le creeper : il s'approche, allume sa mèche et explose (bruit, puis des blocs en moins)
    let dir = std::env::temp_dir().join(format!("basalt-e2e-creeper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "18000").unwrap();
    let plugin = r#"
local Boum = {}
function Boum:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  p:setGamemode(0)
  server.entities.spawn("minecraft:creeper", x + 4, y, z)
end
function Boum:onExplosion(event) server.broadcast("boum " .. math.floor(event:getPower())) end
server.registerEvents(Boum)
"#;
    std::fs::write(dir.join("boum.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut noise, mut told, mut holes) = (false, false, 0);
    let until = std::time::Instant::now() + Duration::from_secs(40);
    while std::time::Instant::now() < until && !(noise && told && holes >= 3) {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        noise |= id == 123 && has(b"explode");
        told |= id == 9 && has(b"boum 3");
        if id == 21 && noise {
            holes += 1;
        }
    }
    assert!(noise, "le creeper n'a pas explosé");
    assert!(told, "le plugin n'a pas été prévenu de l'explosion");
    assert!(holes >= 3, "l'explosion n'a pas creusé le sol ({} blocs changés)", holes);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Une araignée qui poursuit un joueur escalade le mur qui les sépare.
#[test]
fn spiders_climb_walls_to_reach_players() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-mur-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "18000").unwrap();
    let plugin = r#"
local Mur = {}
function Mur:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  x, y, z = math.floor(x), math.floor(y), math.floor(z)
  local world = server.getWorld()
  for dz = -20, 20 do
    for dy = 0, 3 do world:setBlock(x + 3, y + dy, z + dz, "minecraft:stone") end
  end
  p:setGamemode(0)
  server.entities.spawn("minecraft:spider", x + 7, y, z)
  server.broadcast("sol " .. y)
end
server.registerEvents(Mur)
"#;
    std::fs::write(dir.join("mur.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut ground, mut top) = (None::<f32>, f32::MIN);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < until && !ground.map(|g| top >= g + 3.5).unwrap_or(false) {
        let (id, body) = c.recv_game(true);
        if id == 9 {
            if let Some(i) = body.windows(4).position(|w| w == b"sol ") {
                let digits: String = body[i + 4..].iter().take_while(|b| b.is_ascii_digit() || **b == b'-').map(|b| *b as char).collect();
                ground = digits.parse::<f32>().ok();
            }
        }
        if id == 18 {
            let mut pos = 0;
            if read_varu32(&body, &mut pos).is_some() && body.len() >= pos + 13 {
                let y = f32::from_le_bytes([body[pos + 5], body[pos + 6], body[pos + 7], body[pos + 8]]);
                top = top.max(y);
            }
        }
    }
    let ground = ground.expect("le plugin n'a pas donné la hauteur du sol");
    assert!(top >= ground + 3.5, "l'araignée n'a pas escaladé le mur (montée jusqu'à {} pour un sol à {})", top, ground);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un joueur bande son arc (clic, puis bouton relâché une seconde plus tard) : une flèche part.
#[test]
fn players_shoot_arrows_with_a_bow() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-arc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    let plugin = r#"
local Arc = {}
function Arc:onPlayerJoin(event) event:getPlayer():give("minecraft:bow", 1) end
server.registerEvents(Arc)
"#;
    std::fs::write(dir.join("arc.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    // Le temps que l'arc arrive dans la main
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < until {
        c.recv_game(true);
    }
    // Transaction « objet utilisé, dans le vide », puis « objet relâché »
    c.game(30, &[0, 0, 2, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0], Some(0xff));
    let until = std::time::Instant::now() + Duration::from_millis(1300);
    while std::time::Instant::now() < until {
        c.recv_game(true);
    }
    c.game(30, &[0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], Some(0xff));
    let mut arrow = false;
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < until && !arrow {
        let (id, body) = c.recv_game(true);
        arrow = id == 13 && body.windows(15).any(|w| w == b"minecraft:arrow");
    }
    assert!(arrow, "la flèche n'est pas partie");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un plugin donne un effet à un joueur : le jeu en est prévenu (icône, décompte), puis de son retrait.
#[test]
fn players_receive_effects() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-effets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    let plugin = r#"
local Effets = {}
function Effets:onPlayerJoin(event)
  local p = event:getPlayer()
  p:addEffect("regeneration", 10, 2)
  p:addEffect("speed", 60)
  p:removeEffect("speed")
end
server.registerEvents(Effets)
"#;
    std::fs::write(dir.join("effets.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    // (opération, effet, niveau, durée) de chaque annonce reçue
    let mut seen: Vec<(u8, i32, i32, i32)> = Vec::new();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < until && seen.len() < 3 {
        let (id, body) = c.recv_game(true);
        if id == 28 {
            let mut pos = 0;
            read_varu32(&body, &mut pos).unwrap();
            let op = body[pos];
            pos += 1;
            let zig = |v: u32| ((v >> 1) as i32) ^ -((v & 1) as i32);
            let effect = zig(read_varu32(&body, &mut pos).unwrap());
            let level = zig(read_varu32(&body, &mut pos).unwrap());
            pos += 1;
            let ticks = zig(read_varu32(&body, &mut pos).unwrap());
            seen.push((op, effect, level, ticks));
        }
    }
    // Régénération II pour dix secondes, vitesse I pour une minute, puis vitesse retirée
    assert_eq!(seen, vec![(1, 10, 1, 200), (1, 1, 0, 1200), (3, 1, 0, 0)]);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Les créatures volantes quittent le sol : le ghast monte et crache une boule de feu, le
/// phantom s'élève au-dessus du joueur.
#[test]
fn flying_monsters_take_off_and_ghasts_shoot() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-vol-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("world").join("time.txt"), "18000").unwrap();
    let plugin = r#"
local Vol = {}
function Vol:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  p:setGamemode(0)
  server.entities.spawn("minecraft:ghast", x + 12, y, z)
  server.entities.spawn("minecraft:phantom", x - 6, y, z)
  server.broadcast("sol " .. math.floor(y))
end
server.registerEvents(Vol)
"#;
    std::fs::write(dir.join("vol.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut ground, mut fireball) = (None::<f32>, false);
    // Hauteur la plus haute atteinte par chaque entité
    let mut tops: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    let until = std::time::Instant::now() + Duration::from_secs(40);
    let high = |tops: &std::collections::HashMap<u32, f32>, g: f32| tops.values().filter(|y| **y >= g + 6.0).count();
    while std::time::Instant::now() < until && !(fireball && ground.map(|g| high(&tops, g) >= 2).unwrap_or(false)) {
        let (id, body) = c.recv_game(true);
        let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        if id == 9 {
            if let Some(i) = body.windows(4).position(|w| w == b"sol ") {
                let digits: String = body[i + 4..].iter().take_while(|b| b.is_ascii_digit() || **b == b'-').map(|b| *b as char).collect();
                ground = digits.parse::<f32>().ok();
            }
        }
        fireball |= id == 13 && has(b"minecraft:fireball");
        if id == 18 {
            let mut pos = 0;
            if let Some(actor) = read_varu32(&body, &mut pos) {
                if body.len() >= pos + 13 {
                    let y = f32::from_le_bytes([body[pos + 5], body[pos + 6], body[pos + 7], body[pos + 8]]);
                    let top = tops.entry(actor).or_insert(f32::MIN);
                    *top = top.max(y);
                }
            }
        }
    }
    let ground = ground.expect("le plugin n'a pas donné la hauteur du sol");
    assert!(high(&tops, ground) >= 2, "le ghast et le phantom devaient s'élever de 6 blocs au moins : {:?} pour un sol à {}", tops, ground);
    assert!(fireball, "le ghast n'a pas craché de boule de feu");
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Le blaze tire ses petites boules de feu par rafales de trois.
#[test]
fn blazes_shoot_bursts_of_three() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-blaze-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    let plugin = r#"
local Feu = {}
function Feu:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  p:setGamemode(0)
  p:addEffect("resistance", 60, 5)
  server.entities.spawn("minecraft:blaze", x + 6, y, z)
end
server.registerEvents(Feu)
"#;
    std::fs::write(dir.join("feu.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    // Instants des tirs
    let mut shots: Vec<std::time::Instant> = Vec::new();
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < until && shots.len() < 4 {
        let (id, body) = c.recv_game(true);
        if id == 13 && body.windows(24).any(|w| w == b"minecraft:small_fireball") {
            shots.push(std::time::Instant::now());
        }
    }
    assert!(shots.len() >= 4, "le blaze devait tirer au moins quatre fois ({} tirs)", shots.len());
    // Trois tirs serrés (moins d'une seconde et demie en tout), puis une pause avant le quatrième
    assert!(shots[2] - shots[0] < Duration::from_millis(1500), "la rafale est trop lente : {:?}", shots[2] - shots[0]);
    assert!(shots[3] - shots[2] > Duration::from_millis(2500), "pas de pause après la rafale : {:?}", shots[3] - shots[2]);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// La sorcière lance des potions sur un joueur en survie : la fiole part, se brise, et le
/// joueur en reçoit l'effet.
#[test]
fn witches_throw_potions_that_splash_players() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-sorciere-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    let plugin = r#"
local Fiole = {}
function Fiole:onPlayerJoin(event)
  local p = event:getPlayer()
  local x, y, z = p:getPosition()
  p:setGamemode(0)
  server.entities.spawn("minecraft:witch", x + 6, y, z)
end
server.registerEvents(Fiole)
"#;
    std::fs::write(dir.join("fiole.lua"), plugin).unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::join(addr, "Steve");
    let (mut thrown, mut splashed, mut effect) = (false, false, None::<i32>);
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < until && !(thrown && splashed && effect.is_some()) {
        let (id, body) = c.recv_game(true);
        thrown |= id == 13 && body.windows(23).any(|w| w == b"minecraft:splash_potion");
        // LevelEvent 2002 : les éclaboussures (varint signé : 4004)
        if id == 25 {
            let mut pos = 0;
            splashed |= read_varu32(&body, &mut pos) == Some(4004);
        }
        if id == 28 {
            let mut pos = 0;
            read_varu32(&body, &mut pos).unwrap();
            pos += 1;
            let zig = |v: u32| ((v >> 1) as i32) ^ -((v & 1) as i32);
            effect = Some(zig(read_varu32(&body, &mut pos).unwrap()));
        }
    }
    assert!(thrown, "la sorcière n'a pas lancé de potion");
    assert!(splashed, "la potion ne s'est pas brisée");
    // Poison (19), lenteur (2) ou faiblesse (18) : les potions à durée de la sorcière
    assert!(matches!(effect, Some(19) | Some(2) | Some(18)), "effet reçu : {:?}", effect);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// L'API de gestion : fermée sans jeton, en lecture seule, et elle décrit le serveur.
#[test]
fn the_management_api_answers_with_a_token_only() {
    use std::io::{Read, Write};
    let dir = std::env::temp_dir().join(format!("basalt-e2e-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("world")).unwrap();
    std::fs::write(dir.join("api"), "").unwrap();
    let (addr, stop, handle) = start(&dir);
    // Le port et le jeton, écrits par le serveur à l'ouverture de l'API
    let until = std::time::Instant::now() + Duration::from_secs(10);
    let port: u16 = loop {
        if let Some(p) = std::fs::read_to_string(dir.join("api-port.txt")).ok().and_then(|t| t.trim().parse().ok()) {
            break p;
        }
        assert!(std::time::Instant::now() < until, "l'API ne s'est pas ouverte");
        std::thread::sleep(Duration::from_millis(50));
    };
    let token = std::fs::read_to_string(dir.join("api-token.txt")).unwrap().trim().to_string();
    assert_eq!(token.len(), 64);
    let send = |method: &str, path: &str, token: Option<&str>, body: &str| -> (u16, String) {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let auth = token.map(|t| format!("Authorization: Bearer {}\r\n", t)).unwrap_or_default();
        stream.write_all(format!("{} {} HTTP/1.1\r\nHost: localhost\r\n{}Content-Length: {}\r\n\r\n{}", method, path, auth, body.len(), body).as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        let code = answer.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (code, answer.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
    };
    let ask = |method: &str, path: &str, token: Option<&str>| send(method, path, token, "");
    // Le tableau de bord se charge sans jeton (il le demande lui-même), avec ses garde-fous
    let (code, page) = ask("GET", "/", None);
    assert!(code == 200 && page.contains("<title>Basalt, tableau de bord</title>") && page.contains("/panel.js"), "{}", code);
    assert!(ask("GET", "/panel.js", None).1.contains("sessionStorage") && ask("GET", "/panel.css", None).1.contains(".orgues"));
    assert!(ask("GET", "/logo.svg", None).1.starts_with("<svg"));
    assert_eq!(ask("GET", "/v1/me", Some(&token)), (200, "{\"admin\":false}".to_string()));
    // Sans jeton, ou avec un mauvais : rien
    assert_eq!(ask("GET", "/v1/status", None).0, 401);
    assert_eq!(ask("GET", "/v1/status", Some("0000000000000000000000000000000000000000000000000000000000000000")).0, 401);
    // Le jeton de lecture ne permet aucune action ; les autres méthodes n'existent pas
    assert_eq!(send("POST", "/v1/disable", Some(&token), r#"{"name":"tnt"}"#).0, 403);
    assert_eq!(ask("DELETE", "/v1/status", Some(&token)).0, 405);
    assert_eq!(ask("GET", "/v1/inconnu", Some(&token)).0, 404);
    // L'état du serveur
    let (code, body) = ask("GET", "/v1/status", Some(&token));
    assert_eq!(code, 200, "{}", body);
    assert!(body.contains("\"name\":\"Basalt\"") && body.contains("\"players_online\":0") && body.contains("ticks_per_second"), "{}", body);
    // Un joueur arrive : il apparaît dans la liste
    let _c = Client::join(addr, "Steve");
    let (code, body) = ask("GET", "/v1/players", Some(&token));
    assert_eq!(code, 200, "{}", body);
    assert!(body.contains("\"name\":\"Steve\"") && body.contains("\"gamemode\"") && body.contains("\"online\":1") && body.contains("\"total\":1"), "{}", body);
    // La recherche se fait côté serveur, et la liste vient par pages
    let (_, body) = ask("GET", "/v1/players?q=ste&limit=10", Some(&token));
    assert!(body.contains("\"name\":\"Steve\"") && body.contains("\"total\":1"), "{}", body);
    let (_, body) = ask("GET", "/v1/players?q=personne", Some(&token));
    assert!(body.contains("\"online\":1") && body.contains("\"total\":0") && body.contains("\"players\":[]"), "{}", body);
    let (_, body) = ask("GET", "/v1/players?offset=5", Some(&token));
    assert!(body.contains("\"total\":1") && body.contains("\"players\":[]"), "{}", body);
    // Les objets et les blocs du jeu
    let (code, body) = ask("GET", "/v1/items", Some(&token));
    assert!(code == 200 && body.contains("\"name\":\"minecraft:diamond_sword\"") && body.contains("\"durability\":1561"), "objets : {} octets", body.len());
    let (code, body) = ask("GET", "/v1/blocks", Some(&token));
    assert!(code == 200 && body.contains("\"name\":\"minecraft:stone\"") && body.contains("\"light\":15"), "blocs : {} octets", body.len());

    // Les images : aucune n'est livrée avec Basalt, et sans pack du jeu il le dit
    assert_eq!(ask("GET", "/v1/textures", Some(&token)).1, "{\"vanilla\":false,\"folder\":\"vanilla_resource_pack\"}");
    assert_eq!(ask("GET", "/v1/texture?kind=item&name=diamond", Some(&token)).0, 404);
    assert_eq!(ask("GET", "/v1/texture?kind=item&name=diamond", None).0, 401);
    assert_eq!(ask("GET", "/v1/texture?kind=autre&name=..%2F..%2Fsecret", Some(&token)).0, 404);
    // Les actions, avec le jeton d'administration (qui lit aussi)
    let admin = std::fs::read_to_string(dir.join("api-admin-token.txt")).unwrap().trim().to_string();
    assert!(admin.len() == 64 && admin != token);
    assert_eq!(ask("GET", "/v1/me", Some(&admin)).1, "{\"admin\":true}");
    assert_eq!(ask("GET", "/v1/status", Some(&admin)).0, 200);
    let (code, body) = send("POST", "/v1/disable", Some(&admin), r#"{"name":"minecraft:tnt"}"#);
    assert!(code == 200 && body.contains("\"disabled\":true") && body.contains("\"changed\":true"), "{} {}", code, body);
    assert_eq!(ask("GET", "/v1/disabled", Some(&token)), (200, "[\"tnt\"]".to_string()));
    assert!(std::fs::read_to_string(dir.join("disabled.txt")).unwrap().lines().any(|l| l == "tnt"), "la liste est enregistrée");
    // Un nom inconnu, un corps illisible
    assert_eq!(send("POST", "/v1/disable", Some(&admin), r#"{"name":"nimportequoi"}"#).0, 404);
    assert_eq!(send("POST", "/v1/disable", Some(&admin), "pas du json").0, 400);
    assert_eq!(send("POST", "/v1/enable", Some(&admin), r#"{"name":"tnt"}"#).0, 200);
    assert_eq!(ask("GET", "/v1/disabled", Some(&token)).1, "[]");
    // Agir sur un joueur : le téléporter, changer son mode ; pas avec le jeton de lecture
    assert_eq!(send("POST", "/v1/players/action", Some(&token), r#"{"name":"Steve","action":"kill"}"#).0, 403);
    let (code, body) = send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Steve","action":"teleport","x":100.5,"y":80,"z":-40.5}"#);
    assert_eq!(code, 200, "{}", body);
    let (_, body) = ask("GET", "/v1/players", Some(&token));
    assert!(body.contains("\"x\":100.5") && body.contains("\"z\":-40.5"), "téléportation : {}", body);
    assert_eq!(send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Steve","action":"gamemode","mode":"creative"}"#).0, 200);
    assert!(ask("GET", "/v1/players", Some(&token)).1.contains("\"gamemode\":\"creative\""));
    assert_eq!(send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Steve","action":"effect","effect":"speed","seconds":30,"level":2}"#).0, 200);
    assert!(ask("GET", "/v1/players", Some(&token)).1.contains("\"effects\":[\"speed\"]"));
    // Un joueur absent, une action inconnue, des coordonnées hors du monde
    assert_eq!(send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Fantome","action":"kill"}"#).0, 404);
    assert_eq!(send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Steve","action":"danser"}"#).0, 400);
    assert_eq!(send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Steve","action":"teleport","x":0,"y":9000,"z":0}"#).0, 400);
    // L'analyse de la carte : lancée par l'administration, lue par tous
    assert_eq!(ask("GET", "/v1/scan", Some(&token)).1, "{\"state\":\"idle\"}");
    assert_eq!(send("POST", "/v1/scan", Some(&token), "").0, 403);
    assert_eq!(send("POST", "/v1/players/action", Some(&admin), r#"{"name":"Steve","action":"give","item":"diamond_block","count":70}"#).0, 200);
    assert_eq!(send("POST", "/v1/scan", Some(&admin), "").0, 200);
    let until = std::time::Instant::now() + Duration::from_secs(10);
    let report = loop {
        let (_, body) = ask("GET", "/v1/scan", Some(&token));
        if body.contains("\"state\":\"done\"") {
            break body;
        }
        assert!(std::time::Instant::now() < until, "l'analyse ne s'est pas terminée : {}", body);
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(report.contains("\"players_online\":1") && report.contains("\"item\":\"diamond_block\",\"total\":70") && report.contains("joueur Steve"), "{}", report);
    // Une commande de console : le joueur reçoit le message
    let mut c = _c;
    assert_eq!(send("POST", "/v1/command", Some(&admin), r#"{"command":"say Bonjour de l'API"}"#).0, 200);
    let mut heard = false;
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < until && !heard {
        let (id, body) = c.recv_game(true);
        heard = id == 9 && body.windows(17).any(|w| w == b"Bonjour de l'API".as_slice() || w.starts_with(b"Bonjour de l'API"));
    }
    assert!(heard, "la commande envoyée par l'API n'a pas atteint le joueur");
    assert_eq!(send("POST", "/v1/command", Some(&admin), r#"{"command":"say a\nb"}"#).0, 400);

    // La carte : une tuile en octets, le bloc d'une colonne, et un bloc changé à distance
    let (_, list) = ask("GET", "/v1/players", Some(&token));
    let coord = |key: &str| -> i32 {
        let at = list.find(key).unwrap() + key.len();
        list[at..].split(|ch: char| ch == ',' || ch == '}').next().unwrap().trim().parse::<f64>().unwrap().floor() as i32
    };
    let (px, pz) = (coord("\"x\":"), coord("\"z\":"));
    let (code, found) = ask("GET", &format!("/v1/world/block?x={}&z={}", px, pz), Some(&token));
    assert_eq!(code, 200, "{}", found);
    let top: i32 = found.split("\"y\":").nth(1).unwrap().split(',').next().unwrap().trim().parse::<f64>().unwrap() as i32;
    let spot = format!(r#"{{"x":{},"y":{},"z":{},"block":"gold_block"}}"#, px, top, pz);
    assert_eq!(send("POST", "/v1/world/block", Some(&token), &spot).0, 403, "changer le monde demande le jeton d'administration");
    let (code, done) = send("POST", "/v1/world/block", Some(&admin), &spot);
    assert!(code == 200 && done.contains("\"changed\":1"), "{} {}", code, done);
    assert!(ask("GET", &format!("/v1/world/block?x={}&y={}&z={}", px, top, pz), Some(&token)).1.contains("gold_block"));
    let raw = |path: &str| -> Vec<u8> {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.write_all(format!("GET {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n", path, token).as_bytes()).unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        let head = answer.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        answer.split_off(head)
    };
    let tile = raw(&format!("/v1/map?cx={}&cz={}", px >> 4, pz >> 4));
    assert_eq!(tile.len(), 64 * 64 * 4, "une tuile fait 64 colonnes de côté, quatre octets chacune");
    let (lx, lz) = ((px - ((px >> 4) & !3) * 16) as usize, (pz - ((pz >> 4) & !3) * 16) as usize);
    let dot = &tile[(lz * 64 + lx) * 4..(lz * 64 + lx) * 4 + 4];
    assert!(dot[3] == 1 && [206u8, 246, 255].contains(&dot[0]) && dot[1] > 150 && dot[2] < 80, "la colonne du bloc d'or doit être dorée : {:?}", dot);
    // Remplir : seul le voisin change, le premier est déjà en or
    let (code, done) = send("POST", "/v1/world/fill", Some(&admin), &format!(r#"{{"x":{},"y":{},"z":{},"x2":{},"y2":{},"z2":{},"block":"gold_block"}}"#, px, top, pz, px + 1, top, pz));
    assert!(code == 200 && done.contains("\"changed\":1"), "{} {}", code, done);
    assert_eq!(send("POST", "/v1/world/fill", Some(&admin), r#"{"x":0,"y":0,"z":0,"x2":100,"y2":100,"z2":100,"block":"stone"}"#).0, 400);
    // La vue en relief : le chunk du joueur arrive en entier, avec le bloc d'or à sa place ;
    // redemandé dans la même version il n'est pas renvoyé, et un chunk lointain est inconnu
    let (pcx, pcz) = (px >> 4, pz >> 4);
    let first = raw(&format!("/v1/world/chunks?list={}.{}.-1", pcx, pcz));
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    assert_eq!((u32_at(&first, 0) as i32, u32_at(&first, 4) as i32, first[12]), (pcx, pcz, 2), "le chunk du joueur doit venir avec ses blocs");
    let (version, len) = (u32_at(&first, 8), u32_at(&first, 13) as usize);
    let body = &first[17..17 + len];
    let (base, height, colours) = (i16::from_le_bytes([body[0], body[1]]) as i32, u16::from_le_bytes([body[2], body[3]]) as usize, body[4] as usize);
    assert!(top >= base && ((top - base) as usize) < height, "le sommet {} doit être dans le chunk envoyé ({} + {})", top, base, height);
    let mut at = 5 + colours * 10;
    let mut gold = None;
    for column in 0..256usize {
        let mut y = 0;
        while y < height {
            let (run, entry) = (body[at] as usize, body[at + 1] as usize);
            at += 2;
            if column == ((pz & 15) * 16 + (px & 15)) as usize && (y..y + run).contains(&((top - base) as usize)) {
                gold = Some(body[5 + entry * 10..5 + entry * 10 + 4].to_vec());
            }
            y += run;
        }
        assert_eq!(y, height, "chaque colonne fait la hauteur annoncée");
    }
    assert_eq!(at, body.len());
    assert_eq!(gold, Some(vec![246, 208, 61, 0]), "le bloc d'or doit être là où il a été posé");
    let again = raw(&format!("/v1/world/chunks?list={}.{}.{},56000.56000.-1", pcx, pcz, version));
    assert_eq!((again[12], u32_at(&again, 13), again[17 + 12]), (1, 0, 0), "inchangé : pas renvoyé ; lointain : inconnu");
    let (code, near) = ask("GET", &format!("/v1/entities?x={}&z={}", px, pz), Some(&token));
    assert!(code == 200 && near.contains("\"player\":true") && near.contains("\"h\":1.8"), "{} {}", code, near);
    assert!(near.contains("\"yaw\":"), "{}", near);
    assert!(ask("GET", "/relief.js", None).1.contains("WebGL"));
    // Sans pack du jeu : ni planche de textures ni peau, la carte garde ses couleurs
    assert_eq!(ask("GET", "/v1/world/atlas", Some(&token)).0, 404);
    assert_eq!(ask("GET", "/v1/world/skin", Some(&token)).0, 404);
    // Loin de tout joueur, rien n'est chargé : on ne lit ni ne change, et rien n'est généré
    assert_eq!(ask("GET", "/v1/world/block?x=900000&z=900000", Some(&token)).0, 409);
    assert_eq!(send("POST", "/v1/world/block", Some(&admin), r#"{"x":900000,"y":64,"z":900000,"block":"stone"}"#).0, 409);

    // Des mauvais jetons en série, vus à travers un tunnel (l'en-tête du relais dit d'où ils
    // viennent) : cette adresse-là est écartée, même avec le bon jeton ensuite ; les autres non
    let relayed = |from: &str, token: &str| -> u16 {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.write_all(format!("GET /v1/me HTTP/1.1\r\nHost: localhost\r\nX-Forwarded-For: {}\r\nAuthorization: Bearer {}\r\n\r\n", from, token).as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
    };
    for _ in 0..8 {
        assert_eq!(relayed("203.0.113.9", "mauvais-jeton"), 401);
    }
    assert_eq!(relayed("203.0.113.9", &admin), 429, "l'adresse fautive est écartée");
    assert_eq!(relayed("203.0.113.10", &admin), 200, "une autre adresse passe");
    assert_eq!(ask("GET", "/v1/me", Some(&token)).0, 200);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Le Nether est un monde à part, avec sa dimension : un joueur qui se tient dans un portail
/// déclenche `PlayerPortalEvent` (le serveur ne le déplace pas), et le plugin qui l'envoie dans
/// un monde `nether` lui fait changer de dimension — paquet ChangeDimension, puis des chunks
/// de la dimension 1.
#[test]
fn a_portal_tells_plugins_and_a_nether_world_is_another_dimension() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-portail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("portail.lua"),
        r#"
server.command("enfer", "crée le nether", function() server.worlds.load("enfer", "nether", "7") end)
local Portails = {}
function Portails:onPlayerPortal(e)
  e:getPlayer():sendMessage("portail " .. e:getPortal() .. " dans " .. e:getWorld())
  e:getPlayer():setWorld("enfer")
end
server.registerEvents(Portails)
"#,
    )
    .unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/enfer");
    command(&mut steve, "/gamemode creative");
    // Un bloc de portail du Nether posé là où il marche
    command(&mut steve, "/setblock 3 -60 0 portal");
    std::thread::sleep(Duration::from_millis(500));
    steve.drain_games(Duration::from_millis(300));
    steve.walk_to(3.5, 0.5);
    let got = steve.drain_games(Duration::from_millis(900));
    let said = got.iter().filter(|(id, _)| *id == 9).any(|(_, body)| String::from_utf8_lossy(body).contains("portail nether dans world"));
    assert!(said, "le plugin est prévenu du passage dans le portail");
    // Le changement de dimension : vers le Nether (1, en zigzag 2), sans « réapparition »
    let change = got.iter().find(|(id, _)| *id == 61).expect("paquet de changement de dimension");
    assert_eq!(change.1[0], 2, "{:?}", change.1);
    assert_eq!(change.1.len(), 19, "{:?}", change.1);
    // Puis le terrain de cette dimension : chaque chunk la porte après ses coordonnées
    let mut chunks = 0;
    for (_, body) in got.iter().filter(|(id, _)| *id == 58) {
        let mut pos = 0;
        basalt::bedrock::read_vari32(body, &mut pos);
        basalt::bedrock::read_vari32(body, &mut pos);
        assert_eq!(basalt::bedrock::read_vari32(body, &mut pos), Some(1), "chunk du Nether");
        let count = basalt::bedrock::read_varu32(body, &mut pos).unwrap();
        assert!((1..=8).contains(&count), "{} tranches dans un chunk du Nether", count);
        chunks += 1;
    }
    assert!(chunks > 0, "des chunks du Nether sont envoyés");
    // Et le signal qui referme l'écran de chargement
    assert!(got.iter().any(|(id, body)| *id == 2 && body == &[0, 0, 0, 3]), "PlayStatus PlayerSpawn après les chunks");
    // Et « changement de dimension terminé » (PlayerAction 14), sans quoi le joueur reste figé
    let spawned = got.iter().position(|(id, body)| *id == 2 && body == &[0, 0, 0, 3]).unwrap();
    let done = got.iter().skip(spawned).find(|(id, _)| *id == 36).expect("PlayerAction « dimension changée »");
    let mut pos = 0;
    basalt::bedrock::read_varu32(&done.1, &mut pos);
    assert_eq!(basalt::bedrock::read_vari32(&done.1, &mut pos), Some(14), "{:?}", done.1);
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Le reste du bestiaire : un poisson nage sans quitter son bassin, une chauve-souris vole
/// au-dessus du sol, un renard marche.
#[test]
fn fish_swim_bats_fly_and_foxes_walk() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-bestiaire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    // Un bassin de pierre de 9 × 9 rempli d'eau : fond à -60, surface à -54
    command(&mut steve, "/fill 2 -60 2 12 -55 12 stone");
    std::thread::sleep(Duration::from_millis(600));
    command(&mut steve, "/fill 3 -60 3 11 -55 11 water");
    std::thread::sleep(Duration::from_millis(600));
    command(&mut steve, "/summon cod 7.5 -58 7.5");
    command(&mut steve, "/summon bat -8.5 -58 -8.5");
    command(&mut steve, "/summon fox -8.5 -60 6.5");
    let kinds: [&[u8]; 3] = [b"minecraft:cod", b"minecraft:bat", b"minecraft:fox"];
    let mut ids: [Option<u8>; 3] = [None; 3];
    let mut seen: [Vec<(f32, f32, f32)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    // (le renard flâne quand l'envie lui en prend, comme dans le jeu : une fois toutes les
    // six secondes en moyenne ; on lui laisse jusqu'à trente secondes)
    let started = std::time::Instant::now();
    let walked = |p: &[(f32, f32, f32)]| p.len() >= 3 && p.iter().any(|q| (q.0 - p[0].0).abs().max((q.2 - p[0].2).abs()) > 0.5);
    while started.elapsed() < Duration::from_secs(14) || (!walked(&seen[2]) && started.elapsed() < Duration::from_secs(40)) {
        if let Some((id, body)) = steve.poll_game(Duration::from_millis(200)) {
            for (i, kind) in kinds.iter().enumerate() {
                if id == 13 && body.windows(kind.len()).any(|w| w == *kind) {
                    ids[i] = Some(body[1]);
                }
                if id == 18 && body.len() >= 14 && body.first() == ids[i].as_ref() {
                    let f = |at: usize| f32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
                    seen[i].push((f(2), f(6), f(10)));
                }
            }
        }
    }
    assert!(ids.iter().all(|i| i.is_some()), "poisson, chauve-souris et renard montrés : {:?}", ids);
    let span = |p: &[(f32, f32, f32)]| {
        let (lo, hi) = p.iter().fold(((f32::MAX, f32::MAX), (f32::MIN, f32::MIN)), |(lo, hi), q| ((lo.0.min(q.0), lo.1.min(q.2)), (hi.0.max(q.0), hi.1.max(q.2))));
        (hi.0 - lo.0).max(hi.1 - lo.1)
    };
    // Le poisson a nagé, et n'est jamais sorti de l'eau du bassin
    assert!(span(&seen[0]) > 0.8, "le poisson nage ({} positions)", seen[0].len());
    assert!(seen[0].iter().all(|(x, y, z)| (3.0..12.0).contains(x) && (3.0..12.0).contains(z) && (-60.0..-54.0).contains(y)), "le poisson reste dans l'eau : {:?}", seen[0]);
    // La chauve-souris a volé, au-dessus du sol (le dessus de l'herbe est à -60)
    assert!(span(&seen[1]) > 1.0, "la chauve-souris vole ({} positions)", seen[1].len());
    assert!(seen[1].iter().skip(20).all(|(_, y, _)| *y > -59.9), "elle vole au-dessus du sol : {:?}", seen[1]);
    // Le renard s'est promené, les pattes au sol
    assert!(span(&seen[2]) > 0.5, "le renard marche ({} positions)", seen[2].len());
    assert!(seen[2].iter().all(|(_, y, _)| (-60.1..-59.0).contains(y)), "{:?}", seen[2]);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Les deux numéros en tête d'une apparition (AddActor) : l'identifiant unique, puis celui de
/// la partie, tels qu'écrits (varints).
fn ids_of(body: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let unique = body.iter().position(|b| b & 0x80 == 0).unwrap() + 1;
    let runtime = unique + body[unique..].iter().position(|b| b & 0x80 == 0).unwrap() + 1;
    (body[..unique].to_vec(), body[unique..runtime].to_vec())
}

/// Comme dans le jeu : un piglin prend le lingot d'or qu'on lui tend, le montre dans sa main, et
/// jette son troc six secondes plus tard ; un seau d'eau tendu à une morue la met en seau.
#[test]
fn a_piglin_barters_gold_and_a_bucket_catches_a_fish() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-troc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/give @s gold_ingot 1");
    std::thread::sleep(Duration::from_millis(1200));
    // (un bassin à portée de main : la morue ne peut pas s'éloigner de plus de 6 blocs)
    command(&mut steve, "/fill ~3 ~-1 ~-2 ~6 ~ ~2 water");
    std::thread::sleep(Duration::from_millis(600));
    command(&mut steve, "/summon piglin ~2 ~ ~");
    command(&mut steve, "/summon cod ~4 ~-1 ~");
    // Les deux créatures apparaissent : leur numéro
    let (mut piglin, mut cod) = (None, None);
    let started = std::time::Instant::now();
    while (piglin.is_none() || cod.is_none()) && started.elapsed() < Duration::from_secs(8) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(200)) {
            if body.windows(16).any(|w| w == b"minecraft:piglin") {
                piglin = Some(ids_of(&body));
            }
            if body.windows(13).any(|w| w == b"minecraft:cod") {
                cod = Some(ids_of(&body));
            }
        }
    }
    let (piglin, cod) = (piglin.expect("piglin montré"), cod.expect("morue montrée"));
    let interact = |c: &mut Client, id: (Vec<u8>, Vec<u8>)| {
        let mut use_on = vec![0x00, 0x03, 0x00];
        use_on.extend_from_slice(&id.1);
        use_on.push(0x00);
        use_on.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        c.game(30, &use_on, Some(0xff));
    };
    // (ce qui était déjà en route : l'épée montrée à son apparition)
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_millis(500) {
        let _ = steve.poll_game(Duration::from_millis(100));
    }
    interact(&mut steve, piglin.clone());
    // Il montre le lingot (équipement), puis un objet tombe au sol
    let (mut shown, mut dropped) = (false, false);
    let started = std::time::Instant::now();
    while !(shown && dropped) && started.elapsed() < Duration::from_secs(12) {
        match steve.poll_game(Duration::from_millis(200)) {
            Some((31, body)) if body.starts_with(&piglin.1) => shown = true,
            Some((15, _)) if shown => dropped = true,
            _ => {}
        }
    }
    assert!(shown, "le piglin tient le lingot");
    assert!(dropped, "le troc tombe au sol");
    // Un seau d'eau tendu à la morue : elle disparaît dans le seau
    command(&mut steve, "/clear");
    command(&mut steve, "/give @s water_bucket 1");
    std::thread::sleep(Duration::from_millis(1200));
    interact(&mut steve, cod.clone());
    let started = std::time::Instant::now();
    let mut caught = false;
    while !caught && started.elapsed() < Duration::from_secs(6) {
        if let Some((14, body)) = steve.poll_game(Duration::from_millis(200)) {
            caught |= body.starts_with(&cod.0) && body.len() == cod.0.len();
        }
    }
    assert!(caught, "la morue est dans le seau");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Comme dans le jeu : un golem de neige bombarde de boules de neige le monstre qui passe.
#[test]
fn a_snow_golem_throws_snowballs_at_monsters() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-golem-neige-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/summon snow_golem ~3 ~ ~");
    command(&mut steve, "/summon zombie ~8 ~ ~");
    let started = std::time::Instant::now();
    let mut thrown = false;
    while !thrown && started.elapsed() < Duration::from_secs(8) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(200)) {
            thrown |= body.windows(18).any(|w| w == b"minecraft:snowball");
        }
    }
    assert!(thrown, "une boule de neige part vers le zombie");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Comme dans le jeu : l'évocateur ne frappe pas, il fait jaillir des crocs vers sa proie.
#[test]
fn an_evoker_summons_fangs() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-evocateur-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/gamemode survival");
    std::thread::sleep(Duration::from_millis(300));
    command(&mut steve, "/summon evocation_illager ~6 ~ ~");
    let started = std::time::Instant::now();
    let mut fangs = false;
    while !fangs && started.elapsed() < Duration::from_secs(10) {
        if let Some((13, body)) = steve.poll_game(Duration::from_millis(200)) {
            fangs |= body.windows(24).any(|w| w == b"minecraft:evocation_fang");
        }
    }
    assert!(fangs, "des crocs jaillissent");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Comme dans le jeu : le warden invoqué sort de terre (7 s), rugit en trouvant sa cible
/// (4,2 s), puis l'atteint de son onde sonique, même de loin.
#[test]
fn a_warden_emerges_roars_and_booms() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-warden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    command(&mut steve, "/gamemode survival");
    std::thread::sleep(Duration::from_millis(300));
    command(&mut steve, "/summon warden ~3 ~ ~");
    let started = std::time::Instant::now();
    let (mut warden, mut hurt_at, mut hit) = (None::<(Vec<u8>, Vec<u8>)>, None, false);
    while hurt_at.is_none() && started.elapsed() < Duration::from_secs(20) {
        // Steve le frappe : aveugle, c'est à lui qu'il en voudra (il ne bouge pas, sinon)
        if let (false, Some(w)) = (hit, warden.as_ref()) {
            let mut attack = vec![0x00, 0x03, 0x00];
            attack.extend_from_slice(&w.1);
            attack.push(0x01);
            attack.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
            steve.game(30, &attack, Some(0xff));
            hit = true;
        }
        match steve.poll_game(Duration::from_millis(200)) {
            Some((13, body)) if body.windows(16).any(|w| w == b"minecraft:warden") => warden = Some(ids_of(&body)),
            Some((27, body)) if body.get(1) == Some(&2) && warden.as_ref().map(|w| !body.starts_with(&w.1)).unwrap_or(false) => hurt_at = Some(started.elapsed()),
            _ => {}
        }
    }
    let hurt_at = hurt_at.expect("l'onde sonique atteint le joueur");
    // (pas avant d'être sorti de terre et d'avoir rugi)
    assert!(hurt_at > Duration::from_secs(10), "trop tôt : {:?}", hurt_at);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Comme dans le jeu : le warden entend ce qui bouge (ici une chauve-souris qui volète) et
/// finit par s'en prendre à elle, joueur ou pas.
#[test]
fn a_warden_hunts_what_it_hears() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-warden-entend-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ops.txt"), "steve\n").unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let command = |c: &mut Client, text: &str| {
        let mut body = vec![text.len() as u8];
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        c.game(77, &body, Some(0xff));
    };
    // (Steve en créatif : le warden ne s'occupe pas de lui)
    command(&mut steve, "/summon warden ~6 ~ ~");
    command(&mut steve, "/summon bat ~6 ~2 ~3");
    let started = std::time::Instant::now();
    let (mut bat, mut dead) = (None::<(Vec<u8>, Vec<u8>)>, false);
    // (la chauve-souris peut s'éloigner un moment : on laisse le temps)
    while !dead && started.elapsed() < Duration::from_secs(50) {
        match steve.poll_game(Duration::from_millis(200)) {
            Some((13, body)) if body.windows(13).any(|w| w == b"minecraft:bat") => bat = Some(ids_of(&body)),
            Some((14, body)) => dead |= bat.as_ref().map(|b| body == b.0).unwrap_or(false),
            Some((27, body)) => dead |= bat.as_ref().map(|b| body.starts_with(&b.1) && body.get(b.1.len()) == Some(&3)).unwrap_or(false),
            _ => {}
        }
    }
    assert!(bat.is_some(), "chauve-souris montrée");
    assert!(dead, "le warden a eu la chauve-souris");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `joueur:setItemInHand` remplace l'objet tenu en main ; le plugin Chaos (outil de test de
/// charge, privé) s'en sert pour armer les bots.
#[test]
fn a_plugin_puts_an_item_in_hand() {
    let dir = std::env::temp_dir().join(format!("basalt-e2e-main-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Main")).unwrap();
    std::fs::write(dir.join("Main").join("plugin.yml"), "name: Main\nversion: 1.0.0\nmain: main.lua\napi: 1\n").unwrap();
    std::fs::write(
        dir.join("Main").join("main.lua"),
        r#"
local T = {}
function T:onPlayerJoin(e)
  local nom = e:getPlayer():getName()
  server.afterSeconds(0.5, function()
    local j = server.getPlayer(nom)
    j:give("minecraft:stone", 3)
    j:setItemInHand("minecraft:tnt", 8)
    server.afterSeconds(0.5, function()
      local j2 = server.getPlayer(nom)
      j2:sendMessage("main " .. j2:getItemInHand())
      j2:setItemInHand()
      server.afterSeconds(0.5, function()
        local j3 = server.getPlayer(nom)
        j3:sendMessage("vide [" .. j3:getItemInHand() .. "]")
      end)
    end)
  end)
end
server.registerEvents(T)
"#,
    )
    .unwrap();
    let (addr, stop, handle) = start(&dir);
    std::thread::sleep(Duration::from_millis(500));
    let mut steve = Client::join(addr, "Steve");
    let started = std::time::Instant::now();
    let (mut tnt, mut empty) = (false, false);
    while !(tnt && empty) && started.elapsed() < Duration::from_secs(8) {
        if let Some((9, body)) = steve.poll_game(Duration::from_millis(200)) {
            tnt |= body.windows(18).any(|w| w == b"main minecraft:tnt");
            empty |= body.windows(7).any(|w| w == b"vide []");
        }
    }
    assert!(tnt, "la TNT est en main");
    assert!(empty, "la main est vidée");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dir);
}
