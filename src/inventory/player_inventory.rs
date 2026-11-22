//! `PlayerInventory` : le sac du joueur. 36 emplacements, dont les 9 premiers forment la
//! barre d'objets ; l'un d'eux est tenu en main.

use super::base_inventory::{inventory_type, plain_inventory};
use super::inventory::Inventory;
use crate::item::ItemStack;

inventory_type!(
    /// Sac et barre d'objets du joueur.
    PlayerInventory,
    "joueur",
    36
);
plain_inventory!(PlayerInventory);

/// Nombre d'emplacements de la barre d'objets.
pub const HOTBAR_SIZE: usize = 9;

/// Emplacement de la barre tenu en main, rangé à part pour rester lié au sac.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeldSlot(pub u8);

impl PlayerInventory {
    /// Objet de la barre à l'emplacement `held`.
    pub fn item_in_hand(&self, held: HeldSlot) -> ItemStack {
        self.slots[(held.0 as usize).min(HOTBAR_SIZE - 1)]
    }

    /// Retire un exemplaire de l'objet tenu (bloc posé). Faux si la main est vide.
    pub fn consume_in_hand(&mut self, held: HeldSlot) -> bool {
        let slot = (held.0 as usize).min(HOTBAR_SIZE - 1);
        let stack = self.slots[slot];
        if stack.count == 0 {
            return false;
        }
        let left = ItemStack { count: stack.count - 1, ..stack };
        self.set_item(slot, left)
    }

}
