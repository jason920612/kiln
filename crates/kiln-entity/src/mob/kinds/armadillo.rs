//! Armadillo: rolls up into its shell when something scary is near (undead, whoever hurt it,
//! sprinting or riding players within 7 blocks), peeks out now and then and unrolls once the
//! danger has been gone 80 ticks; hurt while rolled up it takes (damage - 1) / 2. It sheds a
//! scute every 5 to 10 minutes and gives one to a brush.
//!
//! Driven by the brain of `ArmadilloAi` (core: swim, panic, look sink, move sink, cooldowns;
//! idle: look at players, love, temptation or following an adult, looking about, strolling;
//! panic: the ball-up), on [`crate::mob::brain`].

use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::sensors::{self, MobSensor};
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Gate, Mem, Status, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Living};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, MobData};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Armadillo;

pub static KIND: Armadillo = Armadillo;

static INFO: Info = Info { head: (32, 40, 10), ..Info::animal("minecraft:armadillo", &[(MaxHealth, 12.0), (MovementSpeed, 0.14)]) };

/// `ArmadilloState`: name, threatened, animation length.
const STATES: [(&str, bool, i64); 4] = [("idle", false, 0), ("rolling", true, 10), ("scared", true, 50), ("unrolling", true, 30)];
const IDLE: u8 = 0;
const ROLLING: u8 = 1;
const SCARED: u8 = 2;
const UNROLLING: u8 = 3;

#[derive(Clone, Debug)]
pub struct State {
    pub state: u8,
    in_state_ticks: i64,
    scute_time: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("armadillo state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("armadillo state")
}

pub fn is_scared(m: &MobData) -> bool {
    st(m).state != IDLE
}

fn switch_to(m: &mut MobData, state: u8) {
    let s = st_mut(m);
    if s.state != state {
        s.in_state_ticks = 0;
    }
    s.state = state;
}

/// `canStayRolledUp`: not panicking, in a liquid or riding.
fn can_stay_rolled_up(e: &Entity, panicking: bool) -> bool {
    !panicking && !e.is_in_water() && !e.is_in_lava() && e.vehicle.is_none() && e.passengers.is_empty()
}

/// `rollUp`.
fn roll_up(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if is_scared(m) {
        return;
    }
    // `stopInPlace`.
    m.nav.stop();
    m.xxa = 0.0;
    m.yya = 0.0;
    mob::control::set_speed(m, 0.0);
    e.delta = Vec3::ZERO;
    m.in_love = 0;
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    mob::make_sound(e, m, level, "minecraft:entity.armadillo.roll");
    switch_to(m, ROLLING);
}

/// `rollOut`.
fn roll_out(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !is_scared(m) {
        return;
    }
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    mob::make_sound(e, m, level, "minecraft:entity.armadillo.unroll_finish");
    switch_to(m, IDLE);
}

fn pick_scute_time(r: &mut dyn RandomSource) -> i32 {
    r.next_int_bounded(6000) + 6000
}

/// `isScaredBy`: within 7 (2 up and down), undead, the last attacker, or a sprinting or
/// riding player.
fn scared_by(cx: &mut Cx, t: &Living) -> bool {
    if !cx.e.bounding_box().inflate(7.0, 2.0, 7.0).intersects(&t.bb) {
        return false;
    }
    if !t.player && mob::entity_type_tag(t.type_name, "minecraft:undead") {
        return true;
    }
    if cx.m.last_hurt_by_mob == Some(t.id) {
        return true;
    }
    if t.player {
        if t.spectator {
            return false;
        }
        return cx.level.player(t.id).is_some_and(|p| p.sprinting || p.vehicle.is_some());
    }
    false
}

fn ready(cx: &Cx) -> bool {
    can_stay_rolled_up(cx.e, cx.b.mem.has(Mem::IsPanicking))
}

fn panicking(m: &MobData) -> bool {
    m.brain.as_ref().is_some_and(|b| b.st.mem.has(Mem::IsPanicking))
}

impl Kind for Armadillo {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        Some(Box::new(State { state: IDLE, in_state_ticks: 0, scute_time: pick_scute_time(random) }))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:armadillo_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:armadillo_food")
    }


