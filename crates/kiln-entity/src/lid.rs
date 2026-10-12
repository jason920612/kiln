//! The lid of a shulker box and the entities in its way (`ShulkerBoxBlockEntity.getBoundingBox`,
//! `moveCollidedEntities`, and the space `ShulkerBoxBlock.canOpen` asks to be free).
//!
//! The animation itself lives in kiln-blocks (`behaviour::shulker`); the level hands the lid's
//! shown progress over ([`EntityLevel::shulker_lid`]) for the box's shape, and the region pushes
//! with a [`LidPlan`] for each tick that lifted a lid, so its players, which are not entities of
//! the level, are pushed with the same arithmetic.

use crate::entity::{Entity, MoverType};
use crate::level::{EntityFilter, EntityLevel};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::mob::kinds::shulker::{progress_aabb, progress_delta_aabb};
use crate::shape::Shape;

/// `ShulkerBoxBlockEntity.getBoundingBox`, in block coordinates: `Shulker.getProgressAabb(1, facing, 0.5 * progress, (0.5, 0, 0.5))`
/// for the lid's shown progress (`getProgress(1.0f)`).
pub fn lid_box(facing: Direction, shown: f32) -> Aabb {
    progress_aabb(1.0, facing, 0.5 * shown, Vec3::new(0.5, 0.0, 0.5))
}

/// `ShulkerBoxBlock.getShape` of a box whose lid is at `shown`: the box of [`lid_box`].
pub fn lid_shape(facing: Direction, shown: f32) -> Option<Shape> {
    Shape::from_box(&lid_box(facing, shown))
}

/// What one tick of an opening lid asks of the entities around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LidPlan {
    /// Where entities are looked for: the strip the lid covered this tick.
    pub area: Aabb,
    /// What each entity found is moved by (`moveCollidedEntities`).
    pub movement: Vec3,
}

/// `moveCollidedEntities`' first half for the box at `pos` whose lid went from `from` to `to`.
pub fn plan(pos: BlockPos, facing: Direction, from: f32, to: f32) -> LidPlan {
    let area = progress_delta_aabb(1.0, facing, from, to, Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5));
    let (sx, sy, sz) = facing.step();
    let movement = Vec3::new((area.x_size() + 0.01) * sx as f64, (area.y_size() + 0.01) * sy as f64, (area.z_size() + 0.01) * sz as f64);
    LidPlan { area, movement }
}

/// What `moveCollidedEntities` does to one entity of the strip.
pub fn push_entity(plan: &LidPlan, e: &mut Entity, level: &mut dyn EntityLevel) {
    if crate::piston::ignored_by_pistons(e) {
        return;
    }
    e.do_move(level, MoverType::ShulkerBox, plan.movement);
}

/// `moveCollidedEntities` for the level's entities, in the order the level finds them (players are the caller's).
pub fn move_entities(level: &mut dyn EntityLevel, plan: &LidPlan) {
    for id in level.entities_in(&plan.area, EntityFilter::Any, -1) {
        if level.entity(id).is_some_and(|e| e.type_name == "minecraft:player") {
            continue;
        }
        level.with_entity_taken(id, &mut |e, lv| push_entity(plan, e, lv));
    }
}

/// The strip `ShulkerBoxBlock.canOpen` wants free of collisions: where the lid goes on its way up, a hair smaller.
pub fn opening_space(pos: BlockPos, facing: Direction) -> Aabb {
    progress_delta_aabb(1.0, facing, 0.0, 0.5, Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5)).deflate_all(1.0e-6)
}

/// The `facing` of a shulker box state (the way its lid opens).
pub fn facing_of(state: u16) -> Direction {
    match kiln_data::blocks_types::block_of(state).property(state, "facing") {
        Some("down") => Direction::Down,
        Some("north") => Direction::North,
        Some("south") => Direction::South,
        Some("west") => Direction::West,
        Some("east") => Direction::East,
        _ => Direction::Up,
    }
}
