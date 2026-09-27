//! `AbstractArrow` flight: sticks in blocks (`inGround`), shakes, despawns after a minute,
//! drops when its block disappears. Entity hits are reported as [`Event::ProjectileHit`].

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::projectile::{Hit, can_be_hit_by_projectile, lerp_rotation, mth_atan2};
use kiln_javamath::random::RandomSource;

/// `AbstractArrow` state (arrows and spectral arrows).
#[derive(Clone, Debug)]
pub struct ArrowData {
    pub owner: Option<i32>,
    pub left_owner: bool,
    pub(crate) left_owner_checked: bool,
    pub(crate) has_been_shot: bool,
    pub in_ground: bool,
    pub in_ground_time: i32,
    pub shake_time: i32,
    pub life: i32,
    /// The block the arrow is stuck in (`lastState`).
    pub last_state: Option<u16>,
    pub crit: bool,
    pub base_damage: f64,
}

/// A flying arrow (`minecraft:arrow` or `minecraft:spectral_arrow`).
pub fn new(id: i32, uuid: u128, type_name: &'static str, pos: Vec3, delta: Vec3, owner: Option<i32>, seed: i64) -> Entity {
    let data = ArrowData {
        owner,
        left_owner: false,
        left_owner_checked: false,
        has_been_shot: false,
        in_ground: false,
        in_ground_time: 0,
        shake_time: 0,
        life: 0,
        last_state: None,
        crit: false,
        base_damage: 2.0,
    };
    let mut e = Entity::new(type_name, id, uuid, EntityKind::Arrow(data), seed);
    e.set_pos(pos);
    e.delta = delta;
    e.set_old_pos_and_rot();
    e
}

fn data(e: &mut Entity) -> &mut ArrowData {
    match &mut e.kind {
        EntityKind::Arrow(d) => d,
        _ => unreachable!("not an arrow"),
    }
}

/// `AbstractArrow.tick`.
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    let physics = !e.no_physics;
    let v = e.delta;
    let pos = e.block_position();
    let state = level.block(pos);
    if !crate::physics::is_air(state) && physics {
        let (shape, _) = collision::collision_shape(state, pos, &collision::CollisionContext::EMPTY);
        let p = e.position();
        if !shape.is_empty() && shape.boxes().iter().any(|b| b.offset(pos.x as f64, pos.y as f64, pos.z as f64).contains(p)) {
            e.delta = Vec3::ZERO;
            data(e).in_ground = true;
        }
    }
    if data(e).shake_time > 0 {
        data(e).shake_time -= 1;
    }
    if e.is_in_water() || level.is_raining_at(e.block_position()) {
        e.clear_fire();
    }
    if data(e).in_ground && physics {
        if data(e).last_state != Some(state) && should_fall(e, level) {
            start_falling(e);
        } else {
            let d = data(e);
            d.life += 1;
            if d.life >= 1200 {
                e.discard();
            }
        }
        data(e).in_ground_time += 1;
        if e.is_alive() {
            e.apply_effects_from_blocks(level);
        }
        return;
    }
    data(e).in_ground_time = 0;
    let start = e.position();
    if e.is_in_water() {
        e.delta = e.delta.scale(0.6f32 as f64);
    }
    let yaw = if physics { mth_atan2(v.x, v.z) } else { mth_atan2(-v.x, -v.z) };
    let pitch = mth_atan2(v.y, v.horizontal_distance());
    e.x_rot = lerp_rotation(e.x_rot, (pitch * 57.2957763671875) as f32);
    e.y_rot = lerp_rotation(e.y_rot, (yaw * 57.2957763671875) as f32);
    check_left_owner(e, level);
    if physics {
        let to = start + v;
        let ctx = e.collision_context();
        let block = clip::traverse_blocks(start, to, |pos| {
            let (shape, _) = collision::collision_shape(level.block(pos), pos, &ctx);
            clip::shape_clip(&shape, start, to, pos).map(|(location, face)| (pos, face, location))
        });
        step_move_and_hit(e, level, start, to, block);
    } else {
        e.set_pos(start + v);
        e.apply_effects_from_blocks(level);
    }
    if !e.is_in_water() {
        e.delta = e.delta.scale(0.99f32 as f64);
    }
    if physics && !data(e).in_ground {
        e.apply_gravity();
    }
    // Projectile.tick
    if !data(e).has_been_shot {
        level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: data(e).owner });
        data(e).has_been_shot = true;
    }
    check_left_owner(e, level);
    e.base_tick(level);
    data(e).left_owner_checked = false;
}

