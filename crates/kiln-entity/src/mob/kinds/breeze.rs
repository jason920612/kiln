//! Breeze: a trial-chamber monster that slides around its target, leaps far on gusts of wind
//! and shoots wind charges ([`crate::ext_entity::wind_charge`]) whose wind burst knocks things
//! back. Other breezes cannot hurt it; it only fights players and iron golems.
//!
//! Driven by the brain of `BreezeAi` on [`crate::mob::brain`]: core (swim, look), idle (pick a
//! target: the nearest attackable or whoever hurt it; slide to the walk target; stand or
//! stroll) and fight (drop an invalid target, `Shoot`, `LongJump`, `ShootWhenStuck`, `Slide`)
//! while it has a target and no walk target. The state of the fight is the memories
//! `BREEZE_SHOOT`, `BREEZE_SHOOT_CHARGING`, `_RECOVERING`, `_COOLDOWN`, `BREEZE_JUMP_COOLDOWN`,
//! `BREEZE_JUMP_INHALING`, `BREEZE_JUMP_TARGET` and `BREEZE_LEAVING_WATER` with their expiry;
//! the pose (`SHOOTING`, `INHALING`, `LONG_JUMPING`, `SLIDING`) stays in [`State`].

use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::brain::behaviors::{DoNothing, LookAtTargetSink, MoveToTargetSink, StrollKind, direction_from_rotation, stroll};
use crate::mob::brain::combat;
use crate::mob::brain::sensors::{HurtBy, NearestLivingEntities, Players};
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Control, Cx, Gate, Mem, Sensor, ShotBehavior, Status, Timed, Val, WalkTarget};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::{self, Living};
use crate::mob::{self, DamageSource, MobData, mth, path, random_pos};
use crate::persist::{Input, Output};
use crate::sensor_boilerplate;
use kiln_data::entities::pose;
use kiln_javamath::random::RandomSource;
use kiln_javamath::trig;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::{Registered, ValueAbsent, ValuePresent};

pub struct Breeze;

pub static KIND: Breeze = Breeze;

static INFO: Info = Info {
    head: (30, 40, 25),
    monster_base: false,
    ..Info::monster("minecraft:breeze", &[(MovementSpeed, 0.6299999952316284), (MaxHealth, 30.0), (FollowRange, 24.0), (AttackDamage, 3.0)])
};

/// `Math.round` of 15, 4, 10 and the long jump's inhaling.
const SHOOT_INITIAL_DELAY: i32 = 15;
const SHOOT_RECOVER_DELAY: i32 = 4;
const SHOOT_COOLDOWN: i32 = 10;
const INHALING_DURATION: i32 = 10;

#[derive(Clone, Debug, Default)]
pub struct State {
    /// `getPose` (a `kiln_data::entities::pose` id).
    pub pose: i32,
    sound_tick: i32,
    /// `Entity.setDiscardFriction`: set for a long jump's flight.
    discard_friction: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("breeze state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("breeze state")
}

fn pose_of(m: &MobData) -> i32 {
    st(m).pose
}

/// `Entity.setPose`.
fn set_pose(e: &mut Entity, m: &mut MobData, p: i32) {
    let s = st_mut(m);
    if s.pose != p {
        s.pose = p;
        e.needs_sync = true;
    }
}

/// `getFiringYPosition`.
fn firing_y(e: &Entity) -> f64 {
    e.y() + (e.height / 2.0) as f64 + 0.30000001192092896
}

fn play(e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, volume: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "hostile", volume, pitch: 1.0 });
    }
}

