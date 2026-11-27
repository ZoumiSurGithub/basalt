//! `Tile` : la base des données attachées à un bloc.

use crate::block::TileType;
use crate::inventory::{BaseInventory, Inventory};
use crate::nbt::Tag;

/// Ce dont une tile a besoin du serveur : le catalogue des objets et des numéros de pile.
pub trait TileContext {
    /// Ce dont un bloc à données a besoin du serveur : noms et identifiants des objets.
    fn name_of(&self, id: i16) -> Option<String>;
    fn id_of(&self, name: &str) -> Option<i16>;
    fn next_net_id(&mut self) -> i32;
}

pub trait Tile {
    /// Sorte de données (coffre, four, panneau…).
    fn tile_type(&self) -> TileType;

    /// Identifiant de ces données dans le jeu (« Chest », « Furnace », « Sign »…).
    fn network_id(&self) -> &'static str;

    // ---- hérité par toutes les tiles, redéfini au besoin ------------------------

    /// Inventaire porté par le bloc, s'il en a un.
    fn inventory(&self) -> Option<&dyn Inventory> {
        None
    }

    /// Version modifiable de `inventory`.
    fn inventory_mut(&mut self) -> Option<&mut dyn Inventory> {
        None
    }

    /// Appelé vingt fois par seconde. Renvoie vrai si quelque chose a changé.
    fn tick(&mut self, _ctx: &mut dyn TileContext) -> bool {
        false
    }

    /// Jauges à afficher sur l'écran du bloc : (cuisson, cuisson totale, feu restant, feu total).
    fn gauges(&self) -> Option<(u32, u32, u32, u32)> {
        None
    }

    /// État propre à sauvegarder, en plus de l'inventaire : (clé NBT, valeur).
    fn save_extra(&self) -> Vec<(&'static str, Tag)> {
        Vec::new()
    }

    /// Relit une clé enregistrée par `save_extra`.
    fn load_extra(&mut self, _key: &str, _value: &Tag) {}

    /// L'autre moitié d'un coffre double : ses coordonnées x et z (même hauteur).
    fn pair(&self) -> Option<(i32, i32)> {
        None
    }

    /// Unit ce coffre à un voisin, ou le sépare (None).
    fn set_pair(&mut self, _pair: Option<(i32, i32)>) {}

    /// Coffre posé en étant accroupi : il reste simple, même contre un autre coffre.
    fn alone(&self) -> bool {
        false
    }
    fn set_alone(&mut self, _alone: bool) {}
}

/// Coffre, tonneau, boîte de Shulker, entonnoir, distributeur, dropper, alambic : un
/// inventaire, sans autre état.
pub struct ContainerTile {
    kind: TileType,
    inventory: BaseInventory,
    /// L'autre moitié d'un coffre double (x, z), et ce qui a été relu du fichier
    pair: Option<(i32, i32)>,
    read: (Option<i32>, Option<i32>),
    alone: bool,
}

impl ContainerTile {
    /// Conteneur vide de la bonne taille pour cette sorte de bloc.
    pub fn new(kind: TileType) -> ContainerTile {
        let (label, size) = match kind {
            TileType::Hopper => ("entonnoir", 5),
            TileType::Dispenser => ("distributeur", 9),
            TileType::Dropper => ("dropper", 9),
            TileType::BrewingStand => ("alambic", 5),
            TileType::Barrel => ("tonneau", 27),
            TileType::ShulkerBox => ("boîte de Shulker", 27),
            _ => ("coffre", 27),
        };
        ContainerTile { kind, inventory: BaseInventory::new(label, size), pair: None, read: (None, None), alone: false }
    }
}

impl Tile for ContainerTile {
    /// Les conteneurs simples ne gardent rien d'autre que leur inventaire.
    fn tile_type(&self) -> TileType {
        self.kind
    }
    fn network_id(&self) -> &'static str {
        match self.kind {
            TileType::Hopper => "Hopper",
            TileType::Dispenser => "Dispenser",
            TileType::Dropper => "Dropper",
            TileType::BrewingStand => "BrewingStand",
            TileType::Barrel => "Barrel",
            TileType::ShulkerBox => "ShulkerBox",
            _ => "Chest",
        }
    }
    fn inventory(&self) -> Option<&dyn Inventory> {
        Some(&self.inventory)
    }
    fn inventory_mut(&mut self) -> Option<&mut dyn Inventory> {
        Some(&mut self.inventory)
    }
    fn pair(&self) -> Option<(i32, i32)> {
        self.pair
    }
    fn set_pair(&mut self, pair: Option<(i32, i32)>) {
        self.pair = pair;
    }
    fn alone(&self) -> bool {
        self.alone
    }
    fn set_alone(&mut self, alone: bool) {
        self.alone = alone;
    }
    fn save_extra(&self) -> Vec<(&'static str, Tag)> {
        match self.pair {
            Some((x, z)) => vec![("pairx", Tag::Int(x)), ("pairz", Tag::Int(z))],
            None if self.alone => vec![("alone", Tag::Byte(1))],
            None => Vec::new(),
        }
    }
    fn load_extra(&mut self, key: &str, value: &Tag) {
        if key == "alone" {
            self.alone = matches!(value, Tag::Byte(1));
        }
        if let Tag::Int(v) = value {
            match key {
                "pairx" => self.read.0 = Some(*v),
                "pairz" => self.read.1 = Some(*v),
                _ => {}
            }
            if let (Some(x), Some(z)) = self.read {
                self.pair = Some((x, z));
            }
        }
    }
}

