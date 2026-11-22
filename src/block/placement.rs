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

/// Face opposée (le bit faible distingue les deux faces d'un même axe).
pub fn opposite(face: u8) -> u8 {
    face ^ 1
}

/// Nom d'une face, pour les messages.
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

/// Portillon qu'on ouvre : il pivote toujours en s'éloignant du joueur. S'il est tourné
/// vers lui, il se retourne (même axe, sens opposé) avant de s'ouvrir.
pub fn gate_away(data: &GameData, state: u32, looks: u8) -> u32 {
    let gate = data.block_name(state).map(|n| n.ends_with("fence_gate")).unwrap_or(false);
    if !gate || data.prop(state, "open_bit") != Some(Prop::Int(1)) {
        return state;
    }
    match data.prop(state, "minecraft:cardinal_direction") {
        Some(Prop::Str(dir)) if dir == word(opposite(looks)) => data.with_props(state, &[("minecraft:cardinal_direction", Prop::Str(word(looks).to_string()))]).unwrap_or(state),
        _ => state,
    }
}

/// Vrai pour un bloc en deux moitiés superposées (porte).
pub fn is_door(data: &GameData, state: u32) -> bool {
    data.prop(state, "upper_block_bit").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_opens_away_from_the_player() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = GameData::load(&dir).unwrap();
        let gate = data.default_state("minecraft:fence_gate").unwrap();
        let north = data.with_props(gate, &[("minecraft:cardinal_direction", Prop::Str("north".into())), ("open_bit", Prop::Int(1))]).unwrap();
        // Joueur qui regarde vers le sud, portillon tourné vers le nord : il se retourne
        let turned = gate_away(&data, north, SOUTH);
        assert_eq!(data.prop(turned, "minecraft:cardinal_direction"), Some(Prop::Str("south".into())));
        assert_eq!(data.prop(turned, "open_bit"), Some(Prop::Int(1)));
        // Même sens, ou de côté : inchangé ; portillon fermé : inchangé
        assert_eq!(gate_away(&data, north, NORTH), north);
        assert_eq!(gate_away(&data, north, EAST), north);
        let closed = data.with_props(north, &[("open_bit", Prop::Int(0))]).unwrap();
        assert_eq!(gate_away(&data, closed, SOUTH), closed);
    }

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

/// Double dalle correspondant à une dalle simple (`oak_slab` → `oak_double_slab`,
/// `cut_copper_slab` → `double_cut_copper_slab`). `None` pour tout autre bloc.
pub fn double_slab_of(data: &GameData, slab: u32) -> Option<u32> {
    let name = data.block_name(slab)?;
    if !name.ends_with("_slab") || name.contains("double") {
        return None;
    }
    let double = if name.ends_with("cut_copper_slab") {
        name.replacen("cut_copper_slab", "double_cut_copper_slab", 1)
    } else {
        format!("{}_double_slab", name.trim_end_matches("_slab"))
    };
    data.default_state(&double)
}

// --- Barrières, murets, vitres et barreaux : liaisons avec les voisins ------------------

/// Sorte de bloc qui se relie à ses voisins.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Link {
    /// Barrière ; `true` pour celle en briques du Nether (qui ne se lie qu'aux siennes)
    Fence(bool),
    Wall,
    /// Vitre ou barreaux de fer
    Pane,
}

/// Le bloc se relie-t-il à ses voisins, et comment ?
pub fn link_of(data: &GameData, state: u32) -> Option<Link> {
    let name = data.block_name(state)?;
    if data.prop(state, "wall_post_bit").is_some() {
        return Some(Link::Wall);
    }
    data.prop(state, "minecraft:connection_east")?;
    Some(if name.contains("fence") { Link::Fence(name.contains("nether_brick")) } else { Link::Pane })
}

