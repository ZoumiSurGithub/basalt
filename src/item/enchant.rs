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
pub const BLAST_PROTECTION: u8 = 3;
pub const PROJECTILE_PROTECTION: u8 = 4;
pub const THORNS: u8 = 5;
pub const RESPIRATION: u8 = 6;
pub const SHARPNESS: u8 = 9;
pub const SMITE: u8 = 10;
pub const BANE_OF_ARTHROPODS: u8 = 11;
pub const KNOCKBACK: u8 = 12;
pub const FIRE_ASPECT: u8 = 13;
pub const LOOTING: u8 = 14;
pub const SILK_TOUCH: u8 = 16;
pub const UNBREAKING: u8 = 17;
pub const FORTUNE: u8 = 18;
pub const POWER: u8 = 19;
pub const PUNCH: u8 = 20;
pub const FLAME: u8 = 21;
pub const INFINITY: u8 = 22;
pub const MULTISHOT: u8 = 33;
pub const QUICK_CHARGE: u8 = 35;
pub const IMPALING: u8 = 29;
pub const RIPTIDE: u8 = 30;
pub const LOYALTY: u8 = 31;
pub const CHANNELING: u8 = 32;
pub const LUCK_OF_THE_SEA: u8 = 23;
pub const LURE: u8 = 24;
pub const MENDING: u8 = 26;

/// Tous les enchantements : (numéro, niveau maximal).
pub fn all() -> Vec<(u8, u8)> {
    ENCHANTS.iter().map(|e| (e.0, e.2)).collect()
}

pub const DEPTH_STRIDER: u8 = 7;
pub const FROST_WALKER: u8 = 25;

/// Cet enchantement va-t-il sur cet objet (nom sans `minecraft:`) ? Les règles du jeu :
/// chaque enchantement a ses objets. Un livre enchanté les accepte tous.
pub fn fits(id: u8, short: &str) -> bool {
    let ends = |tails: &[&str]| tails.iter().any(|t| short.ends_with(t));
    let (helmet, chest, legs, boots) = (ends(&["_helmet"]), ends(&["_chestplate"]), ends(&["_leggings"]), ends(&["_boots"]));
    let armor = helmet || chest || legs || boots;
    let (sword, axe, pickaxe, shovel, hoe) = (ends(&["_sword"]), ends(&["_axe"]), ends(&["_pickaxe"]), ends(&["_shovel"]), ends(&["_hoe"]));
    let digger = pickaxe || shovel || axe || hoe;
    let (bow, crossbow, trident, rod, shears) = (short == "bow", short == "crossbow", short == "trident", short == "fishing_rod", short == "shears");
    let durable = armor || sword || digger || bow || crossbow || trident || rod || shears || matches!(short, "flint_and_steel" | "shield" | "elytra" | "carrot_on_a_stick" | "warped_fungus_on_a_stick" | "brush" | "mace");
    if short == "enchanted_book" {
        return true;
    }
    match id {
        0 | 1 | 3 | 4 | 5 => armor,
        2 | 7 | 25 | 36 => boots,
        6 | 8 => helmet,
        37 => legs,
        9..=11 => sword || axe,
        12..=14 => sword,
        15 | 16 => digger || shears,
        18 => digger,
        17 | 26 => durable,
        19..=22 => bow,
        23 | 24 => rod,
        27 => armor || matches!(short, "elytra" | "carved_pumpkin") || short.ends_with("_skull") || short.ends_with("_head"),
        28 => durable || matches!(short, "carved_pumpkin" | "compass" | "recovery_compass") || short.ends_with("_skull") || short.ends_with("_head"),
        29..=32 => trident,
        33..=35 => crossbow,
        _ => false,
    }
}

/// Deux enchantements que le jeu refuse sur un même objet.
pub fn conflict(a: u8, b: u8) -> bool {
    const GROUPS: [&[u8]; 3] = [&[0, 1, 3, 4], &[9, 10, 11], &[16, 18]];
    const PAIRS: [(u8, u8); 5] = [(7, 25), (22, 26), (30, 31), (30, 32), (33, 34)];
    a != b && (GROUPS.iter().any(|g| g.contains(&a) && g.contains(&b)) || PAIRS.iter().any(|&(x, y)| (a, b) == (x, y) || (a, b) == (y, x)))
}

