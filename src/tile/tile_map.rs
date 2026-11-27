//! `TileMap` : les données attachées aux blocs d'un monde, et leur sauvegarde.

use std::collections::HashMap;
use std::path::Path;

use super::tile::{create, type_from, type_name, Tile, TileContext};
use crate::block::TileType;
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

    /// Les données de tous les blocs au format du jeu, chunk par chunk : pour chacun, ses
    /// blocs à données mis bout à bout (NBT petit-boutiste), tels que le jeu les range dans la
    /// base du monde. Les blocs sont ensuite considérés comme enregistrés.
    pub fn snapshot_chunks(&mut self, ctx: &dyn TileContext) -> HashMap<(i32, i32), Vec<u8>> {
        let mut positions: Vec<&Pos> = self.tiles.keys().collect();
        positions.sort();
        let mut chunks: HashMap<(i32, i32), Vec<u8>> = HashMap::new();
        for pos in positions {
            let tile = &self.tiles[pos];
            let kind = tile.tile_type();
            let mut entries: Vec<(String, Tag)> = vec![("id".into(), Tag::string(type_name(kind))), ("x".into(), Tag::Int(pos.0)), ("y".into(), Tag::Int(pos.1)), ("z".into(), Tag::Int(pos.2)), ("isMovable".into(), Tag::Byte(1))];
            if let Some(inv) = tile.inventory() {
                let items: Vec<Tag> = inv
                    .contents()
                    .into_iter()
                    .filter_map(|(slot, item)| {
                        let name = ctx.name_of(item.id)?;
                        let mut entry = vec![("Slot", Tag::Byte(slot as i8)), ("Name", Tag::string(&name)), ("Count", Tag::Byte(item.count as i8)), ("Damage", Tag::Short(item.meta as i16)), ("WasPickedUp", Tag::Byte(0))];
                        // Usure et enchantements : dans `tag`, comme le jeu
                        let mut extra = Vec::new();
                        if item.damage != 0 {
                            extra.push(("Damage", Tag::Int(item.damage as i32)));
                        }
                        if !item.ench.is_empty() {
                            let list = item.ench.list().into_iter().map(|(id, level)| Tag::compound(vec![("id", Tag::Short(id as i16)), ("lvl", Tag::Short(level as i16))])).collect();
                            extra.push(("ench", Tag::List(10, list)));
                        }
                        if !extra.is_empty() {
                            entry.push(("tag", Tag::compound(extra)));
                        }
                        Some(Tag::compound(entry))
                    })
                    .collect();
                entries.push(("Items".into(), Tag::List(10, items)));
            }
            for (key, value) in tile.save_extra() {
                match (key, value) {
                    // Un panneau : son texte est celui de sa face avant
                    ("Text", Tag::Str(text)) if kind == TileType::Sign => {
                        let side = |text: &str| Tag::compound(vec![("Text", Tag::string(text)), ("TextOwner", Tag::string("")), ("SignTextColor", Tag::Int(-16_777_216)), ("IgnoreLighting", Tag::Byte(0)), ("HideGlowOutline", Tag::Byte(0)), ("PersistFormatting", Tag::Byte(1))]);
                        entries.push(("FrontText".into(), side(&text)));
                        entries.push(("BackText".into(), side("")));
                        entries.push(("IsWaxed".into(), Tag::Byte(0)));
                    }
                    // Un four : ses temps sont des entiers courts dans le jeu
                    (key @ ("BurnTime" | "CookTime"), value) => entries.push((key.into(), Tag::Short(value.as_i64().unwrap_or(0).clamp(0, 32_767) as i16))),
                    (key, value) => entries.push((key.into(), value)),
                }
            }
            if let Some((x, z)) = tile.pair() {
                // (dans le jeu, une des deux moitiés d'un coffre double mène l'autre)
                entries.push(("pairlead".into(), Tag::Byte(((pos.0, pos.2) < (x, z)) as i8)));
            }
            chunks.entry((pos.0 >> 4, pos.2 >> 4)).or_default().extend_from_slice(&nbt::write_le(&Tag::Compound(entries)));
        }
        self.dirty = false;
        chunks
    }

    /// Remet en place un bloc à données lu dans une sauvegarde : celle de Basalt (`tiles.nbt`)
    /// ou celle du jeu. Faux si l'entrée n'est pas reconnue.
    pub fn restore(&mut self, entry: &Tag, ctx: &mut dyn TileContext) -> bool {
        let coord = |key: &str| entry.get(key).and_then(|t| t.as_i64()).filter(|v| v.abs() < 40_000_000).map(|v| v as i32);
        let (pos, kind) = match (coord("x"), coord("y"), coord("z"), entry.str_of("id").and_then(type_from)) {
            (Some(x), Some(y), Some(z), Some(kind)) => ((x, y, z), kind),
            _ => return false,
        };
        let mut tile = match create(kind) {
            Some(t) => t,
            None => return false,
        };
        if let Some(inv) = tile.inventory_mut() {
            for item in entry.get("Items").and_then(|t| t.as_list()).unwrap_or(&[]) {
                let slot = item.get("Slot").and_then(|t| t.as_i64()).unwrap_or(-1);
                let count = item.get("Count").and_then(|t| t.as_i64()).unwrap_or(0);
                if let (Some(id), 0..=255, 1..=64) = (item.str_of("Name").and_then(|n| ctx.id_of(n)), slot, count) {
                    let max = inv.max_stack();
                    let extra = |key: &str| item.get(key).and_then(|t| t.as_i64()).unwrap_or(0);
                    // Usure et enchantements : dans `tag` (format du jeu), ou à plat (tiles.nbt)
                    let tag = item.get("tag");
                    let wear = tag.and_then(|t| t.get("Damage")).and_then(|t| t.as_i64()).unwrap_or_else(|| extra("Wear"));
                    let mut ench = crate::item::Ench(extra("BasaltEnch") as u64);
                    for e in tag.and_then(|t| t.get("ench")).and_then(|t| t.as_list()).unwrap_or(&[]) {
                        if let (Some(id @ 0..=125), Some(level @ 1..=31)) = (e.get("id").and_then(|t| t.as_i64()), e.get("lvl").and_then(|t| t.as_i64())) {
                            ench = ench.with(id as u8, level as u8);
                        }
                    }
                    let stack = ItemStack { meta: extra("Damage").clamp(0, 32767) as u16, damage: wear.clamp(0, 65535) as u16, ench, ..ItemStack::new(id, (count as u8).min(max), ctx.next_net_id()) };
                    inv.set_item(slot as usize, stack);
                }
            }
        }
        if let Tag::Compound(entries) = entry {
            for (key, value) in entries {
                match key.as_str() {
                    "id" | "x" | "y" | "z" | "Items" => {}
                    // Panneau du jeu : le texte de sa face avant
                    "FrontText" => {
                        if let Some(text) = value.get("Text") {
                            tile.load_extra("Text", text);
                        }
                    }
                    "BurnTime" | "CookTime" | "pairx" | "pairz" => tile.load_extra(key, &Tag::Int(value.as_i64().unwrap_or(0) as i32)),
                    _ => tile.load_extra(key, value),
                }
            }
        }
        self.insert(pos, tile);
        true
    }

    /// Recharge. Renvoie le nombre de blocs porteurs de données (0 si aucun fichier).
    pub fn load(&mut self, dir: &Path, ctx: &mut dyn TileContext) -> Result<usize, String> {
        let doc = match nbt::load_file(&dir.join("tiles.nbt"))? {
            Some(d) => d,
            None => return Ok(0),
        };
        for entry in doc.get("Tiles").and_then(|t| t.as_list()).unwrap_or(&[]) {
            self.restore(entry, ctx);
        }
        self.dirty = false;
        Ok(self.tiles.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Names;
    impl TileContext for Names {
        fn name_of(&self, id: i16) -> Option<String> {
            Some(format!("minecraft:objet_{}", id))
        }
        fn id_of(&self, name: &str) -> Option<i16> {
            name.strip_prefix("minecraft:objet_")?.parse().ok()
        }
        fn next_net_id(&mut self) -> i32 {
            1
        }
    }

    /// Coffre, four et panneau écrits au format du jeu, puis relus : rien ne se perd.
    #[test]
    fn block_entities_round_trip_through_the_format_of_the_game() {
        let mut map = TileMap::new();
        let mut chest = create(TileType::Chest).unwrap();
        let sword = ItemStack { meta: 2, damage: 31, ench: crate::item::Ench(0).with(9, 5).with(17, 3), ..ItemStack::new(7, 1, 1) };
        chest.inventory_mut().unwrap().set_item(4, sword);
        chest.inventory_mut().unwrap().set_item(26, ItemStack::new(12, 64, 1));
        chest.set_pair(Some((-17, 40)));
        map.insert((-18, 70, 40), chest);
        let mut furnace = create(TileType::Furnace).unwrap();
        furnace.load_extra("BurnTime", &Tag::Int(900));
        furnace.load_extra("CookTime", &Tag::Int(120));
        map.insert((3, -10, 200), furnace);
        let mut sign = create(TileType::Sign).unwrap();
        sign.load_extra("Text", &Tag::string("Bonjour\nle monde"));
        map.insert((3, -9, 200), sign);
        let mut bed = create(TileType::Bed).unwrap();
        bed.load_extra("color", &Tag::Byte(11));
        map.insert((0, 64, 0), bed);

        let chunks = map.snapshot_chunks(&Names);
        assert_eq!(chunks.len(), 3, "rangés par chunk");
        // Tel que le jeu l'attend : usure et enchantements dans `tag`, texte dans `FrontText`
        let mut pos = 0;
        let first = nbt::read_le(&chunks[&(-2, 2)], &mut pos).unwrap();
        let item = &first.get("Items").and_then(|t| t.as_list()).unwrap()[0];
        assert_eq!((item.str_of("Name"), item.get("Damage"), item.get("tag").and_then(|t| t.get("Damage"))), (Some("minecraft:objet_7"), Some(&Tag::Short(2)), Some(&Tag::Int(31))));
        assert_eq!((first.get("pairx"), first.get("pairz"), first.get("pairlead")), (Some(&Tag::Int(-17)), Some(&Tag::Int(40)), Some(&Tag::Byte(1))));

        let mut back = TileMap::new();
        for data in chunks.values() {
            let mut pos = 0;
            while pos < data.len() {
                assert!(back.restore(&nbt::read_le(data, &mut pos).unwrap(), &mut Names));
            }
        }
        assert_eq!(back.len(), 4);
        let chest = back.get((-18, 70, 40)).unwrap();
        let items = chest.inventory().unwrap().contents();
        let found = items.iter().find(|(slot, _)| *slot == 4).unwrap().1;
        assert_eq!((found.id, found.meta, found.damage, found.ench.level(9), found.ench.level(17)), (7, 2, 31, 5, 3));
        assert_eq!((items.len(), chest.pair()), (2, Some((-17, 40))));
        let extras = |pos: Pos| back.get(pos).unwrap().save_extra();
        assert_eq!(extras((3, -10, 200)), vec![("BurnTime", Tag::Int(900)), ("CookTime", Tag::Int(120))]);
        assert_eq!(extras((3, -9, 200)), vec![("Text", Tag::string("Bonjour\nle monde"))]);
        assert_eq!(extras((0, 64, 0)), vec![("color", Tag::Byte(11))]);
    }
}