/// `LivingEntity.lookAt(EYES, pos)`: turns the body and head at once.
fn look_at_eyes(e: &mut Entity, m: &mut MobData, pos: Vec3) {
    let from = Vec3::new(e.x(), e.y() + e.eye_height as f64, e.z());
    let (dx, dy, dz) = (pos.x - from.x, pos.y - from.y, pos.z - from.z);
    let h = (dx * dx + dz * dz).sqrt();
    e.x_rot = mth::wrap_degrees((-(mth::atan2(dy, h) * 57.2957763671875)) as f32);
    e.y_rot = mth::wrap_degrees((mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0);
    m.y_head_rot = e.y_rot;
    e.x_rot_o = e.x_rot;
    e.y_rot_o = e.y_rot;
    // `LivingEntity.lookAt`: the body and the old values follow the head.
    m.y_head_rot_o = m.y_head_rot;
    m.y_body_rot = m.y_head_rot;
    m.y_body_rot_o = m.y_body_rot;
}

/// `Entity.distanceTo(entity)`: float arithmetic.
fn distance_to(e: &Entity, p: Vec3) -> f32 {
    let (dx, dy, dz) = ((e.x() - p.x) as f32, (e.y() - p.y) as f32, (e.z() - p.z) as f32);
    ((dx * dx + dy * dy + dz * dz) as f64).sqrt() as f32
}

/// The target's `yHeadRot`.
fn head_yaw(level: &dyn EntityLevel, id: i32) -> f32 {
    if let Some(p) = level.player(id) {
        return p.yaw;
    }
    level.entity(id).and_then(mob::data).map_or(0.0, |m| m.y_head_rot)
}

/// `BreezeUtil.randomPointBehindTarget`.
fn point_behind(e: &mut Entity, level: &dyn EntityLevel, t: &Living) -> Vec3 {
    let angle = head_yaw(level, t.id) + 180.0 + e.random.next_gaussian() as f32 * 90.0 / 2.0;
    let r = mth::lerp_f(e.random.next_float(), 4.0, 8.0);
    let dir = direction_from_rotation(0.0, angle).scale(r as f64);
    t.pos + dir
}

/// `Level.clip(ClipContext(from, to, COLLIDER, NONE))`: where a block is hit, if one is.
fn clip_hit(level: &dyn EntityLevel, from: Vec3, to: Vec3) -> Option<Vec3> {
    crate::clip::traverse_blocks(from, to, |p| {
        let (shape, _) = crate::collision::collision_shape(level.block(p), p, &crate::collision::CollisionContext::EMPTY);
        crate::clip::shape_clip(&shape, from, to, p).map(|(loc, _)| loc)
    })
}

/// `BreezeUtil.hasLineOfSight`: nothing solid between its feet and the point.
fn has_line_of_sight(e: &Entity, m: &MobData, level: &dyn EntityLevel, to: Vec3) -> bool {
    let from = Vec3::new(e.x(), e.y(), e.z());
    let range = 50.0f64.max(m.attrs.value(FollowRange));
    if to.distance_to_sqr(from).sqrt() > range {
        return false;
    }
    clip_hit(level, from, to).is_none()
}

/// `LongJump.snapToSurface`: the block above the ground below (or the ceiling above) the point.
fn snap_to_surface(level: &dyn EntityLevel, p: Vec3) -> Option<BlockPos> {
    let above = |loc: Vec3| BlockPos::containing(loc.x, loc.y, loc.z).above();
    if let Some(loc) = clip_hit(level, p, Vec3::new(p.x, p.y - 10.0, p.z)) {
        return Some(above(loc));
    }
    clip_hit(level, p, Vec3::new(p.x, p.y + 10.0, p.z)).map(above)
}

/// `EntityType.isBlockDangerous` for a breeze.
fn block_dangerous(state: u16) -> bool {
    matches!(
        crate::blocks::block_name(state),
        "minecraft:fire" | "minecraft:soul_fire" | "minecraft:lava" | "minecraft:magma_block" | "minecraft:lava_cauldron" | "minecraft:wither_rose" | "minecraft:sweet_berry_bush" | "minecraft:cactus" | "minecraft:powder_snow"
    )
}

/// `LongJumpUtil.calculateJumpVectorForAngle` without the collision check.
fn jump_vector(e: &Entity, m: &MobData, target: Vec3, max_v: f32, angle: i32) -> Option<Vec3> {
    let pos = e.position();
    let plane = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z).normalize().scale(0.5);
    let d = (target - plane) - pos;
    let a = angle as f32 * std::f32::consts::PI / 180.0;
    let xz = trig::atan2(d.z, d.x);
    let r2 = Vec3::new(d.x, 0.0, d.z).length_sqr();
    let r = r2.sqrt();
    let g = m.attrs.value(Attr::Gravity);
    let (sin2a, cosa, sina) = (trig::sin((2.0 * a) as f64), trig::cos(a as f64), trig::sin(a as f64));
    let v0sqr = r2 * g / (r * sin2a - 2.0 * d.y * (cosa * cosa));
    if v0sqr < 0.0 {
        return None;
    }
    let v0 = v0sqr.sqrt();
    if v0 > max_v as f64 {
        return None;
    }
    let (v0r, v0y) = (v0 * cosa, v0 * sina);
    Some(Vec3::new(v0r * trig::cos(xz), v0y, v0r * trig::sin(xz)).scale(0.949999988079071))
}

