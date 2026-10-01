//! The behaviours many brains share (`net.minecraft.world.entity.ai.behavior`): `Swim`,
//! `LookAtTargetSink`, `MoveToTargetSink`, `CountDownCooldownTicks`, `DoNothing`, `RandomLookAround`,
//! `SetEntityLookTarget(Sometimes)`, `SetWalkTargetFromLookTarget`, `RandomStroll`,
//! `FollowTemptation`, `BabyFollowAdult`, `AnimalMakeLove`, `AnimalPanic`.

use super::memory::{Tracker, Val, WalkTarget};
use super::util::{self, uniform};
use super::{Behavior, Control, Cx, Mem, Shot, ShotBehavior, Status, Timed, shot};
use crate::behavior_boilerplate;
use crate::math::{BlockPos, Vec3};
use crate::mob::path::{self, Path};
use crate::mob::random_pos;
use kiln_javamath::random::RandomSource;

use Status::{Registered, ValueAbsent, ValuePresent};

// ---------------------------------------------------------------------------- Swim

/// `Swim(chance)`: jumps now and then while in deep water or lava.
#[derive(Clone, Debug)]
pub struct Swim {
    pub chance: f32,
}

impl Swim {
    pub fn new(chance: f32) -> Box<dyn Control> {
        Timed::new(Swim { chance })
    }

    fn in_fluid(cx: &Cx) -> bool {
        let threshold = if (cx.e.eye_height as f64) < 0.4 { 0.0 } else { 0.4 };
        cx.e.fluid_height_water() > threshold || cx.e.is_in_lava()
    }
}