    /// The brain, `ArmadilloAi.updateActivity`, then the scute (`customServerAiStep`).
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Panic, Activity::Idle]);
        }
        st_mut(m).scute_time -= 1;
        if mob::is_alive(e, m) && st(m).scute_time <= 0 {
            if level.mob_drops() {
                level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:gameplay/armadillo_shed", pos: e.position() });
                let pitch = (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0;
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.armadillo.scute_drop", source: "neutral", volume: 1.0, pitch });
                }
                level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: e.position(), entity: Some(e.id) });
            }
            st_mut(m).scute_time = pick_scute_time(&mut e.random);
        }
    }

    /// `Armadillo.tick` after `Mob.tick`: the head keeps to the body while scared.
    fn post_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if is_scared(m) {
            m.y_head_rot = m.y_body_rot;
        }
        st_mut(m).in_state_ticks += 1;
    }

    /// The body stays put while scared.
    fn tick_body(&self, _e: &mut Entity, m: &mut MobData) -> bool {
        is_scared(m)
    }

    /// `hurtServer`: the shell halves the damage (less one).
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let amount = if is_scared(m) { (amount - 1.0) / 2.0 } else { amount };
        Some(mob::hurt_base(e, m, level, *source, amount))
    }

    /// `actuallyHurt`: an attacker scares it, fire and the like make it unroll.
    fn actually_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) {
        if m.no_ai || m.is_dead_or_dying() {
            return;
        }
        if source.attacker.is_some_and(|a| goals::living(level, a).is_some()) {
            if let Some(b) = m.brain.as_mut() {
                b.st.mem.set_expiring(Mem::DangerDetectedRecently, brain::Val::Bool(true), 80);
            }
            if can_stay_rolled_up(e, panicking(m)) {
                roll_up(e, m, level);
            }
        } else if source.kind.is_tag("minecraft:panic_environmental_causes") {
            roll_out(e, m, level);
        }
    }

    /// A brush takes a scute off an adult (16 durability).
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if !stack.is_empty() && mob::item_name(stack) == "minecraft:brush" && !m.baby() {
            level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:brush/armadillo", pos: e.position() });
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.armadillo.brush", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
            level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(e.id) });
            return Some(Outcome::success(HeldChange::Damage(16)));
        }
        if is_scared(m) {
            return Some(Outcome::PASS);
        }
        None
    }

    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        !is_scared(m) && !is_scared(partner)
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        is_scared(m).then_some(None)
    }

    /// `checkArmadilloSpawnRules`: on `#armadillo_spawnable_on` in the light.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(super::wolf::block_in_tag(view.block(pos.below()), "minecraft:armadillo_spawnable_on") && view.raw_brightness(pos, 0) > 8)
    }

    /// `BABY_DIMENSIONS`: 0.6 of the adult, eyes at 0.21875.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (base.0 * 0.6, base.1 * 0.6, 0.21875) } else { base }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let state = r.get("state").and_then(Tag::as_str).and_then(|n| STATES.iter().position(|s| s.0 == n)).unwrap_or(0) as u8;
        let scute = r.num("scute_time").map(|v| v as i32);
        switch_to(m, state);
        if let Some(t) = scute {
            st_mut(m).scute_time = t;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("state", Tag::String(STATES[s.state as usize].0.into()));
        o.put("scute_time", Tag::Int(s.scute_time));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::armadillo::ARMADILLO_STATE, &DataValue::Enum(st(m).state as i32));
    }
}

/// `ArmadilloAi.getActivities` and the sensors of `Armadillo.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::HurtBy),
        Box::new(sensors::Tempting::for_animal()),
        Box::new(sensors::Adult { any_type: false }),
        Box::new(MobSensor { scan_rate: 5, mob_test: scared_by, ready_test: ready, to_set: Mem::DangerDetectedRecently, ttl: 80 }),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            Swim::new(0.8),
            brain::Timed::new(ArmadilloPanic(AnimalPanic { speed: 2.0, causes: "minecraft:panic_environmental_causes", air: None })),
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::vetoed(|cx| is_scared(cx.m)),
            CountDownCooldownTicks::new(Mem::TemptationCooldownTicks),
            CountDownCooldownTicks::new(Mem::GazeCooldownTicks),
            rolling_out(),
        ],
    );
    let idle = ActivityData::with_priorities(
        Activity::Idle,
        vec![
            (0, SetEntityLookTargetSometimes::new(Some("minecraft:player"), 6.0, (30, 60))),
            (1, AnimalMakeLove::new("minecraft:armadillo", 1.0, 1)),
            (
                2,
                Gate::run_one(vec![
                    (FollowTemptation::with(|_| 1.25, |cx| if cx.m.baby() { 1.0 } else { 2.0 }, false), 1),
                    (baby_follow_adult((5, 16), |_| 1.25, Mem::NearestVisibleAdult, false), 1),
                ]),
            ),
            (3, RandomLookAround::new((150, 250), 30.0, 0.0, 0.0)),
            (
                4,
                Gate::run_one_when(
                    &[(Mem::WalkTarget, Status::ValueAbsent)],
                    vec![
                        (stroll(1.0, StrollKind::Land { avoid_water: true }), 1),
                        (set_walk_target_from_look_target(1.0, 3), 1),
                        (DoNothing::new(30, 60), 1),
                    ],
                ),
            ),
        ],
    );
    let scared = ActivityData::with_conditions(
        Activity::Panic,
        vec![(0, brain::Timed::new(BallUp { next_peek: 0, danger_was_around: false }))],
        &[(Mem::DangerDetectedRecently, Status::ValuePresent), (Mem::IsPanicking, Status::ValueAbsent)],
    );
    Brain::new(&[], sensors, vec![core, idle, scared], random)
}

