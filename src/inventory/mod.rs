//! Inventaires, organisés comme dans PocketMine et Altay.
//!
//! - `Inventory` : la base commune (lire, écrire, ajouter, retirer, règles propres) ;
//! - `BaseInventory` : une simple rangée d'emplacements, dont tous les autres héritent ;
//! - `PlayerInventory`, `ArmorInventory`, `PlayerCursorInventory`, `PlayerOffHandInventory`,
//!   `CraftingGrid`, `ChestInventory`, `FurnaceInventory` : les inventaires concrets ;
//! - `PlayerWindows` : l'ensemble des inventaires d'un joueur ;
//! - `transaction` : les demandes du jeu (déplacer, échanger, jeter) et leurs règles.
//!
//! Ajouter un inventaire (une enclume, par exemple) revient à le déclarer avec
//! `inventory_type!` et, s'il a des règles, à écrire sa méthode `accepts`.

mod base_inventory;
mod containers;
#[allow(clippy::module_inception)]
mod inventory;
mod player_inventory;
mod player_windows;
pub mod transaction;

pub use self::base_inventory::BaseInventory;
pub use self::containers::{ArmorInventory, ChestInventory, CraftingGrid, CraftingResultInventory, FurnaceInventory, PlayerCursorInventory, PlayerOffHandInventory};
pub use self::inventory::Inventory;
pub use self::player_inventory::{HeldSlot, PlayerInventory, HOTBAR_SIZE};
pub use self::player_windows::PlayerWindows;
pub use self::transaction::{protocol_name, resolve, Action, Target};
pub use crate::item::{ItemStack, MAX_STACK};

#[cfg(test)]
mod tests {
    use super::*;

    const P: Target = Target::Player;
    const C: Target = Target::Cursor;

    #[test]
    fn adding_stacks_then_fills_free_slots() {
        let mut inv = PlayerWindows::new();
        assert_eq!(inv.add(1, 70), 0);
        assert_eq!((inv.main.slots[0].count, inv.main.slots[1].count), (64, 6));
        assert_ne!(inv.main.slots[0].net_id, inv.main.slots[1].net_id);
        assert_eq!(inv.add(1, 10), 0);
        assert_eq!(inv.main.slots[1].count, 16);
        // inventaire plein : le surplus est signalé
        assert_eq!(inv.add(3, 36 * 64), 2 * 64);
    }

    #[test]
    fn moving_splitting_and_merging() {
        let mut inv = PlayerWindows::new();
        inv.add(1, 64);
        let original = inv.main.slots[0].net_id;
        // prendre la moitié au curseur : nouvelle pile, nouveau numéro
        assert!(inv.apply(Action::Move { count: 32, from: (P, 0), to: (C, 0) }).is_some());
        assert_eq!((inv.main.slots[0].count, inv.cursor.slots[0].count), (32, 32));
        assert_ne!(inv.cursor.slots[0].net_id, original);
        // la déposer ailleurs en entier : le numéro suit la pile
        let cursor_id = inv.cursor.slots[0].net_id;
        assert!(inv.apply(Action::Move { count: 32, from: (C, 0), to: (P, 20) }).is_some());
        assert!(inv.cursor.slots[0].is_empty());
        assert_eq!(inv.main.slots[20].net_id, cursor_id);
        // fusionner dans la première pile
        assert!(inv.apply(Action::Move { count: 32, from: (P, 20), to: (P, 0) }).is_some());
        assert_eq!(inv.main.slots[0].count, 64);
        assert!(inv.main.slots[20].is_empty());
    }

    #[test]
    fn impossible_requests_change_nothing() {
        let mut inv = PlayerWindows::new();
        inv.add(1, 10);
        inv.add(3, 5);
        let before = inv.main.slots.clone();
        // plus d'objets qu'il n'y en a, pile d'un autre objet, emplacement inexistant, conteneur absent
        assert!(inv.apply(Action::Move { count: 11, from: (P, 0), to: (P, 5) }).is_none());
        assert!(inv.apply(Action::Move { count: 1, from: (P, 0), to: (P, 1) }).is_none());
        assert!(inv.apply(Action::Move { count: 1, from: (P, 0), to: (P, 99) }).is_none());
        assert!(inv.apply(Action::Move { count: 1, from: (P, 0), to: (Target::Block, 0) }).is_none());
        assert!(inv.apply(Action::Remove { count: 20, from: (P, 0) }).is_none());
        assert!(inv.apply(Action::Move { count: 0, from: (P, 0), to: (P, 5) }).is_none());
        assert_eq!(inv.main.slots, before);
        // dépassement de la taille d'une pile
        inv.add(1, 118);
        assert!(inv.apply(Action::Move { count: 10, from: (P, 0), to: (P, 2) }).is_none());
    }

