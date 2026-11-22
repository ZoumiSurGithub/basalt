//! Demandes du jeu sur les inventaires : quel emplacement est visé, et quelles actions
//! (déplacer, échanger, jeter) sont permises. Les règles sont vérifiées ici, côté serveur.

/// Inventaire visé par une demande du jeu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Player,
    Cursor,
    Armor,
    Offhand,
    /// Inventaire du bloc ouvert par le joueur (coffre, four…)
    Block,
    /// Résultat d'une création (menu créatif, fabrication) en attente d'être pris
    Created,
    /// Grille de fabrication
    Craft,
}

/// Emplacement désigné par le jeu : (nom de conteneur du protocole, numéro).
/// Renvoie l'inventaire du serveur et l'indice dans celui-ci.
pub fn resolve(container_name: u8, slot: u8) -> Option<(Target, usize)> {
    let slot = slot as usize;
    match container_name {
        // barre d'objets, sac, ou les deux réunis : même inventaire côté serveur
        28 if slot < 9 => Some((Target::Player, slot)),
        29 if (9..36).contains(&slot) => Some((Target::Player, slot)),
        12 if slot < 36 => Some((Target::Player, slot)),
        59 if slot == 0 => Some((Target::Cursor, 0)),
        6 if slot < 4 => Some((Target::Armor, slot)),
        34 => Some((Target::Offhand, 0)),
        7 | 30 | 58 => Some((Target::Block, slot)),
        // Selle et armure (ou tapis) d'une monture : mêmes numéros que son inventaire
        27 if slot < 2 => Some((Target::Block, slot)),
        // four : ingrédient, combustible, résultat
        25 => Some((Target::Block, 0)),
        24 => Some((Target::Block, 1)),
        26 => Some((Target::Block, 2)),
        60 => Some((Target::Created, 0)),
        13 if (28..=40).contains(&slot) => Some((Target::Craft, slot)),
        // Échange avec un villageois : les deux lots demandés (cases 4 et 5 de l'écran)
        47 if slot == 4 => Some((Target::Craft, 4)),
        48 if slot == 5 => Some((Target::Craft, 5)),
        _ => None,
    }
}

/// Nom de conteneur à annoncer au jeu pour un emplacement du serveur.
pub fn protocol_name(target: Target, slot: usize) -> u8 {
    match target {
        Target::Player if slot < 9 => 28,
        Target::Player => 29,
        Target::Cursor => 59,
        Target::Armor => 6,
        Target::Offhand => 34,
        Target::Block => 7,
        Target::Created => 60,
        Target::Craft if slot == 4 => 47,
        Target::Craft if slot == 5 => 48,
        Target::Craft => 13,
    }
}

/// Une action demandée par le jeu sur les inventaires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Déplace `count` objets d'un emplacement vers un autre (prendre ou déposer)
    Move { count: u8, from: (Target, usize), to: (Target, usize) },
    Swap { a: (Target, usize), b: (Target, usize) },
    /// Jette ou détruit `count` objets
    Remove { count: u8, from: (Target, usize) },
}