/// `stepMoveAndHit`: to the nearest entity or block hit on the path, then the hit.
fn step_move_and_hit(e: &mut Entity, level: &mut dyn EntityLevel, from: Vec3, to: Vec3, block: Option<(BlockPos, Direction, Vec3)>) {
    if !e.is_alive() {
        return;
    }
    let end = block.map_or(to, |(_, _, l)| l);
    let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0));
    let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
    let owner = match &e.kind {
        EntityKind::Arrow(d) if !d.left_owner => d.owner,
        _ => None,
    };
    // Approximation: only the nearest entity on the segment (vanilla collects all for piercing).
    let mut target: Option<(i32, Vec3)> = None;
    let mut best = f64::MAX;
    for id in level.entities_in(&area, EntityFilter::Any, e.id) {
        let Some(t) = level.entity(id) else { continue };
        if !can_be_hit_by_projectile(t) || Some(id) == owner {
            continue;
        }
        if let Some(p) = t.bounding_box().inflate_all(margin as f64).clip(from, end) {
            let d = from.distance_to_sqr(t.position());
            if d < best {
                best = d;
                target = Some((id, p));
            }
        }
    }
    let dest = target.map_or(end, |(_, p)| p);
    e.set_pos(dest);
    e.apply_effects_between(level, from, dest);
    match target {
        None => {
            if let (true, Some((pos, face, location))) = (e.is_alive(), block) {
                hit_block(e, level, pos, face, location);
                e.needs_sync = true;
            }
        }
        Some((id, location)) => {
            if e.is_alive() && !e.no_physics {
                let owner = data(e).owner;
                level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: e.type_name, owner, hit: Hit::Entity { id, location } });
                e.needs_sync = true;
                e.discard();
            }
        }
    }
}

/// `AbstractArrow.onHitBlock`: sticks in the block, backed off 0.05 against the motion.
fn hit_block(e: &mut Entity, level: &mut dyn EntityLevel, pos: BlockPos, face: Direction, location: Vec3) {
    let state = level.block(pos);
    data(e).last_state = Some(state);
    let owner = data(e).owner;
    level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: e.type_name, owner, hit: Hit::Block { pos, face, location } });
    let d = e.delta;
    let back = Vec3::new(signum(d.x), signum(d.y), signum(d.z)).scale(0.05000000074505806);
    e.set_pos(e.position() - back);
    e.delta = Vec3::ZERO;
    let pitch = 1.2 / (e.random.next_float() * 0.2 + 0.9);
    e.play_sound(level, "minecraft:entity.arrow.hit", 1.0, pitch);
    let a = data(e);
    a.in_ground = true;
    a.shake_time = 7;
    a.crit = false;
}

fn signum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() { v } else { 1.0f64.copysign(v) }
}

/// `shouldFall`: nothing solid within 0.06 of the tip.
fn should_fall(e: &Entity, level: &dyn EntityLevel) -> bool {
    let p = e.position();
    let area = Aabb::new(p.x, p.y, p.z, p.x, p.y, p.z).inflate_all(0.06);
    collision::no_collision(level, &collision::CollisionContext::EMPTY, i32::MIN, &area)
}

/// `startFalling`: drops out with a small random motion.
fn start_falling(e: &mut Entity) {
    data(e).in_ground = false;
    let fx = (e.random.next_float() * 0.2) as f64;
    let fy = (e.random.next_float() * 0.2) as f64;
    let fz = (e.random.next_float() * 0.2) as f64;
    e.delta = e.delta.multiply(fx, fy, fz);
    data(e).life = 0;
}

fn check_left_owner(e: &mut Entity, level: &dyn EntityLevel) {
    let (owner, left, checked) = {
        let d = data(e);
        (d.owner, d.left_owner, d.left_owner_checked)
    };
    if left || checked {
        return;
    }
    let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
    let outside = match owner.and_then(|id| level.entity(id)) {
        Some(o) => !(can_be_hit_by_projectile(o) && area.intersects(&o.bounding_box())),
        None => true,
    };
    let d = data(e);
    d.left_owner = outside;
    d.left_owner_checked = true;
}
