//! Riding: where passengers sit on their vehicle (`Entity.positionRider`, the entity types'
//! passenger and vehicle attachment points) and where they get off (`dismountVehicle`).
//!
//! An entity's vehicle and passengers are ids ([`Entity::vehicle`], [`Entity::passengers`];
//! players by their network id). The level ticks a vehicle, then its passengers
//! (`ServerLevel.tickPassenger`: [`ride_tick`] for mobs, the simulation positions players).

use crate::entity::{Entity, EntityKind};
use crate::level::EntityLevel;
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::mth;

/// `Vec3.yRot(angle)`.
pub fn y_rot(v: Vec3, angle: f32) -> Vec3 {
    let c = mth::cos(angle as f64) as f64;
    let s = mth::sin(angle as f64) as f64;
    Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}

/// The age scale of a mob (`getAgeScale`: babies 0.5, or the type's own).
fn age_scale(e: &Entity) -> f32 {
    match crate::mob::data(e) {
        Some(m) if m.baby() => match m.kind {
            k if crate::mob::kinds::horse::is_equine(k) => 1.0,
            _ => 0.5,
        },
        _ => 1.0,
    }
}

/// `getVehicleAttachmentPoint` of a rider (the `VEHICLE` attachment of its type, from the
/// entity type's dimensions): players sit 0.6 lower, zombie-like and skeleton-like riders 0.7
/// (`ridingOffset(-0.7)`), illagers 0.6; babies of the zombie family have attachments of their
/// own (`BABY_DIMENSIONS`: 0.1875, the zombie villager's 0.125). The others scale with the age.
pub fn vehicle_attachment(type_name: &str, scale: f32) -> Vec3 {
    vehicle_attachment_of_age(type_name, false, scale)
}

/// [`vehicle_attachment`] for a rider that may be a baby.
pub fn vehicle_attachment_of_age(type_name: &str, baby: bool, scale: f32) -> Vec3 {
    let scaled = |y: f64| y * scale as f64;
    let y = match type_name {
        "minecraft:player" | "minecraft:mannequin" => 0.6,
        // (`ridingOffset(-0.7F)`: a float attachment, scaled in double.)
        "minecraft:zombie_villager" if baby => 0.125,
        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" | "minecraft:zombified_piglin" | "minecraft:piglin" if baby => 0.1875,
        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" | "minecraft:zombie_villager" | "minecraft:zombified_piglin" | "minecraft:piglin" | "minecraft:piglin_brute" => scaled(0.699_999_988_079_071),
        "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged" | "minecraft:parched" => scaled(0.699_999_988_079_071),
        "minecraft:wither_skeleton" => scaled(0.875),
        "minecraft:pillager" | "minecraft:vindicator" | "minecraft:evoker" | "minecraft:illusioner" => scaled(0.600_000_023_841_857_9),
        "minecraft:allay" | "minecraft:vex" => scaled(-0.039_999_999_105_930_33),
        "minecraft:phantom" => scaled(0.125),
        _ => 0.0,
    };
    Vec3::new(0.0, y, 0.0)
}

