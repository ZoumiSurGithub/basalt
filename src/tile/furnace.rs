//! `FurnaceTile` : four, haut fourneau et fumoir. Bâtie sur `Tile`, elle y ajoute la
//! combustion et la cuisson.

use super::tile::{Tile, TileContext};
use crate::block::TileType;
use crate::inventory::{FurnaceInventory, Inventory};
use crate::item::{ItemStack, MAX_STACK};
use crate::nbt::Tag;

/// Durée de combustion d'un objet, en ticks (0 : ce n'est pas un combustible).
pub fn fuel_time(short: &str) -> u32 {
    match short {
        "lava_bucket" => 20_000,
        "coal_block" => 16_000,
        "dried_kelp_block" => 4_000,
        "blaze_rod" => 2_400,
        "coal" | "charcoal" => 1_600,
        "stick" | "bamboo" => 100,
        "bookshelf" | "crafting_table" | "chest" | "bow" | "fishing_rod" | "ladder" => 300,
        _ if short.ends_with("_planks") || short.ends_with("_log") || short.ends_with("_wood") || short.ends_with("_stem") || short.ends_with("_hyphae") => 300,
        _ if short.ends_with("_slab") && is_wood(short) => 150,
        _ if is_wood(short) && (short.ends_with("_stairs") || short.ends_with("_fence") || short.ends_with("_fence_gate") || short.ends_with("_trapdoor") || short.ends_with("_pressure_plate")) => 300,
        _ if is_wood(short) && (short.ends_with("_door") || short.ends_with("_sign")) => 200,
        _ if short.starts_with("wooden_") => 200,
        _ if short.ends_with("_sapling") || short.ends_with("_button") && is_wood(short) || short.ends_with("_wool") => 100,
        _ if short.ends_with("_carpet") => 67,
        _ => 0,
    }
}

/// Objet en bois (bon combustible) ?
fn is_wood(short: &str) -> bool {
    ["oak", "spruce", "birch", "jungle", "acacia", "mangrove", "cherry", "bamboo", "pale_oak", "wooden"].iter().any(|w| short.contains(w))
}

/// Résultat de la cuisson d'un objet : (objet obtenu, est-ce un minerai, est-ce un aliment).
pub fn smelt_result(short: &str) -> Option<(&'static str, bool, bool)> {
    let ore = |r| Some((r, true, false));
    let food = |r| Some((r, false, true));
    let other = |r| Some((r, false, false));
    let plain = short.strip_prefix("deepslate_").unwrap_or(short);
    match plain {
        "raw_iron" | "iron_ore" => ore("iron_ingot"),
        "raw_gold" | "gold_ore" | "nether_gold_ore" => ore("gold_ingot"),
        "raw_copper" | "copper_ore" => ore("copper_ingot"),
        "coal_ore" => ore("coal"),
        "diamond_ore" => ore("diamond"),
        "emerald_ore" => ore("emerald"),
        "lapis_ore" => ore("lapis_lazuli"),
        "redstone_ore" => ore("redstone"),
        "quartz_ore" => ore("quartz"),
        "ancient_debris" => ore("netherite_scrap"),
        "beef" => food("cooked_beef"),
        "porkchop" => food("cooked_porkchop"),
        "chicken" => food("cooked_chicken"),
        "mutton" => food("cooked_mutton"),
        "rabbit" => food("cooked_rabbit"),
        "cod" => food("cooked_cod"),
        "salmon" => food("cooked_salmon"),
        "potato" => food("baked_potato"),
        "kelp" => food("dried_kelp"),
        "sand" | "red_sand" => other("glass"),
        "cobblestone" => other("stone"),
        "stone" => other("smooth_stone"),
        "cobbled_deepslate" => other("deepslate"),
        "clay_ball" => other("brick"),
        "clay" => other("hardened_clay"),
        "netherrack" => other("netherbrick"),
        "wet_sponge" => other("sponge"),
        "cactus" => other("green_dye"),
        "sea_pickle" => other("lime_dye"),
        "chorus_fruit" => other("popped_chorus_fruit"),
        _ if plain.ends_with("_log") || plain.ends_with("_wood") => other("charcoal"),
        _ => None,
    }
}

pub struct FurnaceTile {
    kind: TileType,
    inventory: FurnaceInventory,
    /// Ticks de feu restants
    burn_left: u32,
    /// Avancement de la cuisson en cours, en ticks
    cook: u32,
    /// Durée du combustible en train de brûler (pour la jauge de flamme)
    burn_total: u32,
}

impl FurnaceTile {
    /// Four éteint et vide.
    pub fn new(kind: TileType) -> FurnaceTile {
        FurnaceTile { kind, inventory: FurnaceInventory::new(), burn_left: 0, cook: 0, burn_total: 0 }
    }

