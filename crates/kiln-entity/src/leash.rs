//! Leads: vanilla's `Leashable` (mobs that are not enemies, and boats), the elastic pull and the
//! snapping of a lead, leads restored from a save, and what a player's clicks with a lead and
//! with shears do (`Entity.interact`, `LeadItem.bindPlayerMobs`).
//!
//! The lead is data on the leashed [`Entity`] ([`LeashData`]); the holder is another entity (a
//! player, a mob, a [`leash knot`](crate::ext_entity::leash_knot)) named by its network id. A
//! mob being ticked is out of the level, so every function that changes a leash takes the
//! leashed entity (and its mob data, when that is out of the entity) as arguments, and the
//! functions that reach another entity do their changes in two steps: the entity's own data,
//! then what the level has to do about it.

use crate::entity::{Entity, EntityKind};
use crate::ext_entity::boat::is_boat;
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::goals::{Goal, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, MobData, MobKind, path};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;

/// `Leashable.LEASH_TOO_FAR_DIST`: farther than this the lead snaps.
pub const SNAP_DISTANCE: f64 = 12.0;
/// `Leashable.LEASH_ELASTIC_DIST`: past this (less the widths of both) the lead pulls.
pub const ELASTIC_DISTANCE: f64 = 6.0;
/// `Leashable.AXIS_SPECIFIC_ELASTICITY`.
const AXIS_SPECIFIC_ELASTICITY: Vec3 = Vec3 { x: 0.8, y: 0.2, z: 0.8 };
const TORSIONAL_ELASTICITY: f64 = 10.0;
const STIFFNESS: f64 = 0.11;

/// The entity type of a leash knot.
pub const KNOT: &str = "minecraft:leash_knot";

/// What a loaded or just-made leash waits to find (`LeashData.delayedLeashInfo`), and how a
/// live holder is written to a save: by UUID, or for a knot by the block it is on.
#[derive(Clone, Debug, PartialEq)]
pub enum Delayed {
    Holder(u128),
    Knot(BlockPos),
}

/// `Leashable.LeashData`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LeashData {
    /// `leashHolder`: the network id of the entity that holds the lead.
    pub holder: Option<i32>,
    /// How the holder is saved (kept up to date while it is known).
    pub holder_key: Option<Delayed>,
    /// `delayedLeashInfo`: a loaded lead's holder, not found yet.
    pub delayed: Option<Delayed>,
    pub angular_momentum: f64,
}

// ---------------------------------------------------------------------------- queries

/// `instanceof Leashable`: mobs and boats.
pub fn is_leashable(e: &Entity) -> bool {
    matches!(e.kind, EntityKind::Mob(_) | EntityKind::MobTicking { .. }) || is_boat(e.type_name)
}

/// `Leashable.isLeashed`.
pub fn is_leashed(e: &Entity) -> bool {
    e.leash.as_ref().is_some_and(|d| d.holder.is_some())
}

/// `Leashable.getLeashHolder`.
pub fn holder_of(e: &Entity) -> Option<i32> {
    e.leash.as_ref().and_then(|d| d.holder)
}

/// `Entity.canInteractWithLevel` of an entity: alive (a living one with health left).
fn alive(e: &Entity) -> bool {
    !e.is_removed() && mob::data(e).is_none_or(|m| m.health > 0.0)
}

/// `Mob.canBeLeashed` and the types' overrides.
pub fn can_be_leashed_mob(m: &MobData, level: &dyn EntityLevel) -> bool {
    use MobKind::*;
    match m.kind {
        // Ambient, fish and the like.
        Bat | Cod | Salmon | TropicalFish | Pufferfish | Tadpole | Turtle | Panda | Villager => false,
        Squid | GlowSquid | Axolotl | Hoglin | Zoglin => true,
        // `Wolf.canBeLeashed`: not while angry.
        Wolf => !mob::kinds::wolf::is_angry(m, level),
        k => k.category() != mob::Category::Monster,
    }
}

/// `Leashable.canBeLeashed`; `m` is the mob data of `e` when it is out of the entity.
pub fn can_be_leashed(e: &Entity, m: Option<&MobData>, level: &dyn EntityLevel) -> bool {
    match m.or_else(|| mob::data(e)) {
        Some(m) => can_be_leashed_mob(m, level),
        None => is_boat(e.type_name),
    }
}

