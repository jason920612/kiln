//! Sniffer: an ancient animal that sniffs the air, walks to a spot of diggable soil, digs up a
//! seed (torchflower seeds or a pitcher pod, from `gameplay/sniffer_digging`), rises happy and
//! rests for 8 minutes before sniffing again; bred with torchflower seeds it lays an egg.
//!
//! Driven by the brain of `SnifferAi` (core: swim, panic, move sink, temptation cooldown; idle:
//! love, temptation, look sink, feeling happy, then one of walk to the look target / scenting /
//! sniffing / looking at a player / strolling / nothing; sniff: searching; dig: digging and
//! rising), on [`crate::mob::brain`]. The seed and the digging pose are `Sniffer.tick` and
//! `Sniffer.transitionTo`.

use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::animals::Anon;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::memory::GlobalPos;
use crate::mob::brain::sensors;
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Gate, Mem, Status, Timed, Val, WalkTarget, util};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::{self, MobData, path, random_pos};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Sniffer;

pub static KIND: Sniffer = Sniffer;

static INFO: Info = Info { head: (50, 40, 10), ..Info::animal("minecraft:sniffer", &[(MovementSpeed, 0.10000000149011612), (MaxHealth, 14.0)]) };

/// `Sniffer.State` ids.
pub const IDLING: i32 = 0;
pub const FEELING_HAPPY: i32 = 1;
pub const SCENTING: i32 = 2;
pub const SNIFFING: i32 = 3;
pub const SEARCHING: i32 = 4;
pub const DIGGING: i32 = 5;
pub const RISING: i32 = 6;

#[derive(Clone, Debug, Default)]
pub struct State {
    pub state: i32,
    drop_seed_at: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("sniffer state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("sniffer state")
}

fn play(e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch });
    }
}

/// `transitionTo`: the state with its sound (and the digging's seed time); the dimensions follow
/// the state (`onSyncedDataUpdated`).
fn transition(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, to: i32) {
    match to {
        SCENTING => play(e, level, "minecraft:entity.sniffer.scenting", if m.baby() { 1.3 } else { 1.0 }),
        SNIFFING => play(e, level, "minecraft:entity.sniffer.sniffing", 1.0),
        DIGGING => {
            st_mut(m).drop_seed_at = e.tick_count + 120;
            level.emit(Event::EntityEvent { entity: e.id, event: 63 });
        }
        RISING => play(e, level, "minecraft:entity.sniffer.digging_stop", 1.0),
        FEELING_HAPPY => play(e, level, "minecraft:entity.sniffer.happy", 1.0),
        _ => {}
    }
    let changed = st(m).state != to;
    st_mut(m).state = to;
    if changed {
        mob::refresh_dimensions_in(e, m, &*level);
    }
}

/// `getHeadBlock`: 2.25 ahead (`getForward`), 0.2 up.
fn head_block(e: &Entity) -> BlockPos {
    let f = direction_from_rotation(e.x_rot, e.y_rot);
    BlockPos::containing(e.x() + f.x * 2.25, e.y() + 0.20000000298023224, e.z() + f.z * 2.25)
}

/// `canSniff`.
fn can_sniff(cx: &Cx) -> bool {
    let mem = &cx.b.mem;
    !mem.has(Mem::TemptingPlayer) && !mem.has(Mem::IsPanicking) && !cx.e.is_in_water() && cx.m.in_love <= 0 && cx.e.on_ground && cx.e.vehicle.is_none()
}

/// `canDig(pos)`: diggable soil not dug before that the sniffer can walk to.
fn can_dig_at(cx: &mut Cx, p: BlockPos) -> bool {
    if !super::wolf::block_in_tag(cx.level.block(p), "minecraft:sniffer_diggable_block") {
        return false;
    }
    if cx.b.mem.positions(Mem::SnifferExploredPositions).iter().any(|g| &*g.dim == crate::mob::brain::persist::OVERWORLD && g.pos == p) {
        return false;
    }
    path::create_path(cx.e, cx.m, &*cx.level, p, 1).is_some_and(|p| p.reached)
}

