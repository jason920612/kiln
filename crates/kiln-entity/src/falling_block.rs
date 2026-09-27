//! `FallingBlockEntity`.

use crate::entity::Entity;
use crate::level::EntityLevel;

#[derive(Clone, Debug)]
pub struct FallingBlockData {
    pub state: u16,
    pub time: i32,
    pub drop_item: bool,
    pub cancel_drop: bool,
    pub hurt_entities: bool,
    pub fall_damage_max: i32,
    pub fall_damage_per_distance: f32,
}

pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    e.base_tick(level);
}

pub fn cause_fall_damage(_e: &mut Entity, _level: &mut dyn EntityLevel, _distance: f64, _multiplier: f32) -> bool {
    false
}
