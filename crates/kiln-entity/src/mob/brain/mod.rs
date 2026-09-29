//! Vanilla's `Brain`: memories with expiry, sensors that scan on their own interval, activities
//! (sets of behaviours by priority) chosen by requirements or a schedule, and behaviours that
//! start, tick and stop on vanilla's timing, with the random draws in vanilla's order.
//!
//! The mob's brain lives in [`MobData::brain`]; while it ticks it is taken out of the mob
//! (like the goal selectors), so behaviours borrow the entity, the mob's data, the level and the
//! brain's state ([`Cx`]) at once. Behaviours come in two styles, as in vanilla:
//!
//! * [`Behavior`]: entry conditions, a random duration, `start` / `tick` / `stop`
//!   (`net.minecraft.world.entity.ai.behavior.Behavior`), wrapped by [`Timed`];
//! * [`Shot`]: declarative one-shots (`BehaviorBuilder.create`): conditions, then an action that
//!   never becomes "running" ([`shot`] builds one from a closure).
//!
//! [`Gate`] is `GateBehavior` (`RunOne`, `TriggerGate`). Randomness: vanilla draws from the
//! level's random in `Behavior.tryStart` and most behaviours; Kiln uses the mob's own
//! `brain_random` for those (so the outcome does not depend on which entities share a region),
//! and `Entity.random` where vanilla uses the entity's random.

pub mod behaviors;
pub mod gate;
pub mod memory;
pub mod sensors;
pub mod util;

pub use gate::{Gate, OrderPolicy, RunningPolicy, TriggerGate};
pub use memory::{GlobalPos, Mem, Memories, NearestVisible, Slot, Status, Tracker, Val, WalkTarget};

use super::MobData;
use crate::entity::Entity;
use crate::level::EntityLevel;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::fmt::Debug;
use std::sync::Arc;

/// `Activity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Activity {
    Core,
    Idle,
    Work,
    Play,
    Rest,
    Meet,
    Panic,
    Raid,
    PreRaid,
    Hide,
    Fight,
    Celebrate,
    AdmireItem,
    Avoid,
    Ride,
    PlayDead,
    LongJump,
    Ram,
    Tongue,
    Swim,
    LaySpawn,
    Sniff,
    Investigate,
    Roar,
    Emerge,
    Dig,
}

impl Activity {
    pub const COUNT: usize = 26;

    pub fn name(self) -> &'static str {
        match self {
            Activity::Core => "core",
            Activity::Idle => "idle",
            Activity::Work => "work",
            Activity::Play => "play",
            Activity::Rest => "rest",
            Activity::Meet => "meet",
            Activity::Panic => "panic",
            Activity::Raid => "raid",
            Activity::PreRaid => "pre_raid",
            Activity::Hide => "hide",
            Activity::Fight => "fight",
            Activity::Celebrate => "celebrate",
            Activity::AdmireItem => "admire_item",
            Activity::Avoid => "avoid",
            Activity::Ride => "ride",
            Activity::PlayDead => "play_dead",
            Activity::LongJump => "long_jump",
            Activity::Ram => "ram",
            Activity::Tongue => "tongue",
            Activity::Swim => "swim",
            Activity::LaySpawn => "lay_spawn",
            Activity::Sniff => "sniff",
            Activity::Investigate => "investigate",
            Activity::Roar => "roar",
            Activity::Emerge => "emerge",
            Activity::Dig => "dig",
        }
    }

    pub fn by_name(name: &str) -> Option<Activity> {
        (0..Activity::COUNT as u8).map(|i| ACTIVITIES[i as usize]).find(|a| a.name() == name)
    }

    /// `String.hashCode` of the name: `Activity.hashCode`.
    pub fn java_hash(self) -> i32 {
        self.name().bytes().fold(0i32, |h, b| h.wrapping_mul(31).wrapping_add(b as i32))
    }

    /// The bucket of vanilla's `HashMap<Activity, ...>` (16 buckets) the activity iterates in.
    fn bucket(self) -> i32 {
        let h = self.java_hash();
        (h ^ ((h as u32) >> 16) as i32) & 15
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

const ACTIVITIES: [Activity; Activity::COUNT] = [
    Activity::Core,
    Activity::Idle,
    Activity::Work,
    Activity::Play,
    Activity::Rest,
    Activity::Meet,
    Activity::Panic,
    Activity::Raid,
    Activity::PreRaid,
    Activity::Hide,
    Activity::Fight,
    Activity::Celebrate,
    Activity::AdmireItem,
    Activity::Avoid,
    Activity::Ride,
    Activity::PlayDead,
    Activity::LongJump,
    Activity::Ram,
    Activity::Tongue,
    Activity::Swim,
    Activity::LaySpawn,
    Activity::Sniff,
    Activity::Investigate,
    Activity::Roar,
    Activity::Emerge,
    Activity::Dig,
];

/// What behaviours and sensors work on: the mob, its data, the level and the brain's state.
pub struct Cx<'a> {
    pub e: &'a mut Entity,
    pub m: &'a mut MobData,
    pub level: &'a mut dyn EntityLevel,
    pub b: &'a mut BrainState,
    /// `level.getGameTime()`.
    pub time: i64,
}