/// `canDig()`.
fn can_dig(cx: &mut Cx) -> bool {
    let mem = &cx.b.mem;
    if mem.has(Mem::IsPanicking) || mem.has(Mem::TemptingPlayer) || cx.m.baby() || cx.e.is_in_water() || !cx.e.on_ground || cx.e.vehicle.is_some() {
        return false;
    }
    let at = head_block(cx.e).below();
    can_dig_at(cx, at)
}

/// `calculateDigPosition`: land spots 10 to 18 away whose block below can be dug.
fn calculate_dig_position(cx: &mut Cx) -> Option<BlockPos> {
    for i in 0..5 {
        let Some(p) = random_pos::land_pos(cx.e, &*cx.m, &*cx.level, 10 + 2 * i, 3) else { continue };
        let b = BlockPos::containing(p.x, p.y, p.z);
        // The world border test: Kiln has no border in the mob's level view.
        let below = b.below();
        if can_dig_at(cx, below) {
            return Some(below);
        }
    }
    None
}

/// `storeExploredPosition`: the last 20 dug spots with the new one first.
fn store_explored_position(cx: &mut Cx, pos: BlockPos) {
    let mut list: Vec<GlobalPos> = cx.b.mem.positions(Mem::SnifferExploredPositions).iter().take(20).cloned().collect();
    list.insert(0, GlobalPos::new(crate::mob::brain::persist::OVERWORLD, pos));
    cx.b.mem.set(Mem::SnifferExploredPositions, Val::Positions(list));
}

/// `SnifferAi.resetSniffing`.
fn reset_sniffing(cx: &mut Cx) {
    cx.b.mem.erase(Mem::SnifferDigging);
    cx.b.mem.erase(Mem::SnifferSniffingTarget);
    transition(cx.e, cx.m, cx.level, IDLING);
}

impl Kind for Sniffer {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        m.maluses.push((path::PathType::Water, -1.0));
        m.maluses.push((path::PathType::OnTopOfPowderSnow, -1.0));
        m.maluses.push((path::PathType::DamageCautious, -1.0));
        Some(Box::new(State::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:sniffer_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:sniffer_food")
    }

    /// `Sniffer.tick`: the seed comes up at its time.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).state == DIGGING {
            if e.tick_count % 10 == 0 {
                let h = head_block(e);
                level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: Vec3::new(h.x as f64 + 0.5, h.y as f64 + 0.5, h.z as f64 + 0.5), entity: Some(e.id) });
            }
            if st(m).drop_seed_at == e.tick_count {
                let h = head_block(e);
                level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:gameplay/sniffer_digging", pos: Vec3::new(h.x as f64, h.y as f64, h.z as f64) });
                play(e, level, "minecraft:entity.sniffer.drop_seed", 1.0);
            }
        }
    }

    /// The brain, then `SnifferAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Dig, Activity::Sniff, Activity::Idle]);
        }
    }

    /// `jumpFromGround`: a nudge forward when jumping from a standstill.
    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        let power = m.attrs.value(JumpStrength) as f32 * e.block_jump_factor(level) + mob::effects::jump_boost_power(m);
        if power > 1.0e-5 {
            e.delta = Vec3::new(e.delta.x, (power as f64).max(e.delta.y), e.delta.z);
            e.needs_sync = true;
        }
        if m.mov.speed_modifier > 0.0 && e.delta.horizontal_distance_sqr() < 0.01 {
            mob::move_relative(e, 0.1, Vec3::new(0.0, 0.0, 1.0));
        }
        true
    }

    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        let ok = |x: &MobData| ext::state::<State>(x).is_some_and(|s| matches!(s.state, IDLING | SCENTING | FEELING_HAPPY));
        ok(m) && ok(partner)
    }

    /// `spawnChildFromBreeding`: an egg, not a baby.
    fn breed_as_item(&self) -> Option<&'static str> {
        Some("minecraft:sniffer_egg")
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(if matches!(st(m).state, DIGGING | SEARCHING) { None } else { Some("minecraft:entity.sniffer.idle") })
    }

    /// `die`: back to idling first.
    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &mob::DamageSource) {
        if st(m).state != IDLING {
            st_mut(m).state = IDLING;
            mob::refresh_dimensions_in(e, m, &*level);
        }
    }

    /// `DIGGING_DIMENSIONS`: 0.4 lower, eyes at 0.81.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        let (w, h, eye) = if st(m).state == DIGGING { (base.0, base.1 - 0.4, 0.81) } else { base };
        if m.baby() { (w * 0.5, h * 0.5, eye * 0.5) } else { (w, h, eye) }
    }

    /// The explored spots are a brain memory: the `Brain` tag has them.
    fn load(&self, _e: &mut Entity, _m: &mut MobData, _r: &mut Input) {}

    fn save(&self, _e: &Entity, _m: &MobData, _o: &mut Output) {}

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::sniffer as f;
        let s = st(m);
        d.set(f::STATE, &DataValue::Enum(s.state));
        d.set(f::DROP_SEED_AT_TICK, &DataValue::Int(s.drop_seed_at));
    }
}

