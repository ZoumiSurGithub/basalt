//! Blocs modifiés par les plugins.
//!
//! Le pendant de `item/overrides.rs` : un plugin change ce que répond un bloc existant
//! (`server.blocks.modify("minecraft:gravel", { falls = false })`, voir `docs/PLUGINS.md`).
//! `VanillaBlocks::with_props` enveloppe alors le bloc dans `Modified` ; les questions de
//! `behavior.rs` (tombe-t-il ? lui faut-il un sol ?) regardent ici d'abord.
//!
//! Ne sont proposées que les valeurs dont le serveur décide seul : la dureté, la lumière ou
//! la glissance sont calculées par le jeu du joueur et ne peuvent pas être changées d'ici.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use super::block::{BaseBlock, Block, Drop, Interaction, TileType};
use crate::item::{Item, ToolTier, ToolType};

/// Ce qu'un plugin a changé sur un bloc. `None` : la réponse du gabarit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockOverride {
    /// Butin, à la place de celui du bloc : (objet, quantité)
    pub drops: Option<Vec<(String, u8)>>,
    /// Tombe sans appui
    pub falls: Option<bool>,
    /// Ne tient pas sans bloc dessous
    pub needs_floor: Option<bool>,
    /// Peut être cassé en survie
    pub breakable: Option<bool>,
}

static ANY: AtomicBool = AtomicBool::new(false);
static TABLE: RwLock<Option<HashMap<String, BlockOverride>>> = RwLock::new(None);

