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

/// Face à droite d'un joueur qui regarde vers `face` (un quart de tour dans le sens horaire).
pub fn right_of(face: u8) -> u8 {
    opposite(left_of(face))
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

/// Escalier lu dans un état de bloc : (face vers laquelle il monte, à l'envers, coin).
/// None si ce n'est pas un escalier, ou si cette version du jeu ne note pas les coins.
pub fn stair_of(data: &GameData, state: u32) -> Option<(u8, bool, String)> {
    if !data.block_name(state)?.ends_with("stairs") {
        return None;
    }
    let corner = match data.prop(state, "minecraft:corner")? {
        Prop::Str(s) => s,
        _ => return None,
    };
    let facing = match data.prop(state, "weirdo_direction")? {
        Prop::Int(0) => EAST,
        Prop::Int(1) => WEST,
        Prop::Int(2) => SOUTH,
        _ => NORTH,
    };
    let top = matches!(data.prop(state, "upside_down_bit"), Some(Prop::Int(1)));
    Some((facing, top, corner))
}

/// Forme d'un escalier d'après ses voisins : (face, coin). `at(dx, dz)` donne l'escalier
/// voisin (face, à l'envers) s'il y en a un.
///
/// Règle du jeu : un escalier perpendiculaire devant donne un coin extérieur, un escalier
/// perpendiculaire derrière un coin intérieur. Le jeu ne note que les coins « droits » : un
/// coin gauche s'écrit comme le coin droit de l'escalier tourné d'un quart de tour à gauche.
pub fn stair_shape(facing: u8, top: bool, at: &dyn Fn(i32, i32) -> Option<(u8, bool)>) -> (u8, &'static str) {
    let near = |face: u8| {
        let (dx, dz) = step(face);
        at(dx, dz)
    };
    let perpendicular = |g: u8| g != facing && g != opposite(facing);
    // Le coin ne se forme pas si un escalier identique prolonge déjà la rangée de ce côté
    let free = |face: u8| !matches!(near(face), Some((g, t)) if g == facing && t == top);
    if let Some((g, t)) = near(facing) {
        if t == top && perpendicular(g) && free(opposite(g)) {
            return if g == left_of(facing) { (left_of(facing), "outer_right") } else { (facing, "outer_right") };
        }
    }
    if let Some((g, t)) = near(opposite(facing)) {
        if t == top && perpendicular(g) && free(g) {
            return if g == left_of(facing) { (left_of(facing), "inner_right") } else { (facing, "inner_right") };
        }
    }
    (facing, "none")
}

/// État qu'un escalier doit prendre compte tenu de ses voisins (None : inchangé ou pas un escalier).
pub fn settled_stair(data: &GameData, state: u32, at: &dyn Fn(i32, i32) -> Option<(u8, bool)>) -> Option<u32> {
    let (facing, top, corner) = stair_of(data, state)?;
    // Un coin déjà formé a pu être noté tourné d'un quart de tour : les deux lectures sont essayées
    let candidates = if corner == "none" { vec![facing] } else { vec![facing, right_of(facing)] };
    let mut best = (facing, "none");
    for f in candidates {
        let shape = stair_shape(f, top, at);
        if shape.1 != "none" {
            best = shape;
            break;
        }
    }
    let direction = match best.0 {
        EAST => 0,
        WEST => 1,
        SOUTH => 2,
        _ => 3,
    };
    let new = data.with_props(state, &[("weirdo_direction", Prop::Int(direction)), ("minecraft:corner", Prop::Str(best.1.to_string()))])?;
    if new == state {
        None
    } else {
        Some(new)
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
            // Portillon : dans le sens du regard. Four, coffre… : tournés vers le joueur.
            // Porte : le jeu note sa direction avec un quart de tour d'écart (héritage de
            // l'ancien format) — regard vers le sud, direction « ouest ».
            "minecraft:cardinal_direction" => {
                let dir = if short.ends_with("door") && !short.ends_with("trapdoor") {
                    right_of(looks)
                } else if short.ends_with("fence_gate") {
                    looks
                } else {
                    opposite(looks)
                };
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
    fn stairs_form_corners_with_their_neighbours() {
        // Seul : droit
        assert_eq!(stair_shape(NORTH, false, &|_, _| None), (NORTH, "none"));
        // Escalier perpendiculaire devant (au nord), montant vers l'est : coin extérieur droit
        let front_east = |dx: i32, dz: i32| if (dx, dz) == (0, -1) { Some((EAST, false)) } else { None };
        assert_eq!(stair_shape(NORTH, false, &front_east), (NORTH, "outer_right"));
        // Le même montant vers l'ouest : coin extérieur gauche, noté tourné d'un quart de tour
        let front_west = |dx: i32, dz: i32| if (dx, dz) == (0, -1) { Some((WEST, false)) } else { None };
        assert_eq!(stair_shape(NORTH, false, &front_west), (WEST, "outer_right"));
        // Escalier perpendiculaire derrière (au sud) : coin intérieur
        let back_east = |dx: i32, dz: i32| if (dx, dz) == (0, 1) { Some((EAST, false)) } else { None };
        assert_eq!(stair_shape(NORTH, false, &back_east), (NORTH, "inner_right"));
        let back_west = |dx: i32, dz: i32| if (dx, dz) == (0, 1) { Some((WEST, false)) } else { None };
        assert_eq!(stair_shape(NORTH, false, &back_west), (WEST, "inner_right"));
        // Pas de coin avec un escalier à l'envers, ni avec un escalier dans le même axe
        let upside_down = |dx: i32, dz: i32| if (dx, dz) == (0, -1) { Some((EAST, true)) } else { None };
        assert_eq!(stair_shape(NORTH, false, &upside_down), (NORTH, "none"));
        let same_axis = |dx: i32, dz: i32| if (dx, dz) == (0, -1) { Some((SOUTH, false)) } else { None };
        assert_eq!(stair_shape(NORTH, false, &same_axis), (NORTH, "none"));
        // Pas de coin si la rangée continue déjà de ce côté
        let row = |dx: i32, dz: i32| match (dx, dz) {
            (0, -1) => Some((EAST, false)),
            (-1, 0) => Some((NORTH, false)),
            _ => None,
        };
        assert_eq!(stair_shape(NORTH, false, &row), (NORTH, "none"));
    }

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
        assert_eq!(right_of(SOUTH), WEST);
        assert_eq!(right_of(WEST), NORTH);
        assert_eq!(left_of(left_of(left_of(left_of(NORTH)))), NORTH);
    }
}
