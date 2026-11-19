//! `Entity` : la base de tout ce qui existe dans le monde.

use super::clean_text;

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub pos: (f32, f32, f32),
    /// Texte affiché au-dessus de l'entité
    pub name_tag: String,
}

impl Entity {
    pub fn new(name_tag: &str) -> Entity {
        Entity { pos: (0.0, 0.0, 0.0), name_tag: name_tag.to_string() }
    }

    pub fn has_valid_position(&self) -> bool {
        self.pos.0.is_finite() && self.pos.1.is_finite() && self.pos.2.is_finite()
    }

    pub fn set_name_tag(&mut self, text: &str) {
        self.name_tag = clean_text(text, 64);
    }

    pub fn distance_squared(&self, other: (f32, f32, f32)) -> f32 {
        let (dx, dy, dz) = (self.pos.0 - other.0, self.pos.1 - other.1, self.pos.2 - other.2);
        dx * dx + dy * dy + dz * dz
    }

    /// Remet les valeurs dans leurs bornes.
    pub fn sanitize(&mut self) {
        self.name_tag = clean_text(&self.name_tag, 64);
    }
}
