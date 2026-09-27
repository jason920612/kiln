//! `ItemEntity`: dropped item stacks — fluid buoyancy, ground friction, merging, pickup delay
//! and despawning.

use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Vec3, floor};
use crate::physics;
use kiln_item::{HolderSet, ItemStack, keys};
use kiln_javamath::random::RandomSource;

pub const LIFETIME: i32 = 6000;
pub const INFINITE_PICKUP_DELAY: i32 = 32767;
pub const INFINITE_LIFETIME: i32 = -32768;

#[derive(Clone, Debug)]
pub struct ItemData {
    pub stack: ItemStack,
    pub age: i32,
    pub pickup_delay: i32,
    pub health: i32,
    pub thrower: Option<u128>,
    /// The only player allowed to pick it up (`target`).
    pub target: Option<u128>,
    pub bob_offset: f32,
}

impl ItemData {
    pub fn new(stack: ItemStack) -> ItemData {
        ItemData { stack, age: 0, pickup_delay: 0, health: 5, thrower: None, target: None, bob_offset: 0.0 }
    }
}

/// A new item entity as `new ItemEntity(EntityType.ITEM, level)` makes it (random bob offset
/// and yaw), with `stack`.
pub fn new(id: i32, uuid: u128, stack: ItemStack, random_seed: i64) -> Entity {
    let mut e = Entity::new("minecraft:item", id, uuid, EntityKind::Item(ItemData::new(stack)), random_seed);
    let bob = e.random.next_float() * 3.1415927 * 2.0;
    e.y_rot = e.random.next_float() * 360.0;
    if let EntityKind::Item(d) = &mut e.kind {
        d.bob_offset = bob;
    }
    e
}

/// `new ItemEntity(level, x, y, z, stack)`: at a position with a small random throw.
pub fn new_at(id: i32, uuid: u128, stack: ItemStack, pos: Vec3, random_seed: i64) -> Entity {
    let mut e = new(id, uuid, stack, random_seed);
    e.set_pos(pos);
    let dx = e.random.next_double() * 0.2 - 0.1;
    let dz = e.random.next_double() * 0.2 - 0.1;
    e.delta = Vec3::new(dx, 0.2, dz);
    e
}

fn data(e: &Entity) -> &ItemData {
    match &e.kind {
        EntityKind::Item(d) => d,
        _ => unreachable!("not an item entity"),
    }
}

fn data_mut(e: &mut Entity) -> &mut ItemData {
    match &mut e.kind {
        EntityKind::Item(d) => d,
        _ => unreachable!("not an item entity"),
    }
}

/// `ItemStack.canBeHurtBy`: false when the `damage_resistant` component covers the source.
fn can_be_hurt_by(stack: &ItemStack, kind: DamageKind) -> bool {
    let Some(resistant) = stack.get(keys::DAMAGE_RESISTANT) else { return true };
    let (tag, name) = match kind {
        DamageKind::OnFire => ("minecraft:is_fire", "minecraft:on_fire"),
        DamageKind::InFire => ("minecraft:is_fire", "minecraft:in_fire"),
        DamageKind::Lava => ("minecraft:is_fire", "minecraft:lava"),
        DamageKind::HotFloor => ("minecraft:is_fire", "minecraft:hot_floor"),
        DamageKind::Explosion => ("minecraft:is_explosion", "minecraft:explosion"),
        DamageKind::Cactus => ("", "minecraft:cactus"),
        _ => ("", ""),
    };
    match &resistant.types {
        HolderSet::Tag(t) => t.as_str() != tag,
        HolderSet::Direct(ids) => {
            let id = kiln_data::synced_id("minecraft:damage_type", name);
            !id.is_some_and(|id| ids.contains(&id))
        }
    }
}

/// `ItemEntity.fireImmune`.
pub fn fire_immune(d: &ItemData) -> bool {
    !can_be_hurt_by(&d.stack, DamageKind::InFire)
}

