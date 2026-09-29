//! What frogs and tadpoles share: the long jump behaviours (`LongJumpMidJump`,
//! `LongJumpToRandomPos`, `LongJumpToPreferredBlock`), the land finders and the spawn layer
//! (`TryFindLand`, `TryFindLandNearLiquid`, `TryLaySpawnOnFluidNearLand`), and the
//! `SmoothSwimmingMoveControl` / `SmoothSwimmingLookControl` of swimmers that also walk.

use super::memory::{Tracker, Val, WalkTarget};
use super::{Behavior, Control, Cx, Mem, Status, Timed, shot};
use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{Axis, BlockPos, Vec3};
use crate::mob::attributes::Attr;
use crate::mob::control::{self, Operation};
use crate::mob::{self, MobData, mth};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::sync::Arc;

use Status::{Registered, ValueAbsent, ValuePresent};

// ---------------------------------------------------------------------------- long jumps

/// The settings of a long jumper (`LongJumpToRandomPos`'s constructor arguments) and what ties
/// the behaviours to their mob type.
#[derive(Clone, Copy, Debug)]
pub struct LongJumpCfg {
    /// `timeBetweenLongJumps`: `UniformInt.of(min, max)`.
    pub time_between: (i32, i32),
    pub max_height: i32,
    pub max_width: i32,
    pub max_velocity: f32,
    /// The jump and landing sounds.
    pub jump_sound: &'static str,
    pub landing_sound: &'static str,
    /// `acceptableLandingSpot`.
    pub acceptable: fn(&mut Cx, BlockPos) -> bool,
    /// `LongJumpToPreferredBlock`: the preferred blocks (a tag test) and how often they are wanted.
    pub preferred: Option<(fn(u16) -> bool, f32)>,
    /// `setPose(LONG_JUMPING)` (true) and `setPose(STANDING)` (false).
    pub set_pose: fn(&mut MobData, bool),
    /// The random `Collections.shuffle` uses: unseedable in vanilla, so the parity replay pins
    /// it ([`crate::mob::ext::Kind::pin_replay`]); otherwise the mob's own stream.
    pub shuffle: fn(&mut MobData) -> &mut LegacyRandom,
}

/// `UniformInt.sample`.
fn sample(cx: &mut Cx, (min, max): (i32, i32)) -> i32 {
    cx.rng().next_int_bounded(max - min + 1) + min
}

/// `LongJumpMidJump`: in the air; on landing the jumper skids, then waits to jump again.
#[derive(Clone, Debug)]
pub struct LongJumpMidJump {
    cfg: LongJumpCfg,
}

impl LongJumpMidJump {
    pub fn new(cfg: LongJumpCfg) -> Box<dyn Control> {
        Timed::new(LongJumpMidJump { cfg })
    }
}

impl Behavior for LongJumpMidJump {
    fn name(&self) -> &'static str {
        "LongJumpMidJump"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, Registered), (Mem::LongJumpMidJump, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (100, 100)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        !cx.e.on_ground
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.m.discard_friction = true;
        (self.cfg.set_pose)(cx.m, true);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if cx.e.on_ground {
            cx.e.delta = cx.e.delta.multiply(0.10000000149011612, 1.0, 0.10000000149011612);
            let pos = cx.e.position();
            cx.level.emit(Event::Sound { pos, sound: self.cfg.landing_sound, source: "neutral", volume: 2.0, pitch: 1.0 });
        }
        cx.m.discard_friction = false;
        (self.cfg.set_pose)(cx.m, false);
        cx.b.mem.erase(Mem::LongJumpMidJump);
        let n = sample(cx, self.cfg.time_between);
        cx.b.mem.set(Mem::LongJumpCooldownTicks, Val::Int(n));
    }
    behavior_boilerplate!();
}

/// `LongJumpToRandomPos` (and `LongJumpToPreferredBlock` when the settings prefer blocks): picks
/// a spot within reach that cannot be walked to, works out a jump, prepares for two seconds and
/// leaps.
#[derive(Clone, Debug)]
pub struct LongJumpToRandomPos {
    cfg: LongJumpCfg,
    /// `jumpCandidates`: (position, weight).
    candidates: Vec<(BlockPos, i32)>,
    initial_position: Option<Vec3>,
    chosen_jump: Option<Vec3>,
    find_jump_tries: i32,
    prepare_jump_start: i64,
    not_preferred: Vec<(BlockPos, i32)>,
    currently_wanting_preferred: bool,
}

