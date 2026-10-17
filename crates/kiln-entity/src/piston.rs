//! Pistons and entities: the collision shape of a moving block (`PistonMovingBlockEntity
//! .getCollisionShape`) and what a moving block does to the entities in its way
//! (`moveCollidedEntities`, `moveStuckEntities`, `fixEntityWithinPistonBase`).
//!
//! The block entity itself lives in kiln-blocks; the level hands its state over as a
//! [`MovingPistonView`]. The pushing is split in plans (what the block entity computes from
//! its state: where to look, what each box asks) and [`push_entity`], so the region can push
//! its players, which are not entities of the level, with the same arithmetic.

use crate::blocks::block_name;
use crate::entity::{Entity, MoverType};
use crate::level::{EntityFilter, EntityLevel};
use crate::math::{Aabb, Axis, BlockPos, Direction, Vec3};
use crate::physics;
use crate::shape::Shape;
use smallvec::SmallVec;
use std::borrow::Cow;

/// `PistonMovingBlockEntity`, as far as collision and pushing read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovingPistonView {
    /// The block in motion (for the piston itself: its head when extending, its base when retracting).
    pub moved: u16,
    /// The piston's facing.
    pub direction: Direction,
    pub extending: bool,
    /// The piston's own head or base rather than a pushed block.
    pub source: bool,
    pub progress: f32,
}

fn is_piston_base(state: u16) -> bool {
    kiln_data::block_logic::block_class(state) == kiln_data::block_logic::BlockClass::PistonBaseBlock
}

fn set_property(state: u16, name: &str, value: &str) -> u16 {
    kiln_data::blocks_types::block_of(state).with_property(state, name, value).unwrap_or(state)
}

fn direction_name(d: Direction) -> &'static str {
    ["down", "up", "north", "south", "west", "east"][d.index()]
}

impl MovingPistonView {
    /// `getMovementDirection`: where the block is going.
    pub fn movement_direction(&self) -> Direction {
        if self.extending { self.direction } else { self.direction.opposite() }
    }

    /// `getExtendedProgress(progress)`: how far the block is from where it ends up (negative
    /// while extending, positive while retracting).
    pub fn extended_progress(&self, progress: f32) -> f32 {
        if self.extending { progress - 1.0 } else { 1.0 - progress }
    }

    /// `getCollisionRelatedBlockState`: a retracting piston base moves like its head.
    fn collision_related_state(&self) -> u16 {
        if !self.extending && self.source && is_piston_base(self.moved) {
            let facing = kiln_data::blocks_types::block_of(self.moved).property(self.moved, "facing").unwrap_or("north");
            let sticky = block_name(self.moved) == "minecraft:sticky_piston";
            let mut s = kiln_data::blocks::default_state::PISTON_HEAD;
            s = set_property(s, "short", if self.progress > 0.25 { "true" } else { "false" });
            s = set_property(s, "type", if sticky { "sticky" } else { "normal" });
            set_property(s, "facing", facing)
        } else {
            self.moved
        }
    }

    /// `getCollisionShape(level, pos)`: the shapes the block has at its position (in vanilla one
    /// shape, the union). `noclip` is the direction the entity being pushed moves in (the
    /// `NOCLIP` thread-local): blocks moving that way give way to it.
    pub fn collision_shapes(&self, noclip: Option<Direction>) -> SmallVec<[Cow<'static, Shape>; 2]> {
        let mut out: SmallVec<[Cow<'static, Shape>; 2]> = SmallVec::new();
        let base: &'static Shape = if !self.extending && self.source && is_piston_base(self.moved) {
            physics::collision_shape(set_property(self.moved, "extended", "true"))
        } else {
            physics::empty_shape()
        };
        if !base.is_empty() {
            out.push(Cow::Borrowed(base));
        }
        if (self.progress as f64) < 1.0 && noclip == Some(self.movement_direction()) {
            return out;
        }
        let state = if self.source {
            let mut s = kiln_data::blocks::default_state::PISTON_HEAD;
            s = set_property(s, "facing", direction_name(self.direction));
            // (`SHORT` is `extending != (1 - progress < 0.25)`.)
            let short = self.extending != (1.0 - self.progress < 0.25);
            set_property(s, "short", if short { "true" } else { "false" })
        } else {
            self.moved
        };
        let f = self.extended_progress(self.progress);
        let (sx, sy, sz) = self.direction.step();
        let (dx, dy, dz) = ((sx as f32 * f) as f64, (sy as f32 * f) as f64, (sz as f32 * f) as f64);
        let moved = physics::collision_shape(state);
        if !moved.is_empty() {
            out.push(Cow::Owned(moved.moved(dx, dy, dz)));
        }
        out
    }
}

