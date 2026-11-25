//! Potions : ce que contient une fiole d'après sa variante (le numéro que le jeu lui donne).
//!
//! Trois objets partagent ces numéros : `potion` (on la boit), `splash_potion` (on la lance,
//! elle éclabousse autour de son point de chute) et `lingering_potion` (lancée aussi ; son
//! nuage persistant n'existe pas encore, elle agit comme une potion jetable plus brève).

/// Effets du jeu utilisés par les potions (numéros de `MobEffect`).
const SPEED: i32 = 1;
const SLOWNESS: i32 = 2;
const STRENGTH: i32 = 5;
const INSTANT_HEALTH: i32 = 6;
const INSTANT_DAMAGE: i32 = 7;
const JUMP_BOOST: i32 = 8;
const REGENERATION: i32 = 10;
const RESISTANCE: i32 = 11;
const FIRE_RESISTANCE: i32 = 12;
const WATER_BREATHING: i32 = 13;
const INVISIBILITY: i32 = 14;
const NIGHT_VISION: i32 = 16;
const WEAKNESS: i32 = 18;
const POISON: i32 = 19;
const WITHER: i32 = 20;
const SLOW_FALLING: i32 = 27;

/// Un effet d'une potion : (effet, niveau à partir de 0, durée en ticks ; 1 : instantané).
pub type Dose = (i32, i32, u32);

const MIN: u32 = 20 * 60;

/// Les potions du jeu : (variante, nom, effets).
const POTIONS: [(u16, &str, &[Dose]); 43] = [
    (0, "water", &[]),
    (1, "mundane", &[]),
    (2, "long_mundane", &[]),
    (3, "thick", &[]),
    (4, "awkward", &[]),
    (5, "night_vision", &[(NIGHT_VISION, 0, 3 * MIN)]),
    (6, "long_night_vision", &[(NIGHT_VISION, 0, 8 * MIN)]),
    (7, "invisibility", &[(INVISIBILITY, 0, 3 * MIN)]),
    (8, "long_invisibility", &[(INVISIBILITY, 0, 8 * MIN)]),
    (9, "leaping", &[(JUMP_BOOST, 0, 3 * MIN)]),
    (10, "long_leaping", &[(JUMP_BOOST, 0, 8 * MIN)]),
    (11, "strong_leaping", &[(JUMP_BOOST, 1, MIN * 3 / 2)]),
    (12, "fire_resistance", &[(FIRE_RESISTANCE, 0, 3 * MIN)]),
    (13, "long_fire_resistance", &[(FIRE_RESISTANCE, 0, 8 * MIN)]),
    (14, "swiftness", &[(SPEED, 0, 3 * MIN)]),
    (15, "long_swiftness", &[(SPEED, 0, 8 * MIN)]),
    (16, "strong_swiftness", &[(SPEED, 1, MIN * 3 / 2)]),
    (17, "slowness", &[(SLOWNESS, 0, MIN * 3 / 2)]),
    (18, "long_slowness", &[(SLOWNESS, 0, 4 * MIN)]),
    (19, "water_breathing", &[(WATER_BREATHING, 0, 3 * MIN)]),
    (20, "long_water_breathing", &[(WATER_BREATHING, 0, 8 * MIN)]),
    (21, "healing", &[(INSTANT_HEALTH, 0, 1)]),
    (22, "strong_healing", &[(INSTANT_HEALTH, 1, 1)]),
    (23, "harming", &[(INSTANT_DAMAGE, 0, 1)]),
    (24, "strong_harming", &[(INSTANT_DAMAGE, 1, 1)]),
    (25, "poison", &[(POISON, 0, 45 * 20)]),
    (26, "long_poison", &[(POISON, 0, 2 * MIN)]),
    (27, "strong_poison", &[(POISON, 1, 22 * 20)]),
    (28, "regeneration", &[(REGENERATION, 0, 45 * 20)]),
    (29, "long_regeneration", &[(REGENERATION, 0, 2 * MIN)]),
    (30, "strong_regeneration", &[(REGENERATION, 1, 22 * 20)]),
    (31, "strength", &[(STRENGTH, 0, 3 * MIN)]),
    (32, "long_strength", &[(STRENGTH, 0, 8 * MIN)]),
    (33, "strong_strength", &[(STRENGTH, 1, MIN * 3 / 2)]),
    (34, "weakness", &[(WEAKNESS, 0, MIN * 3 / 2)]),
    (35, "long_weakness", &[(WEAKNESS, 0, 4 * MIN)]),
    (36, "wither", &[(WITHER, 1, 40 * 20)]),
    (37, "turtle_master", &[(SLOWNESS, 3, 20 * 20), (RESISTANCE, 2, 20 * 20)]),
    (38, "long_turtle_master", &[(SLOWNESS, 3, 40 * 20), (RESISTANCE, 2, 40 * 20)]),
    (39, "strong_turtle_master", &[(SLOWNESS, 5, 20 * 20), (RESISTANCE, 3, 20 * 20)]),
    (40, "slow_falling", &[(SLOW_FALLING, 0, MIN * 3 / 2)]),
    (41, "long_slow_falling", &[(SLOW_FALLING, 0, 4 * MIN)]),
    (42, "strong_slowness", &[(SLOWNESS, 3, 20 * 20)]),
];