/// Ce que le toucher de soie change au butin d'un bloc.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Silk {
    /// Rien : le butin habituel
    No,
    /// Le bloc lui-même, à la place de son butin, si l'outil sait le récolter (minerai, pierre)
    Instead,
    /// Le bloc lui-même, alors qu'il ne rend rien d'habitude (verre, glace, feuilles)
    Always,
}

/// Le toucher de soie sur ce bloc (nom sans `minecraft:`).
pub fn silk_touch(short: &str) -> Silk {
    const INSTEAD: [&str; 14] = [
        "stone", "grass_block", "mycelium", "podzol", "clay", "snow", "ender_chest", "amethyst_cluster", "gilded_blackstone", "glowstone", "sea_lantern", "bookshelf", "melon_block",
        "sculk",
    ];
    const ALWAYS: [&str; 3] = ["ice", "packed_ice", "blue_ice"];
    if ALWAYS.contains(&short) || short.ends_with("glass") || short.ends_with("glass_pane") || short.ends_with("leaves") || short.ends_with("leaves_flowered") {
        Silk::Always
    } else if INSTEAD.contains(&short) || short.ends_with("_ore") || short.starts_with("sculk_") || (short.contains("coral") && !short.starts_with("dead_")) {
        Silk::Instead
    } else {
        Silk::No
    }
}

/// Une chance sur combien : le silex du gravier (1 sur 10 ; fortune : 1 sur 7, 4, puis toujours).
pub fn flint_odds(level: u8) -> u32 {
    [10, 7, 4, 1][level.min(3) as usize]
}

/// Ce que des feuilles cassées à la main lâchent, en chances sur 10 000, selon la fortune :
/// (pousse, bâtons, pomme). La pomme : chêne et chêne noir seulement ; la pousse de jungle
/// est deux fois plus rare.
pub fn leaf_odds(short: &str, level: u8) -> (u32, u32, u32) {
    let l = level.min(3) as usize;
    let sapling = if short.starts_with("jungle") { [250, 278, 313, 417][l] } else { [500, 625, 833, 1000][l] };
    let apple = if short.starts_with("oak") || short.starts_with("dark_oak") { [50, 56, 63, 83][l] } else { 0 };
    (sapling, [200, 222, 250, 333][l], apple)
}

/// La pousse que lâchent ces feuilles.
pub fn sapling_of(short: &str) -> Option<String> {
    match short {
        "azalea_leaves" => Some("azalea".into()),
        "azalea_leaves_flowered" => Some("flowering_azalea".into()),
        "mangrove_leaves" => None,
        _ => short.strip_suffix("_leaves").map(|wood| format!("{}_sapling", wood)),
    }
}

