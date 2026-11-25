//! Les enchantements du jeu : leur identifiant, leur nom et leur niveau maximal.

/// (identifiant du jeu, nom, niveau maximal)
pub const ENCHANTS: [(u8, &str, u8); 38] = [
    (0, "protection", 4), (1, "fire_protection", 4), (2, "feather_falling", 4), (3, "blast_protection", 4), (4, "projectile_protection", 4),
    (5, "thorns", 3), (6, "respiration", 3), (7, "depth_strider", 3), (8, "aqua_affinity", 1), (9, "sharpness", 5),
    (10, "smite", 5), (11, "bane_of_arthropods", 5), (12, "knockback", 2), (13, "fire_aspect", 2), (14, "looting", 3),
    (15, "efficiency", 5), (16, "silk_touch", 1), (17, "unbreaking", 3), (18, "fortune", 3), (19, "power", 5),
    (20, "punch", 2), (21, "flame", 1), (22, "infinity", 1), (23, "luck_of_the_sea", 3), (24, "lure", 3),
    (25, "frost_walker", 2), (26, "mending", 1), (27, "binding", 1), (28, "vanishing", 1), (29, "impaling", 5),
    (30, "riptide", 3), (31, "loyalty", 3), (32, "channeling", 1), (33, "multishot", 1), (34, "piercing", 4),
    (35, "quick_charge", 3), (36, "soul_speed", 3), (37, "swift_sneak", 3),
];

pub const PROTECTION: u8 = 0;
pub const FIRE_PROTECTION: u8 = 1;
pub const FEATHER_FALLING: u8 = 2;
pub const RESPIRATION: u8 = 6;
pub const SHARPNESS: u8 = 9;
pub const FIRE_ASPECT: u8 = 13;
pub const UNBREAKING: u8 = 17;
pub const POWER: u8 = 19;
pub const INFINITY: u8 = 22;

/// Un enchantement d'après son nom (« sharpness », « minecraft:sharpness ») ou son numéro :
/// (identifiant, niveau maximal).
pub fn enchant_of(word: &str) -> Option<(u8, u8)> {
    let w = word.trim().trim_start_matches("minecraft:").to_ascii_lowercase();
    ENCHANTS.iter().find(|e| e.1 == w || w.parse::<u8>().ok() == Some(e.0)).map(|e| (e.0, e.2))
}

/// Le nom d'un enchantement.
pub fn enchant_name(id: u8) -> &'static str {
    ENCHANTS.iter().find(|e| e.0 == id).map(|e| e.1).unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Ench;

    #[test]
    fn enchantments_pack_into_a_stack() {
        let e = Ench::default().with(SHARPNESS, 5).with(UNBREAKING, 3).with(PROTECTION, 4);
        assert_eq!((e.level(SHARPNESS), e.level(UNBREAKING), e.level(PROTECTION), e.level(POWER)), (5, 3, 4, 0));
        // Remplacer un niveau, en retirer un
        let e = e.with(SHARPNESS, 2).with(UNBREAKING, 0);
        assert_eq!(e.list().len(), 2);
        assert_eq!((e.level(SHARPNESS), e.level(UNBREAKING)), (2, 0));
        // Cinq au plus : le sixième n'entre pas
        let full = (0..6).fold(Ench::default(), |e, id| e.with(id, 1));
        assert_eq!(full.list().len(), 5);
        assert!(Ench::default().is_empty() && Ench::default().with(37, 3).level(37) == 3);
        assert_eq!(enchant_of("minecraft:Sharpness"), Some((9, 5)));
        assert_eq!((enchant_of("17"), enchant_of("rien")), (Some((17, 3)), None));
        // Sur le réseau : la liste « ench » de l'objet, un identifiant et un niveau par entrée
        let wire = crate::bedrock::item_extra(0, Ench::default().with(SHARPNESS, 5).0);
        assert_eq!(&wire[..6], &[0xff, 0xff, 0x01, 0x0a, 0x00, 0x00]);
        assert_eq!(&wire[6..18], &[0x09, 4, 0, b'e', b'n', b'c', b'h', 0x0a, 1, 0, 0, 0]);
        assert_eq!(&wire[18..32], &[0x02, 2, 0, b'i', b'd', 9, 0, 0x02, 3, 0, b'l', b'v', b'l', 5]);
        assert!(crate::bedrock::item_extra(0, 0).is_empty());
        // Avec de l'usure, les deux sont là
        let both = crate::bedrock::item_extra(12, Ench::default().with(UNBREAKING, 3).0);
        assert!(both.windows(6).any(|w| w == b"Damage") && both.windows(4).any(|w| w == b"ench"));
    }
}