// ---------------------------------------------------------------------------- behaviours

/// `Swim(0.8)` with the breeze's own fluid jump threshold (its eye height).
#[derive(Clone, Debug)]
struct BreezeSwim;

impl BreezeSwim {
    fn in_fluid(cx: &Cx) -> bool {
        cx.e.fluid_height_water() > cx.e.eye_height as f64 || cx.e.is_in_lava()
    }
}

impl Behavior for BreezeSwim {
    fn name(&self) -> &'static str {
        "Swim"
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        BreezeSwim::in_fluid(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        BreezeSwim::in_fluid(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.e.random.next_float() < 0.8 {
            cx.m.jump.jump = true;
        }
    }
    behavior_boilerplate!();
}

/// `BreezeAi.SlideToTargetSink(20, 40)`: `MoveToTargetSink` that slides (pose, sound) and shoots
/// once it gets there.
#[derive(Clone, Debug)]
struct SlideToTargetSink(MoveToTargetSink);

impl Behavior for SlideToTargetSink {
    fn name(&self) -> &'static str {
        "SlideToTargetSink"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        self.0.entry()
    }
    fn duration(&self) -> (i32, i32) {
        self.0.duration()
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        self.0.check_extra_start(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        self.0.can_still_use(cx)
    }
    fn start(&mut self, cx: &mut Cx) {
        self.0.start(cx);
        play(cx.e, cx.level, "minecraft:entity.breeze.slide", 1.0);
        set_pose(cx.e, cx.m, pose::SLIDING);
    }
    fn tick(&mut self, cx: &mut Cx) {
        self.0.tick(cx);
    }
    fn stop(&mut self, cx: &mut Cx) {
        self.0.stop(cx);
        set_pose(cx.e, cx.m, pose::STANDING);
        if cx.b.mem.has(Mem::AttackTarget) {
            cx.b.mem.set_expiring(Mem::BreezeShoot, Val::Unit, 60);
        }
    }
    behavior_boilerplate!();
}

/// `StopAttackingIfTargetInvalid.create(!wasEntityAttackableLastNTicks(this, 100))`: the target
/// goes once it could not be attacked for over 100 checks (`rememberPositives`).
#[derive(Clone, Debug)]
struct StopAttacking {
    remember: i32,
}

impl ShotBehavior for StopAttacking {
    fn name(&self) -> &'static str {
        "StopAttackingIfTargetInvalid"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::AttackTarget, ValuePresent), (Mem::CantReachWalkTargetSince, Registered)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
        let t = util::living(cx, id);
        let invalid = match &t {
            None => true,
            Some(t) => {
                !goals::can_attack(cx.m, &*cx.level, t)
                    // `isTiredOfTryingToReachTarget`.
                    || cx.b.mem.long(Mem::CantReachWalkTargetSince).is_some_and(|since| cx.time - since > 200)
                    || !t.alive
                    || {
                        // `!rememberPositives(100, isEntityAttackable)`.
                        if util::is_entity_attackable(cx, t) {
                            self.remember = 100;
                            false
                        } else {
                            self.remember -= 1;
                            self.remember < 0
                        }
                    }
            }
        };
        if invalid {
            cx.b.mem.erase(Mem::AttackTarget);
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `Shoot`: inhales for 15 ticks, then a wind charge, recovering for 4 (20 in all).
#[derive(Clone, Debug)]
struct Shoot;

impl Behavior for Shoot {
    fn name(&self) -> &'static str {
        "Shoot"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::AttackTarget, ValuePresent),
            (Mem::BreezeShootCooldown, ValueAbsent),
            (Mem::BreezeShootCharging, ValueAbsent),
            (Mem::BreezeShootRecovering, ValueAbsent),
            (Mem::BreezeShoot, ValuePresent),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::BreezeJumpTarget, ValueAbsent),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        let d = SHOOT_INITIAL_DELAY + 1 + SHOOT_RECOVER_DELAY;
        (d, d)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if pose_of(cx.m) != pose::STANDING {
            return false;
        }
        let Some(t) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return false };
        let within = cx.e.position().distance_to_sqr(t.pos) < 256.0;
        if !within {
            cx.b.mem.erase(Mem::BreezeShoot);
        }
        within
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::AttackTarget) && cx.b.mem.has(Mem::BreezeShoot)
    }
    fn start(&mut self, cx: &mut Cx) {
        if cx.b.mem.has(Mem::AttackTarget) {
            set_pose(cx.e, cx.m, pose::SHOOTING);
        }
        cx.b.mem.set_expiring(Mem::BreezeShootCharging, Val::Unit, SHOOT_INITIAL_DELAY as i64);
        play(cx.e, cx.level, "minecraft:entity.breeze.inhale", 1.0);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if pose_of(cx.m) == pose::SHOOTING {
            set_pose(cx.e, cx.m, pose::STANDING);
        }
        cx.b.mem.set_expiring(Mem::BreezeShootCooldown, Val::Unit, SHOOT_COOLDOWN as i64);
        cx.b.mem.erase(Mem::BreezeShoot);
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return };
        look_at_eyes(cx.e, cx.m, t.pos);
        if cx.b.mem.has(Mem::BreezeShootCharging) || cx.b.mem.has(Mem::BreezeShootRecovering) {
            return;
        }
        cx.b.mem.set_expiring(Mem::BreezeShootRecovering, Val::Unit, SHOOT_RECOVER_DELAY as i64);
        let e = &*cx.e;
        let (dx, dz) = (t.pos.x - e.x(), t.pos.z - e.z());
        // `getY(0.8)` when riding, else `getY(0.3)`.
        let riding = cx.level.entity(t.id).is_some_and(|o| o.vehicle.is_some()) || cx.level.player(t.id).is_some_and(|p| p.vehicle.is_some());
        let dy = t.pos.y + (t.bb.max_y - t.bb.min_y) * if riding { 0.8 } else { 0.3 } - firing_y(e);
        let id = cx.level.next_entity_id();
        let seed = cx.level.fresh_seed();
        let mut charge = crate::ext_entity::wind_charge::new(id, e.id, Vec3::new(e.x(), firing_y(e), e.z()), seed);
        mob::species::shoot(&mut charge, dx, dy, dz, 0.7, (5 - cx.level.difficulty() as i32 * 4) as f32);
        charge.set_old_pos_and_rot();
        cx.level.add_entity(charge);
        play(cx.e, cx.level, "minecraft:entity.breeze.shoot", 1.5);
    }
    behavior_boilerplate!();
}

