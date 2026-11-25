//! La carte vue du dessus, pour le tableau de bord : une couleur par colonne du monde.
//!
//! Basalt ne lit aucune texture pour ça : la couleur vient du nom du bloc. Le rendu se fait
//! hors du fil du serveur, sur une vue du monde ; seuls les chunks déjà en mémoire sont
//! dessinés, la carte ne fait donc générer aucun terrain.

use std::collections::HashMap;

use crate::data::GameData;
use crate::world::WorldView;

/// Les couleurs tirées des textures des packs (nom du bloc sans `minecraft:` → couleur),
/// calculées une fois sur un fil à part. `None` tant que ce n'est pas fini.
pub(super) type Colours = std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<super::textures::Look>>>>;

/// Attend (cinq secondes au plus) que les couleurs des textures soient prêtes.
pub(super) fn ready(shared: &Colours) -> std::sync::Arc<super::textures::Look> {
    for _ in 0..250 {
        if let Some(found) = shared.lock().ok().and_then(|g| g.clone()) {
            return found;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Default::default()
}

/// Couleur d'un bloc : celle de sa texture si un pack la donne, sinon celle de son nom.
fn colour_in(name: &str, real: &HashMap<String, [u8; 3]>) -> [u8; 3] {
    real.get(name.trim_start_matches("minecraft:")).copied().unwrap_or_else(|| colour(name))
}

/// Côté d'une tuile, en chunks
pub(super) const TILE: i32 = 4;

/// Couleur d'un bloc vu du dessus, d'après son nom.
pub(super) fn colour(name: &str) -> [u8; 3] {
    let n = name.trim_start_matches("minecraft:");
    const DYES: [(&str, [u8; 3]); 16] = [
        ("light_blue", [58, 175, 217]), ("light_gray", [142, 142, 134]), ("white", [233, 236, 236]), ("orange", [240, 118, 19]),
        ("magenta", [189, 68, 179]), ("yellow", [248, 197, 39]), ("lime", [112, 185, 25]), ("pink", [237, 141, 172]),
        ("gray", [62, 68, 71]), ("cyan", [21, 137, 145]), ("purple", [121, 42, 172]), ("blue", [53, 57, 157]),
        ("brown", [114, 71, 40]), ("green", [84, 109, 27]), ("red", [160, 39, 34]), ("black", [20, 21, 25]),
    ];
    if ["wool", "carpet", "concrete", "terracotta", "stained_glass", "shulker_box", "candle", "banner"].iter().any(|k| n.contains(k)) || n.ends_with("_bed") {
        if let Some((_, c)) = DYES.iter().find(|(dye, _)| n.starts_with(dye)) {
            return *c;
        }
    }
    // (du plus précis au plus vague : le premier mot trouvé dans le nom l'emporte)
    const WORDS: [(&str, [u8; 3]); 74] = [
        ("water", [52, 95, 190]), ("lava", [222, 96, 22]), ("magma", [150, 60, 20]), ("fire", [230, 130, 30]),
        ("snow", [240, 244, 247]), ("packed_ice", [140, 170, 225]), ("blue_ice", [110, 160, 235]), ("ice", [160, 190, 240]),
        ("grass_block", [104, 148, 70]), ("grass_path", [150, 126, 72]), ("dirt_path", [150, 126, 72]), ("farmland", [110, 72, 42]),
        ("podzol", [92, 64, 26]), ("mycelium", [112, 98, 112]), ("mud", [62, 57, 61]), ("dirt", [134, 96, 67]),
        ("red_sandstone", [182, 98, 32]), ("red_sand", [190, 103, 33]), ("sandstone", [216, 203, 156]), ("soul_sand", [82, 62, 50]),
        ("sand", [219, 207, 163]), ("gravel", [132, 128, 126]), ("clay", [160, 166, 179]),
        ("cherry_leaves", [232, 170, 200]), ("azalea_leaves", [100, 128, 46]), ("spruce_leaves", [60, 96, 60]), ("birch_leaves", [110, 150, 78]),
        ("leaves", [66, 122, 46]), ("cactus", [86, 128, 42]), ("moss", [90, 110, 44]), ("vine", [58, 104, 40]),
        ("lily", [34, 122, 46]), ("melon", [112, 146, 30]), ("pumpkin", [198, 118, 24]), ("hay", [166, 138, 36]),
        ("wheat", [176, 160, 70]), ("sugar_cane", [130, 168, 90]), ("bamboo", [94, 144, 30]), ("fern", [78, 122, 60]),
        ("grass", [96, 144, 64]), ("flower", [200, 190, 60]), ("dandelion", [240, 210, 50]), ("poppy", [200, 50, 40]), ("tulip", [220, 110, 110]),
        ("mushroom", [150, 110, 90]), ("sapling", [70, 120, 50]),
        ("dark_oak", [66, 43, 20]), ("spruce", [114, 84, 48]), ("birch", [192, 175, 121]), ("jungle", [160, 115, 80]),
        ("acacia", [168, 90, 50]), ("mangrove", [117, 54, 48]), ("cherry", [226, 178, 172]), ("crimson", [101, 48, 70]), ("warped", [43, 104, 99]),
        ("oak", [162, 130, 78]), ("planks", [162, 130, 78]), ("log", [102, 81, 50]), ("wood", [102, 81, 50]),
        ("bookshelf", [120, 92, 54]), ("chest", [150, 108, 42]), ("crafting", [130, 96, 56]),
        ("netherrack", [110, 53, 51]), ("nether_brick", [44, 21, 26]), ("basalt", [72, 72, 78]), ("blackstone", [42, 36, 41]), ("obsidian", [20, 18, 29]),
        ("end_stone", [221, 223, 165]), ("deepslate", [80, 80, 82]), ("tuff", [108, 109, 102]), ("calcite", [223, 224, 220]),
        ("glass", [190, 215, 225]), ("torch", [240, 200, 80]), ("brick", [150, 97, 83]),
    ];
    if let Some((_, c)) = WORDS.iter().find(|(word, _)| n.contains(word)) {
        return *c;
    }
    const LAST: [(&str, [u8; 3]); 16] = [
        ("coal", [50, 50, 50]), ("iron", [200, 196, 190]), ("gold", [246, 208, 61]), ("diamond", [98, 219, 214]), ("emerald", [42, 176, 67]),
        ("redstone", [170, 26, 10]), ("lapis", [38, 67, 138]), ("copper", [192, 107, 79]), ("quartz", [235, 229, 222]), ("amethyst", [133, 97, 191]),
        ("granite", [149, 103, 85]), ("diorite", [188, 188, 188]), ("andesite", [136, 136, 136]), ("cobble", [122, 122, 122]), ("bedrock", [60, 60, 60]),
        ("stone", [125, 125, 125]),
    ];
    LAST.iter().find(|(word, _)| n.contains(word)).map(|(_, c)| *c).unwrap_or([138, 132, 126])
}

/// Ce bloc n'occupe pas sa case : une plante, une torche, un rail… La vue en relief le montre
/// en petit plutôt qu'en cube plein.
pub(super) fn slight(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    if ["mushroom_block", "mushroom_stem", "coral_block", "snow_block", "bamboo_block", "bamboo_planks", "bamboo_mosaic", "grass_block", "grass_path"].iter().any(|k| n.contains(k)) {
        return false;
    }
    const WORDS: [&str; 52] = [
        "short_grass", "tall_grass", "tallgrass", "fern", "flower", "dandelion", "poppy", "tulip", "orchid", "allium", "azure_bluet", "daisy", "cornflower",
        "lily_of_the_valley", "rose", "peony", "lilac", "sapling", "mushroom", "torch", "deadbush", "dead_bush", "vine", "wheat", "carrots", "potatoes",
        "beetroot", "sugar_cane", "reeds", "rail", "lever", "button", "pressure_plate", "redstone_wire", "tripwire", "sign", "banner", "ladder", "snow_layer",
        "carpet", "seagrass", "kelp", "coral", "web", "fire", "berry", "bamboo", "lantern", "flower_pot", "candle", "chain", "skull",
    ];
    WORDS.iter().any(|w| n.contains(w)) || n == "snow" || n == "grass"
}

/// Un chunk pour la vue en relief, ou `None` s'il n'est pas en mémoire. Octets : y du bas
/// (i16), hauteur (u16), taille de la palette (u8), la palette (rouge, vert, bleu, drapeaux :
/// 1 = petit bloc, 2 = liquide ; l'entrée 0 est l'air), puis pour chacune des 256 colonnes
/// (z * 16 + x) des paires (longueur, entrée de palette) du bas vers le haut, jusqu'à la
/// hauteur. Les grottes sous la surface ne sont pas envoyées.
pub(super) fn chunk3d(view: &WorldView, data: &GameData, look: &super::textures::Look, air: u32, cx: i32, cz: i32) -> Option<Vec<u8>> {
    let real = &look.colours;
    let (base, height, blocks) = view.volume(cx, cz, 6)?;
    let mut palette: Vec<[u8; 10]> = vec![[0; 10]];
    let mut index: HashMap<u32, u8> = HashMap::new();
    index.insert(air, 0);
    let mut runs: Vec<u8> = Vec::with_capacity(256 * 12);
    for column in blocks.chunks(height) {
        let mut at = 0;
        while at < height {
            let state = column[at];
            let mut len = 1;
            while at + len < height && len < 255 && column[at + len] == state {
                len += 1;
            }
            let entry = match index.get(&state) {
                Some(e) => *e,
                None => {
                    let name = data.block_name(state).unwrap_or("");
                    let [r, g, b] = colour_in(name, real);
                    let (faces, holes) = look.faces.get(name.trim_start_matches("minecraft:")).copied().unwrap_or(([0; 3], false));
                    let flags = if name.contains("water") || name.contains("lava") { 2 } else if slight(name) { 1 } else if holes { 4 } else { 0 };
                    let [t, s, d] = faces.map(|f| f.to_le_bytes());
                    let entry = [r, g, b, flags, t[0], t[1], s[0], s[1], d[0], d[1]];
                    // (au-delà de 255 sortes dans un chunk, les suivantes prennent une entrée voisine)
                    let e = match palette.iter().position(|p| *p == entry) {
                        Some(found) if found > 0 => found as u8,
                        _ if palette.len() < 255 => {
                            palette.push(entry);
                            (palette.len() - 1) as u8
                        }
                        _ => 1,
                    };
                    index.insert(state, e);
                    e
                }
            };
            runs.push(len as u8);
            runs.push(entry);
            at += len;
        }
    }
    let mut out = Vec::with_capacity(5 + palette.len() * 10 + runs.len());
    out.extend_from_slice(&(base as i16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.push(palette.len() as u8);
    for p in &palette {
        out.extend_from_slice(p);
    }
    out.extend_from_slice(&runs);
    Some(out)
}

/// Une tuile de `TILE` × `TILE` chunks à partir du chunk (cx, cz) : quatre octets par colonne
/// (rouge, vert, bleu, puis 1 si la colonne est connue), rangée par rangée du nord au sud.
/// Les pentes sont ombrées : ce qui monte vers le sud est éclairci, ce qui descend assombri.
pub(super) fn tile(view: &WorldView, data: &GameData, real: &HashMap<String, [u8; 3]>, cx: i32, cz: i32) -> Vec<u8> {
    let side = (TILE * 16) as usize;
    let mut tops: Vec<Option<(u32, i16)>> = vec![None; side * side];
    for dz in 0..TILE {
        for dx in 0..TILE {
            let Some(surface) = view.surface(cx + dx, cz + dz) else { continue };
            for (i, top) in surface.into_iter().enumerate() {
                let (x, z) = (dx as usize * 16 + (i & 15), dz as usize * 16 + (i >> 4));
                tops[z * side + x] = Some(top);
            }
        }
    }
    let mut colours: HashMap<u32, ([u8; 3], bool)> = HashMap::new();
    let mut out = vec![0u8; side * side * 4];
    for z in 0..side {
        for x in 0..side {
            let Some((state, y)) = tops[z * side + x] else { continue };
            let (base, flat) = *colours.entry(state).or_insert_with(|| {
                let name = data.block_name(state).unwrap_or("");
                (colour_in(name, real), name.contains("water") || name.contains("lava"))
            });
            let north = if z > 0 { tops[(z - 1) * side + x].map(|t| t.1) } else { None };
            let shade: u32 = match north {
                Some(n) if !flat && y > n => 114,
                Some(n) if !flat && y < n => 84,
                _ => 100,
            };
            let at = (z * side + x) * 4;
            for c in 0..3 {
                out[at + c] = (base[c] as u32 * shade / 100).min(255) as u8;
            }
            out[at + 3] = 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_get_a_colour_from_their_name() {
        assert_eq!(colour("minecraft:water"), [52, 95, 190]);
        assert_eq!(colour("minecraft:red_wool"), [160, 39, 34]);
        assert_eq!(colour("light_blue_concrete"), [58, 175, 217]);
        // (le mot le plus précis l'emporte : du grès rouge n'est pas du sable)
        assert_eq!(colour("red_sandstone"), [182, 98, 32]);
        assert_eq!(colour("minecraft:grass_block"), [104, 148, 70]);
        assert_eq!(colour("minecraft:iron_ore"), [200, 196, 190]);
        assert_eq!(colour("un_bloc_de_plugin"), [138, 132, 126]);
        assert!(slight("minecraft:short_grass") && slight("torch") && slight("minecraft:red_mushroom"));
        assert!(!slight("minecraft:grass_block") && !slight("stone") && !slight("red_mushroom_block"));
    }
}
