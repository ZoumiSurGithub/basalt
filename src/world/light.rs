//! Lumière, calculée à la demande.
//!
//! Le jeu calcule lui-même la lumière qu'il affiche : le serveur n'a rien à lui envoyer.
//! Basalt n'en a besoin que pour ses propres règles (où un monstre peut apparaître), donc
//! rien n'est stocké dans les tronçons : on répond pour **un** endroit, au moment où la
//! question se pose.
//!
//! Le modèle est celui du jeu :
//! - une source (torche, lave…) éclaire à son niveau, et la lumière perd 1 par bloc
//!   parcouru (2 dans l'eau) ; un bloc opaque l'arrête ;
//! - le ciel éclaire à 15 tout ce qui n'a rien au-dessus de soi, et cette lumière se répand
//!   ensuite de la même façon.
//!
//! Plutôt que de partir de toutes les sources, on part de l'endroit demandé et on remonte
//! vers elles : au-delà de 14 blocs de chemin, plus rien ne peut l'éclairer.

/// Ce qu'un bloc fait à la lumière.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Lumière émise, de 0 à 15
    pub emit: u8,
    /// Lumière retenue : 0 (air, verre), 1 (feuilles), 2 (eau), 15 (opaque)
    pub damp: u8,
}

impl Cell {
    pub const CLEAR: Cell = Cell { emit: 0, damp: 0 };
    pub const OPAQUE: Cell = Cell { emit: 0, damp: 15 };
}

/// Lumière reçue à un endroit, de 0 à 15.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Light {
    /// Venue des blocs lumineux
    pub block: u8,
    /// Venue du ciel, en plein jour
    pub sky: u8,
}

impl Light {
    /// Lumière du ciel à cette heure : la nuit lui retire 11 niveaux, comme dans le jeu.
    pub fn sky_now(&self, night: bool) -> u8 {
        self.sky.saturating_sub(if night { 11 } else { 0 })
    }

    /// Ce que l'on voit réellement : le plus fort des deux.
    pub fn level(&self, night: bool) -> u8 {
        self.block.max(self.sky_now(night))
    }
}

const REACH: i32 = 15;
const SIDE: usize = (2 * REACH + 1) as usize;