/// `Leashable.leashDistanceTo`: between the centres of the boxes.
pub fn leash_distance_to(e: &Entity, other: &Entity) -> f64 {
    let (a, b) = (other.bounding_box().center(), e.bounding_box().center());
    a.distance_to_sqr(b).sqrt()
}

/// `Leashable.canHaveALeashAttachedTo(holder)`.
pub fn can_have_leash_attached_to(e: &Entity, m: Option<&MobData>, holder: &Entity, level: &dyn EntityLevel) -> bool {
    e.id != holder.id && leash_distance_to(e, holder) <= SNAP_DISTANCE && can_be_leashed(e, m, level)
}

/// `Leashable.leashableLeashedTo` / `leashableInArea(level, centre, leashed to holder)`: the
/// leashables within 16 blocks of `centre` (a 32 block cube) that `holder` holds, in the level's
/// entity order.
pub fn leashed_to(level: &dyn EntityLevel, holder: i32, centre: Vec3) -> Vec<i32> {
    let area = Aabb::new(centre.x - 16.0, centre.y - 16.0, centre.z - 16.0, centre.x + 16.0, centre.y + 16.0, centre.z + 16.0);
    level
        .entities_in(&area, EntityFilter::Any, i32::MIN)
        .into_iter()
        .filter(|&id| level.entity(id).is_some_and(|o| is_leashable(o) && holder_of(o) == Some(holder)))
        .collect()
}

fn key_of(holder: &Entity) -> Delayed {
    if holder.type_name == KNOT {
        let p = holder.position();
        Delayed::Knot(BlockPos::containing(p.x, p.y, p.z))
    } else {
        Delayed::Holder(holder.uuid)
    }
}

/// The mob data of an entity: the one handed in, or the entity's own.
fn mob_of<'a>(e: &'a mut Entity, m: &'a mut Option<&mut MobData>) -> Option<&'a mut MobData> {
    match m {
        Some(m) => Some(&mut **m),
        None => mob::data_mut(e),
    }
}

// ---------------------------------------------------------------------------- changes

/// The data half of `Leashable.setLeashedTo`: `e` is now led by `holder` (key: how a save
/// names it). Returns the previous holder and the vehicle `e` got off.
fn assign(e: &mut Entity, holder: i32, key: Option<Delayed>) -> (Option<i32>, Option<i32>) {
    let data = e.leash.get_or_insert_with(Default::default);
    let old = data.holder;
    data.holder = Some(holder);
    data.holder_key = key;
    data.delayed = None;
    // `isPassenger` → `stopRiding`.
    (old, e.vehicle.take())
}

/// What the level does after [`assign`]: the old holder hears of it, the vehicle loses its rider.
fn after_assign(level: &mut dyn EntityLevel, leashee: i32, holder: i32, (old, vehicle): (Option<i32>, Option<i32>)) {
    if let Some(old) = old.filter(|&o| o != holder) {
        notify_leashee_removed(level, old);
    }
    if let Some(v) = vehicle
        && let Some(vp) = level.entity_mut(v)
    {
        crate::ride::remove_passenger(vp, leashee);
    }
}

/// `Leashable.setLeashedTo(holder, true)` for the entity `e`, which is not in the level; `holder`
/// is another entity (in the level, or `holder_entity`).
pub fn set_leashed_to(e: &mut Entity, level: &mut dyn EntityLevel, holder: &Entity) {
    if e.id == holder.id {
        return;
    }
    let r = assign(e, holder.id, Some(key_of(holder)));
    after_assign(level, e.id, holder.id, r);
}

/// `Leashable.setLeashedTo(holder, true)` for entity `id` of the level.
pub fn set_leashed_to_in_level(level: &mut dyn EntityLevel, id: i32, holder: i32) {
    if id == holder {
        return;
    }
    let key = level.entity(holder).map(key_of);
    let Some(t) = level.entity_mut(id) else { return };
    let r = assign(t, holder, key);
    after_assign(level, id, holder, r);
}