    #[test]
    fn swap_remove_and_held_item() {
        let mut inv = PlayerWindows::new();
        inv.add(1, 2);
        inv.add(3, 7);
        assert!(inv.apply(Action::Swap { a: (P, 0), b: (P, 1) }).is_some());
        assert_eq!((inv.main.slots[0].id, inv.main.slots[1].id), (3, 1));
        assert!(inv.apply(Action::Remove { count: 7, from: (P, 0) }).is_some());
        assert!(inv.main.slots[0].is_empty());
        inv.selected = 1;
        assert_eq!(inv.held().id, 1);
        assert!(inv.consume_held());
        assert!(inv.consume_held());
        assert!(!inv.consume_held());
        assert!(inv.held().is_empty());
    }

    #[test]
    fn protocol_slots_are_resolved() {
        assert_eq!(resolve(28, 3), Some((P, 3)));
        assert_eq!(resolve(29, 20), Some((P, 20)));
        assert_eq!(resolve(29, 3), None);
        assert_eq!(resolve(28, 9), None);
        assert_eq!(resolve(59, 0), Some((C, 0)));
        assert_eq!(resolve(200, 0), None);
        assert_eq!(protocol_name(P, 4), 28);
        assert_eq!(protocol_name(P, 30), 29);
    }

    #[test]
    fn every_inventory_inherits_the_same_base() {
        let mut next = 0;
        let mut id = || {
            next += 1;
            next
        };
        // Les mêmes opérations sur un coffre, un four et une armure
        let mut chest = ChestInventory::new();
        assert_eq!((chest.size(), Inventory::name(&chest)), (27, "coffre"));
        assert_eq!(chest.add_item(5, 100, &mut id), 0);
        assert_eq!((chest.slots[0].count, chest.slots[1].count), (64, 36));
        assert_eq!(chest.count(5), 100);
        assert!(chest.contains(5, 100) && !chest.contains(5, 101));
        assert_eq!(chest.remove_item(5, 70), 0);
        assert_eq!(chest.count(5), 30);
        assert_eq!(chest.remove_item(5, 40), 10);
        assert!(chest.is_empty());
        assert_eq!(chest.first_empty(), Some(0));

        let mut furnace = FurnaceInventory::new();
        assert!(furnace.set_item(1, ItemStack::new(7, 3, 1)));
        assert_eq!(furnace.fuel().count, 3);
        assert!(!furnace.set_item(3, ItemStack::new(7, 1, 2)));
        assert_eq!(furnace.contents(), vec![(1, ItemStack::new(7, 3, 1))]);

        let armor: &dyn Inventory = &ArmorInventory::new();
        assert_eq!(armor.size(), 4);

        // Un inventaire peut avoir ses règles : la grille n'accepte que ses propres cases
        let mut grid = CraftingGrid::new();
        assert!(!grid.set_item(3, ItemStack::new(1, 1, 1)));
        assert!(grid.set_item(29, ItemStack::new(1, 1, 1)));
        assert_eq!(grid.add_item(1, 1, &mut id), 0);
        assert_eq!(grid.slots[29].count, 2);
        // … et ces règles valent aussi pour les demandes du jeu
        let mut w = PlayerWindows::new();
        w.add(1, 4);
        assert!(w.apply(Action::Move { count: 1, from: (Target::Player, 0), to: (Target::Craft, 3) }).is_none());
        assert!(w.apply(Action::Move { count: 1, from: (Target::Player, 0), to: (Target::Craft, 30) }).is_some());
        assert_eq!((w.main.slots[0].count, w.craft.slots[30].count), (3, 1));
    }
}
