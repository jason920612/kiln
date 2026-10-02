//! Frog: hops and swims (amphibious navigation, `SmoothSwimmingMoveControl`), long-jumps between
//! lily pads, eats small slimes and magma cubes with its tongue (they drop froglights by the
//! frog's variant), follows slime balls, breeds into frogspawn. Variants by biome temperature.
//!
//! Driven by the brain of `FrogAi` on [`crate::mob::brain`]: core (panic, look and move sinks,
//! cooldowns), idle (look at players, love, temptation, attacking, finding land, strolling or
//! croaking), swim, lay spawn (pregnant), tongue (eating) and long jump.

use crate::behavior_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::amphibian::{self, LongJumpCfg};
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat;
use crate::mob::brain::sensors;
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Gate, Mem, Sensor, Status, Timed, Val, WalkTarget, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::path::PathType;
use crate::mob::{self, GroupData, MobData, MobKind, SpawnContext, mth, species};
use crate::persist::{Input, Output};
use crate::sensor_boilerplate;
use kiln_data::entities::pose;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Frog;

pub static KIND: Frog = Frog;

static INFO: Info = Info {
    head: (5, 40, 35),
    breathes_under_water: true,
    ..Info::animal("minecraft:frog", &[(MaxHealth, 10.0), (MovementSpeed, 1.0), (AttackDamage, 10.0), (StepHeight, 1.0)])
};

/// `FrogAi.TIME_BETWEEN_LONG_JUMPS`.
const TIME_BETWEEN_LONG_JUMPS: (i32, i32) = (100, 140);

#[derive(Clone, Debug)]
pub struct State {
    /// `getPose`: standing, croaking, using its tongue or long jumping.
    pub pose: i32,
    /// `Frog.DATA_TONGUE_TARGET_ID`.
    pub tongue_target: Option<i32>,
    /// What `Collections.shuffle` draws from (pinned by the parity replay only).
    pub shuffle: Option<LegacyRandom>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("frog state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("frog state")
}

pub fn tongue_target(m: &MobData) -> Option<i32> {
    st(m).tongue_target
}

/// Whether block `state` is in `#frog_prefer_jump_to` (lily pads, big dripleaves).
pub fn prefers_jump_to(state: u16) -> bool {
    block_flag(state, 0)
}

fn frogs_spawnable_on(state: u16) -> bool {
    block_flag(state, 1)
}

/// The state's membership in the block tags this module reads (a table of flags per state).
fn block_flag(state: u16, which: usize) -> bool {
    use std::sync::OnceLock;
    static T: OnceLock<Vec<u8>> = OnceLock::new();
    T.get_or_init(|| {
        let tags = ["minecraft:frog_prefer_jump_to", "minecraft:frogs_spawnable_on"];
        let mut out = vec![0u8; kiln_data::blocks::STATE_COUNT as usize];
        let block_tags = kiln_data::registries::TAGS.iter().find(|(r, _)| *r == "minecraft:block").map_or(&[][..], |(_, t)| *t);
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        for (i, name) in tags.iter().enumerate() {
            let ids = block_tags.iter().find(|(t, _)| t == name).map_or(&[][..], |(_, ids)| *ids);
            for &id in ids {
                if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                    for s in &mut out[info.first as usize..=info.last as usize] {
                        *s |= 1 << i;
                    }
                }
            }
        }
        out
    })[state as usize]
        & (1 << which)
        != 0
}

/// `Frog.canEat`: a slime or magma cube of size 1 (`#frog_food`).
pub fn can_eat(level: &dyn EntityLevel, id: i32) -> bool {
    let Some(e) = level.entity(id) else { return false };
    if let Some(m) = mob::data(e)
        && matches!(m.kind, MobKind::Slime | MobKind::MagmaCube)
        && super::slime::size(m) != 1
    {
        return false;
    }
    mob::entity_type_tag(e.type_name, "minecraft:frog_food")
}

/// `FluidTags.FROG_TRIES_TO_FIND_LAND_NEAR` and `SUPPORTS_FROGSPAWN` (a water source).
fn water_source(state: u16) -> bool {
    crate::physics::fluid_state(state).kind == crate::physics::FluidKind::Water
}

