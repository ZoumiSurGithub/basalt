//! Objets créés par les plugins (l'équivalent de Customies pour PocketMine).
//!
//! Un plugin déclare un objet au nom nouveau
//! (`server.items.create("monserveur:rubis", { name = "Rubis", icon = "emerald" })`, voir
//! `docs/PLUGINS.md`). Deux choses en découlent :
//!
//! - côté serveur, `CustomItem` est un gabarit comme les autres : il implémente `Item`, et
//!   `VanillaItems::get` le rend quand on demande son nom. Faim, usure, dégâts : le serveur
//!   l'interroge sans savoir qu'il vient d'un plugin ;
//! - côté joueur, l'objet est **déclaré au jeu** à la connexion, avec ses composants
//!   (`components`) : taille de pile, durée de vie, valeur nutritive, icône. Le jeu les
//!   applique lui-même, donc l'écran et le serveur disent la même chose — ce qu'on ne peut
//!   pas obtenir en modifiant un objet du jeu.
//!
//! L'icône est celle d'un objet du jeu (`emerald`, `diamond_sword`…) : aucune image n'est
//! envoyée aux joueurs. Des images propres demanderont un pack de ressources.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use super::consumable::EAT_TICKS;
use super::durable::Usage;
use super::item::{BaseItem, FoodValues, Item};
use crate::nbt::Tag;

/// Premier identifiant donné aux objets des plugins (ceux du jeu sont bien en dessous).
pub const FIRST_ID: i16 = 20000;
/// Nombre d'objets que les plugins peuvent créer.
pub const MAX_ITEMS: usize = 2000;

/// Fiche d'un objet créé par un plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomDef {
    /// Nom complet, avec son espace de noms (`monserveur:rubis`)
    pub name: String,
    /// Nom affiché au joueur
    pub display: String,
    /// Icône : le nom de texture d'un objet du jeu (`emerald`)
    pub icon: String,
    pub max_stack: u8,
    /// Se mange : ce qu'il rend
    pub food: Option<FoodValues>,
    /// Utilisations avant de casser (0 : ne s'use pas)
    pub durability: u32,
    /// Dégâts d'un coup
    pub attack_damage: f32,
    /// Tenu comme un outil
    pub hand_equipped: bool,
    /// Brille comme un objet enchanté
    pub glint: bool,
    /// Onglet du menu créatif : 1 construction, 2 nature, 3 équipement, 4 objets ; 0 : absent
    /// du menu
    pub category: u8,
    /// Groupe du menu créatif (les objets d'un même groupe se replient sous une seule case) ;
    /// vide : l'objet est seul
    pub group: String,
}

