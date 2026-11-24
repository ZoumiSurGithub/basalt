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

/// Part des dégâts qui passe à travers une armure de tant de points : chaque point en
/// retire 4 %, jusqu'à 80 % pour une armure complète en diamant (20 points), comme dans le jeu.
pub fn armor_factor(points: u32) -> f32 {
    1.0 - points.min(20) as f32 * 0.04
}

/// Usure de chaque pièce pour un coup de tant de dégâts : un point par tranche de 4, un au moins.
pub fn armor_wear(damage: f32) -> u16 {
    ((damage / 4.0).floor() as u16).max(1)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armour_absorbs_four_percent_per_point() {
        assert_eq!(armor_factor(0), 1.0);
        // Armure de fer complète : 15 points, 60 % des dégâts en moins
        let iron: u32 = ["iron_helmet", "iron_chestplate", "iron_leggings", "iron_boots"].iter().map(|n| Armor::from_name(n).unwrap().points as u32).sum();
        assert_eq!(iron, 15);
        assert!((armor_factor(iron) - 0.4).abs() < 1e-6);
        // Diamant : 20 points, 80 % ; jamais plus
        assert!((armor_factor(20) - 0.2).abs() < 1e-6 && (armor_factor(40) - 0.2).abs() < 1e-6);
        assert_eq!((armor_wear(0.5), armor_wear(3.0), armor_wear(9.0)), (1, 1, 2));
    }
}