/// `ArmadilloAi.ARMADILLO_ROLLING_OUT`: no danger any more: unroll.
fn rolling_out() -> Box<dyn brain::Control> {
    shot("", &[(Mem::DangerDetectedRecently, Status::ValueAbsent)], |cx| {
        if is_scared(cx.m) {
            roll_out(cx.e, cx.m, cx.level);
            true
        } else {
            false
        }
    })
}

/// `ArmadilloAi.ArmadilloPanic`: unrolls, then panics.
#[derive(Clone, Debug)]
struct ArmadilloPanic(AnimalPanic);

impl Behavior for ArmadilloPanic {
    fn name(&self) -> &'static str {
        "ArmadilloPanic"
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
        roll_out(cx.e, cx.m, cx.level);
        self.0.start(cx);
    }
    fn stop(&mut self, cx: &mut Cx) {
        self.0.stop(cx);
    }
    fn tick(&mut self, cx: &mut Cx) {
        self.0.tick(cx);
    }
    behavior_boilerplate!();
}

/// `ArmadilloAi.ArmadilloBallUp`: while danger was detected recently, on the ground and dry:
/// rolled up, peeking out, unrolling when the danger fades.
#[derive(Clone, Debug)]
struct BallUp {
    next_peek: i32,
    danger_was_around: bool,
}

impl BallUp {
    fn pick_peek(e: &mut Entity) -> i32 {
        STATES[SCARED as usize].2 as i32 + mob::mth::next_int_between(&mut e.random, 100, 400)
    }
}

impl Behavior for BallUp {
    fn name(&self) -> &'static str {
        "ArmadilloBallUp"
    }
    /// `BALL_UP_STAY_IN_STATE`: 5 minutes.
    fn duration(&self) -> (i32, i32) {
        (6000, 6000)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.e.on_ground && !cx.e.is_in_water() && !cx.e.is_in_lava()
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        STATES[st(cx.m).state as usize].1
    }
    fn start(&mut self, cx: &mut Cx) {
        roll_up(cx.e, cx.m, cx.level);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if !can_stay_rolled_up(cx.e, cx.b.mem.has(Mem::IsPanicking)) {
            roll_out(cx.e, cx.m, cx.level);
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        if self.next_peek > 0 {
            self.next_peek -= 1;
        }
        let s = st(cx.m);
        if s.state == ROLLING && s.in_state_ticks > STATES[ROLLING as usize].2 {
            switch_to(cx.m, SCARED);
            if cx.e.on_ground && !cx.e.silent {
                let pos = cx.e.position();
                cx.level.emit(Event::Sound { pos, sound: "minecraft:entity.armadillo.land", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
            return;
        }
        let state = st(cx.m).state;
        let left = cx.b.mem.time_until_expiry(Mem::DangerDetectedRecently);
        let danger = left > 75;
        if danger != self.danger_was_around {
            self.next_peek = Self::pick_peek(cx.e);
        }
        self.danger_was_around = danger;
        match state {
            SCARED => {
                if self.next_peek == 0 && cx.e.on_ground && danger {
                    let id = cx.e.id;
                    cx.level.emit(Event::EntityEvent { entity: id, event: 64 });
                    self.next_peek = Self::pick_peek(cx.e);
                }
                if left < STATES[UNROLLING as usize].2 {
                    if !cx.e.silent {
                        let pos = cx.e.position();
                        cx.level.emit(Event::Sound { pos, sound: "minecraft:entity.armadillo.unroll_start", source: "neutral", volume: 1.0, pitch: 1.0 });
                    }
                    switch_to(cx.m, UNROLLING);
                }
            }
            UNROLLING if left > STATES[UNROLLING as usize].2 => switch_to(cx.m, SCARED),
            _ => {}
        }
    }
    behavior_boilerplate!();
}