impl CustomDef {
    /// Lit la fiche envoyée par un plugin : une ligne `clé<TAB>valeur` par réglage.
    pub fn parse(name: &str, text: &str) -> Result<CustomDef, String> {
        let (space, short) = name.split_once(':').ok_or("le nom doit avoir un espace de noms, par exemple « monserveur:rubis »")?;
        let plain = |s: &str| !s.is_empty() && s.len() <= 48 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !plain(space) || !plain(short) {
            return Err("le nom n'accepte que des minuscules, des chiffres et « _ », de part et d'autre de « : »".into());
        }
        if space == "minecraft" {
            return Err("l'espace de noms « minecraft » est réservé au jeu : choisissez le vôtre".into());
        }
        let mut def = CustomDef {
            name: name.to_string(),
            display: short.to_string(),
            icon: "stick".to_string(),
            max_stack: 64,
            food: None,
            durability: 0,
            attack_damage: 1.0,
            hand_equipped: false,
            glint: false,
            category: 0,
            group: String::new(),
        };
        let (mut category, mut listed) = (None, true);
        let (mut food, mut saturation, mut always, mut leftover, mut stack) = (None, None, false, None, None);
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (key, value) = line.split_once('\t').unwrap_or((line, ""));
            let number = || value.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| format!("{} attend un nombre positif, reçu « {} »", key, value));
            let flag = || match value {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(format!("{} attend true ou false, reçu « {} »", key, value)),
            };
            match key {
                "name" => def.display = value.chars().filter(|c| !c.is_control()).take(64).collect(),
                "icon" => {
                    if !plain(value) {
                        return Err(format!("icône invalide « {} » : donnez le nom d'un objet du jeu, par exemple « emerald »", value));
                    }
                    def.icon = value.to_string();
                }
                "maxStack" => stack = Some(number()?.clamp(1.0, 64.0) as u8),
                "food" => food = Some(number()?.min(20.0) as u8),
                "saturation" => saturation = Some(number()?.min(20.0) as f32),
                "alwaysEdible" => always = flag()?,
                "leftover" => leftover = Some(value.strip_prefix("minecraft:").unwrap_or(value).to_string()),
                "durability" => def.durability = number()?.min(u16::MAX as f64) as u32,
                "attackDamage" => def.attack_damage = number()?.min(1000.0) as f32,
                "handEquipped" => def.hand_equipped = flag()?,
                "glint" => def.glint = flag()?,
                "creative" => listed = flag()?,
                "category" => {
                    category = Some(match value {
                        "construction" => 1,
                        "nature" => 2,
                        "equipment" => 3,
                        "items" => 4,
                        _ => return Err(format!("catégorie inconnue « {} » (construction, nature, equipment, items)", value)),
                    })
                }
                "group" => def.group = value.chars().filter(|c| !c.is_control()).take(48).collect(),
                _ => return Err(format!("réglage inconnu « {} »", key)),
            }
        }
        if let Some(hunger) = food {
            def.food = Some(FoodValues {
                hunger,
                saturation: saturation.unwrap_or(hunger as f32 * 0.6),
                always_edible: always,
                eat_ticks: EAT_TICKS,
                // Le nom vit autant que le serveur : une poignée d'octets par objet créé
                leftover: leftover.filter(|l| !l.is_empty()).map(|l| &*Box::leak(l.into_boxed_str())),
            });
        } else if saturation.is_some() || always || leftover.is_some() {
            return Err("saturation, alwaysEdible et leftover demandent aussi « food »".into());
        }
        // Un objet qui s'use ne s'empile pas, comme dans le jeu
        def.max_stack = if def.durability > 0 { 1 } else { stack.unwrap_or(64) };
        if def.durability > 0 {
            def.hand_equipped = true;
        }
        // Par défaut : avec l'équipement pour ce qui s'use ou frappe, avec les objets sinon
        def.category = match (listed, category) {
            (false, _) => 0,
            (true, Some(c)) => c,
            (true, None) if def.durability > 0 || def.attack_damage > 1.0 => 3,
            (true, None) => 4,
        };
        Ok(def)
    }

    /// Les composants déclarés au jeu du joueur, calqués sur ceux que le serveur officiel
    /// envoie pour ses propres objets (la pomme pour un aliment, la lance pour un outil).
    pub fn components(&self) -> Tag {
        let byte = |b: bool| Tag::Byte(b as i8);
        let text = |s: &str| Tag::Str(s.to_string());
        let mut properties = vec![
            ("allow_off_hand", byte(false)),
            ("can_destroy_in_creative", byte(true)),
            ("creative_category", Tag::Int(if self.category == 0 { 4 } else { self.category as i32 })),
            ("creative_group", text(&self.group)),
            // (0 : le jeu n'affiche « points d'attaque » que pour une arme)
            ("damage", Tag::Int(if self.attack_damage > 1.0 { self.attack_damage.round() as i32 } else { 0 })),
            ("enchantable_slot", text("none")),
            ("enchantable_value", Tag::Int(0)),
            ("foil", byte(self.glint)),
            ("frame_count", Tag::Int(1)),
            ("hand_equipped", byte(self.hand_equipped)),
            ("hidden_in_commands", Tag::Byte(0)),
            ("liquid_clipped", byte(false)),
            ("max_stack_size", Tag::Int(self.max_stack as i32)),
            ("minecraft:icon", Tag::compound(vec![("textures", Tag::compound(vec![("default", text(&self.icon))]))])),
            ("mining_speed", Tag::Float(1.0)),
            ("should_despawn", byte(true)),
            ("stacked_by_data", byte(false)),
        ];
        let mut tags: Vec<Tag> = Vec::new();
        let mut components: Vec<(&str, Tag)> = Vec::new();
        match self.food {
            Some(f) => {
                properties.push(("use_animation", Tag::Int(1)));
                properties.push(("use_duration", Tag::Int(f.eat_ticks as i32)));
                tags.push(text("minecraft:is_food"));
                // (le jeu calcule la satiété : faim × coefficient × 2)
                let modifier = if f.hunger > 0 { f.saturation / (2.0 * f.hunger as f32) } else { 0.0 };
                components.push((
                    "minecraft:food",
                    Tag::compound(vec![
                        ("can_always_eat", byte(f.always_edible)),
                        ("nutrition", Tag::Int(f.hunger as i32)),
                        ("saturation_modifier", Tag::Float(modifier)),
                        ("using_converts_to", Tag::compound(vec![])),
                    ]),
                ));
                components.push(("minecraft:use_animation", Tag::compound(vec![("value", text("eat"))])));
                components.push((
                    "minecraft:use_modifiers",
                    Tag::compound(vec![
                        ("emit_vibrations", byte(true)),
                        ("movement_modifier", Tag::Float(0.35)),
                        ("start_using", text("always")),
                        ("use_duration", Tag::Float(f.eat_ticks as f32 / 20.0)),
                    ]),
                ));
            }
            None => {
                properties.push(("use_animation", Tag::Int(0)));
                properties.push(("use_duration", Tag::Int(0)));
            }
        }
        if self.durability > 0 {
            components.push((
                "minecraft:durability",
                Tag::compound(vec![("damage_chance", Tag::compound(vec![("max", Tag::Int(100)), ("min", Tag::Int(0))])), ("max_durability", Tag::Int(self.durability as i32))]),
            ));
        }
        if self.attack_damage > 1.0 {
            components.push(("minecraft:damage", Tag::compound(vec![("value", Tag::Short(self.attack_damage.round() as i16))])));
        }
        if self.hand_equipped {
            components.push(("minecraft:hand_equipped", Tag::compound(vec![("value", byte(true))])));
        }
        components.push(("minecraft:display_name", Tag::compound(vec![("value", text(&self.display))])));
        components.push(("minecraft:max_stack_size", Tag::compound(vec![("value", Tag::Byte(self.max_stack as i8))])));
        if !tags.is_empty() {
            components.push(("minecraft:tags", Tag::compound(vec![("tags", Tag::List(8, tags.clone()))])));
        }
        let mut all = vec![("item_properties", Tag::compound(properties)), ("item_tags", Tag::List(8, tags))];
        all.extend(components);
        Tag::compound(vec![("components", Tag::compound(all))])
    }
}

