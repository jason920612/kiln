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

/// `PathNavigation.isStableDestination` of the mob's navigation.
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
    default_pos_home(e, m, level, h, v, m.home)
}

/// A mob's home (`Mob.homePosition`, `homeRadius`): a block and a radius (-1: anywhere).
pub type Home = Option<(BlockPos, i32)>;

/// `Mob.isWithinHome(pos)`.
pub fn within_home(home: Home, p: BlockPos) -> bool {
    match home {
        None => true,
        Some((_, -1)) => true,
        Some((c, r)) => {
            let (dx, dy, dz) = ((c.x - p.x) as f64, (c.y - p.y) as f64, (c.z - p.z) as f64);
            dx * dx + dy * dy + dz * dz < (r * r) as f64
        }
    }
}

/// `GoalUtils.mobRestricted(mob, h)`: the mob is near enough to its home to keep to it.
fn mob_restricted(e: &Entity, home: Home, h: f64) -> bool {
    let Some((c, r)) = home else { return false };
    let d = r as f64 + h + 1.0;
    let (dx, dy, dz) = (c.x as f64 + 0.5 - e.x(), c.y as f64 + 0.5 - e.y(), c.z as f64 + 0.5 - e.z());
    dx * dx + dy * dy + dz * dz < d * d
}

/// `RandomPos.generateRandomPosTowardDirection`: pulled toward the home, if any.
fn toward_home(e: &mut Entity, h: f64, dir: BlockPos, home: Home) -> BlockPos {
    let (mut x, mut z) = (dir.x as f64, dir.z as f64);
    if let Some((c, _)) = home
        && h > 1.0
    {
        if e.x() > c.x as f64 {
            x -= e.random.next_double() * h / 2.0;
        } else {
            x += e.random.next_double() * h / 2.0;
        }
        if e.z() > c.z as f64 {
            z -= e.random.next_double() * h / 2.0;
        } else {
            z += e.random.next_double() * h / 2.0;
        }
    }
    BlockPos::containing(x + e.x(), dir.y as f64 + e.y(), z + e.z())
}

/// `DefaultRandomPos.generateRandomPosTowardDirection`.
fn default_toward(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, restrict: bool, dir: BlockPos, home: Home) -> Option<BlockPos> {
    let p = toward_home(e, h as f64, dir, home);
    if outside_limits(level, p) || (restrict && !within_home(home, p)) || !path::stable_destination(m, level, p) || has_malus(m, level, p) {
        return None;
    }
    Some(p)
}

/// `DefaultRandomPos.getPos(mob, h, v)` for a mob with a home.
pub fn default_pos_home(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, home: Home) -> Option<Vec3> {
    let restrict = mob_restricted(e, home, h as f64);
    generate(e, m, level, |e| {
        let dir = random_direction(e, h, v);
        default_toward(e, m, level, h, restrict, dir, home)
    })
}

/// `DefaultRandomPos.getPosTowards(mob, h, v, target, angle)` for a mob with a home.
#[allow(clippy::too_many_arguments)]
pub fn default_pos_towards_home(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, target: Vec3, angle: f64, home: Home) -> Option<Vec3> {
    let d = target - e.position();
    let restrict = mob_restricted(e, home, h as f64);
    generate(e, m, level, |e| {
        let dir = direction_within_radians(e, 0.0, h as f64, v, 0, d.x, d.z, angle)?;
        default_toward(e, m, level, h, restrict, dir, home)
    })
}

/// `HoverRandomPos.getPos(mob, h, v, x, z, angle, maxHover, minHover)`: a stable block toward the
/// direction, raised a few blocks over the solid ground.
#[allow(clippy::too_many_arguments)]
pub fn hover_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, dx: f64, dz: f64, angle: f32, max_hover: i32, min_hover: i32) -> Option<Vec3> {
    generate(e, m, level, |e| {
        let dir = direction_within_radians(e, 0.0, h as f64, v, 0, dx, dz, angle as f64)?;
        // `LandRandomPos.generateRandomPosTowardDirection` (no home).
        let p = toward_home(e, h as f64, dir, None);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) {
            return None;
        }
        let above = e.random.next_int_bounded(max_hover - min_hover + 1) + min_hover;
        let p = move_up_to_above_solid(level, p, above);
        if crate::physics::fluid_state(level.block(p)).kind.is_water() || has_malus(m, level, p) {
            return None;
        }
        Some(p)
    })
}

