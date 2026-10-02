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
    /// `Arrow` potion effects (`minecraft:mob_effect` name, duration, amplifier): a stray's
    /// slowness. Not saved yet.
    pub effects: Vec<(&'static str, i32, i32)>,
    /// `AbstractArrow.Pickup` ordinal: 0 disallowed, 1 allowed, 2 creative only.
    pub pickup: u8,
    /// `pickupItemStack`: what a player picking the arrow up gets (`None`: the type's item).
    pub pickup_item: Option<kiln_item::ItemStack>,
    /// `firedFromWeapon` (the bow or crossbow).
    pub weapon: Option<kiln_item::ItemStack>,
    /// `getPierceLevel` and the entities it went through (`piercingIgnoreEntityIds`).
    pub pierce_level: u8,
    pub pierced: Vec<i32>,
    /// Entities a piercing arrow killed (`piercedAndKilledEntities`, `killed_by_arrow`).
    pub killed: Vec<crate::level::Seen>,
    /// The weapon's knockback (`EnchantmentHelper.modifyKnockback`, punch), worked out when
    /// it was shot.
    pub knockback: f64,
    /// Spectral arrows: ticks of glowing their hit gives (`duration`).
    pub glowing: i32,
}

/// `AbstractArrow.Pickup`.
pub const PICKUP_DISALLOWED: u8 = 0;
pub const PICKUP_ALLOWED: u8 = 1;
pub const PICKUP_CREATIVE_ONLY: u8 = 2;

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
        effects: Vec::new(),
        pickup: PICKUP_DISALLOWED,
        pickup_item: None,
        weapon: None,
        pierce_level: 0,
        pierced: Vec::new(),
        killed: Vec::new(),
        knockback: 0.0,
        glowing: 200,
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
    let (owner, pierced) = match &e.kind {
        EntityKind::Arrow(d) => (if d.left_owner { None } else { d.owner }, d.pierced.clone()),
        _ => (None, Vec::new()),
    };
    // Every entity on the segment (`findHitEntities`), the nearest (by position) first.
    let mut targets: Vec<(f64, i32, Vec3)> = Vec::new();
    for id in level.entities_in(&area, EntityFilter::Any, e.id) {
        let Some(t) = level.entity(id) else { continue };
        if !(can_be_hit_by_projectile(t) || is_vehicle(t)) || pierced.contains(&id) {
            continue;
        }
        // (`canHitEntity`: until it has left its owner, an arrow passes what rides or is ridden
        // with the owner: `isPassengerOfSameVehicle`.)
        if owner.is_some_and(|o| root_vehicle(level, o) == root_vehicle(level, id)) {
            continue;
        }
        if let Some(p) = entity_hit_point(e, level, t, margin as f64, from, end) {
            targets.push((from.distance_to_sqr(t.position()), id, p));
        }
    }
    // (`ArrayList.sort`: stable.)
    targets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let dest = targets.first().map_or(end, |&(_, _, p)| p);
    e.set_pos(dest);
    e.apply_effects_between(level, from, dest);
    if targets.is_empty() {
        if let (true, Some((pos, face, location))) = (e.is_alive(), block) {
            hit_block(e, level, pos, face, location);
            e.needs_sync = true;
        }
        return;
    }
    if e.is_alive() && !e.no_physics {
        // `hitTargetsOrDeflectSelf`: each target in turn until the arrow is gone (a hit that did
        // not land bounces it back and it goes on to the next one).
        for (_, id, location) in targets {
            let owner = data(e).owner;
            // `hitTargetOrDeflectSelf`: a breeze turns the arrow back; the rest of the targets
            // are left alone.
            if crate::projectile::deflected_by_target(e, level, id) {
                break;
            }
            if !hit_living(e, level, id, owner) {
                level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: e.type_name, owner, hit: Hit::Entity { id, location } });
                e.discard();
            }
            if !e.is_alive() {
                break;
            }
        }
        e.needs_sync = true;
    }
}

/// `Entity.getRootVehicle`: the entity at the bottom of the stack `id` rides.
fn root_vehicle(level: &dyn EntityLevel, id: i32) -> i32 {
    let mut at = id;
    while let Some(v) = level.entity(at).and_then(|e| e.vehicle).or_else(|| level.player(at).and_then(|p| p.vehicle)) {
        if v == at {
            break;
        }
        at = v;
    }
    at
}