/// `FrogAi.isAcceptableLandingSpot`: dry ground, lily pads and trapdoors; else the default.
fn acceptable_landing_spot(cx: &mut Cx, pos: BlockPos) -> bool {
    crate::prof!("jump", "frog acceptable");
    let below = pos.below();
    let level = &*cx.level;
    if !crate::physics::fluid_state(level.block(pos)).is_empty()
        || !crate::physics::fluid_state(level.block(below)).is_empty()
        || !crate::physics::fluid_state(level.block(pos.above())).is_empty()
    {
        return false;
    }
    let (state, state_below) = (level.block(pos), level.block(below));
    if prefers_jump_to(state) || prefers_jump_to(state_below) {
        return true;
    }
    let t = mob::path::path_type_static(level, pos.x, pos.y, pos.z);
    let t_below = mob::path::path_type_static(level, below.x, below.y, below.z);
    if t == PathType::Trapdoor || (kiln_data::blocks_types::is_air(state) && t_below == PathType::Trapdoor) {
        return true;
    }
    amphibian::default_acceptable_landing_spot(cx, pos)
}

fn set_long_jump_pose(m: &mut MobData, jumping: bool) {
    st_mut(m).pose = if jumping { pose::LONG_JUMPING } else { pose::STANDING };
}

fn shuffle_random(m: &mut MobData) -> &mut LegacyRandom {
    if st(m).shuffle.is_some() {
        st_mut(m).shuffle.as_mut().unwrap()
    } else {
        &mut m.brain_random
    }
}

const JUMP: LongJumpCfg = LongJumpCfg {
    time_between: TIME_BETWEEN_LONG_JUMPS,
    max_height: 2,
    max_width: 4,
    max_velocity: 3.5714288,
    jump_sound: "minecraft:entity.frog.long_jump",
    landing_sound: "minecraft:entity.frog.step",
    acceptable: acceptable_landing_spot,
    preferred: Some((prefers_jump_to, 0.5)),
    set_pose: set_long_jump_pose,
    shuffle: shuffle_random,
};

/// `FrogAi.initMemories`: the first long jump comes after 100 to 140 ticks.
pub fn init_memories(m: &mut MobData, r: &mut dyn RandomSource) {
    let n = r.next_int_bounded(TIME_BETWEEN_LONG_JUMPS.1 - TIME_BETWEEN_LONG_JUMPS.0 + 1) + TIME_BETWEEN_LONG_JUMPS.0;
    if let Some(b) = m.brain.as_mut() {
        b.st.mem.set(Mem::LongJumpCooldownTicks, Val::Int(n));
    }
}

/// `VariantUtils.selectVariantToSpawn` for frogs: the cold or warm variant in their biomes,
/// else temperate.
fn spawn_variant(biome: Option<i32>) -> i32 {
    let name = match biome {
        Some(b) if species::biome_tag(b, "minecraft:spawns_cold_variant_frogs") => "minecraft:cold",
        Some(b) if species::biome_tag(b, "minecraft:spawns_warm_variant_frogs") => "minecraft:warm",
        _ => "minecraft:temperate",
    };
    kiln_data::synced_id("minecraft:frog_variant", name).unwrap_or(0)
}

impl Kind for Frog {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// The constructor: amphibious navigation, no cost for water, no trapdoors.
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.amphibious = true;
        m.nav.frog = true;
        super::tame::set_malus(m, PathType::Water, 0.0);
        super::tame::set_malus(m, PathType::Trapdoor, -1.0);
        Some(Box::new(State { pose: pose::STANDING, tongue_target: None, shuffle: None }))
    }

    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn pin_replay(&self, m: &mut MobData, base: i64) {
        st_mut(m).shuffle = Some(LegacyRandom::new(base + 1000));
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:frog_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:frog_food")
    }

    /// `Frog.customServerAiStep`: the brain, then `FrogAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Tongue, Activity::LaySpawn, Activity::LongJump, Activity::Swim, Activity::Idle]);
        }
        // `Frog.getTarget`: `getTargetFromBrain`.
        m.target = m.brain.as_ref().and_then(|b| b.st.mem.entity(Mem::AttackTarget));
    }

    /// `SmoothSwimmingMoveControl(this, 85, 10, 0.02f, 0.1f, true)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        amphibian::smooth_swimming_move(e, m, 85, 10, 0.02, 0.1, true);
        true
    }

    /// `FrogLookControl`: the pitch is kept while the tongue is out.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let reset = st(m).tongue_target.is_none();
        mob::control::tick_look_with(e, m, reset);
        true
    }

    /// `Frog.travelInWater`: no drag but 0.9, no jumping out.
    fn travel_in_water(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        let speed = m.speed;
        mob::move_relative(e, speed, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        true
    }

    fn pushed_by_fluid(&self) -> bool {
        false
    }

    /// `Frog.calculateFallDamage`: five points off.
    fn fall_damage_reduction(&self) -> i32 {
        5
    }

    fn breed_as_pregnancy(&self) -> bool {
        true
    }

    /// `checkFrogSpawnRules`: on `#frogs_spawnable_on` in the light.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(frogs_spawnable_on(view.block(pos.below())) && view.raw_brightness(pos, 0) > 8)
    }

    /// `Frog.finalizeSpawn`: the variant of the biome, the first long jump's delay, then the
    /// animal's own.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        // `PriorityProvider.pick` draws among the (single) best candidates.
        let _ = r.next_int_bounded(1);
        m.variant = spawn_variant(ctx.biome);
        init_memories(m, r);
        ext::ageable_finalize(e, m, r, group, 0.05);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        if let Some(v) = r.get("variant").and_then(Tag::as_str).and_then(|v| kiln_data::synced_id("minecraft:frog_variant", v)) {
            m.variant = v;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        if let Some(n) = super::wolf::synced_name("minecraft:frog_variant", m.variant) {
            o.put("variant", Tag::String(n.to_owned()));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let s = st(m);
        d.set(data::frog::VARIANT, &DataValue::Holder(m.variant));
        d.set(data::frog::TONGUE_TARGET, &DataValue::OptionalUnsignedInt(s.tongue_target.map(|t| t as u32)));
        if !m.is_dead_or_dying() {
            d.set(data::entity::POSE, &DataValue::Pose(s.pose));
        }
    }
}