impl Cx<'_> {
    /// The random vanilla draws from `level.getRandom()` (the mob's own stream, see the module doc).
    pub fn rng(&mut self) -> &mut LegacyRandom {
        match self.level.shared_ai_random() {
            Some(r) => r,
            None => &mut self.m.brain_random,
        }
    }

    /// Shorthand for the memories.
    pub fn mem(&self) -> &Memories {
        &self.b.mem
    }
}

/// `BehaviorControl`: what the brain runs. `Timed` and `Shot` adapt the two behaviour styles; a
/// `Gate` holds more of them.
pub trait Control: Debug + Send + Sync {
    fn name(&self) -> &'static str;
    /// `getStatus() == RUNNING`.
    fn running(&self) -> bool;
    /// `getRequiredMemories` (registered with the brain).
    fn required(&self, out: &mut Vec<Mem>);
    fn try_start(&mut self, cx: &mut Cx) -> bool;
    fn tick_or_stop(&mut self, cx: &mut Cx);
    fn do_stop(&mut self, cx: &mut Cx);
    /// Names of what runs now (for traces): a gate lists its running children.
    fn running_names(&self, out: &mut Vec<String>) {
        if self.running() {
            out.push(self.name().to_owned());
        }
    }
    /// Seeds the shuffles of the gates at and below this behaviour (pre-order): gate `k` gets
    /// `LegacyRandom(base + k)`.
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        let _ = (base, k);
    }
    fn box_clone(&self) -> Box<dyn Control>;
}

impl Clone for Box<dyn Control> {
    fn clone(&self) -> Self {
        (**self).box_clone()
    }
}

/// `Behavior`: the timed style.
pub trait Behavior: Debug + Send + Sync {
    /// The vanilla class's simple name (traces compare it).
    fn name(&self) -> &'static str;
    /// `entryCondition`.
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[]
    }
    /// (`minDuration`, `maxDuration`): `Behavior(entry)` is 60, 60.
    fn duration(&self) -> (i32, i32) {
        (60, 60)
    }
    /// `checkExtraStartConditions`.
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        let _ = cx;
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        let _ = cx;
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let _ = cx;
        false
    }
    fn tick(&mut self, cx: &mut Cx) {
        let _ = cx;
    }
    fn stop(&mut self, cx: &mut Cx) {
        let _ = cx;
    }
    /// `timedOut(gameTime)` (`gameTime > endTimestamp`).
    fn timed_out(&self, time: i64, end: i64) -> bool {
        time > end
    }
    fn box_clone(&self) -> Box<dyn Behavior>;
}

impl Clone for Box<dyn Behavior> {
    fn clone(&self) -> Self {
        (**self).box_clone()
    }
}

/// Implements `Behavior::box_clone` for a `Clone` behaviour.
#[macro_export]
macro_rules! behavior_boilerplate {
    () => {
        fn box_clone(&self) -> Box<dyn $crate::mob::brain::Behavior> {
            Box::new(self.clone())
        }
    };
}

/// `Behavior` with its status and end time.
#[derive(Clone, Debug)]
pub struct Timed {
    pub b: Box<dyn Behavior>,
    running: bool,
    end: i64,
}

impl Timed {
    pub fn new(b: impl Behavior + 'static) -> Box<dyn Control> {
        Box::new(Timed { b: Box::new(b), running: false, end: 0 })
    }
}