/// Les images des outils et armures du jeu ne portent pas le nom de leur objet (le jeu les
/// range autrement, et dit `gold` ou `wood` là où l'objet dit `golden` ou `wooden`) : un tel
/// nom donne un objet sans image. Vrai si `icon` en est un, pour prévenir le plugin.
pub fn icon_advice(icon: &str) -> bool {
    let Some((material, kind)) = icon.split_once('_') else { return false };
    matches!(material, "wooden" | "wood" | "stone" | "iron" | "golden" | "gold" | "diamond" | "netherite" | "copper" | "leather" | "chainmail")
        && matches!(kind, "sword" | "pickaxe" | "axe" | "shovel" | "hoe" | "helmet" | "chestplate" | "leggings" | "boots")
}

/// Gabarit d'un objet créé par un plugin.
pub struct CustomItem {
    base: BaseItem,
    def: CustomDef,
}

impl Item for CustomItem {
    fn base(&self) -> &BaseItem {
        &self.base
    }
    fn max_durability(&self) -> u32 {
        self.def.durability
    }
    /// Un cran par usage, qu'on mine ou qu'on frappe.
    fn wear(&self, _usage: Usage) -> u16 {
        (self.def.durability > 0) as u16
    }
    fn attack_damage(&self) -> f32 {
        self.def.attack_damage
    }
    fn food(&self) -> Option<FoodValues> {
        self.def.food
    }
}

static ANY: AtomicBool = AtomicBool::new(false);
static TABLE: RwLock<Option<HashMap<String, (CustomDef, i16)>>> = RwLock::new(None);
/// Identifiant donné à chaque nom : gardé d'un rechargement des plugins à l'autre, pour que
/// les joueurs déjà en jeu continuent de désigner le même objet.
static IDS: RwLock<Option<HashMap<String, i16>>> = RwLock::new(None);

/// Enregistre un objet (ou remplace celui du même nom) et renvoie son identifiant.
pub fn define(def: CustomDef) -> Result<i16, String> {
    let mut ids = IDS.write().map_err(|_| "table des objets indisponible".to_string())?;
    let ids = ids.get_or_insert_with(HashMap::new);
    let id = match ids.get(&def.name) {
        Some(id) => *id,
        None => {
            if ids.len() >= MAX_ITEMS {
                return Err(format!("trop d'objets créés ({} au plus)", MAX_ITEMS));
            }
            let id = FIRST_ID + ids.len() as i16;
            ids.insert(def.name.clone(), id);
            id
        }
    };
    let mut table = TABLE.write().map_err(|_| "table des objets indisponible".to_string())?;
    table.get_or_insert_with(HashMap::new).insert(def.name.clone(), (def, id));
    ANY.store(true, Ordering::Relaxed);
    Ok(id)
}

/// Oublie les objets créés (rechargement des plugins) ; leurs identifiants restent réservés.
pub fn clear() {
    if let Ok(mut table) = TABLE.write() {
        *table = None;
    }
    ANY.store(false, Ordering::Relaxed);
}

/// Tous les objets créés, avec leur identifiant, dans l'ordre de celui-ci.
pub fn all() -> Vec<(CustomDef, i16)> {
    let mut list: Vec<(CustomDef, i16)> = TABLE.read().ok().and_then(|t| t.as_ref().map(|m| m.values().cloned().collect())).unwrap_or_default();
    list.sort_by_key(|(_, id)| *id);
    list
}

/// Y a-t-il des objets créés par les plugins ?
pub fn any() -> bool {
    ANY.load(Ordering::Relaxed)
}

