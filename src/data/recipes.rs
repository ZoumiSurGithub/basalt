//! Recettes de fabrication (`data/recipes.json`), pour vérifier ce que le jeu annonce.
//!
//! Le jeu calcule lui-même le résultat d'une fabrication et l'annonce au serveur. Un client
//! modifié pourrait annoncer n'importe quel objet : le serveur retrouve donc une recette qui
//! donne cet objet et qui correspond exactement au contenu de la grille, et en déduit ce que
//! chaque case doit perdre.
//!
//! Ne sont vérifiées que les recettes de l'établi et de la grille d'inventaire (avec forme ou
//! sans forme). La variante d'un objet (`auxValue`) n'est pas comparée : le serveur ne garde
//! pas de variante dans ses piles.

use std::collections::HashMap;

use crate::json::Value;

/// Ingrédient d'une case : un objet précis, ou une famille d'objets (« planks »…).
#[derive(Debug, Clone, PartialEq)]
enum Ingredient {
    Item(String),
    Tag(String),
}

impl Ingredient {
    /// Ingrédient d'une recette : un objet précis, ou une famille (« planches »).
    fn read(v: &Value) -> Option<Ingredient> {
        match v.get("type").and_then(|t| t.as_str())? {
            "name" => Some(Ingredient::Item(v.get("itemId")?.as_str()?.to_string())),
            "item_tag" => Some(Ingredient::Tag(v.get("itemTag")?.as_str()?.to_string())),
            _ => None,
        }
    }

    /// Cet objet convient-il ?
    fn accepts(&self, item: &str) -> bool {
        match self {
            Ingredient::Item(name) => name == item,
            Ingredient::Tag(tag) => has_tag(item, tag),
        }
    }
}

#[derive(Debug, Clone)]
enum Shape {
    /// Cases ligne par ligne (`None` : case vide)
    Shaped { width: usize, height: usize, cells: Vec<Option<Ingredient>> },
    Shapeless(Vec<Ingredient>),
}

#[derive(Debug, Clone)]
struct Recipe {
    shape: Shape,
    count: u16,
}

/// Ce qu'une fabrication validée retire de la grille.
#[derive(Debug, Clone, PartialEq)]
pub struct Craft {
    /// Nombre de fabrications d'affilée (fabrication en série)
    pub times: u16,
    /// Pour chaque case utilisée (numéro de la grille du jeu, 28 à 40), objets à retirer
    pub consumed: Vec<(usize, u8)>,
}

#[derive(Default)]
pub struct Recipes {
    by_output: HashMap<String, Vec<Recipe>>,
}

impl Recipes {
    /// Lit les recettes de data/ et les range par objet fabriqué.
    pub fn parse(doc: &Value) -> Recipes {
        let mut by_output: HashMap<String, Vec<Recipe>> = HashMap::new();
        for r in doc.get("recipes").and_then(|v| v.as_arr()).unwrap_or(&[]) {
            let block = r.get("block").and_then(|b| b.as_str()).unwrap_or("");
            if block != "crafting_table" && block != "deprecated" {
                continue; // four, tailleur de pierre, forge… : pas dans la grille
            }
            let output = match r.get("output").and_then(|o| o.as_arr()).and_then(|o| o.first()) {
                Some(o) => o,
                None => continue,
            };
            let (name, count) = match (output.get("id").and_then(|v| v.as_str()), output.get("count").and_then(|v| v.as_f64())) {
                (Some(n), Some(c)) if c >= 1.0 && c <= 64.0 => (n.to_string(), c as u16),
                _ => continue,
            };
            let shape = match (r.get("shape").and_then(|s| s.as_arr()), r.get("input")) {
                (Some(rows), Some(Value::Obj(keys))) => {
                    let rows: Vec<Vec<char>> = rows.iter().filter_map(|row| row.as_str()).map(|s| s.chars().collect()).collect();
                    let height = rows.len();
                    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                    if height == 0 || height > 3 || width == 0 || width > 3 {
                        continue;
                    }
                    let mut cells = Vec::with_capacity(width * height);
                    let mut ok = true;
                    for row in &rows {
                        for x in 0..width {
                            match row.get(x) {
                                None | Some(' ') => cells.push(None),
                                Some(c) => match keys.iter().find(|(k, _)| k.chars().next() == Some(*c)).and_then(|(_, v)| Ingredient::read(v)) {
                                    Some(i) => cells.push(Some(i)),
                                    None => ok = false,
                                },
                            }
                        }
                    }
                    if !ok {
                        continue;
                    }
                    Shape::Shaped { width, height, cells }
                }
                (None, Some(Value::Arr(list))) => {
                    let items: Vec<Ingredient> = list.iter().filter_map(Ingredient::read).collect();
                    if items.len() != list.len() || items.is_empty() || items.len() > 9 {
                        continue;
                    }
                    Shape::Shapeless(items)
                }
                _ => continue,
            };
            by_output.entry(name).or_default().push(Recipe { shape, count });
        }
        Recipes { by_output }
    }