impl LongJumpToRandomPos {
    pub fn new(cfg: LongJumpCfg) -> Box<dyn Control> {
        Timed::new(LongJumpToRandomPos {
            cfg,
            candidates: Vec::new(),
            initial_position: None,
            chosen_jump: None,
            find_jump_tries: 0,
            prepare_jump_start: 0,
            not_preferred: Vec::new(),
            currently_wanting_preferred: false,
        })
    }

    /// `WeightedRandom.getRandomItem(level.getRandom(), jumpCandidates, weight)`, removing the pick.
    fn base_candidate(&mut self, cx: &mut Cx) -> Option<(BlockPos, i32)> {
        let total: i64 = self.candidates.iter().map(|c| c.1 as i64).sum();
        if total == 0 {
            return None;
        }
        let mut r = cx.rng().next_int_bounded(total as i32);
        for i in 0..self.candidates.len() {
            r -= self.candidates[i].1;
            if r < 0 {
                return Some(self.candidates.remove(i));
            }
        }
        None
    }

    /// `getJumpCandidate` (`LongJumpToPreferredBlock`'s takes preferred landing blocks first).
    fn jump_candidate(&mut self, cx: &mut Cx) -> Option<(BlockPos, i32)> {
        let Some((is_preferred, _)) = self.cfg.preferred else { return self.base_candidate(cx) };
        if !self.currently_wanting_preferred {
            return self.base_candidate(cx);
        }
        while !self.candidates.is_empty() {
            if let Some(c) = self.base_candidate(cx) {
                if is_preferred(cx.level.block(c.0.below())) {
                    return Some(c);
                }
                self.not_preferred.push(c);
            }
        }
        if self.not_preferred.is_empty() { None } else { Some(self.not_preferred.remove(0)) }
    }

    /// `isAcceptableLandingPosition`.
    fn acceptable_landing(&self, cx: &mut Cx, pos: BlockPos) -> bool {
        let here = cx.e.block_position();
        if here.x == pos.x && here.z == pos.z {
            return false;
        }
        (self.cfg.acceptable)(cx, pos)
    }

    /// `calculateOptimalJumpVector`: the four launch angles in a shuffled order, the first that works.
    fn optimal_jump_vector(&self, cx: &mut Cx, target: Vec3) -> Option<Vec3> {
        let mut angles = [65, 70, 75, 80];
        {
            let rnd = (self.cfg.shuffle)(cx.m);
            // `Collections.shuffle(list, rnd)`: for i from size down to 2, swap(i - 1, nextInt(i)).
            for i in (2..=angles.len()).rev() {
                let j = rnd.next_int_bounded(i as i32) as usize;
                angles.swap(i - 1, j);
            }
        }
        let max_velocity = (cx.m.attrs.value(Attr::JumpStrength) * self.cfg.max_velocity as f64) as f32;
        for angle in angles {
            if let Some(v) = jump_vector_for_angle(cx, target, max_velocity, angle, true) {
                return Some(v);
            }
        }
        None
    }

    /// `pickCandidate`.
    fn pick_candidate(&mut self, cx: &mut Cx) {
        while !self.candidates.is_empty() {
            let Some((pos, _)) = self.jump_candidate(cx) else { continue };
            if !self.acceptable_landing(cx, pos) {
                continue;
            }
            let center = pos.center();
            let Some(v) = self.optimal_jump_vector(cx, center) else { continue };
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(pos)));
            let path = mob::path::create_path_max(cx.e, cx.m, &*cx.level, pos, 0, 8.0);
            if path.as_ref().is_none_or(|p| !p.reached) {
                self.chosen_jump = Some(v);
                self.prepare_jump_start = cx.time;
                return;
            }
        }
    }
}

