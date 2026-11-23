//! Blocs, organisés comme dans PocketMine et Altay.
//!
//! - `Block` : la base de tous les blocs (dureté, outil, butin, temps de minage, clic) ;
//! - `GenericBlock`, `Air`, `Liquid`, `DropsOther`, `Fragile`, `Cobweb`, `Crop`, `Interactive`,
//!   `WithTile` :
//!   les blocs vanilla, bâtis dessus ;
//! - `VanillaBlocks` : le registre des blocs vanilla, comme dans PocketMine 5 —
//!   `VanillaBlocks::stone()`, ou `VanillaBlocks::get(nom, dureté)` d'après un nom.
//!
//! Ajouter un bloc particulier revient à écrire une structure qui contient un `BaseBlock`
//! et à redéfinir, dans son `impl Block`, les seules méthodes qui le distinguent.

#[allow(clippy::module_inception)]
pub mod behavior;
mod block;
pub mod overrides;
pub mod placement;
mod types;
mod vanilla_blocks;

pub use self::block::{BaseBlock, Block, BlockProps, ContainerType, Drop, Interaction, TileType};
pub use self::types::{Air, Cobweb, Crop, DropsOther, Fragile, GenericBlock, Interactive, Liquid, WithTile};
pub use self::vanilla_blocks::{BlockRegistry, VanillaBlocks};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{VanillaItems, ToolTier, ToolType};

    fn drops(block: &str, tool: &str) -> Vec<(String, u8)> {
        let tool = if tool.is_empty() { VanillaItems::hand() } else { VanillaItems::get(tool) };
        VanillaBlocks::get(block, None).drops(tool.as_ref()).into_iter().map(|d| (d.item.replace("minecraft:", ""), d.count)).collect()
    }

    #[test]
    fn vanilla_blocks_inherit_from_the_same_base() {
        // La pierre : pioche exigée, donne de la pierre taillée
        let stone = VanillaBlocks::get("minecraft:stone", Some(1.5));
        assert_eq!((stone.name(), stone.tool_type(), stone.required_tier()), ("minecraft:stone", ToolType::Pickaxe, Some(ToolTier::Wood)));
        assert_eq!(drops("stone", ""), vec![]);
        assert_eq!(drops("stone", "wooden_pickaxe"), vec![("cobblestone".to_string(), 1)]);
        assert_eq!(drops("stone", "iron_shovel"), vec![]);
        // Minerais : le matériau de la pioche compte
        assert_eq!(drops("diamond_ore", "stone_pickaxe"), vec![]);
        assert_eq!(drops("deepslate_diamond_ore", "iron_pickaxe"), vec![("diamond".to_string(), 1)]);
        assert_eq!(drops("iron_ore", "wooden_pickaxe"), vec![]);
        assert_eq!(drops("iron_ore", "stone_pickaxe"), vec![("raw_iron".to_string(), 1)]);
        assert_eq!(drops("gold_ore", "golden_pickaxe"), vec![]);
        assert_eq!(drops("obsidian", "iron_pickaxe"), vec![]);
        assert_eq!(drops("obsidian", "diamond_pickaxe"), vec![("obsidian".to_string(), 1)]);
        // Terre et bois : à la main aussi
        assert_eq!(drops("grass_block", ""), vec![("dirt".to_string(), 1)]);
        assert_eq!(drops("oak_planks", ""), vec![("oak_planks".to_string(), 1)]);
        assert_eq!(VanillaBlocks::get("spruce_log", None).tool_type(), ToolType::Axe);
        assert_eq!(VanillaBlocks::get("sand", None).tool_type(), ToolType::Shovel);
        // Noms trompeurs
        for (name, tool) in [
            ("sandstone", ToolType::Pickaxe), ("red_sandstone_stairs", ToolType::Pickaxe), ("mud_bricks", ToolType::Pickaxe),
            ("mossy_cobblestone", ToolType::Pickaxe), ("crimson_nylium", ToolType::Pickaxe), ("moss_block", ToolType::Hoe),
            ("concrete_powder", ToolType::Shovel), ("white_concrete", ToolType::Pickaxe), ("oak_sapling", ToolType::None),
            ("iron_door", ToolType::Pickaxe), ("crimson_planks", ToolType::Axe), ("white_wool", ToolType::Shears),
        ] {
            assert_eq!(VanillaBlocks::get(name, None).tool_type(), tool, "{}", name);
        }
        // Fragiles : rien, sauf les feuilles aux cisailles
        assert_eq!(drops("glass", "diamond_pickaxe"), vec![]);
        assert_eq!(drops("oak_leaves", ""), vec![]);
        assert_eq!(drops("oak_leaves", "shears"), vec![("oak_leaves".to_string(), 1)]);
        // Incassables et non solides
        let bedrock = VanillaBlocks::get("bedrock", None);
        assert!(!bedrock.is_breakable() && bedrock.drops(VanillaItems::get("netherite_pickaxe").as_ref()).is_empty());
        let air = VanillaBlocks::get("air", None);
        assert!(!air.is_solid() && air.is_replaceable() && !air.is_breakable());
        // Blocs qui réagissent au clic
        assert_eq!(VanillaBlocks::get("chest", None).interaction(), Interaction::Open(ContainerType::Chest));
        assert_eq!(VanillaBlocks::get("lit_furnace", None).interaction(), Interaction::Open(ContainerType::Furnace));
        assert_eq!(VanillaBlocks::get("crafting_table", None).interaction(), Interaction::Open(ContainerType::CraftingTable));
        assert_eq!(VanillaBlocks::get("oak_door", None).interaction(), Interaction::Toggle);
        assert_eq!(VanillaBlocks::get("stone", None).interaction(), Interaction::None);
        // Bloc inconnu : traité comme un bloc ordinaire
        assert_eq!(drops("monplugin:rubis_brut", ""), vec![("monplugin:rubis_brut".to_string(), 1)]);
    }

    #[test]
    fn blocks_are_reachable_by_name_like_in_pocketmine() {
        assert_eq!(VanillaBlocks::stone().name(), "minecraft:stone");
        assert_eq!(VanillaBlocks::diamond_ore().required_tier(), Some(ToolTier::Iron));
        assert_eq!(VanillaBlocks::chest().interaction(), Interaction::Open(ContainerType::Chest));
        assert!(!VanillaBlocks::bedrock().is_breakable());
        assert!(VanillaBlocks::air().is_replaceable());
        assert_eq!(VanillaBlocks::oak_planks().tool_type(), ToolType::Axe);
        // Chaque bloc nommé existe bien sous ce nom
        for (label, block) in VanillaBlocks::all() {
            assert_eq!(block.short_name(), label);
        }
        assert!(VanillaBlocks::all().len() >= 40);
    }

    #[test]
    fn mining_time_follows_the_game_rule() {
        let stone = VanillaBlocks::get("stone", Some(1.5));
        let time = |tool: &str| {
            let tool = if tool.is_empty() { VanillaItems::hand() } else { VanillaItems::get(tool) };
            (stone.break_time(tool.as_ref()) * 1000.0).round() / 1000.0
        };
        // 1,5 × 5 à la main ; 1,5 × 1,5 divisé par la vitesse de la pioche sinon
        assert_eq!(time(""), 7.5);
        assert_eq!(time("wooden_pickaxe"), 1.125);
        assert_eq!(time("iron_pickaxe"), 0.375);
        assert_eq!(time("diamond_pickaxe"), 0.281);
        assert_eq!(time("diamond_shovel"), 7.5);
        assert!(VanillaBlocks::get("bedrock", None).break_time(VanillaItems::hand().as_ref()).is_infinite());
        // L'obsidienne à la pioche en diamant : 50 × 1,5 / 8
        assert_eq!(VanillaBlocks::get("obsidian", Some(50.0)).break_time(VanillaItems::get("diamond_pickaxe").as_ref()), 9.375);
    }

    /// Tous les blocs du jeu passent par le registre, et tout ce qu'ils donnent existe.
    #[test]
    fn every_block_of_the_game_is_covered() {
        use crate::item::VanillaItems;
        let data = crate::data::GameData::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).expect("données du jeu");
        let registry = data.block_registry();
        assert!(registry.len() >= 1400, "{} blocs seulement", registry.len());
        let tools = [VanillaItems::hand(), VanillaItems::netherite_pickaxe(), VanillaItems::get("shears"), VanillaItems::netherite_shovel(), VanillaItems::diamond_sword()];
        let mut missing = Vec::new();
        let (mut with_tile, mut screens, mut unbreakable, mut no_tool) = (0, 0, 0, 0);
        for block in registry.iter() {
            for tool in &tools {
                for drop in block.drops(tool.as_ref()) {
                    let known = data.item(&drop.item).is_some() || (drop.item == block.name() && data.item_for_block_name(block.name()).is_some());
                    if !known {
                        missing.push(format!("{} -> {}", block.short_name(), drop.item));
                    }
                    assert!(drop.count >= 1 && drop.count <= 64);
                }
            }
            with_tile += block.tile().is_some() as u32;
            screens += matches!(block.interaction(), Interaction::Open(_)) as u32;
            unbreakable += !block.is_breakable() as u32;
            no_tool += (block.tool_type() == ToolType::None) as u32;
            assert!(block.break_time(tools[0].as_ref()) >= 0.0);
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "objets inconnus du jeu : {}", missing.join(", "));
        println!("blocs {} | données attachées {} | écrans {} | incassables {} | sans outil {}", registry.len(), with_tile, screens, unbreakable, no_tool);
        assert!(with_tile >= 100 && screens >= 30 && unbreakable >= 10);
        // Quelques blocs précis, avec les propriétés des données
        let get = |n: &str| registry.get(&format!("minecraft:{}", n)).unwrap_or_else(|| panic!("{} absent", n));
        assert_eq!(get("glowstone").light_level(), 15);
        assert_eq!(get("lava").light_level(), 15);
        assert!(get("oak_planks").is_flammable() && !get("stone").is_flammable());
        assert!(!get("torch").is_solid() && get("glass").is_solid() && get("glass").is_transparent());
        assert!(get("obsidian").blast_resistance() > get("stone").blast_resistance());
        assert!(!get("bedrock").is_breakable() && !get("command_block").is_breakable());
        assert_eq!(get("lit_furnace").tile(), Some(TileType::Furnace));
        assert_eq!(get("spruce_wall_sign").tile(), Some(TileType::Sign));
        assert_eq!(get("white_shulker_box").interaction(), Interaction::Open(ContainerType::ShulkerBox));
    }
}