/// `LongJump`: inhales for 10 ticks, then leaps along the first workable angle of 40..80 degrees
/// to a spot behind the target and shoots for 100 ticks when it lands.
#[derive(Clone, Debug)]
struct LongJump;

impl LongJump {
    /// `LongJump.canRun`.
    fn can_run(cx: &mut Cx) -> bool {
        if !cx.e.on_ground && !cx.e.is_in_water() {
            return false;
        }
        if cx.e.fluid_height_water() > cx.e.eye_height as f64 || cx.e.is_in_lava() {
            return false;
        }
        if cx.b.mem.has(Mem::BreezeJumpTarget) {
            return true;
        }
        let Some(t) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return false };
        // `outOfAggroRange`.
        let range = cx.m.attrs.value(FollowRange);
        if !(cx.e.position().distance_to_sqr(t.pos) < range * range) {
            cx.b.mem.erase(Mem::AttackTarget);
            return false;
        }
        // `tooCloseForJump`.
        if distance_to(cx.e, t.pos) - 4.0 <= 0.0 {
            return false;
        }
        if !LongJump::can_jump_from_current_position(cx) {
            return false;
        }
        let behind = point_behind(cx.e, &*cx.level, &t);
        let Some(p) = snap_to_surface(&*cx.level, behind) else { return false };
        if block_dangerous(cx.level.block(p.below())) {
            return false;
        }
        let centre = |p: BlockPos| Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
        if !has_line_of_sight(cx.e, cx.m, &*cx.level, centre(p)) && !has_line_of_sight(cx.e, cx.m, &*cx.level, centre(p.offset(0, 4, 0))) {
            return false;
        }
        cx.b.mem.set(Mem::BreezeJumpTarget, Val::Block(p));
        true
    }

    /// `canJumpFromCurrentPosition`: not on honey, four blocks of air or water above.
    fn can_jump_from_current_position(cx: &Cx) -> bool {
        let here = cx.e.block_position();
        if crate::blocks::block_name(cx.level.block(here)) == "minecraft:honey_block" {
            return false;
        }
        (1..=4).all(|i| {
            let s = cx.level.block(here.offset(0, i, 0));
            kiln_data::blocks_types::is_air(s) || crate::physics::fluid_state(s).kind.is_water()
        })
    }

    /// `calculateOptimalJumpVector`: the angles in a shuffled order, the first that works.
    fn optimal_vector(cx: &mut Cx, target: Vec3) -> Option<Vec3> {
        let mut angles = [40, 55, 60, 75, 80];
        // `Util.shuffledCopy`.
        for i in (1..angles.len()).rev() {
            let j = cx.e.random.next_int_bounded(i as i32 + 1) as usize;
            angles.swap(i, j);
        }
        let max_v = 0.058333334f32 * cx.m.attrs.value(FollowRange) as f32;
        let v = angles.iter().find_map(|&a| jump_vector(cx.e, cx.m, target, max_v, a))?;
        if let Some(a) = mob::effects::amplifier(cx.m, crate::effect::ids::jump_boost()) {
            let _ = a;
            let power = mob::effects::jump_boost_power(cx.m) as f64;
            return Some(v.add(0.0, v.normalize().y * power, 0.0));
        }
        Some(v)
    }
}