/// `PistonMath.getMovementArea`: the strip in front of `aabb`, `delta` thick, as it moves `dir`.
pub fn movement_area(aabb: &Aabb, dir: Direction, delta: f64) -> Aabb {
    let step = dir.step();
    let signed = delta * (step.0 + step.1 + step.2) as f64;
    let (lo, hi) = (signed.min(0.0), signed.max(0.0));
    match dir {
        Direction::West => Aabb::new(aabb.min_x + lo, aabb.min_y, aabb.min_z, aabb.min_x + hi, aabb.max_y, aabb.max_z),
        Direction::East => Aabb::new(aabb.max_x + lo, aabb.min_y, aabb.min_z, aabb.max_x + hi, aabb.max_y, aabb.max_z),
        Direction::Down => Aabb::new(aabb.min_x, aabb.min_y + lo, aabb.min_z, aabb.max_x, aabb.min_y + hi, aabb.max_z),
        Direction::Up => Aabb::new(aabb.min_x, aabb.max_y + lo, aabb.min_z, aabb.max_x, aabb.max_y + hi, aabb.max_z),
        Direction::North => Aabb::new(aabb.min_x, aabb.min_y, aabb.min_z + lo, aabb.max_x, aabb.max_y, aabb.min_z + hi),
        Direction::South => Aabb::new(aabb.min_x, aabb.min_y, aabb.max_z + lo, aabb.max_x, aabb.max_y, aabb.max_z + hi),
    }
}

/// `getMovement(area, dir, entityBox)`: how far the entity has to go to leave the area.
fn movement_needed(area: &Aabb, dir: Direction, e: &Aabb) -> f64 {
    match dir {
        Direction::East => area.max_x - e.min_x,
        Direction::West => e.max_x - area.min_x,
        Direction::Down => e.max_y - area.min_y,
        Direction::South => area.max_z - e.min_z,
        Direction::North => e.max_z - area.min_z,
        Direction::Up => area.max_y - e.min_y,
    }
}

/// `moveByPositionAndProgress`: `aabb` (in block coordinates) where the block is now.
fn at_progress(pos: BlockPos, aabb: &Aabb, view: &MovingPistonView) -> Aabb {
    let f = view.extended_progress(view.progress) as f64;
    let (sx, sy, sz) = view.direction.step();
    aabb.offset(pos.x as f64 + f * sx as f64, pos.y as f64 + f * sy as f64, pos.z as f64 + f * sz as f64)
}

/// `VoxelShape.bounds()` of a non-empty shape.
fn bounds(shape: &Shape) -> Aabb {
    Aabb::new(shape.min(Axis::X, 0.0), shape.min(Axis::Y, 0.0), shape.min(Axis::Z, 0.0), shape.max(Axis::X, 0.0), shape.max(Axis::Y, 0.0), shape.max(Axis::Z, 0.0))
}

/// What `moveCollidedEntities` works from for one tick of one block.
#[derive(Clone, Debug)]
pub struct CollidedPlan {
    pub pos: BlockPos,
    view: MovingPistonView,
    pub dir: Direction,
    /// `progress_after - progress_before`.
    pub movement: f64,
    /// The boxes of the block's collision shape (`toAabbs`), in block coordinates.
    boxes: Vec<Aabb>,
    /// Where entities are looked for.
    pub query: Aabb,
    pub slime: bool,
}

