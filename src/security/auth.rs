//! Authentification Xbox Live.
//!
//! Un joueur connecté à son compte Microsoft présente un jeton (JWT) émis et signé
//! en RS256 par le service d'autorisation de Minecraft. Ce jeton contient son
//! identifiant Xbox (`xid`), son pseudo (`xname`) et une clé publique (`cpk`)
//! avec laquelle le jeu signe ensuite ses données de connexion.
//!
//! Le serveur vérifie donc : la signature du jeton avec les clés publiques de
//! Mojang, son émetteur, son audience, sa validité dans le temps, puis la
//! signature des données du client avec la clé `cpk`.
//!
//! Les clés publiques de Mojang sont téléchargées en HTTPS avec `curl`
//! (présent sur Windows 10+ et sur la plupart des Linux), puis mises en cache.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::crypto::{EcKey, RsaKey};
use crate::json::{self, Value};

pub const AUDIENCE: &str = "api://auth-minecraft-services/multiplayer";
const DISCOVERY: &str = "https://client.discovery.minecraft-services.net/api/v1.0/discovery/MinecraftPE/builds/";
const TRUSTED_DOMAIN: &str = ".minecraft-services.net";
/// Tolérance sur les horloges, en secondes.
const LEEWAY: u64 = 60;
const REFRESH: Duration = Duration::from_secs(30 * 60);
const MIN_REFRESH_GAP: Duration = Duration::from_secs(60);

/// Clés publiques du service d'autorisation.
pub struct KeySet {
    pub issuer: String,
    keys: Vec<(String, RsaKey)>,
}

impl KeySet {
    /// Construit le jeu de clés depuis un document JWKS (`{"keys":[…]}`).
    pub fn from_jwks(issuer: &str, jwks: &Value) -> Result<KeySet, String> {
        let list = jwks.get("keys").and_then(|k| k.as_arr()).ok_or("JWKS : liste « keys » absente")?;
        let mut keys = Vec::new();
        for k in list.iter().take(32) {
            let field = |name: &str| k.get(name).and_then(|v| v.as_str());
            if field("kty") != Some("RSA") {
                continue;
            }
            if let (Some(kid), Some(n), Some(e)) = (field("kid"), field("n"), field("e")) {
                let n = json::base64_decode(n).ok_or("JWKS : module illisible")?;
                let e = json::base64_decode(e).ok_or("JWKS : exposant illisible")?;
                match RsaKey::new(&n, &e) {
                    Ok(key) => keys.push((kid.to_string(), key)),
                    Err(why) => warn!("Clé d'authentification {} ignorée : {}", crate::log::clean(kid, 64), why),
                }
            }
        }
        if keys.is_empty() {
            return Err("JWKS : aucune clé RSA utilisable".into());
        }
        Ok(KeySet { issuer: issuer.to_string(), keys })
    }

    /// Lit un fichier `{"issuer":"…","keys":[…]}` (cache, ou fichier fourni par l'administrateur).
    pub fn load(path: &Path) -> Result<KeySet, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{} : {}", path.display(), e))?;
        let doc = json::parse(&text).map_err(|e| e.to_string())?;
        let issuer = doc.get("issuer").and_then(|v| v.as_str()).ok_or("fichier de clés : « issuer » absent")?;
        KeySet::from_jwks(issuer, &doc)
    }

    /// Nombre de clés de confiance.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[derive(Debug, PartialEq)]
pub enum AuthError {
    /// Le client n'a présenté aucun jeton : joueur non connecté à Xbox Live.
    NotSignedIn,
    /// Jeton signé par une clé que le serveur ne connaît pas (encore).
    UnknownKey,
    /// Jeton ou données de connexion invalides.
    Invalid(&'static str),
}

/// Identité vérifiée d'un joueur.
#[derive(Debug, PartialEq)]
pub struct Identity {
    pub name: String,
    pub xuid: String,
    /// Clé publique du joueur (base64 d'une clé P-384), telle qu'inscrite dans son jeton.
    pub cpk: String,
}

struct Jwt<'a> {
    header: Value,
    payload: Value,
    signed: &'a [u8],
    signature: Vec<u8>,
}