impl Behavior for Swim {
    fn name(&self) -> &'static str {
        "Swim"
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        Swim::in_fluid(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        Swim::in_fluid(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.e.random.next_float() < self.chance {
            cx.m.jump.jump = true;
        }
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- look and move sinks

/// `LookAtTargetSink(min, max)`: looks at `LOOK_TARGET` while it is visible.
#[derive(Clone, Debug)]
pub struct LookAtTargetSink {
    pub min: i32,
    pub max: i32,
}

impl LookAtTargetSink {
    pub fn new(min: i32, max: i32) -> Box<dyn Control> {
        Timed::new(LookAtTargetSink { min, max })
    }
}

impl Behavior for LookAtTargetSink {
    fn name(&self) -> &'static str {
        "LookAtTargetSink"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (self.min, self.max)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        match cx.b.mem.look_target() {
            Some(t) => util::tracker_visible(cx, &t),
            None => false,
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::LookTarget);
    }
    fn tick(&mut self, cx: &mut Cx) {
        if let Some(t) = cx.b.mem.look_target()
            && let Some(p) = util::tracker_pos(cx, &t)
        {
            crate::mob::control::look_at(cx.m, p.x, p.y, p.z);
        }
    }
    behavior_boilerplate!();
}

/// `MoveToTargetSink`: paths to `WALK_TARGET`.
#[derive(Clone, Debug)]
pub struct MoveToTargetSink {
    min: i32,
    max: i32,
    remaining_cooldown: i32,
    /// The path `tryComputePath` made, for `start` to hand to the navigation.
    path: Option<Path>,
    /// `this.path != null`.
    path_some: bool,
    last_target_pos: Option<BlockPos>,
    speed: f32,
    /// An anonymous subclass overriding `checkExtraStartConditions` to refuse (armadillos that
    /// are rolled up): it has no class name.
    veto: Option<fn(&Cx) -> bool>,
}

impl MoveToTargetSink {
    pub fn new() -> Box<dyn Control> {
        MoveToTargetSink::with_durations(150, 250)
    }

    pub fn with_durations(min: i32, max: i32) -> Box<dyn Control> {
        Timed::new(MoveToTargetSink { min, max, remaining_cooldown: 0, path: None, path_some: false, last_target_pos: None, speed: 0.0, veto: None })
    }

    /// An anonymous `MoveToTargetSink` that does not start while `veto` holds.
    pub fn vetoed(veto: fn(&Cx) -> bool) -> Box<dyn Control> {
        Timed::new(MoveToTargetSink { min: 150, max: 250, remaining_cooldown: 0, path: None, path_some: false, last_target_pos: None, speed: 0.0, veto: Some(veto) })
    }

    fn reached(cx: &Cx, w: &WalkTarget) -> bool {
        util::tracker_block(cx, &w.target).is_some_and(|p| util::dist_manhattan(p, cx.e.block_position()) <= w.close_enough)
    }

    /// `tryComputePath`.
    fn try_compute_path(&mut self, cx: &mut Cx, w: &WalkTarget) -> bool {
        let Some(target) = util::tracker_block(cx, &w.target) else { return false };
        self.path = path::create_path(cx.e, cx.m, &*cx.level, target, 0);
        self.path_some = self.path.is_some();
        self.speed = w.speed;
        if Self::reached(cx, w) {
            cx.b.mem.erase(Mem::CantReachWalkTargetSince);
            return false;
        }
        let can_reach = self.path.as_ref().is_some_and(|p| p.reached);
        if can_reach {
            cx.b.mem.erase(Mem::CantReachWalkTargetSince);
        } else if !cx.b.mem.has(Mem::CantReachWalkTargetSince) {
            cx.b.mem.set(Mem::CantReachWalkTargetSince, Val::Long(cx.time));
        }
        if self.path.is_some() {
            return true;
        }
        let center = Vec3::new(target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5);
        if let Some(v) = random_pos::default_pos_towards(cx.e, cx.m, &*cx.level, 10, 7, center, 1.5707963705062866) {
            self.path = path::create_path(cx.e, cx.m, &*cx.level, BlockPos::containing(v.x, v.y, v.z), 0);
            self.path_some = self.path.is_some();
            return self.path.is_some();
        }
        false
    }

    fn spectator_target(cx: &Cx, w: &WalkTarget) -> bool {
        match w.target {
            Tracker::Entity { id, .. } => util::living(cx, id).is_some_and(|l| l.spectator),
            _ => false,
        }
    }
}

impl Behavior for MoveToTargetSink {
    fn name(&self) -> &'static str {
        if self.veto.is_some() { "" } else { "MoveToTargetSink" }
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::CantReachWalkTargetSince, Registered), (Mem::Path, ValueAbsent), (Mem::WalkTarget, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (self.min, self.max)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if self.veto.is_some_and(|v| v(cx)) {
            return false;
        }
        if self.remaining_cooldown > 0 {
            self.remaining_cooldown -= 1;
            return false;
        }
        let Some(w) = cx.b.mem.walk_target() else { return false };
        let reached = Self::reached(cx, &w);
        if !reached && self.try_compute_path(cx, &w) {
            self.last_target_pos = util::tracker_block(cx, &w.target);
            return true;
        }
        cx.b.mem.erase(Mem::WalkTarget);
        if reached {
            cx.b.mem.erase(Mem::CantReachWalkTargetSince);
        }
        false
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        if !self.path_some || self.last_target_pos.is_none() {
            return false;
        }
        let w = cx.b.mem.walk_target();
        let spectator = w.as_ref().is_some_and(|w| Self::spectator_target(cx, w));
        !cx.m.nav.is_done() && w.is_some_and(|w| !Self::reached(cx, &w)) && !spectator
    }
    fn stop(&mut self, cx: &mut Cx) {
        if let Some(w) = cx.b.mem.walk_target()
            && !Self::reached(cx, &w)
            && cx.m.nav.is_stuck
        {
            self.remaining_cooldown = cx.rng().next_int_bounded(40);
        }
        cx.m.nav.stop();
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::Path);
        self.path = None;
        self.path_some = false;
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.set(Mem::Path, Val::Marker);
        let p = self.path.take();
        path::move_to_path(cx.e, cx.m, &*cx.level, p, self.speed as f64);
    }
    fn tick(&mut self, cx: &mut Cx) {
        let nav_some = cx.m.nav.path.is_some();
        if nav_some != self.path_some {
            self.path_some = nav_some;
            if nav_some {
                cx.b.mem.set(Mem::Path, Val::Marker);
            } else {
                cx.b.mem.erase(Mem::Path);
            }
        }
        if !nav_some || self.last_target_pos.is_none() {
            return;
        }
        let Some(w) = cx.b.mem.walk_target() else { return };
        let Some(cur) = util::tracker_block(cx, &w.target) else { return };
        if util::dist_sqr_pos(cur, self.last_target_pos.unwrap()) > 4.0 && self.try_compute_path(cx, &w) {
            self.last_target_pos = util::tracker_block(cx, &w.target);
            self.start(cx);
        }
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- cooldowns

/// `CountDownCooldownTicks(memory)`: counts an integer memory down to zero, then erases it.
#[derive(Clone, Debug)]
pub struct CountDownCooldownTicks {
    pub mem: Mem,
}

impl CountDownCooldownTicks {
    pub fn new(mem: Mem) -> Box<dyn Control> {
        Timed::new(CountDownCooldownTicks { mem })
    }
}

impl Behavior for CountDownCooldownTicks {
    fn name(&self) -> &'static str {
        "CountDownCooldownTicks"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        // The entry is per instance; the static slice is only used for registration and checks,
        // so hand out a leaked, cached one.
        cooldown_entry(self.mem)
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.int(self.mem).is_some_and(|v| v > 0)
    }
    fn tick(&mut self, cx: &mut Cx) {
        if let Some(v) = cx.b.mem.int(self.mem) {
            cx.b.mem.set(self.mem, Val::Int(v - 1));
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(self.mem);
    }
    behavior_boilerplate!();
}

/// A `'static` one-entry condition list for `mem` present (leaked once per memory).
pub fn cooldown_entry(mem: Mem) -> &'static [(Mem, Status)] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<[(Mem, Status); 1]>> = OnceLock::new();
    let t = TABLE.get_or_init(|| Mem::ALL.iter().map(|&m| [(m, ValuePresent)]).collect());
    &t[mem as usize]
}

/// `DoNothing(min, max)`.
#[derive(Clone, Debug)]
pub struct DoNothing {
    min: i32,
    max: i32,
    running: bool,
    end: i64,
}

impl DoNothing {
    pub fn new(min: i32, max: i32) -> Box<dyn Control> {
        Box::new(DoNothing { min, max, running: false, end: 0 })
    }
}

impl Control for DoNothing {
    fn name(&self) -> &'static str {
        "DoNothing"
    }
    fn running(&self) -> bool {
        self.running
    }
    fn required(&self, _out: &mut Vec<Mem>) {}
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        self.running = true;
        let d = self.min + cx.rng().next_int_bounded(self.max + 1 - self.min);
        self.end = cx.time + d as i64;
        true
    }
    fn tick_or_stop(&mut self, cx: &mut Cx) {
        if cx.time > self.end {
            self.do_stop(cx);
        }
    }
    fn do_stop(&mut self, _cx: &mut Cx) {
        self.running = false;
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

// ---------------------------------------------------------------------------- looking around

/// `RandomLookAround(interval, maxYaw, minPitch, maxPitch)`.
#[derive(Clone, Debug)]
pub struct RandomLookAround {
    pub interval: (i32, i32),
    pub max_yaw: f32,
    pub min_pitch: f32,
    pub pitch_range: f32,
}

impl RandomLookAround {
    pub fn new(interval: (i32, i32), max_yaw: f32, min_pitch: f32, max_pitch: f32) -> Box<dyn Control> {
        Timed::new(RandomLookAround { interval, max_yaw, min_pitch, pitch_range: max_pitch - min_pitch })
    }
}

impl Behavior for RandomLookAround {
    fn name(&self) -> &'static str {
        "RandomLookAround"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, ValueAbsent), (Mem::GazeCooldownTicks, ValueAbsent)]
    }
    fn start(&mut self, cx: &mut Cx) {
        let r = &mut cx.e.random;
        let pitch = crate::mob::mth::clamp(r.next_float() * self.pitch_range + self.min_pitch, -90.0, 90.0);
        let yaw_off = 2.0 * r.next_float() * self.max_yaw;
        let yaw = crate::mob::mth::wrap_degrees(cx.e.y_rot + yaw_off - self.max_yaw);
        let dir = direction_from_rotation(pitch, yaw);
        let eye = Vec3::new(cx.e.x(), cx.e.eye_y(), cx.e.z());
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::vec(eye.add(dir.x, dir.y, dir.z))));
        let n = uniform(&mut cx.e.random, self.interval.0, self.interval.1);
        cx.b.mem.set(Mem::GazeCooldownTicks, Val::Int(n));
    }
    behavior_boilerplate!();
}