/// The `PASSENGER` attachment of the types whose seat is not the top of their box (from the
/// entity types' dimensions): its height and its offset along the body (rotated with the yaw).
/// The types with a code override are asked first.
fn passenger_point(type_name: &str) -> Option<(f64, f64)> {
    Some(match type_name {
        "minecraft:chicken" => (0.7, -0.1),
        // (`passengerAttachments(0.86875F)`.)
        "minecraft:pig" => (0.868_749_976_158_142_1, 0.0),
        "minecraft:ravager" => (2.2625, -0.0625),
        "minecraft:fox" => (0.6375, -0.25),
        "minecraft:frog" => (0.375, -0.25),
        "minecraft:llama" | "minecraft:trader_llama" => (1.37, -0.3),
        "minecraft:turtle" => (0.55625, -0.25),
        "minecraft:wolf" => (0.81875, -0.0625),
        "minecraft:cat" => (0.512_499_988_079_071, 0.0),
        "minecraft:cow" | "minecraft:mooshroom" => (1.368_749_976_158_142, 0.0),
        "minecraft:donkey" | "minecraft:goat" => (1.112_499_952_316_284_2, 0.0),
        "minecraft:mule" => (1.212_499_976_158_142, 0.0),
        "minecraft:elder_guardian" => (2.350_625_038_146_972_7, 0.0),
        "minecraft:guardian" => (0.975_000_023_841_857_9, 0.0),
        "minecraft:enderman" => (2.806_250_095_367_431_6, 0.0),
        "minecraft:endermite" | "minecraft:silverfish" => (0.237_499_997_019_767_76, 0.0),
        "minecraft:ender_dragon" => (3.0, 0.0),
        "minecraft:evoker" | "minecraft:illusioner" | "minecraft:pillager" | "minecraft:vindicator" | "minecraft:zombified_piglin" => (2.0, 0.0),
        "minecraft:ghast" => (4.0625, 0.0),
        "minecraft:hoglin" | "minecraft:zoglin" => (1.493_749_976_158_142, 0.0),
        "minecraft:husk" => (2.075_000_047_683_716, 0.0),
        "minecraft:ocelot" => (0.637_499_988_079_071, 0.0),
        "minecraft:parrot" => (0.462_500_005_960_464_5, 0.0),
        "minecraft:phantom" => (0.337_500_005_960_464_5, 0.0),
        "minecraft:pig" => (0.868_749_976_158_142_1, 0.0),
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:zombie" | "minecraft:drowned" => (2.012_500_047_683_716, 0.0),
        "minecraft:sheep" => (1.237_499_952_316_284_2, 0.0),
        "minecraft:sniffer" => (2.093_75, 0.0),
        "minecraft:spider" => (0.764_999_985_694_885_3, 0.0),
        "minecraft:vex" => (0.737_500_011_920_929, 0.0),
        "minecraft:warden" => (3.150_000_095_367_431_6, 0.0),
        "minecraft:witch" => (2.262_500_047_683_716, 0.0),
        "minecraft:zombie_villager" => (2.125, 0.0),
        "minecraft:skeleton_horse" | "minecraft:zombie_horse" => (1.318_750_023_841_858, 0.0),
        _ => return None,
    })
}

/// `getPassengerAttachmentPoint` of `vehicle` for its passenger at `index` (before rotating
/// by the vehicle's yaw where the type says so).
pub fn passenger_attachment(vehicle: &Entity, index: usize) -> Vec3 {
    if let EntityKind::Ext(x) = &vehicle.kind
        && let Some(v) = x.passenger_offset(vehicle, index, false)
    {
        return v;
    }
    if let EntityKind::Mob(m) = &vehicle.kind {
        if let Some(v) = m.kind.ext().and_then(|k| k.passenger_offset_at(vehicle, m, index)) {
            return v;
        }
        let s = age_scale(vehicle) as f64;
        if let Some((y, z)) = passenger_point(vehicle.type_name) {
            return y_rot(Vec3::new(0.0, y * s, z * s), -vehicle.y_rot * 0.017453292);
        }
    }
    Vec3::new(0.0, vehicle.height as f64, 0.0)
}

/// `EntityAttachments.getAverage(PASSENGER)` of `vehicle`: where its passenger point is, before it turns with the vehicle.
pub fn passenger_attachment_unrotated(vehicle: &Entity, m: &crate::mob::MobData) -> Vec3 {
    if let Some(v) = m.kind.ext().and_then(|k| k.passenger_offset_at(vehicle, m, 0)) {
        return v;
    }
    let s = if m.baby() && !crate::mob::kinds::horse::is_equine(m.kind) { 0.5 } else { 1.0 };
    if let Some((y, z)) = passenger_point(vehicle.type_name) {
        return Vec3::new(0.0, y * s, z * s);
    }
    Vec3::new(0.0, vehicle.height as f64, 0.0)
}

