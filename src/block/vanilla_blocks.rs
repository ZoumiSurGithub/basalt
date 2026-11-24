//! `VanillaBlocks` : le registre des blocs vanilla (l'équivalent de `VanillaBlocks` dans
//! PocketMine 5, qui a remplacé l'ancienne `BlockFactory`). On obtient un bloc par son
//! nom de fonction, `VanillaBlocks::stone()`, ou d'après un nom, `VanillaBlocks::get(...)`.
//!
//! Tous les blocs du jeu sont couverts. Leurs propriétés physiques (dureté, résistance aux
//! explosions, lumière, inflammabilité) viennent des données du jeu ; l'outil adapté, le
//! butin, la réaction au clic et les données attachées sont déterminés ici, par famille :
//! le nommage du jeu est régulier (`deepslate_iron_ore` est un minerai de fer,
//! `spruce_planks` du bois), ce qui évite d'écrire 1 500 blocs un par un.

use std::collections::HashMap;

use super::block::{BaseBlock, Block, BlockProps, ContainerType, Interaction, TileType};
use super::types::{Air, Cobweb, Crop, DropsOther, Fragile, GenericBlock, Interactive, Liquid, WithTile};
use crate::item::{ToolTier, ToolType};

const WOODS: [&str; 15] = ["oak", "spruce", "birch", "jungle", "acacia", "mangrove", "cherry", "crimson", "warped", "bamboo", "pale_oak", "poplar", "wooden", "wood", "darkoak"];
const AXE: [&str; 33] = [
    "log", "planks", "stem", "hyphae", "chest", "crafting_table", "bookshelf", "barrel", "ladder", "pumpkin", "melon", "campfire", "noteblock", "jukebox",
    "lectern", "loom", "composter", "beehive", "bee_nest", "mushroom_block", "cartography_table", "fletching_table", "smithing_table", "creaking_heart",
    "daylight_detector", "standing_sign", "wall_sign", "hanging_sign", "banner", "trapdoor", "fence_gate", "cocoa", "mushroom_stem",
];
const SHOVEL: [&str; 14] = ["dirt", "grass_block", "sand", "gravel", "clay", "snow", "soul_soil", "mud", "podzol", "mycelium", "farmland", "grass_path", "dirt_path", "concrete_powder"];
const HOE: [&str; 10] = ["leaves", "hay_block", "sponge", "sculk", "moss_block", "moss_carpet", "wart_block", "shroomlight", "target", "dried_kelp_block"];
const PICKAXE: [&str; 72] = [
    "stone", "ore", "brick", "deepslate", "obsidian", "concrete", "terracotta", "andesite", "diorite", "granite", "basalt", "blackstone", "netherrack",
    "furnace", "iron", "gold", "diamond", "emerald", "copper", "quartz", "prismarine", "purpur", "end_stone", "anvil", "rail", "ice", "amethyst", "tuff",
    "calcite", "dripstone", "lantern", "chain", "cauldron", "hopper", "dispenser", "dropper", "observer", "piston", "smoker", "brewing_stand",
    "enchanting_table", "spawner", "bell", "grindstone", "lodestone", "coal_block", "redstone_block", "lapis_block", "netherite", "raw_", "ancient_debris",
    "respawn_anchor", "ender_chest", "beacon", "conduit", "magma", "glowstone", "bone_block", "coral_block", "cinnabar", "sulfur", "lightning_rod",
    "weighted_pressure_plate", "shulker_box", "crafter", "vault", "resin", "froglight", "end_rod", "icicle", "heavy_core", "pointed_",
];
/// Plantes et tapis végétaux que seules les cisailles récupèrent.
const SHEAR_ONLY: [&str; 17] = [
    "short_grass", "tall_grass", "fern", "large_fern", "deadbush", "seagrass", "nether_sprouts", "hanging_roots", "glow_lichen", "vine", "twisting_vines",
    "weeping_vines", "short_dry_grass", "tall_dry_grass", "small_dripleaf_block", "pale_hanging_moss", "leaf_litter",
];
/// Blocs qui ne donnent jamais rien.
const NO_DROP: [&str; 30] = [
    "fire", "soul_fire", "portal", "end_portal", "end_gateway", "bubble_column", "moving_block", "frosted_ice", "powder_snow", "budding_amethyst",
    "small_amethyst_bud", "medium_amethyst_bud", "large_amethyst_bud", "mob_spawner", "trial_spawner", "vault", "reinforced_deepslate", "sculk",
    "sculk_vein", "sculk_catalyst", "sculk_sensor", "sculk_shrieker", "calibrated_sculk_sensor", "turtle_egg", "frog_spawn", "bee_nest", "cake",
    "piston_arm_collision", "sticky_piston_arm_collision", "sweet_berry_bush",
];

