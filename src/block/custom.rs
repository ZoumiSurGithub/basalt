//! Blocs créés par les plugins.
//!
//! Le pendant de `item/custom.rs` : un plugin déclare un bloc au nom nouveau
//! (`server.blocks.create("monserveur:minerai_rubis", { texture = "minerai_rubis" })`, voir
//! `docs/PLUGINS.md`). Trois choses en découlent :
//!
//! - côté serveur, le bloc rejoint les blocs connus avec ses propriétés (dureté, lumière) :
//!   `VanillaBlocks` en fait un bloc ordinaire, que le monde pose, casse et sauvegarde comme
//!   les autres. Son butin se règle par `block/overrides.rs` ;
//! - côté joueur, le bloc est **déclaré au jeu** dans le paquet d'entrée dans le monde
//!   (`definition`) : image, dureté, lumière. Le jeu les applique lui-même, donc la dureté
//!   d'un bloc créé est réellement réglable, ce qu'on ne peut pas faire pour un bloc du jeu ;
//! - l'objet qui pose le bloc porte le même nom ; son identifiant est négatif, comme pour
//!   les blocs du jeu.
//!
//! Le numéro d'un bloc sur le réseau est le hachage de son nom (`bedrock::block_hash`),
//! comme pour les blocs du jeu : ajouter un bloc ne décale donc rien.
//!
//! L'image vient d'un pack de ressources (`textures/terrain_texture.json`), ou d'un bloc du
//! jeu (`stone`, `diamond_block`…).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use super::block::{BaseBlock, Block};
use crate::item::{ToolTier, ToolType};
use crate::nbt::Tag;

/// Premier numéro de bloc donné aux blocs des plugins (ceux du jeu sont bien en dessous).
pub const FIRST_BLOCK_ID: i32 = 10000;
/// Nombre de blocs que les plugins peuvent créer.
pub const MAX_BLOCKS: usize = 2000;

/// Ce qui distingue les états d'un bloc créé. Un bloc sans états n'en a qu'un.
#[derive(Debug, Clone, PartialEq)]
pub enum Variants {
    None,
    /// Couché selon l'axe de la face cliquée, comme une bûche : debout, est-ouest, nord-sud
    Axis,
    /// Tourné vers le joueur qui le pose, comme un four : nord, est, sud, ouest
    Facing,
    /// Stades de croissance, chacun avec son image ; le dernier est la maturité
    Stages(Vec<String>),
}

impl Variants {
    /// Nombre d'états du bloc.
    pub fn count(&self) -> usize {
        match self {
            Variants::None => 1,
            Variants::Axis => 3,
            Variants::Facing => 4,
            Variants::Stages(images) => images.len(),
        }
    }

    /// Nom de la propriété qui porte l'état, telle que déclarée au jeu.
    pub fn property(&self) -> Option<&'static str> {
        match self {
            Variants::None => None,
            Variants::Axis => Some("basalt:axis"),
            Variants::Facing => Some("basalt:facing"),
            Variants::Stages(_) => Some("basalt:stage"),
        }
    }
}

/// Fiche d'un bloc créé par un plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomBlock {
    /// Nom complet, avec son espace de noms (`monserveur:minerai_rubis`)
    pub name: String,
    /// Nom affiché au joueur
    pub display: String,
    /// Image : un nom de `terrain_texture.json` (pack de ressources ou jeu)
    pub texture: String,
    /// Dureté, à la manière du jeu (pierre : 1,5 ; obsidienne : 50)
    pub hardness: f32,
    /// Lumière émise, de 0 à 15
    pub light: u8,
    /// Glissance (0,6 en général)
    pub friction: f32,
    /// Onglet du menu créatif (1 construction, 2 nature, 3 équipement, 4 objets ; 0 : absent)
    pub category: u8,
    /// Groupe du menu créatif, vide si le bloc est seul
    pub group: String,
    /// Image du dessus et du dessous, si elles diffèrent de `texture` (vide : la même)
    pub texture_top: String,
    pub texture_bottom: String,
    /// Laisse voir à travers les parties vides de son image (verre, feuillage, plante)
    pub transparent: bool,
    /// Arrête les joueurs et les créatures ; faux pour une plante, qu'on traverse
    pub solid: bool,
    /// Forme : un modèle du jeu ou du pack de ressources (vide : cube plein)
    pub geometry: String,
    /// Outil qui convient au bloc, et matériau minimal pour qu'il rende son butin
    pub tool: ToolType,
    pub tier: Option<ToolTier>,
    /// Expérience rendue quand il est cassé (de … à …)
    pub xp: (u32, u32),
    /// États du bloc : orientation à la pose, ou stades de croissance
    pub variants: Variants,
    /// Image de la face avant, pour un bloc tourné vers le joueur (vide : comme les côtés)
    pub texture_front: String,
}