/// The data half of `Leashable.dropLeash(e, send, drop)`: the holder and where `e` was, if it
/// was led.
fn release(e: &mut Entity, m: &mut Option<&mut MobData>) -> Option<(i32, Vec3)> {
    let holder = e.leash.as_ref()?.holder?;
    e.leash = None;
    // `Mob.onLeashRemoved`: the home the lead gave goes with it.
    if let Some(m) = mob_of(e, m) {
        m.home = None;
    }
    Some((holder, e.position()))
}

fn finish_release(level: &mut dyn EntityLevel, r: (i32, Vec3), drop_item: bool) {
    if drop_item && let Some(lead) = ItemStack::of("minecraft:lead", 1) {
        mob::spawn_at(r.1, level, lead, 0.0);
    }
    notify_leashee_removed(level, r.0);
}

/// `Leashable.dropLeash`: the lead is gone, as an item where the entity is.
pub fn drop_leash(e: &mut Entity, mut m: Option<&mut MobData>, level: &mut dyn EntityLevel) {
    if let Some(r) = release(e, &mut m) {
        finish_release(level, r, true);
    }
}

/// `Leashable.removeLeash`: the lead is gone, no item.
pub fn remove_leash(e: &mut Entity, mut m: Option<&mut MobData>, level: &mut dyn EntityLevel) {
    if let Some(r) = release(e, &mut m) {
        finish_release(level, r, false);
    }
}

/// `dropLeash` / `removeLeash` of entity `id` of the level.
pub fn release_in_level(level: &mut dyn EntityLevel, id: i32, drop_item: bool) {
    let Some(t) = level.entity_mut(id) else { return };
    let mut none = None;
    if let Some(r) = release(t, &mut none) {
        finish_release(level, r, drop_item);
    }
}

/// `Entity.notifyLeasheeRemoved` of the holder: a knot with nothing led to it goes away.
fn notify_leashee_removed(level: &mut dyn EntityLevel, holder: i32) {
    let Some(h) = level.entity(holder) else { return };
    if h.type_name != KNOT || h.is_removed() {
        return;
    }
    let at = h.position();
    if leashed_to(level, holder, at).is_empty()
        && let Some(h) = level.entity_mut(holder)
    {
        h.discard();
    }
}

// ---------------------------------------------------------------------------- the tick

/// `Leashable.restoreLeashFromSave`: a loaded lead finds its holder (a knot is made if none is
/// there), or after 100 ticks without one the lead falls off.
fn restore(e: &mut Entity, level: &mut dyn EntityLevel) {
    let Some(delayed) = e.leash.as_ref().and_then(|d| d.delayed.clone()) else { return };
    match delayed {
        Delayed::Holder(uuid) => {
            if let Some(h) = level.entity_by_uuid(uuid) {
                let (id, key) = (h.id, key_of(h));
                let r = assign(e, id, Some(key));
                after_assign(level, e.id, id, r);
                return;
            }
        }
        Delayed::Knot(pos) => {
            let knot = find_knot(level, pos);
            let id = match knot {
                Some(id) => id,
                None => create_knot(level, pos),
            };
            let r = assign(e, id, Some(Delayed::Knot(pos)));
            after_assign(level, e.id, id, r);
            return;
        }
    }
    if e.tick_count > 100 {
        if let Some(lead) = ItemStack::of("minecraft:lead", 1) {
            mob::spawn_at(e.position(), level, lead, 0.0);
        }
        e.leash = None;
    }
}

/// `LeashFenceKnotEntity.getKnot`: the knot on the block at `pos`.
pub fn find_knot(level: &dyn EntityLevel, pos: BlockPos) -> Option<i32> {
    let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
    let area = Aabb::new(x - 1.0, y - 1.0, z - 1.0, x + 1.0, y + 1.0, z + 1.0);
    level.entities_in(&area, EntityFilter::Any, i32::MIN).into_iter().find(|&id| {
        level.entity(id).is_some_and(|o| {
            o.type_name == KNOT && {
                let p = o.position();
                BlockPos::containing(p.x, p.y, p.z) == pos
            }
        })
    })
}

/// `LeashFenceKnotEntity.createKnot`: a new knot on the block at `pos`. Its id is a stand-in
/// the simulation replaces (the same for the same block, so leads made to it this tick find it).
pub fn create_knot(level: &mut dyn EntityLevel, pos: BlockPos) -> i32 {
    let id = -2_000_000 - ((pos.x as i64 * 31 + pos.y as i64 * 17 + pos.z as i64 * 13).rem_euclid(900_000) as i32);
    let seed = level.fresh_seed();
    level.add_entity(crate::ext_entity::leash_knot::new(id, pos, seed));
    id
}

