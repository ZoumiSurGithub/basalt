//! Gabarit « durable » : un objet qui s'use et finit par casser (l'équivalent de `Durable`
//! dans PocketMine, dont héritent les outils et les armures).
//!
//! Deux choses décrivent l'usure d'un objet, toutes deux dans le trait `Item` :
//!
//! - `max_durability()` : le nombre d'utilisations avant de casser (0 : ne s'use pas) ;
//! - `wear(usage)` : ce que coûte un usage. Une pioche s'use d'un cran en minant et de deux
//!   en frappant ; une épée, l'inverse.
//!
//! L'usure elle-même est portée par la pile (`ItemStack::damage`), pas par l'objet : deux
//! pioches en fer sont le même objet, usées différemment. `apply_wear` fait le calcul ; le
//! serveur (`server/survival.rs`) l'appelle quand un bloc casse ou qu'un coup porte, sans
//! rien savoir des outils.
//!
//! Rendre un objet durable : redéfinir `max_durability()` et `wear()` dans son `impl Item`.

use super::item::Item;
use super::stack::ItemStack;

/// Ce qu'on vient de faire avec l'objet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    /// Un bloc a été cassé avec
    Mine,
    /// Un coup a été porté avec
    Strike,
}

/// Use la pile d'un usage. Renvoie vrai si l'objet vient de casser (la pile est alors vide).
/// Sans effet sur un objet qui ne s'use pas.
pub fn apply_wear(stack: &mut ItemStack, item: &dyn Item, usage: Usage) -> bool {
    let max = item.max_durability().min(u16::MAX as u32) as u16;
    let cost = item.wear(usage);
    if stack.is_empty() || max == 0 || cost == 0 {
        return false;
    }
    stack.damage = stack.damage.saturating_add(cost);
    if stack.damage >= max {
        *stack = ItemStack::EMPTY;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::VanillaItems;

    #[test]
    fn tools_wear_out_and_break() {
        let pickaxe = VanillaItems::get("wooden_pickaxe");
        let sword = VanillaItems::get("iron_sword");
        // Une pioche : un cran en minant, deux en frappant ; une épée, l'inverse
        assert_eq!((pickaxe.wear(Usage::Mine), pickaxe.wear(Usage::Strike)), (1, 2));
        assert_eq!((sword.wear(Usage::Mine), sword.wear(Usage::Strike)), (2, 1));
        // 59 utilisations pour une pioche en bois, puis elle casse
        let mut stack = ItemStack::new(1, 1, 0);
        for _ in 0..58 {
            assert!(!apply_wear(&mut stack, pickaxe.as_ref(), Usage::Mine));
        }
        assert_eq!(stack.damage, 58);
        assert!(apply_wear(&mut stack, pickaxe.as_ref(), Usage::Mine));
        assert!(stack.is_empty());
        // Un objet ordinaire ne s'use pas
        let mut bread = ItemStack::new(2, 3, 0);
        assert!(!apply_wear(&mut bread, VanillaItems::get("bread").as_ref(), Usage::Strike));
        assert_eq!((bread.count, bread.damage), (3, 0));
    }
}
