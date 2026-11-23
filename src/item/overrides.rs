//! Objets modifiés par les plugins.
//!
//! Un plugin peut changer ce que répond le gabarit d'un objet existant
//! (`server.items.modify("minecraft:bread", { food = 20 })`, voir `docs/PLUGINS.md`). La
//! modification est rangée ici, par nom d'objet ; `VanillaItems::get` enveloppe alors
//! l'objet dans `Modified`, qui répond à la place du gabarit pour ce qui a été changé et
//! lui laisse tout le reste. Le serveur ne voit aucune différence : il interroge un `Item`.
//!
//! Ne sont proposées que les valeurs dont le serveur décide seul. Ce que le jeu du joueur
//! calcule de son côté (taille de pile, vitesse de minage, durée de l'animation du repas)
//! ne peut pas être changé d'ici sans que l'écran du joueur dise autre chose que le serveur.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use super::consumable::EAT_TICKS;
use super::durable::Usage;
use super::item::{BaseItem, FoodValues, Item};
use super::tool::{ToolTier, ToolType};

/// Ce qu'un plugin a changé sur un objet. `None` : la valeur du gabarit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemOverride {
    /// Points de faim rendus
    pub food: Option<u8>,
    /// Satiété rendue
    pub saturation: Option<f32>,
    /// Ce qui reste en main après le repas (nom court ; chaîne vide : rien)
    pub leftover: Option<&'static str>,
    /// Utilisations avant de casser (0 : incassable)
    pub durability: Option<u32>,
    /// Dégâts d'un coup
    pub attack_damage: Option<f32>,
}

static ANY: AtomicBool = AtomicBool::new(false);
static TABLE: RwLock<Option<HashMap<String, ItemOverride>>> = RwLock::new(None);

