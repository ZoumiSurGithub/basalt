//! `PlayerWindows` : tous les inventaires d'un joueur (sac, curseur, armure, seconde main,
//! grille de fabrication) et l'application des demandes du jeu sur cet ensemble.

use super::containers::{ArmorInventory, CraftingGrid, CraftingResultInventory, PlayerCursorInventory, PlayerOffHandInventory};
use super::base_inventory::BaseInventory;
use super::inventory::Inventory;
use super::player_inventory::{HeldSlot, PlayerInventory};
use super::transaction::{Action, Target};
use crate::item::{ItemStack, MAX_STACK};

#[derive(Debug, Clone)]
pub struct PlayerWindows {
    pub main: PlayerInventory,
    pub cursor: PlayerCursorInventory,
    pub armor: ArmorInventory,
    pub offhand: PlayerOffHandInventory,
    /// Objet créé par la demande en cours (vidé à la fin de chaque demande)
    pub created: CraftingResultInventory,
    /// Grille de fabrication (inventaire ou établi)
    pub craft: CraftingGrid,
    /// Emplacement de la barre tenu en main
    pub selected: u8,
    /// Fenêtre ouverte chez le joueur (0 : aucune)
    pub open_window: u8,
    /// Copie de l'inventaire du bloc ouvert (coffre, four) ; le serveur la recopie depuis et
    /// vers le bloc autour de chaque demande du jeu
    pub block: BaseInventory,
    /// Position du bloc ouvert
    pub block_pos: Option<(i32, i32, i32)>,
    /// Le bloc ouvert est un four (ingrédient, combustible, résultat)
    pub block_is_furnace: bool,
    next_net_id: i32,
}

impl Default for PlayerWindows {
    fn default() -> PlayerWindows {
        PlayerWindows::new()
    }
}

impl PlayerWindows {
    pub fn new() -> PlayerWindows {
        PlayerWindows {
            main: PlayerInventory::new(),
            cursor: PlayerCursorInventory::new(),
            armor: ArmorInventory::new(),
            offhand: PlayerOffHandInventory::new(),
            created: CraftingResultInventory::new(),
            craft: CraftingGrid::new(),
            selected: 0,
            open_window: 0,
            block: BaseInventory::new("bloc", 0),
            block_pos: None,
            block_is_furnace: false,
            next_net_id: 1,
        }
    }

    fn fresh_net_id(&mut self) -> i32 {
        let id = self.next_net_id;
        self.next_net_id = if id >= i32::MAX - 1 { 1 } else { id + 1 };
        id
    }

    /// Inventaire désigné par une demande du jeu.
    pub fn inventory(&self, target: Target) -> Option<&dyn Inventory> {
        match target {
            Target::Player => Some(&self.main),
            Target::Cursor => Some(&self.cursor),
            Target::Armor => Some(&self.armor),
            Target::Offhand => Some(&self.offhand),
            Target::Created => Some(&self.created),
            Target::Craft => Some(&self.craft),
            Target::Block if self.block_pos.is_some() => Some(&self.block),
            Target::Block => None,
        }
    }

    pub fn inventory_mut(&mut self, target: Target) -> Option<&mut dyn Inventory> {
        match target {
            Target::Player => Some(&mut self.main),
            Target::Cursor => Some(&mut self.cursor),
            Target::Armor => Some(&mut self.armor),
            Target::Offhand => Some(&mut self.offhand),
            Target::Created => Some(&mut self.created),
            Target::Craft => Some(&mut self.craft),
            Target::Block if self.block_pos.is_some() => Some(&mut self.block),
            Target::Block => None,
        }
    }

    pub fn get(&self, at: (Target, usize)) -> Option<ItemStack> {
        self.inventory(at.0)?.get_item(at.1)
    }

    fn set(&mut self, at: (Target, usize), stack: ItemStack) -> bool {
        // On ne dépose rien dans la case « résultat » d'un four : on ne peut qu'y prendre
        if at == (Target::Block, 2) && self.block_is_furnace && !stack.is_empty() {
            let current = self.block.slots.get(2).copied().unwrap_or(ItemStack::EMPTY);
            if current.is_empty() || stack.id != current.id || stack.count > current.count {
                return false;
            }
        }
        match self.inventory_mut(at.0) {
            Some(inv) => inv.set_item(at.1, stack),
            None => false,
        }
    }

    /// Ajoute des objets au sac. Renvoie ce qui n'a pas pu entrer.
    pub fn add(&mut self, id: i16, count: u32) -> u32 {
        let mut next = self.next_net_id;
        let left = self.main.add_item(id, count, &mut || {
            let id = next;
            next = if next >= i32::MAX - 1 { 1 } else { next + 1 };
            id
        });
        self.next_net_id = next;
        left
    }

