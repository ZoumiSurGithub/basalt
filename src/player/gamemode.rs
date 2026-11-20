//! Modes de jeu : survie, créatif, aventure, spectateur.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    /// D'après un nom ou un numéro, comme la commande /gamemode du jeu : `survival`, `s`, `0`,
    /// `creative`, `c`, `1`, `adventure`, `a`, `2`, `spectator`, `sp`, `6` (et les noms français).
    pub fn parse(text: &str) -> Option<GameMode> {
        match text.trim().to_lowercase().as_str() {
            "survival" | "s" | "0" | "survie" => Some(GameMode::Survival),
            "creative" | "c" | "1" | "créatif" | "creatif" => Some(GameMode::Creative),
            "adventure" | "a" | "2" | "aventure" => Some(GameMode::Adventure),
            "spectator" | "sp" | "6" | "3" | "spectateur" => Some(GameMode::Spectator),
            _ => None,
        }
    }

    /// Numéro envoyé au jeu (SetPlayerGameType, StartGame) : 6 pour spectateur.
    pub fn bedrock_id(self) -> i32 {
        match self {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
            GameMode::Adventure => 2,
            GameMode::Spectator => 6,
        }
    }

    /// Numéro enregistré dans le fichier du joueur (comme PocketMine : 0 à 3), aussi donné
    /// aux plugins.
    pub fn save_id(self) -> i32 {
        match self {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
            GameMode::Adventure => 2,
            GameMode::Spectator => 3,
        }
    }

    pub fn from_save_id(id: i32) -> GameMode {
        match id {
            1 => GameMode::Creative,
            2 => GameMode::Adventure,
            3 | 6 => GameMode::Spectator,
            _ => GameMode::Survival,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GameMode::Survival => "Survie",
            GameMode::Creative => "Créatif",
            GameMode::Adventure => "Aventure",
            GameMode::Spectator => "Spectateur",
        }
    }

    /// Peut casser et poser des blocs.
    pub fn can_build(self) -> bool {
        matches!(self, GameMode::Survival | GameMode::Creative)
    }

    /// Prend des dégâts (chutes, coups).
    pub fn takes_damage(self) -> bool {
        matches!(self, GameMode::Survival | GameMode::Adventure)
    }

    /// Peut voler.
    pub fn may_fly(self) -> bool {
        matches!(self, GameMode::Creative | GameMode::Spectator)
    }

    /// Spectateur : invisible des autres, traverse les blocs, ne touche à rien.
    pub fn is_spectator(self) -> bool {
        self == GameMode::Spectator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_read_like_the_game_command() {
        for (text, mode) in [("survival", GameMode::Survival), ("c", GameMode::Creative), ("2", GameMode::Adventure), ("sp", GameMode::Spectator), ("Spectator", GameMode::Spectator)] {
            assert_eq!(GameMode::parse(text), Some(mode), "{}", text);
        }
        assert_eq!(GameMode::parse("hardcore"), None);
        assert_eq!(GameMode::Spectator.bedrock_id(), 6);
        for m in [GameMode::Survival, GameMode::Creative, GameMode::Adventure, GameMode::Spectator] {
            assert_eq!(GameMode::from_save_id(m.save_id()), m);
        }
        assert!(!GameMode::Adventure.can_build() && GameMode::Adventure.takes_damage());
        assert!(GameMode::Spectator.may_fly() && !GameMode::Spectator.takes_damage());
    }
}
