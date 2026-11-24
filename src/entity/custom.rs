//! Créatures créées par les plugins.
//!
//! Un plugin déclare une créature au nom nouveau à partir d'une créature du jeu
//! (`server.entities.create("monserveur:vache_doree", { looks = "minecraft:cow", health = 30 })`,
//! voir `docs/PLUGINS.md`). Elle **a l'apparence et le comportement** de celle dont elle part
//! (une vache se promène, se reproduit, suit le blé), et **ses propres valeurs** : vie,
//! vitesse, taille, butin, expérience, nom affiché.
//!
//! Côté serveur, c'est une créature de l'espèce de départ qui porte en plus le rang de sa
//! fiche (`Mob::custom`, voir `server/mobs.rs`) : tout ce que sait faire l'espèce reste
//! valable, et la fiche ne répond que pour ce qu'elle change. Côté joueur, rien n'est à
//! déclarer : le jeu reçoit une créature qu'il connaît, avec une taille et un nom.
//!
//! Une apparence propre (modèle et image du pack de ressources) viendra ensuite.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

/// Nombre de créatures que les plugins peuvent créer.
pub const MAX_ENTITIES: usize = 1000;

/// Fiche d'une créature créée par un plugin. `None` : la valeur de l'espèce de départ.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomEntity {
    /// Nom complet, avec son espace de noms (`monserveur:vache_doree`)
    pub name: String,
    /// Créature du jeu dont elle a l'apparence et le comportement (`minecraft:cow`)
    pub looks: String,
    /// Son apparence vient du pack de ressources, qui la définit sous son propre nom ; faux :
    /// elle a celle de la créature du jeu dont elle part
    pub own_model: bool,
    /// Nom affiché au-dessus d'elle (vide : aucun)
    pub display: String,
    /// Le nom reste visible de loin, sans viser la créature
    pub name_visible: bool,
    pub health: Option<f32>,
    /// Vitesse, à l'échelle du jeu (vache : 0,25 ; cheval : 0,1 à 0,34)
    pub speed: Option<f32>,
    /// Taille, par rapport à celle de l'espèce (2 : deux fois plus grande)
    pub scale: f32,
    /// Boîte de collision, en blocs : (largeur, hauteur). `None` : celle de l'espèce, à
    /// l'échelle de `scale`. Donnée, elle ne dépend plus de `scale`.
    pub hitbox: Option<(f32, f32)>,
    /// Butin à sa mort : (objet, quantité de … à …)
    pub drops: Option<Vec<(String, u8, u8)>>,
    /// Expérience rendue quand un joueur la tue (de … à …)
    pub xp: Option<(u32, u32)>,
}