/// Bloc plein, contre lequel une barrière ou un muret s'appuie. Les blocs qui ne remplissent
/// pas leur case (escaliers, dalles, portes, plantes, torches…) n'en sont pas.
pub fn is_full_block(data: &GameData, state: u32) -> bool {
    let name = match data.block_name(state) {
        Some(n) => n.trim_start_matches("minecraft:"),
        None => return false,
    };
    if name.contains("air") || name.contains("water") || name.contains("lava") {
        return false;
    }
    const PARTIAL: &[&str] = &[
        "stairs", "slab", "fence", "wall", "pane", "bars", "door", "sign", "torch", "lantern", "carpet", "button", "lever", "plate", "rail", "bed",
        "chest", "banner", "pot", "head", "skull", "candle", "cake", "ladder", "vine", "chain", "anvil", "cauldron", "hopper", "brewing", "enchanting",
        "lectern", "grindstone", "bell", "campfire", "scaffolding", "snow_layer", "leaves", "cactus", "web", "sapling", "coral", "kelp", "seagrass",
        "pickle", "cluster", "bud", "dripstone", "azalea", "mushroom", "flower", "grass", "fern", "bush", "petals", "litter", "lily", "roots",
        "sprouts", "wart", "crop", "stem", "poppy", "dandelion", "allium", "bluet", "tulip", "daisy", "cornflower", "rose", "torchflower",
        "pitcher", "peony", "lilac", "sunflower", "eyeblossom", "deadbush", "hanging", "spore", "frogspawn", "tripwire", "redstone_wire",
        "repeater", "comparator", "daylight", "flower_pot", "end_portal", "nether_portal", "portal", "string",
    ];
    if PARTIAL.iter().any(|p| name.contains(p)) {
        return name.ends_with("mushroom_block") || name == "mushroom_stem" || name == "grass_block";
    }
    !matches!(name, "wheat" | "carrots" | "potatoes" | "beetroot" | "sugar_cane" | "farmland" | "grass_path" | "end_rod" | "chorus_plant" | "chorus_flower" | "conduit" | "beacon")
}