impl CustomBlock {
    /// Lit la fiche envoyée par un plugin : une ligne `clé<TAB>valeur` par réglage.
    pub fn parse(name: &str, text: &str) -> Result<CustomBlock, String> {
        let (space, short) = name.split_once(':').ok_or("le nom doit avoir un espace de noms, par exemple « monserveur:minerai_rubis »")?;
        let plain = |s: &str| !s.is_empty() && s.len() <= 48 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !plain(space) || !plain(short) {
            return Err("le nom n'accepte que des minuscules, des chiffres et « _ », de part et d'autre de « : »".into());
        }
        if space == "minecraft" {
            return Err("l'espace de noms « minecraft » est réservé au jeu : choisissez le vôtre".into());
        }
        let mut def = CustomBlock {
            name: name.to_string(),
            display: short.to_string(),
            texture: "stone".to_string(),
            hardness: 1.5,
            light: 0,
            friction: 0.6,
            category: 1,
            group: String::new(),
            texture_top: String::new(),
            texture_bottom: String::new(),
            transparent: false,
            solid: true,
            geometry: String::new(),
            tool: ToolType::None,
            tier: None,
            xp: (0, 0),
            variants: Variants::None,
            texture_front: String::new(),
        };
        let mut listed = true;
        let flag = |key: &str, value: &str| match value {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("{} attend true ou false, reçu « {} »", key, value)),
        };
        let image = |value: &str| if plain(value) { Ok(value.to_string()) } else { Err(format!("image invalide « {} » : donnez un nom de terrain_texture.json, par exemple « minerai_rubis »", value)) };
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (key, value) = line.split_once('\t').unwrap_or((line, ""));
            let number = || value.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| format!("{} attend un nombre positif, reçu « {} »", key, value));
            match key {
                "name" => def.display = value.chars().filter(|c| !c.is_control()).take(64).collect(),
                "texture" => def.texture = image(value)?,
                "textureTop" => def.texture_top = image(value)?,
                "textureFront" => def.texture_front = image(value)?,
                "rotation" => {
                    def.variants = match (value, &def.variants) {
                        (_, Variants::Stages(_)) => return Err("un bloc a soit une orientation (rotation), soit des stades (stages), pas les deux".into()),
                        ("axis", _) => Variants::Axis,
                        ("facing", _) => Variants::Facing,
                        _ => return Err(format!("orientation inconnue « {} » (axis : couché comme une bûche ; facing : tourné vers le joueur)", value)),
                    }
                }
                "stages" => {
                    if def.variants != Variants::None {
                        return Err("un bloc a soit une orientation (rotation), soit des stades (stages), pas les deux".into());
                    }
                    let images = value.split(',').map(image).collect::<Result<Vec<String>, String>>()?;
                    if images.len() < 2 || images.len() > 16 {
                        return Err(format!("stages attend de 2 à 16 images, reçu {}", images.len()));
                    }
                    def.variants = Variants::Stages(images);
                }
                "textureBottom" => def.texture_bottom = image(value)?,
                "transparent" => def.transparent = flag(key, value)?,
                "solid" => def.solid = flag(key, value)?,
                "geometry" => {
                    // « geometry.mon_modele » (pack) ou « minecraft:geometry.cross » (jeu)
                    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b':')) || !value.contains("geometry.") {
                        return Err(format!("forme invalide « {} » : donnez un modèle, par exemple « minecraft:geometry.cross » ou « geometry.ma_dalle »", value));
                    }
                    def.geometry = value.to_string();
                }
                "tool" => {
                    def.tool = match value {
                        "pickaxe" => ToolType::Pickaxe,
                        "axe" => ToolType::Axe,
                        "shovel" => ToolType::Shovel,
                        "hoe" => ToolType::Hoe,
                        "sword" => ToolType::Sword,
                        "shears" => ToolType::Shears,
                        "none" => ToolType::None,
                        _ => return Err(format!("outil inconnu « {} » (pickaxe, axe, shovel, hoe, sword, shears, none)", value)),
                    }
                }
                "tier" => {
                    def.tier = Some(match value {
                        "wood" => ToolTier::Wood,
                        "gold" => ToolTier::Gold,
                        "stone" => ToolTier::Stone,
                        "copper" => ToolTier::Copper,
                        "iron" => ToolTier::Iron,
                        "diamond" => ToolTier::Diamond,
                        "netherite" => ToolTier::Netherite,
                        _ => return Err(format!("matériau inconnu « {} » (wood, gold, stone, copper, iron, diamond, netherite)", value)),
                    })
                }
                "xp" => {
                    let n = number()?.min(1000.0) as u32;
                    def.xp = (n, n);
                }
                "xpMax" => def.xp.1 = number()?.min(1000.0) as u32,
                "hardness" => def.hardness = number()?.min(1000.0) as f32,
                "light" => def.light = number()?.min(15.0) as u8,
                "friction" => def.friction = number()?.clamp(0.1, 1.0) as f32,
                "creative" => {
                    listed = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(format!("creative attend true ou false, reçu « {} »", value)),
                    }
                }
                "category" => {
                    def.category = match value {
                        "construction" => 1,
                        "nature" => 2,
                        "equipment" => 3,
                        "items" => 4,
                        _ => return Err(format!("catégorie inconnue « {} » (construction, nature, equipment, items)", value)),
                    }
                }
                "group" => def.group = value.chars().filter(|c| !c.is_control()).take(48).collect(),
                _ => return Err(format!("réglage inconnu « {} »", key)),
            }
        }
        if !listed {
            def.category = 0;
        }
        if def.tier.is_some() && def.tool == ToolType::None {
            return Err("tier demande aussi « tool » : le matériau est celui de l'outil exigé".into());
        }
        def.xp.1 = def.xp.1.max(def.xp.0);
        Ok(def)
    }

    /// Propriétés du bloc côté serveur.
    pub fn props(&self) -> crate::block::BlockProps {
        crate::block::BlockProps { hardness: self.hardness, blast_resistance: self.hardness, light: self.light, flammability: 0, friction: self.friction, opacity: if self.transparent { 0.0 } else { 1.0 } }
    }

    /// Identifiant sur le réseau de l'état par défaut du bloc (celui que pose son objet).
    pub fn network_id(&self) -> u32 {
        self.state(0)
    }

    /// Identifiant sur le réseau d'un état du bloc : le hachage de son nom et de sa
    /// propriété, comme pour les blocs du jeu.
    pub fn state(&self, variant: usize) -> u32 {
        match self.variants.property() {
            Some(property) => crate::bedrock::block_hash_int(&self.name, property, variant.min(self.variants.count() - 1) as i32),
            None => crate::bedrock::block_hash(&self.name),
        }
    }

    /// Tous les états du bloc, le premier étant celui par défaut.
    pub fn states(&self) -> Vec<u32> {
        (0..self.variants.count()).map(|v| self.state(v)).collect()
    }

    /// L'état à poser, d'après la face cliquée (0 dessous, 1 dessus, 2 nord, 3 sud, 4 ouest,
    /// 5 est) et la direction dans laquelle regarde le joueur.
    pub fn placed_state(&self, face: u8, looks: u8) -> u32 {
        let variant = match self.variants {
            // Debout sur le sol ou au plafond ; couché le long de la face d'un mur
            Variants::Axis => match face {
                4 | 5 => 1,
                2 | 3 => 2,
                _ => 0,
            },
            // La face avant regarde le joueur : à l'opposé de son regard
            Variants::Facing => match looks {
                3 => 0, // il regarde au sud : l'avant est au nord
                4 => 1, // à l'ouest : l'avant est à l'est
                2 => 2,
                _ => 3,
            },
            _ => 0,
        };
        self.state(variant)
    }

    /// La déclaration du bloc au jeu du joueur : sa forme (cube plein par défaut), ses images,
    /// sa dureté, sa lumière. `block_id` : son numéro de bloc (à partir de `FIRST_BLOCK_ID`).
    pub fn definition(&self, block_id: i32) -> Tag {
        let text = |s: &str| Tag::Str(s.to_string());
        let category = match self.category {
            2 => "nature",
            3 => "equipment",
            4 => "items",
            _ => "construction",
        };
        let material = |texture: &str| {
            Tag::compound(vec![
                ("ambient_occlusion", Tag::Byte(!self.transparent as i8)),
                ("face_dimming", Tag::Byte(1)),
                ("render_method", text(if self.transparent { "alpha_test" } else { "opaque" })),
                ("texture", text(texture)),
            ])
        };
        // « * » : toutes les faces ; « up », « down » et « north » (l'avant) quand ils diffèrent
        let mut materials = vec![("*", material(&self.texture))];
        if !self.texture_front.is_empty() {
            materials.push(("north", material(&self.texture_front)));
        }
        if !self.texture_top.is_empty() {
            materials.push(("up", material(&self.texture_top)));
        }
        if !self.texture_bottom.is_empty() {
            materials.push(("down", material(&self.texture_bottom)));
        }
        let cube = |enabled: bool| {
            Tag::compound(vec![
                ("enabled", Tag::Byte(enabled as i8)),
                ("origin", Tag::List(5, vec![Tag::Float(-8.0), Tag::Float(0.0), Tag::Float(-8.0)])),
                ("size", Tag::List(5, vec![Tag::Float(16.0), Tag::Float(16.0), Tag::Float(16.0)])),
            ])
        };
        let geometry = if self.geometry.is_empty() { "minecraft:geometry.full_block" } else { &self.geometry };
        let components = Tag::compound(vec![
            ("minecraft:collision_box", cube(self.solid)),
            // La dureté, à l'échelle des blocs du jeu (à la main : une fois et demie ce nombre,
            // en secondes). Écrite sous les deux noms que le jeu a donnés à ce réglage.
            ("minecraft:destructible_by_mining", Tag::compound(vec![("seconds_to_destroy", Tag::Float(self.hardness)), ("value", Tag::Float(self.hardness))])),
            ("minecraft:display_name", Tag::compound(vec![("value", text(&self.display))])),
            ("minecraft:friction", Tag::compound(vec![("value", Tag::Float(1.0 - self.friction))])),
            ("minecraft:geometry", Tag::compound(vec![("identifier", text(geometry))])),
            ("minecraft:light_dampening", Tag::compound(vec![("lightLevel", Tag::Byte(if self.transparent { 0 } else { 15 }))])),
            ("minecraft:light_emission", Tag::compound(vec![("emission", Tag::Byte(self.light as i8))])),
            ("minecraft:material_instances", Tag::compound(vec![("mappings", Tag::compound(vec![])), ("materials", Tag::compound(materials))])),
            ("minecraft:selection_box", cube(true)),
        ]);
        // Étiquettes : elles disent au jeu quel outil va vite sur ce bloc, comme pour les siens
        let mut tags: Vec<Tag> = Vec::new();
        if let Some(kind) = match self.tool {
            ToolType::Pickaxe => Some("pickaxe"),
            ToolType::Axe => Some("axe"),
            ToolType::Shovel => Some("shovel"),
            ToolType::Hoe => Some("hoe"),
            ToolType::Sword => Some("sword"),
            ToolType::Shears => Some("shears"),
            ToolType::None => None,
        } {
            tags.push(Tag::Str(format!("minecraft:is_{}_item_destructible", kind)));
        }
        if let Some(tier) = match self.tier {
            Some(ToolTier::Stone) | Some(ToolTier::Copper) => Some("stone"),
            Some(ToolTier::Iron) => Some("iron"),
            Some(ToolTier::Diamond) | Some(ToolTier::Netherite) => Some("diamond"),
            _ => None,
        } {
            tags.push(Tag::Str(format!("minecraft:{}_tier_destructible", tier)));
        }
        // États : une propriété entière, et pour chaque valeur ce qui change (une rotation
        // du modèle, ou l'image du stade)
        let turn = |rx: i32, ry: i32, rz: i32| {
            let part = |k: &'static str, v: Tag| (k, v);
            (
                "minecraft:transformation",
                Tag::compound(vec![
                    part("RX", Tag::Int(rx)),
                    part("RY", Tag::Int(ry)),
                    part("RZ", Tag::Int(rz)),
                    part("SX", Tag::Float(1.0)),
                    part("SY", Tag::Float(1.0)),
                    part("SZ", Tag::Float(1.0)),
                    part("TX", Tag::Float(0.0)),
                    part("TY", Tag::Float(0.0)),
                    part("TZ", Tag::Float(0.0)),
                ]),
            )
        };
        let (mut properties, mut permutations) = (Vec::new(), Vec::new());
        if let Some(property) = self.variants.property() {
            let count = self.variants.count();
            properties.push(Tag::compound(vec![("enum", Tag::List(3, (0..count as i32).map(Tag::Int).collect())), ("name", text(property))]));
            for variant in 0..count {
                let change = match &self.variants {
                    // (en quarts de tour) couché est-ouest : autour de l'axe nord-sud ; couché
                    // nord-sud : autour de l'axe est-ouest
                    Variants::Axis => turn(if variant == 2 { 1 } else { 0 }, 0, if variant == 1 { 1 } else { 0 }),
                    // nord, est, sud, ouest : en tournant vers la droite vu de dessus
                    Variants::Facing => turn(0, [0, 3, 2, 1][variant], 0),
                    Variants::Stages(images) => {
                        ("minecraft:material_instances", Tag::compound(vec![("mappings", Tag::compound(vec![])), ("materials", Tag::compound(vec![("*", material(&images[variant]))]))]))
                    }
                    Variants::None => continue,
                };
                permutations.push(Tag::compound(vec![("components", Tag::compound(vec![change])), ("condition", Tag::Str(format!("q.block_state('{}') == {}", property, variant)))]));
            }
        }
        // (les conditions ci-dessus demandent une version récente du langage du jeu ; un bloc
        // sans états garde celle que le jeu a toujours acceptée ici)
        let molang = if permutations.is_empty() { 1 } else { 9 };
        Tag::compound(vec![
            ("blockTags", Tag::List(8, tags)),
            ("components", components),
            ("menu_category", Tag::compound(vec![("category", text(category)), ("group", text(&self.group)), ("is_hidden_in_commands", Tag::Byte(0))])),
            ("molangVersion", Tag::Int(molang)),
            ("permutations", Tag::List(10, permutations)),
            ("properties", Tag::List(10, properties)),
            ("vanilla_block_data", Tag::compound(vec![("block_id", Tag::Int(block_id))])),
        ])
    }
}