/// `SnifferAi.getActivities` and the sensors of `Sniffer.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::HurtBy),
        Box::new(sensors::Players),
        Box::new(sensors::Tempting::for_animal()),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            Swim::new(0.8),
            Anon::on_start(AnimalPanic::new(2.0), reset_sniffing),
            MoveToTargetSink::with_durations(500, 700),
            CountDownCooldownTicks::new(Mem::TemptationCooldownTicks),
        ],
    );
    let idle = ActivityData::with_conditions(
        Activity::Idle,
        vec![
            (0, Anon::on_start(AnimalMakeLove::new("minecraft:sniffer", 1.0, 2), reset_sniffing)),
            (1, Anon::on_start(FollowTemptation::with(|_| 1.25, |cx| if cx.m.baby() { 2.5 } else { 3.5 }, false), reset_sniffing)),
            (2, LookAtTargetSink::new(45, 90)),
            (3, Timed::new(FeelingHappy)),
            (
                4,
                Gate::run_one(vec![
                    (set_walk_target_from_look_target(1.0, 3), 2),
                    (Timed::new(Scenting), 1),
                    (Timed::new(Sniffing), 1),
                    (set_entity_look_target(|cx, id| util::living(cx, id).is_some_and(|l| l.player), 6.0), 1),
                    (stroll(1.0, StrollKind::Land { avoid_water: false }), 1),
                    (DoNothing::new(5, 20), 2),
                ]),
            ),
        ],
        &[(Mem::SnifferDigging, Status::ValueAbsent)],
    );
    let sniff = ActivityData::with_conditions(
        Activity::Sniff,
        vec![(0, Timed::new(Searching))],
        &[(Mem::IsPanicking, Status::ValueAbsent), (Mem::SnifferSniffingTarget, Status::ValuePresent), (Mem::WalkTarget, Status::ValuePresent)],
    );
    let dig = ActivityData::with_conditions(
        Activity::Dig,
        vec![(0, Timed::new(Digging)), (0, Timed::new(FinishedDigging))],
        &[(Mem::IsPanicking, Status::ValueAbsent), (Mem::WalkTarget, Status::ValueAbsent), (Mem::SnifferDigging, Status::ValuePresent)],
    );
    Brain::new(&[Mem::SnifferExploredPositions], sensors, vec![core, idle, sniff, dig], random)
}

/// `Vec3.directionFromRotation` as `Entity.getForward` uses it.
fn direction_from_rotation(x_rot: f32, y_rot: f32) -> Vec3 {
    brain::behaviors::direction_from_rotation(x_rot, y_rot)
}

/// `SnifferAi.Searching`: walking to the spot it will dig.
#[derive(Clone, Debug)]
struct Searching;

impl Behavior for Searching {
    fn name(&self) -> &'static str {
        "Searching"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, Status::ValuePresent), (Mem::IsPanicking, Status::ValueAbsent), (Mem::SnifferSniffingTarget, Status::ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (600, 600)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        can_sniff(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        if !can_sniff(cx) {
            transition(cx.e, cx.m, cx.level, IDLING);
            return false;
        }
        let walk = cx.b.mem.walk_target().and_then(|w| util::tracker_block(cx, &w.target));
        let target = cx.b.mem.block(Mem::SnifferSniffingTarget);
        matches!((walk, target), (Some(a), Some(b)) if a == b)
    }
    fn start(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, SEARCHING);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if can_dig(cx) && can_sniff(cx) {
            cx.b.mem.set(Mem::SnifferDigging, Val::Bool(true));
        }
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::SnifferSniffingTarget);
    }
    behavior_boilerplate!();
}

