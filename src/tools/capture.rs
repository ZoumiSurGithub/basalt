//! Capture des données du jeu auprès du serveur officiel de Mojang.
//!
//! Basalt se connecte comme un joueur à un serveur officiel (Bedrock Dedicated
//! Server) lancé sur la même machine, et enregistre ce que ce serveur envoie à
//! tout joueur qui entre : liste des objets, recettes, inventaire créatif, biomes,
//! entités. Ces paquets sont rangés par version du protocole dans `data/` ; Basalt
//! peut ensuite les renvoyer tels quels aux joueurs de la même version.
//!
//! Ce fichier contient la logique du dialogue, indépendante du réseau (et testée) ;
//! le transport est dans `capture_rak.rs` (RakNet, celui qui sert) et `capture_net.rs`
//! (NetherNet, que le serveur officiel refuse sans compte Xbox).
//!
//! Le serveur officiel doit être réglé avec `online-mode=false` : le faux joueur
//! signe lui-même son identité, sans compte Xbox.

use std::path::{Path, PathBuf};

use crate::bedrock;
use crate::json::base64_encode;

/// Paquets à conserver : (identifiant, nom du fichier).
pub const WANTED: [(u32, &str); 9] = [
    (11, "start_game"),
    // Apparition d'un autre joueur : relevée pour connaître son format (jamais renvoyée telle
    // quelle). Elle n'arrive que si un second joueur est présent : voir le « témoin ».
    (12, "add_player"),
    // Liste des commandes : relevée pour connaître son format (elle n'est pas renvoyée telle quelle)
    (76, "available_commands"),
    (52, "crafting_data"),
    (119, "actor_identifiers"),
    (122, "biome_definitions"),
    (145, "creative_content"),
    (162, "item_registry"),
    (302, "trim_data"),
];

/// Réponses du jeu 1.26.65 à l'échange des packs de ressources, relevées telles quelles.
const PACKS_DOWNLOADED: [u8; 21] = [
    0x02, 0x13, 0x64, 0x6f, 0x77, 0x6e, 0x6c, 0x6f, 0x61, 0x64, 0x69, 0x6e, 0x67, 0x66, 0x69, 0x6e, 0x69, 0x73, 0x68, 0x65,
    0x64,
];
const PACKS_STACK_DONE: [u8; 27] = [
    0x03, 0x19, 0x72, 0x65, 0x73, 0x6f, 0x75, 0x72, 0x63, 0x65, 0x70, 0x61, 0x63, 0x6b, 0x73, 0x74, 0x61, 0x63, 0x6b, 0x66,
    0x69, 0x6e, 0x69, 0x73, 0x68, 0x65, 0x64,
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// En attente des réglages réseau
    Settings,
    /// Login envoyé, en attente de la réponse
    Login,
    /// Entrée en jeu : tout ce qui arrive est enregistré
    Recording,
    /// Le serveur a refusé ou fermé
    Failed,
}

pub struct Capture {
    pub protocol: i32,
    pub stage: Stage,
    compression: bool,
    login: Vec<u8>,
    /// Paquets reçus : (identifiant, contenu)
    pub records: Vec<(u32, Vec<u8>)>,
    /// Raison donnée par le serveur en cas de refus
    pub refusal: Option<String>,
    /// Le serveur a envoyé le signal d'apparition : tout est arrivé
    pub complete: bool,
    /// Jeton de mise en place du chiffrement, à traiter par le transport
    pub handshake: Option<String>,
    /// Envoyer les gros paquets sous forme « deflate » (blocs stockés), comme le vrai jeu
    pub stored_deflate: bool,
}