/// `Vec3.directionFromRotation(pitch, yaw)`.
pub fn direction_from_rotation(pitch: f32, yaw: f32) -> Vec3 {
    use crate::mob::mth::{cos, sin};
    let a = (-yaw) * 0.017453292f32 - 3.1415927f32;
    let b = (-pitch) * 0.017453292f32;
    let f = cos(a as f64);
    let f1 = sin(a as f64);
    let f2 = -cos(b as f64);
    let f3 = sin(b as f64);
    Vec3::new((f1 * f2) as f64, f3 as f64, (f * f2) as f64)
}

/// `SetEntityLookTarget.create(predicate, maxDist)`: the closest visible entity that passes
/// `pred` within `max_dist` becomes the look target.
pub fn set_entity_look_target(pred: fn(&Cx, i32) -> bool, max_dist: f32) -> Box<dyn Control> {
    let max_sqr = (max_dist * max_dist) as f64;
    shot("SetEntityLookTarget", &[(Mem::LookTarget, ValueAbsent), (Mem::NearestVisibleLivingEntities, ValuePresent)], move |cx| {
        let me = cx.e.id;
        let found = util::find_closest_visible(cx, |cx, id| {
            pred(cx, id) && util::living(cx, id).is_some_and(|l| cx.e.position().distance_to_sqr(l.pos) <= max_sqr) && !cx.e.passengers.contains(&id) && id != me
        });
        match found {
            Some(id) => {
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                true
            }
            None => false,
        }
    })
}