/// Gabarit d'un bloc créé par un plugin : il répond comme un bloc du jeu, d'après sa fiche.
/// (Aucune règle n'est devinée d'après son nom, à la différence des blocs du jeu.)
pub struct CustomBlockTemplate {
    base: BaseBlock,
    solid: bool,
    transparent: bool,
}

impl Block for CustomBlockTemplate {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn is_solid(&self) -> bool {
        self.solid
    }
    fn is_transparent(&self) -> bool {
        self.transparent || !self.solid
    }
}

/// Le gabarit du bloc créé sous ce nom complet, s'il existe.
pub fn get(name: &str) -> Option<Box<dyn Block>> {
    let def = find(name)?;
    let base = BaseBlock::with_props(&def.name, def.props(), def.tool, def.tier);
    Some(Box::new(CustomBlockTemplate { base, solid: def.solid, transparent: def.transparent }))
}

/// Le bloc créé auquel appartient cet état, et le rang de l'état (orientation ou stade).
pub fn variant_of(state: u32) -> Option<(CustomBlock, usize)> {
    if !any() {
        return None;
    }
    let (name, variant) = STATES.read().ok()?.as_ref()?.get(&state).cloned()?;
    find(&name).map(|def| (def, variant))
}

/// La fiche du bloc créé sous ce nom complet, s'il existe.
pub fn find(name: &str) -> Option<CustomBlock> {
    if !any() || !name.contains(':') || name.starts_with("minecraft:") {
        return None;
    }
    TABLE.read().ok()?.as_ref()?.get(name).map(|(def, _)| def.clone())
}

