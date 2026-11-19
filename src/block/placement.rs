//! Orientation des blocs à la pose et changement d'état au clic (portes, leviers…), comme
//! dans le jeu. Tout se déduit des propriétés d'état que le bloc possède : aucune liste de
//! blocs à tenir à jour.

use crate::data::{GameData, Prop};

/// Faces d'un bloc, telles que le jeu les numérote.
pub const DOWN: u8 = 0;
pub const UP: u8 = 1;
pub const NORTH: u8 = 2;
pub const SOUTH: u8 = 3;
pub const WEST: u8 = 4;
pub const EAST: u8 = 5;

/// Circonstances d'une pose.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// Face cliquée du bloc voisin
    pub face: u8,
    /// Point cliqué sur cette face, de 0 à 1 sur chaque axe
    pub click: (f32, f32, f32),
    /// Face vers laquelle le joueur regarde (horizontalement)
    pub looks: u8,
}

/// Face vers laquelle le joueur regarde (horizontalement).
pub fn facing(yaw: f32) -> u8 {
    match ((yaw.rem_euclid(360.0) + 45.0) / 90.0) as i32 % 4 {
        0 => SOUTH,
        1 => WEST,
        2 => NORTH,
        _ => EAST,
    }
}

/// Face vers laquelle va un regard dont on connaît le déplacement horizontal (x vers
/// l'est, z vers le sud). None s'il est vertical.
pub fn facing_of(dx: f32, dz: f32) -> Option<u8> {
    if dx.abs() < 0.05 && dz.abs() < 0.05 {
        return None;
    }
    Some(if dx.abs() > dz.abs() {
        if dx > 0.0 {
            EAST
        } else {
            WEST
        }
    } else if dz > 0.0 {
        SOUTH
    } else {
        NORTH
    })
}

/// Face à gauche d'un joueur qui regarde vers `face`.
pub fn left_of(face: u8) -> u8 {
    match face {
        SOUTH => EAST,
        EAST => NORTH,
        NORTH => WEST,
        _ => SOUTH,
    }
}

/// Décalage (x, z) d'un pas vers une face horizontale.
pub fn step(face: u8) -> (i32, i32) {
    match face {
        NORTH => (0, -1),
        SOUTH => (0, 1),
        WEST => (-1, 0),
        _ => (1, 0),
    }
}

pub fn opposite(face: u8) -> u8 {
    face ^ 1
}

fn word(face: u8) -> &'static str {
    match face {
        DOWN => "down",
        UP => "up",
        NORTH => "north",
        SOUTH => "south",
        WEST => "west",
        _ => "east",
    }
}