/// L'état d'un bloc qui se relie, d'après ses voisins. `at(dx, dy, dz)` donne l'état d'un
/// voisin. None si le bloc ne se relie pas ou que son état ne change pas.
pub fn linked_state(data: &GameData, state: u32, at: &dyn Fn(i32, i32, i32) -> u32) -> Option<u32> {
    let link = link_of(data, state)?;
    let gate = |s: u32| data.block_name(s).map(|n| n.contains("fence_gate")).unwrap_or(false);
    let joins = |s: u32| -> bool {
        let other = link_of(data, s);
        match link {
            Link::Fence(nether) => other == Some(Link::Fence(nether)) || gate(s) || is_full_block(data, s),
            Link::Wall | Link::Pane => matches!(other, Some(Link::Wall) | Some(Link::Pane)) || gate(s) || is_full_block(data, s),
        }
    };
    // Est, nord, sud, ouest
    let sides = [("east", (1, 0)), ("north", (0, -1)), ("south", (0, 1)), ("west", (-1, 0))];
    let linked: Vec<bool> = sides.iter().map(|(_, (dx, dz))| joins(at(*dx, 0, *dz))).collect();
    let new = match link {
        Link::Fence(_) | Link::Pane => {
            let changes: Vec<(String, Prop)> = sides.iter().zip(&linked).map(|((side, _), on)| (format!("minecraft:connection_{}", side), Prop::Int(*on as i32))).collect();
            let refs: Vec<(&str, Prop)> = changes.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            data.with_props(state, &refs)?
        }
        Link::Wall => {
            let above = at(0, 1, 0);
            let covered = is_full_block(data, above);
            let height = if covered { "tall" } else { "short" };
            // Poteau, sauf pour un muret droit (deux côtés opposés seulement) dégagé au-dessus
            let straight = (linked[0] && linked[3] && !linked[1] && !linked[2]) || (linked[1] && linked[2] && !linked[0] && !linked[3]);
            let above_wall = link_of(data, above) == Some(Link::Wall);
            let post = !straight || above_wall;
            let mut changes: Vec<(String, Prop)> =
                sides.iter().zip(&linked).map(|((side, _), on)| (format!("wall_connection_type_{}", side), Prop::Str(if *on { height } else { "none" }.to_string()))).collect();
            changes.push(("wall_post_bit".to_string(), Prop::Int(post as i32)));
            let refs: Vec<(&str, Prop)> = changes.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            data.with_props(state, &refs)?
        }
    };
    (new != state).then_some(new)
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn data() -> Option<GameData> {
        GameData::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).ok()
    }

    /// Petit monde : des blocs posés par position, de l'air ailleurs.
    fn around<'a>(d: &'a GameData, blocks: &'a [((i32, i32, i32), &'a str)]) -> impl Fn((i32, i32, i32)) -> u32 + 'a {
        let air = d.default_state("minecraft:air").unwrap();
        move |p| blocks.iter().find(|(q, _)| *q == p).map(|(_, n)| d.state_from_spec(&format!("minecraft:{}", n)).unwrap()).unwrap_or(air)
    }

    fn settled(d: &GameData, blocks: &[((i32, i32, i32), &str)], at: (i32, i32, i32)) -> Vec<(String, String)> {
        let get = around(d, blocks);
        let here = get(at);
        let state = linked_state(d, here, &|dx, dy, dz| get((at.0 + dx, at.1 + dy, at.2 + dz))).unwrap_or(here);
        let mut keys = d.prop_keys(state);
        keys.sort();
        keys.into_iter().map(|k| {
            let v = match d.prop(state, &k) {
                Some(Prop::Int(i)) => i.to_string(),
                Some(Prop::Str(s)) => s,
                None => String::new(),
            };
            (k.trim_start_matches("minecraft:").to_string(), v)
        }).collect()
    }

    fn has(props: &[(String, String)], key: &str, value: &str) -> bool {
        props.iter().any(|(k, v)| k == key && v == value)
    }

    #[test]
    fn fences_walls_and_panes_link_to_their_neighbours() {
        let Some(d) = data() else { return };
        // Barrière entre une barrière (est) et un bloc de pierre (nord) ; rien au sud ni à l'ouest
        let p = settled(&d, &[((0, 0, 0), "oak_fence"), ((1, 0, 0), "spruce_fence"), ((0, 0, -1), "stone"), ((-1, 0, 0), "nether_brick_fence")], (0, 0, 0));
        assert!(has(&p, "connection_east", "1") && has(&p, "connection_north", "1"), "{:?}", p);
        assert!(has(&p, "connection_south", "0") && has(&p, "connection_west", "0"), "la barrière du Nether ne se lie pas au bois : {:?}", p);
        // Muret droit : pas de poteau ; avec un bloc posé dessus : côtés hauts
        let walls = [((0, 0, 0), "cobblestone_wall"), ((1, 0, 0), "cobblestone_wall"), ((-1, 0, 0), "stone")];
        let p = settled(&d, &walls, (0, 0, 0));
        assert!(has(&p, "wall_connection_type_east", "short") && has(&p, "wall_connection_type_west", "short") && has(&p, "wall_post_bit", "0"), "{:?}", p);
        let covered = [((0, 0, 0), "cobblestone_wall"), ((1, 0, 0), "cobblestone_wall"), ((0, 1, 0), "stone")];
        let p = settled(&d, &covered, (0, 0, 0));
        assert!(has(&p, "wall_connection_type_east", "tall") && has(&p, "wall_post_bit", "1"), "{:?}", p);
        // Muret en coin : poteau
        let corner = [((0, 0, 0), "cobblestone_wall"), ((1, 0, 0), "cobblestone_wall"), ((0, 0, 1), "cobblestone_wall")];
        assert!(has(&settled(&d, &corner, (0, 0, 0)), "wall_post_bit", "1"));
        // Vitre contre une vitre et contre un muret ; pas contre une barrière ni une fleur
        let p = settled(&d, &[((0, 0, 0), "glass_pane"), ((0, 0, 1), "glass_pane"), ((0, 0, -1), "cobblestone_wall"), ((1, 0, 0), "oak_fence"), ((-1, 0, 0), "poppy")], (0, 0, 0));
        assert!(has(&p, "connection_south", "1") && has(&p, "connection_north", "1") && has(&p, "connection_east", "0") && has(&p, "connection_west", "0"), "{:?}", p);
    }
}