// ---------------------------------------------------------------------------- the brain

/// `FrogAttackablesSensor`: the closest edible mob within 10 blocks that can be reached.
#[derive(Clone, Debug)]
struct FrogAttackables;

impl Sensor for FrogAttackables {
    fn name(&self) -> &'static str {
        "FrogAttackablesSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestAttackable, Mem::NearestVisibleLivingEntities, Mem::UnreachableTongueTargets]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let found = util::find_closest_visible(cx, |cx, id| {
            let Some(t) = util::living(cx, id) else { return false };
            if !util::is_entity_attackable(cx, &t) || !can_eat(&*cx.level, id) {
                return false;
            }
            let uuid = cx.level.entity(id).map_or(0, |e| e.uuid);
            if let Some(Val::Uuids(l)) = cx.b.mem.get(Mem::UnreachableTongueTargets)
                && l.contains(&uuid)
            {
                return false;
            }
            cx.e.position().distance_to_sqr(t.pos) < 10.0 * 10.0
        });
        cx.b.mem.set_opt(Mem::NearestAttackable, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

/// `triggerIf(predicate)`.
fn trigger_if(pred: fn(&Cx) -> bool) -> Box<dyn brain::Control> {
    shot("", &[], move |cx| pred(cx))
}

fn on_ground(cx: &Cx) -> bool {
    cx.e.on_ground
}

fn in_water(cx: &Cx) -> bool {
    cx.e.is_in_water()
}

fn not_breeding(cx: &mut Cx) -> bool {
    !util::is_breeding(cx)
}

fn nearest_attackable(cx: &mut Cx) -> Option<i32> {
    cx.b.mem.entity(Mem::NearestAttackable)
}

fn start_attacking() -> Box<dyn brain::Control> {
    combat::start_attacking(not_breeding, nearest_attackable)
}

fn look_at_player() -> Box<dyn brain::Control> {
    SetEntityLookTargetSometimes::new(Some("minecraft:player"), 6.0, (30, 60))
}

/// `FrogAi.getActivities` and `Frog.BRAIN_PROVIDER`'s sensors.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::HurtBy),
        Box::new(FrogAttackables),
        Box::new(sensors::Tempting { items: Some(&["minecraft:slime_ball"]) }),
        Box::new(sensors::IsInWater),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            AnimalPanic::new(2.0),
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::new(),
            CountDownCooldownTicks::new(Mem::TemptationCooldownTicks),
            CountDownCooldownTicks::new(Mem::LongJumpCooldownTicks),
        ],
    );
    let land = StrollKind::Land { avoid_water: false };
    let idle = ActivityData::with_conditions(
        Activity::Idle,
        vec![
            (0, look_at_player()),
            (0, AnimalMakeLove::new("minecraft:frog", 1.0, 2)),
            (1, FollowTemptation::new(|_| 1.25)),
            (2, start_attacking()),
            (3, amphibian::try_find_land(6, 1.0)),
            (
                4,
                Gate::run_one_when(
                    &[(Mem::WalkTarget, Status::ValueAbsent)],
                    vec![
                        (stroll(1.0, land), 1),
                        (set_walk_target_from_look_target(1.0, 3), 1),
                        (Timed::new(Croak { counter: 0 }), 3),
                        (trigger_if(on_ground), 2),
                    ],
                ),
            ),
        ],
        &[(Mem::LongJumpMidJump, Status::ValueAbsent), (Mem::IsInWater, Status::ValueAbsent)],
    );
    let swim = ActivityData::with_conditions(
        Activity::Swim,
        vec![
            (0, look_at_player()),
            (1, FollowTemptation::new(|_| 1.25)),
            (2, start_attacking()),
            (3, amphibian::try_find_land(8, 1.5)),
            (
                5,
                Gate::new(
                    "GateBehavior",
                    &[(Mem::WalkTarget, Status::ValueAbsent)],
                    &[],
                    brain::OrderPolicy::Ordered,
                    brain::RunningPolicy::TryAll,
                    vec![
                        (stroll(0.75, StrollKind::Swim), 1),
                        (stroll(1.0, land), 1),
                        (set_walk_target_from_look_target(1.0, 3), 1),
                        (trigger_if(in_water), 5),
                    ],
                ),
            ),
        ],
        &[(Mem::LongJumpMidJump, Status::ValueAbsent), (Mem::IsInWater, Status::ValuePresent)],
    );
    let lay_spawn = ActivityData::with_conditions(
        Activity::LaySpawn,
        vec![
            (0, look_at_player()),
            (1, start_attacking()),
            (2, amphibian::try_find_land_near_liquid(8, 1.0, water_source)),
            (3, amphibian::try_lay_spawn_on_fluid_near_land(kiln_data::blocks::default_state::FROGSPAWN, supports_frogspawn)),
            (
                4,
                Gate::run_one(vec![
                    (stroll(1.0, land), 2),
                    (set_walk_target_from_look_target(1.0, 3), 1),
                    (Timed::new(Croak { counter: 0 }), 2),
                    (trigger_if(on_ground), 1),
                ]),
            ),
        ],
        &[(Mem::LongJumpMidJump, Status::ValueAbsent), (Mem::IsPregnant, Status::ValuePresent)],
    );
    let tongue = ActivityData::with_conditions(
        Activity::Tongue,
        vec![(0, combat::stop_attacking_if_target_invalid_default()), (1, Timed::new(ShootTongue { eat_timer: 0, path_counter: 0, state: TongueState::Done }))],
        &[(Mem::AttackTarget, Status::ValuePresent)],
    );
    let jump = ActivityData::with_conditions(
        Activity::LongJump,
        vec![(0, amphibian::LongJumpMidJump::new(JUMP)), (1, amphibian::LongJumpToRandomPos::new(JUMP))],
        &[
            (Mem::TemptingPlayer, Status::ValueAbsent),
            (Mem::BreedTarget, Status::ValueAbsent),
            (Mem::LongJumpCooldownTicks, Status::ValueAbsent),
            (Mem::IsInWater, Status::ValueAbsent),
        ],
    );
    Brain::new(&[], sensors, vec![core, idle, swim, lay_spawn, tongue, jump], random)
}