impl Behavior for LongJump {
    fn name(&self) -> &'static str {
        "LongJump"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::AttackTarget, ValuePresent),
            (Mem::BreezeJumpCooldown, ValueAbsent),
            (Mem::BreezeJumpInhaling, Registered),
            (Mem::BreezeJumpTarget, Registered),
            (Mem::BreezeShoot, ValueAbsent),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::BreezeLeavingWater, Registered),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        (200, 200)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        LongJump::can_run(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        pose_of(cx.m) != pose::STANDING && !cx.b.mem.has(Mem::BreezeJumpCooldown)
    }
    fn start(&mut self, cx: &mut Cx) {
        if cx.b.mem.check(Mem::BreezeJumpInhaling, ValueAbsent) {
            cx.b.mem.set_expiring(Mem::BreezeJumpInhaling, Val::Unit, INHALING_DURATION as i64);
        }
        set_pose(cx.e, cx.m, pose::INHALING);
        play(cx.e, cx.level, "minecraft:entity.breeze.charge", 1.0);
        if let Some(p) = cx.b.mem.block(Mem::BreezeJumpTarget) {
            look_at_eyes(cx.e, cx.m, Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5));
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        let in_water = cx.e.is_in_water();
        if !in_water && cx.b.mem.check(Mem::BreezeLeavingWater, ValuePresent) {
            cx.b.mem.erase(Mem::BreezeLeavingWater);
        }
        // `isFinishedInhaling`.
        if !cx.b.mem.has(Mem::BreezeJumpInhaling) && pose_of(cx.m) == pose::INHALING {
            let jump = cx.b.mem.block(Mem::BreezeJumpTarget).and_then(|p| LongJump::optimal_vector(cx, Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5)));
            let Some(v) = jump else {
                set_pose(cx.e, cx.m, pose::STANDING);
                return;
            };
            if in_water {
                cx.b.mem.set(Mem::BreezeLeavingWater, Val::Unit);
            }
            play(cx.e, cx.level, "minecraft:entity.breeze.jump", 1.0);
            set_pose(cx.e, cx.m, pose::LONG_JUMPING);
            cx.e.y_rot = cx.m.y_body_rot;
            st_mut(cx.m).discard_friction = true;
            cx.e.delta = v;
            cx.e.needs_sync = true;
        } else if pose_of(cx.m) == pose::LONG_JUMPING && (cx.e.on_ground || (in_water && cx.b.mem.check(Mem::BreezeLeavingWater, ValueAbsent))) {
            // `isFinishedJumping`.
            play(cx.e, cx.level, "minecraft:entity.breeze.land", 1.0);
            set_pose(cx.e, cx.m, pose::STANDING);
            st_mut(cx.m).discard_friction = false;
            let cooldown = if cx.b.mem.has(Mem::HurtBy) { 2 } else { 10 };
            cx.b.mem.set_expiring(Mem::BreezeJumpCooldown, Val::Unit, cooldown);
            cx.b.mem.set_expiring(Mem::BreezeShoot, Val::Unit, 100);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        if matches!(pose_of(cx.m), pose::LONG_JUMPING | pose::INHALING) {
            set_pose(cx.e, cx.m, pose::STANDING);
        }
        cx.b.mem.erase(Mem::BreezeJumpTarget);
        cx.b.mem.erase(Mem::BreezeJumpInhaling);
        cx.b.mem.erase(Mem::BreezeLeavingWater);
    }
    behavior_boilerplate!();
}