    /// Nombre de recettes.
    pub fn len(&self) -> usize {
        self.by_output.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_output.is_empty()
    }

    /// Vérifie une fabrication. `grid` : les 13 cases de la grille du jeu (28 à 40) avec le nom
    /// et la quantité de leur objet. `output` et `count` : ce que le jeu annonce avoir créé.
    pub fn check(&self, grid: &[Option<(String, u8)>; 13], output: &str, count: u16) -> Option<Craft> {
        let output = if output.contains(':') { output.to_string() } else { format!("minecraft:{}", output) };
        let recipes = self.by_output.get(&output)?;
        // Grille d'établi (cases 32 à 40) si elle porte quelque chose, sinon grille 2×2 (28 à 31)
        let big = grid[4..].iter().any(|c| c.is_some());
        let (size, cells): (usize, Vec<(usize, Option<&(String, u8)>)>) = if big {
            (3, (0..9).map(|i| (32 + i, grid[4 + i].as_ref())).collect())
        } else {
            (2, (0..4).map(|i| (28 + i, grid[i].as_ref())).collect())
        };
        if cells.iter().all(|(_, c)| c.is_none()) {
            return None;
        }
        for recipe in recipes {
            if count == 0 || count % recipe.count != 0 {
                continue;
            }
            let times = count / recipe.count;
            let used = match &recipe.shape {
                Shape::Shaped { width, height, cells: pattern } => match_shaped(size, &cells, *width, *height, pattern),
                Shape::Shapeless(items) => match_shapeless(&cells, items),
            };
            let used = match used {
                Some(u) => u,
                None => continue,
            };
            // Chaque case utilisée doit pouvoir fournir un objet par fabrication
            if times > 64 || used.iter().any(|slot| cells.iter().find(|(s, _)| s == slot).and_then(|(_, c)| *c).map(|(_, n)| (*n as u16) < times).unwrap_or(true)) {
                continue;
            }
            return Some(Craft { times, consumed: used.into_iter().map(|s| (s, times as u8)).collect() });
        }
        None
    }
}

/// Recette avec forme : n'importe quelle position dans la grille, éventuellement retournée
/// de gauche à droite ; toutes les autres cases doivent être vides. Renvoie les cases utilisées.
fn match_shaped(size: usize, cells: &[(usize, Option<&(String, u8)>)], width: usize, height: usize, pattern: &[Option<Ingredient>]) -> Option<Vec<usize>> {
    if width > size || height > size {
        return None;
    }
    for oy in 0..=(size - height) {
        for ox in 0..=(size - width) {
            for mirror in [false, true] {
                let mut used = Vec::new();
                let mut ok = true;
                for gy in 0..size {
                    for gx in 0..size {
                        let (slot, item) = cells[gy * size + gx];
                        let inside = gx >= ox && gx < ox + width && gy >= oy && gy < oy + height;
                        let want = if inside {
                            let px = if mirror { width - 1 - (gx - ox) } else { gx - ox };
                            pattern[(gy - oy) * width + px].as_ref()
                        } else {
                            None
                        };
                        match (want, item) {
                            (None, None) => {}
                            (Some(ing), Some((name, _))) if ing.accepts(name) => used.push(slot),
                            _ => ok = false,
                        }
                    }
                }
                if ok {
                    return Some(used);
                }
            }
        }
    }
    None
}

