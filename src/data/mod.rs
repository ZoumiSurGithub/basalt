//! Données du jeu : tous les objets, tous les blocs et leurs états.
//!
//! Elles viennent du dossier `data/`, placé à côté du serveur. Ce sont les fichiers
//! du projet BedrockData (licence CC0), extraits du jeu officiel : ils changent à
//! chaque version de Minecraft et se remplacent sans toucher au code.
//!
//! Sans ce dossier, le serveur fonctionne avec une poignée de blocs connus par cœur.

pub use crate::nbt;

pub mod recipes;
pub use recipes::{Craft, Recipes};

use std::collections::HashMap;
use std::path::Path;

use crate::inflate::inflate;
use crate::json::{self, Value};
use nbt::Tag;

/// Un objet du jeu, tel que le serveur l'annonce au jeu.
#[derive(Debug, Clone)]
pub struct ItemDef {
    pub name: String,
    pub id: i16,
    pub version: i32,
    /// Composants de l'objet au format réseau, pour les objets qui en ont
    pub components: Option<Vec<u8>>,
}

/// Valeur d'une propriété d'état de bloc (les bits sont des entiers 0 ou 1).
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Int(i32),
    Str(String),
}

/// Noms des blocs pour les plugins (qui tournent sur leur propre fil) : état -> nom, et nom
/// -> état par défaut.
#[derive(Default)]
pub struct BlockNames {
    pub names: HashMap<u32, String>,
    pub defaults: HashMap<String, u32>,
}

#[derive(Clone)]
pub struct GameData {
    pub items: Vec<ItemDef>,
    item_by_name: HashMap<String, usize>,
    item_by_id: HashMap<i16, usize>,
    /// Nom de bloc -> identifiants réseau de ses états (le premier est l'état par défaut)
    block_states: HashMap<String, Vec<u32>>,
    /// Recettes de fabrication (vides si `recipes.json` manque : toute fabrication est refusée)
    pub recipes: Recipes,
    /// Identifiant réseau -> propriétés de cet état (`open_bit`, `pillar_axis`…)
    state_props: HashMap<u32, Vec<(String, Prop)>>,
    /// Identifiant réseau -> nom de bloc
    pub(crate) block_names: HashMap<u32, String>,
    /// Nom de bloc -> nom de l'objet correspondant
    block_to_item: HashMap<String, String>,
    /// Nom d'objet -> nom du bloc qu'il pose
    item_to_block: HashMap<String, String>,
    hardness: HashMap<String, f32>,
    /// Propriétés physiques de chaque bloc, par nom
    props: HashMap<String, crate::block::BlockProps>,
    /// Liste des entités, prête à être envoyée (NBT réseau)
    pub entity_identifiers: Option<Vec<u8>>,
}

/// Décompresse un fichier gzip (les fichiers .nbt du jeu de données le sont).
pub fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err("pas un fichier gzip");
    }
    let flags = data[3];
    let mut pos = 10;
    if flags & 4 != 0 {
        let extra = u16::from_le_bytes([*data.get(pos).ok_or("gzip tronqué")?, *data.get(pos + 1).ok_or("gzip tronqué")?]) as usize;
        pos += 2 + extra;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            while *data.get(pos).ok_or("gzip tronqué")? != 0 {
                pos += 1;
            }
            pos += 1;
        }
    }
    if flags & 2 != 0 {
        pos += 2;
    }
    let body = data.get(pos..data.len() - 8).ok_or("gzip tronqué")?;
    inflate(body, limit)
}

/// Lecture des fichiers de data/, avec le nom du fichier dans le message d'erreur.
fn read_file(dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    std::fs::read(dir.join(name)).map_err(|e| format!("{} : {}", name, e))
}

fn read_json(dir: &Path, name: &str) -> Result<Value, String> {
    let bytes = read_file(dir, name)?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{} : texte invalide", name))?;
    json::parse(&text).map_err(|e| format!("{} : {}", name, e))
}