impl CustomEntity {
    /// Lit la fiche envoyée par un plugin : une ligne `clé<TAB>valeur` par réglage.
    pub fn parse(name: &str, text: &str) -> Result<CustomEntity, String> {
        let (space, short) = name.split_once(':').ok_or("le nom doit avoir un espace de noms, par exemple « monserveur:vache_doree »")?;
        let plain = |s: &str| !s.is_empty() && s.len() <= 48 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !plain(space) || !plain(short) {
            return Err("le nom n'accepte que des minuscules, des chiffres et « _ », de part et d'autre de « : »".into());
        }
        if space == "minecraft" {
            return Err("l'espace de noms « minecraft » est réservé au jeu : choisissez le vôtre".into());
        }
        let mut def = CustomEntity { name: name.to_string(), looks: String::new(), own_model: false, display: String::new(), name_visible: true, health: None, speed: None, scale: 1.0, hitbox: None, drops: None, xp: None };
        let (mut width, mut height) = (None, None);
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (key, value) = line.split_once('\t').unwrap_or((line, ""));
            let number = || value.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| format!("{} attend un nombre positif, reçu « {} »", key, value));
            match key {
                "looks" => def.looks = if value.contains(':') { value.to_string() } else { format!("minecraft:{}", value) },
                "name" => def.display = value.chars().filter(|c| !c.is_control()).take(64).collect(),
                "ownModel" => {
                    def.own_model = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(format!("ownModel attend true ou false, reçu « {} »", value)),
                    }
                }
                "nameVisible" => {
                    def.name_visible = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(format!("nameVisible attend true ou false, reçu « {} »", value)),
                    }
                }
                "health" => def.health = Some((number()? as f32).clamp(1.0, 1024.0)),
                "speed" => def.speed = Some((number()? as f32).clamp(0.01, 1.0)),
                "scale" => def.scale = (number()? as f32).clamp(0.1, 10.0),
                "width" => width = Some((number()? as f32).clamp(0.1, 16.0)),
                "height" => height = Some((number()? as f32).clamp(0.1, 16.0)),
                "xp" => {
                    let n = number()?.min(1000.0) as u32;
                    def.xp = Some((n, def.xp.map(|x| x.1).unwrap_or(0).max(n)));
                }
                "xpMax" => {
                    let n = number()?.min(1000.0) as u32;
                    def.xp = Some((def.xp.map(|x| x.0).unwrap_or(0).min(n), n));
                }
                // « objet=2 », « objet=1-3 », séparés par des virgules ; vide : aucun butin
                "drops" => {
                    let mut drops = Vec::new();
                    for part in value.split(',').filter(|p| !p.is_empty()) {
                        let (item, amount) = part.split_once('=').unwrap_or((part, "1"));
                        let (lo, hi) = amount.split_once('-').unwrap_or((amount, amount));
                        let (lo, hi) = (lo.parse::<u8>(), hi.parse::<u8>());
                        match (lo, hi) {
                            (Ok(lo), Ok(hi)) if !item.is_empty() && lo <= hi && hi <= 64 && drops.len() < 16 => drops.push((item.to_string(), lo, hi)),
                            _ => return Err(format!("butin invalide : « {} » (16 objets au plus, quantités de 0 à 64)", part)),
                        }
                    }
                    def.drops = Some(drops);
                }
                _ => return Err(format!("réglage inconnu « {} »", key)),
            }
        }
        def.hitbox = match (width, height) {
            (Some(w), Some(h)) => Some((w, h)),
            (None, None) => None,
            _ => return Err("width et height vont ensemble : donnez les deux, en blocs".into()),
        };
        if def.looks.is_empty() {
            return Err("« looks » est obligatoire : la créature du jeu dont elle a l'apparence, par exemple \"minecraft:cow\"".into());
        }
        Ok(def)
    }
}

static ANY: AtomicBool = AtomicBool::new(false);
/// Les fiches, dans l'ordre de leur création : le rang d'une fiche ne change pas tant que le
/// serveur tourne, même si les plugins sont rechargés (la place est gardée, la fiche vidée).
static TABLE: RwLock<Vec<(String, Option<CustomEntity>)>> = RwLock::new(Vec::new());

/// Enregistre une créature (ou remplace celle du même nom) et renvoie son rang.
pub fn define(def: CustomEntity) -> Result<u16, String> {
    let mut table = TABLE.write().map_err(|_| "table des créatures indisponible".to_string())?;
    let rank = match table.iter().position(|(name, _)| *name == def.name) {
        Some(rank) => rank,
        None => {
            if table.len() >= MAX_ENTITIES {
                return Err(format!("trop de créatures créées ({} au plus)", MAX_ENTITIES));
            }
            table.push((def.name.clone(), None));
            table.len() - 1
        }
    };
    table[rank].1 = Some(def);
    ANY.store(true, Ordering::Relaxed);
    Ok(rank as u16)
}

/// Oublie les créatures créées (rechargement des plugins) ; leurs rangs restent réservés.
pub fn clear() {
    if let Ok(mut table) = TABLE.write() {
        for entry in table.iter_mut() {
            entry.1 = None;
        }
    }
    ANY.store(false, Ordering::Relaxed);
}

/// La fiche de ce rang, si la créature existe toujours.
pub fn get(rank: u16) -> Option<CustomEntity> {
    TABLE.read().ok()?.get(rank as usize)?.1.clone()
}

