//! `RandomPos`, `DefaultRandomPos` and `LandRandomPos`: random walk targets.

use super::{MobData, mth, path};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{BlockPos, Vec3};
use kiln_javamath::random::RandomSource;

/// `RandomPos.generateRandomDirection`.
fn random_direction(e: &mut Entity, h: i32, v: i32) -> BlockPos {
    let x = e.random.next_int_bounded(2 * h + 1) - h;
    let y = e.random.next_int_bounded(2 * v + 1) - v;
    let z = e.random.next_int_bounded(2 * h + 1) - h;
    BlockPos::new(x, y, z)
}

/// `RandomPos.generateRandomPosTowardDirection` (mobs here have no home).
fn toward(e: &Entity, dir: BlockPos) -> BlockPos {
    BlockPos::containing(dir.x as f64 + e.x(), dir.y as f64 + e.y(), dir.z as f64 + e.z())
}

fn outside_limits(level: &dyn EntityLevel, p: BlockPos) -> bool {
    p.y < level.min_y() || p.y > level.max_y()
}

fn has_malus(m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> bool {
    path::malus(m, path::path_type_static(level, p.x, p.y, p.z)) != 0.0
}

/// `RandomPos.generateRandomPos(supplier, mob::getWalkTargetValue)`.
fn generate(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, mut next: impl FnMut(&mut Entity) -> Option<BlockPos>) -> Option<Vec3> {
    let mut best = f64::NEG_INFINITY;
    let mut found = None;
    for _ in 0..10 {
        let Some(p) = next(e) else { continue };
        let v = super::walk_target_value(m, level, p) as f64;
        if v > best {
            best = v;
            found = Some(p);
        }
    }
    found.map(|p| Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5))
}

/// `DefaultRandomPos.getPos(mob, h, v)`.
pub fn default_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32) -> Option<Vec3> {
    generate(e, m, level, |e| {
        let dir = random_direction(e, h, v);
        let p = toward(e, dir);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) || has_malus(m, level, p) {
            return None;
        }
        Some(p)
    })
}

/// `DefaultRandomPos.getPosTowards(mob, h, v, target, angle)`.
pub fn default_pos_towards(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, target: Vec3, angle: f64) -> Option<Vec3> {
    let d = target - e.position();
    generate(e, m, level, |e| {
        let dir = direction_within_radians(e, 0.0, h as f64, v, 0, d.x, d.z, angle)?;
        let p = toward(e, dir);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) || has_malus(m, level, p) {
            return None;
        }
        Some(p)
    })
}

/// `LandRandomPos.getPos(mob, h, v)`.
pub fn land_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32) -> Option<Vec3> {
    generate(e, m, level, |e| {
        let dir = random_direction(e, h, v);
        let p = toward(e, dir);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) {
            return None;
        }
        move_up_out_of_solid(m, level, p)
    })
}

/// `LandRandomPos.getPosAway(mob, h, v, from)` (`getPosInDirection` with half-pi spread).
pub fn land_pos_away(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, from: Vec3) -> Option<Vec3> {
    let mut d = e.position() - from;
    if d.length() == 0.0 {
        d = Vec3::new(e.random.next_double() - 0.5, 0.0, e.random.next_double() - 0.5);
    }
    generate(e, m, level, |e| {
        let dir = direction_within_radians(e, 0.0, h as f64, v, 0, d.x, d.z, std::f32::consts::FRAC_PI_2 as f64)?;
        let p = toward(e, dir);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) {
            return None;
        }
        move_up_out_of_solid(m, level, p)
    })
}

/// `RandomPos.generateRandomDirectionWithinRadians`.
#[allow(clippy::too_many_arguments)]
fn direction_within_radians(e: &mut Entity, min: f64, max: f64, v: i32, y_off: i32, dx: f64, dz: f64, spread: f64) -> Option<BlockPos> {
    let a = mth::atan2(dz, dx) - std::f32::consts::FRAC_PI_2 as f64;
    let b = a + ((2.0 * e.random.next_float() - 1.0) as f64) * spread;
    let r = crate::math::lerp(e.random.next_double().sqrt(), min, max) * std::f32::consts::SQRT_2 as f64;
    let x = -r * kiln_javamath::trig::sin(b);
    let z = r * kiln_javamath::trig::cos(b);
    if x.abs() > max || z.abs() > max {
        return None;
    }
    let y = e.random.next_int_bounded(2 * v + 1) - v + y_off;
    Some(BlockPos::containing(x, y as f64, z))
}

/// `LandRandomPos.movePosUpOutOfSolid`.
fn move_up_out_of_solid(m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<BlockPos> {
    let solid = |p: BlockPos| kiln_data::block_logic::is_solid(level.block(p));
    let mut p = p;
    if solid(p) {
        let mut q = p.above();
        while q.y <= level.max_y() && solid(q) {
            q = q.above();
        }
        p = q;
    }
    if crate::physics::fluid_state(level.block(p)).kind.is_water() || has_malus(m, level, p) {
        return None;
    }
    Some(p)
}