/// Where the segment `from..to` meets `t` (`ProjectileUtil.getManyEntityHitResult` as the arrow
/// calls it): its own box first; failing that, a box inflated by `margin` the segment enters,
/// when a clear line from there to the box's centre (blocks stop it) reaches the box itself:
/// the entry point of the inflated box.
fn entity_hit_point(e: &Entity, level: &dyn EntityLevel, t: &Entity, margin: f64, from: Vec3, to: Vec3) -> Option<Vec3> {
    if t.type_name == "minecraft:ender_dragon" {
        return crate::projectile::clip_entity(t, margin, from, to);
    }
    let bb = t.bounding_box();
    if let Some(p) = bb.clip(from, to) {
        return Some(p);
    }
    if margin <= 0.0 {
        return None;
    }
    let entry = bb.inflate_all(margin).clip(from, to)?;
    let mut target = bb.center();
    let ctx = e.collision_context();
    if let Some((_, _, location)) = clip::traverse_blocks(entry, target, |pos| {
        let (shape, _) = collision::collision_shape(level.block(pos), pos, &ctx);
        clip::shape_clip(&shape, entry, target, pos).map(|(location, face)| (pos, face, location))
    }) {
        target = location;
    }
    bb.clip(entry, target).map(|_| entry)
}

/// Boats and minecarts: pickable, so arrows hit them (`isPickable` is `!isRemoved()`).
fn is_vehicle(t: &Entity) -> bool {
    t.is_alive() && matches!(&t.kind, EntityKind::Ext(x) if x.attackable())
}