impl Behavior for LongJumpToRandomPos {
    fn name(&self) -> &'static str {
        if self.cfg.preferred.is_some() { "LongJumpToPreferredBlock" } else { "LongJumpToRandomPos" }
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, Registered), (Mem::LongJumpCooldownTicks, ValueAbsent), (Mem::LongJumpMidJump, ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (200, 200)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        let honey = crate::blocks::block_name(cx.level.block(cx.e.block_position())) == "minecraft:honey_block";
        let ok = cx.e.on_ground && !cx.e.is_in_water() && !cx.e.is_in_lava() && !honey;
        if !ok {
            let n = sample(cx, self.cfg.time_between) / 2;
            cx.b.mem.set(Mem::LongJumpCooldownTicks, Val::Int(n));
        }
        ok
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let ok = self.initial_position.is_some_and(|p| p == cx.e.position())
            && self.find_jump_tries > 0
            && !cx.e.is_in_water()
            && (self.chosen_jump.is_some() || !self.candidates.is_empty());
        if !ok && !cx.b.mem.has(Mem::LongJumpMidJump) {
            let n = sample(cx, self.cfg.time_between) / 2;
            cx.b.mem.set(Mem::LongJumpCooldownTicks, Val::Int(n));
            cx.b.mem.erase(Mem::LookTarget);
        }
        ok
    }
    fn start(&mut self, cx: &mut Cx) {
        self.chosen_jump = None;
        self.find_jump_tries = 20;
        self.initial_position = Some(cx.e.position());
        let p = cx.e.block_position();
        let (w, h) = (self.cfg.max_width, self.cfg.max_height);
        self.candidates.clear();
        // `BlockPos.betweenClosedStream`: x varies fastest, then y, then z.
        for dz in -w..=w {
            for dy in -h..=h {
                for dx in -w..=w {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let d = (dx * dx + dy * dy + dz * dz) as f64;
                    self.candidates.push((BlockPos::new(p.x + dx, p.y + dy, p.z + dz), mth::ceil(d)));
                }
            }
        }
        if let Some((_, chance)) = self.cfg.preferred {
            self.not_preferred.clear();
            self.currently_wanting_preferred = cx.e.random.next_float() < chance;
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        if let Some(jump) = self.chosen_jump {
            if cx.time - self.prepare_jump_start >= 40 {
                cx.e.y_rot = cx.m.y_body_rot;
                cx.m.discard_friction = true;
                let len = jump.length();
                let boost = mob::effects::jump_boost_power(cx.m) as f64;
                cx.e.delta = jump.scale((len + boost) / len);
                cx.b.mem.set(Mem::LongJumpMidJump, Val::Bool(true));
                let pos = cx.e.position();
                cx.level.emit(Event::Sound { pos, sound: self.cfg.jump_sound, source: "neutral", volume: 1.0, pitch: 1.0 });
            }
        } else {
            self.find_jump_tries -= 1;
            self.pick_candidate(cx);
        }
    }
    behavior_boilerplate!();
}

/// `LongJumpUtil.calculateJumpVectorForAngle`: the launch velocity that lands the mob half a
/// block short of `target` at `angle` degrees, if it is within `max_velocity` (and, when asked,
/// nothing is in the way along the arc).
pub fn jump_vector_for_angle(cx: &mut Cx, target: Vec3, max_velocity: f32, angle: i32, check_clear: bool) -> Option<Vec3> {
    let pos = cx.e.position();
    let offset = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z).normalize().scale(0.5);
    let adjusted = target - offset;
    let d = adjusted - pos;
    let a = (angle as f32 * 3.1415927f32) / 180.0f32;
    let yaw = d.z.atan2(d.x);
    let horizontal_sqr = d.subtract(0.0, d.y, 0.0).length_sqr();
    let horizontal = horizontal_sqr.sqrt();
    let dy = d.y;
    let gravity = cx.m.attrs.value(Attr::Gravity);
    let sin2a = ((2.0f32 * a) as f64).sin();
    let cos_sqr = { let c = (a as f64).cos(); c * c };
    let sin_a = (a as f64).sin();
    let cos_a = (a as f64).cos();
    let sin_yaw = yaw.sin();
    let cos_yaw = yaw.cos();
    let v2 = (horizontal_sqr * gravity) / ((horizontal * sin2a) - ((2.0 * dy) * cos_sqr));
    if !(v2 >= 0.0) {
        return None;
    }
    let v = v2.sqrt();
    if !(v <= max_velocity as f64) {
        return None;
    }
    let vh = v * cos_a;
    let vv = v * sin_a;
    if check_clear {
        let steps = mth::ceil(horizontal / vh) * 2;
        let mut travelled = 0.0;
        let mut previous: Option<Vec3> = None;
        for _ in 0..steps - 1 {
            travelled += horizontal / steps as f64;
            let h = ((sin_a / cos_a) * travelled) - (((travelled * travelled) * gravity) / ((2.0 * v2) * (cos_a * cos_a)));
            let p = Vec3::new(pos.x + travelled * cos_yaw, pos.y + h, pos.z + travelled * sin_yaw);
            if let Some(prev) = previous
                && !is_clear_transition(cx, prev, p)
            {
                return None;
            }
            previous = Some(p);
        }
    }
    Some(Vec3::new(vh * cos_yaw, vv, vh * sin_yaw).scale(0.949999988079071))
}

