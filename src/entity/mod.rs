//! Entités, organisées comme dans PocketMine et Altay : chaque niveau ajoute ce qui lui est
//! propre et hérite du précédent.
//!
//! ```text
//! Entity   position, texte au-dessus de la tête
//!   └ Living   vie, vie maximale
//!       └ Human   faim, expérience
//!           └ Player   compte, nom affiché, permissions, sauvegarde   (module `player`)
//! ```
//!
//! L'héritage est rendu par composition : un `Human` contient un `Living`, qui contient une
//! `Entity`, et l'on accède directement aux données des niveaux du dessus
//! (`joueur.health`, `joueur.pos`).

mod entity;
pub mod item_physics;
mod human;
mod living;

pub use self::entity::Entity;
pub use self::human::Human;
pub use self::living::Living;

/// Donne à `$child` l'accès direct aux données et méthodes de `$parent` (champ `$field`).
macro_rules! extends {
    ($child:ty, $field:ident, $parent:ty) => {
        impl std::ops::Deref for $child {
            type Target = $parent;
            /// Une entité « hérite » des champs de son parent (Human -> Living -> Entity).
            fn deref(&self) -> &$parent {
                &self.$field
            }
        }
        impl std::ops::DerefMut for $child {
            fn deref_mut(&mut self) -> &mut $parent {
                &mut self.$field
            }
        }
    };
}
pub(crate) use extends;

/// Retire ce qui casserait un fichier de sauvegarde ou l'affichage.
pub fn clean_text(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_inherits_from_the_one_above() {
        let mut h = Human::new("Enzo");
        // données d'Entity et de Living lues et modifiées depuis un Human
        h.pos = (1.0, 2.0, 3.0);
        h.set_max_health(40.0);
        h.set_health(100.0);
        assert_eq!((h.health, h.max_health, h.name_tag.as_str()), (40.0, 40.0, "Enzo"));
        h.damage(15.0);
        assert_eq!(h.health, 25.0);
        h.heal(1000.0);
        assert_eq!(h.health, 40.0);
        h.damage(1000.0);
        assert!(h.is_dead() && h.health == 0.0);
        h.add_xp_levels(5);
        h.set_food(99.0);
        assert_eq!((h.xp_level, h.food), (5, 20.0));
        // valeurs abîmées remises dans les bornes à chaque niveau
        h.health = f32::NAN;
        h.max_health = -3.0;
        h.xp_progress = 7.0;
        h.pos = (f32::INFINITY, 0.0, 0.0);
        h.sanitize();
        assert_eq!((h.health, h.max_health, h.xp_progress), (1.0, 1.0, 1.0));
        assert!(!h.has_valid_position());
    }
}