/// Découpe un jeton JWT en en-tête, contenu et signature.
fn split_jwt(token: &str) -> Result<Jwt<'_>, AuthError> {
    let bad = AuthError::Invalid("jeton mal formé");
    if token.len() > 4 * 1024 * 1024 {
        return Err(bad);
    }
    let mut parts = token.split('.');
    let (h, p, s) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(s), None) => (h, p, s),
        _ => return Err(bad),
    };
    let decode = |part: &str| -> Option<Value> {
        let bytes = json::base64_decode(part)?;
        json::parse(std::str::from_utf8(&bytes).ok()?).ok()
    };
    Ok(Jwt {
        header: decode(h).ok_or(AuthError::Invalid("jeton mal formé"))?,
        payload: decode(p).ok_or(AuthError::Invalid("jeton mal formé"))?,
        signed: &token.as_bytes()[..h.len() + 1 + p.len()],
        signature: json::base64_decode(s).ok_or(AuthError::Invalid("jeton mal formé"))?,
    })
}

/// Nombre entier positif d'un champ JSON (dates des jetons).
fn number(v: Option<&Value>) -> Option<u64> {
    match v {
        Some(Value::Num(n)) if *n >= 0.0 && *n < 1e15 => Some(*n as u64),
        _ => None,
    }
}

/// Vérifie le jeton Xbox seul : signature, émetteur, audience, dates, identité.
/// Renvoie l'identité et la clé publique du joueur.
fn verify_token(token: &str, keys: &KeySet, now: u64) -> Result<(Identity, EcKey), AuthError> {
    let jwt = split_jwt(token)?;
    let text = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string());

    // 1. Signature RS256 par une clé connue. Tout autre algorithme est refusé
    //    (« none », HS256…), ce qui ferme les attaques par confusion d'algorithme.
    if text(&jwt.header, "alg").as_deref() != Some("RS256") {
        return Err(AuthError::Invalid("algorithme de signature refusé"));
    }
    let kid = text(&jwt.header, "kid").ok_or(AuthError::Invalid("jeton sans identifiant de clé"))?;
    let key = keys.keys.iter().find(|(k, _)| *k == kid).map(|(_, k)| k).ok_or(AuthError::UnknownKey)?;
    if !key.verify_sha256(jwt.signed, &jwt.signature) {
        return Err(AuthError::Invalid("signature du jeton invalide"));
    }

    // 2. Émetteur, audience et dates
    if text(&jwt.payload, "iss").as_deref() != Some(keys.issuer.as_str()) {
        return Err(AuthError::Invalid("émetteur du jeton inattendu"));
    }
    let aud_ok = match jwt.payload.get("aud") {
        Some(Value::Str(a)) => a == AUDIENCE,
        Some(Value::Arr(list)) => list.iter().any(|a| a.as_str() == Some(AUDIENCE)),
        _ => false,
    };
    if !aud_ok {
        return Err(AuthError::Invalid("jeton destiné à un autre service"));
    }
    let exp = number(jwt.payload.get("exp")).ok_or(AuthError::Invalid("jeton sans date d'expiration"))?;
    if exp + LEEWAY < now {
        return Err(AuthError::Invalid("jeton expiré"));
    }
    if let Some(nbf) = number(jwt.payload.get("nbf")) {
        if nbf > now + LEEWAY {
            return Err(AuthError::Invalid("jeton pas encore valide"));
        }
    }

    // 3. Identité
    let xuid = text(&jwt.payload, "xid").unwrap_or_default();
    if xuid.is_empty() || xuid.len() > 20 || !xuid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AuthError::Invalid("identifiant Xbox absent ou invalide"));
    }
    let name = crate::log::clean(&text(&jwt.payload, "xname").unwrap_or_default(), 32);
    if name.trim().is_empty() {
        return Err(AuthError::Invalid("pseudo absent du jeton"));
    }

    // 4. Clé publique du joueur
    let cpk = text(&jwt.payload, "cpk").ok_or(AuthError::Invalid("clé du client absente du jeton"))?;
    let der = json::base64_decode(&cpk).ok_or(AuthError::Invalid("clé du client illisible"))?;
    let client_key = EcKey::from_spki(&der).map_err(AuthError::Invalid)?;
    Ok((Identity { name, xuid, cpk }, client_key))
}

