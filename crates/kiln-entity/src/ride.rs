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
            crate::mob::MobKind::Horse | crate::mob::MobKind::Donkey | crate::mob::MobKind::Mule => 1.0,
            _ => 0.5,
        },
        _ => 1.0,
    }
}

/// `getVehicleAttachmentPoint` of a rider: players sit 0.6 lower, zombie-like riders 0.7
/// (`ridingOffset(-0.7)`), scaled with babies.
pub fn vehicle_attachment(type_name: &str, scale: f32) -> Vec3 {
    let y = match type_name {
        "minecraft:player" => 0.6,
        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" | "minecraft:zombie_villager" | "minecraft:zombified_piglin" => 0.7 * scale as f64,
        "minecraft:skeleton" | "minecraft:stray" | "minecraft:wither_skeleton" | "minecraft:bogged" => 0.7 * scale as f64,
        _ => 0.0,
    };
    Vec3::new(0.0, y, 0.0)
}

/// `getPassengerAttachmentPoint` of `vehicle` for its passenger at `index` (before rotating
/// by the vehicle's yaw where the type says so).
pub fn passenger_attachment(vehicle: &Entity, index: usize) -> Vec3 {
    let _ = index;
    if let EntityKind::Mob(m) = &vehicle.kind {
        if let Some(v) = m.kind.ext().and_then(|k| k.passenger_offset(vehicle, m)) {
            return v;
        }
        let s = age_scale(vehicle) as f64;
        if m.kind == crate::mob::MobKind::Chicken {
            return y_rot(Vec3::new(0.0, 0.7 * s, -0.1 * s), -vehicle.y_rot * 0.017453292);
        }
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
/// the vehicle. `vehicle` is the vehicle as it is after its own tick.
pub fn ride_tick(e: &mut Entity, level: &mut dyn EntityLevel, vehicle: &Entity) {
    e.delta = Vec3::ZERO;
    e.tick(level);
    if e.vehicle != Some(vehicle.id) {
        return;
    }
    position_rider(e, vehicle);
    // `LivingEntity.rideTick`: `resetFallDistance`.
    e.fall_distance = 0.0;
}

/// `positionRider(passenger)` for an entity passenger.
pub fn position_rider(e: &mut Entity, vehicle: &Entity) {
    let Some(index) = vehicle.passengers.iter().position(|&p| p == e.id) else { return };
    let p = rider_position(vehicle, index, e.type_name, age_scale(e));
    e.set_pos(p);
    // `AbstractHorse.positionRider`: the rider's body faces the horse's way.
    if let (EntityKind::Mob(vm), EntityKind::Mob(rm)) = (&vehicle.kind, &mut e.kind)
        && matches!(vm.kind, crate::mob::MobKind::Horse | crate::mob::MobKind::Donkey | crate::mob::MobKind::Mule)
    {
        rm.y_body_rot = vm.y_body_rot;
    }
}

/// `Entity.startRiding` + `addPassenger` for two entities of the level: `rider` rides `vehicle`
/// (players go first, as the controlling passenger). False when it cannot.
pub fn start_riding(rider: &mut Entity, vehicle: &mut Entity, rider_is_player: bool) -> bool {
    if rider.vehicle == Some(vehicle.id) || rider.id == vehicle.id || vehicle.vehicle == Some(rider.id) {
        return false;
    }
    rider.vehicle = Some(vehicle.id);
    add_passenger(vehicle, rider.id, rider_is_player, false);
    true
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
        && matches!(m.kind, crate::mob::MobKind::Horse | crate::mob::MobKind::Donkey | crate::mob::MobKind::Mule)
    {
        return horse_dismount(level, vehicle, rider_width, rider_height);
    }
    Vec3::new(vehicle.x(), vehicle.bounding_box().max_y, vehicle.z())
}