fn short(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// Change une réponse d'un bloc. `key` : `drops` (« objet=quantité,objet=quantité », vide :
/// aucun butin), `falls`, `needsFloor`, `breakable` (`true` ou `false`) ; `reset` rend le
/// bloc au gabarit. Erreur en clair si la valeur ne va pas.
pub fn set(name: &str, key: &str, value: &str) -> Result<(), String> {
    let flag = || match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{} attend true ou false, reçu « {} »", key, value)),
    };
    let mut table = TABLE.write().map_err(|_| "table des blocs indisponible".to_string())?;
    let map = table.get_or_insert_with(HashMap::new);
    if key == "reset" {
        map.remove(short(name));
        ANY.store(!map.is_empty(), Ordering::Relaxed);
        return Ok(());
    }
    let mut o = map.get(short(name)).cloned().unwrap_or_default();
    match key {
        "falls" => o.falls = Some(flag()?),
        "needsFloor" => o.needs_floor = Some(flag()?),
        "breakable" => o.breakable = Some(flag()?),
        "drops" => {
            let mut drops = Vec::new();
            for part in value.split(',').filter(|p| !p.is_empty()) {
                let (item, count) = part.split_once('=').unwrap_or((part, "1"));
                let count = count.parse::<u16>().map_err(|_| format!("quantité invalide dans le butin : « {} »", part))?;
                if item.is_empty() || count == 0 || drops.len() >= 16 {
                    return Err(format!("butin invalide : « {} » (16 objets au plus, quantité d'au moins 1)", part));
                }
                drops.push((item.to_string(), count.min(64) as u8));
            }
            o.drops = Some(drops);
        }
        _ => return Err(format!("valeur inconnue « {} » (drops, falls, needsFloor, breakable)", key)),
    }
    map.insert(short(name).to_string(), o);
    ANY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Oublie toutes les modifications (rechargement des plugins).
pub fn clear() {
    if let Ok(mut table) = TABLE.write() {
        *table = None;
    }
    ANY.store(false, Ordering::Relaxed);
}

/// Ce qu'un plugin a changé sur ce bloc, s'il y a touché.
pub fn of(name: &str) -> Option<BlockOverride> {
    if !ANY.load(Ordering::Relaxed) {
        return None;
    }
    TABLE.read().ok().and_then(|t| t.as_ref().and_then(|m| m.get(short(name)).cloned()))
}

/// Un plugin a-t-il fixé le butin de ce bloc ?
pub fn has_drops(name: &str) -> bool {
    of(name).map(|o| o.drops.is_some()).unwrap_or(false)
}

/// Le bloc tel que les plugins l'ont modifié ; lui-même s'ils n'y ont pas touché.
pub fn apply(block: Box<dyn Block>) -> Box<dyn Block> {
    match of(block.name()) {
        Some(changes) => Box::new(Modified { inner: block, changes }),
        None => block,
    }
}

/// Un bloc du jeu, avec ce qu'un plugin y a changé.
pub struct Modified {
    inner: Box<dyn Block>,
    changes: BlockOverride,
}

impl Block for Modified {
    fn base(&self) -> &BaseBlock {
        self.inner.base()
    }
    fn hardness(&self) -> f32 {
        self.inner.hardness()
    }
    fn tool_type(&self) -> ToolType {
        self.inner.tool_type()
    }
    fn required_tier(&self) -> Option<ToolTier> {
        self.inner.required_tier()
    }
    fn is_breakable(&self) -> bool {
        self.changes.breakable.unwrap_or_else(|| self.inner.is_breakable())
    }
    fn is_solid(&self) -> bool {
        self.inner.is_solid()
    }
    fn is_transparent(&self) -> bool {
        self.inner.is_transparent()
    }
    fn tile(&self) -> Option<TileType> {
        self.inner.tile()
    }
    fn is_replaceable(&self) -> bool {
        self.inner.is_replaceable()
    }
    fn interaction(&self) -> Interaction {
        self.inner.interaction()
    }
    fn can_harvest(&self, tool: &dyn Item) -> bool {
        self.inner.can_harvest(tool)
    }
    /// Le butin fixé par le plugin, aux mêmes conditions que celui du jeu : il faut l'outil
    /// qui permet de récolter le bloc.
    fn drops_with_tool(&self, tool: &dyn Item) -> Vec<Drop> {
        match &self.changes.drops {
            Some(drops) => drops.iter().map(|(item, count)| Drop::new(item, *count)).collect(),
            None => self.inner.drops_with_tool(tool),
        }
    }
    fn break_time(&self, tool: &dyn Item) -> f32 {
        self.inner.break_time(tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::behavior::{falls, needs_floor};
    use crate::block::VanillaBlocks;
    use crate::item::VanillaItems;

    #[test]
    fn plugins_change_what_a_block_answers() {
        // (blocs que les autres tests n'interrogent pas : la table est commune)
        let pickaxe = VanillaItems::get("iron_pickaxe");
        let hand = VanillaItems::hand();
        assert!(!falls("minecraft:end_stone") && falls("minecraft:red_sand"));
        set("minecraft:end_stone", "falls", "true").unwrap();
        set("red_sand", "falls", "false").unwrap();
        set("end_stone", "drops", "minecraft:diamond=2,ender_pearl").unwrap();
        assert!(falls("minecraft:end_stone") && !falls("minecraft:red_sand"));
        let stone = VanillaBlocks::get("minecraft:end_stone", Some(3.0));
        assert!(stone.falls());
        let loot: Vec<(String, u8)> = stone.drops(pickaxe.as_ref()).into_iter().map(|d| (d.item, d.count)).collect();
        assert_eq!(loot, vec![("minecraft:diamond".to_string(), 2), ("minecraft:ender_pearl".to_string(), 1)]);
        // Sans l'outil exigé, toujours rien ; incassable, rien non plus
        assert!(stone.drops(hand.as_ref()).is_empty());
        set("end_stone", "breakable", "false").unwrap();
        assert!(!VanillaBlocks::get("end_stone", Some(3.0)).is_breakable());
        // Un bloc plein à qui il faut un sol ; valeurs refusées
        set("purpur_block", "needsFloor", "true").unwrap();
        assert!(needs_floor("minecraft:purpur_block"));
        assert!(set("end_stone", "falls", "oui").is_err() && set("end_stone", "drops", "diamond=0").is_err() && set("end_stone", "hardness", "1").is_err());
        // Retour au gabarit
        for name in ["end_stone", "red_sand", "purpur_block"] {
            set(name, "reset", "").unwrap();
        }
        assert!(!falls("minecraft:end_stone") && falls("minecraft:red_sand") && !needs_floor("minecraft:purpur_block"));
        assert!(VanillaBlocks::get("end_stone", Some(3.0)).is_breakable());
    }
}