/// `ShootWhenStuck`: in water or riding it just shoots (for 60 ticks).
#[derive(Clone, Debug)]
struct ShootWhenStuck;

impl Behavior for ShootWhenStuck {
    fn name(&self) -> &'static str {
        "ShootWhenStuck"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::AttackTarget, ValuePresent),
            (Mem::BreezeJumpInhaling, ValueAbsent),
            (Mem::BreezeJumpTarget, ValueAbsent),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::BreezeShoot, ValueAbsent),
        ]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.e.vehicle.is_some() || cx.e.is_in_water() || mob::effects::has(cx.m, crate::effect::ids::levitation())
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.set_expiring(Mem::BreezeShoot, Val::Unit, 60);
    }
    behavior_boilerplate!();
}

/// `Slide`: picks a spot to slide to around the target (a walk target).
#[derive(Clone, Debug)]
struct Slide;

impl Behavior for Slide {
    fn name(&self) -> &'static str {
        "Slide"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::AttackTarget, ValuePresent), (Mem::WalkTarget, ValueAbsent), (Mem::BreezeJumpCooldown, ValueAbsent), (Mem::BreezeShoot, ValueAbsent)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.e.on_ground && !cx.e.is_in_water() && pose_of(cx.m) == pose::STANDING
    }
    fn start(&mut self, cx: &mut Cx) {
        let Some(t) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return };
        // `withinInnerCircleRange`: closer than 4 across and 10 up or down of the block's centre.
        let centre = cx.e.block_position().center();
        let (dx, dy, dz) = (t.pos.x - centre.x, t.pos.y - centre.y, t.pos.z - centre.z);
        let inner = dx * dx + dz * dz < 16.0 && dy.abs() < 10.0;
        let mut spot = None;
        if inner
            && let Some(p) = random_pos::default_pos_away(cx.e, cx.m, &*cx.level, 5, 5, t.pos)
            && has_line_of_sight(cx.e, cx.m, &*cx.level, p)
            && t.pos.distance_to_sqr(p) > t.pos.distance_to_sqr(cx.e.position())
        {
            spot = Some(p);
        }
        let spot = match spot {
            Some(p) => p,
            None if cx.e.random.next_bool() => point_behind(cx.e, &*cx.level, &t),
            None => {
                // `randomPointInMiddleCircle`.
                let d = t.pos - cx.e.position();
                let dist = d.length() - crate::math::lerp(cx.e.random.next_double(), 8.0, 4.0);
                cx.e.position() + d.normalize().scale(dist)
            }
        };
        cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(BlockPos::containing(spot.x, spot.y, spot.z), 0.6, 1)));
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- sensor

/// `BreezeAttackEntitySensor`: `NearestLivingEntitySensor`, then the nearest living entity (not a
/// creative or spectating player) it can attack, as `NEAREST_ATTACKABLE`.
#[derive(Clone, Debug)]
struct BreezeAttackEntitySensor;

