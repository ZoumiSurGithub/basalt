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
    /// Hostile : elle s'en prend aux joueurs qu'elle voit. `None` : comme son espèce de départ
    /// (un zombie attaque, une vache non)
    pub hostile: Option<bool>,
    /// Dégâts d'un de ses coups, en points de vie
    pub attack_damage: Option<f32>,
    /// Distance, en blocs, à laquelle elle repère un joueur
    pub sight: Option<f32>,
    /// Tireur : (projectile, dégâts, ticks entre deux tirs, portée en blocs)
    pub ranged: Option<(String, f32, u32, f32)>,
    /// Explose au contact d'un joueur, comme un creeper : la puissance (3 pour un creeper)
    pub explosion: Option<f32>,
    /// Potion que contiennent ses projectiles (variante), quand elle lance des potions
    pub potion: Option<u16>,
    /// Son nom dans le message de mort d'un joueur (« tué par … »)
    pub cause: &'static str,
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
        let mut def = CustomEntity { name: name.to_string(), looks: String::new(), own_model: false, display: String::new(), name_visible: true, health: None, speed: None, scale: 1.0, hitbox: None, drops: None, xp: None, hostile: None, attack_damage: None, sight: None, ranged: None, explosion: None, potion: None, cause: "" };
        let (mut width, mut height) = (None, None);
        let (mut projectile, mut shot_damage, mut shot_every, mut shot_range) = (None, None, None, None);
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
                "hostile" => {
                    def.hostile = Some(match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(format!("hostile attend true ou false, reçu « {} »", value)),
                    })
                }
                "attackDamage" => def.attack_damage = Some((number()? as f32).clamp(0.0, 1000.0)),
                "sight" => def.sight = Some((number()? as f32).clamp(1.0, 64.0)),
                "projectile" => {
                    let ok = !value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b':');
                    if !ok {
                        return Err(format!("projectile attend une entité du jeu, par exemple \"minecraft:arrow\", reçu « {} »", value));
                    }
                    projectile = Some(if value.contains(':') { value.to_string() } else { format!("minecraft:{}", value) });
                }
                "projectileDamage" => shot_damage = Some((number()? as f32).clamp(0.0, 1000.0)),
                "shootInterval" => shot_every = Some(((number()? * 20.0) as u32).clamp(5, 12_000)),
                "shootRange" => shot_range = Some((number()? as f32).clamp(2.0, 48.0)),
                "potion" => {
                    def.potion = Some(crate::item::potion::potion_id(value).ok_or_else(|| format!("potion inconnue « {} » (poison, slowness, weakness, harming, healing, regeneration…)", value))?);
                }
                "explosion" => def.explosion = Some((number()? as f32).clamp(0.5, 8.0)),
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
        def.ranged = match projectile {
            Some(kind) => Some((kind, shot_damage.unwrap_or(3.0), shot_every.unwrap_or(40), shot_range.unwrap_or(15.0))),
            None if shot_damage.is_some() || shot_every.is_some() || shot_range.is_some() => {
                return Err("projectileDamage, shootInterval et shootRange demandent « projectile » (par exemple \"minecraft:arrow\")".into());
            }
            None => None,
        };
        if def.potion.is_some() && !def.ranged.as_ref().map(|r| r.0.contains("potion")).unwrap_or(false) {
            return Err("potion demande projectile = \"minecraft:splash_potion\"".into());
        }
        if def.hostile == Some(false) && (def.ranged.is_some() || def.explosion.is_some()) {
            return Err("projectile et explosion n'ont pas de sens avec hostile = false".into());
        }
        if def.hostile == Some(false) && (def.attack_damage.is_some() || def.sight.is_some()) {
            return Err("attackDamage et sight n'ont pas de sens avec hostile = false".into());
        }
        // (ce nom vit autant que le serveur : une poignée d'octets par créature créée)
        let shown = if def.display.is_empty() { short.replace('_', " ") } else { def.display.clone() };
        def.cause = Box::leak(shown.into_boxed_str());
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

/// Les créatures à déclarer au jeu : (nom, numéro d'entité, tiré de leur rang). Toutes le
/// sont, pour que `/summon` les propose et les accepte chez le joueur ; seules celles dont
/// l'apparence vient du pack sont ensuite annoncées sous ce nom quand elles apparaissent.
pub fn declared() -> Vec<(String, i32)> {
    let Ok(table) = TABLE.read() else { return Vec::new() };
    table.iter().enumerate().filter_map(|(rank, (name, def))| def.as_ref().map(|_| (name.clone(), 10000 + rank as i32))).collect()
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
        // Toutes sont déclarées au jeu, qu'elles aient leur apparence ou celle d'une autre
        define(CustomEntity::parse("essai:gardien", "looks\tiron_golem\nownModel\ttrue").unwrap()).unwrap();
        let declared = declared();
        assert!(declared.iter().any(|(n, rid)| n == "essai:gardien" && *rid >= 10000));
        assert!(declared.iter().any(|(n, _)| n == "essai:vache_doree"));
        // Sans réglage : celles de l'espèce de départ ; un butin vide est un butin
        let plain = CustomEntity::parse("essai:simple", "looks\tminecraft:pig\ndrops\t").unwrap();
        assert_eq!((plain.health, plain.speed, plain.scale, plain.drops, plain.xp), (None, None, 1.0, Some(vec![]), None));
        // Hostile : dégâts, vue, et le nom qui paraîtra dans le message de mort
        let ogre = CustomEntity::parse("essai:ogre", "looks\tcow\nname\tOgre\nhostile\ttrue\nattackDamage\t6\nsight\t20").unwrap();
        assert_eq!((ogre.hostile, ogre.attack_damage, ogre.sight, ogre.cause), (Some(true), Some(6.0), Some(20.0), "Ogre"));
        assert_eq!((plain.hostile, plain.cause), (None, "simple"));
        assert!(CustomEntity::parse("essai:x", "looks\tzombie\nhostile\tfalse\nattackDamage\t4").is_err());
        // Tireur et explosif
        let archer = CustomEntity::parse("essai:archer", "looks\tzombie\nprojectile\tsnowball\nprojectileDamage\t2\nshootInterval\t1.5\nshootRange\t20").unwrap();
        assert_eq!(archer.ranged, Some(("minecraft:snowball".to_string(), 2.0, 30, 20.0)));
        let bomb = CustomEntity::parse("essai:bombe", "looks\tcow\nhostile\ttrue\nexplosion\t2").unwrap();
        assert_eq!((bomb.explosion, bomb.ranged.is_none()), (Some(2.0), true));
        assert!(CustomEntity::parse("essai:x", "looks\tcow\nshootRange\t5").is_err());
        // Lanceur de potions
        let healer = CustomEntity::parse("essai:soigneur", "looks\tvillager\nprojectile\tsplash_potion\npotion\tstrong_healing").unwrap();
        assert_eq!(healer.potion, Some(22));
        assert!(CustomEntity::parse("essai:x", "looks\tcow\nprojectile\tarrow\npotion\tpoison").is_err());
        assert!(CustomEntity::parse("essai:x", "looks\tcow\nprojectile\tsplash_potion\npotion\tsoupe").is_err());
        assert!(CustomEntity::parse("essai:x", "looks\tcow\nhostile\tfalse\nexplosion\t2").is_err());
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