fn short(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// Change une valeur d'un objet. `key` : `food`, `saturation`, `leftover`, `durability`,
/// `attackDamage` ; `reset` rend l'objet au gabarit. Erreur en clair si la valeur ne va pas.
pub fn set(name: &str, key: &str, value: &str) -> Result<(), String> {
    let number = || value.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| format!("{} attend un nombre positif, reçu « {} »", key, value));
    let mut table = TABLE.write().map_err(|_| "table des objets indisponible".to_string())?;
    let map = table.get_or_insert_with(HashMap::new);
    if key == "reset" {
        map.remove(short(name));
        ANY.store(!map.is_empty(), Ordering::Relaxed);
        return Ok(());
    }
    let mut o = map.get(short(name)).cloned().unwrap_or_default();
    match key {
        "food" => o.food = Some(number()?.min(20.0) as u8),
        "saturation" => o.saturation = Some(number()?.min(20.0) as f32),
        "durability" => o.durability = Some(number()?.min(u16::MAX as f64) as u32),
        "attackDamage" => o.attack_damage = Some(number()?.min(1000.0) as f32),
        // Le nom vit autant que le serveur : une poignée d'octets par modification
        "leftover" => o.leftover = Some(Box::leak(short(value).to_string().into_boxed_str())),
        _ => return Err(format!("valeur inconnue « {} » (food, saturation, leftover, durability, attackDamage)", key)),
    }
    map.insert(short(name).to_string(), o);
    ANY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Oublie toutes les modifications (rechargement des plugins).
pub fn clear() {
    if let Ok(mut table) = TABLE.write() {
        *table = None;
    }
    ANY.store(false, Ordering::Relaxed);
}

/// L'objet tel que les plugins l'ont modifié ; lui-même s'ils n'y ont pas touché.
pub fn apply(item: Box<dyn Item>) -> Box<dyn Item> {
    if !ANY.load(Ordering::Relaxed) {
        return item;
    }
    let found = TABLE.read().ok().and_then(|t| t.as_ref().and_then(|m| m.get(item.short_name()).cloned()));
    match found {
        Some(changes) => Box::new(Modified { inner: item, changes }),
        None => item,
    }
}

/// Un objet du jeu, avec ce qu'un plugin y a changé.
pub struct Modified {
    inner: Box<dyn Item>,
    changes: ItemOverride,
}

impl Item for Modified {
    fn base(&self) -> &BaseItem {
        self.inner.base()
    }
    fn max_stack_size(&self) -> u8 {
        self.inner.max_stack_size()
    }
    fn max_durability(&self) -> u32 {
        self.changes.durability.unwrap_or_else(|| self.inner.max_durability())
    }
    /// Un objet rendu durable par un plugin s'use d'un cran par usage.
    fn wear(&self, usage: Usage) -> u16 {
        match (self.inner.wear(usage), self.changes.durability) {
            (0, Some(d)) if d > 0 && self.inner.max_durability() == 0 => 1,
            (wear, _) => wear,
        }
    }
    fn tool_type(&self) -> ToolType {
        self.inner.tool_type()
    }
    fn tool_tier(&self) -> Option<ToolTier> {
        self.inner.tool_tier()
    }
    fn mining_efficiency(&self, block_tool: ToolType) -> f32 {
        self.inner.mining_efficiency(block_tool)
    }
    fn attack_damage(&self) -> f32 {
        self.changes.attack_damage.unwrap_or_else(|| self.inner.attack_damage())
    }
    fn armor_points(&self) -> u8 {
        self.inner.armor_points()
    }
    /// Seul ce qui se mange déjà peut être changé : le jeu du joueur ne porte rien d'autre
    /// à la bouche.
    fn food(&self) -> Option<FoodValues> {
        let c = &self.changes;
        self.inner.food().map(|f| FoodValues {
            hunger: c.food.unwrap_or(f.hunger),
            saturation: c.saturation.unwrap_or(f.saturation),
            leftover: match c.leftover {
                Some("") => None,
                Some(name) => Some(name),
                None => f.leftover,
            },
            eat_ticks: if f.eat_ticks == 0 { EAT_TICKS } else { f.eat_ticks },
            ..f
        })
    }
    fn block(&self) -> Option<&str> {
        self.inner.block()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{apply_wear, ItemStack, VanillaItems};

    #[test]
    fn plugins_change_what_an_item_answers() {
        // (objets que les autres tests n'interrogent pas : la table est commune)
        assert_eq!(VanillaItems::get("tropical_fish").food().map(|f| f.hunger), Some(1));
        set("minecraft:tropical_fish", "food", "12").unwrap();
        set("tropical_fish", "leftover", "minecraft:bone").unwrap();
        let fish = VanillaItems::get("minecraft:tropical_fish").food().unwrap();
        assert_eq!((fish.hunger, fish.saturation, fish.leftover), (12, 0.2, Some("bone")));
        // Une houe en pierre qui casse au troisième usage ; un bâton rendu durable
        set("stone_hoe", "durability", "3").unwrap();
        set("stone_hoe", "attackDamage", "9.5").unwrap();
        let hoe = VanillaItems::get("stone_hoe");
        assert_eq!((hoe.max_durability(), hoe.attack_damage()), (3, 9.5));
        let mut stack = ItemStack::new(1, 1, 0);
        assert!(!apply_wear(&mut stack, hoe.as_ref(), Usage::Mine) && !apply_wear(&mut stack, hoe.as_ref(), Usage::Mine));
        assert!(apply_wear(&mut stack, hoe.as_ref(), Usage::Mine));
        // Valeurs refusées, sans rien changer
        assert!(set("stone_hoe", "durability", "beaucoup").is_err() && set("stone_hoe", "couleur", "1").is_err());
        // Retour au gabarit
        set("tropical_fish", "reset", "").unwrap();
        set("stone_hoe", "reset", "").unwrap();
        assert_eq!(VanillaItems::get("tropical_fish").food().map(|f| f.hunger), Some(1));
        assert_eq!(VanillaItems::get("stone_hoe").max_durability(), 131);
    }
}