impl Sensor for BreezeAttackEntitySensor {
    fn name(&self) -> &'static str {
        "BreezeAttackEntitySensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestLivingEntities, Mem::NearestVisibleLivingEntities, Mem::NearestAttackable]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        NearestLivingEntities.do_tick(cx);
        let ids = cx.b.mem.entities(Mem::NearestLivingEntities).to_vec();
        let mut found = None;
        for id in ids {
            let Some(l) = util::living(cx, id) else { continue };
            if l.player && (l.creative || l.spectator) {
                continue;
            }
            if util::is_entity_attackable(cx, &l) {
                found = Some(id);
                break;
            }
        }
        cx.b.mem.set_opt(Mem::NearestAttackable, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

// ---------------------------------------------------------------------------- brain

/// `BreezeAi.getActivities` and `Breeze.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![Box::new(NearestLivingEntities), Box::new(HurtBy), Box::new(Players), Box::new(BreezeAttackEntitySensor)];
    let core = ActivityData::create(Activity::Core, 0, vec![Timed::new(BreezeSwim), LookAtTargetSink::new(45, 90)]);
    let idle = ActivityData::with_priorities(
        Activity::Idle,
        vec![
            (0, combat::start_attacking(|_| true, |cx| cx.b.mem.entity(Mem::NearestAttackable))),
            (
                1,
                combat::start_attacking(
                    |_| true,
                    // The living entity behind the damage that hurt it.
                    |cx| cx.b.mem.damage(Mem::HurtBy).and_then(|d| d.attacker).filter(|&a| util::living(cx, a).is_some()),
                ),
            ),
            (2, Timed::new(SlideToTargetSink(MoveToTargetSink::plain(20, 40)))),
            (3, Gate::run_one(vec![(DoNothing::new(20, 100), 1), (stroll(0.6, StrollKind::Land { avoid_water: false }), 2)])),
        ],
    );
    let fight = ActivityData::with_conditions(
        Activity::Fight,
        vec![
            (0, shot_of(StopAttacking { remember: 0 })),
            (1, Timed::new(Shoot)),
            (2, Timed::new(LongJump)),
            (3, Timed::new(ShootWhenStuck)),
            (4, Timed::new(Slide)),
        ],
        &[(Mem::AttackTarget, ValuePresent), (Mem::WalkTarget, ValueAbsent)],
    );
    let mut brain = Brain::new(&[], sensors, vec![core, idle, fight], random);
    // `makeBrain`: fighting is the default activity.
    brain.st.set_default_activity(Activity::Fight);
    brain.st.use_default_activity();
    brain
}

fn shot_of(b: StopAttacking) -> Box<dyn Control> {
    brain::Shot::new(b)
}

/// `BreezeAi.updateActivity`.
fn update_activity(m: &mut MobData) {
    if let Some(b) = m.brain.as_mut() {
        b.st.set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
    }
}

fn sync_target(m: &mut MobData) {
    if let Some(b) = m.brain.as_ref() {
        m.target = b.st.mem.entity(Mem::AttackTarget);
    }
}

impl Kind for Breeze {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.maluses.push((path::PathType::OnTopOfTrapdoor, -1.0));
        m.maluses.push((path::PathType::Fire, -1.0));
        Some(Box::new(State::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    /// `Breeze.tick` before `Mob.tick`: the ground particles' draw, the whirl sound's clock.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if matches!(st(m).pose, pose::SHOOTING | pose::INHALING | pose::STANDING) {
            e.random.next_int_bounded(1);
        }
        let s = st_mut(m);
        s.sound_tick = if s.sound_tick == 0 { mob::mth::next_int_between(&mut e.random, 1, 80) } else { s.sound_tick - 1 };
        if st(m).sound_tick == 0 {
            // `playWhirlSound` (heard only where it is played: the draws).
            e.random.next_float();
            e.random.next_float();
        }
    }

    /// `Breeze.customServerAiStep`: the brain, then `BreezeAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        update_activity(m);
        sync_target(m);
    }

    /// Its ambient sounds are the client's own (`playLocalSound`).
    fn ambient_sound(&self, _e: &mut Entity, _m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(None)
    }

    fn can_attack(&self, _m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        t.player || t.type_name == "minecraft:iron_golem"
    }

    /// `isInvulnerableTo`: another breeze's doing.
    fn hurt(&self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        let by_breeze = source.attacker.and_then(|a| level.entity(a)).is_some_and(|a| a.type_name == "minecraft:breeze");
        by_breeze.then_some(false)
    }

    fn discard_friction(&self, m: &MobData) -> bool {
        st(m).discard_friction
    }

    /// `getFluidJumpThreshold`: its eye height.
    fn fluid_jump_threshold(&self, e: &Entity) -> Option<f64> {
        Some(e.eye_height as f64)
    }

    fn swim_sound(&self) -> Option<&'static str> {
        None
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(10)
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let p = st(m).pose;
        if p != pose::STANDING {
            d.set(kiln_data::entities::data::entity::POSE, &DataValue::Pose(p));
        }
    }

    fn load(&self, _e: &mut Entity, _m: &mut MobData, _r: &mut Input) {}

    fn save(&self, _e: &Entity, _m: &MobData, _o: &mut Output) {}
}
