//! `Human` : un être humain (joueur ou personnage). Ajoute la faim et l'expérience à `Living`.

use super::{extends, Living};

#[derive(Debug, Clone, PartialEq)]
pub struct Human {
    pub living: Living,
    pub food: f32,
    pub xp_level: i32,
    /// Progression vers le niveau suivant, de 0 à 1
    pub xp_progress: f32,
}
extends!(Human, living, Living);

pub const MAX_XP_LEVEL: i32 = 24_791;

impl Human {
    /// Joueur en pleine forme, sans expérience.
    pub fn new(name_tag: &str) -> Human {
        Human { living: Living::new(name_tag, 20.0), food: 20.0, xp_level: 0, xp_progress: 0.0 }
    }

    /// Faim, de 0 à 20 (une valeur invalide remet 20).
    pub fn set_food(&mut self, value: f32) {
        self.food = if value.is_finite() { value.clamp(0.0, 20.0) } else { 20.0 };
    }

    /// Niveau d'expérience, borné.
    pub fn set_xp_level(&mut self, level: i32) {
        self.xp_level = level.clamp(0, MAX_XP_LEVEL);
    }

    /// Ajoute (ou retire) des niveaux.
    pub fn add_xp_levels(&mut self, levels: i32) {
        self.set_xp_level(self.xp_level.saturating_add(levels));
    }

    /// Progression vers le niveau suivant, de 0 à 1.
    pub fn set_xp_progress(&mut self, value: f32) {
        self.xp_progress = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
    }

    /// Corrige des valeurs impossibles (lues dans un fichier abîmé, par exemple).
    pub fn sanitize(&mut self) {
        self.living.sanitize();
        let (food, level, progress) = (self.food, self.xp_level, self.xp_progress);
        self.set_food(food);
        self.set_xp_level(level);
        self.set_xp_progress(progress);
    }
}