/// Outil le plus rapide pour un bloc, deviné d'après son nom (pierre -> pioche, bois -> hache…).
fn tool_for(short: &str) -> ToolType {
    let has = |list: &[&str]| list.iter().any(|w| short.contains(w));
    if short.contains("wool") || short == "web" || short == "cobweb" || short.contains("vine") {
        ToolType::Shears
    } else if short.contains("sandstone") || short.contains("brick") || short.contains("hardened_clay") || short.contains("nylium") || short.contains("cobblestone") {
        // Noms trompeurs : du grès n'est pas du sable, des briques de boue ne sont pas de la boue
        ToolType::Pickaxe
    } else if has(&HOE) {
        ToolType::Hoe
    } else if has(&SHOVEL) {
        ToolType::Shovel
    } else if short.contains("iron_door") || short.contains("iron_trapdoor") || short.contains("copper") {
        ToolType::Pickaxe
    } else if short.contains("sapling") || short.contains("leaves") || short == "bamboo_sapling" {
        ToolType::None
    } else if has(&AXE) || has(&WOODS) {
        ToolType::Axe
    } else if short == "nether_wart" || short.contains("glass") || short.contains("candle") || short.contains("carpet") {
        ToolType::None
    } else if has(&PICKAXE) {
        ToolType::Pickaxe
    } else {
        ToolType::None
    }
}

/// Matériau minimal de la pioche pour qu'un bloc de pierre ou de métal donne son butin.
fn tier_for(short: &str) -> ToolTier {
    let has = |list: &[&str]| list.iter().any(|w| short.contains(w));
    if has(&["obsidian", "ancient_debris", "netherite_block", "respawn_anchor"]) {
        ToolTier::Diamond
    } else if has(&["diamond", "emerald", "gold_ore", "gold_block", "raw_gold", "redstone_ore"]) {
        ToolTier::Iron
    } else if has(&["iron_ore", "iron_block", "raw_iron", "lapis", "copper"]) {
        ToolTier::Stone
    } else {
        ToolTier::Wood
    }
}

/// Dureté approximative, pour le cas où les données du jeu ne sont pas chargées.
fn fallback_hardness(short: &str, tool: ToolType) -> f32 {
    match short {
        "bedrock" | "barrier" | "end_portal_frame" | "command_block" => -1.0,
        "obsidian" | "crying_obsidian" => 50.0,
        "stone" | "cobblestone" => 1.5,
        _ if short.ends_with("_ore") => 3.0,
        _ => match tool {
            ToolType::Pickaxe => 2.0,
            ToolType::Axe => 2.0,
            ToolType::Shovel => 0.5,
            ToolType::Hoe | ToolType::Shears => 0.2,
            _ => 0.3,
        },
    }
}

/// Butin d'un minerai : (objet, quantité). Les quantités variables du jeu sont ramenées à
/// leur valeur moyenne.
fn ore_drop(short: &str) -> Option<(&'static str, u8)> {
    let ore = short.strip_prefix("deepslate_").unwrap_or(short);
    Some(match ore {
        "coal_ore" => ("coal", 1),
        "iron_ore" => ("raw_iron", 1),
        "gold_ore" => ("raw_gold", 1),
        "copper_ore" => ("raw_copper", 3),
        "diamond_ore" => ("diamond", 1),
        "emerald_ore" => ("emerald", 1),
        "lapis_ore" => ("lapis_lazuli", 6),
        "redstone_ore" | "lit_redstone_ore" => ("redstone", 4),
        "nether_quartz_ore" | "quartz_ore" => ("quartz", 1),
        "nether_gold_ore" => ("gold_nugget", 4),
        _ => return None,
    })
}