/// Découpe un lot du serveur officiel (qui peut contenir des centaines de paquets).
fn decode_all(d: &[u8], compression: bool, max: usize) -> Result<Vec<(u32, Vec<u8>)>, &'static str> {
    let owned;
    let data: &[u8] = if compression {
        match d.first() {
            Some(0xff) => &d[1..],
            Some(0x00) => {
                owned = crate::inflate::inflate(&d[1..], max)?;
                &owned
            }
            Some(_) => return Err("compression non gérée (mettez compression-algorithm=zlib dans le server.properties du serveur officiel)"),
            None => return Err("lot vide"),
        }
    } else {
        d
    };
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let len = bedrock::read_varu32(data, &mut pos).ok_or("longueur de paquet invalide")? as usize;
        let end = pos.checked_add(len).filter(|e| *e <= data.len()).ok_or("paquet tronqué")?;
        let mut p = pos;
        let header = bedrock::read_varu32(&data[..end], &mut p).ok_or("en-tête de paquet invalide")?;
        out.push((header & 0x3ff, data[p..end].to_vec()));
        pos = end;
    }
    Ok(out)
}

fn b64url(data: &[u8]) -> String {
    base64_encode(data, true, false)
}

/// Donne au témoin d'autres identifiants d'appareil que ceux du joueur de capture. Les deux
/// partent du même modèle (relevé auprès du vrai jeu) ; or le serveur officiel refuse un second
/// joueur portant les mêmes identifiants (« raison 44 », conflit d'identifiant).
/// `payload_b64` : données client en base64 ; renvoie la variante, ou l'original s'il est illisible.
pub fn vary_client_payload(payload_b64: &str) -> String {
    vary_client_payload_report(payload_b64).0
}

/// Comme `vary_client_payload`, avec en plus les noms des clés modifiées et la liste des noms
/// de clés du modèle (les noms seulement, jamais les valeurs) pour le journal de capture.
pub fn vary_client_payload_report(payload_b64: &str) -> (String, Vec<String>, Vec<String>) {
    vary_client_payload_with(payload_b64, true, 2)
}

/// `bump` : modifier les identifiants présents (pour le témoin). `serial` : numéro inscrit dans
/// l'identifiant d'installation (`SelfSignedId`) quand le modèle le laisse vide — deux joueurs
/// dont cet identifiant est vide sont pris pour la même installation du jeu.
pub fn vary_client_payload_with(payload_b64: &str, bump_ids: bool, serial: u8) -> (String, Vec<String>, Vec<String>) {
    let mut json = match crate::json::base64_decode(payload_b64).and_then(|b| String::from_utf8(b).ok()) {
        Some(j) => j,
        None => return (payload_b64.to_string(), Vec::new(), Vec::new()),
    };
    // Début de la valeur associée à une clé (espaces tolérés autour des deux-points)
    fn value_start(json: &str, key: &str) -> Option<usize> {
        let marker = format!("\"{}\"", key);
        let mut from = 0;
        while let Some(found) = json[from..].find(&marker) {
            let after = from + found + marker.len();
            let rest = json[after..].trim_start();
            if let Some(value) = rest.strip_prefix(':') {
                return Some(json.len() - value.trim_start().len());
            }
            from = after;
        }
        None
    }
    // Change le dernier caractère (chiffre ou lettre) de la valeur d'une clé. Faux si la clé
    // est absente ou sa valeur vide.
    fn bump(json: &mut String, key: &str) -> bool {
        let start = match value_start(json, key) {
            Some(s) => s,
            None => return false,
        };
        // Fin de la valeur : guillemet fermant pour un texte, sinon virgule ou accolade
        let end = if json[start..].starts_with('"') {
            match json[start + 1..].find('"') {
                Some(e) => start + 1 + e,
                None => return false,
            }
        } else {
            match json[start..].find(|c| c == ',' || c == '}') {
                Some(e) => start + e,
                None => return false,
            }
        };
        let at = match json[start..end].char_indices().rev().find(|(_, c)| c.is_ascii_alphanumeric()) {
            Some((i, _)) => start + i,
            None => return false,
        };
        let new = if json.as_bytes()[at] == b'7' { "3" } else { "7" };
        json.replace_range(at..at + 1, new);
        true
    }
    let mut changed = Vec::new();
    if bump_ids {
        for key in ["ClientRandomId", "SelfSignedId", "DeviceId", "PlayFabId", "PlatformOnlineId", "PlatformUserId", "PlatformOfflineId", "ThirdPartyName"] {
            if bump(&mut json, key) {
                changed.push(key.to_string());
            }
        }
    }
    // Identifiant d'installation vide : on en inscrit un, propre à ce joueur
    if let Some(start) = value_start(&json, "SelfSignedId") {
        if json[start..].starts_with("\"\"") {
            json.replace_range(start..start + 2, &format!("\"b45a17c0-de00-4000-8000-0000000000{:02x}\"", serial));
            changed.push("SelfSignedId (était vide)".to_string());
        }
    }
    // Noms des clés de premier niveau : un guillemet ouvrant précédé de « { » ou « , »
    let mut keys = Vec::new();
    let bytes = json.as_bytes();
    let (mut depth, mut i, mut expect_key) = (0i32, 0usize, false);
    while i < bytes.len() && keys.len() < 80 {
        match bytes[i] {
            b'{' | b'[' => {
                depth += 1;
                expect_key = depth == 1 && bytes[i] == b'{';
            }
            b'}' | b']' => depth -= 1,
            b',' => expect_key = depth == 1,
            b'"' => {
                // saute le texte entier (les guillemets échappés restent à l'intérieur)
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != b'"' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                if expect_key && depth == 1 {
                    let name: String = json.get(i + 1..j.min(json.len())).unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
                    keys.push(name);
                }
                expect_key = false;
                i = j;
            }
            _ => {}
        }
        i += 1;
    }
    (b64url(json.as_bytes()), changed, keys)
}