    /// Le feu brûle-t-il encore ?
    pub fn is_burning(&self) -> bool {
        self.burn_left > 0
    }

    /// Ticks nécessaires pour cuire un objet : le haut fourneau et le fumoir vont deux fois plus vite.
    pub fn cook_time(&self) -> u32 {
        if self.kind == TileType::Furnace {
            200
        } else {
            100
        }
    }

    /// Avancement de la cuisson, de 0 à 1.
    pub fn progress(&self) -> f32 {
        self.cook as f32 / self.cook_time() as f32
    }

    /// Objet que donnerait l'ingrédient présent, si ce four sait le cuire et s'il y a la place.
    fn pending(&self, ctx: &dyn TileContext) -> Option<i16> {
        let input = self.inventory.smelting();
        if input.is_empty() {
            return None;
        }
        let name = ctx.name_of(input.id)?;
        let (result, is_ore, is_food) = smelt_result(name.strip_prefix("minecraft:").unwrap_or(&name))?;
        let allowed = match self.kind {
            TileType::BlastFurnace => is_ore,
            TileType::Smoker => is_food,
            _ => true,
        };
        if !allowed {
            return None;
        }
        let id = ctx.id_of(result)?;
        let out = self.inventory.result();
        if out.is_empty() || (out.id == id && out.count < MAX_STACK) {
            Some(id)
        } else {
            None
        }
    }
}

impl Tile for FurnaceTile {
    /// Four, haut fourneau ou fumoir.
    fn tile_type(&self) -> TileType {
        self.kind
    }
    fn network_id(&self) -> &'static str {
        match self.kind {
            TileType::BlastFurnace => "BlastFurnace",
            TileType::Smoker => "Smoker",
            _ => "Furnace",
        }
    }
    /// Inventaire du four.
    fn inventory(&self) -> Option<&dyn Inventory> {
        Some(&self.inventory)
    }
    fn inventory_mut(&mut self) -> Option<&mut dyn Inventory> {
        Some(&mut self.inventory)
    }

    /// Un tick de cuisson ; vrai si quelque chose a changé (à envoyer aux joueurs).
    fn tick(&mut self, ctx: &mut dyn TileContext) -> bool {
        let before = (self.burn_left, self.cook);
        let result = self.pending(ctx);
        // Rallumer le feu seulement s'il y a quelque chose à cuire : pas de combustible gaspillé
        if self.burn_left == 0 && result.is_some() {
            let fuel = self.inventory.fuel();
            let time = if fuel.is_empty() { 0 } else { ctx.name_of(fuel.id).map(|n| fuel_time(n.strip_prefix("minecraft:").unwrap_or(&n))).unwrap_or(0) };
            if time > 0 {
                // Dans un haut fourneau ou un fumoir, le combustible part deux fois plus vite
                self.burn_left = time * self.cook_time() / 200;
                self.burn_total = self.burn_left;
                self.inventory.set_item(1, ItemStack { count: fuel.count - 1, ..fuel });
            }
        }
        if self.burn_left > 0 {
            self.burn_left -= 1;
            match result {
                Some(id) => {
                    self.cook += 1;
                    if self.cook >= self.cook_time() {
                        self.cook = 0;
                        let input = self.inventory.smelting();
                        self.inventory.set_item(0, ItemStack { count: input.count - 1, ..input });
                        let out = self.inventory.result();
                        let new = if out.is_empty() { ItemStack::new(id, 1, ctx.next_net_id()) } else { ItemStack { count: out.count + 1, ..out } };
                        self.inventory.set_item(2, new);
                    }
                }
                None => self.cook = 0,
            }
        } else {
            // Feu éteint : la cuisson commencée retombe peu à peu
            self.cook = self.cook.saturating_sub(2);
        }
        before != (self.burn_left, self.cook)
    }

    /// Jauges affichées : cuisson et combustible restant.
    fn gauges(&self) -> Option<(u32, u32, u32, u32)> {
        Some((self.cook, self.cook_time(), self.burn_left, self.burn_total.max(self.burn_left).max(1)))
    }

    /// Temps de feu et de cuisson, enregistrés avec le monde.
    fn save_extra(&self) -> Vec<(&'static str, Tag)> {
        vec![("BurnTime", Tag::Int(self.burn_left as i32)), ("CookTime", Tag::Int(self.cook as i32))]
    }

    /// Relit ces temps (bornés au cas où le fichier serait faux).
    fn load_extra(&mut self, key: &str, value: &Tag) {
        let ticks = value.as_i64().unwrap_or(0).clamp(0, 40_000) as u32;
        match key {
            "BurnTime" => self.burn_left = ticks,
            "CookTime" => self.cook = ticks.min(200),
            _ => {}
        }
    }
}