/// `SnifferAi.Digging`.
#[derive(Clone, Debug)]
struct Digging;

impl Behavior for Digging {
    fn name(&self) -> &'static str {
        "Digging"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::IsPanicking, Status::ValueAbsent), (Mem::WalkTarget, Status::ValueAbsent), (Mem::SnifferDigging, Status::ValuePresent), (Mem::SniffCooldown, Status::ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (160, 180)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        can_sniff(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::SnifferDigging) && can_dig(cx) && cx.m.in_love <= 0
    }
    fn start(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, DIGGING);
    }
    fn stop_timed(&mut self, cx: &mut Cx, timed_out: bool) {
        if timed_out {
            cx.b.mem.set_expiring(Mem::SniffCooldown, Val::Unit, 9600);
        } else {
            reset_sniffing(cx);
        }
    }
    behavior_boilerplate!();
}

/// `SnifferAi.FinishedDigging`: rising out of the hole.
#[derive(Clone, Debug)]
struct FinishedDigging;

impl Behavior for FinishedDigging {
    fn name(&self) -> &'static str {
        "FinishedDigging"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::IsPanicking, Status::ValueAbsent), (Mem::WalkTarget, Status::ValueAbsent), (Mem::SnifferDigging, Status::ValuePresent), (Mem::SniffCooldown, Status::ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (40, 40)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::SnifferDigging)
    }
    fn start(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, RISING);
    }
    fn stop_timed(&mut self, cx: &mut Cx, timed_out: bool) {
        transition(cx.e, cx.m, cx.level, IDLING);
        if timed_out {
            let at = cx.e.on_pos(&*cx.level, 0.2);
            store_explored_position(cx, at);
        }
        cx.b.mem.erase(Mem::SnifferDigging);
        cx.b.mem.set(Mem::SnifferHappy, Val::Bool(true));
    }
    behavior_boilerplate!();
}

/// `SnifferAi.FeelingHappy`.
#[derive(Clone, Debug)]
struct FeelingHappy;

impl Behavior for FeelingHappy {
    fn name(&self) -> &'static str {
        "FeelingHappy"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::SnifferHappy, Status::ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (40, 100)
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, FEELING_HAPPY);
    }
    fn stop(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, IDLING);
        cx.b.mem.erase(Mem::SnifferHappy);
    }
    behavior_boilerplate!();
}

/// `SnifferAi.Scenting`.
#[derive(Clone, Debug)]
struct Scenting;

impl Behavior for Scenting {
    fn name(&self) -> &'static str {
        "Scenting"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::IsPanicking, Status::ValueAbsent),
            (Mem::SnifferDigging, Status::ValueAbsent),
            (Mem::SnifferSniffingTarget, Status::ValueAbsent),
            (Mem::SnifferHappy, Status::ValueAbsent),
            (Mem::BreedTarget, Status::ValueAbsent),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        (40, 80)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        !cx.b.mem.has(Mem::TemptingPlayer)
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, SCENTING);
    }
    fn stop(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, IDLING);
    }
    behavior_boilerplate!();
}

/// `SnifferAi.Sniffing`: sniffs, then picks a spot to dig.
#[derive(Clone, Debug)]
struct Sniffing;

impl Behavior for Sniffing {
    fn name(&self) -> &'static str {
        "Sniffing"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, Status::ValueAbsent), (Mem::SnifferSniffingTarget, Status::ValueAbsent), (Mem::SniffCooldown, Status::ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (40, 80)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        !cx.m.baby() && can_sniff(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        can_sniff(cx)
    }
    fn start(&mut self, cx: &mut Cx) {
        transition(cx.e, cx.m, cx.level, SNIFFING);
    }
    fn stop_timed(&mut self, cx: &mut Cx, timed_out: bool) {
        transition(cx.e, cx.m, cx.level, IDLING);
        if timed_out && let Some(pos) = calculate_dig_position(cx) {
            cx.b.mem.set(Mem::SnifferSniffingTarget, Val::Block(pos));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(pos, 1.25, 0)));
        }
    }
    behavior_boilerplate!();
}
