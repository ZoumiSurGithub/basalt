//! `VanillaItems` : le registre des objets vanilla (l'équivalent de `VanillaItems` dans
//! PocketMine 5, qui a remplacé l'ancienne `ItemFactory`). On obtient un objet par son nom
//! de fonction, `VanillaItems::diamond_pickaxe()`, ou d'après un nom, `VanillaItems::get(...)`.
//!
//! Les familles (outils, armures) suivent une règle de nommage, ce qui évite d'énumérer
//! chaque objet : `iron_pickaxe` est une pioche en fer. Un nom inconnu donne un objet
//! générique, empilable par 64.

use super::armor::Armor;
use super::consumable::Food;
use super::item::{BaseItem, BlockItem, GenericItem, Item};
use super::tool::{Tool, ToolTier, ToolType};

/// Taille de pile des objets qui ne s'empilent pas par 64.
fn stack_size(short: &str) -> u8 {
    const SIXTEEN: [&str; 8] = ["ender_pearl", "snowball", "egg", "bucket", "honey_bottle", "armor_stand", "written_book", "wind_charge"];
    const SINGLE: [&str; 22] = [
        "bow", "crossbow", "trident", "shield", "fishing_rod", "flint_and_steel", "elytra", "saddle", "totem_of_undying", "mace", "brush",
        "spyglass", "enchanted_book", "writable_book", "potion", "splash_potion", "lingering_potion", "cake", "minecart", "carrot_on_a_stick",
        "warped_fungus_on_a_stick", "goat_horn",
    ];
    if SIXTEEN.contains(&short) || short.ends_with("_sign") || short.ends_with("_banner") {
        16
    } else if SINGLE.contains(&short)
        || short.ends_with("_bucket")
        || short.ends_with("_bed")
        || short == "bed"
        || short.ends_with("_boat")
        || short.ends_with("_minecart")
        || short.ends_with("_horse_armor")
        || short.starts_with("music_disc")
        || short.ends_with("_stew")
        || short.ends_with("_soup")
    {
        1
    } else {
        64
    }
}

/// Outil d'après son nom (« iron_pickaxe »), ou None.
fn tool(short: &str) -> Option<Tool> {
    if short == "shears" {
        return Some(Tool::new(short, ToolType::Shears, None));
    }
    let (material, kind) = short.rsplit_once('_')?;
    let kind = match kind {
        "pickaxe" => ToolType::Pickaxe,
        "axe" => ToolType::Axe,
        "shovel" => ToolType::Shovel,
        "hoe" => ToolType::Hoe,
        "sword" => ToolType::Sword,
        _ => return None,
    };
    Some(Tool::new(short, kind, Some(ToolTier::from_prefix(material)?)))
}

pub struct VanillaItems;

/// Déclare les objets accessibles par leur nom de fonction, et leur liste.
macro_rules! named_items {
    ($($function:ident => $name:literal,)*) => {
        impl VanillaItems {
            $(
                pub fn $function() -> Box<dyn Item> {
                    VanillaItems::get($name)
                }
            )*

            /// Tous les objets nommés ci-dessus : (nom, objet).
            pub fn all() -> Vec<(&'static str, Box<dyn Item>)> {
                vec![$(($name, VanillaItems::get($name)),)*]
            }
        }
    };
}

