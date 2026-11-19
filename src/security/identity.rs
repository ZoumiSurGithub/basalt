//! Identité de l'opérateur du serveur pour NetherNet.
//!
//! Le jeu exige que chaque réponse du serveur soit signée par une clé P-384 qui
//! lui appartient. Sur une connexion HTTP simple, le joueur voit une demande de
//! confiance la première fois, puis sa machine retient la clé : il faut donc la
//! conserver d'un redémarrage à l'autre, et ne jamais la publier.
//!
//! La signature utilise la bibliothèque `p384`, écrite pour ne pas laisser fuir
//! la clé privée par les temps de calcul.

use std::path::Path;

use p384::ecdsa::signature::Signer;
use p384::ecdsa::{Signature, SigningKey};

use crate::crypto::{os_random, sha256};
use crate::json::base64_encode;

const SPKI_PREFIX: [u8; 24] = [
    0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b, 0x81, 0x04, 0x00,
    0x22, 0x03, 0x62, 0x00, 0x04,
];

pub struct ServerIdentity {
    key: SigningKey,
    /// Clé publique au format attendu dans le jeton (objet JWK, ou chaîne base64)
    cpk: String,
    domain: String,
    fingerprint: String,
    /// Clé publique au format SPKI, en base64 (utilisée par l'outil de capture)
    spki_b64: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let t = text.trim();
    if t.len() % 2 != 0 {
        return None;
    }
    (0..t.len() / 2).map(|i| u8::from_str_radix(t.get(2 * i..2 * i + 2)?, 16).ok()).collect()
}

impl ServerIdentity {
    /// Charge la clé du serveur, ou la crée au premier démarrage.
    pub fn load_or_generate(path: &Path, domain: &str, cpk_as_jwk: bool) -> Result<ServerIdentity, String> {
        let key = match std::fs::read_to_string(path) {
            Ok(text) => {
                let secret = unhex(&text).ok_or_else(|| format!("{} : contenu illisible", path.display()))?;
                SigningKey::from_slice(&secret).map_err(|_| format!("{} : clé invalide", path.display()))?
            }
            Err(_) => {
                let mut found = None;
                for _ in 0..16 {
                    let mut secret = [0u8; 48];
                    if !os_random(&mut secret) {
                        return Err("aucune source d'aléa disponible sur ce système".into());
                    }
                    if let Ok(k) = SigningKey::from_slice(&secret) {
                        std::fs::write(path, hex(&secret)).map_err(|e| format!("{} : {}", path.display(), e))?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                        }
                        found = Some(k);
                        break;
                    }
                }
                found.ok_or("impossible de générer la clé du serveur")?
            }
        };
        let point = key.verifying_key().to_encoded_point(false);
        let raw = point.as_bytes(); // 0x04 || X || Y
        if raw.len() != 97 {
            return Err("clé publique du serveur inattendue".into());
        }
        let (x, y) = (&raw[1..49], &raw[49..97]);
        let mut spki = SPKI_PREFIX.to_vec();
        spki.extend_from_slice(x);
        spki.extend_from_slice(y);
        let cpk = if cpk_as_jwk {
            format!(
                "{{\"crv\":\"P-384\",\"kty\":\"EC\",\"x\":\"{}\",\"y\":\"{}\"}}",
                base64_encode(x, true, false),
                base64_encode(y, true, false)
            )
        } else {
            format!("\"{}\"", base64_encode(&spki, false, true))
        };
        let spki_b64 = base64_encode(&spki, false, true);
        Ok(ServerIdentity { key, cpk, domain: domain.to_string(), fingerprint: hex(&sha256(&spki)[..8]), spki_b64 })
    }

    /// Signe un message en ES384 : renvoie r || s (96 octets).
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let signature: Signature = self.key.sign(message);
        signature.to_bytes().to_vec()
    }

    pub fn spki_b64(&self) -> &str {
        &self.spki_b64
    }

    /// Empreinte courte de la clé, à communiquer aux joueurs pour qu'ils la reconnaissent.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Valeur de `a=identity` pour une réponse dont les empreintes DTLS sont données.
    pub fn assertion(&self, fingerprints: &[(String, String)], now: u64) -> String {
        let sign = |message: &[u8]| -> Vec<u8> {
            let signature: Signature = self.key.sign(message);
            signature.to_bytes().to_vec()
        };
        crate::auth::build_server_assertion(&sign, &self.cpk, &self.domain, fingerprints, now)
    }
}
