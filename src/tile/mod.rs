//! Données attachées aux blocs (« tiles » dans PocketMine et Altay) : le contenu d'un coffre,
//! l'état d'un four, le texte d'un panneau.
//!
//! - `Tile` : la base de toutes ces données ;
//! - `ContainerTile` (coffre, tonneau, boîte de Shulker, entonnoir, distributeur…),
//!   `FurnaceTile` (four, haut fourneau, fumoir), `SignTile`, `GenericTile` : bâties dessus ;
//! - `TileMap` : les données de tous les blocs du monde, avec leur sauvegarde.

mod furnace;
mod tile;
mod tile_map;

pub use self::furnace::{fuel_time, smelt_result, FurnaceTile};
pub use self::tile::{create, ContainerTile, GenericTile, SignTile, Tile, TileContext};
pub use self::tile_map::TileMap;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::TileType;
    use crate::item::ItemStack;

    /// Faux catalogue d'objets pour les tests : quelques noms, des numéros à la suite.
    struct Names {
        next: i32,
    }
    const KNOWN: [&str; 8] = ["raw_iron", "iron_ingot", "coal", "oak_planks", "beef", "cooked_beef", "stick", "sand"];

    impl TileContext for Names {
        fn name_of(&self, id: i16) -> Option<String> {
            KNOWN.get(id as usize - 1).map(|n| format!("minecraft:{}", n))
        }
        fn id_of(&self, name: &str) -> Option<i16> {
            let short = name.strip_prefix("minecraft:").unwrap_or(name);
            KNOWN.iter().position(|n| *n == short).map(|i| i as i16 + 1)
        }
        fn next_net_id(&mut self) -> i32 {
            self.next += 1;
            self.next
        }
    }

    fn id(name: &str) -> i16 {
        Names { next: 0 }.id_of(name).unwrap()
    }

    #[test]
    fn a_furnace_smelts_while_it_has_fuel() {
        let mut ctx = Names { next: 100 };
        let mut furnace = FurnaceTile::new(TileType::Furnace);
        {
            let inv = furnace.inventory_mut().unwrap();
            inv.set_item(0, ItemStack::new(id("raw_iron"), 3, 1));
            inv.set_item(1, ItemStack::new(id("coal"), 1, 2));
        }
        // Un charbon brûle 1600 ticks, une cuisson en demande 200 : 8 lingots au plus, 3 ici
        for _ in 0..199 {
            furnace.tick(&mut ctx);
        }
        assert!(furnace.is_burning());
        assert!(furnace.inventory().unwrap().get_item(2).unwrap().is_empty());
        furnace.tick(&mut ctx);
        let out = furnace.inventory().unwrap().get_item(2).unwrap();
        assert_eq!((out.id, out.count), (id("iron_ingot"), 1));
        for _ in 0..400 {
            furnace.tick(&mut ctx);
        }
        let inv = furnace.inventory().unwrap();
        assert_eq!(inv.get_item(2).unwrap().count, 3);
        assert!(inv.get_item(0).unwrap().is_empty() && inv.get_item(1).unwrap().is_empty());
        // Le feu finit par s'éteindre, et rien ne se passe sans combustible
        for _ in 0..1600 {
            furnace.tick(&mut ctx);
        }
        assert!(!furnace.is_burning());
        furnace.inventory_mut().unwrap().set_item(0, ItemStack::new(id("sand"), 1, 9));
        for _ in 0..400 {
            furnace.tick(&mut ctx);
        }
        assert_eq!(furnace.inventory().unwrap().get_item(0).unwrap().count, 1);
        // Un fumoir cuit deux fois plus vite, mais seulement de la nourriture
        let mut smoker = FurnaceTile::new(TileType::Smoker);
        smoker.inventory_mut().unwrap().set_item(0, ItemStack::new(id("beef"), 1, 1));
        smoker.inventory_mut().unwrap().set_item(1, ItemStack::new(id("oak_planks"), 1, 2));
        for _ in 0..100 {
            smoker.tick(&mut ctx);
        }
        assert_eq!(smoker.inventory().unwrap().get_item(2).unwrap().id, id("cooked_beef"));
        let mut wrong = FurnaceTile::new(TileType::Smoker);
        wrong.inventory_mut().unwrap().set_item(0, ItemStack::new(id("raw_iron"), 1, 1));
        wrong.inventory_mut().unwrap().set_item(1, ItemStack::new(id("coal"), 1, 2));
        for _ in 0..300 {
            wrong.tick(&mut ctx);
        }
        assert!(wrong.inventory().unwrap().get_item(2).unwrap().is_empty());
        assert_eq!(wrong.inventory().unwrap().get_item(1).unwrap().count, 1, "le combustible n'est pas gaspillé");
    }

    #[test]
    fn tiles_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("basalt-tiles-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut ctx = Names { next: 0 };
        let mut map = TileMap::new();
        assert_eq!(map.load(&dir, &mut ctx).unwrap(), 0);

        let mut chest = create(TileType::Chest).unwrap();
        assert_eq!(chest.inventory().unwrap().size(), 27);
        chest.inventory_mut().unwrap().set_item(5, ItemStack::new(id("stick"), 12, 1));
        map.insert((1, -60, 2), chest);
        let mut furnace = FurnaceTile::new(TileType::BlastFurnace);
        furnace.inventory_mut().unwrap().set_item(0, ItemStack::new(id("raw_iron"), 2, 2));
        furnace.inventory_mut().unwrap().set_item(1, ItemStack::new(id("coal"), 1, 3));
        for _ in 0..10 {
            furnace.tick(&mut ctx);
        }
        map.insert((-5, 70, 9), Box::new(furnace));
        let mut sign = SignTile::new();
        sign.set_text("Bienvenue\nchez = moi");
        map.insert((0, 0, 0), Box::new(sign));
        assert_eq!(create(TileType::Hopper).unwrap().inventory().unwrap().size(), 5);
        assert!(map.dirty);
        map.save(&dir, &ctx).unwrap();
        assert!(!map.dirty);

        let mut again = TileMap::new();
        assert_eq!(again.load(&dir, &mut ctx).unwrap(), 3);
        let chest = again.get((1, -60, 2)).unwrap();
        assert_eq!(chest.tile_type(), TileType::Chest);
        let item = chest.inventory().unwrap().get_item(5).unwrap();
        assert_eq!((item.id, item.count), (id("stick"), 12));
        let furnace = again.get((-5, 70, 9)).unwrap();
        assert_eq!(furnace.tile_type(), TileType::BlastFurnace);
        assert_eq!(furnace.save_extra(), map.get((-5, 70, 9)).unwrap().save_extra());
        assert_eq!(again.get((0, 0, 0)).unwrap().save_extra(), vec![("Text", crate::nbt::Tag::string("Bienvenue\nchez = moi"))]);
        assert!(dir.join("tiles.nbt").exists());
        // Retirer un bloc rend son contenu
        let removed = again.remove((1, -60, 2)).unwrap();
        assert_eq!(removed.inventory().unwrap().contents().len(), 1);
        assert!(again.get((1, -60, 2)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
