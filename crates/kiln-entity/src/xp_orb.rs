//! `ExperienceOrb`.

use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel};

#[derive(Clone, Debug)]
pub struct OrbData {
    pub value: i32,
    pub count: i32,
    pub age: i32,
    pub health: i32,
}

pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    e.base_tick(level);
}

pub fn hurt(_e: &mut Entity, _level: &mut dyn EntityLevel, _kind: DamageKind, _amount: f32) -> bool {
    false
}