/// `moveCollidedEntities`' first half: `None` when the block has no collision shape.
pub fn plan_collided(pos: BlockPos, view: &MovingPistonView, next: f32) -> Option<CollidedPlan> {
    let dir = view.movement_direction();
    let movement = (next - view.progress) as f64;
    let shape = physics::collision_shape(view.collision_related_state());
    if shape.is_empty() {
        return None;
    }
    let total = at_progress(pos, &bounds(shape), view);
    let query = movement_area(&total, dir, movement).minmax(&total);
    Some(CollidedPlan { pos, view: *view, dir, movement, boxes: shape.boxes().to_vec(), query, slime: block_name(view.moved) == "minecraft:slime_block" })
}

impl CollidedPlan {
    /// How far the box of an entity has to be pushed (before the 0.01 cushion), `<= 0` when the
    /// block does not reach it.
    fn needed(&self, bb: &Aabb) -> f64 {
        let mut delta = 0.0f64;
        for b in &self.boxes {
            let area = movement_area(&at_progress(self.pos, b, &self.view), self.dir, self.movement);
            if area.intersects(bb) {
                delta = delta.max(movement_needed(&area, self.dir, bb));
                if delta >= self.movement {
                    break;
                }
            }
        }
        delta
    }
}

/// `Entity.getPistonPushReaction() == IGNORE_ENTITY`.
pub fn ignored_by_pistons(e: &Entity) -> bool {
    match e.type_name {
        "minecraft:area_effect_cloud" | "minecraft:interaction" | "minecraft:marker" | "minecraft:ominous_item_spawner" | "minecraft:block_display" | "minecraft:item_display" | "minecraft:text_display" => true,
        "minecraft:armor_stand" => crate::ext_entity::get::<crate::ext_entity::armor_stand::ArmorStand>(e).is_some_and(|a| a.marker),
        _ => false,
    }
}

/// `moveEntityByPiston`: the entity moves `amount` along `toward` through a world whose blocks
/// moving `noclip` do not stop it, and the blocks it passed through act on it.
pub fn move_entity_by_piston(e: &mut Entity, level: &mut dyn EntityLevel, noclip: Direction, amount: f64, toward: Direction) {
    e.piston_noclip = Some(noclip);
    let from = e.position();
    let (sx, sy, sz) = toward.step();
    e.do_move(level, MoverType::Piston, Vec3::new(amount * sx as f64, amount * sy as f64, amount * sz as f64));
    let to = e.position();
    e.apply_effects_between(level, from, to);
    e.remove_latest_movement_recording();
    e.piston_noclip = None;
}

/// `fixEntityWithinPistonBase`: a retracting piston base that ends up inside an entity pushes
/// it back out.
fn fix_within_base(pos: BlockPos, e: &mut Entity, level: &mut dyn EntityLevel, dir: Direction, movement: f64) {
    let bb = e.bounding_box();
    let base = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0).offset(pos.x as f64, pos.y as f64, pos.z as f64);
    if !bb.intersects(&base) {
        return;
    }
    let back = dir.opposite();
    let a = movement_needed(&base, back, &bb) + 0.01;
    let b = movement_needed(&base, back, &bb.intersect(&base)) + 0.01;
    if (a - b).abs() < 0.01 {
        let a = a.min(movement) + 0.01;
        move_entity_by_piston(e, level, dir, a, back);
    }
}