/// `AirAndWaterRandomPos.getPos(mob, h, v, flyingHeight, x, z, angle)`.
#[allow(clippy::too_many_arguments)]
pub fn air_and_water_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, flying_height: i32, dx: f64, dz: f64, angle: f64) -> Option<Vec3> {
    generate(e, m, level, |e| {
        let dir = direction_within_radians(e, 0.0, h as f64, v, flying_height, dx, dz, angle)?;
        let p = toward_home(e, h as f64, dir, None);
        if outside_limits(level, p) {
            return None;
        }
        let p = move_up_out_of_solid_raw(level, p);
        if has_malus(m, level, p) {
            return None;
        }
        Some(p)
    })
}

fn is_solid(level: &dyn EntityLevel, p: BlockPos) -> bool {
    kiln_data::block_logic::is_solid(level.block(p))
}

/// `RandomPos.moveUpOutOfSolid`.
fn move_up_out_of_solid_raw(level: &dyn EntityLevel, p: BlockPos) -> BlockPos {
    if !is_solid(level, p) {
        return p;
    }
    let mut q = p.above();
    while q.y <= level.max_y() && is_solid(level, q) {
        q = q.above();
    }
    q
}

/// `RandomPos.moveUpToAboveSolid`: out of the solid blocks, then up to `above` more while the
/// way is clear.
fn move_up_to_above_solid(level: &dyn EntityLevel, p: BlockPos, above: i32) -> BlockPos {
    if !is_solid(level, p) {
        return p;
    }
    let mut q = p.above();
    while q.y <= level.max_y() && is_solid(level, q) {
        q = q.above();
    }
    let first = q.y;
    while q.y <= level.max_y() && q.y - first < above {
        q = q.above();
        if is_solid(level, q) {
            q = q.below();
            break;
        }
    }
    q
}

/// `DefaultRandomPos.getPosTowards(mob, h, v, target, angle)`.
pub fn default_pos_towards(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, target: Vec3, angle: f64) -> Option<Vec3> {
    crate::prof!("path", "default_pos_towards");
    default_pos_towards_home(e, m, level, h, v, target, angle, m.home)
}

/// `LandRandomPos.getPos(mob, h, v)`.
pub fn land_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32) -> Option<Vec3> {
    crate::prof!("path", "land_pos");
    let home = m.home;
    let restrict = mob_restricted(e, home, h as f64);
    generate(e, m, level, |e| {
        let dir = random_direction(e, h, v);
        let p = toward_home(e, h as f64, dir, home);
        if outside_limits(level, p) || (restrict && !within_home(home, p)) || !path::stable_destination(m, level, p) {
            return None;
        }
        move_up_out_of_solid(m, level, p)
    })
}

/// `LandRandomPos.getPos(mob, h, v, weight)`: the best of ten spots by `weight` (not the mob's
/// own walk target value).
pub fn land_pos_weighted(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, weight: &dyn Fn(BlockPos) -> f64) -> Option<Vec3> {
    let mut best = f64::NEG_INFINITY;
    let mut found = None;
    for _ in 0..10 {
        let dir = random_direction(e, h, v);
        let p = toward(e, dir);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) {
            continue;
        }
        let Some(p) = move_up_out_of_solid(m, level, p) else { continue };
        let w = weight(p);
        if w > best {
            best = w;
            found = Some(p);
        }
    }
    found.map(|p| Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5))
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

/// `DefaultRandomPos.getPosAway(mob, h, v, from)`.
pub fn default_pos_away(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32, from: Vec3) -> Option<Vec3> {
    let d = e.position() - from;
    generate(e, m, level, |e| {
        let dir = direction_within_radians(e, 0.0, h as f64, v, 0, d.x, d.z, std::f32::consts::FRAC_PI_2 as f64)?;
        let p = toward(e, dir);
        if outside_limits(level, p) || !path::stable_destination(m, level, p) || has_malus(m, level, p) {
            return None;
        }
        Some(p)
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
