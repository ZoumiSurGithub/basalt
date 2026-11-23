//! Gabarit `Armor` : une pièce d'armure (casque, plastron, jambières, bottes), bâtie sur
//! `Item`. Elle s'use comme un outil (voir `durable.rs`) et protège de ses points d'armure.

use super::item::{BaseItem, Item};

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