/// `LeashFenceKnotEntity.getOrCreateKnot`.
pub fn get_or_create_knot(level: &mut dyn EntityLevel, pos: BlockPos) -> i32 {
    match find_knot(level, pos) {
        Some(id) => id,
        None => create_knot(level, pos),
    }
}

/// `Leashable.tickLeash` (in `Entity.baseTick`): restores a saved lead, drops it when either end
/// cannot take part, snaps it when too far, pulls the entity in when stretched, and turns it by
/// the lead's angular momentum.
pub fn tick_leash(e: &mut Entity, mut m: Option<&mut MobData>, level: &mut dyn EntityLevel) {
    if e.leash.as_ref().is_some_and(|d| d.delayed.is_some()) {
        restore(e, level);
    }
    let Some(holder_id) = e.leash.as_ref().and_then(|d| d.holder) else { return };
    // `Mob.startRiding` drops the lead of a mob that got on a vehicle.
    let is_alive = match m.as_deref() {
        Some(m) => !e.is_removed() && m.health > 0.0,
        None => alive(e),
    };
    let holder_alive = level.entity(holder_id).is_some_and(alive_holder);
    if !is_alive || !holder_alive || e.vehicle.is_some() {
        if e.vehicle.is_some() || level.entity_drops() {
            drop_leash(e, m.as_deref_mut(), level);
        } else {
            remove_leash(e, m.as_deref_mut(), level);
        }
    }
    let Some(holder_id) = e.leash.as_ref().and_then(|d| d.holder) else {
        // The lead went with `drop_leash`: `data.angularMomentum` of the old data still turns.
        return;
    };
    let Some(h) = level.entity(holder_id) else { return };
    let hp = h.position();
    let (h_bb_center, h_width, h_y_rot, h_height) = (h.bounding_box().center(), h.width, h.y_rot, h.height);
    let h_no_ai = mob::data(h).is_some_and(|hm| hm.no_ai);
    let h_move = level.known_movement(holder_id);
    let h_key = key_of(h);
    // The holder's identity as a save names it.
    if let Some(d) = e.leash.as_mut() {
        d.holder_key = Some(h_key);
    }
    let dist = h_bb_center.distance_to_sqr(e.bounding_box().center()).sqrt();
    // `PathfinderMob.whenLeashedTo`: the home follows the holder.
    let leasher_block = BlockPos::containing(hp.x, hp.y, hp.z);
    if let Some(m) = mob_of(e, &mut m) {
        m.home = Some((leasher_block, ELASTIC_DISTANCE as i32 - 1));
    }
    if dist > SNAP_DISTANCE {
        level.emit(Event::Sound { pos: hp, sound: "minecraft:item.lead.break", source: "neutral", volume: 1.0, pitch: 1.0 });
        // `Leashable.leashTooFarBehaviour`, and the mob's: no more walking this tick.
        drop_leash(e, m.as_deref_mut(), level);
        if let Some(m) = mob_of(e, &mut m) {
            m.goals.set_control_flag(MOVE, false);
        }
    } else if dist > ELASTIC_DISTANCE - h_width as f64 - e.width as f64
        && check_elastic_interactions(e, m.as_deref(), (hp, h_width, h_height, h_y_rot, h_no_ai, h_move), level)
    {
        on_elastic_leash_pull(e, &mut m, level);
    } else {
        close_range_leash_behaviour(e, &mut m, hp, level);
    }
    // The lead turns the entity, and the turn dies down with the ground it is on.
    let angular = e.leash.as_ref().map_or(0.0, |d| d.angular_momentum);
    e.y_rot = (e.y_rot as f64 - angular) as f32;
    let friction = angular_friction(e, level);
    if let Some(d) = e.leash.as_mut() {
        d.angular_momentum *= friction as f64;
    }
}

fn alive_holder(h: &Entity) -> bool {
    alive(h)
}

