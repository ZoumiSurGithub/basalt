//! Début de connexion : réglages réseau, état de connexion, lecture du login.

#[allow(unused_imports)]
use super::*;
use crate::json::{self, Value};

/// PlayStatus : 0 login accepté, 3 joueur prêt à apparaître, autres valeurs : refus.
pub fn play_status(status: i32) -> Vec<u8> {
    status.to_be_bytes().to_vec()
}

/// NetworkSettings : seuil et méthode de compression.
pub fn network_settings(f: crate::versions::Features) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&256u16.to_le_bytes()); // seuil de compression
    b.extend_from_slice(&0u16.to_le_bytes()); // algorithme : zlib
    b.push(0); // limitation du client désactivée
    b.push(0);
    b.extend_from_slice(&0f32.to_le_bytes());
    if f.sctp_recovery_profile {
        b.push(0); // profil de récupération SCTP : par défaut
    }
    b
}

/// Message de déconnexion affiché au joueur.
pub fn disconnect(message: &str) -> Vec<u8> {
    let mut b = vec![0]; // raison (varint) : inconnue
    b.push(0); // le message est affiché
    write_string(&mut b, message);
    write_string(&mut b, message); // version filtrée
    b
}

#[derive(Debug, Clone)]
pub struct Login {
    pub protocol: i32,
    pub name: String,
    pub xuid: String,
    pub game_version: String,
    /// Jeton Xbox présenté par le client, s'il y en a un (non vérifié ici).
    pub token: Option<String>,
    /// Données du client (JWT signé par la clé du joueur).
    pub client_jwt: String,
}

/// Texte non vide d'un champ JSON.
pub(crate) fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string())
}

/// Lit le paquet Login. Les signatures ne sont pas vérifiées ici : le pseudo et
/// l'identifiant renvoyés sont ceux que le client annonce. La vérification est
/// faite par `auth::verify`, qui les remplace par l'identité prouvée.
pub fn parse_login(body: &[u8]) -> Result<Login, &'static str> {
    let proto = body.get(..4).ok_or("login tronqué")?;
    let protocol = i32::from_be_bytes([proto[0], proto[1], proto[2], proto[3]]);
    let mut pos = 4;
    let len = read_varu32(body, &mut pos).ok_or("login tronqué")? as usize;
    let data = body.get(pos..pos.checked_add(len).ok_or("login tronqué")?).ok_or("login tronqué")?;

    let le32 = |d: &[u8], at: usize| -> Option<usize> {
        d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let chain_len = le32(data, 0).ok_or("login tronqué")?;
    let chain = data.get(4..4usize.checked_add(chain_len).ok_or("login tronqué")?).ok_or("login tronqué")?;
    let at = 4 + chain_len;
    let client_len = le32(data, at).ok_or("login tronqué")?;
    let client = data
        .get(at + 4..(at + 4).checked_add(client_len).ok_or("login tronqué")?)
        .ok_or("login tronqué")?;

    let root = json::parse(std::str::from_utf8(chain).map_err(|_| "login : UTF-8 invalide")?)?;
    let mut jwts: Vec<String> = Vec::new();
    let mut add_chain = |v: &Value| {
        if let Some(items) = v.get("chain").and_then(|c| c.as_arr()) {
            for it in items.iter().take(8) {
                if let Some(s) = it.as_str() {
                    jwts.push(s.to_string());
                }
            }
        }
    };
    add_chain(&root);
    if let Some(cert) = root.get("Certificate").and_then(|c| c.as_str()) {
        if let Ok(inner) = json::parse(cert) {
            add_chain(&inner);
        }
    }
    let token = text(root.get("Token"));
    if let Some(tok) = &token {
        jwts.push(tok.clone());
    }

    let (mut name, mut xuid) = (None, None);
    for jwt in &jwts {
        if let Some(p) = json::jwt_payload(jwt) {
            let extra = p.get("extraData");
            name = name
                .or_else(|| text(extra.and_then(|e| e.get("displayName"))))
                .or_else(|| text(p.get("xname")));
            xuid = xuid
                .or_else(|| text(extra.and_then(|e| e.get("XUID"))))
                .or_else(|| text(p.get("xid")));
        }
    }

    let mut game_version = String::new();
    let client_jwt = std::str::from_utf8(client).map_err(|_| "login : UTF-8 invalide")?.to_string();
    if let Some(p) = json::jwt_payload(&client_jwt) {
        game_version = text(p.get("GameVersion")).unwrap_or_default();
        name = name.or_else(|| text(p.get("ThirdPartyName")));
    }

    let name = crate::log::clean(&name.ok_or("login : pseudo manquant")?, 32);
    if name.trim().is_empty() {
        return Err("login : pseudo invalide");
    }
    Ok(Login {
        protocol,
        name,
        xuid: crate::log::clean(&xuid.unwrap_or_default(), 32),
        game_version: crate::log::clean(&game_version, 24),
        token,
        client_jwt,
    })
}
