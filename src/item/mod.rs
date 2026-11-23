//! Objets, organisés comme dans PocketMine et Altay.
//!
//! - `Item` : la base de tous les objets (nom, taille de pile, usure, minage, dégâts…) ;
//! - `Tool`, `Armor`, `Food`, `BlockItem`, `GenericItem` : les objets vanilla, bâtis dessus ;
//! - `VanillaItems` : le registre des objets vanilla, comme dans PocketMine 5 —
//!   `VanillaItems::diamond_pickaxe()`, ou `VanillaItems::get(nom)` d'après un nom ;
//! - `ItemStack` : une pile d'objets dans un inventaire.
//!
//! Ajouter un objet particulier revient à écrire une structure qui contient un `BaseItem`
//! et à redéfinir, dans son `impl Item`, les seules méthodes qui le distinguent.

mod armor;
mod consumable;
pub mod custom;
mod durable;
pub mod overrides;
mod vanilla_items;
#[allow(clippy::module_inception)]
mod item;
mod stack;
mod tool;

pub use self::armor::Armor;
pub use self::consumable::{Food, EAT_TICKS};
pub use self::durable::{apply_wear, Usage};
pub use self::vanilla_items::VanillaItems;
pub use self::item::{BaseItem, BlockItem, FoodValues, GenericItem, Item};
pub use self::stack::{ItemStack, MAX_STACK};
pub use self::tool::{Tool, ToolTier, ToolType};

use std::collections::HashMap;
use std::sync::RwLock;

/// Par objet du jeu chargé : (taille de pile, usure maximale). Rempli au chargement des
/// données ; sans données, tout s'empile par 64 et rien ne s'use.
static LIMITS: RwLock<Option<HashMap<i16, (u8, u16)>>> = RwLock::new(None);

/// Enregistre les limites de chaque objet d'après son nom (voir `VanillaItems`).
pub fn set_limits<'a>(items: impl Iterator<Item = (i16, &'a str)>) {
    let table: HashMap<i16, (u8, u16)> = items
        .map(|(id, name)| {
            let item = VanillaItems::get(name);
            (id, (item.max_stack_size(), item.max_durability().min(u16::MAX as u32) as u16))
        })
        .collect();
    if let Ok(mut limits) = LIMITS.write() {
        *limits = Some(table);
    }
}

/// Taille de pile d'un objet (64 s'il est inconnu).
pub fn max_stack_of(id: i16) -> u8 {
    LIMITS.read().ok().and_then(|l| l.as_ref().and_then(|t| t.get(&id).map(|v| v.0))).unwrap_or(MAX_STACK)
}

/// Usure qu'un outil supporte avant de casser (0 : l'objet ne s'use pas).
pub fn max_durability_of(id: i16) -> u16 {
    LIMITS.read().ok().and_then(|l| l.as_ref().and_then(|t| t.get(&id).map(|v| v.1))).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_items_inherit_from_the_same_base() {
        let pick = VanillaItems::get("minecraft:iron_pickaxe");
        assert_eq!((pick.name(), pick.short_name(), pick.max_stack_size()), ("minecraft:iron_pickaxe", "iron_pickaxe", 1));
        assert_eq!((pick.tool_type(), pick.tool_tier(), pick.max_durability()), (ToolType::Pickaxe, Some(ToolTier::Iron), 250));
        assert_eq!(pick.mining_efficiency(ToolType::Pickaxe), 6.0);
        assert_eq!(pick.mining_efficiency(ToolType::Axe), 1.0);
        assert!(pick.is_tool());

        let sword = VanillaItems::get("diamond_sword");
        assert_eq!((sword.attack_damage(), sword.max_durability()), (8.0, 1561));
        // L'or mine vite mais ne récolte pas mieux que le bois
        let gold = VanillaItems::get("golden_pickaxe");
        assert_eq!(gold.mining_efficiency(ToolType::Pickaxe), 12.0);
        assert!(gold.tool_tier().unwrap().harvest_level() < ToolTier::Stone.harvest_level());
        assert_eq!(VanillaItems::get("shears").tool_type(), ToolType::Shears);

        let chest = VanillaItems::get("diamond_chestplate");
        assert_eq!((chest.armor_points(), chest.max_durability(), chest.max_stack_size()), (8, 528, 1));
        let bread = VanillaItems::get("bread");
        assert_eq!(bread.food().map(|f| f.hunger), Some(5));
        assert!(!bread.is_tool());

        // Tailles de pile, et objet inconnu traité comme un objet ordinaire
        assert_eq!(VanillaItems::get("ender_pearl").max_stack_size(), 16);
        assert_eq!(VanillaItems::get("oak_sign").max_stack_size(), 16);
        assert_eq!(VanillaItems::get("water_bucket").max_stack_size(), 1);
        assert_eq!(VanillaItems::get("stick").max_stack_size(), 64);
        let unknown = VanillaItems::get("monplugin:rubis");
        assert_eq!((unknown.name(), unknown.max_stack_size(), unknown.tool_type()), ("monplugin:rubis", 64, ToolType::None));
        assert_eq!(VanillaItems::block_item("oak_planks").block(), Some("minecraft:oak_planks"));
        assert_eq!(VanillaItems::hand().mining_efficiency(ToolType::Pickaxe), 1.0);
    }

    #[test]
    fn items_are_reachable_by_name_like_in_pocketmine() {
        assert_eq!(VanillaItems::diamond_pickaxe().tool_tier(), Some(ToolTier::Diamond));
        assert_eq!(VanillaItems::iron_sword().attack_damage(), 7.0);
        assert_eq!(VanillaItems::stick().max_stack_size(), 64);
        assert_eq!(VanillaItems::bread().food().map(|f| f.hunger), Some(5));
        assert_eq!(VanillaItems::ender_pearl().max_stack_size(), 16);
        for (label, item) in VanillaItems::all() {
            assert_eq!(item.short_name(), label);
        }
        assert!(VanillaItems::all().len() >= 40);
    }
}
