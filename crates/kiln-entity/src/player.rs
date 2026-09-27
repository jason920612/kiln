//! Server-side player movement: vanilla's movement check replays the client's motion with
//! `player.move(MoverType.PLAYER, delta)` and flags the player if it ends up elsewhere.

use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::EntityLevel;
use crate::math::{Aabb, Vec3};

#[derive(Clone, Debug)]
pub struct PlayerData {
    /// `abilities.flying`.
    pub flying: bool,
    /// Leather boots or similar (`PowderSnowBlock.canEntityWalkOnPowderSnow`).
    pub walks_on_powder_snow: bool,
    /// Spectators move without collision (`noPhysics`).
    pub spectator: bool,
}

/// A player entity for movement checks: `height` is the current pose's (1.8 standing, 1.5
/// crouching, 0.6 swimming or gliding), `step_height` the `step_height` attribute (0.6).
pub fn new(id: i32, uuid: u128, pos: Vec3, height: f32, step_height: f32) -> Entity {
    let mut e = Entity::new(
        "minecraft:player",
        id,
        uuid,
        EntityKind::Player(PlayerData { flying: false, walks_on_powder_snow: false, spectator: false }),
        0,
    );
    e.height = height;
    e.max_up_step = step_height;
    e.set_pos(pos);
    e
}

/// `ServerPlayer.move(MoverType.PLAYER, delta)` as the movement check calls it: collision,
/// step-up, sneaking edge back-off and ground state; returns the position reached.
///
/// Set `player.on_ground`, `fall_distance`, `shift_key_down` and the pose box beforehand from
/// the server's view of the player.
pub fn server_move(level: &mut dyn EntityLevel, player: &mut Entity, delta: Vec3) -> Vec3 {
    if let EntityKind::Player(d) = &player.kind {
        player.no_physics = d.spectator;
    }
    player.do_move(level, MoverType::Player, delta);
    player.position()
}

/// `Player.maybeBackOffFromEdge`: a sneaking player on the ground does not walk off edges
/// higher than its step height.
pub(crate) fn back_off_from_edge(e: &Entity, level: &dyn EntityLevel, movement: Vec3, mover: MoverType) -> Vec3 {
    let EntityKind::Player(d) = &e.kind else { return movement };
    let step = e.max_up_step;
    if d.flying
        || movement.y > 0.0
        || !(mover == MoverType::SelfMove || mover == MoverType::Player)
        || !e.shift_key_down
        || !is_above_ground(e, level, step)
    {
        return movement;
    }
    let (mut x, mut z) = (movement.x, movement.z);
    let sx = jsignum(x) * 0.05;
    let sz = jsignum(z) * 0.05;
    while x != 0.0 && can_fall_at_least(e, level, x, 0.0, step as f64) {
        if x.abs() <= 0.05 {
            x = 0.0;
            break;
        }
        x -= sx;
    }
    while z != 0.0 && can_fall_at_least(e, level, 0.0, z, step as f64) {
        if z.abs() <= 0.05 {
            z = 0.0;
            break;
        }
        z -= sz;
    }
    while x != 0.0 && z != 0.0 && can_fall_at_least(e, level, x, z, step as f64) {
        if x.abs() <= 0.05 {
            x = 0.0;
        } else {
            x -= sx;
        }
        if z.abs() <= 0.05 {
            z = 0.0;
        } else {
            z -= sz;
        }
    }
    Vec3::new(x, movement.y, z)
}

fn jsignum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() { v } else { 1.0f64.copysign(v) }
}

/// `Player.isAboveGround`.
fn is_above_ground(e: &Entity, level: &dyn EntityLevel, step: f32) -> bool {
    e.on_ground || (e.fall_distance < step as f64 && !can_fall_at_least(e, level, 0.0, 0.0, step as f64 - e.fall_distance))
}

/// `Player.canFallAtLeast`: no collision in the box shifted by (x, z) and extended `down`.
fn can_fall_at_least(e: &Entity, level: &dyn EntityLevel, x: f64, z: f64, down: f64) -> bool {
    let bb = e.bounding_box();
    let probe = Aabb::new(
        bb.min_x + 1.0e-7 + x,
        bb.min_y - down - 1.0e-7,
        bb.min_z + 1.0e-7 + z,
        bb.max_x - 1.0e-7 + x,
        bb.min_y,
        bb.max_z - 1.0e-7 + z,
    );
    crate::collision::no_collision(level, &e.collision_context(), e.id, &probe)
}