/// `getPassengerRidingPosition`.
pub fn riding_position(vehicle: &Entity, index: usize) -> Vec3 {
    vehicle.position() + passenger_attachment(vehicle, index)
}

/// `positionRider`: where rider `rider_type` (scaled by `rider_scale`) at `index` stands.
pub fn rider_position(vehicle: &Entity, index: usize, rider_type: &str, rider_scale: f32) -> Vec3 {
    riding_position(vehicle, index) - vehicle_attachment(rider_type, rider_scale)
}

/// `Entity.rideTick` for a mob passenger: no motion of its own, its tick, then its place on
/// the vehicle. `vehicle` is the vehicle as it is after its own tick. A rider that is the
/// controlling passenger of a mob vehicle ticks with the vehicle's navigation and move control
/// in hand (`Mob.getNavigation`, `getMoveControl`): they are lent to it, and what its goals did
/// with them (and the vehicle's turning, `Mob.lookAt`) is in `vehicle` afterwards. True when the
/// rider steered: a caller that ticked a copy of the vehicle takes the changes back with
/// [`copy_steering_back`].
pub fn ride_tick(e: &mut Entity, level: &mut dyn EntityLevel, vehicle: &mut Entity) -> bool {
    e.delta = Vec3::ZERO;
    let steering = lend_mount(e, vehicle);
    e.tick(level);
    if steering {
        return_mount(e, vehicle);
    }
    if e.vehicle != Some(vehicle.id) {
        return steering;
    }
    position_rider(e, vehicle);
    // `LivingEntity.rideTick`: `resetFallDistance`.
    e.fall_distance = 0.0;
    // `AbstractSkeleton.rideTick`, `Drowned.rideTick`: the body turns with the vehicle's.
    if steering
        && matches!(e.type_name, "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged" | "minecraft:parched" | "minecraft:wither_skeleton" | "minecraft:drowned")
        && let (EntityKind::Mob(rm), EntityKind::Mob(vm)) = (&mut e.kind, &vehicle.kind)
    {
        rm.y_body_rot = vm.y_body_rot;
    }
    steering
}

/// `Entity.getControlledVehicle` is a mob: `vehicle` has no AI switched off, this rider is a mob
/// that may control a vehicle and sits first. Lends the vehicle's mob data to the rider.
fn lend_mount(e: &mut Entity, vehicle: &mut Entity) -> bool {
    let EntityKind::Mob(vm) = &vehicle.kind else { return false };
    if vm.no_ai || vehicle.passengers.first() != Some(&e.id) || e.vehicle != Some(vehicle.id) {
        return false;
    }
    if !matches!(e.kind, EntityKind::Mob(_)) || crate::mob::entity_type_tag(e.type_name, "minecraft:non_controlling_rider") {
        return false;
    }
    let m = crate::mob::take(vehicle);
    let copy = vehicle.clone();
    match &mut e.kind {
        EntityKind::Mob(rm) => rm.mount = Some(Box::new(crate::mob::Mount { e: copy, m })),
        _ => unreachable!("checked above"),
    }
    true
}

/// Takes the vehicle's data back from the rider that ticked, with what `lookAt` turned.
fn return_mount(e: &mut Entity, vehicle: &mut Entity) {
    let Some(rm) = crate::mob::data_mut(e) else { return };
    let Some(c) = rm.mount.take() else { return };
    let c = *c;
    vehicle.y_rot = c.e.y_rot;
    vehicle.x_rot = c.e.x_rot;
    crate::mob::put(vehicle, c.m);
}

/// What a steering rider changed of a vehicle it ticked as a copy: the navigation, the move
/// control and the turning of `steered` go to the real `vehicle`.
pub fn copy_steering_back(steered: &Entity, vehicle: &mut Entity) {
    vehicle.y_rot = steered.y_rot;
    vehicle.x_rot = steered.x_rot;
    if let (EntityKind::Mob(from), EntityKind::Mob(to)) = (&steered.kind, &mut vehicle.kind) {
        to.nav = from.nav.clone();
        to.mov = from.mov.clone();
    }
}