/// `LongJumpUtil.isClearTransition`: the mob's box, stepped from one point to the next, touches nothing.
fn is_clear_transition(cx: &mut Cx, from: Vec3, to: Vec3) -> bool {
    let diff = to - from;
    let size = cx.e.width.min(cx.e.height) as f64;
    let n = mth::ceil(diff.length() / size);
    let dir = diff.normalize();
    let mut cur = from;
    let ctx = cx.e.collision_context();
    for i in 0..n {
        cur = if i == n - 1 { to } else { cur + dir.scale(size * 0.8999999761581421) };
        let b = cx.e.make_bounding_box(cur);
        if !crate::collision::no_collision(&*cx.level, &ctx, cx.e.id, &b) {
            return false;
        }
    }
    true
}

/// `LongJumpToRandomPos.defaultAcceptableLandingSpot`: a solid block below and a free (no malus)
/// path type at the spot.
pub fn default_acceptable_landing_spot(cx: &mut Cx, pos: BlockPos) -> bool {
    let below = pos.below();
    kiln_data::block_props::solid_render(cx.level.block(below))
        && mob::path::malus(cx.m, mob::path::path_type_static(&*cx.level, pos.x, pos.y, pos.z)) == 0.0
}

// ---------------------------------------------------------------------------- land finders

/// `BlockPos.differsHorizontally`.
fn differs_horizontally(a: BlockPos, b: BlockPos) -> bool {
    a.x != b.x || a.z != b.z
}

/// `TryFindLand.canStandOn`: an empty collision shape over a block with a sturdy top.
fn can_stand_on(cx: &Cx, pos: BlockPos, state: u16, ctx: &crate::collision::CollisionContext) -> bool {
    let (shape, _) = crate::collision::collision_shape(state, pos, ctx);
    shape.is_empty() && crate::physics::is_face_sturdy(cx.level.block(pos.below()), crate::math::Direction::Up)
}

/// `TryFindLand.create(range, speed)`: a mob in the water looks for dry land within `range`
/// (re-checked at most once a minute).
pub fn try_find_land(range: i32, speed: f32) -> Box<dyn Control> {
    let next_ok = Arc::new(std::sync::atomic::AtomicI64::new(0));
    shot(
        "TryFindLand",
        &[(Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered)],
        move |cx| {
            use std::sync::atomic::Ordering::Relaxed;
            let here = cx.e.block_position();
            if !crate::physics::fluid_state(cx.level.block(here)).kind.is_water() {
                return false;
            }
            if cx.time < next_ok.load(Relaxed) {
                next_ok.store(cx.time + 60, Relaxed);
                return true;
            }
            let ctx = cx.e.collision_context();
            let found = mob::kinds::turtle::within_manhattan(here, range, range, range)
                .filter(|p| differs_horizontally(*p, here))
                .filter(|p| crate::physics::fluid_state(cx.level.block(*p)).is_empty())
                .find(|&p| can_stand_on(cx, p, cx.level.block(p), &ctx));
            if let Some(p) = found {
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(p)));
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::block(p), speed, close_enough: 1 }));
            }
            next_ok.store(cx.time + 60, Relaxed);
            true
        },
    )
}