fn has_required(entry: &[(Mem, Status)], mem: &Memories) -> bool {
    entry.iter().all(|&(m, s)| mem.check(m, s))
}

impl Control for Timed {
    fn name(&self) -> &'static str {
        self.b.name()
    }
    fn running(&self) -> bool {
        self.running
    }
    fn required(&self, out: &mut Vec<Mem>) {
        out.extend(self.b.entry().iter().map(|&(m, _)| m));
    }
    /// `Behavior.tryStart`.
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        if has_required(self.b.entry(), &cx.b.mem) && self.b.check_extra_start(cx) {
            self.running = true;
            let (min, max) = self.b.duration();
            let d = min + cx.rng().next_int_bounded(max + 1 - min);
            self.end = cx.time + d as i64;
            self.b.start(cx);
            true
        } else {
            false
        }
    }
    /// `Behavior.tickOrStop`.
    fn tick_or_stop(&mut self, cx: &mut Cx) {
        if !self.b.timed_out(cx.time, self.end) && self.b.can_still_use(cx) {
            self.b.tick(cx);
        } else {
            self.do_stop(cx);
        }
    }
    fn do_stop(&mut self, cx: &mut Cx) {
        self.running = false;
        self.b.stop(cx);
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// `OneShot` (declarative behaviours): the trigger checks its conditions and acts; the status
/// stays `STOPPED`.
pub trait ShotBehavior: Debug + Send + Sync {
    fn name(&self) -> &'static str;
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[]
    }
    /// The trigger after the entry conditions held: true when it acted.
    fn trigger(&mut self, cx: &mut Cx) -> bool;
    fn box_clone(&self) -> Box<dyn ShotBehavior>;
}

impl Clone for Box<dyn ShotBehavior> {
    fn clone(&self) -> Self {
        (**self).box_clone()
    }
}

#[derive(Clone, Debug)]
pub struct Shot(pub Box<dyn ShotBehavior>);

impl Shot {
    pub fn new(b: impl ShotBehavior + 'static) -> Box<dyn Control> {
        Box::new(Shot(Box::new(b)))
    }
}

impl Control for Shot {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn running(&self) -> bool {
        false
    }
    fn required(&self, out: &mut Vec<Mem>) {
        out.extend(self.0.entry().iter().map(|&(m, _)| m));
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        has_required(self.0.entry(), &cx.b.mem) && self.0.trigger(cx)
    }
    fn tick_or_stop(&mut self, _cx: &mut Cx) {}
    fn do_stop(&mut self, _cx: &mut Cx) {}
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// A one-shot from a closure: `shot("SetEntityLookTarget", ENTRY, move |cx| ...)`.
pub type ShotFn = Arc<dyn Fn(&mut Cx) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct FnShot {
    name: &'static str,
    entry: &'static [(Mem, Status)],
    f: ShotFn,
}

impl Debug for FnShot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Shot({})", self.name)
    }
}

impl ShotBehavior for FnShot {
    fn name(&self) -> &'static str {
        self.name
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        self.entry
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        (self.f)(cx)
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

pub fn shot(name: &'static str, entry: &'static [(Mem, Status)], f: impl Fn(&mut Cx) -> bool + Send + Sync + 'static) -> Box<dyn Control> {
    Shot::new(FnShot { name, entry, f: Arc::new(f) })
}

/// `Sensor`.
pub trait Sensor: Debug + Send + Sync {
    fn name(&self) -> &'static str;
    /// The scan interval (`Sensor(int)`; 20 by default).
    fn scan_rate(&self) -> i32 {
        20
    }
    /// `requires` (registered with the brain).
    fn requires(&self) -> &'static [Mem];
    fn do_tick(&mut self, cx: &mut Cx);
    fn box_clone(&self) -> Box<dyn Sensor>;
}

impl Clone for Box<dyn Sensor> {
    fn clone(&self) -> Self {
        (**self).box_clone()
    }
}

#[macro_export]
macro_rules! sensor_boilerplate {
    () => {
        fn box_clone(&self) -> Box<dyn $crate::mob::brain::Sensor> {
            Box::new(self.clone())
        }
    };
}

#[derive(Clone, Debug)]
struct SensorSlot {
    s: Box<dyn Sensor>,
    time_to_tick: i64,
}

/// `ActivityData`.
pub struct ActivityData {
    pub activity: Activity,
    /// (priority, behaviour) in registration order.
    pub behaviors: Vec<(i32, Box<dyn Control>)>,
    pub conditions: Vec<(Mem, Status)>,
    pub erase_when_stopped: Vec<Mem>,
}

impl ActivityData {
    /// `ActivityData.create(activity, startPriority, behaviours)`: consecutive priorities.
    pub fn create(activity: Activity, start: i32, behaviors: Vec<Box<dyn Control>>) -> ActivityData {
        ActivityData {
            activity,
            behaviors: behaviors.into_iter().enumerate().map(|(i, b)| (start + i as i32, b)).collect(),
            conditions: Vec::new(),
            erase_when_stopped: Vec::new(),
        }
    }