/// Vérifie le jeton Xbox d'un joueur puis la signature de ses données de connexion.
/// `now` est l'heure courante en secondes Unix.
pub fn verify(token: Option<&str>, client_jwt: &str, keys: &KeySet, now: u64) -> Result<Identity, AuthError> {
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => return Err(AuthError::NotSignedIn),
    };
    let (identity, client_key) = verify_token(token, keys, now)?;
    // Les données du client doivent être signées (ES384) par la clé du jeton
    let client = split_jwt(client_jwt)?;
    let alg = client.header.get("alg").and_then(|a| a.as_str());
    if alg != Some("ES384") {
        return Err(AuthError::Invalid("données du client : algorithme refusé"));
    }
    if !client_key.verify_sha384(client.signed, &client.signature) {
        return Err(AuthError::Invalid("données du client non signées par la clé du jeton"));
    }
    Ok(identity)
}

/// JSON canonique des empreintes DTLS d'une description SDP : c'est ce texte que
/// chaque partie signe pour prouver qu'elle possède le certificat de la liaison.
pub fn canonical_fingerprints(fingerprints: &[(String, String)]) -> String {
    let items: Vec<String> = fingerprints
        .iter()
        .map(|(algorithm, digest)| {
            format!(
                "{{\"algorithm\":{},\"digest\":{}}}",
                crate::signaling::json_string(algorithm),
                crate::signaling::json_string(digest)
            )
        })
        .collect();
    format!("{{\"fingerprint\":[{}]}}", items.join(","))
}

/// Vérifie l'attribut `a=identity` d'une offre NetherNet : jeton Xbox valide, et
/// empreintes DTLS de l'offre signées par la clé du joueur. Cette signature lie
/// l'identité à la liaison chiffrée, ce qui empêche de rejouer le login d'un autre.
pub fn verify_offer(
    identity: &str,
    fingerprints: &[(String, String)],
    keys: &KeySet,
    now: u64,
) -> Result<Identity, AuthError> {
    let bad = AuthError::Invalid("identité de l'offre mal formée");
    if identity.len() > 64 * 1024 || fingerprints.is_empty() || fingerprints.len() > 4 {
        return Err(bad);
    }
    let parse = |bytes: Vec<u8>| -> Option<Value> { json::parse(std::str::from_utf8(&bytes).ok()?).ok() };
    let envelope = json::base64_decode(identity.trim()).and_then(parse).ok_or(AuthError::Invalid("identité de l'offre mal formée"))?;
    let assertion = envelope
        .get("assertion")
        .and_then(|a| a.as_str())
        .and_then(|a| json::parse(a).ok())
        .ok_or(AuthError::Invalid("identité de l'offre mal formée"))?;
    let token = assertion.get("token").and_then(|t| t.as_str()).ok_or(AuthError::NotSignedIn)?;
    let jws = assertion
        .get("fingerprints")
        .and_then(|t| t.as_str())
        .ok_or(AuthError::Invalid("identité de l'offre mal formée"))?;
    let (identity, client_key) = verify_token(token, keys, now)?;

    // JWS détaché : en-tête, charge utile omise, signature
    let mut parts = jws.split('.');
    let (header, signature) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(""), Some(s), None) => (h, s),
        _ => return Err(bad),
    };
    let alg = json::base64_decode(header).and_then(parse);
    if alg.as_ref().and_then(|h| h.get("alg")).and_then(|a| a.as_str()) != Some("ES384") {
        return Err(AuthError::Invalid("signature des empreintes : algorithme refusé"));
    }
    let signature = json::base64_decode(signature).ok_or(AuthError::Invalid("identité de l'offre mal formée"))?;
    let payload = json::base64_encode(canonical_fingerprints(fingerprints).as_bytes(), true, false);
    let signed = format!("{}.{}", header, payload);
    if !client_key.verify_sha384(signed.as_bytes(), &signature) {
        return Err(AuthError::Invalid("empreintes de l'offre non signées par la clé du joueur"));
    }
    Ok(identity)
}

