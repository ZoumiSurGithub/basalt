//! `Inventory` : la base commune à tous les inventaires (sac du joueur, armure, coffre,
//! four…). Un inventaire concret fournit ses emplacements et, s'il en a, ses règles ;
//! tout le reste (lire, écrire, ajouter, retirer, compter) est hérité d'ici.

use crate::item::{ItemStack, MAX_STACK};

pub trait Inventory {
    /// Nom lisible, pour les journaux et les sauvegardes.
    fn name(&self) -> &'static str;

    /// Emplacements, dans l'ordre.
    fn slots(&self) -> &[ItemStack];
    /// Cases de l'inventaire, à lire et à modifier.
    fn slots_mut(&mut self) -> &mut [ItemStack];

    /// Règle propre à l'inventaire : cet objet peut-il aller à cet emplacement ?
    /// (par défaut tout est permis ; un four refuserait un non-combustible, par exemple)
    fn accepts(&self, _slot: usize, _item: &ItemStack) -> bool {
        true
    }

    /// Taille maximale d'une pile dans cet inventaire.
    fn max_stack(&self) -> u8 {
        MAX_STACK
    }

    // ---- hérité par tous les inventaires ----------------------------------------

    /// Nombre de cases.
    fn size(&self) -> usize {
        self.slots().len()
    }

    /// Objet à un emplacement ; `None` si l'emplacement n'existe pas.
    fn get_item(&self, slot: usize) -> Option<ItemStack> {
        self.slots().get(slot).copied()
    }

    /// Place un objet. Faux si l'emplacement n'existe pas ou si l'inventaire le refuse.
    fn set_item(&mut self, slot: usize, item: ItemStack) -> bool {
        if slot >= self.size() || (!item.is_empty() && !self.accepts(slot, &item)) {
            return false;
        }
        self.slots_mut()[slot] = if item.count == 0 { ItemStack::EMPTY } else { item };
        true
    }

    /// Vide une case, ou toutes.
    fn clear(&mut self, slot: usize) -> bool {
        self.set_item(slot, ItemStack::EMPTY)
    }

    fn clear_all(&mut self) {
        for s in self.slots_mut() {
            *s = ItemStack::EMPTY;
        }
    }

    /// Emplacements occupés : (numéro, objet).
    fn contents(&self) -> Vec<(usize, ItemStack)> {
        self.slots().iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| (i, *s)).collect()
    }

    /// Première case vide.
    fn first_empty(&self) -> Option<usize> {
        self.slots().iter().position(|s| s.is_empty())
    }

    /// Rien dans l'inventaire ?
    fn is_empty(&self) -> bool {
        self.slots().iter().all(|s| s.is_empty())
    }

    /// Nombre total d'exemplaires d'un objet.
    fn count(&self, id: i16) -> u32 {
        self.slots().iter().filter(|s| !s.is_empty() && s.id == id).map(|s| s.count as u32).sum()
    }

    /// Au moins `count` objets de ce type ?
    fn contains(&self, id: i16, count: u32) -> bool {
        self.count(id) >= count
    }

    /// Ajoute des objets : complète d'abord les piles existantes, puis occupe les emplacements
    /// libres. `next_net_id` fournit un numéro pour chaque nouvelle pile. Renvoie ce qui n'a
    /// pas pu entrer.
    fn add_item(&mut self, id: i16, mut count: u32, next_net_id: &mut dyn FnMut() -> i32) -> u32 {
        let max = self.max_stack();
        for i in 0..self.size() {
            if count == 0 {
                return 0;
            }
            let s = self.slots()[i];
            if !s.is_empty() && s.id == id && s.count < max {
                let room = (max - s.count) as u32;
                let moved = room.min(count);
                self.slots_mut()[i].count += moved as u8;
                count -= moved;
            }
        }
        for i in 0..self.size() {
            if count == 0 {
                return 0;
            }
            let fresh = ItemStack { id, count: count.min(max as u32) as u8, net_id: 0 };
            if self.slots()[i].is_empty() && self.accepts(i, &fresh) {
                self.slots_mut()[i] = ItemStack { net_id: next_net_id(), ..fresh };
                count -= fresh.count as u32;
            }
        }
        count
    }

    /// Retire des exemplaires d'un objet, où qu'ils soient. Renvoie ce qui manquait.
    fn remove_item(&mut self, id: i16, mut count: u32) -> u32 {
        for s in self.slots_mut().iter_mut().rev() {
            if count == 0 {
                break;
            }
            if !s.is_empty() && s.id == id {
                let taken = (s.count as u32).min(count);
                s.count -= taken as u8;
                count -= taken;
                if s.count == 0 {
                    *s = ItemStack::EMPTY;
                }
            }
        }
        count
    }
}