/// `ItemEntity.tick`.
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    if data(e).stack.is_empty() {
        e.discard();
        return;
    }
    e.base_tick(level);
    {
        let d = data_mut(e);
        if d.pickup_delay > 0 && d.pickup_delay != INFINITE_PICKUP_DELAY {
            d.pickup_delay -= 1;
        }
    }
    e.old_pos = e.position();
    let start_delta = e.delta;
    if e.is_in_water() && e.fluid_height_water() > 0.10000000149011612 {
        set_fluid_movement(e, 0.9900000095367432);
    } else if e.is_in_lava() && e.fluid_height_lava() > 0.10000000149011612 {
        set_fluid_movement(e, 0.949999988079071);
    } else {
        e.apply_gravity();
    }
    let ctx = e.collision_context();
    e.no_physics = !crate::collision::no_collision(level, &ctx, e.id, &e.bounding_box().deflate_all(1.0e-7));
    if e.no_physics {
        let bb = e.bounding_box();
        e.move_towards_closest_space(level, e.x(), (bb.min_y + bb.max_y) / 2.0, e.z());
    }
    if !e.on_ground || e.delta.horizontal_distance_sqr() > 9.999999747378752e-6 || (e.tick_count + e.id) % 4 == 0 {
        e.do_move(level, MoverType::SelfMove, e.delta);
        e.apply_effects_from_blocks(level);
        let drag = e.air_drag();
        let mut friction = drag;
        if e.on_ground {
            let below = e.block_pos_below_that_affects_movement(level);
            friction *= physics::block_factors(level.block(below)).friction;
        }
        e.delta = e.delta.multiply(friction as f64, drag as f64, friction as f64);
        if e.on_ground && e.delta.y < 0.0 {
            e.delta = e.delta.multiply(1.0, -0.5, 1.0);
        }
    } else {
        e.apply_effects_from_last_movements(level);
    }
    let old = e.old_pos;
    let moved = floor(old.x) != floor(e.x()) || floor(old.y) != floor(e.y()) || floor(old.z) != floor(e.z());
    let period = if moved { 2 } else { 40 };
    if e.tick_count % period == 0 && is_mergable(e) {
        merge_with_neighbours(e, level);
    }
    {
        let d = data_mut(e);
        if d.age != INFINITE_LIFETIME {
            d.age += 1;
        }
    }
    e.needs_sync |= e.update_fluid_interaction(level);
    if (e.delta - start_delta).length_sqr() > 0.01 {
        e.needs_sync = true;
    }
    if data(e).age >= LIFETIME {
        e.discard();
    }
}

/// `setUnderwaterMovement` / `setUnderLavaMovement`.
fn set_fluid_movement(e: &mut Entity, multiplier: f64) {
    let v = e.delta;
    let lift = if v.y < 0.05999999865889549 { 5.0e-4f32 } else { 0.0 };
    e.delta = Vec3::new(v.x * multiplier, v.y + lift as f64, v.z * multiplier);
}

/// `isMergable`.
pub fn is_mergable(e: &Entity) -> bool {
    let d = data(e);
    e.is_alive()
        && d.pickup_delay != INFINITE_PICKUP_DELAY
        && d.age != INFINITE_LIFETIME
        && d.age < LIFETIME
        && d.stack.count() < d.stack.max_stack_size()
}

/// `mergeWithNeighbours`.
fn merge_with_neighbours(e: &mut Entity, level: &mut dyn EntityLevel) {
    if !is_mergable(e) {
        return;
    }
    let area = e.bounding_box().inflate(0.5, 0.0, 0.5);
    let ids = level.entities_in(&area, EntityFilter::Item, e.id);
    // The class query filters with isMergable before the loop re-checks it.
    let candidates: Vec<i32> =
        ids.into_iter().filter(|&id| level.entity(id).is_some_and(|o| matches!(o.kind, EntityKind::Item(_)) && is_mergable(o))).collect();
    for id in candidates {
        let Some(other) = level.entity_mut(id) else { continue };
        if !is_mergable(other) {
            continue;
        }
        try_to_merge(e, other);
        if e.is_removed() {
            break;
        }
    }
}

/// `tryToMerge`: the smaller stack flows into the larger one.
fn try_to_merge(e: &mut Entity, other: &mut Entity) {
    let (mine, theirs) = (data(e), data(other));
    if mine.target != theirs.target || !are_mergable(&mine.stack, &theirs.stack) {
        return;
    }
    if theirs.stack.count() < mine.stack.count() {
        merge_into(e, other);
    } else {
        merge_into(other, e);
    }
}

/// `ItemEntity.areMergable`.
pub fn are_mergable(a: &ItemStack, b: &ItemStack) -> bool {
    if b.count() + a.count() > b.max_stack_size() {
        return false;
    }
    a.is_same_item_same_components(b)
}

/// `merge(target, targetStack, source, sourceStack)`.
fn merge_into(target: &mut Entity, source: &mut Entity) {
    let src_stack = data(source).stack.clone();
    let mut src_left = src_stack;
    let merged = merge_stacks(&data(target).stack, &mut src_left, 64);
    let (src_delay, src_age) = (data(source).pickup_delay, data(source).age);
    {
        let t = data_mut(target);
        t.stack = merged;
        t.pickup_delay = t.pickup_delay.max(src_delay);
        t.age = t.age.min(src_age);
    }
    let empty = src_left.is_empty();
    data_mut(source).stack = src_left;
    if empty {
        source.discard();
    }
}

/// `ItemEntity.merge(stack, other, maxCount)`: moves up to the limit from `other`.
pub fn merge_stacks(stack: &ItemStack, other: &mut ItemStack, max_count: i32) -> ItemStack {
    let n = (stack.max_stack_size().min(max_count) - stack.count()).min(other.count());
    let merged = stack.with_count(stack.count() + n);
    other.shrink(n);
    merged
}

/// `ItemEntity.hurtServer`.
pub fn hurt(e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
    if e.is_invulnerable_to_base(kind) {
        return false;
    }
    let _ = attacker;
    if !can_be_hurt_by(&data(e).stack, kind) {
        return false;
    }
    let d = data_mut(e);
    d.health = (d.health as f32 - amount) as i32;
    let dead = d.health <= 0;
    level.emit(Event::GameEvent { event: "minecraft:entity_damage", pos: e.position(), entity: Some(e.id) });
    if dead {
        e.discard();
    }
    true
}