/// Construit le paquet Login d'un joueur hors ligne. `client_payload` est la partie
/// centrale (base64url) des données client d'un vrai jeu, réutilisée telle quelle : elle
/// décrit un skin et un appareil valides. `sign` signe en ES384 (96 octets).
pub fn build_login(
    protocol: i32,
    name: &str,
    spki_b64: &str,
    client_payload: &str,
    sign: &dyn Fn(&[u8]) -> Vec<u8>,
    now: u64,
) -> Vec<u8> {
    build_login_as(protocol, name, spki_b64, client_payload, sign, now, ("", ""))
}

/// Comme `build_login`, avec les identifiants de compte du jeton : (`mid`, `xid`). Deux joueurs
/// présents en même temps sur le serveur officiel doivent en porter de différents, sinon le
/// second est refusé (« raison 44 » : ce compte joue déjà ici).
pub fn build_login_as(
    protocol: i32,
    name: &str,
    spki_b64: &str,
    client_payload: &str,
    sign: &dyn Fn(&[u8]) -> Vec<u8>,
    now: u64,
    account: (&str, &str),
) -> Vec<u8> {
    // Uniquement des chiffres et des lettres : ces valeurs sont placées telles quelles dans le jeton
    let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect::<String>();
    let (mid, xid) = (clean(account.0), clean(account.1));
    let header = b64url(format!("{{\"alg\":\"ES384\",\"x5u\":\"{}\"}}", spki_b64).as_bytes());
    let jwt = |payload_b64: &str| -> String {
        let signing = format!("{}.{}", header, payload_b64);
        let signature = sign(signing.as_bytes());
        format!("{}.{}", signing, b64url(&signature))
    };
    // Identité signée par le joueur lui-même. Le témoin (second joueur de la capture) porte
    // un autre identifiant, sinon le serveur officiel le prendrait pour le même joueur.
    let uuid_end = if name == "BasaltCapture" { "10" } else { "11" };
    let identity = format!(
        "{{\"exp\":{},\"extraData\":{{\"XUID\":\"{}\",\"displayName\":\"{}\",\"identity\":\"3c1a4c2e-9d0b-4a6f-8f1e-5b7d2a9c4e{}\"}},\"identityPublicKey\":\"{}\",\"nbf\":{}}}",
        now + 86_400,
        xid,
        name,
        uuid_end,
        spki_b64,
        now.saturating_sub(60)
    );
    let chain = jwt(&b64url(identity.as_bytes()));
    let certificate = format!("{{\"chain\":[\"{}\"]}}", chain);
    // Depuis la 1.26.10, le serveur officiel lit l'identité dans un jeton « multijoueur » ;
    // hors ligne, le joueur le signe lui-même avec la clé annoncée dans `cpk`.
    let token_claims = format!(
        "{{\"aud\":\"api://auth-minecraft-services/multiplayer\",\"cpk\":\"{}\",\"exp\":{},\"iat\":{},\"ipt\":\"PlayFab\",\"iss\":\"self\",\"mid\":\"{}\",\"nbf\":{},\"tid\":\"20CA2\",\"xid\":\"{}\",\"xname\":\"{}\"}}",
        spki_b64,
        now + 86_400,
        now.saturating_sub(60),
        mid,
        now.saturating_sub(60),
        xid,
        name
    );
    let token = jwt(&b64url(token_claims.as_bytes()));
    let auth = format!(
        "{{\"AuthenticationType\":2,\"Certificate\":{},\"Token\":\"{}\"}}",
        crate::signaling::json_string(&certificate),
        token
    );
    let client = jwt(client_payload);

    let mut inner = Vec::with_capacity(auth.len() + client.len() + 8);
    inner.extend_from_slice(&(auth.len() as u32).to_le_bytes());
    inner.extend_from_slice(auth.as_bytes());
    inner.extend_from_slice(&(client.len() as u32).to_le_bytes());
    inner.extend_from_slice(client.as_bytes());
    let mut body = protocol.to_be_bytes().to_vec();
    bedrock::write_varu32(&mut body, inner.len() as u32);
    body.extend_from_slice(&inner);
    body
}