/// Construit la valeur de `a=identity` d'une réponse SDP : un jeton auto-signé qui
/// porte la clé publique du serveur, et la signature des empreintes DTLS de la
/// réponse. `sign` signe en ES384 et renvoie r || s (96 octets) ; `cpk` est la clé
/// publique déjà formatée en JSON (objet JWK ou chaîne).
pub fn build_server_assertion(
    sign: &dyn Fn(&[u8]) -> Vec<u8>,
    cpk: &str,
    domain: &str,
    fingerprints: &[(String, String)],
    now: u64,
) -> String {
    let b = |d: &[u8]| json::base64_encode(d, true, false);
    let header = b(br#"{"alg":"ES384","typ":"JWT"}"#);
    let claims = format!("{{\"cpk\":{},\"exp\":{},\"iat\":{}}}", cpk, now + 3600, now);
    let signing = format!("{}.{}", header, b(claims.as_bytes()));
    let token = format!("{}.{}", signing, b(&sign(signing.as_bytes())));

    let jws_header = b(br#"{"alg":"ES384"}"#);
    let payload = b(canonical_fingerprints(fingerprints).as_bytes());
    let jws = format!("{}..{}", jws_header, b(&sign(format!("{}.{}", jws_header, payload).as_bytes())));

    let assertion = format!("{{\"fingerprints\":\"{}\",\"token\":\"{}\"}}", jws, token);
    let envelope = format!(
        "{{\"assertion\":{},\"idp\":{{\"domain\":{},\"protocol\":\"default\"}}}}",
        crate::signaling::json_string(&assertion),
        crate::signaling::json_string(domain)
    );
    json::base64_encode(envelope.as_bytes(), false, true)
}

// ------------------------------------------------ Téléchargement des clés ----

/// N'accepte que des adresses HTTPS du domaine des services Minecraft.
fn trusted_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| "adresse non HTTPS refusée".to_string())?;
    let host = rest.split(|c| c == '/' || c == ':' || c == '?' || c == '#').next().unwrap_or("");
    let clean = url.len() < 300 && url.bytes().all(|b| b.is_ascii_graphic());
    if !clean || !host.ends_with(TRUSTED_DOMAIN) || url.contains('@') || url.contains('\\') {
        return Err(format!("adresse hors du domaine de confiance : {}", crate::log::clean(url, 100)));
    }
    Ok(())
}

/// Télécharge du JSON avec curl (adresses de confiance seulement).
fn curl(url: &str) -> Result<Value, String> {
    trusted_url(url)?;
    let out = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-time",
            "15",
            "--max-filesize",
            "1048576",
            "-H",
            "Accept: application/json",
        ])
        .arg(url)
        .output()
        .map_err(|e| format!("impossible de lancer curl ({})", e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("échec du téléchargement : {}", crate::log::clean(err.trim(), 200)));
    }
    let text = std::str::from_utf8(&out.stdout).map_err(|_| "réponse illisible".to_string())?;
    json::parse(text).map_err(|e| e.to_string())
}

