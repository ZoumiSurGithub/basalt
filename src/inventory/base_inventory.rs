//! `BaseInventory` : un inventaire générique, simple rangée d'emplacements. Les inventaires
//! concrets (joueur, armure, coffre…) sont construits dessus.

use super::inventory::Inventory;
use crate::item::ItemStack;

#[derive(Debug, Clone, PartialEq)]
pub struct BaseInventory {
    name: &'static str,
    pub slots: Vec<ItemStack>,
}

impl BaseInventory {
    pub fn new(name: &'static str, size: usize) -> BaseInventory {
        BaseInventory { name, slots: vec![ItemStack::EMPTY; size] }
    }
}

impl Inventory for BaseInventory {
    fn name(&self) -> &'static str {
        self.name
    }
    fn slots(&self) -> &[ItemStack] {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut [ItemStack] {
        &mut self.slots
    }
}

/// Déclare un inventaire concret bâti sur `BaseInventory` : il en hérite les emplacements
/// (`inv.slots`) et toutes les opérations d'`Inventory`.
macro_rules! inventory_type {
    ($(#[$doc:meta])* $name:ident, $label:literal, $size:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            base: $crate::inventory::BaseInventory,
        }

        impl $name {
            pub const SIZE: usize = $size;

            pub fn new() -> $name {
                $name { base: $crate::inventory::BaseInventory::new($label, $size) }
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $name::new()
            }
        }

        impl std::ops::Deref for $name {
            type Target = $crate::inventory::BaseInventory;
            fn deref(&self) -> &Self::Target {
                &self.base
            }
        }

        impl std::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.base
            }
        }
    };
}
pub(crate) use inventory_type;

/// Branche un inventaire concret sur `Inventory` sans règle particulière.
macro_rules! plain_inventory {
    ($name:ident) => {
        impl $crate::inventory::Inventory for $name {
            fn name(&self) -> &'static str {
                $crate::inventory::Inventory::name(&self.base)
            }
            fn slots(&self) -> &[$crate::item::ItemStack] {
                &self.base.slots
            }
            fn slots_mut(&mut self) -> &mut [$crate::item::ItemStack] {
                &mut self.base.slots
            }
        }
    };
}
pub(crate) use plain_inventory;