/// `AbstractArrow.onHitEntity` for a mob or a player: damage from the speed and base damage
/// (a critical arrow adds a random bonus), then the arrow breaks, or bounces back when the hit
/// did not land. Returns false for other entities (the simulation handles them).
fn hit_living(e: &mut Entity, level: &mut dyn EntityLevel, id: i32, owner: Option<i32>) -> bool {
    let is_player = level.player(id).is_some();
    // An end crystal explodes; the arrow is gone.
    if crate::projectile::hurt_crystal(level, id, crate::level::DamageKind::Arrow, 0.0, owner.or(Some(e.id))) {
        e.discard();
        return true;
    }
    let vehicle = level.entity(id).is_some_and(is_vehicle);
    let target = match level.entity(id) {
        Some(t) if matches!(t.kind, EntityKind::Mob(_)) || is_player || vehicle => t.position(),
        _ => return false,
    };
    let v = e.delta;
    let speed = v.length() as f32;
    let (base, crit) = { let d = data(e); (d.base_damage, d.crit) };
    let mut damage = crate::mob::mth::ceil((speed as f64 * base).clamp(0.0, 2.147483647e9));
    // Piercing: the arrow goes on through up to `pierce_level` entities.
    let pierce = data(e).pierce_level;
    if pierce > 0 {
        if data(e).pierced.len() >= pierce as usize + 1 {
            e.discard();
            return true;
        }
        data(e).pierced.push(id);
    }
    if crit {
        let bonus = e.random.next_int_bounded(damage / 2 + 2) as i64;
        damage = (bonus + damage as i64).min(i32::MAX as i64) as i32;
    }
    let owner_is_player = owner.is_some_and(|o| level.player(o).is_some());
    // Knockback goes along the arrow's motion (`calculateHorizontalHurtKnockbackDirection`).
    let source = crate::mob::DamageSource {
        kind: crate::level::DamageKind::Arrow,
        attacker: owner.or(Some(e.id)),
        direct: Some(e.id),
        pos: Some(Vec3::new(target.x - v.x, target.y, target.z - v.z)),
        attacker_is_player: owner_is_player,
    };
    // (`getRemainingFireTicks` before the arrow's fire: given back when the hit does not land.)
    let fire_before = level.entity(id).map(|t| t.remaining_fire_ticks);
    if e.is_on_fire() {
        level.ignite(id, 5.0);
    }
    let hurt = if is_player {
        level.hurt_player(id, source, damage as f32)
    } else if vehicle {
        // A boat or minecart: `VehicleEntity.hurtServer` (a TNT minecart reads the arrow).
        let (on_fire, speed_sqr) = (e.is_on_fire(), e.delta.length_sqr());
        let Some(slot) = level.entity_mut(id) else { return false };
        let mut t = std::mem::replace(slot, Entity::new("minecraft:marker", i32::MIN, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
        let r = t.hurt_by_projectile(level, crate::level::DamageKind::Arrow, damage as f32, owner.or(Some(e.id)), on_fire, speed_sqr);
        if let Some(slot) = level.entity_mut(id) {
            *slot = t;
        }
        r
    } else {
        let Some(slot) = level.entity_mut(id) else { return false };
        let mut t = std::mem::replace(slot, Entity::new("minecraft:marker", i32::MIN, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
        crate::mob::kinds::ender_dragon::aim_at(&mut t, e.position());
        // (The knockback goes opposite the arrow's own horizontal motion.)
        if let Some(m) = crate::mob::data_mut(&mut t) {
            m.knock_override = Some((-v.x, -v.z));
        }
        let r = crate::mob::hurt_entity(&mut t, level, source, damage as f32);
        if let Some(m) = crate::mob::data_mut(&mut t) {
            m.knock_override = None;
        }
        if let Some(slot) = level.entity_mut(id) {
            *slot = t;
        }
        r
    };
    if hurt && vehicle {
        let pitch = 1.2 / (e.random.next_float() * 0.2 + 0.9);
        e.play_sound(level, "minecraft:entity.arrow.hit", 1.0, pitch);
        if pierce == 0 {
            e.discard();
        }
    } else if hurt {
        // `doKnockback`: the weapon's knockback along the arrow's horizontal motion.
        let knockback = data(e).knockback;
        if knockback > 0.0 {
            let resistance = level.entity(id).and_then(crate::mob::data).map_or(0.0, |m| m.attrs.value(crate::mob::attributes::Attr::KnockbackResistance));
            let push = Vec3::new(v.x, 0.0, v.z).normalize().scale(knockback * 0.6 * (1.0 - resistance).max(0.0));
            if push.length_sqr() > 0.0 {
                level.push(id, Vec3::new(push.x, 0.1, push.z));
            }
        }
        // `Arrow.doPostHurtEffects`: an eighth of each effect's duration.
        let effects = data(e).effects.clone();
        for (effect, duration, amplifier) in effects {
            level.add_effect(id, effect, (duration / 8).max(1), amplifier, owner.or(Some(e.id)));
        }
        // `SpectralArrow.doPostHurtEffects`.
        if e.type_name == "minecraft:spectral_arrow" {
            let ticks = data(e).glowing;
            level.add_effect(id, "minecraft:glowing", ticks, 0, owner.or(Some(e.id)));
        }
        // `killed_by_arrow` for a player's arrow (every kill of a piercing one).
        if let Some(o) = owner.filter(|&o| level.player(o).is_some()) {
            let dead = match level.entity(id) {
                Some(t) => crate::mob::data(t).is_some_and(|m| m.health <= 0.0) || !t.is_alive(),
                None => true,
            };
            let seen = level.entity(id).map(crate::level::Seen::of);
            let weapon = data(e).weapon.clone();
            if pierce > 0 {
                if dead && let Some(s) = seen {
                    data(e).killed.push(s);
                }
                let victims = data(e).killed.clone();
                level.emit(Event::Criterion { player: o, criterion: crate::level::Criterion::KilledByArrow { victims, weapon } });
            } else if dead && let Some(s) = seen {
                level.emit(Event::Criterion { player: o, criterion: crate::level::Criterion::KilledByArrow { victims: vec![s], weapon } });
            }
        }
        let pitch = 1.2 / (e.random.next_float() * 0.2 + 0.9);
        e.play_sound(level, "minecraft:entity.arrow.hit", 1.0, pitch);
        if pierce == 0 {
            e.discard();
        }
    } else if !crate::projectile::receives_side_effects_on_hit(level, id, false) {
        // An enderman that teleported away: the arrow flies on through where it stood.
    } else {
        if let (Some(before), Some(t)) = (fire_before, level.entity_mut(id)) {
            t.remaining_fire_ticks = before;
        }
        // `deflect(REVERSE, ..., 0.2)`: a fifth of the motion, backwards, and the turn of `170 +
        // nextFloat() * 20` degrees.
        crate::projectile::deflect_reverse(e, Vec3::new(0.2, 0.2, 0.2));
    }
    true
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