/// Recette sans forme : chaque objet de la grille correspond à un ingrédient différent, et
/// tous les ingrédients sont présents.
fn match_shapeless(cells: &[(usize, Option<&(String, u8)>)], items: &[Ingredient]) -> Option<Vec<usize>> {
    let filled: Vec<(usize, &str)> = cells.iter().filter_map(|(s, c)| c.map(|(n, _)| (*s, n.as_str()))).collect();
    if filled.len() != items.len() {
        return None;
    }
    // Affectation par essais successifs (au plus 9 objets)
    fn assign(filled: &[(usize, &str)], items: &[Ingredient], taken: &mut [bool], i: usize) -> bool {
        if i == filled.len() {
            return true;
        }
        for (k, ing) in items.iter().enumerate() {
            if !taken[k] && ing.accepts(filled[i].1) {
                taken[k] = true;
                if assign(filled, items, taken, i + 1) {
                    return true;
                }
                taken[k] = false;
            }
        }
        false
    }
    let mut taken = vec![false; items.len()];
    if assign(&filled, items, &mut taken, 0) {
        Some(filled.iter().map(|(s, _)| *s).collect())
    } else {
        None
    }
}

const WOODS: [&str; 12] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "pale_oak", "bamboo", "crimson", "warped"];

/// Familles d'objets employées par les recettes. Les données du jeu ne donnent pas leur
/// contenu : il est reconstitué d'après les noms des objets.
pub fn has_tag(item: &str, tag: &str) -> bool {
    let short = item.trim_start_matches("minecraft:");
    let wood = |suffix: &str| WOODS.iter().any(|w| short == format!("{}{}", w, suffix));
    let tag = tag.trim_start_matches("minecraft:");
    match tag {
        "planks" => short.ends_with("_planks"),
        "wooden_slabs" => wood("_slab"),
        "logs" | "logs_that_burn" => {
            let burns = !(short.contains("crimson") || short.contains("warped"));
            let log = short.ends_with("_log") || short.ends_with("_wood") || short.ends_with("_stem") || short.ends_with("_hyphae") || short == "bamboo_block" || short == "stripped_bamboo_block";
            log && (tag == "logs" || burns)
        }
        "wool" => short.ends_with("_wool"),
        "coals" => short == "coal" || short == "charcoal",
        "egg" => short == "egg" || short == "blue_egg" || short == "brown_egg",
        "metal_nuggets" => short == "iron_nugget" || short == "gold_nugget" || short == "copper_nugget",
        "mushrooms_for_stew" => short == "red_mushroom" || short == "brown_mushroom",
        "soul_fire_base_blocks" => short == "soul_sand" || short == "soul_soil",
        "stone_crafting_materials" | "stone_tool_materials" => short == "cobblestone" || short == "blackstone" || short == "cobbled_deepslate",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cells: &[(usize, &str, u8)]) -> [Option<(String, u8)>; 13] {
        let mut g: [Option<(String, u8)>; 13] = Default::default();
        for (slot, name, n) in cells {
            g[slot - 28] = Some((format!("minecraft:{}", name), *n));
        }
        g
    }

    fn recipes() -> Recipes {
        let text = r#"{"recipes":[
          {"type":0,"block":"crafting_table","shape":["A","A"],"input":{"A":{"type":"item_tag","count":1,"itemTag":"minecraft:planks"}},"output":[{"id":"minecraft:stick","count":4}]},
          {"type":0,"block":"crafting_table","shape":["AA ","AB "," B "],"input":{"A":{"type":"name","count":1,"itemId":"minecraft:iron_ingot"},"B":{"type":"name","count":1,"itemId":"minecraft:stick"}},"output":[{"id":"minecraft:iron_axe","count":1}]},
          {"type":1,"block":"crafting_table","input":[{"type":"name","count":1,"itemId":"minecraft:oak_log"}],"output":[{"id":"minecraft:oak_planks","count":4}]},
          {"type":1,"block":"furnace","input":[{"type":"name","count":1,"itemId":"minecraft:cobblestone"}],"output":[{"id":"minecraft:stone","count":1}]}
        ]}"#;
        Recipes::parse(&crate::json::parse(text).unwrap())
    }

    #[test]
    fn a_matching_grid_gives_what_each_slot_loses() {
        let r = recipes();
        // Bâtons dans la grille 2×2, à droite : 2 planches donnent 4 bâtons
        let g = grid(&[(29, "spruce_planks", 3), (31, "oak_planks", 5)]);
        assert_eq!(r.check(&g, "minecraft:stick", 4), Some(Craft { times: 1, consumed: vec![(29, 1), (31, 1)] }));
        // En série : 3 fois (12 bâtons), limité par la plus petite pile
        assert_eq!(r.check(&g, "minecraft:stick", 12).unwrap().times, 3);
        assert!(r.check(&g, "minecraft:stick", 16).is_none(), "4 fois : il manque une planche");
        // Hache : forme retournée acceptée, sur l'établi
        let axe = grid(&[(33, "iron_ingot", 1), (34, "iron_ingot", 1), (36, "stick", 1), (37, "iron_ingot", 1), (39, "stick", 1)]);
        assert!(r.check(&axe, "iron_axe", 1).is_some(), "hache retournée");
        // Planches sans forme, n'importe où
        assert!(r.check(&grid(&[(38, "oak_log", 1)]), "oak_planks", 4).is_some());
    }

    #[test]
    fn anything_else_is_refused() {
        let r = recipes();
        let g = grid(&[(29, "oak_planks", 1), (31, "oak_planks", 1)]);
        assert!(r.check(&g, "diamond_block", 1).is_none(), "objet sans rapport avec la grille");
        assert!(r.check(&g, "stick", 5).is_none(), "quantité qui n'est pas un multiple de la recette");
        assert!(r.check(&grid(&[(29, "oak_planks", 1), (31, "dirt", 1)]), "stick", 4).is_none(), "mauvais ingrédient");
        assert!(r.check(&grid(&[(29, "oak_planks", 1), (31, "oak_planks", 1), (28, "dirt", 1)]), "stick", 4).is_none(), "objet en trop");
        assert!(r.check(&grid(&[]), "stick", 4).is_none(), "grille vide");
        assert!(r.check(&grid(&[(28, "cobblestone", 1)]), "stone", 1).is_none(), "recette du four");
        // Hache dans la grille 2×2 : trop petite
        assert!(r.check(&grid(&[(28, "iron_ingot", 1), (29, "iron_ingot", 1), (30, "iron_ingot", 1), (31, "stick", 1)]), "iron_axe", 1).is_none());
    }

    #[test]
    fn real_recipes_load() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join("recipes.json");
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return,
        };
        let r = Recipes::parse(&crate::json::parse(&text).unwrap());
        assert!(r.len() > 2500, "{} recettes", r.len());
        // Établi : 4 planches en carré, n'importe quel bois
        let g = grid(&[(28, "oak_planks", 1), (29, "birch_planks", 1), (30, "oak_planks", 1), (31, "spruce_planks", 1)]);
        assert!(r.check(&g, "crafting_table", 1).is_some());
        // Torches : charbon de bois sur un bâton
        assert!(r.check(&grid(&[(28, "charcoal", 1), (30, "stick", 1)]), "torch", 4).is_some());
        // Coffre : 8 planches autour d'une case vide
        let chest: Vec<(usize, &str, u8)> = [32, 33, 34, 35, 37, 38, 39, 40].iter().map(|s| (*s, "oak_planks", 1)).collect();
        assert!(r.check(&grid(&chest), "chest", 1).is_some());
        assert!(r.check(&grid(&chest), "diamond", 1).is_none());
    }
}