/// Panneau : son texte.
#[derive(Default)]
pub struct SignTile {
    text: String,
}

impl SignTile {
    /// Panneau vide.
    pub fn new() -> SignTile {
        SignTile::default()
    }

    /// Texte du panneau.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Quatre lignes au plus, sans caractères de contrôle.
    pub fn set_text(&mut self, text: &str) {
        let lines: Vec<String> = text.lines().take(4).map(|l| l.chars().filter(|c| !c.is_control()).take(64).collect()).collect();
        self.text = lines.join("\n");
    }
}

impl Tile for SignTile {
    /// Un panneau ne garde que son texte.
    fn tile_type(&self) -> TileType {
        TileType::Sign
    }
    fn network_id(&self) -> &'static str {
        "Sign"
    }
    fn save_extra(&self) -> Vec<(&'static str, Tag)> {
        vec![("Text", Tag::string(&self.text))]
    }
    fn load_extra(&mut self, key: &str, value: &Tag) {
        if let ("Text", Some(text)) = (key, value.as_str()) {
            self.set_text(text);
        }
    }
}

/// Lit : sa couleur (0 blanc … 14 rouge, 15 noir), celle de l'objet qui l'a posé.
#[derive(Default)]
pub struct BedTile {
    pub color: u8,
}

impl Tile for BedTile {
    fn tile_type(&self) -> TileType {
        TileType::Bed
    }
    fn network_id(&self) -> &'static str {
        "Bed"
    }
    fn save_extra(&self) -> Vec<(&'static str, Tag)> {
        vec![("color", Tag::Byte(self.color as i8))]
    }
    fn load_extra(&mut self, key: &str, value: &Tag) {
        if let ("color", Some(c)) = (key, value.as_i64()) {
            self.color = c.clamp(0, 15) as u8;
        }
    }
}

/// Bloc qui a des données propres que le serveur ne gère pas encore en détail (lit,
/// bannière, tête, balise…) : sa présence est notée, rien de plus.
pub struct GenericTile {
    kind: TileType,
}

impl Tile for GenericTile {
    /// Coffre (ou coffre de l'Ender, qui a un autre nom dans le jeu).
    fn tile_type(&self) -> TileType {
        self.kind
    }
    fn network_id(&self) -> &'static str {
        if self.kind == TileType::EnderChest {
            "EnderChest"
        } else {
            "Unknown"
        }
    }
}

/// Crée les données d'un bloc qui vient d'être posé. `None` pour les blocs dont le serveur
/// n'a rien à retenir.
pub fn create(kind: TileType) -> Option<Box<dyn Tile>> {
    Some(match kind {
        TileType::Chest | TileType::Barrel | TileType::ShulkerBox | TileType::Hopper | TileType::Dispenser | TileType::Dropper | TileType::BrewingStand => {
            Box::new(ContainerTile::new(kind))
        }
        TileType::Furnace | TileType::BlastFurnace | TileType::Smoker => Box::new(super::furnace::FurnaceTile::new(kind)),
        TileType::Sign => Box::new(SignTile::new()),
        TileType::Bed => Box::new(BedTile::default()),
        TileType::EnderChest => Box::new(GenericTile { kind }),
        TileType::Other => return None,
    })
}

/// Nom d'un type dans le fichier de sauvegarde, et l'inverse.
pub(super) fn type_name(kind: TileType) -> &'static str {
    match kind {
        TileType::Chest => "Chest",
        TileType::Barrel => "Barrel",
        TileType::ShulkerBox => "ShulkerBox",
        TileType::EnderChest => "EnderChest",
        TileType::Furnace => "Furnace",
        TileType::BlastFurnace => "BlastFurnace",
        TileType::Smoker => "Smoker",
        TileType::Hopper => "Hopper",
        TileType::Dispenser => "Dispenser",
        TileType::Dropper => "Dropper",
        TileType::BrewingStand => "BrewingStand",
        TileType::Sign => "Sign",
        TileType::Bed => "Bed",
        TileType::Other => "Other",
    }
}

/// Sorte de données d'après leur nom dans le jeu (« Chest », « Furnace »…).
pub(super) fn type_from(name: &str) -> Option<TileType> {
    const ALL: [TileType; 13] = [
        TileType::Bed,
        TileType::Chest, TileType::Barrel, TileType::ShulkerBox, TileType::EnderChest, TileType::Furnace, TileType::BlastFurnace, TileType::Smoker,
        TileType::Hopper, TileType::Dispenser, TileType::Dropper, TileType::BrewingStand, TileType::Sign,
    ];
    ALL.iter().copied().find(|k| type_name(*k) == name)
}