/// `SetEntityLookTargetSometimes.create(type?, maxDist, interval)`.
#[derive(Clone, Debug)]
pub struct SetEntityLookTargetSometimes {
    pub entity_type: Option<&'static str>,
    pub max_sqr: f32,
    pub interval: (i32, i32),
    /// `Ticker.ticksUntilNextStart`.
    ticks: i32,
}

impl SetEntityLookTargetSometimes {
    pub fn new(entity_type: Option<&'static str>, max_dist: f32, interval: (i32, i32)) -> Box<dyn Control> {
        Shot::new(SetEntityLookTargetSometimes { entity_type, max_sqr: max_dist * max_dist, interval, ticks: 0 })
    }
}

impl ShotBehavior for SetEntityLookTargetSometimes {
    fn name(&self) -> &'static str {
        "SetEntityLookTargetSometimes"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, ValueAbsent), (Mem::NearestVisibleLivingEntities, ValuePresent)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let (ty, max_sqr) = (self.entity_type, self.max_sqr as f64);
        let found = util::find_closest_visible(cx, |cx, id| {
            let Some(l) = util::living(cx, id) else { return false };
            ty.is_none_or(|t| l.type_name == t) && cx.e.position().distance_to_sqr(l.pos) <= max_sqr
        });
        let Some(found) = found else { return false };
        // `Ticker.tickDownAndCheck`.
        let fire = if self.ticks == 0 {
            // `tickDownAndCheck(level.getRandom())`.
            self.ticks = uniform(cx.rng(), self.interval.0, self.interval.1) - 1;
            false
        } else {
            self.ticks -= 1;
            self.ticks == 0
        };
        if !fire {
            return false;
        }
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(found, true)));
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `SetWalkTargetFromLookTarget.create(speed, closeEnough)`.
pub fn set_walk_target_from_look_target(speed: f32, close_enough: i32) -> Box<dyn Control> {
    shot("SetWalkTargetFromLookTarget", &[(Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, ValuePresent)], move |cx| {
        let Some(t) = cx.b.mem.look_target() else { return false };
        cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed, close_enough }));
        true
    })
}

// ---------------------------------------------------------------------------- strolling

/// Where `RandomStroll` looks for its next spot.
#[derive(Clone, Copy, Debug)]
pub enum StrollKind {
    /// `stroll(speed, avoidWater)`: `LandRandomPos.getPos(mob, 10, 7)`, never from the water when
    /// `avoid_water`.
    Land { avoid_water: bool },
    /// `stroll(speed, h, v)`.
    LandRange { h: i32, v: i32 },
    /// `fly(speed)`.
    Fly,
    /// `swim(speed)`: only in water.
    Swim,
}

/// `RandomStroll`: sets `WALK_TARGET` to a random spot.
pub fn stroll(speed: f32, kind: StrollKind) -> Box<dyn Control> {
    shot("RandomStroll", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        let ok = match kind {
            StrollKind::Land { avoid_water } => !avoid_water || !cx.e.is_in_water(),
            StrollKind::Swim => cx.e.is_in_water(),
            _ => true,
        };
        if !ok {
            return false;
        }
        let pos = match kind {
            StrollKind::Land { .. } => random_pos::land_pos(cx.e, cx.m, &*cx.level, 10, 7),
            StrollKind::LandRange { h, v } => random_pos::land_pos(cx.e, cx.m, &*cx.level, h, v),
            StrollKind::Fly => {
                let view = view_vector(cx.e.x_rot, cx.e.y_rot);
                random_pos::air_and_water_pos(cx.e, cx.m, &*cx.level, 10, 7, -2, view.x, view.z, 1.5707963705062866)
            }
            StrollKind::Swim => target_swim_pos(cx),
        };
        match pos {
            Some(p) => cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, speed, 0))),
            None => cx.b.mem.erase(Mem::WalkTarget),
        }
        true
    })
}

