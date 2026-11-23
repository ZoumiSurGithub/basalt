//! `Item` : la base de tous les objets du jeu. Un objet concret (outil, armure, nourriture…)
//! fournit sa fiche `BaseItem` et redéfinit seulement ce qui le distingue ; tout le reste est
//! hérité d'ici.

use super::durable::Usage;
use super::tool::{ToolTier, ToolType};

/// Ce que tout objet possède.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseItem {
    /// Nom complet, par exemple `minecraft:iron_pickaxe`
    pub name: String,
    pub max_stack_size: u8,
}

impl BaseItem {
    /// Valeurs communes d'un objet ; le préfixe minecraft: est ajouté s'il manque.
    pub fn new(name: &str, max_stack_size: u8) -> BaseItem {
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{}", name) };
        BaseItem { name, max_stack_size: max_stack_size.clamp(1, 64) }
    }
}

/// Valeur nutritive d'un aliment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoodValues {
    /// Points de faim rendus
    pub hunger: u8,
    pub saturation: f32,
    /// Se mange même le ventre plein (pomme dorée, fruit de chorus…)
    pub always_edible: bool,
    /// Durée du repas, en ticks
    pub eat_ticks: u32,
    /// Ce qui reste en main après le repas (nom court : `bowl`, `glass_bottle`)
    pub leftover: Option<&'static str>,
}

pub trait Item {
    /// Fiche de l'objet : la seule chose qu'un objet concret doit fournir.
    fn base(&self) -> &BaseItem;

    // ---- hérité par tous les objets, redéfini au besoin -------------------------

    /// Nom complet (minecraft:diamond).
    fn name(&self) -> &str {
        &self.base().name
    }

    /// Nom sans le préfixe `minecraft:`.
    fn short_name(&self) -> &str {
        let name = self.name();
        name.strip_prefix("minecraft:").unwrap_or(name)
    }

    /// Taille maximale d'une pile (64, 16 ou 1).
    fn max_stack_size(&self) -> u8 {
        self.base().max_stack_size
    }

    /// Nombre d'utilisations avant de casser (0 : l'objet ne s'use pas).
    fn max_durability(&self) -> u32 {
        0
    }

    /// Usure que coûte un usage (0 : aucun). Voir `durable.rs`.
    fn wear(&self, _usage: Usage) -> u16 {
        0
    }

    /// Sorte d'outil (None pour un objet ordinaire).
    fn tool_type(&self) -> ToolType {
        ToolType::None
    }

    /// Matériau de l'outil.
    fn tool_tier(&self) -> Option<ToolTier> {
        None
    }

    /// Vitesse de minage sur un bloc qui se travaille avec `block_tool` (1 : à la main).
    fn mining_efficiency(&self, _block_tool: ToolType) -> f32 {
        1.0
    }

    /// Dégâts d'un coup porté avec cet objet.
    fn attack_damage(&self) -> f32 {
        1.0
    }

    /// Points d'armure quand l'objet est porté.
    fn armor_points(&self) -> u8 {
        0
    }

    /// Valeur nutritive, si l'objet se mange.
    fn food(&self) -> Option<FoodValues> {
        None
    }

    /// Bloc posé quand on utilise l'objet sur le monde.
    fn block(&self) -> Option<&str> {
        None
    }

    /// Est-ce un outil ?
    fn is_tool(&self) -> bool {
        self.tool_type() != ToolType::None
    }
}

/// Objet sans particularité (et la main vide, sous le nom `minecraft:air`).
#[derive(Debug, Clone, PartialEq)]
pub struct GenericItem {
    pub base: BaseItem,
}

impl Item for GenericItem {
    /// Objet sans comportement particulier.
    fn base(&self) -> &BaseItem {
        &self.base
    }
}

/// Objet qui pose un bloc (la plupart des blocs ont le leur, du même nom).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockItem {
    pub base: BaseItem,
    pub block: String,
}

impl Item for BlockItem {
    /// Objet qui pose un bloc.
    fn base(&self) -> &BaseItem {
        &self.base
    }
    fn block(&self) -> Option<&str> {
        Some(&self.block)
    }
}
