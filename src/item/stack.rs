//! `ItemStack` : une pile d'objets telle qu'elle existe dans un inventaire.

/// Quantité maximale d'une pile ordinaire.
pub const MAX_STACK: u8 = 64;

/// Les enchantements d'un objet : cinq au plus, rangés dans un seul nombre pour que la pile
/// reste une valeur simple à copier (12 bits chacun : 7 pour l'enchantement, 5 pour le niveau).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Ench(pub u64);

impl Ench {
    /// Enchantements portés au plus par un objet
    pub const MAX: usize = 5;
    /// Bit libre au-dessus des cinq enchantements : l'arbalète est chargée d'une flèche.
    pub const CHARGED: u64 = 1 << 63;

    /// L'arbalète est-elle chargée ?
    pub fn charged(self) -> bool {
        self.0 & Self::CHARGED != 0
    }

    /// Le même objet, chargé ou déchargé.
    pub fn with_charged(self, charged: bool) -> Ench {
        Ench(if charged { self.0 | Self::CHARGED } else { self.0 & !Self::CHARGED })
    }

    /// Les enchantements présents : (identifiant du jeu, niveau).
    pub fn list(self) -> Vec<(u8, u8)> {
        (0..Self::MAX).map(|i| (self.0 >> (i * 12)) & 0xfff).filter(|e| *e != 0).map(|e| ((e >> 5) as u8 - 1, (e & 31) as u8)).collect()
    }

    /// Niveau d'un enchantement (0 : absent).
    pub fn level(self, id: u8) -> u8 {
        self.list().into_iter().find(|e| e.0 == id).map(|e| e.1).unwrap_or(0)
    }

    /// Le même objet avec cet enchantement à ce niveau (0 le retire). Inchangé si l'objet en
    /// porte déjà cinq autres.
    pub fn with(self, id: u8, level: u8) -> Ench {
        let mut list: Vec<(u8, u8)> = self.list().into_iter().filter(|e| e.0 != id).collect();
        if level > 0 && list.len() < Self::MAX && id < 126 {
            list.push((id, level.min(31)));
        }
        Ench(list.iter().enumerate().fold(self.0 & Self::CHARGED, |all, (i, (id, level))| all | ((((*id as u64 + 1) << 5) | *level as u64) << (i * 12))))
    }

    pub fn is_empty(self) -> bool {
        self.0 & !Self::CHARGED == 0
    }
}

/// Une pile d'objets. `count == 0` signifie « emplacement vide ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemStack {
    /// Identifiant numérique de l'objet (voir `data::GameData` pour le nom)
    pub id: i16,
    pub count: u8,
    /// Numéro attribué par le serveur à cette pile, que le jeu cite dans ses demandes
    pub net_id: i32,
    /// Variante de l'objet (la couleur d'un lit, par exemple)
    pub meta: u16,
    /// Usure d'un outil : nombre d'utilisations déjà faites
    pub damage: u16,
    /// Enchantements
    pub ench: Ench,
}

impl ItemStack {
    pub const EMPTY: ItemStack = ItemStack { id: 0, count: 0, net_id: 0, meta: 0, damage: 0, ench: Ench(0) };

    /// Pile d'objets ; `net_id` l'identifie auprès du client.
    pub fn new(id: i16, count: u8, net_id: i32) -> ItemStack {
        ItemStack { id, count, net_id, meta: 0, damage: 0, ench: Ench(0) }
    }

    /// Même objet (type, variante, usure) : deux piles qui peuvent se réunir.
    pub fn same_kind(&self, other: &ItemStack) -> bool {
        self.id == other.id && self.meta == other.meta && self.damage == other.damage && self.ench == other.ench
    }

    /// Case vide ?
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

}
