//! Ray casts through blocks (`BlockGetter.traverseBlocks` and `VoxelShape.clip`).

use crate::blocks::{Tag, has_tag};
use crate::entity::Entity;
use crate::fluid;
use crate::level::EntityLevel;
use crate::math::{Aabb, BlockPos, Direction, Vec3, floor, lerp};
use crate::physics;
use crate::shape::Shape;

/// `BlockGetter.traverseBlocks`: visits the blocks along `from → to` (both nudged by 1e-7 of
/// the segment), stopping at the first `Some`.
pub fn traverse_blocks<T>(from: Vec3, to: Vec3, mut visit: impl FnMut(BlockPos) -> Option<T>) -> Option<T> {
    if from == to {
        return None;
    }
    let end = Vec3::new(lerp(-1.0e-7, to.x, from.x), lerp(-1.0e-7, to.y, from.y), lerp(-1.0e-7, to.z, from.z));
    let start = Vec3::new(lerp(-1.0e-7, from.x, to.x), lerp(-1.0e-7, from.y, to.y), lerp(-1.0e-7, from.z, to.z));
    let (mut x, mut y, mut z) = (floor(start.x), floor(start.y), floor(start.z));
    if let Some(r) = visit(BlockPos::new(x, y, z)) {
        return Some(r);
    }
    let (dx, dy, dz) = (end.x - start.x, end.y - start.y, end.z - start.z);
    let sign = |v: f64| if v == 0.0 { 0 } else if v > 0.0 { 1 } else { -1 };
    let frac = |v: f64| v - v.floor() as i64 as f64;
    let (sx, sy, sz) = (sign(dx), sign(dy), sign(dz));
    let tx = if sx == 0 { f64::MAX } else { sx as f64 / dx };
    let ty = if sy == 0 { f64::MAX } else { sy as f64 / dy };
    let tz = if sz == 0 { f64::MAX } else { sz as f64 / dz };
    let mut nx = tx * if sx > 0 { 1.0 - frac(start.x) } else { frac(start.x) };
    let mut ny = ty * if sy > 0 { 1.0 - frac(start.y) } else { frac(start.y) };
    let mut nz = tz * if sz > 0 { 1.0 - frac(start.z) } else { frac(start.z) };
    while nx <= 1.0 || ny <= 1.0 || nz <= 1.0 {
        if nx < ny {
            if nx < nz {
                x += sx;
                nx += tx;
            } else {
                z += sz;
                nz += tz;
            }
        } else if ny < nz {
            y += sy;
            ny += ty;
        } else {
            z += sz;
            nz += tz;
        }
        if let Some(r) = visit(BlockPos::new(x, y, z)) {
            return Some(r);
        }
    }
    None
}

/// `VoxelShape.clip(from, to, pos)` as a hit test.
pub fn shape_clips(shape: &Shape, from: Vec3, to: Vec3, pos: BlockPos) -> bool {
    shape_clip(shape, from, to, pos).is_some()
}

/// `VoxelShape.clip(from, to, pos)`: the hit location and face.
pub fn shape_clip(shape: &Shape, from: Vec3, to: Vec3, pos: BlockPos) -> Option<(Vec3, Direction)> {
    if shape.is_empty() {
        return None;
    }
    let d = to - from;
    if d.length_sqr() < 1.0e-7 {
        return None;
    }
    let v = from + d.scale(0.001);
    let (px, py, pz) = (pos.x as f64, pos.y as f64, pos.z as f64);
    if shape.contains_point(v.x - px, v.y - py, v.z - pz) {
        return Some((v, approximate_nearest(d).opposite()));
    }
    let mut scale = 1.0;
    let mut dir = None;
    for b in shape.boxes() {
        dir = b.offset(px, py, pz).clip_direction(from, &mut scale, dir, d);
    }
    dir.map(|face| (from.add(scale * d.x, scale * d.y, scale * d.z), face))
}

/// `Direction.getApproximateNearest(x, y, z)`: the direction with the largest dot product.
pub fn approximate_nearest(d: Vec3) -> Direction {
    let mut best = Direction::North;
    let mut best_dot = f32::MIN;
    for dir in Direction::ALL {
        let (sx, sy, sz) = dir.step();
        let dot = (d.x * sx as f64 + d.y * sy as f64 + d.z * sz as f64) as f32;
        if dot > best_dot {
            best_dot = dot;
            best = dir;
        }
    }
    best
}

/// `level.clip(ClipContext(from, to, FALLDAMAGE_RESETTING, WATER, entity))` hits something.
pub fn clip_fall_damage_resetting(level: &dyn EntityLevel, from: Vec3, to: Vec3, _entity: &Entity) -> bool {
    traverse_blocks(from, to, |pos| {
        let state = level.block(pos);
        let block_hit = has_tag(state, Tag::FallDamageResetting) && shape_clips(physics::block_shape(), from, to, pos);
        let f = physics::fluid_state(state);
        let fluid_hit = f.kind.is_water() && {
            let h = fluid::height(level, pos, &f);
            let shape = Shape::from_box(&Aabb::new(0.0, 0.0, 0.0, 1.0, h as f64, 1.0));
            shape.is_some_and(|s| shape_clips(&s, from, to, pos))
        };
        (block_hit || fluid_hit).then_some(())
    })
    .is_some()
}
