//! `TileMap` : les données attachées aux blocs d'un monde, et leur sauvegarde.

use std::collections::HashMap;
use std::path::Path;

use super::tile::{create, type_from, type_name, Tile, TileContext};
use crate::item::ItemStack;
use crate::nbt::{self, Tag};

pub type Pos = (i32, i32, i32);

/// Nombre maximal de blocs porteurs de données dans un monde.
const MAX_TILES: usize = 200_000;

#[derive(Default)]
pub struct TileMap {
    tiles: HashMap<Pos, Box<dyn Tile>>,
    /// Modifié depuis le dernier enregistrement
    pub dirty: bool,
}

impl TileMap {
    /// Aucun bloc à données.
    pub fn new() -> TileMap {
        TileMap::default()
    }

    /// Nombre de blocs à données.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Données du bloc à cette position.
    pub fn get(&self, pos: Pos) -> Option<&dyn Tile> {
        self.tiles.get(&pos).map(|t| t.as_ref())
    }

    /// Accès en modification ; le monde est alors considéré comme changé.
    pub fn get_mut(&mut self, pos: Pos) -> Option<&mut Box<dyn Tile>> {
        let tile = self.tiles.get_mut(&pos);
        if tile.is_some() {
            self.dirty = true;
        }
        tile
    }

    /// Attache des données à un bloc. Faux si le monde en porte déjà trop.
    pub fn insert(&mut self, pos: Pos, tile: Box<dyn Tile>) -> bool {
        if self.tiles.len() >= MAX_TILES && !self.tiles.contains_key(&pos) {
            return false;
        }
        self.tiles.insert(pos, tile);
        self.dirty = true;
        true
    }

    /// Retire les données d'un bloc (cassé ou remplacé) et les rend.
    pub fn remove(&mut self, pos: Pos) -> Option<Box<dyn Tile>> {
        let tile = self.tiles.remove(&pos);
        if tile.is_some() {
            self.dirty = true;
        }
        tile
    }

    /// Positions de tous les blocs porteurs de données.
    pub fn positions(&self) -> Vec<Pos> {
        self.tiles.keys().copied().collect()
    }

    /// Fait avancer toutes les tiles d'un tick (fours).
    pub fn tick(&mut self, ctx: &mut dyn TileContext) {
        let mut changed = false;
        for tile in self.tiles.values_mut() {
            changed |= tile.tick(ctx);
        }
        self.dirty |= changed;
    }

    /// Enregistre dans `tiles.nbt` : une section par bloc, avec sa position, son type, ses
    /// objets (notés par leur nom, valable d'une version du jeu à l'autre) et son état propre.
    pub fn save(&mut self, dir: &Path, ctx: &dyn TileContext) -> std::io::Result<()> {
        let doc = self.snapshot(ctx);
        nbt::save_file(&dir.join("tiles.nbt"), &doc)
    }

    /// Document à enregistrer dans `tiles.nbt` ; les blocs sont ensuite considérés comme
    /// enregistrés. L'écriture elle-même peut se faire sur un autre fil.
    pub fn snapshot(&mut self, ctx: &dyn TileContext) -> Tag {
        let mut positions: Vec<&Pos> = self.tiles.keys().collect();
        positions.sort();
        let mut list = Vec::with_capacity(positions.len());
        for pos in positions {
            let tile = &self.tiles[pos];
            let mut entries = vec![("id", Tag::string(type_name(tile.tile_type()))), ("x", Tag::Int(pos.0)), ("y", Tag::Int(pos.1)), ("z", Tag::Int(pos.2))];
            if let Some(inv) = tile.inventory() {
                let items = inv
                    .contents()
                    .into_iter()
                    .filter_map(|(slot, item)| {
                        let name = ctx.name_of(item.id)?;
                        // (variante, usure et enchantements, pour les objets qui en ont)
                        let mut entry = vec![("Slot", Tag::Byte(slot as i8)), ("Name", Tag::string(&name)), ("Count", Tag::Byte(item.count as i8))];
                        if item.meta != 0 {
                            entry.push(("Damage", Tag::Short(item.meta as i16)));
                        }
                        if item.damage != 0 {
                            entry.push(("Wear", Tag::Int(item.damage as i32)));
                        }
                        if !item.ench.is_empty() {
                            entry.push(("BasaltEnch", Tag::Long(item.ench.0 as i64)));
                        }
                        Some(Tag::compound(entry))
                    })
                    .collect();
                entries.push(("Items", Tag::list_of(items)));
            }
            entries.extend(tile.save_extra());
            list.push(Tag::compound(entries));
        }
        self.dirty = false;
        Tag::compound(vec![("Version", Tag::Int(1)), ("Tiles", Tag::list_of(list))])
    }

    /// Recharge. Renvoie le nombre de blocs porteurs de données (0 si aucun fichier).
    pub fn load(&mut self, dir: &Path, ctx: &mut dyn TileContext) -> Result<usize, String> {
        let doc = match nbt::load_file(&dir.join("tiles.nbt"))? {
            Some(d) => d,
            None => return Ok(0),
        };
        for entry in doc.get("Tiles").and_then(|t| t.as_list()).unwrap_or(&[]) {
            let coord = |key: &str| entry.get(key).and_then(|t| t.as_i64()).filter(|v| v.abs() < 40_000_000).map(|v| v as i32);
            let (pos, kind) = match (coord("x"), coord("y"), coord("z"), entry.str_of("id").and_then(type_from)) {
                (Some(x), Some(y), Some(z), Some(kind)) => ((x, y, z), kind),
                _ => continue,
            };
            let mut tile = match create(kind) {
                Some(t) => t,
                None => continue,
            };
            if let Some(inv) = tile.inventory_mut() {
                for item in entry.get("Items").and_then(|t| t.as_list()).unwrap_or(&[]) {
                    let slot = item.get("Slot").and_then(|t| t.as_i64()).unwrap_or(-1);
                    let count = item.get("Count").and_then(|t| t.as_i64()).unwrap_or(0);
                    if let (Some(id), 0..=255, 1..=64) = (item.str_of("Name").and_then(|n| ctx.id_of(n)), slot, count) {
                        let max = inv.max_stack();
                        let extra = |key: &str| item.get(key).and_then(|t| t.as_i64()).unwrap_or(0);
                        let stack = ItemStack { meta: extra("Damage").clamp(0, 32767) as u16, damage: extra("Wear").clamp(0, 65535) as u16, ench: crate::item::Ench(extra("BasaltEnch") as u64), ..ItemStack::new(id, (count as u8).min(max), ctx.next_net_id()) };
                        inv.set_item(slot as usize, stack);
                    }
                }
            }
            if let Tag::Compound(entries) = entry {
                for (key, value) in entries {
                    if !matches!(key.as_str(), "id" | "x" | "y" | "z" | "Items") {
                        tile.load_extra(key, value);
                    }
                }
            }
            self.insert(pos, tile);
        }
        self.dirty = false;
        Ok(self.tiles.len())
    }
}