/// `RandomStroll.getTargetSwimPos`: a swimmable spot at growing distances, each next tier
/// pushed on along the way to the last (`SWIM_XY_DISTANCE_TIERS`), stopping where the water ends.
pub fn target_swim_pos(cx: &mut Cx) -> Option<Vec3> {
    const TIERS: [(i32, i32); 6] = [(1, 1), (3, 3), (5, 5), (6, 5), (7, 7), (10, 7)];
    let mut previous: Option<Vec3> = None;
    let mut pos: Option<Vec3> = None;
    for (h, v) in TIERS {
        pos = match previous {
            None => crate::mob::path::random_swimmable_pos(cx.e, cx.m, &*cx.level, h, v),
            Some(prev) => {
                let here = cx.e.position();
                Some(here + (prev - here).normalize().multiply(h as f64, v as f64, h as f64))
            }
        };
        // (`mobRestricted` is false for a mob without a home.)
        match pos {
            Some(p) if !crate::physics::fluid_state(cx.level.block(BlockPos::containing(p.x, p.y, p.z))).is_empty() => {}
            _ => return previous,
        }
        previous = pos;
    }
    pos
}

/// `Entity.getViewVector(0)`.
pub fn view_vector(x_rot: f32, y_rot: f32) -> Vec3 {
    crate::ext_entity::fireball::view_vector(x_rot, y_rot)
}

// ---------------------------------------------------------------------------- temptation and animals

/// `FollowTemptation(speed, closeEnoughDistance, lookInTheEyes)`.
#[derive(Clone, Debug)]
pub struct FollowTemptation {
    pub speed: fn(&Cx) -> f32,
    pub close_enough: fn(&Cx) -> f64,
    pub look_in_the_eyes: bool,
}

impl FollowTemptation {
    /// `new FollowTemptation(speed)` (close enough: 2.5).
    pub fn new(speed: fn(&Cx) -> f32) -> Box<dyn Control> {
        FollowTemptation::with(speed, |_| 2.5, false)
    }

    pub fn with(speed: fn(&Cx) -> f32, close_enough: fn(&Cx) -> f64, look_in_the_eyes: bool) -> Box<dyn Control> {
        Timed::new(FollowTemptation { speed, close_enough, look_in_the_eyes })
    }
}

impl Behavior for FollowTemptation {
    fn name(&self) -> &'static str {
        "FollowTemptation"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::LookTarget, Registered),
            (Mem::WalkTarget, Registered),
            (Mem::TemptationCooldownTicks, ValueAbsent),
            (Mem::TemptingPlayer, ValuePresent),
            (Mem::BreedTarget, ValueAbsent),
            (Mem::IsPanicking, ValueAbsent),
        ]
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::TemptingPlayer) && !cx.b.mem.has(Mem::BreedTarget) && !cx.b.mem.has(Mem::IsPanicking)
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.set(Mem::TemptationCooldownTicks, Val::Int(100));
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::LookTarget);
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(id) = cx.b.mem.entity(Mem::TemptingPlayer) else { return };
        let Some(p) = util::living(cx, id) else { return };
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
        let d = (self.close_enough)(cx);
        if cx.e.position().distance_to_sqr(p.pos) < d * d {
            cx.b.mem.erase(Mem::WalkTarget);
        } else {
            let t = Tracker::entity3(id, self.look_in_the_eyes, self.look_in_the_eyes);
            let speed = (self.speed)(cx);
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed, close_enough: 2 }));
        }
    }
    behavior_boilerplate!();
}