/// What `moveCollidedEntities` does to one entity of the box it looked in.
pub fn push_entity(plan: &CollidedPlan, e: &mut Entity, level: &mut dyn EntityLevel) {
    if ignored_by_pistons(e) {
        return;
    }
    if plan.slime && simulates_movement(e, level) {
        let mut v = e.delta;
        let (sx, sy, sz) = plan.dir.step();
        match plan.dir.axis() {
            Axis::X => v.x = sx as f64,
            Axis::Y => v.y = sy as f64,
            Axis::Z => v.z = sz as f64,
        }
        e.delta = v;
    }
    let needed = plan.needed(&e.bounding_box());
    if needed <= 0.0 {
        return;
    }
    let amount = needed.min(plan.movement) + 0.01;
    move_entity_by_piston(e, level, plan.dir, amount, plan.dir);
    if !plan.view.extending && plan.view.source {
        fix_within_base(plan.pos, e, level, plan.dir, plan.movement);
    }
}

/// `Entity.canSimulateMovement()` on the server: false for what a player drives (a boat, a
/// saddled mount: the player's client moves it), true for everything else and for players.
pub fn simulates_movement(e: &Entity, level: &dyn EntityLevel) -> bool {
    use crate::entity::EntityKind;
    match e.kind {
        EntityKind::Item(_) | EntityKind::ExperienceOrb(_) | EntityKind::FallingBlock(_) | EntityKind::Tnt(_) | EntityKind::Throwable(_) | EntityKind::Arrow(_) | EntityKind::Player(_) => return true,
        _ => {}
    }
    if e.type_name == "minecraft:player" {
        return true;
    }
    // (Minecarts have no controlling passenger.)
    if e.type_name.ends_with("minecart") {
        return true;
    }
    !e.passengers.first().is_some_and(|&p| level.player(p).is_some())
}

/// `moveStuckEntities`' plan: honey drags what stands on it, sideways.
#[derive(Clone, Debug)]
pub struct StuckPlan {
    pub pos: BlockPos,
    pub dir: Direction,
    pub movement: f64,
    pub area: Aabb,
}

pub fn plan_stuck(pos: BlockPos, view: &MovingPistonView, next: f32) -> Option<StuckPlan> {
    if block_name(view.moved) != "minecraft:honey_block" {
        return None;
    }
    let dir = view.movement_direction();
    if dir.axis() == Axis::Y {
        return None;
    }
    let top = physics::collision_shape(view.moved).max(Axis::Y, 0.0);
    let area = at_progress(pos, &Aabb::new(0.0, top, 0.0, 1.0, 1.5000010000000001, 1.0), view);
    Some(StuckPlan { pos, dir, movement: (next - view.progress) as f64, area })
}

impl StuckPlan {
    /// `matchesStickyCritera`.
    pub fn matches(&self, e: &Entity) -> bool {
        !ignored_by_pistons(e)
            && e.on_ground
            && (e.main_supporting_block_pos == Some(self.pos)
                || (e.x() >= self.area.min_x && e.x() <= self.area.max_x && e.z() >= self.area.min_z && e.z() <= self.area.max_z))
    }
}

/// `moveCollidedEntities` then `moveStuckEntities` for the entities of the level (players are
/// the caller's: their ids come back, with the plans for them to use).
pub fn move_entities(level: &mut dyn EntityLevel, pos: BlockPos, view: &MovingPistonView, next: f32) -> (Option<CollidedPlan>, Option<StuckPlan>) {
    let collided = plan_collided(pos, view, next);
    if let Some(plan) = &collided {
        for id in level.entities_in(&plan.query, EntityFilter::Any, -1) {
            if level.entity(id).is_some_and(|e| e.type_name == "minecraft:player") {
                continue;
            }
            level.with_entity_taken(id, &mut |e, lv| push_entity(plan, e, lv));
        }
    }
    let stuck = plan_stuck(pos, view, next);
    if let Some(plan) = &stuck {
        for id in level.entities_in(&plan.area, EntityFilter::Any, -1) {
            let hit = level.entity(id).is_some_and(|e| e.type_name != "minecraft:player" && plan.matches(e));
            if hit {
                level.with_entity_taken(id, &mut |e, lv| move_entity_by_piston(e, lv, plan.dir, plan.movement, plan.dir));
            }
        }
    }
    (collided, stuck)
}
