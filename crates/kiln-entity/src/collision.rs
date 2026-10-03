//! Block and entity colliders around a box: vanilla's `BlockCollisions` iterator, the
//! `CollisionContext`-dependent block shapes, `collideBoundingBox` and `findSupportingBlock`.

use crate::blocks::{Kind, kind};
use crate::level::EntityLevel;
use crate::math::{Aabb, Axis, BlockPos, Vec3, floor};
use crate::physics;
use crate::shape::{BoxShape, Collider, Shape, collide_all, intersects_box};
use std::borrow::Cow;

/// `EntityCollisionContext`: what context-dependent collision shapes look at.
#[derive(Clone, Copy, Debug)]
pub struct CollisionContext {
    pub descending: bool,
    pub entity_bottom: f64,
    pub placement: bool,
    pub always_collide_with_fluid: bool,
    /// Whether this is an entity context at all (`EntityCollisionContext.Empty` otherwise).
    pub has_entity: bool,
    pub fall_distance: f64,
    pub falling_block: bool,
    /// `PowderSnowBlock.canEntityWalkOnPowderSnow` (leather boots, snow-walking mobs).
    pub walks_on_powder_snow: bool,
    /// `LivingEntity.canStandOnFluid(lava)` (striders).
    pub stands_on_lava: bool,
}

impl CollisionContext {
    /// `CollisionContext.empty()`.
    pub const EMPTY: CollisionContext = CollisionContext {
        descending: false,
        entity_bottom: -f64::MAX,
        placement: false,
        always_collide_with_fluid: false,
        has_entity: false,
        fall_distance: 0.0,
        falling_block: false,
        walks_on_powder_snow: false,
        stands_on_lava: false,
    };

    /// `isAbove(shape, pos, default)`.
    fn is_above(&self, shape_max_y: f64, pos: BlockPos, default: bool) -> bool {
        if !self.has_entity {
            return default;
        }
        self.entity_bottom > pos.y as f64 + shape_max_y - 9.999999747378752e-6
    }
}

/// The collision shape of `state` at `pos` for `ctx`, and whether it is `Shapes.block()`.
pub fn collision_shape(state: u16, pos: BlockPos, ctx: &CollisionContext) -> (Cow<'static, Shape>, bool) {
    match kind(state) {
        Kind::Scaffolding => (Cow::Borrowed(scaffolding_shape(state, pos, ctx)), false),
        Kind::PowderSnow => {
            let (s, cube) = powder_snow_shape(pos, ctx);
            (Cow::Borrowed(s), cube)
        }
        _ => {
            let shape = physics::collision_shape(state);
            match physics::collision_offset(state, pos.x, pos.z) {
                Some((ox, oz)) => (Cow::Owned(shape.moved(ox, 0.0, oz)), false),
                None => (Cow::Borrowed(shape), physics::is_full_cube(state)),
            }
        }
    }
}

fn scaffolding_shape(state: u16, pos: BlockPos, ctx: &CollisionContext) -> &'static Shape {
    let empty = physics::empty_shape();
    if ctx.placement {
        return empty;
    }
    if ctx.is_above(1.0, pos, true) && !ctx.descending {
        return physics::named_shape("scaffolding_stable");
    }
    let info = kiln_data::blocks_types::block_of(state);
    let distance = info.property(state, "distance") != Some("0");
    let bottom = info.property(state, "bottom") == Some("true");
    let below = physics::named_shape("scaffolding_below_block");
    if distance && bottom && ctx.is_above(below.max(Axis::Y, 0.0), pos, true) {
        physics::named_shape("scaffolding_unstable_bottom")
    } else {
        empty
    }
}

fn powder_snow_shape(pos: BlockPos, ctx: &CollisionContext) -> (&'static Shape, bool) {
    let empty = (physics::empty_shape(), false);
    if ctx.placement || !ctx.has_entity {
        return empty;
    }
    if ctx.fall_distance > 2.5 {
        return (physics::named_shape("powder_snow_falling"), false);
    }
    if ctx.falling_block || (ctx.walks_on_powder_snow && ctx.is_above(1.0, pos, false) && !ctx.descending) {
        // Block.getCollisionShape: the full block (powder snow has collision, no custom shape).
        return (physics::block_shape(), true);
    }
    empty
}