fn read_nbt(dir: &Path, name: &str) -> Result<Tag, String> {
    let raw = gunzip(&read_file(dir, name)?, 64 * 1024 * 1024).map_err(|e| format!("{} : {}", name, e))?;
    nbt::read_be(&raw).ok_or_else(|| format!("{} : NBT illisible", name))
}

/// Nombre d'une valeur JSON.
fn num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Num(n)) => Some(*n),
        _ => None,
    }
}

impl GameData {
    /// Charge les données depuis un dossier. Les fichiers indispensables sont la liste
    /// d'objets et la palette de blocs ; les autres sont facultatifs.
    pub fn load(dir: &Path) -> Result<GameData, String> {
        // --- objets ---
        let palette = read_json(dir, "item_palette.json")?;
        let list = palette.get("items").and_then(|v| v.as_arr()).ok_or("item_palette.json : liste « items » absente")?;
        let components = read_nbt(dir, "item_components.nbt").ok();
        let mut items = Vec::with_capacity(list.len());
        for entry in list {
            let name = entry.get("name").and_then(|v| v.as_str()).ok_or("item_palette.json : objet sans nom")?;
            let id = num(entry.get("id")).ok_or("item_palette.json : objet sans identifiant")?;
            if !(-32768.0..=32767.0).contains(&id) {
                return Err(format!("item_palette.json : identifiant hors limites pour {}", name));
            }
            let based = matches!(entry.get("component_based"), Some(Value::Bool(true)));
            let comp = if based {
                components.as_ref().and_then(|c| c.get(name)).map(nbt::write_network)
            } else {
                None
            };
            items.push(ItemDef {
                name: name.to_string(),
                id: id as i16,
                version: num(entry.get("version")).unwrap_or(2.0) as i32,
                components: comp,
            });
        }
        let item_by_name = items.iter().enumerate().map(|(i, it)| (it.name.clone(), i)).collect();
        let item_by_id = items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();

        // --- blocs ---
        let blocks = read_nbt(dir, "block_palette.nbt")?;
        let list = blocks.get("blocks").and_then(|t| t.as_list()).ok_or("block_palette.nbt : liste « blocks » absente")?;
        let mut block_states: HashMap<String, Vec<u32>> = HashMap::new();
        let mut block_names = HashMap::with_capacity(list.len());
        let mut state_props: HashMap<u32, Vec<(String, Prop)>> = HashMap::new();
        for entry in list {
            let name = entry.get("name").and_then(|t| t.as_str()).ok_or("block_palette.nbt : bloc sans nom")?;
            let id = entry.get("network_id").and_then(|t| t.as_int()).ok_or("block_palette.nbt : bloc sans identifiant")? as u32;
            block_states.entry(name.to_string()).or_default().push(id);
            block_names.insert(id, name.to_string());
            if let Some(Tag::Compound(states)) = entry.get("states") {
                let props: Vec<(String, Prop)> = states
                    .iter()
                    .filter_map(|(key, value)| match value {
                        Tag::Str(s) => Some((key.clone(), Prop::Str(s.clone()))),
                        other => other.as_i64().map(|n| (key.clone(), Prop::Int(n as i32))),
                    })
                    .collect();
                if !props.is_empty() {
                    state_props.insert(id, props);
                }
            }
        }

        // --- correspondance blocs / objets ---
        let mut block_to_item = HashMap::new();
        let mut item_to_block = HashMap::new();
        if let Ok(Value::Obj(pairs)) = read_json(dir, "block_id_to_item_id_map.json") {
            for (block, item) in pairs {
                if let Value::Str(item) = item {
                    item_to_block.entry(item.clone()).or_insert_with(|| block.clone());
                    block_to_item.insert(block, item);
                }
            }
        }

        // --- propriétés ---
        let mut hardness = HashMap::new();
        let mut block_props = HashMap::new();
        if let Ok(Value::Obj(pairs)) = read_json(dir, "block_properties_table.json") {
            for (block, props) in pairs {
                if let Some(h) = num(props.get("hardness")) {
                    hardness.insert(block.clone(), h as f32);
                    let get = |key: &str, default: f64| num(props.get(key)).filter(|v| v.is_finite()).unwrap_or(default);
                    block_props.insert(
                        block,
                        crate::block::BlockProps {
                            hardness: h as f32,
                            blast_resistance: get("blastResistance", h.max(0.0)) as f32,
                            light: get("brightness", 0.0).clamp(0.0, 15.0) as u8,
                            flammability: get("flammability", 0.0).clamp(0.0, 255.0) as u8,
                            friction: get("friction", 0.6) as f32,
                            opacity: get("opacity", 1.0).clamp(0.0, 1.0) as f32,
                        },
                    );
                }
            }
        }

        let entity_identifiers = read_nbt(dir, "entity_identifiers.nbt").ok().map(|t| nbt::write_network(&t));

        // --- recettes ---
        let recipes = read_json(dir, "recipes.json").map(|doc| Recipes::parse(&doc)).unwrap_or_default();

        Ok(GameData {
            recipes,
            items,
            item_by_name,
            item_by_id,
            block_states,
            state_props,
            block_names,
            block_to_item,
            item_to_block,
            hardness,
            props: block_props,
            entity_identifiers,
        })
    }