    /// `ActivityData.create(activity, pairs)`.
    pub fn with_priorities(activity: Activity, behaviors: Vec<(i32, Box<dyn Control>)>) -> ActivityData {
        ActivityData { activity, behaviors, conditions: Vec::new(), erase_when_stopped: Vec::new() }
    }

    /// `ActivityData.create(activity, pairs, conditions)`.
    pub fn with_conditions(activity: Activity, behaviors: Vec<(i32, Box<dyn Control>)>, conditions: &[(Mem, Status)]) -> ActivityData {
        ActivityData { activity, behaviors, conditions: conditions.to_vec(), erase_when_stopped: Vec::new() }
    }

    /// `ActivityData.create(activity, pairs, conditions, memoriesToEraseWhenStopped)`.
    pub fn full(activity: Activity, behaviors: Vec<(i32, Box<dyn Control>)>, conditions: &[(Mem, Status)], erase: &[Mem]) -> ActivityData {
        ActivityData { activity, behaviors, conditions: conditions.to_vec(), erase_when_stopped: erase.to_vec() }
    }
}

/// A schedule: the activity for a position and time (`EnvironmentAttribute<Activity>` read from
/// the dimension's timeline: the villagers' day).
pub type Schedule = fn(&dyn EntityLevel) -> Activity;

/// Everything but the behaviours and sensors: what behaviours and sensors read and change.
#[derive(Clone, Debug)]
pub struct BrainState {
    pub mem: Memories,
    active: u32,
    core: u32,
    default_activity: Activity,
    last_schedule_update: i64,
    pub schedule: Option<Schedule>,
    requirements: Vec<Vec<(Mem, Status)>>,
    has_requirements: u32,
    erase_when_stopped: Vec<Vec<Mem>>,
}

impl BrainState {
    /// `Brain.isActive`.
    pub fn is_active(&self, a: Activity) -> bool {
        self.active & a.bit() != 0
    }

    /// `Brain.getActiveActivities`, in enum order.
    pub fn active_activities(&self) -> Vec<Activity> {
        ACTIVITIES.iter().copied().filter(|&a| self.is_active(a)).collect()
    }

    /// `Brain.getActiveNonCoreActivity`.
    pub fn active_non_core(&self) -> Option<Activity> {
        ACTIVITIES.iter().copied().find(|&a| self.is_active(a) && self.core & a.bit() == 0)
    }

    pub fn set_core_activities(&mut self, acts: &[Activity]) {
        self.core = acts.iter().fold(0, |b, a| b | a.bit());
    }

    pub fn set_default_activity(&mut self, a: Activity) {
        self.default_activity = a;
    }

    /// `Brain.useDefaultActivity`.
    pub fn use_default_activity(&mut self) {
        let a = self.default_activity;
        self.set_active_activity(a);
    }

    /// `Brain.setActiveActivityIfPossible`.
    pub fn set_active_activity_if_possible(&mut self, a: Activity) {
        if self.requirements_met(a) {
            self.set_active_activity(a);
        } else {
            self.use_default_activity();
        }
    }

    /// `Brain.setActiveActivityToFirstValid`.
    pub fn set_active_activity_to_first_valid(&mut self, acts: &[Activity]) {
        for &a in acts {
            if self.requirements_met(a) {
                self.set_active_activity(a);
                break;
            }
        }
    }