/// `Leashable.angularFriction`.
fn angular_friction(e: &Entity, level: &dyn EntityLevel) -> f32 {
    if e.on_ground {
        crate::physics::block_factors(level.block(e.block_pos_below_that_affects_movement(level))).friction * 0.91f32
    } else if e.is_in_water() || e.is_in_lava() {
        0.8f32
    } else {
        0.91f32
    }
}

/// `Leashable.getHolderMovement` of `e` (the name is vanilla's: it is the leashed entity's own).
fn own_movement(e: &Entity, m: Option<&MobData>) -> Vec3 {
    if m.or_else(|| mob::data(e)).is_some_and(|m| m.no_ai) {
        return Vec3::ZERO;
    }
    e.delta
}

/// `Leashable.checkElasticInteractions` (no quad leash holders): the spring between the two
/// attachment points pushes the entity toward its holder and twists it.
fn check_elastic_interactions(
    e: &mut Entity,
    m: Option<&MobData>,
    (hp, h_width, h_height, h_y_rot, h_no_ai, h_move): (Vec3, f32, f32, f32, bool, Vec3),
    _level: &dyn EntityLevel,
) -> bool {
    let _ = h_no_ai;
    let elastic = ELASTIC_DISTANCE;
    let movement = own_movement(e, m);
    let y_rad = e.y_rot * 0.017453292f32;
    let size = Vec3::new(e.width as f64, e.height as f64, e.width as f64);
    let holder_rad = h_y_rot * 0.017453292f32;
    let holder_size = Vec3::new(h_width as f64, h_height as f64, h_width as f64);
    // `ENTITY_ATTACHMENT_POINT` and `LEASHER_ATTACHMENT_POINT`.
    let entity_point = Vec3::new(0.0, 0.5, 0.5);
    let leasher_point = Vec3::new(0.0, 0.5, 0.0);
    let entity_offset = y_rot(entity_point.multiply_vec(size), -y_rad);
    let entity_attach = e.position() + entity_offset;
    let holder_offset = y_rot(leasher_point.multiply_vec(holder_size), -holder_rad);
    let holder_attach = hp + holder_offset;
    // `computeDampenedSpringInteraction`.
    let dist = entity_attach.distance_to_sqr(holder_attach).sqrt();
    if dist < elastic {
        return false;
    }
    let mut force = (holder_attach - entity_attach).normalize().scale(dist - elastic);
    let torque = entity_offset.z * force.x - entity_offset.x * force.z;
    if movement.dot(force) >= 0.0 {
        force = force.scale(0.30000001192092896);
    }
    // `Wrench.accumulate` of one wrench, scaled by 1.
    let relative = h_move - e.delta;
    let Some(data) = e.leash.as_mut() else { return false };
    data.angular_momentum += TORSIONAL_ELASTICITY * torque;
    let push = force.multiply_vec(AXIS_SPECIFIC_ELASTICITY) + relative.scale(STIFFNESS);
    e.delta = e.delta + push;
    true
}

/// `Vec3.yRot(float)`.
fn y_rot(v: Vec3, angle: f32) -> Vec3 {
    let c = mob::mth::cos(angle as f64);
    let s = mob::mth::sin(angle as f64);
    Vec3::new(v.x * c as f64 + v.z * s as f64, v.y, v.z * c as f64 - v.x * s as f64)
}

/// `Leashable.onElasticLeashPull`: the fall distance stops counting past a pull, a horse stops
/// eating, a sitting camel stands up.
fn on_elastic_leash_pull(e: &mut Entity, m: &mut Option<&mut MobData>, level: &mut dyn EntityLevel) {
    // `Entity.checkFallDistanceAccumulation`.
    if e.delta.y > -0.5 && e.fall_distance > 1.0 {
        e.fall_distance = 1.0;
    }
    let Some(kind) = mob_of(e, m).map(|m| m.kind) else { return };
    match kind {
        MobKind::Horse | MobKind::Donkey | MobKind::Mule | MobKind::SkeletonHorse | MobKind::Llama | MobKind::TraderLlama => {
            if let Some(m) = mob_of(e, m) {
                mob::kinds::horse::stop_eating(m);
            }
        }
        MobKind::Camel => {
            // (`Camel.onElasticLeashPull`: borrow the mob data out of the entity.)
            match m {
                Some(m) => mob::kinds::camel::elastic_pull(e, m, level),
                None => {
                    let mut taken = mob::take(e);
                    mob::kinds::camel::elastic_pull(e, &mut taken, level);
                    mob::put(e, taken);
                }
            }
        }
        _ => {}
    }
}

