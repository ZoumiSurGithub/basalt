//! Les types de blocs vanilla, tous bâtis sur `Block`. Chacun ne redéfinit que ce qui le
//! distingue du bloc ordinaire.

use super::block::{BaseBlock, Block, Drop, Interaction, TileType};
use crate::item::{Item, ToolType};

/// Bloc ordinaire : il se donne lui-même quand on le casse.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericBlock {
    pub base: BaseBlock,
}

impl Block for GenericBlock {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
}

/// L'air et ses semblables : ni solide, ni cassable, remplaçable.
#[derive(Debug, Clone, PartialEq)]
pub struct Air {
    pub base: BaseBlock,
}

impl Block for Air {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn is_breakable(&self) -> bool {
        false
    }
    fn is_solid(&self) -> bool {
        false
    }
    fn is_transparent(&self) -> bool {
        true
    }
    fn is_replaceable(&self) -> bool {
        true
    }
}

/// Eau et lave.
#[derive(Debug, Clone, PartialEq)]
pub struct Liquid {
    pub base: BaseBlock,
}

impl Block for Liquid {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn is_breakable(&self) -> bool {
        false
    }
    fn is_solid(&self) -> bool {
        false
    }
    fn is_transparent(&self) -> bool {
        true
    }
    fn is_replaceable(&self) -> bool {
        true
    }
}

/// Bloc qui donne autre chose que lui-même, ou plusieurs exemplaires : la pierre donne de
/// la pierre taillée, l'herbe de la terre, un minerai sa matière première, une double dalle
/// deux dalles.
#[derive(Debug, Clone, PartialEq)]
pub struct DropsOther {
    pub base: BaseBlock,
    pub item: String,
    pub count: u8,
}

impl Block for DropsOther {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn drops_with_tool(&self, _tool: &dyn Item) -> Vec<Drop> {
        vec![Drop::new(&self.item, self.count)]
    }
}

/// Bloc qui ne donne rien quand on le casse (verre, glace, générateur de monstres, feu) —
/// sauf, s'il est `shearable`, quand on le coupe aux cisailles (feuilles, herbes, lianes).
#[derive(Debug, Clone, PartialEq)]
pub struct Fragile {
    pub base: BaseBlock,
    /// Vrai si les cisailles récupèrent le bloc
    pub shearable: bool,
}

impl Block for Fragile {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn drops_with_tool(&self, tool: &dyn Item) -> Vec<Drop> {
        if self.shearable && tool.tool_type() == ToolType::Shears {
            vec![Drop::new(self.name(), 1)]
        } else {
            Vec::new()
        }
    }
}

/// Toile d'araignée : du fil à l'épée, la toile elle-même aux cisailles, rien autrement.
#[derive(Debug, Clone, PartialEq)]
pub struct Cobweb {
    pub base: BaseBlock,
}

impl Block for Cobweb {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn can_harvest(&self, tool: &dyn Item) -> bool {
        matches!(tool.tool_type(), ToolType::Sword | ToolType::Shears)
    }
    fn drops_with_tool(&self, tool: &dyn Item) -> Vec<Drop> {
        if tool.tool_type() == ToolType::Shears {
            vec![Drop::new(self.name(), 1)]
        } else {
            vec![Drop::new("string", 1)]
        }
    }
}

/// Plante cultivée : donne sa graine ou son fruit.
/// (Le stade de croissance n'est pas encore pris en compte : une plante mûre donne autant
/// qu'une jeune pousse.)
#[derive(Debug, Clone, PartialEq)]
pub struct Crop {
    pub base: BaseBlock,
    pub item: &'static str,
}

impl Block for Crop {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn is_solid(&self) -> bool {
        false
    }
    fn drops_with_tool(&self, _tool: &dyn Item) -> Vec<Drop> {
        vec![Drop::new(self.item, 1)]
    }
}

/// Bloc qui réagit au clic : coffre, four, établi, enclume, porte, levier… Il peut porter
/// des données (`tile`) et donner autre chose que lui-même (un four allumé donne un four).
#[derive(Debug, Clone, PartialEq)]
pub struct Interactive {
    pub base: BaseBlock,
    pub action: Interaction,
    pub tile: Option<TileType>,
    /// Objet donné quand on le casse (vide : le bloc lui-même)
    pub item: String,
    pub count: u8,
}

impl Block for Interactive {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn interaction(&self) -> Interaction {
        self.action
    }
    fn tile(&self) -> Option<TileType> {
        self.tile
    }
    fn drops_with_tool(&self, _tool: &dyn Item) -> Vec<Drop> {
        if self.item.is_empty() {
            vec![Drop::new(self.name(), 1)]
        } else {
            vec![Drop::new(&self.item, self.count)]
        }
    }
}

/// Bloc sans réaction au clic mais qui porte des données : panneau, lit, bannière, tête…
#[derive(Debug, Clone, PartialEq)]
pub struct WithTile {
    pub base: BaseBlock,
    pub tile: TileType,
    /// Objet donné quand on le casse (vide : le bloc lui-même)
    pub item: String,
}

impl Block for WithTile {
    fn base(&self) -> &BaseBlock {
        &self.base
    }
    fn tile(&self) -> Option<TileType> {
        Some(self.tile)
    }
    fn drops_with_tool(&self, _tool: &dyn Item) -> Vec<Drop> {
        if self.item.is_empty() {
            vec![Drop::new(self.name(), 1)]
        } else {
            vec![Drop::new(&self.item, 1)]
        }
    }
}