/// Identifiant de l'objet qui pose le bloc portant ce numéro (négatif, comme dans le jeu).
pub fn item_id_of(block_id: i32) -> i16 {
    (255 - block_id) as i16
}

static ANY: AtomicBool = AtomicBool::new(false);
static TABLE: RwLock<Option<HashMap<String, (CustomBlock, i32)>>> = RwLock::new(None);
/// À quel bloc et à quel rang appartient chaque état des blocs à états.
static STATES: RwLock<Option<HashMap<u32, (String, usize)>>> = RwLock::new(None);
/// Numéro donné à chaque nom : gardé d'un rechargement des plugins à l'autre.
static IDS: RwLock<Option<HashMap<String, i32>>> = RwLock::new(None);

/// Enregistre un bloc (ou remplace celui du même nom) et renvoie son numéro de bloc.
pub fn define(def: CustomBlock) -> Result<i32, String> {
    let mut ids = IDS.write().map_err(|_| "table des blocs indisponible".to_string())?;
    let ids = ids.get_or_insert_with(HashMap::new);
    let id = match ids.get(&def.name) {
        Some(id) => *id,
        None => {
            if ids.len() >= MAX_BLOCKS {
                return Err(format!("trop de blocs créés ({} au plus)", MAX_BLOCKS));
            }
            let id = FIRST_BLOCK_ID + ids.len() as i32;
            ids.insert(def.name.clone(), id);
            id
        }
    };
    if let Ok(mut states) = STATES.write() {
        let states = states.get_or_insert_with(HashMap::new);
        states.retain(|_, (name, _)| *name != def.name);
        for (variant, state) in def.states().into_iter().enumerate() {
            states.insert(state, (def.name.clone(), variant));
        }
    }
    let mut table = TABLE.write().map_err(|_| "table des blocs indisponible".to_string())?;
    table.get_or_insert_with(HashMap::new).insert(def.name.clone(), (def, id));
    ANY.store(true, Ordering::Relaxed);
    Ok(id)
}

