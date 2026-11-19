//! Orientation et changement d'état des blocs, vérifiés sur les vraies données du jeu.

use basalt::block::placement::{self, Placement, Toggle};
use basalt::data::{GameData, Prop};

fn data() -> GameData {
    GameData::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))).unwrap()
}

fn place(d: &GameData, block: &str, face: u8, click_y: f32, yaw: f32) -> u32 {
    let default = d.default_state(block).unwrap_or_else(|| panic!("bloc inconnu : {}", block));
    placement::placed_state(d, default, Placement { face, click: (0.5, click_y, 0.5), looks: placement::facing(yaw) })
}

#[test]
fn blocks_are_oriented_like_in_the_game() {
    let d = data();
    let s = |v: &str| Some(Prop::Str(v.to_string()));
    // Le joueur regarde au nord (yaw 180) : le four lui fait face, donc tourné vers le sud
    let furnace = place(&d, "minecraft:furnace", placement::UP, 1.0, 180.0);
    assert_eq!(d.prop(furnace, "minecraft:cardinal_direction"), s("south"));
    let chest = place(&d, "minecraft:chest", placement::UP, 1.0, 90.0);
    assert_eq!(d.prop(chest, "minecraft:cardinal_direction"), s("east"));
    // Bûche couchée selon la face cliquée
    assert_eq!(d.prop(place(&d, "minecraft:oak_log", placement::UP, 1.0, 0.0), "pillar_axis"), s("y"));
    assert_eq!(d.prop(place(&d, "minecraft:oak_log", placement::NORTH, 0.5, 0.0), "pillar_axis"), s("z"));
    assert_eq!(d.prop(place(&d, "minecraft:oak_log", placement::EAST, 0.5, 0.0), "pillar_axis"), s("x"));
    // Dalle haute sous un bloc ou dans la moitié haute d'un côté
    assert_eq!(d.prop(place(&d, "minecraft:oak_slab", placement::UP, 1.0, 0.0), "minecraft:vertical_half"), s("bottom"));
    assert_eq!(d.prop(place(&d, "minecraft:oak_slab", placement::DOWN, 0.0, 0.0), "minecraft:vertical_half"), s("top"));
    assert_eq!(d.prop(place(&d, "minecraft:oak_slab", placement::WEST, 0.8, 0.0), "minecraft:vertical_half"), s("top"));
    // Escalier : monte vers l'est quand le joueur regarde à l'est
    let stairs = place(&d, "minecraft:oak_stairs", placement::UP, 1.0, 270.0);
    assert_eq!(d.prop(stairs, "weirdo_direction"), Some(Prop::Int(0)));
    assert_eq!(d.prop(stairs, "upside_down_bit"), Some(Prop::Int(0)));
    // Torche, échelle, levier accrochés à la face cliquée
    assert_eq!(d.prop(place(&d, "minecraft:torch", placement::UP, 1.0, 0.0), "torch_facing_direction"), s("top"));
    assert_eq!(d.prop(place(&d, "minecraft:torch", placement::NORTH, 0.5, 0.0), "torch_facing_direction"), s("south"));
    assert_eq!(d.prop(place(&d, "minecraft:ladder", placement::WEST, 0.5, 0.0), "facing_direction"), Some(Prop::Int(4)));
    assert_eq!(d.prop(place(&d, "minecraft:lever", placement::UP, 1.0, 0.0), "lever_direction"), s("up_north_south"));
    assert_eq!(d.prop(place(&d, "minecraft:lever", placement::EAST, 0.5, 0.0), "lever_direction"), s("east"));
    // Un bloc sans orientation reste tel quel
    let stone = d.default_state("minecraft:stone").unwrap();
    assert_eq!(placement::placed_state(&d, stone, Placement { face: 3, click: (0.2, 0.9, 0.1), looks: placement::WEST }), stone);
}

#[test]
fn doors_levers_and_buttons_react_to_a_click() {
    let d = data();
    let door = place(&d, "minecraft:wooden_door", placement::UP, 1.0, 0.0);
    assert!(placement::is_door(&d, door));
    let upper = d.with_props(door, &[("upper_block_bit", Prop::Int(1))]).unwrap();
    assert_ne!(upper, door);
    let (open, kind) = placement::toggled(&d, door).unwrap();
    assert_eq!(kind, Toggle::Stay);
    assert_eq!(d.prop(open, "open_bit"), Some(Prop::Int(1)));
    // Tout le reste de l'état est conservé, et un second clic referme
    assert_eq!(d.prop(open, "minecraft:cardinal_direction"), d.prop(door, "minecraft:cardinal_direction"));
    assert_eq!(placement::toggled(&d, open).unwrap().0, door);
    for name in ["minecraft:trapdoor", "minecraft:fence_gate", "minecraft:lever"] {
        let s = d.default_state(name).unwrap();
        assert!(placement::toggled(&d, s).is_some(), "{}", name);
    }
    let button = d.default_state("minecraft:wooden_button").unwrap();
    let (pressed, kind) = placement::toggled(&d, button).unwrap();
    assert_eq!(kind, Toggle::Momentary);
    assert!(placement::toggled(&d, pressed).is_none(), "un bouton enfoncé ne réagit plus");
    // Le fer ne s'ouvre pas à la main, et la pierre ne réagit pas
    assert!(placement::toggled(&d, d.default_state("minecraft:iron_door").unwrap()).is_none());
    assert!(placement::toggled(&d, d.default_state("minecraft:stone").unwrap()).is_none());
}