/// `PathfinderMob.closeRangeLeashBehaviour`: a led mob walks toward its holder, unless it
/// panics.
fn close_range_leash_behaviour(e: &mut Entity, m: &mut Option<&mut MobData>, holder_pos: Vec3, level: &mut dyn EntityLevel) {
    let Some(m) = m.as_deref_mut() else { return };
    if m.kind == MobKind::Allay || is_panicking(m) {
        return;
    }
    m.goals.set_control_flag(MOVE, true);
    let (fx, fy, fz) = ((e.x() - holder_pos.x) as f32, (e.y() - holder_pos.y) as f32, (e.z() - holder_pos.z) as f32);
    let distance = (fx * fx + fy * fy + fz * fz).sqrt();
    let v = Vec3::new(holder_pos.x - e.x(), holder_pos.y - e.y(), holder_pos.z - e.z()).normalize().scale((distance - 2.0f32).max(0.0f32) as f64);
    let speed = if matches!(m.kind, MobKind::Llama | MobKind::TraderLlama) { 2.0 } else { 1.0 };
    path::move_to(e, m, level, e.x() + v.x, e.y() + v.y, e.z() + v.z, speed);
}

/// `PathfinderMob.isPanicking`: the brain's panic memory or a running `PanicGoal`.
fn is_panicking(m: &MobData) -> bool {
    m.brain.as_ref().is_some_and(|b| b.st.mem.has(mob::brain::memory::Mem::IsPanicking)) || m.goals.is_running(|g| matches!(g, Goal::Panic { .. }))
}

// ---------------------------------------------------------------------------- clicks

fn is_item(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && mob::item_name(stack) == name
}

fn sound(e: &Entity, level: &mut dyn EntityLevel, name: &'static str) {
    if !e.silent {
        let source = mob::data(e).map_or("neutral", |m| m.kind.sound_source());
        level.emit(Event::Sound { pos: e.position(), sound: name, source, volume: 1.0, pitch: 1.0 });
    }
}

/// `Entity.interact` of a leashable (the part about leads and shears) for the click of the
/// player `who` holding `stack` on `e` (with its mob data in place). `None`: the click is not
/// about a lead.
pub fn interact(e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
    let leashable = is_leashable(e);
    // A sneaking player's click moves the leads it holds to the entity it clicked.
    if who.sneaking && leashable && can_be_leashed(e, None, &*level) && alive(e) && !mob::data(e).is_some_and(|m| m.baby()) {
        let mut any = false;
        for id in leashed_to(level, who.id, e.bounding_box().center()) {
            let ok = level.entity(id).is_some_and(|l| can_have_leash_attached_to(l, None, e, &*level));
            if ok {
                let key = key_of(e);
                if let Some(t) = level.entity_mut(id) {
                    let r = assign(t, e.id, Some(key));
                    after_assign(level, id, e.id, r);
                    any = true;
                }
            }
        }
        if any {
            level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(who.id) });
            sound(e, level, "minecraft:item.lead.tied");
            return Some(Outcome::success(HeldChange::None));
        }
    }
    // Shears cut every lead on the entity and to it.
    if is_item(stack, "minecraft:shears") && shear_off_all(e, level, who.id) {
        return Some(Outcome::success(HeldChange::Damage(1)));
    }
    if alive(e) && leashable {
        if holder_of(e) == Some(who.id) {
            if who.creative {
                remove_leash(e, None, level);
            } else {
                drop_leash(e, None, level);
            }
            level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(who.id) });
            sound(e, level, "minecraft:item.lead.untied");
            return Some(Outcome::success(HeldChange::None));
        }
        let held_by_player = holder_of(e).is_some_and(|h| level.player(h).is_some());
        if is_item(stack, "minecraft:lead") && !held_by_player {
            let ok = level.entity(who.id).is_some_and(|p| can_have_leash_attached_to(e, None, p, &*level));
            if ok {
                if is_leashed(e) {
                    drop_leash(e, None, level);
                }
                if let Some(p) = level.entity(who.id).cloned() {
                    set_leashed_to(e, level, &p);
                }
                sound(e, level, "minecraft:item.lead.tied");
                // `ItemStack.shrink(1)` whatever the game mode.
                return Some(Outcome::success(HeldChange::Shrink(1)));
            }
        }
    }
    None
}