/// `TryFindLandNearLiquid.create(range, speed, fluid)`: a mob out of the fluid looks for dry land
/// with that fluid beside it (`is_fluid` is the fluid tag).
pub fn try_find_land_near_liquid(range: i32, speed: f32, is_fluid: fn(u16) -> bool) -> Box<dyn Control> {
    let next_ok = Arc::new(std::sync::atomic::AtomicI64::new(0));
    shot(
        "TryFindLandNearLiquid",
        &[(Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered)],
        move |cx| {
            use std::sync::atomic::Ordering::Relaxed;
            let here = cx.e.block_position();
            if is_fluid(cx.level.block(here)) {
                return false;
            }
            if cx.time < next_ok.load(Relaxed) {
                next_ok.store(cx.time + 40, Relaxed);
                return true;
            }
            let ctx = cx.e.collision_context();
            let level: &dyn EntityLevel = &*cx.level;
            let found = mob::kinds::turtle::within_manhattan(here, range, range, range).filter(|p| differs_horizontally(*p, here)).find(|&p| {
                let (shape, _) = crate::collision::collision_shape(level.block(p), p, &ctx);
                if !shape.is_empty() {
                    return false;
                }
                let (below, _) = crate::collision::collision_shape(level.block(p.below()), p, &ctx);
                if below.is_empty() {
                    return false;
                }
                use crate::math::Direction::{East, North, South, West};
                [North, East, South, West].iter().any(|&d| {
                    let n = p.relative(d);
                    kiln_data::blocks_types::is_air(level.block(n)) && is_fluid(level.block(n.below()))
                })
            });
            if let Some(p) = found {
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(p)));
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::block(p), speed, close_enough: 0 }));
            }
            next_ok.store(cx.time + 40, Relaxed);
            true
        },
    )
}

/// `VoxelShape.getFaceShape(UP).isEmpty()` for a collision shape: nothing reaches the top of the block.
pub fn top_face_empty(shape: &crate::shape::Shape) -> bool {
    shape.is_empty() || shape.max(Axis::Y, 0.0) < 1.0 - 1.0e-7
}

/// `TryLaySpawnOnFluidNearLand.create(block)`: a pregnant mob on the shore lays `spawn` (the
/// frogspawn block state) on top of a `supports` fluid or block beside and below it.
pub fn try_lay_spawn_on_fluid_near_land(spawn: u16, supports: fn(u16) -> bool) -> Box<dyn Control> {
    shot(
        "TryLaySpawnOnFluidNearLand",
        &[(Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValuePresent), (Mem::IsPregnant, ValuePresent)],
        move |cx| {
            if cx.e.is_in_water() || !cx.e.on_ground {
                return false;
            }
            let below = cx.e.block_position().below();
            use crate::math::Direction::{East, North, South, West};
            for d in [North, East, South, West] {
                let p = below.relative(d);
                let state = cx.level.block(p);
                let (shape, _) = crate::collision::collision_shape(state, p, &crate::collision::CollisionContext::EMPTY);
                if !top_face_empty(&shape) {
                    continue;
                }
                if !supports(state) {
                    continue;
                }
                let above = p.above();
                if kiln_data::blocks_types::is_air(cx.level.block(above)) {
                    cx.level.set_block(above, spawn, 3);
                    let (id, pos) = (cx.e.id, above.center());
                    cx.level.emit(Event::GameEvent { event: "minecraft:block_place", pos, entity: Some(id) });
                    let at = cx.e.position();
                    cx.level.emit(Event::Sound { pos: at, sound: "minecraft:entity.frog.lay_spawn", source: "block", volume: 1.0, pitch: 1.0 });
                    cx.b.mem.erase(Mem::IsPregnant);
                    return true;
                }
            }
            true
        },
    )
}

// ---------------------------------------------------------------------------- smooth swimming

/// `getTurningSpeedFactor`.
fn turning_speed_factor(degrees: f32) -> f32 {
    1.0 - mth::clamp((degrees - 10.0) / 50.0, 0.0, 1.0)
}