/// Ce que contient une potion de cette variante (rien pour de l'eau ou une variante inconnue).
pub fn potion_effects(meta: u16) -> &'static [Dose] {
    POTIONS.iter().find(|p| p.0 == meta).map(|p| p.2).unwrap_or(&[])
}

/// Variante d'une potion d'après son nom (`poison`, `strong_healing`) ou son numéro.
pub fn potion_id(word: &str) -> Option<u16> {
    let short = word.trim_start_matches("minecraft:").to_ascii_lowercase();
    if let Ok(n) = short.parse::<u16>() {
        return POTIONS.iter().any(|p| p.0 == n).then_some(n);
    }
    POTIONS.iter().find(|p| p.1 == short).map(|p| p.0)
}

/// Nom d'une variante de potion.
pub fn potion_name(meta: u16) -> &'static str {
    POTIONS.iter().find(|p| p.0 == meta).map(|p| p.1).unwrap_or("?")
}

/// Couleur des éclaboussures d'une potion (ARGB), d'après son premier effet.
pub fn potion_color(meta: u16) -> i32 {
    let rgb: i32 = match potion_effects(meta).first().map(|d| d.0) {
        Some(SPEED) => 0x7CAFC6,
        Some(SLOWNESS) => 0x5A6C81,
        Some(STRENGTH) => 0x932423,
        Some(INSTANT_HEALTH) => 0xF82423,
        Some(INSTANT_DAMAGE) => 0x430A09,
        Some(JUMP_BOOST) => 0x22FF4C,
        Some(REGENERATION) => 0xCD5CAB,
        Some(FIRE_RESISTANCE) => 0xE49A3A,
        Some(WATER_BREATHING) => 0x2E5299,
        Some(INVISIBILITY) => 0x7F8392,
        Some(NIGHT_VISION) => 0x1F1FA1,
        Some(WEAKNESS) => 0x484D48,
        Some(POISON) => 0x4E9331,
        Some(WITHER) => 0x352A27,
        Some(SLOW_FALLING) => 0xF7F8E0,
        _ => 0x385DC6,
    };
    rgb | (0xFFu32 << 24) as i32
}

/// Un objet est-il une potion à lancer ? La part de durée qu'elle garde (les trois quarts
/// pour une potion jetable, un quart pour une persistante), sinon None.
pub fn thrown_share(item: &str) -> Option<f32> {
    match item.trim_start_matches("minecraft:") {
        "splash_potion" => Some(0.75),
        "lingering_potion" => Some(0.25),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn potions_are_known_by_number_and_name() {
        for (i, p) in POTIONS.iter().enumerate() {
            assert_eq!(p.0 as usize, i, "les variantes se suivent");
        }
        assert_eq!(potion_id("poison"), Some(25));
        assert_eq!(potion_id("minecraft:Strong_Healing"), Some(22));
        assert_eq!((potion_id("21"), potion_id("99"), potion_id("soupe")), (Some(21), None, None));
        assert_eq!(potion_name(14), "swiftness");
        // Poison : 45 secondes ; soin : instantané ; eau : rien
        assert_eq!(potion_effects(25), &[(POISON, 0, 900)]);
        assert_eq!(potion_effects(21), &[(INSTANT_HEALTH, 0, 1)]);
        assert!(potion_effects(0).is_empty() && potion_effects(500).is_empty());
        // Maître tortue : deux effets à la fois
        assert_eq!(potion_effects(37).len(), 2);
        assert_eq!((thrown_share("minecraft:splash_potion"), thrown_share("potion")), (Some(0.75), None));
    }
}