/// Lumière à `pos`. `cell` décrit le bloc d'une position ; `top` donne, pour une colonne,
/// la hauteur du plus haut bloc qui retient la lumière (tout ce qui est au-dessus voit le ciel).
pub fn light_at(pos: (i32, i32, i32), cell: &dyn Fn(i32, i32, i32) -> Cell, top: &dyn Fn(i32, i32) -> i32) -> Light {
    // Distance (en lumière perdue) depuis `pos`, 255 = pas encore vu
    let mut dist = vec![255u8; SIDE * SIDE * SIDE];
    let index = |p: (i32, i32, i32)| -> Option<usize> {
        let (dx, dy, dz) = (p.0 - pos.0 + REACH, p.1 - pos.1 + REACH, p.2 - pos.2 + REACH);
        let inside = |v: i32| (0..SIDE as i32).contains(&v);
        (inside(dx) && inside(dy) && inside(dz)).then(|| (dx as usize * SIDE + dy as usize) * SIDE + dz as usize)
    };
    let mut tops: std::collections::HashMap<(i32, i32), i32> = std::collections::HashMap::new();
    let mut rings: Vec<Vec<(i32, i32, i32)>> = vec![Vec::new(); REACH as usize];
    let mut light = Light { block: 0, sky: 0 };
    dist[index(pos).unwrap()] = 0;
    rings[0].push(pos);
    for d in 0..REACH as u8 {
        // Rien de plus loin ne peut faire mieux
        let best = 15 - d;
        if light.block >= best && light.sky >= best {
            break;
        }
        let ring = std::mem::take(&mut rings[d as usize]);
        for p in ring {
            if dist[index(p).unwrap()] != d {
                continue;
            }
            let here = cell(p.0, p.1, p.2);
            light.block = light.block.max(here.emit.saturating_sub(d));
            // (la colonne n'est sondée que si le ciel peut encore faire mieux ici)
            if light.sky < best && p.1 > *tops.entry((p.0, p.2)).or_insert_with(|| top(p.0, p.2)) {
                light.sky = light.sky.max(best);
            }
            // La lumière qui arrive ici d'un voisin paie l'entrée dans ce bloc
            if here.damp >= 15 {
                continue;
            }
            let next = d + here.damp.max(1);
            if next >= REACH as u8 {
                continue;
            }
            for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let n = (p.0 + dx, p.1 + dy, p.2 + dz);
                if let Some(i) = index(n) {
                    if dist[i] > next {
                        dist[i] = next;
                        rings[next as usize].push(n);
                    }
                }
            }
        }
    }
    light
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Petit monde d'essai : de la pierre jusqu'à `ground` compris, de l'air au-dessus,
    /// et quelques blocs posés à la main.
    struct Land {
        ground: i32,
        set: HashMap<(i32, i32, i32), Cell>,
    }

    impl Land {
        fn cell(&self, x: i32, y: i32, z: i32) -> Cell {
            *self.set.get(&(x, y, z)).unwrap_or(if y <= self.ground { &Cell::OPAQUE } else { &Cell::CLEAR })
        }
        fn top(&self, x: i32, z: i32) -> i32 {
            (-64..320).rev().find(|y| self.cell(x, *y, z).damp > 0).unwrap_or(-65)
        }
        fn light(&self, pos: (i32, i32, i32)) -> Light {
            light_at(pos, &|x, y, z| self.cell(x, y, z), &|x, z| self.top(x, z))
        }
    }

    const TORCH: Cell = Cell { emit: 14, damp: 0 };
    const WATER: Cell = Cell { emit: 0, damp: 2 };
    const LEAVES: Cell = Cell { emit: 0, damp: 1 };
    const LAMP: Cell = Cell { emit: 15, damp: 15 };

    #[test]
    fn open_ground_sees_the_whole_sky() {
        let land = Land { ground: 63, set: HashMap::new() };
        assert_eq!(land.light((0, 64, 0)), Light { block: 0, sky: 15 });
        assert_eq!(land.light((0, 64, 0)).level(false), 15);
        // La nuit, il reste 4 : assez sombre pour les monstres
        assert_eq!(land.light((0, 64, 0)).level(true), 4);
    }

    #[test]
    fn a_torch_fades_by_one_per_block_and_walls_stop_it() {
        // Une galerie d'un bloc de haut, creusée sous terre le long de x
        let mut land = Land { ground: 63, set: HashMap::new() };
        for x in -20..=20 {
            land.set.insert((x, 30, 0), Cell::CLEAR);
        }
        land.set.insert((0, 30, 0), TORCH);
        assert_eq!(land.light((0, 30, 0)), Light { block: 14, sky: 0 });
        assert_eq!(land.light((5, 30, 0)).block, 9);
        assert_eq!(land.light((13, 30, 0)).block, 1);
        assert_eq!(land.light((14, 30, 0)).block, 0);
        // Un mur dans la galerie : de l'autre côté, il fait noir
        land.set.insert((3, 30, 0), Cell::OPAQUE);
        assert_eq!(land.light((2, 30, 0)).block, 12);
        assert_eq!(land.light((4, 30, 0)).block, 0);
    }

    #[test]
    fn light_goes_around_corners() {
        // Galerie en L : 4 blocs le long de x, puis le long de z
        let mut land = Land { ground: 63, set: HashMap::new() };
        for i in 0..=4 {
            land.set.insert((i, 30, 0), Cell::CLEAR);
            land.set.insert((4, 30, i), Cell::CLEAR);
        }
        land.set.insert((0, 30, 0), TORCH);
        // 4 blocs puis 4 blocs : 8 de chemin, et non 5,6 à vol d'oiseau
        assert_eq!(land.light((4, 30, 4)).block, 6);
    }

    #[test]
    fn an_opaque_lamp_lights_its_neighbours() {
        let mut land = Land { ground: 63, set: HashMap::new() };
        land.set.insert((0, 30, 0), LAMP);
        land.set.insert((1, 30, 0), Cell::CLEAR);
        land.set.insert((2, 30, 0), Cell::CLEAR);
        assert_eq!(land.light((1, 30, 0)).block, 14);
        assert_eq!(land.light((2, 30, 0)).block, 13);
    }

    #[test]
    fn the_sky_reaches_under_a_roof_from_the_sides() {
        // Un toit de 9 × 9 posé à 3 blocs du sol
        let mut land = Land { ground: 63, set: HashMap::new() };
        for x in -4..=4 {
            for z in -4..=4 {
                land.set.insert((x, 67, z), Cell::OPAQUE);
            }
        }
        // Au bord, un pas suffit ; au centre, il en faut cinq
        assert_eq!(land.light((4, 64, 0)).sky, 14);
        assert_eq!(land.light((0, 64, 0)).sky, 10);
    }

    #[test]
    fn a_closed_cave_is_dark_and_a_shaft_lets_the_sky_in() {
        let mut land = Land { ground: 63, set: HashMap::new() };
        for x in 0..=6 {
            land.set.insert((x, 40, 0), Cell::CLEAR);
        }
        assert_eq!(land.light((0, 40, 0)), Light { block: 0, sky: 0 });
        // Un puits jusqu'à la surface, au bout de la galerie
        for y in 41..=63 {
            land.set.insert((6, y, 0), Cell::CLEAR);
        }
        assert_eq!(land.light((6, 40, 0)).sky, 15);
        assert_eq!(land.light((0, 40, 0)).sky, 9);
    }

    #[test]
    fn water_and_leaves_dim_the_sky() {
        let mut land = Land { ground: 59, set: HashMap::new() };
        // Un bassin de 4 blocs de profondeur (y 60 à 63), très large
        for x in -16..=16 {
            for z in -16..=16 {
                for y in 60..=63 {
                    land.set.insert((x, y, z), WATER);
                }
            }
        }
        assert_eq!(land.light((0, 63, 0)).sky, 13);
        assert_eq!(land.light((0, 60, 0)).sky, 7);
        // Sous un feuillage épais et large, on perd 1 par étage de feuilles
        let mut wood = Land { ground: 63, set: HashMap::new() };
        for x in -16..=16 {
            for z in -16..=16 {
                wood.set.insert((x, 70, z), LEAVES);
                wood.set.insert((x, 71, z), LEAVES);
            }
        }
        assert_eq!(wood.light((0, 69, 0)).sky, 12);
        assert_eq!(wood.light((0, 64, 0)).sky, 7);
    }
}
