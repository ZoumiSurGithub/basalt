//! Gabarit `Food` : un objet qui se mange (l'équivalent de `Food` et `ConsumableItem` dans
//! PocketMine).
//!
//! Tout ce qui décrit un repas est ici, en un seul endroit : ce qu'il rend de faim et de
//! satiété, s'il se mange le ventre plein, combien de temps il faut pour le manger et ce
//! qu'il laisse en main. Le serveur (`server/survival.rs`) ne connaît aucun aliment par son
//! nom : il demande à l'objet tenu `item.food()` et applique ce qu'il reçoit.
//!
//! Ajouter un aliment : une ligne dans `Food::from_name`. Changer une règle pour un aliment
//! particulier : écrire une structure qui contient un `BaseItem` et redéfinir `food()` dans
//! son `impl Item` (voir `docs/ARCHITECTURE.md`).

use super::item::{BaseItem, FoodValues, Item};

/// Durée d'un repas ordinaire, en ticks (1,6 seconde).
pub const EAT_TICKS: u32 = 32;

/// Aliment.
#[derive(Debug, Clone, PartialEq)]
pub struct Food {
    pub base: BaseItem,
    pub values: FoodValues,
}

impl Food {
    /// Reconnaît un aliment à son nom court (`bread`) ; `None` si ça ne se mange pas.
    /// `stack` : taille de pile de l'objet (un ragoût ne s'empile pas).
    pub fn from_name(short: &str, stack: u8) -> Option<Food> {
        // (faim rendue, satiété) — les valeurs du jeu
        let (hunger, saturation): (u8, f32) = match short {
            "apple" => (4, 2.4),
            "baked_potato" => (5, 6.0),
            "beef" | "porkchop" | "rabbit" => (3, 1.8),
            "beetroot" => (1, 1.2),
            "beetroot_soup" => (6, 7.2),
            "bread" => (5, 6.0),
            "carrot" => (3, 3.6),
            "chicken" | "mutton" => (2, 1.2),
            "chorus_fruit" => (4, 2.4),
            "cod" | "salmon" => (2, 0.4),
            "cooked_beef" | "cooked_porkchop" => (8, 12.8),
            "cooked_chicken" => (6, 7.2),
            "cooked_cod" => (5, 6.0),
            "cooked_mutton" | "cooked_salmon" => (6, 9.6),
            "cooked_rabbit" => (5, 6.0),
            "cookie" => (2, 0.4),
            "dried_kelp" => (1, 0.6),
            "golden_apple" | "enchanted_golden_apple" => (4, 9.6),
            "golden_carrot" => (6, 14.4),
            "glow_berries" | "sweet_berries" => (2, 0.4),
            "honey_bottle" => (6, 1.2),
            "melon_slice" => (2, 1.2),
            "mushroom_stew" | "suspicious_stew" => (6, 7.2),
            "poisonous_potato" => (2, 1.2),
            "potato" => (1, 0.6),
            "pufferfish" => (1, 0.2),
            "pumpkin_pie" => (8, 4.8),
            "rabbit_stew" => (10, 12.0),
            "rotten_flesh" => (4, 0.8),
            "spider_eye" => (2, 3.2),
            "tropical_fish" => (1, 0.2),
            _ => return None,
        };
        let values = FoodValues {
            hunger,
            saturation,
            always_edible: matches!(short, "golden_apple" | "enchanted_golden_apple" | "chorus_fruit" | "honey_bottle" | "suspicious_stew"),
            eat_ticks: match short {
                "dried_kelp" => 16,
                "honey_bottle" => 40,
                _ => EAT_TICKS,
            },
            leftover: match short {
                "mushroom_stew" | "rabbit_stew" | "beetroot_soup" | "suspicious_stew" => Some("bowl"),
                "honey_bottle" => Some("glass_bottle"),
                _ => None,
            },
        };
        Some(Food { base: BaseItem::new(short, stack), values })
    }
}

impl Item for Food {
    /// Aliment.
    fn base(&self) -> &BaseItem {
        &self.base
    }
    fn food(&self) -> Option<FoodValues> {
        Some(self.values)
    }
}