/// The `AbstractHorse` subclasses: horses and their kin, llamas and camels.
fn is_abstract_horse(kind: crate::mob::MobKind) -> bool {
    crate::mob::kinds::horse::is_equine(kind) || matches!(kind, crate::mob::MobKind::Camel | crate::mob::MobKind::CamelHusk)
}

/// `positionRider(passenger)` for an entity passenger.
pub fn position_rider(e: &mut Entity, vehicle: &Entity) {
    let Some(index) = vehicle.passengers.iter().position(|&p| p == e.id) else { return };
    let baby = crate::mob::data(e).is_some_and(|m| m.baby());
    let p = riding_position(vehicle, index) - vehicle_attachment_of_age(e.type_name, baby, age_scale(e));
    e.set_pos(p);
    if let Some(boat) = crate::ext_entity::get::<crate::ext_entity::boat::Boat>(vehicle) {
        boat_clamp_rotation(e, vehicle, boat);
    }
    // `AbstractHorse.positionRider`, `Chicken.positionRider`: the rider's body faces the
    // mount's way.
    if let (EntityKind::Mob(vm), EntityKind::Mob(rm)) = (&vehicle.kind, &mut e.kind)
        && (is_abstract_horse(vm.kind) || vm.kind == crate::mob::MobKind::Chicken)
    {
        rm.y_body_rot = vm.y_body_rot;
    }
}

/// `AbstractBoat.positionRider` after the position, for a mob rider (breezes turn on their own,
/// `#can_turn_in_boats`): the boat's turn since the last tick is added to the rider's yaw and head,
/// then `clampRotation`: the body takes the boat's yaw (an animal in a full boat sits sideways),
/// the mob faces where its body does and its head stays within its limit of it.
fn boat_clamp_rotation(e: &mut Entity, vehicle: &Entity, boat: &crate::ext_entity::boat::Boat) {
    if crate::mob::entity_type_tag(e.type_name, "minecraft:can_turn_in_boats") {
        return;
    }
    let id = e.id;
    let EntityKind::Mob(rm) = &mut e.kind else { return };
    let dr = boat.delta_rotation();
    e.y_rot += dr;
    rm.y_head_rot += dr;
    let body = if rm.kind.is_animal() && vehicle.passengers.len() == boat.max_passengers() {
        vehicle.y_rot + if id % 2 == 0 { 90.0 } else { 270.0 }
    } else {
        vehicle.y_rot
    };
    rm.y_body_rot = body;
    e.y_rot = body;
    // `Mob.clampHeadRotationToBody`.
    let max = rm.kind.max_head_y_rot() as f32;
    let head = rm.y_head_rot;
    let d = crate::mob::mth::wrap_degrees(body - head);
    let clamped = d.clamp(-max, max);
    rm.y_head_rot = head + d - clamped;
}

/// `Entity.startRiding` + `addPassenger` for two entities of the level: `rider` rides `vehicle`
/// (players go first, as the controlling passenger). False when it cannot.
pub fn start_riding(rider: &mut Entity, vehicle: &mut Entity, rider_is_player: bool) -> bool {
    if rider.vehicle == Some(vehicle.id) || rider.id == vehicle.id || vehicle.vehicle == Some(rider.id) {
        return false;
    }
    rider.vehicle = Some(vehicle.id);
    add_passenger(vehicle, rider.id, rider_is_player, false);
    if matches!(&vehicle.kind, EntityKind::Mob(vm) if is_abstract_horse(vm.kind)) {
        snap_rotation_to_mount(rider, vehicle);
    }
    true
}

/// `AbstractHorse.addPassenger`: the new rider snaps to the mount's view rotation
/// (`absSnapRotationTo(getViewYRot(0), getViewXRot(0))`: the mount's rotation of the last tick).
pub fn snap_rotation_to_mount(rider: &mut Entity, mount: &Entity) {
    rider.y_rot = mount.y_rot_o % 360.0;
    rider.x_rot = mount.x_rot_o.clamp(-90.0, 90.0) % 360.0;
    rider.y_rot_o = rider.y_rot;
    rider.x_rot_o = rider.x_rot;
}

