//! `Living` : une entité vivante. Ajoute la vie à `Entity`.

use super::{extends, Entity};

#[derive(Debug, Clone, PartialEq)]
pub struct Living {
    pub entity: Entity,
    pub health: f32,
    pub max_health: f32,
}
extends!(Living, entity, Entity);

/// La mort et la réapparition ne sont pas encore gérées : la vie ne descend pas plus bas.
const MIN_HEALTH: f32 = 1.0;

impl Living {
    pub fn new(name_tag: &str, max_health: f32) -> Living {
        Living { entity: Entity::new(name_tag), health: max_health, max_health }
    }

    pub fn set_max_health(&mut self, value: f32) {
        self.max_health = if value.is_finite() { value.clamp(1.0, 1024.0) } else { 20.0 };
        self.health = self.health.min(self.max_health);
    }

    pub fn set_health(&mut self, value: f32) {
        self.health = if value.is_finite() { value.clamp(MIN_HEALTH, self.max_health) } else { self.max_health };
    }

    pub fn heal(&mut self, amount: f32) {
        if amount.is_finite() && amount > 0.0 {
            self.set_health(self.health + amount);
        }
    }

    pub fn damage(&mut self, amount: f32) {
        if amount.is_finite() && amount > 0.0 {
            self.set_health(self.health - amount);
        }
    }

    pub fn sanitize(&mut self) {
        self.entity.sanitize();
        let max = self.max_health;
        self.set_max_health(max);
        let health = self.health;
        self.set_health(health);
        if !health.is_finite() {
            self.health = MIN_HEALTH.max(self.health.min(MIN_HEALTH));
        }
    }
}