/// Réassemble un corps HTTP envoyé « par morceaux » (Transfer-Encoding: chunked) :
/// chaque morceau est précédé de sa taille en hexadécimal.
pub fn unchunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let line_end = match body[pos..].windows(2).position(|w| w == b"\r\n") {
            Some(i) => pos + i,
            None => break,
        };
        let line = String::from_utf8_lossy(&body[pos..line_end]);
        let size = match usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16) {
            Ok(s) => s,
            Err(_) => break,
        };
        let start = line_end + 2;
        if size == 0 || start + size > body.len() {
            break;
        }
        out.extend_from_slice(&body[start..start + size]);
        pos = start + size + 2;
    }
    out
}

/// Nom des raisons de déconnexion les plus utiles ici (liste du jeu, relevée de mémoire :
/// à prendre comme une indication).
fn disconnect_reason(code: i32) -> &'static str {
    match code {
        8 => "version différente",
        9 => "skin refusé",
        25 => "serveur plein",
        34 => "serveur trop ancien",
        35 => "jeu trop ancien",
        43 => "déjà connecté ailleurs",
        44 => "conflit d'identifiant avec un joueur déjà présent",
        45 => "non autorisé (liste blanche)",
        46 => "non authentifié",
        48 => "paquet inconnu",
        49 => "paquet inattendu",
        52 => "login sans requête",
        53 => "login sans certificat",
        54 => "login sans jeton",
        55..=70 => "login refusé (détail du jeton ou des données client)",
        _ => "raison non répertoriée",
    }
}

impl Capture {
    pub fn new(protocol: i32, login: Vec<u8>) -> Capture {
        Capture {
            protocol,
            stage: Stage::Settings,
            compression: false,
            login,
            records: Vec::new(),
            refusal: None,
            complete: false,
            handshake: None,
            stored_deflate: false,
        }
    }

    /// Lot contenant un paquet, sans l'octet 0xfe de tête (le transport l'ajoute au besoin).
    fn pack(&self, id: u32, body: &[u8]) -> Vec<u8> {
        let plain = bedrock::encode_packet(id, body, self.compression)[1..].to_vec();
        if !(self.compression && self.stored_deflate && plain.len() > 512) {
            return plain;
        }
        let batch = &plain[1..];
        let mut out = Vec::with_capacity(batch.len() + 64);
        out.push(0x00);
        let count = batch.chunks(60_000).count();
        for (i, c) in batch.chunks(60_000).enumerate() {
            out.push(if i + 1 == count { 1 } else { 0 });
            out.extend_from_slice(&(c.len() as u16).to_le_bytes());
            out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
            out.extend_from_slice(c);
        }
        out
    }