/// `addPassenger`: a player jumps to the front unless a player is already first.
pub fn add_passenger(vehicle: &mut Entity, id: i32, is_player: bool, first_is_player: bool) {
    if vehicle.passengers.contains(&id) {
        return;
    }
    if is_player && !vehicle.passengers.is_empty() && !first_is_player {
        vehicle.passengers.insert(0, id);
    } else {
        vehicle.passengers.push(id);
    }
}

/// `Entity.ejectPassengers`: every passenger gets off (players are seen to it by the
/// simulation, which finds them no longer seated).
pub fn eject(vehicle: &mut Entity, level: &mut dyn EntityLevel) {
    for id in std::mem::take(&mut vehicle.passengers) {
        if let Some(rider) = level.entity_mut(id) {
            rider.vehicle = None;
        }
    }
}

/// `removePassenger`.
pub fn remove_passenger(vehicle: &mut Entity, id: i32) {
    vehicle.passengers.retain(|&p| p != id);
}

/// `getCollisionHorizontalEscapeVector(vehicleWidth, passengerWidth, yaw)`.
fn escape_vector(vehicle_width: f64, passenger_width: f64, yaw: f32) -> Vec3 {
    let d = (vehicle_width + passenger_width + 9.999999747378752e-6) / 2.0;
    let f = -mth::sin((yaw * 0.017453292) as f64);
    let g = mth::cos((yaw * 0.017453292) as f64);
    let h = f.abs().max(g.abs());
    Vec3::new(f as f64 * d / h as f64, 0.0, g as f64 * d / h as f64)
}

/// Whether a box of the rider fits at `pos` (`DismountHelper.canDismountTo`, without the
/// entity check).
fn fits(level: &dyn EntityLevel, pos: Vec3, width: f64, height: f64) -> bool {
    let w = width / 2.0;
    let b = Aabb::new(pos.x - w, pos.y, pos.z - w, pos.x + w, pos.y + height, pos.z + w);
    crate::collision::no_collision(level, &crate::collision::CollisionContext::EMPTY, i32::MIN, &b)
}

/// `AbstractHorse.getDismountLocationForPassenger` (a right-handed rider): to the right of the
/// horse, then the left, else where the horse is.
pub fn horse_dismount(level: &dyn EntityLevel, vehicle: &Entity, rider_width: f64, rider_height: f64) -> Vec3 {
    for side in [90.0f32, -90.0] {
        let d = escape_vector(vehicle.width as f64, rider_width, vehicle.y_rot + side);
        let (x, z) = (vehicle.x() + d.x, vehicle.z() + d.z);
        let y0 = vehicle.bounding_box().min_y;
        let max_y = vehicle.bounding_box().max_y + 0.75;
        let mut p = BlockPos::containing(x, y0, z);
        loop {
            let floor = crate::mob::path::floor_level(level, p.above()) - p.y as f64;
            if p.y as f64 + floor > max_y {
                break;
            }
            // `DismountHelper.isBlockFloorValid`: a floor that is not too high.
            if floor.is_finite() && floor < 1.0 {
                let target = Vec3::new(x, p.y as f64 + floor, z);
                if fits(level, target, rider_width, rider_height) {
                    return target;
                }
            }
            p = p.above();
            if p.y as f64 >= max_y {
                break;
            }
        }
    }
    vehicle.position()
}

/// `Entity.getDismountLocationForPassenger` (on top of the vehicle's box), or the type's own.
pub fn dismount_location(level: &dyn EntityLevel, vehicle: &Entity, rider_width: f64, rider_height: f64) -> Vec3 {
    if let EntityKind::Mob(m) = &vehicle.kind
        && crate::mob::kinds::horse::is_equine(m.kind)
    {
        return horse_dismount(level, vehicle, rider_width, rider_height);
    }
    Vec3::new(vehicle.x(), vehicle.bounding_box().max_y, vehicle.z())
}