/// `BabyFollowAdult.create(range, speed)`: a baby walks toward the adult in `adult_mem`.
pub fn baby_follow_adult(range: (i32, i32), speed: fn(&Cx) -> f32, adult_mem: Mem, look_in_the_eyes: bool) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match adult_mem {
        Mem::NearestVisibleAdult => &[(Mem::NearestVisibleAdult, ValuePresent), (Mem::LookTarget, Registered), (Mem::WalkTarget, ValueAbsent)],
        _ => panic!("BabyFollowAdult over {adult_mem:?}: add its entry conditions"),
    };
    shot("BabyFollowAdult", entry, move |cx| {
        if !cx.m.baby() {
            return false;
        }
        let Some(adult) = cx.b.mem.entity(adult_mem) else { return false };
        let Some(a) = util::living(cx, adult) else { return false };
        let d = cx.e.position().distance_to_sqr(a.pos);
        let max = (range.1 + 1) as f64;
        let min = range.0 as f64;
        if d < max * max && !(d < min * min) {
            let w = WalkTarget { target: Tracker::entity3(adult, look_in_the_eyes, look_in_the_eyes), speed: speed(cx), close_enough: range.0 - 1 };
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity3(adult, true, look_in_the_eyes)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(w));
            return true;
        }
        false
    })
}

/// `AnimalMakeLove(partnerType, speed, closeEnough)`.
#[derive(Clone, Debug)]
pub struct AnimalMakeLove {
    pub partner_type: &'static str,
    pub speed: f32,
    pub close_enough: i32,
    spawn_child_at: i64,
}

impl AnimalMakeLove {
    pub fn new(partner_type: &'static str, speed: f32, close_enough: i32) -> Box<dyn Control> {
        Timed::new(AnimalMakeLove { partner_type, speed, close_enough, spawn_child_at: 0 })
    }

    fn valid_partner(&self, cx: &mut Cx) -> Option<i32> {
        let ty = self.partner_type;
        util::find_closest_visible(cx, |cx, id| {
            let Some(o) = cx.level.entity(id) else { return false };
            if o.type_name != ty {
                return false;
            }
            let Some(om) = crate::mob::data(o) else { return false };
            can_mate(cx, om) && !om_panicking(om)
        })
    }
}

fn om_panicking(m: &crate::mob::MobData) -> bool {
    m.brain.as_ref().is_some_and(|b| b.st.mem.has(Mem::IsPanicking))
}

/// `Animal.canMate(other)` (both in love, same type, not itself).
pub fn can_mate(cx: &Cx, other: &crate::mob::MobData) -> bool {
    cx.m.kind == other.kind && cx.m.in_love > 0 && other.in_love > 0 && cx.m.kind.ext().is_none_or(|k| k.can_mate(cx.m, other))
}

impl Behavior for AnimalMakeLove {
    fn name(&self) -> &'static str {
        "AnimalMakeLove"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::NearestVisibleLivingEntities, ValuePresent),
            (Mem::BreedTarget, ValueAbsent),
            (Mem::WalkTarget, Registered),
            (Mem::LookTarget, Registered),
            (Mem::IsPanicking, ValueAbsent),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        (110, 110)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.m.in_love > 0 && self.valid_partner(cx).is_some()
    }
    fn start(&mut self, cx: &mut Cx) {
        let Some(p) = self.valid_partner(cx) else { return };
        cx.b.mem.set(Mem::BreedTarget, Val::Entity(p));
        // The partner's brain (`p.getBrain().setMemory(BREED_TARGET, this)`).
        let me = cx.e.id;
        if let Some(pe) = cx.level.entity_mut(p)
            && let Some(pm) = crate::mob::data_mut(pe)
            && let Some(b) = pm.brain.as_mut()
        {
            b.st.mem.set(Mem::BreedTarget, Val::Entity(me));
        }
        lock_gaze_and_walk_to_each_other(cx, p, self.speed, self.close_enough);
        let d = 60 + cx.e.random.next_int_bounded(50);
        self.spawn_child_at = cx.time + d as i64;
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let Some(t) = cx.b.mem.entity(Mem::BreedTarget) else { return false };
        let Some(o) = cx.level.entity(t) else { return false };
        if o.type_name != self.partner_type {
            return false;
        }
        let Some(om) = crate::mob::data(o) else { return false };
        let (alive, mate, panicking) = (o.is_alive(), can_mate(cx, om), om_panicking(om));
        alive && mate && util::entity_is_visible(cx, t) && cx.time <= self.spawn_child_at && !cx.b.mem.has(Mem::IsPanicking) && !panicking
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = cx.b.mem.entity(Mem::BreedTarget) else { return };
        lock_gaze_and_walk_to_each_other(cx, t, self.speed, self.close_enough);
        let Some(o) = util::living(cx, t) else { return };
        if cx.e.position().distance_to_sqr(o.pos) >= 9.0 {
            return;
        }
        if cx.time >= self.spawn_child_at {
            crate::mob::breed::spawn_child(cx.e, cx.m, cx.level, t);
            // `Frog.spawnChildFromBreeding`: the mother is pregnant.
            if cx.m.kind.ext().is_some_and(|k| k.breed_as_pregnancy()) {
                cx.b.mem.set(Mem::IsPregnant, Val::Unit);
            }
            cx.b.mem.erase(Mem::BreedTarget);
            if let Some(pe) = cx.level.entity_mut(t)
                && let Some(pm) = crate::mob::data_mut(pe)
                && let Some(b) = pm.brain.as_mut()
            {
                b.st.mem.erase(Mem::BreedTarget);
            }
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::BreedTarget);
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::LookTarget);
        self.spawn_child_at = 0;
    }
    behavior_boilerplate!();
}

