//! `Living` : une entité vivante. Ajoute la vie à `Entity`.

use super::{extends, Entity};

#[derive(Debug, Clone, PartialEq)]
pub struct Living {
    pub entity: Entity,
    pub health: f32,
    pub max_health: f32,
}
extends!(Living, entity, Entity);

/// À zéro, l'entité est morte.
const MIN_HEALTH: f32 = 0.0;

impl Living {
    /// Créature en pleine santé.
    pub fn new(name_tag: &str, max_health: f32) -> Living {
        Living { entity: Entity::new(name_tag), health: max_health, max_health }
    }

    /// Vie maximale, de 1 à 1024 ; la vie actuelle ne la dépasse pas.
    pub fn set_max_health(&mut self, value: f32) {
        self.max_health = if value.is_finite() { value.clamp(1.0, 1024.0) } else { 20.0 };
        self.health = self.health.min(self.max_health);
    }

    /// Vie, bornée par le maximum.
    pub fn set_health(&mut self, value: f32) {
        self.health = if value.is_finite() { value.clamp(MIN_HEALTH, self.max_health) } else { self.max_health };
    }

    /// Soigne d'une quantité positive.
    pub fn heal(&mut self, amount: f32) {
        if amount.is_finite() && amount > 0.0 {
            self.set_health(self.health + amount);
        }
    }

    /// Plus de vie ?
    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }

    /// Retire de la vie (les valeurs négatives sont ignorées).
    pub fn damage(&mut self, amount: f32) {
        if amount.is_finite() && amount > 0.0 {
            self.set_health(self.health - amount);
        }
    }

    /// Corrige des valeurs impossibles.
    pub fn sanitize(&mut self) {
        self.entity.sanitize();
        let max = self.max_health;
        self.set_max_health(max);
        let health = self.health;
        self.set_health(health);
        if !health.is_finite() {
            self.health = self.max_health;
        }
    }
}