    /// Réponse à envoyer (chiffrée) une fois le chiffrement en place.
    pub fn handshake_reply(&self) -> Vec<u8> {
        self.pack(4, &[])
    }

    /// Premier message à envoyer une fois la liaison ouverte.
    pub fn hello(&self) -> Vec<u8> {
        self.pack(bedrock::ID_REQUEST_NETWORK_SETTINGS, &self.protocol.to_be_bytes())
    }

    /// Traite un message du serveur ; renvoie les messages à lui envoyer.
    pub fn on_message(&mut self, message: &[u8]) -> Vec<Vec<u8>> {
        let packets = match decode_all(message, self.compression, 256 * 1024 * 1024) {
            Ok(p) => p,
            Err(why) => {
                self.refusal = Some(format!("message illisible : {}", why));
                self.stage = Stage::Failed;
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for (id, body) in packets {
            match (self.stage, id) {
                (Stage::Settings, bedrock::ID_NETWORK_SETTINGS) => {
                    let algorithm = body.get(2..4).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
                    if algorithm != 0 && algorithm != 0xffff {
                        self.refusal = Some("le serveur officiel utilise une compression non gérée : mettez compression-algorithm=zlib dans son server.properties".into());
                        self.stage = Stage::Failed;
                        return out;
                    }
                    self.compression = true;
                    self.stage = Stage::Login;
                    out.push(self.pack(bedrock::ID_LOGIN, &self.login));
                }
                (_, bedrock::ID_DISCONNECT) => {
                    // raison (entier), « sans message » (booléen), puis le texte
                    let mut pos = 0;
                    let reason = bedrock::read_vari32(&body, &mut pos).unwrap_or(-1);
                    let text: String = body[pos.min(body.len())..].iter().filter(|b| (0x20..0x7f).contains(*b)).map(|b| *b as char).collect();
                    self.refusal = Some(format!(
                        "déconnecté par le serveur officiel à l'étape {:?} : raison {} ({}) {}",
                        self.stage,
                        reason,
                        disconnect_reason(reason),
                        text
                    ));
                    self.stage = Stage::Failed;
                    return out;
                }
                (Stage::Login, 3) => {
                    // ServerToClientHandshake : une chaîne contenant le jeton
                    let mut pos = 0;
                    let len = bedrock::read_varu32(&body, &mut pos).unwrap_or(0) as usize;
                    let jwt = body.get(pos..pos + len).map(|b| String::from_utf8_lossy(b).to_string());
                    self.handshake = jwt;
                    return out;
                }
                (Stage::Login, bedrock::ID_PLAY_STATUS) => {
                    let status = body.get(..4).map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(-1);
                    if status != bedrock::STATUS_LOGIN_SUCCESS {
                        self.refusal = Some(format!("login refusé par le serveur officiel (code {})", status));
                        self.stage = Stage::Failed;
                        return out;
                    }
                    self.stage = Stage::Recording;
                }
                (Stage::Recording, bedrock::ID_RESOURCE_PACKS_INFO) => {
                    out.push(self.pack(bedrock::ID_RESOURCE_PACK_RESPONSE, &PACKS_DOWNLOADED));
                }
                (Stage::Recording, bedrock::ID_RESOURCE_PACK_STACK) => {
                    out.push(self.pack(bedrock::ID_RESOURCE_PACK_RESPONSE, &PACKS_STACK_DONE));
                }
                (Stage::Recording, _) => {
                    if id == bedrock::ID_PLAY_STATUS && !self.complete {
                        self.complete = true;
                        // Comme le vrai jeu : « je suis apparu ». Le serveur officiel montre
                        // alors ce joueur aux autres.
                        let runtime_id = self.records.iter().find(|(i, _)| *i == 11).and_then(|(_, start)| {
                            let mut pos = 0;
                            bedrock::read_varu64(start, &mut pos)?; // identifiant unique (même longueur d'écriture)
                            bedrock::read_varu64(start, &mut pos)
                        });
                        if let Some(runtime_id) = runtime_id {
                            let mut body = Vec::new();
                            let mut v = runtime_id;
                            loop {
                                let byte = (v & 0x7f) as u8;
                                v >>= 7;
                                if v == 0 {
                                    body.push(byte);
                                    break;
                                }
                                body.push(byte | 0x80);
                            }
                            out.push(self.pack(113, &body));
                        }
                    }
                    if id == 11 {
                        // Le serveur attend la distance d'affichage pour envoyer la suite
                        out.push(self.pack(69, &[16, 16]));
                    }
                    if self.records.len() < 4096 {
                        self.records.push((id, body));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Paquets utiles trouvés : (nom, taille).
    pub fn found(&self) -> Vec<(&'static str, usize)> {
        WANTED
            .iter()
            .filter_map(|(id, name)| self.records.iter().rev().find(|(i, _)| i == id).map(|(_, b)| (*name, b.len())))
            .collect()
    }

    /// Range les paquets utiles dans `<dossier>/<protocole>/packets/`.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<PathBuf> {
        let dir = data_dir.join(self.protocol.to_string()).join("packets");
        std::fs::create_dir_all(&dir)?;
        for (id, name) in WANTED.iter() {
            if let Some((_, body)) = self.records.iter().rev().find(|(i, _)| i == id) {
                std::fs::write(dir.join(format!("{}.bin", name)), body)?;
            }
        }
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_witness_gets_other_device_identifiers() {
        let original = r#"{"ClientRandomId":-4521,"DeviceId":"abcdef07","GameVersion":"1.26.65","SelfSignedId":"3f2a-00c1","SkinData":"AAAA"}"#;
        let varied = vary_client_payload(&b64url(original.as_bytes()));
        let text = String::from_utf8(crate::json::base64_decode(&varied).unwrap()).unwrap();
        assert_eq!(text, r#"{"ClientRandomId":-4527,"DeviceId":"abcdef03","GameVersion":"1.26.65","SelfSignedId":"3f2a-00c7","SkinData":"AAAA"}"#);
        // Un modèle sans ces clés, ou illisible, est rendu tel quel
        let plain = b64url(br#"{"GameVersion":"1.26.65"}"#);
        assert_eq!(vary_client_payload(&plain), plain);
        assert_eq!(vary_client_payload("pas du base64 !"), "pas du base64 !");
        // Espaces autour des deux-points, valeur vide, et compte rendu sans aucune valeur
        let spaced = r#"{ "SelfSignedId" : "ab-12", "DeviceId":"", "SkinData":"x,\"y", "Nested":{"DeviceId":"zz"} }"#;
        let (varied, changed, keys) = vary_client_payload_report(&b64url(spaced.as_bytes()));
        let text = String::from_utf8(crate::json::base64_decode(&varied).unwrap()).unwrap();
        assert!(text.contains(r#""SelfSignedId" : "ab-17""#), "{}", text);
        assert_eq!(changed, vec!["SelfSignedId"]);
        // Identifiant d'installation vide : rempli, avec un numéro propre à chaque joueur
        let empty = b64url(br#"{"DeviceId":"d1","SelfSignedId":""}"#);
        let (one, changed, _) = vary_client_payload_with(&empty, false, 1);
        let text = String::from_utf8(crate::json::base64_decode(&one).unwrap()).unwrap();
        assert_eq!(text, r#"{"DeviceId":"d1","SelfSignedId":"b45a17c0-de00-4000-8000-000000000001"}"#);
        assert_eq!(changed, vec!["SelfSignedId (était vide)"]);
        assert_ne!(vary_client_payload_with(&empty, true, 2).0, one);
        assert_eq!(keys, vec!["SelfSignedId", "DeviceId", "SkinData", "Nested"]);
    }

    fn fake_sign(_: &[u8]) -> Vec<u8> {
        vec![7u8; 96]
    }

    #[test]
    fn login_is_readable_by_our_own_parser() {
        let payload = b64url(br#"{"GameVersion":"1.26.65","ThirdPartyName":"x"}"#);
        let body = build_login(2225, "BasaltCapture", "QUJD", &payload, &fake_sign, 1_800_000_000);
        let login = bedrock::parse_login(&body).unwrap();
        assert_eq!(login.protocol, 2225);
        assert_eq!(login.name, "BasaltCapture");
        assert_eq!(login.game_version, "1.26.65");
        assert_eq!(login.client_jwt.split('.').count(), 3);
    }

    fn server_batch(id: u32, body: &[u8], compression: bool) -> Vec<u8> {
        bedrock::encode_packet(id, body, compression)[1..].to_vec()
    }

    #[test]
    fn dialogue_reaches_recording_and_keeps_wanted_packets() {
        let mut c = Capture::new(2225, vec![1, 2, 3]);
        let hello = bedrock::decode_batch(&c.hello(), false, 1000).unwrap();
        assert_eq!(hello, vec![(bedrock::ID_REQUEST_NETWORK_SETTINGS, 2225i32.to_be_bytes().to_vec())]);
        // réglages réseau : seuil 1, compression zlib
        let out = c.on_message(&server_batch(bedrock::ID_NETWORK_SETTINGS, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0], false));
        assert_eq!(c.stage, Stage::Login);
        assert_eq!(bedrock::decode_batch(&out[0], true, 1000).unwrap()[0], (bedrock::ID_LOGIN, vec![1, 2, 3]));
        assert!(c.on_message(&server_batch(bedrock::ID_PLAY_STATUS, &[0, 0, 0, 0], true)).is_empty());
        assert_eq!(c.stage, Stage::Recording);
        let out = c.on_message(&server_batch(bedrock::ID_RESOURCE_PACKS_INFO, &[0; 22], true));
        assert_eq!(bedrock::decode_batch(&out[0], true, 1000).unwrap()[0].0, bedrock::ID_RESOURCE_PACK_RESPONSE);
        c.on_message(&server_batch(162, &[9, 9], true));
        c.on_message(&server_batch(52, &[1], true));
        c.on_message(&server_batch(58, &[0; 40], true));
        assert!(!c.complete);
        c.on_message(&server_batch(bedrock::ID_PLAY_STATUS, &[0, 0, 0, 3], true));
        assert!(c.complete);
        assert_eq!(c.found(), vec![("crafting_data", 1), ("item_registry", 2)]);
        let dir = std::env::temp_dir().join(format!("basalt-capture-test-{}", std::process::id()));
        let saved = c.save(&dir).unwrap();
        assert_eq!(std::fs::read(saved.join("item_registry.bin")).unwrap(), vec![9, 9]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chunked_bodies_are_reassembled() {
        assert_eq!(unchunk(b"3\r\nv=0\r\n5\r\n\r\no=-\r\n0\r\n\r\n"), b"v=0\r\no=-".to_vec());
        assert_eq!(unchunk(b"a;ext=1\r\n0123456789\r\n0\r\n\r\n"), b"0123456789".to_vec());
        // tronqué ou invalide : ce qui a pu être lu, sans planter
        assert_eq!(unchunk(b"ff\r\nabc"), Vec::<u8>::new());
        assert_eq!(unchunk(b"zz\r\nabc\r\n"), Vec::<u8>::new());
    }

    #[test]
    fn refusals_are_reported() {
        let mut c = Capture::new(2225, vec![]);
        c.on_message(&server_batch(bedrock::ID_NETWORK_SETTINGS, &[1, 0, 1, 0], false));
        assert_eq!(c.stage, Stage::Failed);
        assert!(c.refusal.as_ref().unwrap().contains("zlib"));
        let mut c = Capture::new(2225, vec![]);
        c.on_message(&server_batch(bedrock::ID_DISCONNECT, b"\x00\x00\x0bNot allowed", false));
        assert!(c.refusal.unwrap().contains("Not allowed"));
        let mut c = Capture::new(2225, vec![]);
        c.on_message(&[0xff, 0xff, 0xff]);
        assert_eq!(c.stage, Stage::Failed);
    }
}