/// Écran ouvert par un bloc, et données qu'il porte.
fn container_for(short: &str) -> Option<(ContainerType, Option<TileType>)> {
    let plain = short.strip_prefix("lit_").unwrap_or(short);
    Some(match plain {
        "chest" | "trapped_chest" => (ContainerType::Chest, Some(TileType::Chest)),
        "ender_chest" => (ContainerType::EnderChest, Some(TileType::EnderChest)),
        "barrel" => (ContainerType::Barrel, Some(TileType::Barrel)),
        "furnace" => (ContainerType::Furnace, Some(TileType::Furnace)),
        "blast_furnace" => (ContainerType::BlastFurnace, Some(TileType::BlastFurnace)),
        "smoker" => (ContainerType::Smoker, Some(TileType::Smoker)),
        "crafting_table" => (ContainerType::CraftingTable, None),
        "anvil" | "chipped_anvil" | "damaged_anvil" => (ContainerType::Anvil, None),
        "enchanting_table" => (ContainerType::EnchantingTable, Some(TileType::Other)),
        "brewing_stand" => (ContainerType::BrewingStand, Some(TileType::BrewingStand)),
        "hopper" => (ContainerType::Hopper, Some(TileType::Hopper)),
        "dispenser" => (ContainerType::Dispenser, Some(TileType::Dispenser)),
        "dropper" => (ContainerType::Dropper, Some(TileType::Dropper)),
        "smithing_table" => (ContainerType::SmithingTable, None),
        "grindstone" => (ContainerType::Grindstone, None),
        "loom" => (ContainerType::Loom, None),
        "stonecutter_block" | "stonecutter" => (ContainerType::Stonecutter, None),
        "cartography_table" => (ContainerType::CartographyTable, None),
        "beacon" => (ContainerType::Beacon, Some(TileType::Other)),
        "lectern" => (ContainerType::Lectern, Some(TileType::Other)),
        "crafter" => (ContainerType::Crafter, Some(TileType::Other)),
        _ if plain.ends_with("shulker_box") => (ContainerType::ShulkerBox, Some(TileType::ShulkerBox)),
        _ => return None,
    })
}

/// Blocs sans écran mais qui portent des données propres.
fn has_other_tile(short: &str) -> bool {
    const EXACT: [&str; 24] = [
        "bed", "flower_pot", "jukebox", "noteblock", "mob_spawner", "trial_spawner", "vault", "bell", "conduit", "cauldron", "beehive", "bee_nest",
        "frame", "glow_frame", "lodestone", "decorated_pot", "chiseled_bookshelf", "campfire", "soul_campfire", "end_portal", "end_gateway",
        "daylight_detector", "daylight_detector_inverted", "jigsaw",
    ];
    EXACT.contains(&short)
        || short.ends_with("_head")
        || short.ends_with("_skull")
        || short.contains("banner")
        || short.contains("command_block")
        || short.contains("comparator")
        || short.contains("sculk_s")
        || short.contains("structure_block")
        || short.contains("piston")
}

/// Objet donné par un bloc dont l'objet ne porte pas le même nom. Vide : le bloc lui-même.
fn renamed_drop(short: &str) -> (String, u8) {
    let own = |s: &str| (s.to_string(), 1u8);
    // Double dalle : deux dalles simples
    if short.ends_with("slab") && short.contains("double_") {
        return (short.replacen("double_", "", 1), 2);
    }
    // Panneaux : le bloc posé au sol ou au mur donne le panneau de son bois
    for suffix in ["_standing_sign", "_wall_sign"] {
        if let Some(wood) = short.strip_suffix(suffix) {
            return own(&format!("{}_sign", if wood == "darkoak" { "dark_oak" } else { wood }));
        }
    }
    if let Some(unlit) = short.strip_prefix("lit_") {
        if unlit != "pumpkin" {
            return own(unlit);
        }
    }
    if let Some(candle) = short.strip_suffix("_cake") {
        return own(candle);
    }
    match short {
        "standing_sign" | "wall_sign" => own("oak_sign"),
        "standing_banner" | "wall_banner" => own("banner"),
        "redstone_wire" => own("redstone"),
        "unlit_redstone_torch" => own("redstone_torch"),
        "powered_repeater" | "unpowered_repeater" => own("repeater"),
        "powered_comparator" | "unpowered_comparator" => own("comparator"),
        "daylight_detector_inverted" => own("daylight_detector"),
        "trip_wire" => own("string"),
        "reeds" => own("sugar_cane"),
        "bamboo_sapling" => own("bamboo"),
        "stonecutter_block" => own("stonecutter"),
        "soul_campfire" => own("soul_soil"),
        "campfire" => ("charcoal".to_string(), 2),
        "ender_chest" => ("obsidian".to_string(), 8),
        "chorus_plant" => own("chorus_fruit"),
        "amethyst_cluster" => ("amethyst_shard".to_string(), 4),
        "snow_layer" => own("snowball"),
        "snow" => ("snowball".to_string(), 4),
        _ => (String::new(), 1),
    }
}

