//! Apparence d'un joueur (skin), telle que son jeu l'annonce à la connexion.
//!
//! Le serveur la garde en mémoire pour la montrer aux autres joueurs. Tout est borné : un
//! skin incohérent ou démesuré est simplement ignoré (le joueur garde l'apparence par défaut).

use crate::json::{self, Value};

/// Taille maximale d'une image de skin ou de cape : 512 × 512 points.
const MAX_IMAGE: usize = 512 * 512 * 4;
/// Taille maximale de la description de la forme du personnage.
const MAX_GEOMETRY: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub id: String,
    /// Petit texte JSON qui désigne la forme à utiliser
    pub patch: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Points de l'image, quatre octets par point
    pub data: Vec<u8>,
    pub cape_width: u32,
    pub cape_height: u32,
    pub cape: Vec<u8>,
    pub cape_id: String,
    /// Forme du personnage (JSON)
    pub geometry: Vec<u8>,
    pub engine_version: Vec<u8>,
    pub animation_data: Vec<u8>,
    /// Bras larges (vrai) ou fins
    pub wide_arms: bool,
    /// Couleur de peau (AARRGGBB)
    pub colour: u32,
    pub premium: bool,
    pub persona: bool,
    pub cape_on_classic: bool,
    pub profile_hash: String,
}

impl Skin {
    /// Lit le skin dans le jeton « données client » du login. None s'il est absent ou invalide.
    pub fn from_client_jwt(jwt: &str) -> Option<Skin> {
        let payload = jwt.split('.').nth(1)?;
        let bytes = json::base64_decode(payload)?;
        let text = std::str::from_utf8(&bytes).ok()?;
        Skin::from_client_data(&json::parse(text).ok()?)
    }

    pub fn from_client_data(doc: &Value) -> Option<Skin> {
        let text = |key: &str, max: usize| -> String { doc.get(key).and_then(|v| v.as_str()).filter(|s| s.len() <= max && !s.chars().any(|c| c.is_control())).unwrap_or("").to_string() };
        let blob = |key: &str, max: usize| -> Option<Vec<u8>> {
            match doc.get(key).and_then(|v| v.as_str()) {
                Some(s) if s.len() <= max * 4 / 3 + 8 => json::base64_decode(s).filter(|b| b.len() <= max),
                Some(_) => None,
                None => Some(Vec::new()),
            }
        };
        let number = |key: &str| -> u32 {
            match doc.get(key) {
                Some(Value::Num(n)) if *n >= 0.0 && *n <= 4096.0 => *n as u32,
                _ => 0,
            }
        };
        let flag = |key: &str| matches!(doc.get(key), Some(Value::Bool(true)));

        let (width, height) = (number("SkinImageWidth"), number("SkinImageHeight"));
        let data = blob("SkinData", MAX_IMAGE)?;
        // L'image doit avoir exactement la taille annoncée
        if width == 0 || height == 0 || data.len() != (width as usize) * (height as usize) * 4 {
            return None;
        }
        let (mut cape_width, mut cape_height) = (number("CapeImageWidth"), number("CapeImageHeight"));
        let mut cape = blob("CapeData", MAX_IMAGE)?;
        if cape.len() != (cape_width as usize) * (cape_height as usize) * 4 {
            cape.clear();
            cape_width = 0;
            cape_height = 0;
        }
        let id = text("SkinId", 128);
        let patch = blob("SkinResourcePatch", 16 * 1024)?;
        if id.is_empty() || patch.is_empty() {
            return None;
        }
        // « #AARRGGBB »
        let colour = doc.get("SkinColor").and_then(|v| v.as_str()).and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok()).unwrap_or(0);
        Some(Skin {
            id,
            patch,
            width,
            height,
            data,
            cape_width,
            cape_height,
            cape,
            cape_id: text("CapeId", 128),
            geometry: blob("SkinGeometryData", MAX_GEOMETRY)?,
            engine_version: blob("SkinGeometryDataEngineVersion", 32)?,
            animation_data: blob("SkinAnimationData", 64 * 1024)?,
            wide_arms: doc.get("ArmSize").and_then(|v| v.as_str()) != Some("slim"),
            colour,
            premium: flag("PremiumSkin"),
            persona: flag("PersonaSkin"),
            cape_on_classic: flag("CapeOnClassicSkin"),
            profile_hash: text("ProfileHash", 128),
        })
    }

    /// Place occupée en mémoire, à peu près.
    pub fn size(&self) -> usize {
        self.data.len() + self.cape.len() + self.geometry.len() + self.patch.len() + self.animation_data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::base64_encode;

    fn b64(data: &[u8]) -> String {
        base64_encode(data, false, true)
    }

    fn client_data(width: u32, height: u32, points: usize) -> String {
        format!(
            r##"{{"ArmSize":"slim","CapeData":"","CapeId":"","CapeImageHeight":0,"CapeImageWidth":0,"PersonaSkin":false,"PremiumSkin":true,"ProfileHash":"abc=","SkinColor":"#ff7e5337","SkinData":"{}","SkinGeometryData":"{}","SkinGeometryDataEngineVersion":"{}","SkinId":"Standard_Custom","SkinImageHeight":{},"SkinImageWidth":{},"SkinResourcePatch":"{}"}}"##,
            b64(&vec![9u8; points * 4]),
            b64(b"{\"format_version\":\"1.14.0\"}"),
            b64(b"1.14.0"),
            height,
            width,
            b64(b"{\"geometry\":{\"default\":\"geometry.humanoid.customSlim\"}}")
        )
    }

    #[test]
    fn a_skin_is_read_from_the_login_data() {
        let doc = json::parse(&client_data(64, 64, 64 * 64)).unwrap();
        let skin = Skin::from_client_data(&doc).unwrap();
        assert_eq!((skin.width, skin.height, skin.data.len()), (64, 64, 16384));
        assert_eq!(skin.id, "Standard_Custom");
        assert!(!skin.wide_arms && skin.premium && !skin.persona);
        assert_eq!(skin.colour, 0xff7e5337);
        assert_eq!(skin.engine_version, b"1.14.0");
        assert_eq!(skin.profile_hash, "abc=");
        // Dans un jeton : en-tête, données, signature
        let jwt = format!("e30.{}.sig", base64_encode(client_data(64, 32, 64 * 32).as_bytes(), true, false));
        assert_eq!(Skin::from_client_jwt(&jwt).map(|s| s.height), Some(32));
    }

    #[test]
    fn inconsistent_or_oversized_skins_are_ignored() {
        // Image plus petite que la taille annoncée
        let doc = json::parse(&client_data(64, 64, 100)).unwrap();
        assert_eq!(Skin::from_client_data(&doc), None);
        // Dimensions nulles, données absentes, jeton illisible
        let doc = json::parse(&client_data(0, 0, 0)).unwrap();
        assert_eq!(Skin::from_client_data(&doc), None);
        assert_eq!(Skin::from_client_data(&json::parse("{}").unwrap()), None);
        assert_eq!(Skin::from_client_jwt("pas un jeton"), None);
        // Image démesurée (1024 × 1024)
        let doc = json::parse(&client_data(1024, 1024, 1024 * 1024)).unwrap();
        assert_eq!(Skin::from_client_data(&doc), None);
    }
}
