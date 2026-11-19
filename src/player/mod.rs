//! `Player` : un joueur. Il hérite de `Human` (faim, expérience), donc de `Living` (vie) et
//! d'`Entity` (position, texte au-dessus de la tête), et ajoute ce qui tient au compte :
//! pseudo, nom affiché, permissions, dates de visite, inventaire sauvegardé.
//!
//! Le cycle est le même pour tous : `load` (ou `new` à la première visite) quand le joueur
//! entre, `save` quand il part, à intervalles réguliers et à l'arrêt du serveur.

pub mod skin;
pub use skin::Skin;

use std::path::{Path, PathBuf};

pub use crate::entity::clean_text;
use crate::entity::{extends, Human};
use crate::nbt::{self, Tag};

/// Objet rangé dans l'inventaire sauvegardé : (conteneur, emplacement, nom de l'objet, quantité).
/// L'objet est noté par son nom, valable d'une version du jeu à l'autre.
pub type SavedItem = (String, u8, String, u8);

#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    pub human: Human,
    /// Pseudo du compte : identifie le joueur et son fichier
    pub name: String,
    /// Nom montré dans le chat et les messages du serveur
    pub display_name: String,
    /// Vrai si une position a déjà été enregistrée
    pub has_pos: bool,
    /// Première et dernière visite (secondes Unix)
    pub first_played: u64,
    pub last_played: u64,
    pub items: Vec<SavedItem>,
    /// Permissions fixées pour ce joueur : (nom, accordée)
    pub permissions: Vec<(String, bool)>,
    /// Mode de jeu (0 survie, 1 créatif) ; None : celui du serveur
    pub game_type: Option<i32>,
    /// Dossier du monde où le joueur se trouvait
    pub level: String,
}
extends!(Player, human, Human);

/// Hauteur des yeux d'un joueur au-dessus de ses pieds.
pub const EYE_HEIGHT: f32 = 1.62;

/// Ancien nom de `Player`.
pub type PlayerData = Player;

fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .take(32)
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("{}.dat", if safe.is_empty() { "_".to_string() } else { safe })
}

impl Player {
    /// Initialisation d'un joueur qui vient pour la première fois.
    pub fn new(name: &str, now: u64) -> PlayerData {
        Player {
            human: Human::new(name),
            name: name.to_string(),
            display_name: name.to_string(),
            has_pos: false,
            first_played: now,
            last_played: now,
            items: Vec::new(),
            permissions: Vec::new(),
            game_type: None,
            level: String::new(),
        }
    }

    pub fn path(dir: &Path, name: &str) -> PathBuf {
        dir.join("players").join(file_name(name))
    }

    /// Remet chaque valeur dans ses bornes (fichier modifié à la main, plugin maladroit…).
    pub fn sanitize(&mut self) {
        self.human.sanitize();
        self.display_name = clean_text(&self.display_name, 32);
        if self.display_name.is_empty() {
            self.display_name = self.name.clone();
        }
        if !self.has_valid_position() {
            self.has_pos = false;
        }
    }

    /// Chargement depuis `players/<pseudo>.dat`, au format de PocketMine (NBT compressé, mêmes
    /// noms de champs). À défaut, les anciens fichiers de Basalt (`.nbt`, puis `.txt`) sont
    /// relus une dernière fois. `Ok(None)` : le joueur n'a pas encore de fichier.
    pub fn load(dir: &Path, name: &str) -> Result<Option<Player>, String> {
        let path = Self::path(dir, name);
        if let Some(doc) = nbt::load_file(&path)? {
            return Ok(Some(Self::from_nbt(&doc, name, false)));
        }
        if let Some(doc) = nbt::load_file(&path.with_extension("nbt"))? {
            return Ok(Some(Self::from_nbt(&doc, name, true)));
        }
        Self::load_legacy(&path.with_extension("txt"), name)
    }

