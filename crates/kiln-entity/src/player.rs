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

// ---------------------------------------------------------------------- the server's own travel

/// What `LivingEntity.travel` of a player reads besides the entity itself.
#[derive(Clone, Copy, Debug)]
pub struct TravelInput {
    /// The `gravity` attribute.
    pub gravity: f64,
    pub slow_falling: bool,
    /// The levitation amplifier.
    pub levitation: Option<i32>,
    pub dolphins_grace: bool,
    /// The `friction_modifier`, `air_drag_modifier` and `water_movement_efficiency` attributes.
    pub friction_modifier: f32,
    pub air_drag_modifier: f32,
    pub water_efficiency: f32,
    pub sprinting: bool,
}

impl Default for TravelInput {
    fn default() -> Self {
        TravelInput { gravity: 0.08, slow_falling: false, levitation: None, dolphins_grace: false, friction_modifier: 1.0, air_drag_modifier: 1.0, water_efficiency: 0.0, sprinting: false }
    }
}

fn modified_friction(f: f32, modifier: f32) -> f32 {
    crate::mob::mth::clamp(1.0 - (1.0 - f) * modifier, 0.0, 1.0)
}

fn on_climbable(e: &Entity, level: &dyn EntityLevel) -> bool {
    !matches!(&e.kind, EntityKind::Player(d) if d.spectator) && crate::blocks::has_tag(level.block(e.block_position()), crate::blocks::Tag::Climbable)
}

/// `LivingEntity.getEffectiveGravity`: slow falling caps it at 0.01 while falling.
fn effective_gravity(e: &Entity, t: &TravelInput) -> f64 {
    if e.delta.y <= 0.0 && t.slow_falling { t.gravity.min(0.01) } else { t.gravity }
}

/// `LivingEntity.handleOnClimbable`.
fn handle_on_climbable(e: &mut Entity, level: &dyn EntityLevel, v: Vec3) -> Vec3 {
    if !on_climbable(e, level) {
        return v;
    }
    e.fall_distance = 0.0;
    let x = crate::mob::mth::clamp_d(v.x, -0.15000000596046448, 0.15000000596046448);
    let z = crate::mob::mth::clamp_d(v.z, -0.15000000596046448, 0.15000000596046448);
    let mut y = v.y.max(-0.15000000596046448);
    // A sneaking player holds on to a ladder (not to scaffolding).
    if y < 0.0 && crate::blocks::kind(level.block(e.block_position())) != crate::blocks::Kind::Scaffolding && e.shift_key_down {
        y = 0.0;
    }
    Vec3::new(x, y, z)
}

/// `LivingEntity.travel(Vec3.ZERO)` for the server's copy of a player: the physics the server
/// runs for a player no input reaches (gravity and drag, ladders, fluids, bounces), which
/// the client's move packets overrule only by position.
pub fn travel(level: &mut dyn EntityLevel, e: &mut Entity, t: &TravelInput) {
    if e.is_in_water() || e.is_in_lava() {
        travel_in_fluid(level, e, t);
    } else {
        travel_in_air(level, e, t);
    }
}

fn travel_in_air(level: &mut dyn EntityLevel, e: &mut Entity, t: &TravelInput) {
    let below = e.block_pos_below_that_affects_movement(level);
    let friction = if e.on_ground { modified_friction(crate::physics::block_factors(level.block(below)).friction, t.friction_modifier) } else { 1.0 };
    // `handleRelativeFrictionAndCalculateMovement`: no input to add (it still turns -0.0 into +0.0).
    e.delta = e.delta + Vec3::ZERO;
    e.delta = handle_on_climbable(e, level, e.delta);
    let d = e.delta;
    e.do_move(level, MoverType::SelfMove, d);
    let mut v = e.delta;
    if e.horizontal_collision && on_climbable(e, level) {
        v = Vec3::new(v.x, 0.2, v.z);
    }
    let mut y = v.y;
    if let Some(a) = t.levitation {
        y += (0.05 * (a + 1) as f64 - v.y) * 0.2;
    } else if level.is_loaded(below) {
        y -= effective_gravity(e, t);
    } else if e.y() > level.min_y() as f64 {
        y = -0.1;
    } else {
        y = 0.0;
    }
    let h = friction * modified_friction(0.91, t.air_drag_modifier);
    let vy = modified_friction(0.98, t.air_drag_modifier);
    e.delta = Vec3::new(v.x * h as f64, y * vy as f64, v.z * h as f64);
}

fn travel_in_fluid(level: &mut dyn EntityLevel, e: &mut Entity, t: &TravelInput) {
    let falling = e.delta.y <= 0.0;
    let y0 = e.y();
    let g = effective_gravity(e, t);
    if e.is_in_water() {
        let mut slow = 0.8f32;
        let mut eff = t.water_efficiency;
        if !e.on_ground {
            eff *= 0.5;
        }
        if eff > 0.0 {
            slow += (0.54600006 - slow) * eff;
        }
        if t.dolphins_grace {
            slow = 0.96;
        }
        e.delta = e.delta + Vec3::ZERO;
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        let mut v = e.delta;
        if e.horizontal_collision && on_climbable(e, level) {
            v = Vec3::new(v.x, 0.2, v.z);
        }
        v = v.multiply(slow as f64, 0.800000011920929, slow as f64);
        e.delta = fluid_falling_adjusted(g, falling, t.sprinting, v);
    } else {
        e.delta = e.delta + Vec3::ZERO;
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        let threshold = if (e.eye_height as f64) < 0.4 { 0.0 } else { 0.4 };
        if e.fluid_height_lava() <= threshold {
            e.delta = e.delta.multiply(0.5, 0.800000011920929, 0.5);
            e.delta = fluid_falling_adjusted(g, falling, t.sprinting, e.delta);
        } else {
            e.delta = e.delta.scale(0.5);
        }
        if g != 0.0 {
            e.delta = e.delta.add(0.0, -g / 4.0, 0.0);
        }
    }
    // `jumpOutOfFluid`.
    let v = e.delta;
    if e.horizontal_collision {
        let b = e.bounding_box().offset(v.x, v.y + 0.6000000238418579 - e.y() + y0, v.z);
        let ctx = e.collision_context();
        if crate::collision::no_collision(level, &ctx, e.id, &b) && !contains_any_liquid(level, &b) {
            e.delta = Vec3::new(v.x, 0.30000001192092896, v.z);
        }
    }
}

fn contains_any_liquid(level: &dyn EntityLevel, b: &Aabb) -> bool {
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::ceil(b.max_x), crate::math::ceil(b.max_y), crate::math::ceil(b.max_z));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                if !crate::physics::fluid_state(level.block(crate::math::BlockPos::new(x, y, z))).is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

/// `getFluidFallingAdjustedMovement`.
fn fluid_falling_adjusted(g: f64, falling: bool, sprinting: bool, v: Vec3) -> Vec3 {
    if g == 0.0 || sprinting {
        return v;
    }
    let y = if falling && (v.y - 0.005).abs() >= 0.003 && (v.y - g / 16.0).abs() < 0.003 { -0.003 } else { v.y - g / 16.0 };
    Vec3::new(v.x, y, v.z)
}