/// Visits vanilla's `BlockCollisions` over `area` in order: every block shape that intersects
/// `area` (full cubes by box test, others by `Shapes.joinIsNotEmpty`), moved to its position.
pub fn for_each_block_collision(
    level: &dyn EntityLevel,
    ctx: &CollisionContext,
    area: &Aabb,
    mut visit: impl FnMut(BlockPos, Cow<'static, Shape>, bool) -> bool,
) {
    let x0 = floor(area.min_x - 1.0e-7) - 1;
    let x1 = floor(area.max_x + 1.0e-7) + 1;
    let y0 = floor(area.min_y - 1.0e-7) - 1;
    let y1 = floor(area.max_y + 1.0e-7) + 1;
    let z0 = floor(area.min_z - 1.0e-7) - 1;
    let z1 = floor(area.max_z + 1.0e-7) + 1;
    let entity_shape = BoxShape::new(area);
    let (w, h, d) = (x1 - x0 + 1, y1 - y0 + 1, z1 - z0 + 1);
    // The whole box read at once when it is loaded and small enough for the stack.
    let mut buf = [0u16; 384];
    let n = (w * h * d) as usize;
    let bulk = {
        crate::prof!("col", "read_blocks");
        n <= buf.len() && level.read_blocks(BlockPos::new(x0, y0, z0), BlockPos::new(x1, y1, z1), &mut buf[..n])
    };
    crate::prof!("col", "block loop");
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                let edges = (x == 0 || x == w - 1) as u8 + (y == 0 || y == h - 1) as u8 + (z == 0 || z == d - 1) as u8;
                if edges == 3 {
                    continue;
                }
                let pos = BlockPos::new(x0 + x, y0 + y, z0 + z);
                let state = if bulk {
                    buf[((y * d + z) * w + x) as usize]
                } else {
                    if !level.is_loaded(pos) {
                        continue;
                    }
                    level.block(pos)
                };
                if edges == 1 && !physics::has_large_collision_shape(state) {
                    continue;
                }
                if edges == 2 && kind(state) != Kind::MovingPiston {
                    continue;
                }
                let (shape, cube) = if ctx.stands_on_lava && stable_lava(level, state, pos, ctx) {
                    (Cow::Borrowed(lava_stable_shape()), false)
                } else {
                    collision_shape(state, pos, ctx)
                };
                let (px, py, pz) = (pos.x as f64, pos.y as f64, pos.z as f64);
                let hit = if cube {
                    area.intersects_raw(px, py, pz, px + 1.0, py + 1.0, pz + 1.0)
                } else {
                    !shape.is_empty()
                        && entity_shape.as_ref().is_some_and(|e| intersects_box(&shape, [px, py, pz], e))
                };
                if hit && !visit(pos, shape, cube) {
                    return;
                }
            }
        }
    }
}

/// `Entity.collide` over blocks alone, for a mover the level's entities do not know (a player the
/// region moves by hand): how far `bx` gets with `movement` through the block shapes `blocks`
/// gives (`None`: not loaded).
pub fn collide_blocks(blocks: &dyn Fn(BlockPos) -> Option<u16>, ctx: &CollisionContext, movement: Vec3, bx: &Aabb) -> Vec3 {
    let area = bx.expand_towards_vec(movement);
    let x0 = floor(area.min_x - 1.0e-7) - 1;
    let x1 = floor(area.max_x + 1.0e-7) + 1;
    let y0 = floor(area.min_y - 1.0e-7) - 1;
    let y1 = floor(area.max_y + 1.0e-7) + 1;
    let z0 = floor(area.min_z - 1.0e-7) - 1;
    let z1 = floor(area.max_z + 1.0e-7) + 1;
    let entity_shape = BoxShape::new(&area);
    let (w, h, d) = (x1 - x0 + 1, y1 - y0 + 1, z1 - z0 + 1);
    let mut shapes: Vec<Collider> = Vec::new();
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                let edges = (x == 0 || x == w - 1) as u8 + (y == 0 || y == h - 1) as u8 + (z == 0 || z == d - 1) as u8;
                if edges == 3 {
                    continue;
                }
                let pos = BlockPos::new(x0 + x, y0 + y, z0 + z);
                let Some(state) = blocks(pos) else { continue };
                if edges == 1 && !physics::has_large_collision_shape(state) {
                    continue;
                }
                if edges == 2 && kind(state) != Kind::MovingPiston {
                    continue;
                }
                let (shape, cube) = collision_shape(state, pos, ctx);
                let (px, py, pz) = (pos.x as f64, pos.y as f64, pos.z as f64);
                let hit = if cube {
                    area.intersects_raw(px, py, pz, px + 1.0, py + 1.0, pz + 1.0)
                } else {
                    !shape.is_empty() && entity_shape.as_ref().is_some_and(|e| intersects_box(&shape, [px, py, pz], e))
                };
                if hit {
                    shapes.push(Collider { shape, offset: [px, py, pz] });
                }
            }
        }
    }
    collide_with_shapes(movement, bx, &shapes)
}

/// `LiquidBlock.STABLE_SHAPE`: the lower half of the block.
fn lava_stable_shape() -> &'static Shape {
    static SHAPE: std::sync::OnceLock<Shape> = std::sync::OnceLock::new();
    SHAPE.get_or_init(|| Shape::from_box(&Aabb::new(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)).expect("a box"))
}