/// `BehaviorUtils.lockGazeAndWalkToEachOther(this, other, speed, closeEnough)`: both look at each
/// other and walk to each other.
pub fn lock_gaze_and_walk_to_each_other(cx: &mut Cx, other: i32, speed: f32, close_enough: i32) {
    let me = cx.e.id;
    util::look_at_entity(cx, other);
    util::set_walk_and_look(cx, Tracker::entity(other, true), speed, close_enough);
    if let Some(pe) = cx.level.entity_mut(other)
        && let Some(pm) = crate::mob::data_mut(pe)
        && let Some(b) = pm.brain.as_mut()
    {
        b.st.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(me, true)));
        let t = Tracker::entity(me, true);
        b.st.mem.set(Mem::LookTarget, Val::Look(t));
        b.st.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed, close_enough }));
    }
}

/// `AnimalPanic(speed, damage types, position getter)`.
#[derive(Clone, Debug)]
pub struct AnimalPanic {
    pub speed: f32,
    /// The damage type tag that makes the mob panic.
    pub causes: &'static str,
    /// Flying/swimming panickers pick their spot in the air (`AirAndWaterRandomPos`, flying height).
    pub air: Option<i32>,
}

impl AnimalPanic {
    pub fn new(speed: f32) -> Box<dyn Control> {
        Timed::new(AnimalPanic { speed, causes: "minecraft:panic_causes", air: None })
    }

    pub fn with(speed: f32, causes: &'static str, air: Option<i32>) -> Box<dyn Control> {
        Timed::new(AnimalPanic { speed, causes, air })
    }

    fn panic_pos(&self, cx: &mut Cx) -> Option<Vec3> {
        // A burning mob looks for water first (`lookForWater`: within 5 blocks, one up and down).
        if cx.e.is_on_fire()
            && let Some(p) = crate::mob::kinds::turtle::look_for_water(cx.e, &*cx.level, 5)
        {
            return Some(Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5));
        }
        match self.air {
            None => random_pos::land_pos(cx.e, cx.m, &*cx.level, 5, 4),
            Some(h) => {
                let view = view_vector(cx.e.x_rot, cx.e.y_rot);
                random_pos::air_and_water_pos(cx.e, cx.m, &*cx.level, 5, 4, h, view.x, view.z, 1.5707963705062866)
            }
        }
    }
}

impl Behavior for AnimalPanic {
    fn name(&self) -> &'static str {
        "AnimalPanic"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::IsPanicking, Registered), (Mem::HurtBy, Registered)]
    }
    fn duration(&self) -> (i32, i32) {
        (100, 120)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.damage(Mem::HurtBy).is_some_and(|d| d.kind.is_tag(self.causes)) || cx.b.mem.has(Mem::IsPanicking)
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.set(Mem::IsPanicking, Val::Bool(true));
        cx.b.mem.erase(Mem::WalkTarget);
        cx.m.nav.stop();
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::IsPanicking);
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.m.nav.is_done()
            && let Some(p) = self.panic_pos(cx)
        {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, self.speed, 0)));
        }
    }
    behavior_boilerplate!();
}

/// A block position as a walk target with a close-enough distance.
pub fn walk_to_block(pos: BlockPos, speed: f32, close_enough: i32) -> WalkTarget {
    WalkTarget::block(pos, speed, close_enough)
}