/// État d'un bloc posé dans ces circonstances, à partir de son état par défaut.
pub fn placed_state(data: &GameData, default: u32, p: Placement) -> u32 {
    let name = data.block_name(default).unwrap_or("").to_string();
    let short = name.strip_prefix("minecraft:").unwrap_or(&name);
    let looks = p.looks;
    // Moitié haute : clic sous un bloc, ou dans la moitié haute d'un côté
    let top = p.face == DOWN || (p.face >= NORTH && p.click.1 > 0.5);
    let mut changes: Vec<(&str, Prop)> = Vec::new();
    for key in data.prop_keys(default) {
        match key.as_str() {
            // Porte, portillon : dans le sens du regard. Four, coffre… : tournés vers le joueur.
            "minecraft:cardinal_direction" => {
                let dir = if short.ends_with("door") || short.ends_with("fence_gate") { looks } else { opposite(looks) };
                changes.push(("minecraft:cardinal_direction", Prop::Str(word(dir).to_string())));
            }
            // Escalier : monte dans le sens du regard (0 est, 1 ouest, 2 sud, 3 nord)
            "weirdo_direction" => {
                let v = match looks {
                    EAST => 0,
                    WEST => 1,
                    SOUTH => 2,
                    _ => 3,
                };
                changes.push(("weirdo_direction", Prop::Int(v)));
            }
            "upside_down_bit" => changes.push(("upside_down_bit", Prop::Int(top as i32))),
            "minecraft:vertical_half" => changes.push(("minecraft:vertical_half", Prop::Str(if top { "top" } else { "bottom" }.to_string()))),
            // Bûche, pilier : couché selon la face cliquée
            "pillar_axis" => {
                let axis = match p.face {
                    DOWN | UP => "y",
                    NORTH | SOUTH => "z",
                    _ => "x",
                };
                changes.push(("pillar_axis", Prop::Str(axis.to_string())));
            }
            // Trappe : charnière du côté du joueur (0 est, 1 ouest, 2 sud, 3 nord)
            "direction" if short.ends_with("trapdoor") => {
                let v = match opposite(looks) {
                    EAST => 0,
                    WEST => 1,
                    SOUTH => 2,
                    _ => 3,
                };
                changes.push(("direction", Prop::Int(v)));
            }
            // Autres blocs à « direction » (0 sud, 1 ouest, 2 nord, 3 est) : face au joueur
            "direction" => {
                let v = match opposite(looks) {
                    SOUTH => 0,
                    WEST => 1,
                    NORTH => 2,
                    _ => 3,
                };
                changes.push(("direction", Prop::Int(v)));
            }
            // Échelle, bouton… : accrochés à la face cliquée
            "facing_direction" => changes.push(("facing_direction", Prop::Int(p.face as i32))),
            // Torche : le mot désigne le côté où se trouve le bloc qui la porte
            "torch_facing_direction" => {
                let v = match p.face {
                    NORTH => "south",
                    SOUTH => "north",
                    WEST => "east",
                    EAST => "west",
                    _ => "top",
                };
                changes.push(("torch_facing_direction", Prop::Str(v.to_string())));
            }
            "lever_direction" => {
                let along = if looks == NORTH || looks == SOUTH { "north_south" } else { "east_west" };
                let v = match p.face {
                    UP => format!("up_{}", along),
                    DOWN => format!("down_{}", along),
                    f => word(f).to_string(),
                };
                changes.push(("lever_direction", Prop::Str(v)));
            }
            _ => {}
        }
    }
    if changes.is_empty() {
        return default;
    }
    data.with_props(default, &changes).unwrap_or(default)
}

/// Nature du changement produit par un clic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Toggle {
    /// Porte, trappe, portillon, levier : reste dans son nouvel état
    Stay,
    /// Bouton : revient seul au repos
    Momentary,
}

/// État du bloc après un clic, s'il réagit au clic. Les portes et trappes en fer ne
/// s'ouvrent pas à la main.
pub fn toggled(data: &GameData, state: u32) -> Option<(u32, Toggle)> {
    let name = data.block_name(state)?;
    if name.contains("iron_") {
        return None;
    }
    if let Some(Prop::Int(open)) = data.prop(state, "open_bit") {
        let next = data.with_props(state, &[("open_bit", Prop::Int((open == 0) as i32))])?;
        return Some((next, Toggle::Stay));
    }
    if let Some(Prop::Int(0)) = data.prop(state, "button_pressed_bit") {
        let next = data.with_props(state, &[("button_pressed_bit", Prop::Int(1))])?;
        return Some((next, Toggle::Momentary));
    }
    None
}

/// Vrai pour un bloc en deux moitiés superposées (porte).
pub fn is_door(data: &GameData, state: u32) -> bool {
    data.prop(state, "upper_block_bit").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_looks_where_the_yaw_says() {
        assert_eq!(facing(0.0), SOUTH);
        assert_eq!(facing(90.0), WEST);
        assert_eq!(facing(-180.0), NORTH);
        assert_eq!(facing(270.0), EAST);
        assert_eq!(facing(-44.0), SOUTH);
        assert_eq!(opposite(NORTH), SOUTH);
        assert_eq!(opposite(EAST), WEST);
        assert_eq!(facing_of(3.0, 0.5), Some(EAST));
        assert_eq!(facing_of(-0.2, -2.0), Some(NORTH));
        assert_eq!(facing_of(0.0, 0.01), None);
        assert_eq!(left_of(SOUTH), EAST);
        assert_eq!(left_of(left_of(left_of(left_of(NORTH)))), NORTH);
    }
}