    /// Lit un joueur. `old` : ancien format de Basalt (hauteur des yeux, dates en secondes).
    /// Les deux jeux de noms sont compris ; ceux de PocketMine passent en premier.
    fn from_nbt(doc: &Tag, name: &str, old: bool) -> Player {
        let mut p = Player::new(name, 0);
        let own = doc.get("Basalt");
        let text = |key: &str| own.and_then(|o| o.str_of(key)).or_else(|| doc.str_of(key)).map(|s| s.to_string());
        let num = |keys: &[&str]| keys.iter().find_map(|k| doc.get(k).and_then(|t| t.as_f32()));
        let int = |keys: &[&str]| keys.iter().find_map(|k| doc.get(k).and_then(|t| t.as_i64()));
        if let Some(v) = text("DisplayName") {
            p.display_name = v;
        }
        if let Some(v) = doc.str_of("NameTag") {
            p.name_tag = v.to_string();
        }
        p.max_health = own.and_then(|o| o.get("MaxHealth")).or_else(|| doc.get("MaxHealth")).and_then(|t| t.as_f32()).unwrap_or(p.max_health);
        p.health = num(&["Health"]).unwrap_or(p.health);
        p.food = num(&["foodLevel", "Food"]).unwrap_or(p.food);
        p.xp_level = int(&["XpLevel"]).unwrap_or(0).clamp(0, i32::MAX as i64) as i32;
        p.xp_progress = num(&["XpP", "XpProgress"]).unwrap_or(0.0);
        // PocketMine note les dates en millisecondes
        let date = |ms: &str, secs: &str| int(&[ms]).map(|v| v / 1000).or_else(|| int(&[secs])).unwrap_or(0).max(0) as u64;
        p.first_played = date("firstPlayed", "FirstPlayed");
        p.last_played = date("lastPlayed", "LastPlayed");
        p.game_type = int(&["playerGameType"]).filter(|g| (0..=1).contains(g)).map(|g| g as i32);
        p.level = doc.str_of("Level").map(|s| clean_text(s, 64)).unwrap_or_default();
        if let Some(pos) = doc.get("Pos").and_then(|t| t.as_list()) {
            let v: Vec<f64> = pos
                .iter()
                .filter_map(|t| match t {
                    Tag::Double(d) => Some(*d),
                    other => other.as_f32().map(|f| f as f64),
                })
                .collect();
            if v.len() == 3 {
                // Le fichier note la position des pieds ; le serveur travaille à hauteur des yeux
                let y = if old { v[1] } else { v[1] + EYE_HEIGHT as f64 };
                p.pos = (v[0] as f32, y as f32, v[2] as f32);
                p.has_pos = true;
            }
        }
        let item_of = |item: &Tag| -> Option<(String, u8)> {
            let count = item.get("Count").and_then(|t| t.as_i64()).unwrap_or(0);
            match (item.str_of("Name"), count) {
                (Some(n), 1..=64) => Some((n.to_string(), count as u8)),
                _ => None,
            }
        };
        for item in doc.get("Inventory").and_then(|t| t.as_list()).unwrap_or(&[]).iter().take(64) {
            let slot = item.get("Slot").and_then(|t| t.as_i64()).unwrap_or(-1);
            let (name, count) = match item_of(item) {
                Some(v) => v,
                None => continue,
            };
            match (item.str_of("Container"), slot) {
                // ancien format de Basalt : le conteneur est écrit en toutes lettres
                (Some(container), 0..=255) => p.items.push((container.to_string(), slot as u8, name, count)),
                // PocketMine : sac de 9 à 44 (la barre d'abord), armure de 100 à 103
                (None, 9..=44) => p.items.push(("main".to_string(), (slot - 9) as u8, name, count)),
                (None, 100..=103) => p.items.push(("armor".to_string(), (slot - 100) as u8, name, count)),
                _ => {}
            }
        }
        if let Some((name, count)) = doc.get("OffHandItem").and_then(item_of) {
            p.items.push(("offhand".to_string(), 0, name, count));
        }
        let perms = own.and_then(|o| o.get("Permissions")).or_else(|| doc.get("Permissions"));
        for perm in perms.and_then(|t| t.as_list()).unwrap_or(&[]).iter().take(256) {
            if let Some(node) = perm.str_of("Name").map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty() && n.len() <= 64) {
                let granted = perm.get("Granted").and_then(|t| t.as_i64()).unwrap_or(0) != 0;
                p.permissions.retain(|(n, _)| *n != node);
                p.permissions.push((node, granted));
            }
        }
        p.sanitize();
        p
    }

    /// Ancien format texte (`players/<pseudo>.txt`) : relu une dernière fois, puis remplacé
    /// par le fichier `.nbt` à la prochaine sauvegarde.
    fn load_legacy(path: &Path, name: &str) -> Result<Option<Player>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut p = Player::new(name, 0);
        for line in text.lines() {
            let (key, value) = match line.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let num = || value.trim().parse::<f32>().ok();
            match key.trim() {
                "display" => p.display_name = value.to_string(),
                "tag" => p.name_tag = value.to_string(),
                "health" => p.health = num().unwrap_or(p.health),
                "max_health" => p.max_health = num().unwrap_or(p.max_health),
                "food" => p.food = num().unwrap_or(p.food),
                "xp_level" => p.xp_level = value.trim().parse().unwrap_or(0),
                "xp_progress" => p.xp_progress = num().unwrap_or(0.0),
                "first_played" => p.first_played = value.trim().parse().unwrap_or(0),
                "last_played" => p.last_played = value.trim().parse().unwrap_or(0),
                "pos" => {
                    let v: Vec<f32> = value.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if v.len() == 3 {
                        p.pos = (v[0], v[1], v[2]);
                        p.has_pos = true;
                    }
                }
                "perm" => {
                    if let Some((node, granted)) = value.split_once(',') {
                        p.permissions.push((node.trim().to_lowercase(), granted.trim() == "true"));
                    }
                }
                "item" => {
                    let v: Vec<&str> = value.split(',').collect();
                    if let (4, Some(slot), Some(count)) = (v.len(), v.get(1).and_then(|s| s.parse().ok()), v.get(3).and_then(|s| s.parse().ok())) {
                        if p.items.len() < 64 {
                            p.items.push((v[0].to_string(), slot, v[2].to_string(), count));
                        }
                    }
                }
                _ => {}
            }
        }
        p.sanitize();
        Ok(Some(p))
    }

    /// Le joueur au format des fichiers de PocketMine : mêmes noms, mêmes types. Ce que
    /// PocketMine ne connaît pas (nom affiché, vie maximale, permissions) est rangé dans une
    /// section « Basalt », qu'il ignore.
    fn to_nbt(&self) -> Tag {
        let doubles = |v: [f64; 3]| Tag::List(6, v.iter().map(|x| Tag::Double(*x)).collect());
        let item = |slot: Option<i8>, name: &str, count: u8| {
            let mut e = vec![("Count", Tag::Byte(count as i8)), ("Damage", Tag::Short(0)), ("Name", Tag::string(name))];
            if let Some(s) = slot {
                e.push(("Slot", Tag::Byte(s)));
            }
            Tag::compound(e)
        };
        let feet = [self.pos.0 as f64, self.pos.1 as f64 - EYE_HEIGHT as f64, self.pos.2 as f64];
        let mut entries = vec![
            ("NameTag", Tag::string(&self.name_tag)),
            ("Motion", doubles([0.0, 0.0, 0.0])),
            ("Rotation", Tag::List(5, vec![Tag::Float(0.0), Tag::Float(0.0)])),
            ("FallDistance", Tag::Float(0.0)),
            ("Fire", Tag::Short(0)),
            ("OnGround", Tag::Byte(1)),
            ("Health", Tag::Float(self.health)),
            ("Air", Tag::Short(300)),
            ("foodLevel", Tag::Int(self.food.round() as i32)),
            ("foodExhaustionLevel", Tag::Float(0.0)),
            ("foodSaturationLevel", Tag::Float(5.0)),
            ("foodTickTimer", Tag::Int(0)),
            ("XpLevel", Tag::Int(self.xp_level)),
            ("XpP", Tag::Float(self.xp_progress)),
            ("XpTotal", Tag::Int(0)),
            ("XpSeed", Tag::Int(0)),
            ("SelectedInventorySlot", Tag::Int(0)),
            ("firstPlayed", Tag::Long(self.first_played as i64 * 1000)),
            ("lastPlayed", Tag::Long(self.last_played as i64 * 1000)),
            ("Level", Tag::string(&self.level)),
        ];
        if self.has_pos {
            entries.push(("Pos", doubles(feet)));
        }
        if let Some(g) = self.game_type {
            entries.push(("playerGameType", Tag::Int(g)));
        }
        let mut inventory = Vec::new();
        for (container, slot, name, count) in &self.items {
            match container.as_str() {
                "main" if *slot < 36 => inventory.push(item(Some(*slot as i8 + 9), name, *count)),
                "armor" if *slot < 4 => inventory.push(item(Some(100 + *slot as i8), name, *count)),
                "offhand" => entries.push(("OffHandItem", item(None, name, *count))),
                _ => {}
            }
        }
        // Une liste vide doit quand même annoncer le type de ses éléments (sections)
        entries.push(("Inventory", if inventory.is_empty() { Tag::List(10, Vec::new()) } else { Tag::list_of(inventory) }));
        entries.push(("EnderChestInventory", Tag::List(10, Vec::new())));
        let perms: Vec<Tag> = self.permissions.iter().map(|(node, granted)| Tag::compound(vec![("Name", Tag::string(node)), ("Granted", Tag::Byte(*granted as i8))])).collect();
        entries.push((
            "Basalt",
            Tag::compound(vec![
                ("DisplayName", Tag::string(&self.display_name)),
                ("MaxHealth", Tag::Float(self.max_health)),
                ("Permissions", if perms.is_empty() { Tag::List(10, Vec::new()) } else { Tag::list_of(perms) }),
            ]),
        ));
        Tag::compound(entries)
    }

    /// Sauvegarde dans `players/<pseudo>.dat` ; les anciens fichiers du joueur sont retirés.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = Self::path(dir, &self.name);
        nbt::save_file(&path, &self.to_nbt())?;
        let _ = std::fs::remove_file(path.with_extension("nbt"));
        let _ = std::fs::remove_file(path.with_extension("txt"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_comes_back_as_he_left() {
        let dir = std::env::temp_dir().join(format!("basalt-player-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(PlayerData::load(&dir, "Enzo").unwrap(), None);
        let mut p = PlayerData::new("Enzo", 1000);
        p.health = 7.5;
        p.max_health = 40.0;
        p.xp_level = 12;
        p.xp_progress = 0.25;
        p.display_name = "§6Enzo le Grand".into();
        p.name_tag = "[Admin] Enzo".into();
        p.pos = (12.5, -58.0, -3.25);
        p.has_pos = true;
        p.last_played = 2000;
        p.items.push(("main".into(), 3, "minecraft:oak_planks".into(), 64));
        p.permissions.push(("gardien.expulser".into(), true));
        p.permissions.push(("monde.casser".into(), false));
        p.game_type = Some(1);
        p.level = "world".into();
        p.items.push(("armor".into(), 1, "minecraft:iron_chestplate".into(), 1));
        p.items.push(("offhand".into(), 0, "minecraft:shield".into(), 1));
        p.save(&dir).unwrap();
        assert_eq!(PlayerData::load(&dir, "Enzo").unwrap(), Some(p.clone()));
        // Le pseudo ne permet pas de sortir du dossier
        assert_eq!(PlayerData::path(&dir, "../../x").file_name().unwrap(), "______x.dat");
        // Le fichier suit la disposition de PocketMine : mêmes noms, mêmes types
        let doc = nbt::load_file(&PlayerData::path(&dir, "Enzo")).unwrap().unwrap();
        assert_eq!(doc.get("Pos").and_then(|t| t.as_list()).map(|l| l.len()), Some(3));
        assert!(matches!(doc.get("Pos").and_then(|t| t.as_list()).and_then(|l| l.first()), Some(Tag::Double(_))), "positions en nombres doubles");
        let feet_y = match doc.get("Pos").and_then(|t| t.as_list()).map(|l| &l[1]) {
            Some(Tag::Double(y)) => *y,
            _ => f64::NAN,
        };
        assert!((feet_y - (-59.62)).abs() < 1e-5, "hauteur des pieds, pas des yeux : {}", feet_y);
        assert_eq!(doc.get("foodLevel").and_then(|t| t.as_i64()), Some(20));
        assert_eq!(doc.get("XpLevel").and_then(|t| t.as_i64()), Some(12));
        assert_eq!(doc.get("lastPlayed").and_then(|t| t.as_i64()), Some(2_000_000), "dates en millisecondes");
        assert_eq!(doc.get("playerGameType").and_then(|t| t.as_i64()), Some(1));
        assert_eq!(doc.str_of("Level"), Some("world"));
        let first = doc.get("Inventory").and_then(|t| t.as_list()).and_then(|l| l.first()).cloned().unwrap();
        assert_eq!((first.get("Slot").and_then(|t| t.as_i64()), first.str_of("Name")), (Some(12), Some("minecraft:oak_planks")), "case 3 du sac = Slot 12");
        assert_eq!(doc.get("Basalt").and_then(|b| b.str_of("DisplayName")), Some("§6Enzo le Grand"));
        // Un ancien fichier de Basalt (.nbt, hauteur des yeux) est relu tel quel
        let old_style = Tag::compound(vec![
            ("DisplayName", Tag::string("Vieux")),
            ("Food", Tag::Float(11.0)),
            ("FirstPlayed", Tag::Long(77)),
            ("Pos", Tag::List(5, vec![Tag::Float(1.0), Tag::Float(64.0), Tag::Float(2.0)])),
            ("Inventory", Tag::list_of(vec![Tag::compound(vec![("Container", Tag::string("main")), ("Slot", Tag::Byte(2)), ("Name", Tag::string("minecraft:stick")), ("Count", Tag::Byte(7))])])),
        ]);
        nbt::save_file(&PlayerData::path(&dir, "Vieux").with_extension("nbt"), &old_style).unwrap();
        let vieux = PlayerData::load(&dir, "Vieux").unwrap().unwrap();
        assert_eq!((vieux.display_name.as_str(), vieux.food, vieux.first_played, vieux.pos), ("Vieux", 11.0, 77, (1.0, 64.0, 2.0)));
        assert_eq!(vieux.items, vec![("main".to_string(), 2, "minecraft:stick".to_string(), 7)]);
        vieux.save(&dir).unwrap();
        assert!(!PlayerData::path(&dir, "Vieux").with_extension("nbt").exists() && PlayerData::path(&dir, "Vieux").exists());
        assert_eq!(PlayerData::load(&dir, "Vieux").unwrap(), Some(vieux));
        // Un fichier abîmé est remis dans les bornes
        let broken = Tag::compound(vec![("Health", Tag::Float(f32::NAN)), ("MaxHealth", Tag::Float(-5.0)), ("XpLevel", Tag::Int(999_999)), ("DisplayName", Tag::string(""))]);
        nbt::save_file(&PlayerData::path(&dir, "Enzo"), &broken).unwrap();
        let fixed = PlayerData::load(&dir, "Enzo").unwrap().unwrap();
        assert_eq!((fixed.health, fixed.max_health, fixed.xp_level), (1.0, 1.0, 24_791));
        assert_eq!(fixed.display_name, "Enzo");
        // Ancien fichier texte : relu, puis remplacé par le .nbt à la sauvegarde suivante
        let old = PlayerData::path(&dir, "Lea").with_extension("txt");
        std::fs::write(&old, "display=Lea la Sage\nhealth=9\nxp_level=4\npos=1,2,3\nitem=main,0,minecraft:stick,5\nperm=a.b,true\n").unwrap();
        let lea = PlayerData::load(&dir, "Lea").unwrap().unwrap();
        assert_eq!((lea.display_name.as_str(), lea.health, lea.xp_level, lea.pos, lea.items.len(), lea.permissions.len()), ("Lea la Sage", 9.0, 4, (1.0, 2.0, 3.0), 1, 1));
        lea.save(&dir).unwrap();
        assert!(!old.exists() && PlayerData::path(&dir, "Lea").exists());
        assert_eq!(PlayerData::load(&dir, "Lea").unwrap(), Some(lea));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
