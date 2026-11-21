//! Armures et nourriture : deux autres familles d'objets bâties sur `Item`.

use super::item::{BaseItem, FoodValues, Item};

/// Pièce d'armure.
#[derive(Debug, Clone, PartialEq)]
pub struct Armor {
    pub base: BaseItem,
    /// 0 casque, 1 plastron, 2 jambières, 3 bottes (l'ordre de `ArmorInventory`)
    pub slot: u8,
    pub points: u8,
    pub durability: u32,
}

impl Armor {
    /// Reconnaît une pièce d'armure à son nom (`iron_chestplate`), ou rien.
    pub fn from_name(short: &str) -> Option<Armor> {
        let (material, piece) = short.rsplit_once('_')?;
        let slot = match piece {
            "helmet" => 0,
            "chestplate" => 1,
            "leggings" => 2,
            "boots" => 3,
            _ => return None,
        };
        // (points par pièce, facteur d'usure)
        let (points, factor): ([u8; 4], u32) = match material {
            "leather" => ([1, 3, 2, 1], 5),
            "golden" => ([2, 5, 3, 1], 7),
            "chainmail" => ([2, 5, 4, 1], 15),
            "copper" => ([2, 4, 3, 1], 11),
            "iron" => ([2, 6, 5, 2], 15),
            "turtle" if slot == 0 => ([2, 0, 0, 0], 25),
            "diamond" => ([3, 8, 6, 3], 33),
            "netherite" => ([3, 8, 6, 3], 37),
            _ => return None,
        };
        let wear = [11u32, 16, 15, 13][slot as usize];
        Some(Armor { base: BaseItem::new(short, 1), slot, points: points[slot as usize], durability: wear * factor })
    }
}

impl Item for Armor {
    /// Usure et protection de la pièce d'armure.
    fn base(&self) -> &BaseItem {
        &self.base
    }
    fn max_durability(&self) -> u32 {
        self.durability
    }
    fn armor_points(&self) -> u8 {
        self.points
    }
}

/// Aliment.
#[derive(Debug, Clone, PartialEq)]
pub struct Food {
    pub base: BaseItem,
    pub values: FoodValues,
}

impl Food {
    /// Valeurs nutritives d'un aliment (faim rendue, saturation) ; None si ça ne se mange pas.
    pub fn from_name(short: &str) -> Option<Food> {
        let (hunger, saturation) = match short {
            "apple" => (4, 2.4),
            "golden_apple" | "enchanted_golden_apple" => (4, 9.6),
            "bread" => (5, 6.0),
            "cookie" => (2, 0.4),
            "melon_slice" => (2, 1.2),
            "carrot" => (3, 3.6),
            "golden_carrot" => (6, 14.4),
            "potato" => (1, 0.6),
            "baked_potato" => (5, 6.0),
            "beetroot" => (1, 1.2),
            "sweet_berries" | "glow_berries" => (2, 0.4),
            "dried_kelp" => (1, 0.6),
            "pumpkin_pie" => (8, 4.8),
            "beef" | "porkchop" => (3, 1.8),
            "cooked_beef" | "cooked_porkchop" => (8, 12.8),
            "chicken" | "mutton" => (2, 1.2),
            "cooked_chicken" | "cooked_mutton" => (6, 9.6),
            "rabbit" => (3, 1.8),
            "cooked_rabbit" => (5, 6.0),
            "cod" | "salmon" => (2, 0.4),
            "cooked_cod" => (5, 6.0),
            "cooked_salmon" => (6, 9.6),
            "rotten_flesh" => (4, 0.8),
            _ => return None,
        };
        Some(Food { base: BaseItem::new(short, 64), values: FoodValues { hunger, saturation } })
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