/// `SmoothSwimmingMoveControl.tick(maxTurnX, maxTurnY, inWaterSpeedModifier, outsideWaterSpeedModifier, applyGravity)`.
pub fn smooth_swimming_move(e: &mut Entity, m: &mut MobData, max_turn_x: i32, max_turn_y: i32, in_water_modifier: f32, outside_water_modifier: f32, apply_gravity: bool) {
    if apply_gravity && e.is_in_water() {
        e.delta = e.delta.add(0.0, 0.005, 0.0);
    }
    if m.mov.operation != Operation::MoveTo || m.nav.is_done() {
        control::set_speed(m, 0.0);
        m.xxa = 0.0;
        m.yya = 0.0;
        m.zza = 0.0;
        return;
    }
    let [wx, wy, wz] = m.mov.wanted;
    let (dx, dy, dz) = (wx - e.x(), wy - e.y(), wz - e.z());
    let dist = dx * dx + dy * dy + dz * dz;
    if dist < 2.500000277905201e-7 {
        m.zza = 0.0;
        return;
    }
    let yaw = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
    e.y_rot = control::rotlerp(e.y_rot, yaw, max_turn_y as f32);
    m.y_body_rot = e.y_rot;
    m.y_head_rot = e.y_rot;
    let speed = (m.mov.speed_modifier * m.attrs.value(Attr::MovementSpeed)) as f32;
    if e.is_in_water() {
        control::set_speed(m, speed * in_water_modifier);
        let horizontal = (dx * dx + dz * dz).sqrt();
        if dy.abs() > 9.999999747378752e-6 || horizontal.abs() > 9.999999747378752e-6 {
            let pitch = -((mth::atan2(dy, horizontal) * 57.2957763671875) as f32);
            let pitch = mth::clamp(mth::wrap_degrees(pitch), -(max_turn_x as f32), max_turn_x as f32);
            e.x_rot = mth::rotate_towards(e.x_rot, pitch, 5.0);
        }
        let cos = mth::cos((e.x_rot * 0.017453292) as f64);
        let sin = mth::sin((e.x_rot * 0.017453292) as f64);
        m.zza = cos * speed;
        m.yya = -sin * speed;
    } else {
        let turn = mth::wrap_degrees(e.y_rot - yaw).abs();
        let f = turning_speed_factor(turn);
        control::set_speed(m, (speed * outside_water_modifier) * f);
    }
}

/// `SmoothSwimmingLookControl.tick(maxYRotFromCenter)`.
pub fn smooth_swimming_look(e: &mut Entity, m: &mut MobData, max_y_rot_from_center: i32) {
    if m.look.cooldown > 0 {
        m.look.cooldown -= 1;
        let [wx, wy, wz] = m.look.wanted;
        let (dx, dz) = (wx - e.x(), wz - e.z());
        if dz.abs() > 9.999999747378752e-6 || dx.abs() > 9.999999747378752e-6 {
            let yaw = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
            m.y_head_rot = mth::rotate_towards(m.y_head_rot, yaw + 20.0, m.look.y_max_rot_speed);
        }
        let dy = wy - e.eye_y();
        let h = (dx * dx + dz * dz).sqrt();
        if dy.abs() > 9.999999747378752e-6 || h.abs() > 9.999999747378752e-6 {
            let pitch = (-(mth::atan2(dy, h) * 57.2957763671875)) as f32;
            e.x_rot = mth::rotate_towards(e.x_rot, pitch + 10.0, m.look.x_max_rot_angle);
        }
    } else {
        if m.nav.is_done() {
            e.x_rot = mth::rotate_towards(e.x_rot, 0.0, 5.0);
        }
        m.y_head_rot = mth::rotate_towards(m.y_head_rot, m.y_body_rot, m.look.y_max_rot_speed);
    }
    let f = mth::wrap_degrees(m.y_head_rot - m.y_body_rot);
    if f < -(max_y_rot_from_center as f32) {
        m.y_body_rot -= 4.0;
    } else if f > max_y_rot_from_center as f32 {
        m.y_body_rot += 4.0;
    }
}