/// `Entity.shearOffAllLeashConnections`: every lead to and on `e` is cut; the shears snip.
pub fn shear_off_all(e: &mut Entity, level: &mut dyn EntityLevel, player: i32) -> bool {
    let led = leashed_to(level, e.id, e.bounding_box().center());
    let mut any = !led.is_empty();
    if is_leashable(e) && is_leashed(e) {
        drop_leash(e, None, level);
        any = true;
    }
    for id in led {
        release_in_level(level, id, true);
    }
    if !any {
        return false;
    }
    // (`notifyLeasheeRemoved` of a knot being clicked: it is out of the level.)
    if e.type_name == KNOT && leashed_to(level, e.id, e.bounding_box().center()).is_empty() {
        e.discard();
    }
    level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(player) });
    let pos = e.position();
    level.emit(Event::Sound { pos: Vec3::new(pos.x.floor() + 0.5, pos.y.floor() + 0.5, pos.z.floor() + 0.5), sound: "minecraft:item.shears.snip", source: "players", volume: 1.0, pitch: 1.0 });
    true
}

/// `LeadItem.bindPlayerMobs`: the leads player `who` holds within reach of the fence at `pos`
/// move to the knot on it. Returns whether any did.
pub fn bind_player_mobs(level: &mut dyn EntityLevel, who: i32, pos: BlockPos) -> bool {
    let centre = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    let list = leashed_to(level, who, centre);
    if list.is_empty() {
        return false;
    }
    let existing = find_knot(level, pos);
    let knot = match existing {
        Some(k) => k,
        None => create_knot(level, pos),
    };
    // The knot's box for the reach test (a new one is not in the level yet).
    let knot_entity = level.entity(knot).cloned().unwrap_or_else(|| crate::ext_entity::leash_knot::new(knot, pos, 0));
    let mut any = false;
    for id in list {
        if level.entity(id).is_some_and(|l| can_have_leash_attached_to(l, None, &knot_entity, &*level)) {
            set_leashed_to_in_level(level, id, knot);
            any = true;
        }
    }
    if any {
        level.emit(Event::Sound { pos: knot_entity.position(), sound: "minecraft:item.lead.tied", source: "neutral", volume: 1.0, pitch: 1.0 });
        level.emit(Event::GameEvent { event: "minecraft:block_attach", pos: centre, entity: Some(who) });
        true
    } else {
        if existing.is_none() {
            discard_pending(level, knot);
        }
        false
    }
}

/// Takes back the knot made a moment ago (it is still waiting to be added to the level).
fn discard_pending(level: &mut dyn EntityLevel, id: i32) {
    if let Some(k) = level.entity_mut(id) {
        k.discard();
    }
}

// ---------------------------------------------------------------------------- saving

/// `Leashable.LeashData.CODEC`: a compound with the holder's `UUID`, or a knot's block as an
/// int array. A lead that has not found its holder yet is written as it was read.
pub fn save(e: &Entity) -> Option<Tag> {
    let d = e.leash.as_ref()?;
    let key = if d.holder.is_some() { d.holder_key.as_ref() } else { d.delayed.as_ref() }?;
    Some(match key {
        Delayed::Holder(uuid) => Tag::Compound(vec![("UUID".into(), crate::persist::uuid_to_tag(*uuid))]),
        Delayed::Knot(p) => Tag::IntArray(vec![p.x, p.y, p.z]),
    })
}

/// Reads [`save`]'s forms (`Leashable.readLeashData`).
pub fn load(tag: &Tag) -> Option<LeashData> {
    let delayed = match tag {
        Tag::IntArray(v) if v.len() == 3 => Delayed::Knot(BlockPos::new(v[0], v[1], v[2])),
        Tag::Compound(_) => Delayed::Holder(crate::persist::uuid_from_tag(tag.get("UUID")?)?),
        _ => return None,
    };
    Some(LeashData { holder: None, holder_key: None, delayed: Some(delayed), angular_momentum: 0.0 })
}