/// `FluidTags.SUPPORTS_FROGSPAWN` (a water source) or `BlockTags.SUPPORTS_FROGSPAWN` (empty).
fn supports_frogspawn(state: u16) -> bool {
    water_source(state)
}

/// `Croak`: a minute of croaking in place.
#[derive(Clone, Debug)]
struct Croak {
    counter: i32,
}

impl Behavior for Croak {
    fn name(&self) -> &'static str {
        "Croak"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, Status::ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (100, 100)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        st(cx.m).pose == pose::STANDING
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        self.counter < 60
    }
    fn start(&mut self, cx: &mut Cx) {
        if cx.e.is_in_water() || cx.e.is_in_lava() {
            return;
        }
        st_mut(cx.m).pose = pose::CROAKING;
        self.counter = 0;
    }
    fn stop(&mut self, cx: &mut Cx) {
        st_mut(cx.m).pose = pose::STANDING;
    }
    fn tick(&mut self, _cx: &mut Cx) {
        self.counter += 1;
    }
    behavior_boilerplate!();
}

/// `ShootTongue.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TongueState {
    MoveToTarget,
    CatchAnimation,
    EatAnimation,
    Done,
}

/// `ShootTongue`: walks up to the target, flicks its tongue, eats it.
#[derive(Clone, Debug)]
struct ShootTongue {
    eat_timer: i32,
    path_counter: i32,
    state: TongueState,
}

impl ShootTongue {
    /// `canPathfindToTarget`: a path that ends within 1.75 blocks of the target.
    fn can_pathfind_to(cx: &mut Cx, target: i32) -> bool {
        let Some(t) = cx.level.entity(target).map(|t| t.block_position()) else { return false };
        let path = mob::path::create_path_to_entity(cx.e, cx.m, &*cx.level, t, 0);
        path.is_some_and(|p| p.dist_to_target < 1.75)
    }