    /// Remet un objet sauvegardé à sa place dans le sac (ou ailleurs dans le sac si la place est prise).
    pub fn restore(&mut self, slot: usize, id: i16, count: u8) {
        let count = count.min(MAX_STACK);
        if count == 0 {
            return;
        }
        if slot < self.main.size() && self.main.slots[slot].is_empty() {
            let net_id = self.fresh_net_id();
            self.main.set_item(slot, ItemStack { id, count, net_id });
        } else {
            self.add(id, count as u32);
        }
    }

    /// Remet un objet sauvegardé dans l'armure ou la seconde main. Si la place est prise (ou
    /// le numéro invalide), l'objet va dans le sac, pour ne jamais être perdu.
    pub fn restore_worn(&mut self, container: &str, slot: usize, id: i16, count: u8) {
        let count = count.min(MAX_STACK);
        if count == 0 {
            return;
        }
        let free = |inv: &dyn Inventory| inv.get_item(slot).map(|s| s.is_empty()).unwrap_or(false);
        let placed = match container {
            "armor" if slot < self.armor.size() && free(&self.armor) => {
                let net_id = self.fresh_net_id();
                self.armor.set_item(slot, ItemStack { id, count, net_id })
            }
            "offhand" if slot < self.offhand.size() && free(&self.offhand) => {
                let net_id = self.fresh_net_id();
                self.offhand.set_item(slot, ItemStack { id, count, net_id })
            }
            _ => false,
        };
        if !placed {
            self.add(id, count as u32);
        }
    }

    /// Place un objet tout neuf dans l'emplacement de création (menu créatif, fabrication).
    pub fn create(&mut self, id: i16, count: u8) {
        let net_id = self.fresh_net_id();
        self.created.set_item(0, ItemStack { id, count: count.min(MAX_STACK), net_id });
    }

    /// Jette ce qui reste dans l'emplacement de création.
    pub fn clear_created(&mut self) {
        self.created.clear_all();
    }

    /// Objet tenu en main.
    pub fn held(&self) -> ItemStack {
        self.main.item_in_hand(HeldSlot(self.selected))
    }

    /// Retire un objet de la pile tenue en main (bloc posé). Faux si la main est vide.
    pub fn consume_held(&mut self) -> bool {
        self.main.consume_in_hand(HeldSlot(self.selected))
    }

    /// Applique une action demandée par le jeu. Renvoie les emplacements modifiés,
    /// ou None si l'action est impossible (rien n'est alors changé).
    pub fn apply(&mut self, action: Action) -> Option<Vec<(Target, usize)>> {
        match action {
            Action::Move { count, from, to } => {
                if count == 0 || from == to {
                    return None;
                }
                let src = self.get(from)?;
                let dst = self.get(to)?;
                if src.count < count {
                    return None;
                }
                let max = self.inventory(to.0)?.max_stack();
                if !dst.is_empty() && (dst.id != src.id || dst.count as u16 + count as u16 > max as u16) {
                    return None;
                }
                let moved_all = src.count == count;
                let new_dst = if dst.is_empty() {
                    // Une pile déplacée en entier garde son numéro ; une pile coupée en reçoit un nouveau
                    let net_id = if moved_all { src.net_id } else { self.fresh_net_id() };
                    ItemStack { id: src.id, count, net_id }
                } else {
                    ItemStack { count: dst.count + count, ..dst }
                };
                // L'inventaire d'arrivée a le dernier mot (règles propres : four, armure…)
                if !self.set(to, new_dst) {
                    return None;
                }
                self.set(from, ItemStack { count: src.count - count, ..src });
                Some(vec![from, to])
            }
            Action::Swap { a, b } => {
                if a == b {
                    return None;
                }
                let (sa, sb) = (self.get(a)?, self.get(b)?);
                if !self.set(a, sb) {
                    return None;
                }
                if !self.set(b, sa) {
                    self.set(a, sa);
                    return None;
                }
                Some(vec![a, b])
            }
            Action::Remove { count, from } => {
                let src = self.get(from)?;
                if count == 0 || src.count < count {
                    return None;
                }
                self.set(from, ItemStack { count: src.count - count, ..src });
                Some(vec![from])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worn_items_come_back_on_the_player() {
        let mut w = PlayerWindows::new();
        w.restore_worn("armor", 1, 300, 1);
        w.restore_worn("offhand", 0, 355, 1);
        assert_eq!(w.armor.get_item(1).map(|s| s.id), Some(300));
        assert_eq!(w.offhand.get_item(0).map(|s| s.id), Some(355));
        // Place déjà prise ou numéro invalide : l'objet va dans le sac, il n'est pas perdu
        w.restore_worn("armor", 1, 301, 1);
        w.restore_worn("armor", 9, 302, 1);
        w.restore_worn("autre", 0, 303, 1);
        let in_bag: Vec<i16> = w.main.slots.iter().filter(|s| !s.is_empty()).map(|s| s.id).collect();
        assert_eq!(in_bag, vec![301, 302, 303]);
    }
}
