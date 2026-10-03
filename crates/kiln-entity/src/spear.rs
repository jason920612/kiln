//! Spears (the 26.x `kinetic_weapon` and `piercing_weapon` components): the geometry of a stab
//! (`ProjectileUtil.getHitEntitiesAlong`) and the speed conditions of a charging weapon
//! (`KineticWeapon.damageEntities`), for whoever wields one.

use crate::level::EntityLevel;
use crate::math::{Aabb, Vec3};
use kiln_item::component::{AttackRange, KineticCondition, KineticWeapon};

/// An entity a stab may hit: its network id and bounding box (the caller has applied
/// `PiercingWeapon.canHitEntity` and the broad phase).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: i32,
    pub bb: Aabb,
}

/// An entity a stab reached, and where.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub id: i32,
    pub location: Vec3,
}

/// `AttackRange.effectiveMinRange` / `effectiveMaxRange` for the wielder: a player gets the
/// plain or creative reach, anything else the plain one times the mob factor.
pub fn effective_range(range: &AttackRange, player: bool, creative: bool) -> (f32, f32) {
    if player {
        if creative { (range.min_creative_reach, range.max_creative_reach) } else { (range.min_reach, range.max_reach) }
    } else {
        (range.min_reach * range.mob_factor, range.max_reach * range.mob_factor)
    }
}

/// `Entity.isPickable` of entities that are not mobs (what `canBeHitByProjectile` asks besides
/// `isAlive`, which the caller has checked): vehicles, end crystals, falling blocks and primed
/// TNT, shulker bullets and the redirectable projectiles. (Arrows, items, orbs and the like
/// cannot be hit.)
pub fn pickable_non_mob(o: &crate::entity::Entity) -> bool {
    use crate::entity::EntityKind;
    if matches!(o.kind, EntityKind::FallingBlock(_) | EntityKind::Tnt(_)) {
        return true;
    }
    let name = o.type_name;
    name.ends_with("_boat")
        || name.ends_with("_raft")
        || name.ends_with("minecart")
        || matches!(name, "minecraft:end_crystal" | "minecraft:leash_knot" | "minecraft:shulker_bullet")
        || redirectable_projectile(name)
}

/// `#minecraft:redirectable_projectile`: fireballs and wind charges, which a hit with a weapon
/// turns around (a player's; a mob's stab leaves them be).
pub fn redirectable_projectile(type_name: &str) -> bool {
    matches!(type_name, "minecraft:fireball" | "minecraft:wind_charge" | "minecraft:breeze_wind_charge")
}

/// `Level.clip` with `ClipContext.Block.COLLIDER` and no fluids: where the first block in the way
/// of `from → to` is hit.
pub fn clip_collider(level: &dyn EntityLevel, from: Vec3, to: Vec3) -> Option<Vec3> {
    crate::clip::traverse_blocks(from, to, |p| {
        let s = level.block(p);
        let (shape, _) = crate::collision::collision_shape(s, p, &crate::collision::CollisionContext::EMPTY);
        crate::clip::shape_clip(&shape, from, to, p).map(|(location, _)| location)
    })
}

/// `AABB.ofSize(center, size, size, size)`.
fn of_size(c: Vec3, size: f64) -> Aabb {
    Aabb::new(c.x - size / 2.0, c.y - size / 2.0, c.z - size / 2.0, c.x + size / 2.0, c.y + size / 2.0, c.z + size / 2.0)
}

/// `ProjectileUtil.getHitEntitiesAlong`: the entities among `candidates` (in the order the level
/// lists them) a stab from the eyes at `eye` along `look` reaches, with the wielder's attack range
/// (`min_range` to `max_range`, the latter lengthened by how fast the wielder moves along its look)
/// and the range's `margin` around hit boxes. `clip` finds the first block in a segment's way. A
/// block between the eyes and the start of the range (or none of the entities) means no hits.
pub fn hit_entities_along(
    eye: Vec3,
    look: Vec3,
    min_range: f32,
    max_range: f32,
    known_movement: Vec3,
    margin: f32,
    clip: &dyn Fn(Vec3, Vec3) -> Option<Vec3>,
    candidates: &[Candidate],
) -> Vec<Hit> {
    let start = eye + look.scale(min_range as f64);
    let d = known_movement.dot(look);
    let mut end = eye + look.scale(max_range as f64 + 0.0f64.max(d));
    if let Some(block) = clip(eye, end) {
        end = block;
        if eye.distance_to_sqr(end) < eye.distance_to_sqr(start) {
            return Vec::new();
        }
    }
    let margin = margin as f64;
    let area = of_size(start, margin).expand_towards_vec(end - start).inflate_all(1.0);
    let mut hits = Vec::new();
    for c in candidates {
        if !area.intersects(&c.bb) {
            continue;
        }
        let bb = c.bb;
        // `getManyEntityHitResult(..., includeInside, returnClip)`.
        if bb.contains(start) {
            hits.push(Hit { id: c.id, location: start });
            continue;
        }
        if let Some(l) = bb.clip(start, end) {
            hits.push(Hit { id: c.id, location: l });
            continue;
        }
        if margin <= 0.0 {
            continue;
        }
        let Some(enter) = bb.inflate_all(margin).clip(start, end) else { continue };
        let mut center = bb.center();
        if let Some(block) = clip(enter, center) {
            center = block;
        }
        if let Some(l) = bb.clip(enter, center) {
            hits.push(Hit { id: c.id, location: l });
        }
    }
    hits
}

/// `KineticWeapon.Condition.test(ticks, speed, relativeSpeed, mobFactor)`.
pub fn condition_holds(c: &KineticCondition, ticks: i32, speed: f64, relative_speed: f64, mob_factor: f64) -> bool {
    ticks <= c.max_duration_ticks && speed >= c.min_speed as f64 * mob_factor && relative_speed >= c.min_relative_speed as f64 * mob_factor
}

/// What a charging weapon does to one entity it touches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KineticHit {
    pub dismount: bool,
    pub knockback: bool,
    pub damage: bool,
    /// The damage dealt when `damage`: the wielder's base attack damage plus the weapon's multiple
    /// of the relative speed, floored.
    pub amount: f32,
}

/// `KineticWeapon.damageEntities` for one touched entity: the conditions by `ticks` (ticks of use
/// past the delay), the wielder's speed along its look (`speed`, blocks per second) and the
/// relative speed. `None` when no condition holds.
pub fn kinetic_hit(w: &KineticWeapon, ticks: i32, speed: f64, target_speed: f64, mob_factor: f64, base_attack_damage: f64) -> Option<KineticHit> {
    let relative = 0.0f64.max(speed - target_speed);
    let test = |c: &Option<KineticCondition>| c.as_ref().is_some_and(|c| condition_holds(c, ticks, speed, relative, mob_factor));
    let (dismount, knockback, damage) = (test(&w.dismount_conditions), test(&w.knockback_conditions), test(&w.damage_conditions));
    if !(dismount || knockback || damage) {
        return None;
    }
    let amount = base_attack_damage as f32 + crate::math::floor(relative * w.damage_multiplier as f64) as f32;
    Some(KineticHit { dismount, knockback, damage, amount })
}

/// `KineticWeapon.computeDamageUseDuration`: the delay and how long the damage condition holds.
pub fn damage_use_duration(w: &KineticWeapon) -> i32 {
    w.delay_ticks + w.damage_conditions.as_ref().map_or(0, |c| c.max_duration_ticks)
}

/// `KineticWeapon.getMotion`'s scale: the known speed per tick as per second.
pub const MOTION_SCALE: f64 = 20.0;
