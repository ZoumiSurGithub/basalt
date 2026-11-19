//! `ItemStack` : une pile d'objets telle qu'elle existe dans un inventaire.

/// Quantité maximale d'une pile ordinaire.
pub const MAX_STACK: u8 = 64;

/// Une pile d'objets. `count == 0` signifie « emplacement vide ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemStack {
    /// Identifiant numérique de l'objet (voir `data::GameData` pour le nom)
    pub id: i16,
    pub count: u8,
    /// Numéro attribué par le serveur à cette pile, que le jeu cite dans ses demandes
    pub net_id: i32,
}

impl ItemStack {
    pub const EMPTY: ItemStack = ItemStack { id: 0, count: 0, net_id: 0 };

    pub fn new(id: i16, count: u8, net_id: i32) -> ItemStack {
        ItemStack { id, count, net_id }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Même objet (la quantité et le numéro de pile ne comptent pas).
    pub fn same_type(&self, other: &ItemStack) -> bool {
        self.id == other.id
    }
}