    fn requirements_met(&self, a: Activity) -> bool {
        if self.has_requirements & a.bit() == 0 {
            return false;
        }
        self.requirements[a as usize].iter().all(|&(m, s)| self.mem.check(m, s))
    }

    fn set_active_activity(&mut self, a: Activity) {
        if self.is_active(a) {
            return;
        }
        // `eraseMemoriesForOtherActivitesThan`.
        for other in ACTIVITIES {
            if self.is_active(other) && other != a {
                for i in 0..self.erase_when_stopped[other as usize].len() {
                    let m = self.erase_when_stopped[other as usize][i];
                    self.mem.erase(m);
                }
            }
        }
        self.active = self.core | a.bit();
    }

    /// `Brain.updateActivityFromSchedule`: at most every 20 ticks.
    pub fn update_activity_from_schedule(&mut self, time: i64, level: &dyn EntityLevel) {
        if time - self.last_schedule_update > 20 {
            self.last_schedule_update = time;
            let a = match self.schedule {
                Some(s) => s(level),
                None => Activity::Idle,
            };
            if !self.is_active(a) {
                self.set_active_activity_if_possible(a);
            }
        }
    }
}

/// The behaviours of one activity at one priority.
#[derive(Clone, Debug)]
struct Group {
    priority: i32,
    /// Activities in the iteration order of vanilla's `HashMap`, each with its behaviours in
    /// registration order.
    activities: Vec<(Activity, Vec<Box<dyn Control>>)>,
}

/// `Brain`.
#[derive(Clone, Debug)]
pub struct Brain {
    pub st: BrainState,
    sensors: Vec<SensorSlot>,
    groups: Vec<Group>,
}

impl Brain {
    /// `new Brain(memoryTypes, sensorTypes, activities, memories, random)`: memories and
    /// sensors registered, each sensor's first scan delayed by a draw from `random`, the activities
    /// added, `CORE` the core activity and the default (`IDLE`) the active one.
    pub fn new(memories: &[Mem], sensors: Vec<Box<dyn Sensor>>, activities: Vec<ActivityData>, random: &mut dyn RandomSource) -> Brain {
        let mut mem = Memories::default();
        for &m in memories {
            mem.register(m);
        }
        let mut slots = Vec::new();
        for s in sensors {
            for &m in s.requires() {
                mem.register(m);
            }
            let t = random.next_int_bounded(s.scan_rate()) as i64;
            slots.push(SensorSlot { s, time_to_tick: t });
        }
        let mut st = BrainState {
            mem,
            active: 0,
            core: 0,
            default_activity: Activity::Idle,
            last_schedule_update: -9999,
            schedule: None,
            requirements: vec![Vec::new(); Activity::COUNT],
            has_requirements: 0,
            erase_when_stopped: vec![Vec::new(); Activity::COUNT],
        };
        let mut brain = Brain { st: st.clone(), sensors: slots, groups: Vec::new() };
        // Registration order of (priority, activity): the first sighting of an activity in a
        // priority decides where it goes among those sharing a hash bucket.
        let mut firsts: Vec<(i32, Activity)> = Vec::new();
        for a in &activities {
            st.requirements[a.activity as usize] = a.conditions.clone();
            st.has_requirements |= a.activity.bit();
            if !a.erase_when_stopped.is_empty() {
                st.erase_when_stopped[a.activity as usize] = a.erase_when_stopped.clone();
            }
        }
        for a in activities {
            for (prio, b) in a.behaviors {
                let mut req = Vec::new();
                b.required(&mut req);
                for m in req {
                    st.mem.register(m);
                }
                if !firsts.contains(&(prio, a.activity)) {
                    firsts.push((prio, a.activity));
                }
                let gi = match brain.groups.iter().position(|g| g.priority == prio) {
                    Some(i) => i,
                    None => {
                        brain.groups.push(Group { priority: prio, activities: Vec::new() });
                        brain.groups.len() - 1
                    }
                };
                let g = &mut brain.groups[gi];
                match g.activities.iter_mut().find(|(x, _)| *x == a.activity) {
                    Some((_, v)) => v.push(b),
                    None => g.activities.push((a.activity, vec![b])),
                }
            }
        }
        brain.groups.sort_by_key(|g| g.priority);
        for g in brain.groups.iter_mut() {
            // Stable: same bucket keeps registration order (`HashMap` chains append).
            g.activities.sort_by_key(|(a, _)| a.bucket());
        }
        st.set_core_activities(&[Activity::Core]);
        st.use_default_activity();
        brain.st = st;
        // The shuffles of the gates draw from randoms of their own (vanilla: unseeded ones).
        let base = random.next_long();
        brain.seed_gates(base);
        brain
    }