/// Découverte -> configuration OpenID -> JWKS. Renvoie le jeu de clés et le texte à mettre en cache.
fn fetch(game_version: &str) -> Result<(KeySet, String), String> {
    let version: String = game_version.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let disc = curl(&format!("{}{}", DISCOVERY, version))?;
    let issuer = disc
        .get("result")
        .and_then(|v| v.get("serviceEnvironments"))
        .and_then(|v| v.get("auth"))
        .and_then(|v| v.get("prod"))
        .and_then(|v| v.get("issuer"))
        .and_then(|v| v.as_str())
        .ok_or("découverte : émetteur introuvable dans la réponse")?
        .to_string();
    let conf = curl(&format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/')))?;
    let issuer = conf.get("issuer").and_then(|v| v.as_str()).unwrap_or(&issuer).to_string();
    let jwks_uri = conf.get("jwks_uri").and_then(|v| v.as_str()).ok_or("configuration OpenID : jwks_uri absent")?;
    let jwks = curl(jwks_uri)?;
    let keys = KeySet::from_jwks(&issuer, &jwks)?;

    // Cache : seuls les champs utiles, ré-encodés à la main
    let mut cache = format!("{{\"issuer\":\"{}\",\"keys\":[", issuer.replace('"', ""));
    let mut first = true;
    for k in jwks.get("keys").and_then(|k| k.as_arr()).unwrap_or(&[]) {
        let f = |n: &str| k.get(n).and_then(|v| v.as_str()).unwrap_or("").replace('"', "").replace('\\', "");
        if f("kty") != "RSA" {
            continue;
        }
        if !first {
            cache.push(',');
        }
        first = false;
        cache.push_str(&format!("{{\"kty\":\"RSA\",\"kid\":\"{}\",\"n\":\"{}\",\"e\":\"{}\"}}", f("kid"), f("n"), f("e")));
    }
    cache.push_str("]}");
    Ok((keys, cache))
}

/// Tâche de fond qui tient les clés à jour sans jamais bloquer le serveur.
pub struct KeyFetcher {
    rx: Receiver<Result<KeySet, String>>,
    trigger: Sender<()>,
}

impl KeyFetcher {
    /// Fil qui récupère les clés Xbox au démarrage puis les rafraîchit.
    pub fn spawn(game_version: String, cache: PathBuf) -> KeyFetcher {
        let (tx, rx) = mpsc::channel();
        let (trigger, wake) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut retry = Duration::from_secs(15);
            loop {
                let started = Instant::now();
                let wait = match fetch(&game_version) {
                    Ok((keys, text)) => {
                        let _ = std::fs::write(&cache, text);
                        if tx.send(Ok(keys)).is_err() {
                            return;
                        }
                        retry = Duration::from_secs(15);
                        REFRESH
                    }
                    Err(e) => {
                        if tx.send(Err(e)).is_err() {
                            return;
                        }
                        retry = (retry * 2).min(Duration::from_secs(300));
                        retry
                    }
                };
                match wake.recv_timeout(wait) {
                    Ok(()) => {
                        // Rafraîchissement demandé (clé inconnue) : pas plus d'un par minute
                        while wake.try_recv().is_ok() {}
                        let since = started.elapsed();
                        if since < MIN_REFRESH_GAP {
                            std::thread::sleep(MIN_REFRESH_GAP - since);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        KeyFetcher { rx, trigger }
    }

    /// Résultat d'un téléchargement terminé, s'il y en a un.
    pub fn poll(&self) -> Option<Result<KeySet, String>> {
        self.rx.try_recv().ok()
    }

    /// Demande un rafraîchissement anticipé.
    pub fn refresh(&self) {
        let _ = self.trigger.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::trusted_url;

    #[test]
    fn only_trusted_https_hosts() {
        assert!(trusted_url("https://authorization.franchise.minecraft-services.net/.well-known/keys").is_ok());
        assert!(trusted_url("http://authorization.franchise.minecraft-services.net/keys").is_err());
        assert!(trusted_url("https://evil.example/minecraft-services.net").is_err());
        assert!(trusted_url("https://minecraft-services.net.evil.example/keys").is_err());
        assert!(trusted_url("https://user@evil.example/.minecraft-services.net").is_err());
        assert!(trusted_url("https://a.minecraft-services.net/x y").is_err());
        assert!(trusted_url("https://evil.example\\@a.minecraft-services.net/keys").is_err());
    }
}