/// `LiquidBlock.getCollisionShape` for a mob that stands on lava: a lava source with no lava
/// above, under the entity's feet.
fn stable_lava(level: &dyn EntityLevel, state: u16, pos: BlockPos, ctx: &CollisionContext) -> bool {
    let f = physics::fluid_state(state);
    f.kind == physics::FluidKind::Lava
        && crate::blocks::block_name(state) == "minecraft:lava"
        && ctx.is_above(0.5, pos, true)
        && !physics::fluid_state(level.block(pos.above())).kind.is_lava()
}

/// The colliders around an entity: a handful, so they live on the stack.
pub type Colliders = smallvec::SmallVec<[Collider; 8]>;

/// `getBlockCollisions` as placed colliders.
pub fn block_colliders(level: &dyn EntityLevel, ctx: &CollisionContext, area: &Aabb, out: &mut Colliders) {
    for_each_block_collision(level, ctx, area, |pos, shape, _| {
        out.push(Collider { shape, offset: [pos.x as f64, pos.y as f64, pos.z as f64] });
        true
    });
}

/// `EntityGetter.getEntityCollisions`: boxes of collidable entities as colliders.
pub fn entity_colliders(level: &dyn EntityLevel, entity: i32, area: &Aabb) -> Vec<Collider> {
    if area.size() < 1.0e-7 {
        return Vec::new();
    }
    level.entity_collision_boxes(entity, &area.inflate_all(1.0e-7)).iter().filter_map(Collider::from_box).collect()
}

/// `Entity.collectCollidersIgnoringWorldBorder`: `entity_shapes` then the block colliders.
pub fn collect_colliders(
    level: &dyn EntityLevel,
    ctx: &CollisionContext,
    entity_shapes: &[Collider],
    area: &Aabb,
) -> Colliders {
    let mut out: Colliders = entity_shapes.iter().cloned().collect();
    block_colliders(level, ctx, area, &mut out);
    out
}

/// `Entity.collideWithShapes`: moves `bx` by `movement` one axis at a time (Y first).
pub fn collide_with_shapes(movement: Vec3, bx: &Aabb, shapes: &[Collider]) -> Vec3 {
    if shapes.is_empty() {
        return movement;
    }
    let mut result = Vec3::ZERO;
    for axis in Axis::step_order(movement) {
        let d = movement.get(axis);
        if d != 0.0 {
            let c = collide_all(axis, &bx.offset_vec(result), shapes, d);
            result = result.with(axis, c);
        }
    }
    result
}

/// `Entity.collideBoundingBox`.
pub fn collide_bounding_box(
    level: &dyn EntityLevel,
    ctx: &CollisionContext,
    movement: Vec3,
    bx: &Aabb,
    entity_shapes: &[Collider],
) -> Vec3 {
    let shapes = collect_colliders(level, ctx, entity_shapes, &bx.expand_towards_vec(movement));
    crate::prof!("col", "collide_with_shapes");
    collide_with_shapes(movement, bx, &shapes)
}

/// `CollisionGetter.noCollision(entity, box)`: no block shape and no collidable entity inside.
pub fn no_collision(level: &dyn EntityLevel, ctx: &CollisionContext, entity: i32, bx: &Aabb) -> bool {
    let mut blocked = false;
    for_each_block_collision(level, ctx, bx, |_, _, _| {
        blocked = true;
        false
    });
    !blocked && entity_colliders(level, entity, bx).is_empty()
}

/// `CollisionGetter.findSupportingBlock`: the colliding block whose center is nearest to
/// `position` (ties broken by `BlockPos.compareTo`).
pub fn find_supporting_block(level: &dyn EntityLevel, ctx: &CollisionContext, position: Vec3, bx: &Aabb) -> Option<BlockPos> {
    let mut best: Option<BlockPos> = None;
    let mut best_d = f64::MAX;
    for_each_block_collision(level, ctx, bx, |pos, _, _| {
        let dx = pos.x as f64 + 0.5 - position.x;
        let dy = pos.y as f64 + 0.5 - position.y;
        let dz = pos.z as f64 + 0.5 - position.z;
        let d = dx * dx + dy * dy + dz * dz;
        if d < best_d || (d == best_d && best.is_none_or(|b| compare(b, pos) < 0)) {
            best = Some(pos);
            best_d = d;
        }
        true
    });
    best
}

/// `Vec3i.compareTo`: y, then z, then x.
fn compare(a: BlockPos, b: BlockPos) -> i32 {
    if a.y == b.y {
        if a.z == b.z { a.x - b.x } else { a.z - b.z }
    } else {
        a.y - b.y
    }
}
