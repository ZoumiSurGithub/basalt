//! Outils et armes : `Tool` est bâti sur `Item` et y ajoute un type, un matériau, une usure
//! et une vitesse de minage.

use super::item::{BaseItem, Item};

/// Famille d'outil avec laquelle un bloc se travaille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolType {
    None,
    Pickaxe,
    Axe,
    Shovel,
    Hoe,
    Sword,
    Shears,
}

/// Matériau d'un outil, du plus faible au plus fort pour ce qui est de récolter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolTier {
    Wood,
    Gold,
    Stone,
    Copper,
    Iron,
    Diamond,
    Netherite,
}

impl ToolTier {
    /// Préfixe du nom des objets de ce matériau.
    pub fn from_prefix(prefix: &str) -> Option<ToolTier> {
        Some(match prefix {
            "wooden" => ToolTier::Wood,
            "golden" => ToolTier::Gold,
            "stone" => ToolTier::Stone,
            "copper" => ToolTier::Copper,
            "iron" => ToolTier::Iron,
            "diamond" => ToolTier::Diamond,
            "netherite" => ToolTier::Netherite,
            _ => return None,
        })
    }

    /// Niveau de récolte : un bloc exige un niveau minimal pour donner son butin.
    /// L'or mine vite mais ne récolte pas mieux que le bois.
    pub fn harvest_level(self) -> u8 {
        match self {
            ToolTier::Wood | ToolTier::Gold => 1,
            ToolTier::Stone | ToolTier::Copper => 2,
            ToolTier::Iron => 3,
            ToolTier::Diamond => 4,
            ToolTier::Netherite => 5,
        }
    }

    /// Usure maximale par matériau.
    pub fn durability(self) -> u32 {
        match self {
            ToolTier::Wood => 59,
            ToolTier::Gold => 32,
            ToolTier::Stone => 131,
            ToolTier::Copper => 190,
            ToolTier::Iron => 250,
            ToolTier::Diamond => 1561,
            ToolTier::Netherite => 2031,
        }
    }

    /// Vitesse de minage par matériau.
    pub fn efficiency(self) -> f32 {
        match self {
            ToolTier::Wood => 2.0,
            ToolTier::Stone => 4.0,
            ToolTier::Copper => 5.0,
            ToolTier::Iron => 6.0,
            ToolTier::Diamond => 8.0,
            ToolTier::Netherite => 9.0,
            ToolTier::Gold => 12.0,
        }
    }

    /// Dégâts ajoutés par le matériau.
    fn damage_bonus(self) -> f32 {
        match self {
            ToolTier::Wood | ToolTier::Gold => 0.0,
            ToolTier::Stone | ToolTier::Copper => 1.0,
            ToolTier::Iron => 2.0,
            ToolTier::Diamond => 3.0,
            ToolTier::Netherite => 4.0,
        }
    }
}

/// Pioche, hache, pelle, houe, épée ou cisailles.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub base: BaseItem,
    pub kind: ToolType,
    /// Matériau ; les cisailles n'en ont pas
    pub tier: Option<ToolTier>,
}

impl Tool {
    /// Outil d'une sorte et d'un matériau ; ne s'empile pas.
    pub fn new(name: &str, kind: ToolType, tier: Option<ToolTier>) -> Tool {
        Tool { base: BaseItem::new(name, 1), kind, tier }
    }
}

impl Item for Tool {
    /// Usure maximale ; les cisailles ont la leur.
    fn base(&self) -> &BaseItem {
        &self.base
    }
    fn max_durability(&self) -> u32 {
        match (self.kind, self.tier) {
            (ToolType::Shears, _) => 238,
            (_, Some(tier)) => tier.durability(),
            _ => 0,
        }
    }
    /// Sorte et matériau de l'outil.
    fn tool_type(&self) -> ToolType {
        self.kind
    }
    fn tool_tier(&self) -> Option<ToolTier> {
        self.tier
    }
    /// Vitesse de minage sur un bloc : celle du matériau si c'est le bon outil, 1 sinon.
    fn mining_efficiency(&self, block_tool: ToolType) -> f32 {
        if block_tool == ToolType::None || block_tool != self.kind {
            // L'épée tranche vite ce qui se coupe aux cisailles (toiles) ; sinon, comme à la main
            return if self.kind == ToolType::Sword && block_tool == ToolType::Shears { 15.0 } else { 1.0 };
        }
        match (self.kind, self.tier) {
            (ToolType::Shears, _) => 5.0,
            (_, Some(tier)) => tier.efficiency(),
            _ => 1.0,
        }
    }
    /// Dégâts d'un coup avec cet outil.
    fn attack_damage(&self) -> f32 {
        let bonus = self.tier.map(|t| t.damage_bonus()).unwrap_or(0.0);
        match self.kind {
            ToolType::Sword => 5.0 + bonus,
            ToolType::Axe => 4.0 + bonus,
            ToolType::Pickaxe => 3.0 + bonus,
            ToolType::Shovel => 2.0 + bonus,
            _ => 1.0,
        }
    }
}