    /// `addUnreachableTargetToMemory`: up to five, for 100 ticks.
    fn add_unreachable(cx: &mut Cx, target: i32) {
        let uuid = cx.level.entity(target).map_or(0, |e| e.uuid);
        let mut list = match cx.b.mem.get(Mem::UnreachableTongueTargets) {
            Some(Val::Uuids(l)) => l.clone(),
            _ => Vec::new(),
        };
        let absent = !list.contains(&uuid);
        if list.len() == 5 && absent {
            list.remove(0);
        }
        if absent {
            list.push(uuid);
        }
        cx.b.mem.set_expiring(Mem::UnreachableTongueTargets, Val::Uuids(list), 100);
    }

    /// `eatEntity`.
    fn eat(cx: &mut Cx) {
        let pos = cx.e.position();
        cx.level.emit(Event::Sound { pos, sound: "minecraft:entity.frog.eat", source: "neutral", volume: 2.0, pitch: 1.0 });
        let Some(id) = st(cx.m).tongue_target else { return };
        let Some(t) = util::living(cx, id) else { return };
        if !t.alive {
            return;
        }
        mob::do_hurt_target(cx.e, cx.m, cx.level, &t);
        if !util::living(cx, id).is_some_and(|t| t.alive) {
            // `remove(KILLED)`.
            if let Some(te) = cx.level.entity_mut(id) {
                te.discard();
            }
        }
    }
}

impl Behavior for ShootTongue {
    fn name(&self) -> &'static str {
        "ShootTongue"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, Status::ValueAbsent), (Mem::LookTarget, Status::Registered), (Mem::AttackTarget, Status::ValuePresent), (Mem::IsPanicking, Status::ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (100, 100)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        let Some(target) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
        let can = Self::can_pathfind_to(cx, target);
        if !can {
            cx.b.mem.erase(Mem::AttackTarget);
            Self::add_unreachable(cx, target);
        }
        can && st(cx.m).pose != pose::CROAKING && can_eat(&*cx.level, target)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::AttackTarget) && self.state != TongueState::Done && !cx.b.mem.has(Mem::IsPanicking)
    }
    fn start(&mut self, cx: &mut Cx) {
        let Some(target) = cx.b.mem.entity(Mem::AttackTarget) else { return };
        util::look_at_entity(cx, target);
        st_mut(cx.m).tongue_target = Some(target);
        if let Some(p) = cx.level.entity(target).map(|t| t.position()) {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, 2.0, 0)));
        }
        self.path_counter = 10;
        self.state = TongueState::MoveToTarget;
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::AttackTarget);
        let s = st_mut(cx.m);
        s.tongue_target = None;
        s.pose = pose::STANDING;
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(target) = cx.b.mem.entity(Mem::AttackTarget) else { return };
        st_mut(cx.m).tongue_target = Some(target);
        match self.state {
            TongueState::MoveToTarget => {
                let Some(tp) = cx.level.entity(target).map(|t| t.position()) else { return };
                // `Entity.distanceTo`, in floats.
                let p = cx.e.position();
                let (dx, dy, dz) = ((tp.x - p.x) as f32, (tp.y - p.y) as f32, (tp.z - p.z) as f32);
                let dist = mth::sqrt_f(dx * dx + dy * dy + dz * dz);
                if dist < 1.75 {
                    let pos = cx.e.position();
                    cx.level.emit(Event::Sound { pos, sound: "minecraft:entity.frog.tongue", source: "neutral", volume: 2.0, pitch: 1.0 });
                    st_mut(cx.m).pose = pose::USING_TONGUE;
                    let pull = (p - tp).normalize().scale(0.75);
                    if let Some(te) = cx.level.entity_mut(target) {
                        te.delta = pull;
                        te.needs_sync = true;
                    }
                    self.eat_timer = 0;
                    self.state = TongueState::CatchAnimation;
                } else if self.path_counter > 0 {
                    self.path_counter -= 1;
                } else {
                    cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(tp, 2.0, 0)));
                    self.path_counter = 10;
                }
            }
            TongueState::CatchAnimation => {
                // `if (eatAnimationTimer++ < 6) return`: the old value is compared.
                let old = self.eat_timer;
                self.eat_timer += 1;
                if old >= 6 {
                    self.state = TongueState::EatAnimation;
                    Self::eat(cx);
                }
            }
            TongueState::EatAnimation => {
                if self.eat_timer >= 10 {
                    self.state = TongueState::Done;
                } else {
                    self.eat_timer += 1;
                }
            }
            TongueState::Done => {}
        }
    }
    behavior_boilerplate!();
}