/// Ce que donne une plante cultivée.
fn crop_drop(short: &str) -> Option<&'static str> {
    Some(match short {
        "wheat" => "wheat_seeds",
        "carrots" => "carrot",
        "potatoes" => "potato",
        "beetroot" => "beetroot_seeds",
        "nether_wart" => "nether_wart",
        "cocoa" => "cocoa_beans",
        "melon_stem" => "melon_seeds",
        "pumpkin_stem" => "pumpkin_seeds",
        "torchflower_crop" => "torchflower_seeds",
        "pitcher_crop" => "pitcher_pod",
        _ => return None,
    })
}

pub struct VanillaBlocks;

/// Déclare les blocs accessibles par leur nom de fonction, et leur liste.
macro_rules! named_blocks {
    ($($function:ident => $name:literal,)*) => {
        impl VanillaBlocks {
            $(
                pub fn $function() -> Box<dyn Block> {
                    VanillaBlocks::get($name, None)
                }
            )*

            /// Tous les blocs nommés ci-dessus : (nom, bloc).
            pub fn all() -> Vec<(&'static str, Box<dyn Block>)> {
                vec![$(($name, VanillaBlocks::get($name, None)),)*]
            }
        }
    };
}

named_blocks! {
    air => "air",
    stone => "stone",
    cobblestone => "cobblestone",
    deepslate => "deepslate",
    cobbled_deepslate => "cobbled_deepslate",
    granite => "granite",
    diorite => "diorite",
    andesite => "andesite",
    bedrock => "bedrock",
    dirt => "dirt",
    grass_block => "grass_block",
    sand => "sand",
    gravel => "gravel",
    clay => "clay",
    sandstone => "sandstone",
    obsidian => "obsidian",
    oak_log => "oak_log",
    oak_planks => "oak_planks",
    oak_leaves => "oak_leaves",
    spruce_planks => "spruce_planks",
    birch_planks => "birch_planks",
    glass => "glass",
    ice => "ice",
    bookshelf => "bookshelf",
    coal_ore => "coal_ore",
    iron_ore => "iron_ore",
    copper_ore => "copper_ore",
    gold_ore => "gold_ore",
    redstone_ore => "redstone_ore",
    lapis_ore => "lapis_ore",
    diamond_ore => "diamond_ore",
    emerald_ore => "emerald_ore",
    iron_block => "iron_block",
    gold_block => "gold_block",
    diamond_block => "diamond_block",
    emerald_block => "emerald_block",
    netherrack => "netherrack",
    glowstone => "glowstone",
    end_stone => "end_stone",
    netherite_block => "netherite_block",
    crafting_table => "crafting_table",
    chest => "chest",
    furnace => "furnace",
    barrel => "barrel",
    torch => "torch",
    oak_door => "oak_door",
    iron_door => "iron_door",
    lever => "lever",
    water => "water",
    lava => "lava",
    tnt => "tnt",
    white_wool => "white_wool",
    cobweb => "cobweb",
}

impl VanillaBlocks {
    /// Bloc vanilla portant ce nom (avec ou sans `minecraft:`). `hardness` est la dureté
    /// tirée des données du jeu, si on la connaît.
    pub fn get(name: &str, hardness: Option<f32>) -> Box<dyn Block> {
        VanillaBlocks::with_props(name, hardness.map(BlockProps::plain))
    }

    /// Bloc vanilla portant ce nom, avec les propriétés physiques des données du jeu.
    /// Ce que les plugins ont changé sur ce bloc est pris en compte (voir `overrides.rs`).
    pub fn with_props(name: &str, props: Option<BlockProps>) -> Box<dyn Block> {
        super::overrides::apply(VanillaBlocks::plain(name, props))
    }