/// L'objet créé sous ce nom complet, s'il existe.
pub fn get(name: &str) -> Option<Box<dyn Item>> {
    if !any() || !name.contains(':') {
        return None;
    }
    let def = TABLE.read().ok()?.as_ref()?.get(name).map(|(def, _)| def.clone())?;
    Some(Box::new(CustomItem { base: BaseItem::new(&def.name, def.max_stack), def }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{apply_wear, ItemStack, VanillaItems};

    #[test]
    fn plugins_create_items_the_server_treats_like_any_other() {
        let ruby = CustomDef::parse("essai:rubis", "name\tRubis\nicon\temerald\nmaxStack\t16\nglint\ttrue").unwrap();
        assert_eq!((ruby.display.as_str(), ruby.icon.as_str(), ruby.max_stack, ruby.glint), ("Rubis", "emerald", 16, true));
        let id = define(ruby).unwrap();
        assert!(id >= FIRST_ID);
        let item = VanillaItems::get("essai:rubis");
        assert_eq!((item.name(), item.max_stack_size(), item.max_durability()), ("essai:rubis", 16, 0));
        // Un aliment et une arme : mêmes questions que pour les objets du jeu
        define(CustomDef::parse("essai:galette", "food\t8\nsaturation\t9.6\nalwaysEdible\ttrue\nleftover\tminecraft:bowl").unwrap()).unwrap();
        let food = VanillaItems::get("essai:galette").food().unwrap();
        assert_eq!((food.hunger, food.saturation, food.always_edible, food.leftover), (8, 9.6, true, Some("bowl")));
        define(CustomDef::parse("essai:lame", "durability\t2\nattackDamage\t11\nmaxStack\t64").unwrap()).unwrap();
        let blade = VanillaItems::get("essai:lame");
        assert_eq!((blade.max_stack_size(), blade.attack_damage()), (1, 11.0));
        let mut stack = ItemStack::new(1, 1, 0);
        assert!(!apply_wear(&mut stack, blade.as_ref(), Usage::Strike));
        assert!(apply_wear(&mut stack, blade.as_ref(), Usage::Strike));
        // Redéfini, un objet garde son identifiant
        assert_eq!(define(CustomDef::parse("essai:rubis", "").unwrap()).unwrap(), id);
        // Noms et réglages refusés
        for (name, text) in [("rubis", ""), ("minecraft:rubis", ""), ("Essai:Rubis", ""), ("essai:x", "couleur\trouge"), ("essai:x", "saturation\t3"), ("essai:x", "icon\tMa Texture")] {
            assert!(CustomDef::parse(name, text).is_err(), "{} {}", name, text);
        }
    }

    #[test]
    fn components_follow_what_the_official_server_sends() {
        let def = CustomDef::parse("essai:tarte", "name\tTarte\nicon\tpumpkin_pie\nfood\t4\nsaturation\t2.4").unwrap();
        let tag = def.components();
        let inner = tag.get("components").unwrap();
        let props = inner.get("item_properties").unwrap();
        assert_eq!(props.get("max_stack_size").and_then(|t| t.as_int()), Some(64));
        // Pas une arme : aucun « point d'attaque » affiché
        assert_eq!(props.get("damage").and_then(|t| t.as_int()), Some(0));
        assert_eq!((icon_advice("diamond_sword"), icon_advice("netherite_pickaxe"), icon_advice("emerald"), icon_advice("diamond_spear")), (true, true, false, false));
        assert_eq!(props.get("use_duration").and_then(|t| t.as_int()), Some(32));
        assert_eq!(props.get("minecraft:icon").and_then(|i| i.get("textures")).and_then(|t| t.str_of("default")), Some("pumpkin_pie"));
        let food = inner.get("minecraft:food").unwrap();
        assert_eq!(food.get("nutrition").and_then(|t| t.as_int()), Some(4));
        // Le coefficient de la pomme du jeu : 4 de faim, 2,4 de satiété → 0,3
        assert!((food.get("saturation_modifier").and_then(|t| t.as_f32()).unwrap() - 0.3).abs() < 1e-6);
        assert_eq!(inner.get("minecraft:display_name").and_then(|d| d.str_of("value")), Some("Tarte"));
        // Un outil : usure déclarée au jeu, pile de 1
        let tool = CustomDef::parse("essai:pic", "durability\t500").unwrap().components();
        let inner = tool.get("components").unwrap();
        assert_eq!(inner.get("minecraft:durability").and_then(|d| d.get("max_durability")).and_then(|t| t.as_int()), Some(500));
        assert_eq!(inner.get("item_properties").and_then(|p| p.get("max_stack_size")).and_then(|t| t.as_int()), Some(1));
        assert!(!crate::nbt::write_network(&tool).is_empty());
    }
}
