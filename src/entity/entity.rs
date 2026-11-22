//! `Entity` : la base de tout ce qui existe dans le monde.

use super::clean_text;

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub pos: (f32, f32, f32),
    /// Texte affiché au-dessus de l'entité
    pub name_tag: String,
}

impl Entity {
    /// Entité à l'origine, avec son nom affiché.
    pub fn new(name_tag: &str) -> Entity {
        Entity { pos: (0.0, 0.0, 0.0), name_tag: name_tag.to_string() }
    }

    /// Position utilisable (pas de NaN ni d'infini) ?
    pub fn has_valid_position(&self) -> bool {
        self.pos.0.is_finite() && self.pos.1.is_finite() && self.pos.2.is_finite()
    }

    /// Remet les valeurs dans leurs bornes.
    pub fn sanitize(&mut self) {
        self.name_tag = clean_text(&self.name_tag, 64);
    }
}
