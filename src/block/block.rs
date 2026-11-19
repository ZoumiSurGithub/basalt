//! `Block` : la base de tous les blocs du jeu. Un bloc concret (pierre, minerai, coffre…)
//! fournit sa fiche `BaseBlock` et redéfinit seulement ce qui le distingue ; tout le reste
//! (peut-on le récolter, en combien de temps, que donne-t-il) est hérité d'ici.

use crate::item::{Item, ToolTier, ToolType};

/// Propriétés physiques d'un bloc, telles que les données du jeu les donnent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockProps {
    /// Dureté ; négative pour un bloc incassable
    pub hardness: f32,
    pub blast_resistance: f32,
    /// Lumière émise, de 0 à 15
    pub light: u8,
    /// Facilité à brûler (0 : ne brûle pas)
    pub flammability: u8,
    pub friction: f32,
    /// Part de lumière arrêtée, de 0 (laisse tout passer) à 1 (opaque)
    pub opacity: f32,
}

impl BlockProps {
    /// Valeurs d'un bloc ordinaire de dureté donnée, quand les données du jeu manquent.
    pub fn plain(hardness: f32) -> BlockProps {
        BlockProps { hardness, blast_resistance: hardness.max(0.0), light: 0, flammability: 0, friction: 0.6, opacity: 1.0 }
    }
}

/// Ce que tout bloc possède.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseBlock {
    /// Nom complet, par exemple `minecraft:stone`
    pub name: String,
    /// Dureté ; négative pour un bloc incassable
    pub hardness: f32,
    pub props: BlockProps,
    /// Outil avec lequel le bloc se travaille vite
    pub tool: ToolType,
    /// Matériau minimal de cet outil pour que le bloc donne son butin (None : même à la main)
    pub required_tier: Option<ToolTier>,
}

impl BaseBlock {
    /// Bloc décrit par les propriétés des données du jeu.
    pub fn with_props(name: &str, props: BlockProps, tool: ToolType, required_tier: Option<ToolTier>) -> BaseBlock {
        BaseBlock { props, ..BaseBlock::new(name, props.hardness, tool, required_tier) }
    }

    pub fn new(name: &str, hardness: f32, tool: ToolType, required_tier: Option<ToolTier>) -> BaseBlock {
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{}", name) };
        BaseBlock { name, hardness, props: BlockProps::plain(hardness), tool, required_tier }
    }
}

/// Butin d'un bloc cassé : (nom de l'objet, quantité).
#[derive(Debug, Clone, PartialEq)]
pub struct Drop {
    pub item: String,
    pub count: u8,
}

impl Drop {
    pub fn new(item: &str, count: u8) -> Drop {
        let item = if item.contains(':') { item.to_string() } else { format!("minecraft:{}", item) };
        Drop { item, count }
    }
}

/// Écran qu'un bloc ouvre chez le joueur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerType {
    Chest,
    EnderChest,
    Barrel,
    ShulkerBox,
    Furnace,
    BlastFurnace,
    Smoker,
    CraftingTable,
    Anvil,
    EnchantingTable,
    BrewingStand,
    Hopper,
    Dispenser,
    Dropper,
    SmithingTable,
    Grindstone,
    Loom,
    Stonecutter,
    CartographyTable,
    Beacon,
    Lectern,
    Crafter,
}

/// Ce qui se passe quand un joueur clique sur le bloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    None,
    /// Ouvre un écran (coffre, four, établi, enclume…)
    Open(ContainerType),
    /// Porte, trappe, portillon, levier, bouton : change d'état
    Toggle,
}

/// Données attachées à certains blocs (« tile » dans PocketMine) : contenu d'un coffre,
/// texte d'un panneau… Voir le module `tile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileType {
    Chest,
    Barrel,
    ShulkerBox,
    EnderChest,
    Furnace,
    BlastFurnace,
    Smoker,
    Hopper,
    Dispenser,
    Dropper,
    BrewingStand,
    Sign,
    /// Bloc qui a des données propres que le serveur ne gère pas encore en détail
    /// (lit, bannière, tête, pot de fleurs, balise, générateur…)
    Other,
}

pub trait Block {
    /// Fiche du bloc : la seule chose qu'un bloc concret doit fournir.
    fn base(&self) -> &BaseBlock;

    // ---- hérité par tous les blocs, redéfini au besoin --------------------------

    fn name(&self) -> &str {
        &self.base().name
    }

    /// Nom sans le préfixe `minecraft:`.
    fn short_name(&self) -> &str {
        let name = self.name();
        name.strip_prefix("minecraft:").unwrap_or(name)
    }

    fn hardness(&self) -> f32 {
        self.base().hardness
    }

    fn tool_type(&self) -> ToolType {
        self.base().tool
    }

    fn required_tier(&self) -> Option<ToolTier> {
        self.base().required_tier
    }

    fn is_breakable(&self) -> bool {
        self.hardness() >= 0.0
    }

    fn blast_resistance(&self) -> f32 {
        self.base().props.blast_resistance
    }

    fn friction(&self) -> f32 {
        self.base().props.friction
    }

    /// Le feu peut-il prendre sur ce bloc ?
    fn is_flammable(&self) -> bool {
        self.base().props.flammability > 0
    }

    /// Un joueur bute-t-il contre ce bloc ? (les fleurs, torches et herbes ne sont pas solides)
    fn is_solid(&self) -> bool {
        let p = &self.base().props;
        !(p.hardness == 0.0 && p.opacity < 0.2)
    }

    fn is_transparent(&self) -> bool {
        self.base().props.opacity < 1.0
    }

    /// Données attachées au bloc, s'il en a.
    fn tile(&self) -> Option<TileType> {
        None
    }

    /// Un joueur peut-il poser un autre bloc à sa place (air, eau, herbes) ?
    fn is_replaceable(&self) -> bool {
        false
    }

    fn light_level(&self) -> u8 {
        self.base().props.light
    }

    fn interaction(&self) -> Interaction {
        Interaction::None
    }

    /// Cet outil permet-il de récupérer le butin du bloc ?
    fn can_harvest(&self, tool: &dyn Item) -> bool {
        match self.required_tier() {
            None => true,
            Some(needed) => tool.tool_type() == self.tool_type() && tool.tool_tier().map(|t| t.harvest_level() >= needed.harvest_level()).unwrap_or(false),
        }
    }

    /// Butin du bloc avec le bon outil (par défaut : le bloc lui-même).
    fn drops_with_tool(&self, _tool: &dyn Item) -> Vec<Drop> {
        vec![Drop::new(self.name(), 1)]
    }

    /// Butin réel : rien si le bloc est incassable ou si l'outil ne permet pas de le récolter.
    fn drops(&self, tool: &dyn Item) -> Vec<Drop> {
        if !self.is_breakable() || !self.can_harvest(tool) {
            return Vec::new();
        }
        self.drops_with_tool(tool)
    }

    /// Temps de minage en secondes avec cet outil (règle du jeu : la dureté, multipliée par
    /// 1,5 si l'outil permet la récolte et par 5 sinon, divisée par la vitesse de l'outil).
    /// Infini pour un bloc incassable.
    fn break_time(&self, tool: &dyn Item) -> f32 {
        let hardness = self.hardness();
        if hardness < 0.0 {
            return f32::INFINITY;
        }
        let factor = if self.can_harvest(tool) { 1.5 } else { 5.0 };
        hardness * factor / tool.mining_efficiency(self.tool_type()).max(1.0)
    }
}