    /// `Brain.tick`.
    pub fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let time = level.game_time();
        let mut cx = Cx { e, m, level, b: &mut self.st, time };
        if debug_on() {
            eprintln!("brain t={time} tick begins rnd {}", cx.e.random.state());
        }
        cx.b.mem.tick();
        for s in self.sensors.iter_mut() {
            s.time_to_tick -= 1;
            if s.time_to_tick <= 0 {
                s.time_to_tick = s.s.scan_rate() as i64;
                s.s.do_tick(&mut cx);
            }
        }
        // startEachNonRunningBehavior
        for g in self.groups.iter_mut() {
            for (act, bs) in g.activities.iter_mut() {
                if cx.b.is_active(*act) {
                    for b in bs.iter_mut() {
                        if !b.running() {
                            let (r0, l0) = (cx.e.random.state(), cx.rng().state());
                            if b.try_start(&mut cx) && debug_on() {
                                let (r1, l1, t) = (cx.e.random.state(), cx.rng().state(), cx.time);
                                eprintln!("brain t={t} start {} rnd {r0}->{r1} lr {l0}->{l1}", b.name());
                            }
                        }
                    }
                }
            }
        }
        // tickEachRunningBehavior: the running ones as of now (behaviours started above included).
        for g in self.groups.iter_mut() {
            for (_, bs) in g.activities.iter_mut() {
                for b in bs.iter_mut() {
                    if b.running() {
                        b.tick_or_stop(&mut cx);
                    }
                }
            }
        }
    }

    /// `Brain.stopAll`.
    pub fn stop_all(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let time = level.game_time();
        let mut cx = Cx { e, m, level, b: &mut self.st, time };
        for g in self.groups.iter_mut() {
            for (_, bs) in g.activities.iter_mut() {
                for b in bs.iter_mut() {
                    if b.running() {
                        b.do_stop(&mut cx);
                    }
                }
            }
        }
    }

    /// The names of the behaviours running now, in `getRunningBehaviors` order.
    pub fn running_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for g in &self.groups {
            for (_, bs) in &g.activities {
                for b in bs {
                    b.running_names(&mut out);
                }
            }
        }
        out
    }

    /// Delays each sensor's first scan by a draw from `random` (as the constructor did): the
    /// parity harness runs it after pinning the mob's random.
    pub fn randomly_delay_sensors(&mut self, random: &mut dyn RandomSource) {
        for s in self.sensors.iter_mut() {
            s.time_to_tick = random.next_int_bounded(s.s.scan_rate()) as i64;
        }
    }

    /// Seeds the shuffles of the gates (vanilla's `ShufflingList` draws from an unpinnable
    /// random): gate `k` (depth first in registration order) gets `LegacyRandom(base + k)`.
    pub fn seed_gates(&mut self, base: i64) {
        let mut k = 0;
        for g in self.groups.iter_mut() {
            for (_, bs) in g.activities.iter_mut() {
                for b in bs.iter_mut() {
                    b.seed_gates(base, &mut k);
                }
            }
        }
    }
}

/// Ticks the mob's brain (`getBrain().tick(level, this)`); nothing for goal-driven mobs.
pub fn tick_brain(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(mut b) = m.brain.take() {
        b.tick(e, m, level);
        m.brain = Some(b);
    }
}

/// The parity harness' pin: the gates' shuffles seeded from the mob's random state and every
/// sensor's first scan delayed by a draw from it (`MobVectors.pinBrain` does the same).
pub fn pin(e: &mut Entity) {
    let base = e.random.state();
    let mut random = e.random.clone();
    if let Some(m) = super::data_mut(e)
        && let Some(b) = m.brain.as_mut()
    {
        b.seed_gates(base);
        b.randomly_delay_sensors(&mut random);
    }
    e.random = random;
}

fn debug_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KILN_BRAIN_DEBUG").is_some())
}
