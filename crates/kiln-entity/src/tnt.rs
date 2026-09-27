//! `PrimedTnt`.

use crate::entity::Entity;
use crate::level::EntityLevel;

#[derive(Clone, Debug)]
pub struct TntData {
    pub fuse: i32,
    pub block_state: u16,
    pub explosion_power: f32,
    pub owner: Option<i32>,
}

pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    e.base_tick(level);
}
