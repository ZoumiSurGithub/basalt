//! Comportements de blocs : ce qu'un bloc *est* aux yeux du monde (l'équivalent des classes
//! `Fallable`, `Flowable`, `Crops`, `Leaves`, `Wood` de PocketMine).
//!
//! Chaque question a sa fonction, et le trait `Block` la reprend en méthode :
//!
//! | Question | Fonction | Méthode de `Block` |
//! |---|---|---|
//! | tombe-t-il sans appui ? (sable, gravier) | `falls` | `falls()` |
//! | lui faut-il un bloc dessous ? (fleur, rail, porte) | `needs_floor` | `needs_floor()` |
//! | est-il en deux moitiés ? (porte, tournesol) | `two_halves` | `two_halves()` |
//! | est-ce un tronc, qui nourrit les feuilles ? | `is_log` | `is_log()` |
//! | est-ce un feuillage, qui dépérit sans tronc ? | `is_leaves` | `is_leaves()` |
//! | est-ce une culture, qui pousse sur terre labourée ? | `is_crop` | `is_crop()` |
//!
//! Le monde qui réagit (`server/nature.rs`) ne contient aucune liste de blocs : il pose ces
//! questions et applique la règle. Donner un comportement à un bloc, c'est donc l'ajouter
//! ici, ou redéfinir la méthode dans son `impl Block` (voir `docs/ARCHITECTURE.md`).
//!
//! Les fonctions prennent un nom, avec ou sans `minecraft:`. `falls` et `needs_floor`
//! regardent d'abord ce qu'un plugin a pu changer (`overrides.rs`).

fn short(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// Tronc ou bois : nourrit les feuilles alentour.
pub fn is_log(name: &str) -> bool {
    name.ends_with("_log") || name.ends_with("_wood")
}

/// Feuillage : dépérit sans tronc à portée.
pub fn is_leaves(name: &str) -> bool {
    name.ends_with("leaves") || name.ends_with("leaves_flowered")
}

/// Culture semée sur terre labourée, qui mûrit par stades.
pub fn is_crop(name: &str) -> bool {
    matches!(short(name), "wheat" | "carrots" | "potatoes" | "beetroot")
}

/// Blocs soumis à la gravité.
pub fn falls(name: &str) -> bool {
    if let Some(answer) = super::overrides::of(name).and_then(|o| o.falls) {
        return answer;
    }
    let n = name.trim_start_matches("minecraft:");
    matches!(n, "sand" | "red_sand" | "gravel" | "anvil" | "chipped_anvil" | "damaged_anvil" | "dragon_egg" | "suspicious_sand" | "suspicious_gravel") || n.ends_with("concrete_powder")
}

/// Blocs posés au sol, qui ne tiennent pas sans bloc dessous.
pub fn needs_floor(name: &str) -> bool {
    if let Some(answer) = super::overrides::of(name).and_then(|o| o.needs_floor) {
        return answer;
    }
    let n = name.trim_start_matches("minecraft:");
    if n.ends_with("_block") || n.ends_with("_stem") || n.contains("chorus") || n.ends_with("trapdoor") || n.contains("planks") || n.contains("mosaic") {
        return false;
    }
    const PARTS: [&str; 20] = [
        "flower", "sapling", "short_grass", "tall_grass", "fern", "deadbush", "dead_bush", "redstone_wire", "tulip", "mushroom", "petals", "leaf_litter", "dry_grass", "rail", "carpet",
        "pressure_plate", "candle", "standing_sign", "standing_banner", "eyeblossom",
    ];
    PARTS.iter().any(|p| n.contains(p))
        || (n.ends_with("_door") || n == "wooden_door")
        || matches!(
            n,
            "dandelion" | "poppy" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose" | "blue_orchid" | "wheat" | "carrots" | "potatoes"
                | "beetroot" | "snow_layer" | "bush" | "firefly_bush" | "wildflowers" | "sweet_berry_bush" | "unpowered_repeater" | "powered_repeater" | "unpowered_comparator"
                | "powered_comparator" | "reeds" | "cactus" | "cake" | "pitcher_plant" | "lilac" | "rose_bush" | "peony" | "nether_wart" | "crimson_roots" | "warped_roots"
                | "crimson_fungus" | "warped_fungus" | "nether_sprouts" | "bamboo"
        )
}

/// Plantes et portes en deux moitiés : celle du bas part si celle du haut disparaît.
pub fn two_halves(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    (n.ends_with("_door") || n == "wooden_door") && !n.ends_with("trapdoor") || matches!(n, "sunflower" | "lilac" | "rose_bush" | "peony" | "tall_grass" | "large_fern" | "pitcher_plant")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::VanillaBlocks;

    #[test]
    fn blocks_answer_for_their_behaviour() {
        assert!(falls("minecraft:sand") && falls("white_concrete_powder") && !falls("minecraft:sandstone"));
        assert!(is_crop("minecraft:wheat") && is_crop("carrots") && !is_crop("minecraft:reeds"));
        // Les mêmes réponses par le trait `Block`
        let sand = VanillaBlocks::get("minecraft:sand", None);
        let door = VanillaBlocks::get("minecraft:oak_door", None);
        let log = VanillaBlocks::get("minecraft:birch_log", None);
        assert!(sand.falls() && !sand.needs_floor());
        assert!(door.needs_floor() && door.two_halves() && !door.falls());
        assert!(log.is_log() && !log.is_leaves() && !log.is_crop());
    }
}
