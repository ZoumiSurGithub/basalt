//! Les autres inventaires concrets, tous bâtis sur `BaseInventory`.

use super::base_inventory::{inventory_type, plain_inventory};
use super::inventory::Inventory;
use crate::item::ItemStack;

inventory_type!(
    /// Armure portée : casque, plastron, jambières, bottes.
    ArmorInventory,
    "armure",
    4
);
plain_inventory!(ArmorInventory);

impl ArmorInventory {
    /// Cases d'armure : casque, plastron, jambières, bottes.
    pub fn helmet(&self) -> ItemStack {
        self.slots[0]
    }
    pub fn chestplate(&self) -> ItemStack {
        self.slots[1]
    }
    pub fn leggings(&self) -> ItemStack {
        self.slots[2]
    }
    pub fn boots(&self) -> ItemStack {
        self.slots[3]
    }
}

inventory_type!(
    /// Objet tenu au bout du curseur dans un écran d'inventaire.
    PlayerCursorInventory,
    "curseur",
    1
);
plain_inventory!(PlayerCursorInventory);

inventory_type!(
    /// Seconde main.
    PlayerOffHandInventory,
    "seconde main",
    1
);
plain_inventory!(PlayerOffHandInventory);

inventory_type!(
    /// Grille de fabrication. Le jeu numérote ses cases 28 à 31 (grille 2×2 de l'inventaire)
    /// et 32 à 40 (établi) : ces numéros sont gardés tels quels, les cases 0 à 27 ne servent pas.
    CraftingGrid,
    "fabrication",
    41
);

impl Inventory for CraftingGrid {
    /// Grille de fabrication et ses cases de sortie.
    fn name(&self) -> &'static str {
        "fabrication"
    }
    fn slots(&self) -> &[ItemStack] {
        &self.base.slots
    }
    fn slots_mut(&mut self) -> &mut [ItemStack] {
        &mut self.base.slots
    }
    /// Seules les cases de la grille (28 à 40) reçoivent des objets.
    fn accepts(&self, slot: usize, _item: &ItemStack) -> bool {
        (28..=40).contains(&slot)
    }
}

inventory_type!(
    /// Résultat d'une création (menu créatif, fabrication) en attente d'être pris.
    CraftingResultInventory,
    "résultat",
    1
);
plain_inventory!(CraftingResultInventory);

inventory_type!(
    /// Coffre simple : 27 emplacements.
    ChestInventory,
    "coffre",
    27
);
plain_inventory!(ChestInventory);

inventory_type!(
    /// Four : ingrédient, combustible, résultat.
    FurnaceInventory,
    "four",
    3
);
plain_inventory!(FurnaceInventory);

impl FurnaceInventory {
    /// Cases du four : à cuire, combustible, résultat.
    pub fn smelting(&self) -> ItemStack {
        self.slots[0]
    }
    pub fn fuel(&self) -> ItemStack {
        self.slots[1]
    }
    pub fn result(&self) -> ItemStack {
        self.slots[2]
    }
}