named_items! {
        wooden_pickaxe => "wooden_pickaxe",
        wooden_axe => "wooden_axe",
        wooden_shovel => "wooden_shovel",
        wooden_hoe => "wooden_hoe",
        wooden_sword => "wooden_sword",
        stone_pickaxe => "stone_pickaxe",
        stone_axe => "stone_axe",
        stone_shovel => "stone_shovel",
        stone_hoe => "stone_hoe",
        stone_sword => "stone_sword",
        iron_pickaxe => "iron_pickaxe",
        iron_axe => "iron_axe",
        iron_shovel => "iron_shovel",
        iron_hoe => "iron_hoe",
        iron_sword => "iron_sword",
        golden_pickaxe => "golden_pickaxe",
        golden_axe => "golden_axe",
        golden_shovel => "golden_shovel",
        golden_hoe => "golden_hoe",
        golden_sword => "golden_sword",
        diamond_pickaxe => "diamond_pickaxe",
        diamond_axe => "diamond_axe",
        diamond_shovel => "diamond_shovel",
        diamond_hoe => "diamond_hoe",
        diamond_sword => "diamond_sword",
        netherite_pickaxe => "netherite_pickaxe",
        netherite_axe => "netherite_axe",
        netherite_shovel => "netherite_shovel",
        netherite_hoe => "netherite_hoe",
        netherite_sword => "netherite_sword",
        shears => "shears",
        bow => "bow",
        stick => "stick",
        coal => "coal",
        charcoal => "charcoal",
        iron_ingot => "iron_ingot",
        gold_ingot => "gold_ingot",
        copper_ingot => "copper_ingot",
        diamond => "diamond",
        emerald => "emerald",
        redstone => "redstone",
        lapis_lazuli => "lapis_lazuli",
        quartz => "quartz",
        raw_iron => "raw_iron",
        raw_gold => "raw_gold",
        raw_copper => "raw_copper",
        string => "string",
        feather => "feather",
        flint => "flint",
        leather => "leather",
        bone => "bone",
        book => "book",
        paper => "paper",
        clay_ball => "clay_ball",
        glowstone_dust => "glowstone_dust",
        ender_pearl => "ender_pearl",
        snowball => "snowball",
        egg => "egg",
        bucket => "bucket",
        water_bucket => "water_bucket",
        lava_bucket => "lava_bucket",
        apple => "apple",
        golden_apple => "golden_apple",
        bread => "bread",
        cooked_beef => "cooked_beef",
        cooked_porkchop => "cooked_porkchop",
        cooked_chicken => "cooked_chicken",
        carrot => "carrot",
        potato => "potato",
        baked_potato => "baked_potato",
        melon_slice => "melon_slice",
        cookie => "cookie",
        leather_helmet => "leather_helmet",
        iron_helmet => "iron_helmet",
        iron_chestplate => "iron_chestplate",
        iron_leggings => "iron_leggings",
        iron_boots => "iron_boots",
        diamond_helmet => "diamond_helmet",
        diamond_chestplate => "diamond_chestplate",
        diamond_leggings => "diamond_leggings",
        diamond_boots => "diamond_boots",
}

impl VanillaItems {
    /// Objet vanilla portant ce nom (avec ou sans `minecraft:`).
    /// Ce que les plugins ont changé sur cet objet est pris en compte (voir `overrides.rs`).
    /// Un objet créé par un plugin est rendu de la même façon (voir `custom.rs`).
    pub fn get(name: &str) -> Box<dyn Item> {
        if let Some(custom) = super::custom::get(name) {
            return super::overrides::apply(custom);
        }
        super::overrides::apply(VanillaItems::plain(name))
    }

    /// L'objet tel que le jeu le définit, sans les modifications des plugins.
    pub fn plain(name: &str) -> Box<dyn Item> {
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        if let Some(t) = tool(short) {
            return Box::new(t);
        }
        if let Some(a) = Armor::from_name(short) {
            return Box::new(a);
        }
        if let Some(f) = Food::from_name(short, stack_size(short)) {
            return Box::new(f);
        }
        Box::new(GenericItem { base: BaseItem::new(short, stack_size(short)) })
    }

    /// Objet qui pose le bloc du même nom.
    pub fn block_item(name: &str) -> Box<dyn Item> {
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let base = BaseItem::new(short, stack_size(short));
        Box::new(BlockItem { block: base.name.clone(), base })
    }

    /// La main vide.
    pub fn hand() -> Box<dyn Item> {
        Box::new(GenericItem { base: BaseItem::new("air", 64) })
    }
}
