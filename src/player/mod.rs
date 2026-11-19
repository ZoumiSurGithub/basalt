//! `Player` : un joueur. Il hérite de `Human` (faim, expérience), donc de `Living` (vie) et
//! d'`Entity` (position, texte au-dessus de la tête), et ajoute ce qui tient au compte :
//! pseudo, nom affiché, permissions, dates de visite, inventaire sauvegardé.
//!
//! Le cycle est le même pour tous : `load` (ou `new` à la première visite) quand le joueur
//! entre, `save` quand il part, à intervalles réguliers et à l'arrêt du serveur.

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
}
extends!(Player, human, Human);

/// Ancien nom de `Player`.
pub type PlayerData = Player;

fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .take(32)
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("{}.nbt", if safe.is_empty() { "_".to_string() } else { safe })
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

    /// Chargement depuis `players/<pseudo>.nbt`. `Ok(None)` : le joueur n'a pas encore de fichier.
    pub fn load(dir: &Path, name: &str) -> Result<Option<Player>, String> {
        let path = Self::path(dir, name);
        let doc = match nbt::load_file(&path)? {
            Some(d) => d,
            None => return Self::load_legacy(&path.with_extension("txt"), name),
        };
        let mut p = Player::new(name, 0);
        let text = |key: &str| doc.str_of(key).map(|s| s.to_string());
        let num = |key: &str| doc.get(key).and_then(|t| t.as_f32());
        let int = |key: &str| doc.get(key).and_then(|t| t.as_i64());
        if let Some(v) = text("DisplayName") {
            p.display_name = v;
        }
        if let Some(v) = text("NameTag") {
            p.name_tag = v;
        }
        p.max_health = num("MaxHealth").unwrap_or(p.max_health);
        p.health = num("Health").unwrap_or(p.health);
        p.food = num("Food").unwrap_or(p.food);
        p.xp_level = int("XpLevel").unwrap_or(0).clamp(0, i32::MAX as i64) as i32;
        p.xp_progress = num("XpProgress").unwrap_or(0.0);
        p.first_played = int("FirstPlayed").unwrap_or(0).max(0) as u64;
        p.last_played = int("LastPlayed").unwrap_or(0).max(0) as u64;
        if let Some(pos) = doc.get("Pos").and_then(|t| t.as_list()) {
            let v: Vec<f32> = pos.iter().filter_map(|t| t.as_f32()).collect();
            if v.len() == 3 {
                p.pos = (v[0], v[1], v[2]);
                p.has_pos = true;
            }
        }
        for item in doc.get("Inventory").and_then(|t| t.as_list()).unwrap_or(&[]).iter().take(64) {
            let slot = item.get("Slot").and_then(|t| t.as_i64()).unwrap_or(-1);
            let count = item.get("Count").and_then(|t| t.as_i64()).unwrap_or(0);
            if let (Some(container), Some(name), 0..=255, 1..=64) = (item.str_of("Container"), item.str_of("Name"), slot, count) {
                p.items.push((container.to_string(), slot as u8, name.to_string(), count as u8));
            }
        }
        for perm in doc.get("Permissions").and_then(|t| t.as_list()).unwrap_or(&[]).iter().take(256) {
            if let Some(node) = perm.str_of("Name").map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty() && n.len() <= 64) {
                let granted = perm.get("Granted").and_then(|t| t.as_i64()).unwrap_or(0) != 0;
                p.permissions.retain(|(n, _)| *n != node);
                p.permissions.push((node, granted));
            }
        }
        p.sanitize();
        Ok(Some(p))
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

    /// Document NBT du joueur, tel qu'il est enregistré.
    pub fn to_nbt(&self) -> Tag {
        let mut entries = vec![
            ("Name", Tag::string(&clean_text(&self.name, 32))),
            ("DisplayName", Tag::string(&clean_text(&self.display_name, 32))),
            ("NameTag", Tag::string(&clean_text(&self.name_tag, 64))),
            ("Health", Tag::Float(self.health)),
            ("MaxHealth", Tag::Float(self.max_health)),
            ("Food", Tag::Float(self.food)),
            ("XpLevel", Tag::Int(self.xp_level)),
            ("XpProgress", Tag::Float(self.xp_progress)),
            ("FirstPlayed", Tag::Long(self.first_played as i64)),
            ("LastPlayed", Tag::Long(self.last_played as i64)),
        ];
        if self.has_pos {
            entries.push(("Pos", Tag::List(5, vec![Tag::Float(self.pos.0), Tag::Float(self.pos.1), Tag::Float(self.pos.2)])));
        }
        let items = self
            .items
            .iter()
            .map(|(container, slot, item, count)| {
                Tag::compound(vec![("Container", Tag::string(container)), ("Slot", Tag::Byte(*slot as i8)), ("Name", Tag::string(item)), ("Count", Tag::Byte(*count as i8))])
            })
            .collect();
        entries.push(("Inventory", Tag::list_of(items)));
        let perms = self.permissions.iter().map(|(node, granted)| Tag::compound(vec![("Name", Tag::string(node)), ("Granted", Tag::Byte(*granted as i8))])).collect();
        entries.push(("Permissions", Tag::list_of(perms)));
        Tag::compound(entries)
    }

    /// Sauvegarde dans `players/<pseudo>.nbt`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = Self::path(dir, &self.name);
        nbt::save_file(&path, &self.to_nbt())?;
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
        p.save(&dir).unwrap();
        assert_eq!(PlayerData::load(&dir, "Enzo").unwrap(), Some(p.clone()));
        // Le pseudo ne permet pas de sortir du dossier
        assert_eq!(PlayerData::path(&dir, "../../x").file_name().unwrap(), "______x.nbt");
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