/// Le rang et la fiche de la créature créée sous ce nom complet.
pub fn find(name: &str) -> Option<(u16, CustomEntity)> {
    if !ANY.load(Ordering::Relaxed) || !name.contains(':') {
        return None;
    }
    let table = TABLE.read().ok()?;
    let rank = table.iter().position(|(n, _)| n == name)?;
    table[rank].1.clone().map(|def| (rank as u16, def))
}

/// Les créatures dont l'apparence vient du pack de ressources, à déclarer au jeu : (nom,
/// numéro d'entité, tiré de leur rang).
pub fn own_models() -> Vec<(String, i32)> {
    let Ok(table) = TABLE.read() else { return Vec::new() };
    table.iter().enumerate().filter_map(|(rank, (name, def))| def.as_ref().filter(|d| d.own_model).map(|_| (name.clone(), 10000 + rank as i32))).collect()
}

/// Les noms des créatures créées.
pub fn names() -> Vec<String> {
    TABLE.read().map(|t| t.iter().filter(|(_, d)| d.is_some()).map(|(n, _)| n.clone()).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_describe_new_creatures() {
        let text = "looks\tcow\nname\tVache dorée\nhealth\t30\nspeed\t0.3\nscale\t1.5\nxp\t5\nxpMax\t9\ndrops\tminecraft:gold_ingot=1-3,minecraft:leather";
        let cow = CustomEntity::parse("essai:vache_doree", text).unwrap();
        assert_eq!((cow.looks.as_str(), cow.display.as_str(), cow.health, cow.speed, cow.scale), ("minecraft:cow", "Vache dorée", Some(30.0), Some(0.3), 1.5));
        assert_eq!(cow.xp, Some((5, 9)));
        assert_eq!(cow.drops, Some(vec![("minecraft:gold_ingot".to_string(), 1, 3), ("minecraft:leather".to_string(), 1, 1)]));
        // Enregistrée : retrouvée par son nom et par son rang, qui ne change pas
        let rank = define(cow.clone()).unwrap();
        assert_eq!(find("essai:vache_doree"), Some((rank, cow.clone())));
        assert_eq!(get(rank), Some(cow.clone()));
        assert_eq!(define(cow).unwrap(), rank);
        assert!(names().contains(&"essai:vache_doree".to_string()));
        assert!(find("essai:inconnue").is_none() && find("cow").is_none());
        // Apparence du pack : la créature est déclarée au jeu sous son nom
        assert!(!own_models().iter().any(|(n, _)| n == "essai:vache_doree"));
        define(CustomEntity::parse("essai:gardien", "looks\tiron_golem\nownModel\ttrue").unwrap()).unwrap();
        assert!(own_models().iter().any(|(n, rid)| n == "essai:gardien" && *rid >= 10000));
        // Sans réglage : celles de l'espèce de départ ; un butin vide est un butin
        let plain = CustomEntity::parse("essai:simple", "looks\tminecraft:pig\ndrops\t").unwrap();
        assert_eq!((plain.health, plain.speed, plain.scale, plain.drops, plain.xp), (None, None, 1.0, Some(vec![]), None));
        // Boîte de collision propre : largeur et hauteur, en blocs, toujours ensemble
        assert_eq!(plain.hitbox, None);
        assert_eq!(CustomEntity::parse("essai:fine", "looks\tcow\nwidth\t0.4\nheight\t2.5").unwrap().hitbox, Some((0.4, 2.5)));
        assert!(CustomEntity::parse("essai:fine", "looks\tcow\nwidth\t0.4").is_err());
        // Noms et réglages refusés
        for (name, text) in [("vache", "looks\tcow"), ("minecraft:vache", "looks\tcow"), ("essai:x", ""), ("essai:x", "looks\tcow\ncouleur\tor"), ("essai:x", "looks\tcow\ndrops\tor=5-2"), ("essai:x", "looks\tcow\nhealth\tbeaucoup")] {
            assert!(CustomEntity::parse(name, text).is_err(), "{} {}", name, text);
        }
    }
}
