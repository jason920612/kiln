//! `ExperienceOrb`: floats in water, follows the nearest player within 8 blocks and merges with
//! orbs of the same value.

use crate::collision::{self, CollisionContext};
use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel};
use crate::math::{Vec3, jmin};
use crate::physics;
use kiln_javamath::random::RandomSource;

pub const LIFETIME: i32 = 6000;

#[derive(Clone, Debug)]
pub struct OrbData {
    pub value: i32,
    pub count: i32,
    pub age: i32,
    pub health: i32,
    pub following: Option<i32>,
}

impl OrbData {
    pub fn new(value: i32) -> Self {
        OrbData { value, count: 1, age: 0, health: 5, following: None }
    }
}

/// `new ExperienceOrb(level, x, y, z, value)`: a random yaw and a small random throw.
pub fn new_at(id: i32, uuid: u128, pos: Vec3, value: i32, seed: i64) -> Entity {
    let mut e = Entity::new("minecraft:experience_orb", id, uuid, EntityKind::ExperienceOrb(OrbData::new(value)), seed);
    e.set_pos(pos);
    e.y_rot = (e.random.next_double() * 360.0) as f32;
    let dx = (e.random.next_double() * 0.2f32 as f64 - 0.1f32 as f64) * 2.0;
    let dy = e.random.next_double() * 0.2 * 2.0;
    let dz = (e.random.next_double() * 0.2f32 as f64 - 0.1f32 as f64) * 2.0;
    e.delta = Vec3::new(dx, dy, dz);
    e
}

fn data(e: &mut Entity) -> &mut OrbData {
    match &mut e.kind {
        EntityKind::ExperienceOrb(d) => d,
        _ => unreachable!("not an experience orb"),
    }
}

/// `ExperienceOrb.tick`.
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    e.base_tick(level);
    let stuck = !no_collision(level, &e.bounding_box());
    if e.fluid.is_eye_in_water() {
        let v = e.delta;
        e.delta = Vec3::new(v.x * 0.9900000095367432, jmin(v.y + 5.000000237487257e-4, 0.05999999865889549), v.z * 0.9900000095367432);
    } else if !stuck {
        e.apply_gravity();
    }
    if physics::fluid_state(level.block(e.block_position())).kind.is_lava() {
        let dx = ((e.random.next_float() - e.random.next_float()) * 0.2) as f64;
        let dz = ((e.random.next_float() - e.random.next_float()) * 0.2) as f64;
        e.delta = Vec3::new(dx, 0.20000000298023224, dz);
    }
    if e.tick_count % 20 == 1 {
        scan_for_merges(e, level);
    }
    follow_nearby_player(e, level);
    if data(e).following.is_none() && stuck {
        let moved = e.bounding_box().offset_vec(e.delta);
        if !no_collision(level, &moved) {
            let bb = e.bounding_box();
            e.move_towards_closest_space(level, e.x(), (bb.min_y + bb.max_y) / 2.0, e.z());
            e.needs_sync = true;
        }
    }
    let dy = e.delta.y;
    e.do_move(level, MoverType::SelfMove, e.delta);
    e.apply_effects_from_blocks(level);
    let mut drag = e.air_drag();
    if e.on_ground {
        drag *= physics::block_factors(level.block(e.block_pos_below_that_affects_movement(level))).friction;
    }
    e.delta = e.delta.scale(drag as f64);
    if e.vertical_collision_below && dy < -e.gravity() {
        e.delta = Vec3::new(e.delta.x, -dy * 0.4, e.delta.z);
    }
    let d = data(e);
    d.age += 1;
    if d.age >= LIFETIME {
        e.discard();
    }
}

/// `Level.noCollision(AABB)`: no entity context.
fn no_collision(level: &dyn EntityLevel, bx: &crate::math::Aabb) -> bool {
    collision::no_collision(level, &CollisionContext::EMPTY, i32::MIN, bx)
}

/// `scanForMerges`: absorbs orbs of equal value whose id differs by a multiple of 40.
fn scan_for_merges(e: &mut Entity, level: &mut dyn EntityLevel) {
    let (id, value) = (e.id, data(e).value);
    let area = e.bounding_box().inflate_all(0.5);
    let candidates: Vec<i32> = level
        .entities_in(&area, EntityFilter::ExperienceOrb, id)
        .into_iter()
        .filter(|&o| {
            level.entity(o).is_some_and(|other| match &other.kind {
                EntityKind::ExperienceOrb(d) => !other.is_removed() && (other.id - id) % 40 == 0 && d.value == value,
                _ => false,
            })
        })
        .collect();
    for o in candidates {
        let Some(other) = level.entity_mut(o) else { continue };
        let (count, age) = match &other.kind {
            EntityKind::ExperienceOrb(d) => (d.count, d.age),
            _ => continue,
        };
        other.discard();
        let d = data(e);
        d.count += count;
        d.age = d.age.min(age);
    }
}

/// `followNearbyPlayer`.
fn follow_nearby_player(e: &mut Entity, level: &mut dyn EntityLevel) {
    let current = data(e).following.and_then(|id| level.player(id));
    let keep = current.is_some_and(|p| !p.spectator && distance_sqr(p.pos, e.position()) <= 64.0);
    let target = if keep {
        current
    } else {
        // Level.getNearestPlayer(entity, 8): nearest non-spectator within 8 blocks.
        let mut best = None;
        let mut best_d = -1.0;
        let at = e.position();
        // (the players around, from the level's grid: every orb asks every tick)
        let area = crate::math::Aabb::new(at.x - 9.0, at.y - 9.0, at.z - 9.0, at.x + 9.0, at.y + 9.0, at.z + 9.0);
        for p in level.players_in(&area).iter().filter(|p| !p.spectator) {
            let d = distance_sqr(p.pos, e.position());
            if d < 64.0 && (best_d == -1.0 || d < best_d) {
                best_d = d;
                best = Some(*p);
            }
        }
        best
    };
    data(e).following = target.map(|p| p.id);
    if let Some(p) = target {
        let v = Vec3::new(p.pos.x - e.x(), p.pos.y + p.eye_height as f64 / 2.0 - e.y(), p.pos.z - e.z());
        let len2 = v.length_sqr();
        let f = 1.0 - len2.sqrt() / 8.0;
        e.delta = e.delta + v.normalize().scale(f * f * 0.1);
    }
}

fn distance_sqr(a: Vec3, b: Vec3) -> f64 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    dx * dx + dy * dy + dz * dz
}

/// `ExperienceOrb.hurtServer`.
pub fn hurt(e: &mut Entity, _level: &mut dyn EntityLevel, kind: DamageKind, amount: f32) -> bool {
    if e.is_invulnerable_to_base(kind) {
        return false;
    }
    let d = data(e);
    d.health = (d.health as f32 - amount) as i32;
    if d.health <= 0 {
        e.discard();
    }
    true
}