    /// Le bloc tel que le jeu le définit, sans les modifications des plugins.
    pub fn plain(name: &str, props: Option<BlockProps>) -> Box<dyn Block> {
        // Bloc créé par un plugin : sa fiche dit tout, rien n'est deviné d'après son nom
        if let Some(custom) = super::custom::get(name) {
            return custom;
        }
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let tool = tool_for(short);
        let mut required = if tool == ToolType::Pickaxe { Some(tier_for(short)) } else { None };
        if short == "snow" || short == "snow_layer" {
            required = Some(ToolTier::Wood); // la neige ne se ramasse qu'à la pelle
        }
        let props = props.unwrap_or_else(|| BlockProps::plain(fallback_hardness(short, tool)));
        let base = BaseBlock::with_props(short, props, tool, required);
        let unbreakable = |base: BaseBlock| BaseBlock { hardness: -1.0, props: BlockProps { hardness: -1.0, ..base.props }, ..base };

        // Ce qui n'est pas un bloc au sens habituel
        if matches!(short, "air" | "cave_air" | "void_air" | "structure_void") || short.starts_with("light_block") {
            return Box::new(Air { base });
        }
        if matches!(short, "water" | "flowing_water" | "lava" | "flowing_lava") {
            return Box::new(Liquid { base: unbreakable(base) });
        }
        if matches!(short, "bedrock" | "barrier" | "invisible_bedrock" | "border_block" | "allow" | "deny") {
            return Box::new(GenericBlock { base: unbreakable(base) });
        }
        // Blocs qui réagissent au clic
        let (renamed, count) = renamed_drop(short);
        if let Some((screen, tile)) = container_for(short) {
            return Box::new(Interactive { base, action: Interaction::Open(screen), tile, item: renamed, count });
        }
        if short.ends_with("_door") || short.ends_with("trapdoor") || short.ends_with("fence_gate") || short.ends_with("_button") || short == "lever" {
            return Box::new(Interactive { base, action: Interaction::Toggle, tile: None, item: renamed, count });
        }
        // Butins particuliers
        if let Some((item, count)) = ore_drop(short) {
            return Box::new(DropsOther { base, item: item.to_string(), count });
        }
        if let Some(item) = crop_drop(short) {
            return Box::new(Crop { base, item });
        }
        if short == "web" || short == "cobweb" {
            return Box::new(Cobweb { base });
        }
        if short.ends_with("leaves") || short.ends_with("leaves_flowered") || SHEAR_ONLY.contains(&short) || short.starts_with("cave_vines") {
            return Box::new(Fragile { base, shearable: true });
        }
        if NO_DROP.contains(&short)
            || short.starts_with("infested_")
            || (short.ends_with("glass") && short != "tinted_glass" && !short.starts_with("hard_"))
            || (short.ends_with("glass_pane") && !short.starts_with("hard_"))
            || short == "ice"
            || (short.contains("coral") && !short.ends_with("coral_block"))
        {
            return Box::new(Fragile { base, shearable: false });
        }
        // Panneaux, puis les autres blocs porteurs de données
        if short.ends_with("_sign") {
            return Box::new(WithTile { base, tile: TileType::Sign, item: renamed });
        }
        if short == "bed" {
            return Box::new(WithTile { base, tile: TileType::Bed, item: renamed });
        }
        if has_other_tile(short) {
            return Box::new(WithTile { base, tile: TileType::Other, item: renamed });
        }
        let other = |item: &str, count: u8| -> Box<dyn Block> { Box::new(DropsOther { base: base.clone(), item: item.to_string(), count }) };
        match short {
            "stone" => return other("cobblestone", 1),
            "deepslate" => return other("cobbled_deepslate", 1),
            "grass_block" | "grass_path" | "dirt_path" | "farmland" | "mycelium" | "podzol" | "dirt_with_roots" => return other("dirt", 1),
            "clay" => return other("clay_ball", 4),
            "bookshelf" => return other("book", 3),
            "melon_block" | "melon" => return other("melon_slice", 5),
            "sea_lantern" => return other("prismarine_crystals", 3),
            "glowstone" => return other("glowstone_dust", 3),
            _ => {}
        }
        if let Some(coral) = short.strip_suffix("_coral_block") {
            if !coral.starts_with("dead_") {
                return other(&format!("dead_{}_coral_block", coral), 1);
            }
        }
        if !renamed.is_empty() {
            return other(&renamed, count);
        }
        Box::new(GenericBlock { base })
    }
}

/// Tous les blocs du jeu, construits une fois au démarrage à partir des données.
pub struct BlockRegistry {
    blocks: HashMap<String, Box<dyn Block>>,
}

impl BlockRegistry {
    /// Construit le registre : un bloc par nom, avec ses propriétés.
    pub fn new(all: impl Iterator<Item = (String, BlockProps)>) -> BlockRegistry {
        let mut blocks: HashMap<String, Box<dyn Block>> = HashMap::new();
        for (name, props) in all {
            let block = VanillaBlocks::with_props(&name, Some(props));
            blocks.insert(block.name().to_string(), block);
        }
        BlockRegistry { blocks }
    }

    /// Bloc portant ce nom complet (`minecraft:stone`).
    pub fn get(&self, name: &str) -> Option<&dyn Block> {
        self.blocks.get(name).map(|b| b.as_ref())
    }

    /// Nombre de blocs connus, et parcours de tous.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Block> {
        self.blocks.values().map(|b| b.as_ref())
    }
}
