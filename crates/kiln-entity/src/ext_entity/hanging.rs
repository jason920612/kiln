//! What the hanging entities (`HangingEntity`: item frames, glow item frames, paintings) have in
//! common: a facing direction and the block they hang in (`pos`, the block in front of the wall
//! they are on), a bounding box worked out from both, a survival check every 100 ticks, and the
//! drops of a frame or painting that is hit or loses its wall.

use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::physics;
use kiln_proto::nbt::Tag;

/// `Direction.get3DDataValue`.
pub fn data_value(d: Direction) -> i32 {
    d.index() as i32
}

/// `Direction.from3DDataValue` (`LEGACY_ID_CODEC`).
pub fn from_data_value(v: i64) -> Option<Direction> {
    (0..6).contains(&v).then(|| Direction::ALL[v as usize])
}

/// `Direction.get2DDataValue`: south 0, west 1, north 2, east 3 (-1 for up and down).
pub fn data_value_2d(d: Direction) -> i32 {
    match d {
        Direction::South => 0,
        Direction::West => 1,
        Direction::North => 2,
        Direction::East => 3,
        _ => -1,
    }
}

/// `Direction.step()` as the position offset.
pub fn offset(pos: BlockPos, d: Direction) -> BlockPos {
    let (x, y, z) = d.step();
    BlockPos::new(pos.x + x, pos.y + y, pos.z + z)
}

/// `Vec3.atCenterOf(pos).relative(direction, distance)`.
pub fn center_relative(pos: BlockPos, d: Direction, distance: f64) -> Vec3 {
    let (x, y, z) = d.step();
    Vec3::new(pos.x as f64 + 0.5 + x as f64 * distance, pos.y as f64 + 0.5 + y as f64 * distance, pos.z as f64 + 0.5 + z as f64 * distance)
}

/// `AABB.ofSize(center, x, y, z)`.
pub fn of_size(c: Vec3, x: f64, y: f64, z: f64) -> Aabb {
    Aabb::new(c.x - x / 2.0, c.y - y / 2.0, c.z - z / 2.0, c.x + x / 2.0, c.y + y / 2.0, c.z + z / 2.0)
}

/// `setDirection`'s rotations: horizontal facings turn the entity (`2D data value * 90`), up and
/// down tip it (`-90 * step`) (the painting only hangs on walls).
pub fn rotations(d: Direction) -> (f32, f32) {
    if d.axis() != crate::math::Axis::Y {
        (data_value_2d(d) as f32 * 90.0, 0.0)
    } else {
        (0.0, -90.0 * if d.is_positive() { 1.0 } else { -1.0 })
    }
}

/// What a hanging entity looks at to decide whether it survives: the blocks, the block shapes in
/// a box, and the other hanging entities.
pub trait HangingWorld {
    fn block(&self, pos: BlockPos) -> u16;
    /// `HangingEntity.hasLevelCollision`: a block shape in the box.
    fn block_collision(&self, bx: &Aabb) -> bool;
    /// The hanging entities other than `exclude` (an entity id) whose box meets `bx`: facing and
    /// type name.
    fn hanging_in(&self, bx: &Aabb, exclude: i32) -> Vec<(Direction, &'static str)>;
}

/// The world as an entity level shows it.
pub struct LevelWorld<'a> {
    pub level: &'a dyn EntityLevel,
    pub e: &'a Entity,
}

impl HangingWorld for LevelWorld<'_> {
    fn block(&self, pos: BlockPos) -> u16 {
        self.level.block(pos)
    }

    fn block_collision(&self, bx: &Aabb) -> bool {
        let mut blocked = false;
        crate::collision::for_each_block_collision(self.level, &self.e.collision_context(), bx, |_, _, _| {
            blocked = true;
            false
        });
        blocked
    }

    fn hanging_in(&self, bx: &Aabb, exclude: i32) -> Vec<(Direction, &'static str)> {
        self.level
            .entities_in(bx, EntityFilter::Any, exclude)
            .into_iter()
            .filter_map(|id| self.level.entity(id))
            .filter(|o| o.id != exclude)
            .filter_map(|o| direction_of(o).map(|d| (d, o.type_name)))
            .collect()
    }
}

/// `HangingEntity.isSupportingBlock`: a solid block or a repeater or comparator.
pub fn is_supporting_block(state: u16) -> bool {
    physics::is_solid(state) || crate::blocks::is_diode(state)
}

/// `HangingEntity.canCoexist(checkSameType)`: no other hanging entity in the box that is of the
/// same type (when asked) or faces the same way.
pub fn can_coexist(world: &dyn HangingWorld, e: &Entity, direction: Direction, pop_box: &Aabb, same_type_counts: bool) -> bool {
    !world.hanging_in(pop_box, e.id).into_iter().any(|(d, t)| (same_type_counts && t == e.type_name) || d == direction)
}

/// The facing of a hanging entity.
pub fn direction_of(e: &Entity) -> Option<Direction> {
    if let Some(f) = crate::ext_entity::get::<crate::ext_entity::item_frame::ItemFrame>(e) {
        return Some(f.direction);
    }
    crate::ext_entity::get::<crate::ext_entity::painting::Painting>(e).map(|p| p.direction)
}

/// `BlockAttachedEntity.getPos` as read from saved data: `block_pos` when it is within 16 blocks
/// of where the entity is, else the block it is in.
pub fn saved_pos(tag: Option<&Tag>, at: Vec3) -> BlockPos {
    let here = BlockPos::containing(at.x, at.y, at.z);
    if let Some(Tag::IntArray(v)) = tag
        && v.len() == 3
    {
        let p = BlockPos::new(v[0], v[1], v[2]);
        let d = |a: i32, b: i32| (a as f64 - b as f64).powi(2);
        if d(p.x, here.x) + d(p.y, here.y) + d(p.z, here.z) < 256.0 {
            return p;
        }
    }
    here
}

/// Whether `e` is a hanging entity.
pub fn is_hanging(e: &Entity) -> bool {
    matches!(e.kind, EntityKind::Ext(_)) && direction_of(e).is_some()
}

/// The block a hanging entity hangs in.
pub fn block_pos_of(e: &Entity) -> Option<BlockPos> {
    if let Some(f) = crate::ext_entity::get::<crate::ext_entity::item_frame::ItemFrame>(e) {
        return Some(f.pos);
    }
    crate::ext_entity::get::<crate::ext_entity::painting::Painting>(e).map(|p| p.pos)
}