/// Oublie les blocs créés (rechargement des plugins) ; leurs numéros restent réservés.
pub fn clear() {
    if let Ok(mut table) = TABLE.write() {
        *table = None;
    }
    if let Ok(mut states) = STATES.write() {
        *states = None;
    }
    ANY.store(false, Ordering::Relaxed);
}

/// Y a-t-il des blocs créés par les plugins ?
pub fn any() -> bool {
    ANY.load(Ordering::Relaxed)
}

/// Tous les blocs créés, avec leur numéro, dans l'ordre de celui-ci.
pub fn all() -> Vec<(CustomBlock, i32)> {
    let mut list: Vec<(CustomBlock, i32)> = TABLE.read().ok().and_then(|t| t.as_ref().map(|m| m.values().cloned().collect())).unwrap_or_default();
    list.sort_by_key(|(_, id)| *id);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_blocks_have_states() {
        // Une bûche : trois états, celui par défaut debout ; chacun a son identifiant
        let log = CustomBlock::parse("essai:tronc", "rotation\taxis\ntexture\tcote\ntextureTop\tdessus").unwrap();
        let states = log.states();
        assert_eq!(states.len(), 3);
        assert!(states[0] != states[1] && states[1] != states[2] && states[0] != crate::bedrock::block_hash("essai:tronc"));
        assert_eq!(log.network_id(), states[0]);
        // Posée sur le sol : debout ; contre un mur à l'est : couchée est-ouest ; au nord : nord-sud
        assert_eq!((log.placed_state(1, 3), log.placed_state(5, 3), log.placed_state(2, 3)), (states[0], states[1], states[2]));
        let tag = log.definition(10005);
        let property = tag.get("properties").and_then(|p| p.as_list()).and_then(|l| l.first()).unwrap();
        assert_eq!(property.str_of("name"), Some("basalt:axis"));
        let permutations = tag.get("permutations").and_then(|p| p.as_list()).unwrap();
        assert_eq!(permutations.len(), 3);
        assert_eq!(permutations[1].str_of("condition"), Some("q.block_state('basalt:axis') == 1"));
        assert_eq!(permutations[1].get("components").and_then(|c| c.get("minecraft:transformation")).and_then(|t| t.get("RZ")).and_then(|t| t.as_int()), Some(1));
        // Un four : l'avant regarde le joueur
        let oven = CustomBlock::parse("essai:fourneau", "rotation\tfacing\ntextureFront\tavant").unwrap();
        assert_eq!(oven.placed_state(1, 3), oven.state(0), "le joueur regarde au sud : l'avant est au nord");
        assert_eq!(oven.placed_state(1, 2), oven.state(2));
        assert_eq!(oven.definition(10006).get("components").and_then(|c| c.get("minecraft:material_instances")).and_then(|m| m.get("materials")).and_then(|m| m.get("north")).and_then(|m| m.str_of("texture")), Some("avant"));
        // Une culture : un état par stade, chacun avec son image ; l'état retrouve son rang
        let crop = CustomBlock::parse("essai:ble_bleu", "stages\tpousse,tige,epi").unwrap();
        assert_eq!(crop.variants.count(), 3);
        let tag = crop.definition(10007);
        let last = &tag.get("permutations").and_then(|p| p.as_list()).unwrap()[2];
        let image = last.get("components").and_then(|c| c.get("minecraft:material_instances")).and_then(|m| m.get("materials")).and_then(|m| m.get("*")).and_then(|m| m.str_of("texture"));
        assert_eq!(image, Some("epi"));
        define(crop.clone()).unwrap();
        assert_eq!(variant_of(crop.state(2)).map(|(b, v)| (b.name, v)), Some(("essai:ble_bleu".to_string(), 2)));
        assert!(variant_of(12345).is_none());
        // Un bloc sans états garde un seul identifiant, celui de son nom
        let plain = CustomBlock::parse("essai:simple", "").unwrap();
        assert_eq!(plain.states(), vec![crate::bedrock::block_hash("essai:simple")]);
    }

    #[test]
    fn plugins_describe_new_blocks() {
        let ore = CustomBlock::parse("essai:minerai", "name\tMinerai\ntexture\tminerai_rubis\nhardness\t3\nlight\t7\ncategory\tnature\ngroup\tMinerais").unwrap();
        assert_eq!((ore.display.as_str(), ore.texture.as_str(), ore.hardness, ore.light, ore.category), ("Minerai", "minerai_rubis", 3.0, 7, 2));
        assert_eq!(ore.props().hardness, 3.0);
        assert_eq!(ore.network_id(), crate::bedrock::block_hash("essai:minerai"));
        // Numéro de bloc, puis objet qui le pose : négatif, comme pour les blocs du jeu
        let id = define(ore.clone()).unwrap();
        assert!(id >= FIRST_BLOCK_ID && item_id_of(id) < 0);
        assert_eq!(define(ore.clone()).unwrap(), id, "redéfini, un bloc garde son numéro");
        assert!(all().iter().any(|(b, n)| b.name == "essai:minerai" && *n == id));
        // La déclaration au jeu
        let tag = ore.definition(id);
        let components = tag.get("components").unwrap();
        let material = components.get("minecraft:material_instances").and_then(|m| m.get("materials")).and_then(|m| m.get("*")).unwrap();
        assert_eq!(material.str_of("texture"), Some("minerai_rubis"));
        assert_eq!(components.get("minecraft:light_emission").and_then(|l| l.get("emission")), Some(&Tag::Byte(7)));
        assert_eq!(tag.get("vanilla_block_data").and_then(|v| v.get("block_id")).and_then(|t| t.as_int()), Some(id));
        assert_eq!(tag.get("menu_category").and_then(|m| m.str_of("category")), Some("nature"));
        // Noms et réglages refusés
        for (name, text) in [("minerai", ""), ("minecraft:minerai", ""), ("Essai:X", ""), ("essai:x", "couleur\trouge"), ("essai:x", "texture\tMon Image"), ("essai:x", "hardness\tdur")] {
            assert!(CustomBlock::parse(name, text).is_err(), "{} {}", name, text);
        }
        assert_eq!(CustomBlock::parse("essai:x", "creative\tfalse").unwrap().category, 0);
    }

    #[test]
    fn plugin_blocks_have_faces_shapes_and_tools() {
        let text = "texture\tbuche_cote\ntextureTop\tbuche_dessus\ntextureBottom\tbuche_dessus\ntool\taxe\nxp\t2\nxpMax\t5";
        let log = CustomBlock::parse("essai:buche", text).unwrap();
        let tag = log.definition(10001);
        let materials = tag.get("components").and_then(|c| c.get("minecraft:material_instances")).and_then(|m| m.get("materials")).unwrap();
        let face = |key: &str| materials.get(key).and_then(|m| m.str_of("texture"));
        assert_eq!((face("*"), face("up"), face("down")), (Some("buche_cote"), Some("buche_dessus"), Some("buche_dessus")));
        assert_eq!((log.tool, log.tier, log.xp), (ToolType::Axe, None, (2, 5)));
        assert_eq!(tag.get("blockTags"), Some(&Tag::List(8, vec![Tag::Str("minecraft:is_axe_item_destructible".into())])));
        // La dureté est déclarée telle quelle : 1,5 par défaut, comme la pierre
        assert_eq!(tag.get("components").and_then(|c| c.get("minecraft:destructible_by_mining")).and_then(|d| d.get("seconds_to_destroy")), Some(&Tag::Float(1.5)));
        // Une plante : on la traverse, elle laisse passer la lumière, sa forme est une croix
        let plant = CustomBlock::parse("essai:fleur", "texture\tfleur\nsolid\tfalse\ntransparent\ttrue\ngeometry\tminecraft:geometry.cross").unwrap();
        let tag = plant.definition(10002);
        let components = tag.get("components").unwrap();
        assert_eq!(components.get("minecraft:collision_box").and_then(|c| c.get("enabled")), Some(&Tag::Byte(0)));
        assert_eq!(components.get("minecraft:geometry").and_then(|g| g.str_of("identifier")), Some("minecraft:geometry.cross"));
        assert_eq!(components.get("minecraft:material_instances").and_then(|m| m.get("materials")).and_then(|m| m.get("*")).and_then(|m| m.str_of("render_method")), Some("alpha_test"));
        // Côté serveur : un minerai qui demande une pioche en fer
        define(CustomBlock::parse("essai:titane", "hardness\t4\ntool\tpickaxe\ntier\tiron").unwrap()).unwrap();
        define(plant).unwrap();
        let ore = crate::block::VanillaBlocks::get("essai:titane", None);
        let (stone, iron) = (crate::item::VanillaItems::get("stone_pickaxe"), crate::item::VanillaItems::get("iron_pickaxe"));
        assert!(ore.drops(stone.as_ref()).is_empty() && ore.drops(iron.as_ref()).len() == 1);
        assert!(ore.break_time(iron.as_ref()) < ore.break_time(crate::item::VanillaItems::hand().as_ref()));
        assert!(ore.is_solid() && !crate::block::VanillaBlocks::get("essai:fleur", None).is_solid());
        // Réglages refusés
        for text in ["tool\tmarteau", "tier\tiron", "geometry\tcube", "solid\tpeut-être", "rotation\tbiais", "stages\tseul", "rotation\taxis\nstages\ta,b"] {
            assert!(CustomBlock::parse("essai:x", text).is_err(), "{}", text);
        }
    }
}