/// La quantité que rend un minerai avec la fortune. `roll` : un nombre au hasard.
/// Comme dans le jeu : une chance sur (niveau + 2) par multiplicateur de 2 à niveau + 1, le
/// reste sans bonus ; la redstone, elle, gagne de 0 à niveau unités. Le gravier, les feuilles
/// et les cultures ont leurs propres règles (`flint_odds`, `leaf_odds`, la récolte).
pub fn fortune(short: &str, count: u8, level: u8, roll: u32) -> u8 {
    // (melon, pierre lumineuse, lanterne marine : de 0 à niveau de plus, avec un plafond)
    let cap = match short {
        "melon_block" => 9,
        "glowstone" => 4,
        "sea_lantern" => 5,
        _ => 0,
    };
    if level > 0 && cap > 0 {
        return (count as u32 + roll % (level as u32 + 1)).min(cap) as u8;
    }
    if level == 0 || !short.ends_with("_ore") {
        return count;
    }
    let level = level as u32;
    let total = if short.contains("redstone") { count as u32 + roll % (level + 1) } else { count as u32 * (roll % (level + 2)).max(1) };
    total.min(64) as u8
}

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
        // Fortune : jamais moins que sans, au plus (niveau + 1) fois ; rien hors des minerais
        assert_eq!((fortune("diamond_ore", 1, 3, 0), fortune("diamond_ore", 1, 3, 1), fortune("diamond_ore", 1, 3, 2), fortune("deepslate_diamond_ore", 1, 3, 4)), (1, 1, 2, 4));
        assert_eq!((fortune("redstone_ore", 4, 3, 3), fortune("stone", 1, 3, 4), fortune("coal_ore", 1, 0, 4), fortune("lapis_ore", 60, 3, 4)), (7, 1, 1, 64));
        assert_eq!((fortune("melon_block", 5, 3, 3), fortune("melon_block", 7, 3, 3), fortune("glowstone", 3, 3, 3), fortune("sea_lantern", 3, 2, 2), fortune("glowstone", 3, 0, 3)), (8, 9, 4, 5, 3));
        // Toucher de soie : le minerai à la place de son butin, le verre alors qu'il ne rend rien
        assert_eq!((silk_touch("diamond_ore"), silk_touch("glass"), silk_touch("oak_leaves"), silk_touch("dirt"), silk_touch("ice")), (Silk::Instead, Silk::Always, Silk::Always, Silk::No, Silk::Always));
        // Les règles du jeu : chaque enchantement a ses objets, et certains s'excluent
        assert!(fits(SHARPNESS, "diamond_sword") && fits(SHARPNESS, "iron_axe") && !fits(SHARPNESS, "diamond_pickaxe") && !fits(SHARPNESS, "stick"));
        assert!(fits(FORTUNE, "iron_pickaxe") && !fits(FORTUNE, "shears") && fits(SILK_TOUCH, "shears") && fits(POWER, "bow") && !fits(POWER, "crossbow"));
        assert!(fits(PROTECTION, "leather_boots") && fits(FEATHER_FALLING, "iron_boots") && !fits(FEATHER_FALLING, "iron_helmet") && fits(RESPIRATION, "turtle_helmet"));
        assert!(fits(UNBREAKING, "shield") && fits(MENDING, "elytra") && !fits(UNBREAKING, "diamond") && fits(KNOCKBACK, "enchanted_book"));
        assert!(ENCHANTS.iter().all(|e| fits(e.0, "enchanted_book")));
        assert!(conflict(SHARPNESS, SMITE) && conflict(FORTUNE, SILK_TOUCH) && conflict(PROTECTION, BLAST_PROTECTION) && conflict(MENDING, INFINITY) && conflict(FROST_WALKER, DEPTH_STRIDER));
        assert!(!conflict(SHARPNESS, SHARPNESS) && !conflict(SHARPNESS, KNOCKBACK) && !conflict(UNBREAKING, MENDING) && !conflict(PROTECTION, THORNS));
        // Gravier et feuilles : la fortune améliore les chances
        assert_eq!((flint_odds(0), flint_odds(3), flint_odds(9)), (10, 1, 1));
        assert_eq!((leaf_odds("oak_leaves", 0), leaf_odds("jungle_leaves", 3), leaf_odds("birch_leaves", 1).2), ((500, 200, 50), (417, 333, 0), 0));
        assert_eq!((sapling_of("oak_leaves").as_deref(), sapling_of("azalea_leaves_flowered").as_deref(), sapling_of("mangrove_leaves"), sapling_of("stone")), (Some("oak_sapling"), Some("flowering_azalea"), None, None));
        // L'arbalète chargée : un bit à part, que les enchantements ne dérangent pas
        let loaded = Ench::default().with(UNBREAKING, 2).with_charged(true);
        assert!(loaded.charged() && loaded.level(UNBREAKING) == 2 && loaded.with(MULTISHOT, 1).charged() && !loaded.with_charged(false).charged());
        assert!(Ench::default().with_charged(true).is_empty() && !Ench::default().charged());
        let wire = crate::bedrock::item_extra(0, Ench::default().with_charged(true).0);
        assert!(wire.windows(11).any(|w| w == b"chargedItem") && wire.windows(15).any(|w| w == b"minecraft:arrow") && !wire.windows(4).any(|w| w == b"ench"));
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