    /// Noms de tous les blocs (`minecraft:stone`), dans l'ordre alphabétique.
    pub fn block_name_list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.block_states.keys().cloned().collect();
        names.sort();
        names
    }

    /// Noms de tous les objets, dans l'ordre alphabétique.
    pub fn item_name_list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.items.iter().map(|i| i.name.clone()).collect();
        names.sort();
        names
    }

    /// Nombre de blocs différents, et d'états de blocs.
    pub fn block_count(&self) -> usize {
        self.block_states.len()
    }

    pub fn state_count(&self) -> usize {
        self.block_names.len()
    }

    /// Objet par son nom, avec ou sans le préfixe « minecraft: ».
    pub fn item(&self, name: &str) -> Option<&ItemDef> {
        let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{}", name) };
        self.item_by_name.get(&full).map(|i| &self.items[*i])
    }

    /// Ajoute un objet créé par un plugin (ou remplace celui du même nom).
    pub fn add_item(&mut self, def: ItemDef) {
        match self.item_by_name.get(&def.name).copied() {
            Some(i) => {
                self.item_by_id.remove(&self.items[i].id);
                self.item_by_id.insert(def.id, i);
                self.items[i] = def;
            }
            None => {
                self.item_by_name.insert(def.name.clone(), self.items.len());
                self.item_by_id.insert(def.id, self.items.len());
                self.items.push(def);
            }
        }
    }

    /// Ajoute un bloc créé par un plugin : ses états (le premier est celui par défaut), ses
    /// propriétés, et l'objet du même nom qui le pose.
    pub fn add_block(&mut self, name: &str, states: &[u32], props: crate::block::BlockProps, item_id: i16) {
        if let Some(old) = self.block_states.insert(name.to_string(), states.to_vec()) {
            for s in old {
                self.block_names.remove(&s);
            }
        }
        for state in states {
            self.block_names.insert(*state, name.to_string());
        }
        self.hardness.insert(name.to_string(), props.hardness);
        self.props.insert(name.to_string(), props);
        self.add_item(ItemDef { name: name.to_string(), id: item_id, version: 2, components: None });
    }

    /// Retire des blocs créés par les plugins, avec leurs objets.
    pub fn remove_blocks(&mut self, names: &[String]) {
        for name in names {
            for state in self.block_states.remove(name).unwrap_or_default() {
                self.block_names.remove(&state);
            }
            self.hardness.remove(name);
            self.props.remove(name);
        }
        self.items.retain(|i| !names.contains(&i.name));
        self.item_by_name = self.items.iter().enumerate().map(|(i, it)| (it.name.clone(), i)).collect();
        self.item_by_id = self.items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();
    }

    /// Retire les objets créés par les plugins (identifiants à partir de `from`).
    pub fn remove_items_from(&mut self, from: i16) {
        self.items.retain(|i| i.id < from);
        self.item_by_name = self.items.iter().enumerate().map(|(i, it)| (it.name.clone(), i)).collect();
        self.item_by_id = self.items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();
    }

    /// Objet d'après son identifiant numérique.
    pub fn item_by_id(&self, id: i16) -> Option<&ItemDef> {
        self.item_by_id.get(&id).map(|i| &self.items[*i])
    }

    /// Identifiant réseau de l'état par défaut d'un bloc.
    pub fn default_state(&self, block: &str) -> Option<u32> {
        self.block_states.get(block).and_then(|s| s.first().copied())
    }

    /// Valeur d'une propriété de cet état de bloc.
    pub fn prop(&self, state: u32, key: &str) -> Option<Prop> {
        self.state_props.get(&state)?.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// Noms des propriétés de cet état de bloc.
    pub fn prop_keys(&self, state: u32) -> Vec<String> {
        self.state_props.get(&state).map(|p| p.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default()
    }

    /// Le même bloc avec quelques propriétés changées (porte ouverte, bûche couchée…).
    /// None si le bloc n'a pas ces propriétés ou pas d'état correspondant.
    pub fn with_props(&self, state: u32, changes: &[(&str, Prop)]) -> Option<u32> {
        let name = self.block_names.get(&state)?;
        let mut wanted = self.state_props.get(&state)?.clone();
        for (key, value) in changes {
            wanted.iter_mut().find(|(k, _)| k == key)?.1 = value.clone();
        }
        self.block_states.get(name)?.iter().copied().find(|s| self.state_props.get(s) == Some(&wanted))
    }

    /// État d'un bloc décrit par un texte : `minecraft:pink_petals[growth=3,direction=north]`
    /// (sans crochets : l'état par défaut). Une propriété inconnue donne `None`.
    pub fn state_from_spec(&self, spec: &str) -> Option<u32> {
        let (name, props) = match spec.find('[') {
            Some(i) => (&spec[..i], spec[i + 1..].trim_end_matches(']')),
            None => (spec, ""),
        };
        let base = self.default_state(name)?;
        if props.is_empty() {
            return Some(base);
        }
        let changes: Vec<(&str, Prop)> = props
            .split(',')
            .filter_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                Some((k.trim(), v.trim().parse::<i32>().map(Prop::Int).unwrap_or_else(|_| Prop::Str(v.trim().to_string()))))
            })
            .collect();
        self.with_props(base, &changes)
    }

    /// Copie des noms de blocs pour les plugins.
    pub fn block_names_for_plugins(&self) -> BlockNames {
        BlockNames {
            names: self.block_names.clone(),
            defaults: self.block_states.iter().filter_map(|(n, s)| s.first().map(|d| (n.clone(), *d))).collect(),
        }
    }

    /// Nom du bloc d'un état (« minecraft:stone »).
    pub fn block_name(&self, state: u32) -> Option<&str> {
        self.block_names.get(&state).map(|s| s.as_str())
    }

    /// Bloc posé par un objet (identifiant réseau de son état par défaut), s'il en pose un.
    pub fn block_for_item(&self, id: i16) -> Option<u32> {
        let item = self.item_by_id(id)?;
        let block = self.item_to_block.get(&item.name).map(|s| s.as_str()).unwrap_or(item.name.as_str());
        // Objets dont le bloc porte un autre nom (la canne à sucre pose des « reeds »)
        self.default_state(block).or_else(|| {
            let other = match item.name.trim_start_matches("minecraft:") {
                "sugar_cane" => "reeds",
                "wheat_seeds" => "wheat",
                "carrot" => "carrots",
                "potato" => "potatoes",
                "beetroot_seeds" => "beetroot",
                "melon_seeds" => "melon_stem",
                "pumpkin_seeds" => "pumpkin_stem",
                "torchflower_seeds" => "torchflower_crop",
                "pitcher_pod" => "pitcher_crop",
                "redstone" => "redstone_wire",
                "string" => "trip_wire",
                "sweet_berries" => "sweet_berry_bush",
                "glow_berries" => "cave_vines",
                "cocoa_beans" => "cocoa",
                "repeater" => "unpowered_repeater",
                "comparator" => "unpowered_comparator",
                _ => return None,
            };
            self.default_state(&format!("minecraft:{}", other))
        })
    }

    /// Objet obtenu en cassant un bloc à la main. Par défaut, le bloc lui-même ; quelques
    /// règles du jeu normal sont appliquées (herbe, pierre, verre, feuillages, bloc incassable).
    pub fn drop_for_block(&self, state: u32) -> Option<i16> {
        let name = self.block_name(state)?;
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let dropped = match short {
            "grass_block" | "grass_path" | "farmland" | "mycelium" | "podzol" => "minecraft:dirt",
            "stone" => "minecraft:cobblestone",
            "bedrock" | "air" | "fire" | "water" | "lava" | "flowing_water" | "flowing_lava" => return None,
            _ if short.ends_with("glass") || short.ends_with("glass_pane") || short.ends_with("leaves") => return None,
            _ => self.block_to_item.get(name).map(|s| s.as_str()).unwrap_or(name),
        };
        self.item(dropped).map(|i| i.id)
    }

    /// Objet correspondant à un bloc d'après son nom (le nom de l'objet diffère parfois).
    pub fn item_for_block_name(&self, name: &str) -> Option<i16> {
        let item = self.block_to_item.get(name).map(|s| s.as_str()).unwrap_or(name);
        self.item(item).map(|i| i.id)
    }

    /// Registre de tous les blocs du jeu, avec leurs propriétés.
    pub fn block_registry(&self) -> crate::block::BlockRegistry {
        crate::block::BlockRegistry::new(self.props.iter().map(|(name, props)| (name.clone(), *props)))
    }

    /// Propriétés physiques d'un bloc (lumière émise, opacité…), par son nom complet.
    pub fn block_props(&self, name: &str) -> Option<&crate::block::BlockProps> {
        self.props.get(name)
    }

    /// Dureté d'un bloc, si elle est connue.
    pub fn hardness(&self, state: u32) -> Option<f32> {
        self.hardness.get(self.block_name(state)?).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> GameData {
        GameData::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).expect("données du jeu")
    }

    #[test]
    fn real_data_files_are_loaded() {
        let d = data();
        assert!(d.items.len() > 2000 && d.block_count() > 1400 && d.state_count() > 20_000);
        assert_eq!(d.item("stone").map(|i| i.id), Some(1));
        assert_eq!(d.item("minecraft:diamond_pickaxe").map(|i| i.id), Some(320));
        // l'identifiant réseau du fichier est bien le hachage que le serveur calcule
        assert_eq!(d.default_state("minecraft:stone"), Some(crate::bedrock::block_hash("minecraft:stone")));
        assert_eq!(d.block_name(crate::bedrock::block_hash("minecraft:air")), Some("minecraft:air"));
        assert!(d.entity_identifiers.as_ref().map(|e| e.len() > 1000).unwrap_or(false));
        assert!(d.items.iter().filter(|i| i.components.is_some()).count() > 50);
    }

    #[test]
    fn blocks_items_and_drops_are_linked() {
        let d = data();
        let stone = d.default_state("minecraft:stone").unwrap();
        let grass = d.default_state("minecraft:grass_block").unwrap();
        assert_eq!(d.block_for_item(1), Some(stone));
        assert_eq!(d.drop_for_block(grass), d.item("dirt").map(|i| i.id));
        assert_eq!(d.drop_for_block(stone), d.item("cobblestone").map(|i| i.id));
        assert_eq!(d.drop_for_block(d.default_state("minecraft:glass").unwrap()), None);
        // une porte : le bloc et l'objet portent des noms différents
        let door = d.item("acacia_door").unwrap().id;
        assert_eq!(d.block_name(d.block_for_item(door).unwrap()), Some("minecraft:acacia_door"));
        // un objet qui n'est pas un bloc ne pose rien
        assert_eq!(d.block_for_item(d.item("stick").unwrap().id), None);
        assert!(d.hardness(stone).unwrap() > 1.0);
    }

    #[test]
    fn gzip_errors_are_reported() {
        assert!(gunzip(&[1, 2, 3], 100).is_err());
        assert!(gunzip(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8], 100).is_err());
    }
}
