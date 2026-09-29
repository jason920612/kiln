//! Axolotl: an amphibious `Animal` on a brain (`AxolotlAi`): it hunts fish, squids and drowned
//! (and guardians) in the water for 2400 ticks at a time, plays dead when hurt in the water
//! (regenerating for 200 ticks), gives nearby players regeneration when its prey dies to them,
//! breeds on tropical fish buckets (variants: four common colors and the rare blue) and goes
//! into a water bucket. It dries out on land after 6000 ticks of air.
//!
//! Moves with `SmoothSwimmingMoveControl` / `SmoothSwimmingLookControl` and
//! `AmphibiousPathNavigation`; the axolotl's own controls stand still while playing dead.

use crate::behavior_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::{melee_attack, set_walk_target_from_attack_target_if_out_of_reach, stop_attacking_if_target_invalid, start_attacking};
use crate::mob::brain::memory::{Tracker, Val, WalkTarget};
use crate::mob::brain::sensors::{self};
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Gate, Mem, OrderPolicy, RunningPolicy, Sensor, ShotBehavior, Status, Timed, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::Living;
use crate::mob::interact::{self, HeldChange, Interactor, Outcome};
use crate::mob::control::{self, Operation};
use crate::mob::mth;
use crate::mob::path::PathType;
use crate::mob::{self, Category, DamageSource, GroupData, MobData, SpawnContext};
use crate::persist::{Input, Output};
use crate::sensor_boilerplate;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::{Registered, ValueAbsent, ValuePresent};

pub struct Axolotl;

pub static KIND: Axolotl = Axolotl;

static INFO: Info = Info {
    category: Category::Axolotls,
    breathes_under_water: true,
    head: (1, 1, 10),
    ..Info::animal("minecraft:axolotl", &[(MaxHealth, 14.0), (MovementSpeed, 1.0), (AttackDamage, 2.0), (StepHeight, 1.0)])
};

/// `Axolotl.Variant`: (id, name, common).
const VARIANTS: [(&str, bool); 5] = [("lucy", true), ("wild", true), ("gold", true), ("cyan", true), ("blue", false)];

/// `Axolotl.REGEN_BUFF_*`.
const REGEN_BUFF_BASE_DURATION: i32 = 100;
const REGEN_BUFF_MAX_DURATION: i32 = 2400;
const TOTAL_AIR_SUPPLY: i32 = 6000;

#[derive(Clone, Debug)]
pub struct State {
    pub variant: i32,
    pub playing_dead: bool,
    pub from_bucket: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("axolotl state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("axolotl state")
}

pub fn is_playing_dead(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.playing_dead)
}

pub fn variant(m: &MobData) -> i32 {
    ext::state::<State>(m).map_or(0, |s| s.variant)
}

/// `Axolotl.Variant.getSpawnVariant`: a random variant of the common (or of the rare) ones.
fn spawn_variant(r: &mut dyn RandomSource, common: bool) -> i32 {
    let list: Vec<i32> = (0..VARIANTS.len() as i32).filter(|&i| VARIANTS[i as usize].1 == common).collect();
    list[r.next_int_bounded(list.len() as i32) as usize]
}

// ---------------------------------------------------------------------- air

/// `Entity.isInWaterOrRain`.
fn in_water_or_rain(e: &Entity, level: &dyn EntityLevel) -> bool {
    e.is_in_water() || level.is_raining_at(e.block_position()) || level.is_raining_at(BlockPos::containing(e.x(), e.y() + e.height as f64, e.z()))
}

// ---------------------------------------------------------------------- controls

/// `SmoothSwimmingMoveControl.tick(maxTurnX, maxTurnY, inWaterSpeedModifier, outsideWaterSpeedModifier,
/// applyGravity)`: turns toward the wanted position (pitching in the water) and swims or walks
/// there; stands still without a path.
pub fn smooth_swim_move(e: &mut Entity, m: &mut MobData, max_turn_x: i32, max_turn_y: i32, in_water_speed: f32, outside_speed: f32, apply_gravity: bool) {
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
    if dx * dx + dy * dy + dz * dz < 2.500000277905201e-7 {
        m.zza = 0.0;
        return;
    }
    let h = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
    e.y_rot = control::rotlerp(e.y_rot, h, max_turn_y as f32);
    m.y_body_rot = e.y_rot;
    m.y_head_rot = e.y_rot;
    let speed = (m.mov.speed_modifier * m.attrs.value(MovementSpeed)) as f32;
    if e.is_in_water() {
        control::set_speed(m, speed * in_water_speed);
        let hd = (dx * dx + dz * dz).sqrt();
        if dy.abs() > 9.999999747378752e-6 || hd.abs() > 9.999999747378752e-6 {
            let pitch = -((mth::atan2(dy, hd) * 57.2957763671875) as f32);
            let pitch = mth::clamp(mth::wrap_degrees(pitch), -(max_turn_x as f32), max_turn_x as f32);
            e.x_rot = mth::rotate_towards(e.x_rot, pitch, 5.0);
        }
        let f = mth::cos((e.x_rot * 0.017453292) as f64);
        let g = mth::sin((e.x_rot * 0.017453292) as f64);
        m.zza = f * speed;
        m.yya = -g * speed;
    } else {
        let d = mth::wrap_degrees(e.y_rot - h).abs();
        let factor = 1.0 - mth::clamp((d - 10.0) / 50.0, 0.0, 1.0);
        control::set_speed(m, (speed * outside_speed) * factor);
    }
}

/// `SmoothSwimmingLookControl.tick(maxYRotFromCenter)`: the head looks a little above the
/// target (20 degrees to the side of the pitch offset 10), the body follows the head at 4 degrees
/// a tick beyond the limit.
pub fn smooth_swim_look(e: &mut Entity, m: &mut MobData, max_y_from_center: i32) {
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
    let d = mth::wrap_degrees(m.y_head_rot - m.y_body_rot);
    if d < -(max_y_from_center as f32) {
        m.y_body_rot -= 4.0;
    } else if d > max_y_from_center as f32 {
        m.y_body_rot += 4.0;
    }
}

// ---------------------------------------------------------------------- the sensor

/// `AxolotlAttackablesSensor`: the nearest visible living entity in the water within 8 blocks that
/// is always hostile to axolotls or (when there is no hunting cooldown) prey, and attackable.
#[derive(Clone, Debug)]
struct AxolotlAttackables;

impl AxolotlAttackables {
    fn matching(cx: &mut Cx, id: i32) -> bool {
        let Some(t) = util::living(cx, id) else { return false };
        // `isClose`: within 8 blocks.
        if cx.e.position().distance_to_sqr(t.pos) > 64.0 {
            return false;
        }
        if !super::drowned::in_water(&*cx.level, &t) {
            return false;
        }
        let hostile = mob::entity_type_tag(t.type_name, "minecraft:axolotl_always_hostiles");
        let hunt = !cx.b.mem.has(Mem::HasHuntingCooldown) && mob::entity_type_tag(t.type_name, "minecraft:axolotl_hunt_targets");
        (hostile || hunt) && util::is_entity_attackable(cx, &t)
    }
}

impl Sensor for AxolotlAttackables {
    fn name(&self) -> &'static str {
        "AxolotlAttackablesSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestAttackable, Mem::NearestVisibleLivingEntities, Mem::HasHuntingCooldown]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let found = util::find_closest_visible(cx, AxolotlAttackables::matching);
        cx.b.mem.set_opt(Mem::NearestAttackable, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

// ---------------------------------------------------------------------- behaviours

/// `AxolotlAi.getSpeedModifier` (idling, tempted): 0.5 in the water, 0.15 on land.
fn speed_modifier(cx: &Cx) -> f32 {
    if cx.e.is_in_water() { 0.5 } else { 0.15 }
}

/// `AxolotlAi.getSpeedModifierChasing` and `getSpeedModifierFollowingAdult`.
fn speed_modifier_fast(cx: &Cx) -> f32 {
    if cx.e.is_in_water() { 0.6 } else { 0.15 }
}

/// `AxolotlAi.findNearestValidAttackTarget`: not while breeding.
fn find_nearest_valid_attack_target(cx: &mut Cx) -> Option<i32> {
    if util::is_breeding(cx) {
        return None;
    }
    cx.b.mem.entity(Mem::NearestAttackable)
}

/// `Axolotl.onStopAttacking`: prey killed by a player within 20 blocks of the axolotl gives that
/// player regeneration.
fn on_stop_attacking(cx: &mut Cx, target: i32) {
    let Some(t) = cx.level.entity(target) else { return };
    let Some(tm) = mob::data(t) else { return };
    if !tm.is_dead_or_dying() {
        return;
    }
    let Some(src) = tm.last_damage_source(cx.time) else { return };
    if !src.attacker_is_player {
        return;
    }
    let Some(player) = src.attacker else { return };
    let area = cx.e.bounding_box().inflate_all(20.0);
    if !cx.level.players_in(&area).iter().any(|p| p.id == player) {
        return;
    }
    apply_supporting_effects(cx.level, cx.e.id, player);
}

/// `Axolotl.applySupportingEffects`: regeneration (100 ticks more, up to 2400) unless it lasts
/// longer already; mining fatigue is not removed (no hook to strip a player's effect).
fn apply_supporting_effects(level: &mut dyn EntityLevel, axolotl: i32, player: i32) {
    let current = level.player_effect(player, "minecraft:regeneration");
    // `endsWithin(2399)`.
    let ends = current.is_none_or(|(_, left)| left != -1 && left <= REGEN_BUFF_MAX_DURATION - 1);
    if ends {
        let base = current.map_or(0, |(_, left)| left.max(0));
        let duration = REGEN_BUFF_MAX_DURATION.min(REGEN_BUFF_BASE_DURATION + base);
        level.add_effect(player, "minecraft:regeneration", duration, 0, Some(axolotl));
    }
}

/// `ValidatePlayDead`: counts `PLAY_DEAD_TICKS` down; at zero it erases them (and the hurt-by
/// entity) and the brain goes back to its default activity.
fn validate_play_dead() -> Box<dyn brain::Control> {
    shot("", &[(Mem::PlayDeadTicks, ValuePresent), (Mem::HurtByEntity, Registered)], |cx| {
        let n = cx.b.mem.int(Mem::PlayDeadTicks).unwrap_or(0);
        if n <= 0 {
            cx.b.mem.erase(Mem::PlayDeadTicks);
            cx.b.mem.erase(Mem::HurtByEntity);
            cx.b.use_default_activity();
        } else {
            cx.b.mem.set(Mem::PlayDeadTicks, Val::Int(n - 1));
        }
        true
    })
}

/// `EraseMemoryIf.create(BehaviorUtils::isBreeding, memory)`.
fn erase_if_breeding(mem: Mem) -> Box<dyn brain::Control> {
    // The entry is one memory per instance: the shared cooldown table has `mem present` for each.
    shot("", cooldown_entry(mem), move |cx| {
        if util::is_breeding(cx) {
            cx.b.mem.erase(mem);
            true
        } else {
            false
        }
    })
}

/// `PlayDead`: for 200 ticks in the water, regenerating; the walk and look targets are dropped.
#[derive(Clone, Debug)]
struct PlayDead;

impl Behavior for PlayDead {
    fn name(&self) -> &'static str {
        "PlayDead"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::PlayDeadTicks, ValuePresent), (Mem::HurtByEntity, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (200, 200)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.e.is_in_water()
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.e.is_in_water() && cx.b.mem.has(Mem::PlayDeadTicks)
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::LookTarget);
        let fx = crate::effect::Effect::simple(crate::effect::ids::regeneration(), 200, 0);
        mob::effects::add(cx.e, cx.m, cx.level, fx, None);
    }
    behavior_boilerplate!();
}

/// `TryFindLiquid.create(6, 0.15, AXOLOTL_TRIES_TO_FIND)`: out of the water, walks to the nearest
/// water block within 6 (Manhattan) that has air above it (or else the first one further than
/// 1.5 blocks away); every 40 ticks.
#[derive(Clone, Debug)]
struct TryFindLiquid {
    range: i32,
    speed: f32,
    /// The `MutableLong` of the behaviour.
    next: i64,
}

impl TryFindLiquid {
    fn new(range: i32, speed: f32) -> Box<dyn brain::Control> {
        brain::Shot::new(TryFindLiquid { range, speed, next: 0 })
    }
}

fn is_water_fluid(level: &dyn EntityLevel, p: BlockPos) -> bool {
    crate::physics::fluid_state(level.block(p)).kind.is_water()
}

impl ShotBehavior for TryFindLiquid {
    fn name(&self) -> &'static str {
        "TryFindLiquid"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let here = cx.e.block_position();
        if is_water_fluid(&*cx.level, here) {
            return false;
        }
        if cx.time < self.next {
            self.next = cx.time + 20 + 2;
            return true;
        }
        let mut found: Option<BlockPos> = None;
        let pos = cx.e.position();
        for p in super::turtle::within_manhattan(here, self.range, self.range, self.range) {
            // `filterPos(differsHorizontally)`, `filterState(a water block)`.
            if p.x == here.x && p.z == here.z {
                continue;
            }
            if crate::blocks::block_name(cx.level.block(p)) != "minecraft:water" {
                continue;
            }
            if kiln_data::blocks_types::is_air(cx.level.block(p.above())) {
                found = Some(p);
                break;
            }
            if found.is_none() {
                let c = Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
                if !(c.distance_to_sqr(pos) < 1.5 * 1.5) {
                    found = Some(p);
                }
            }
        }
        if let Some(p) = found {
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(p)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(p, self.speed, 0)));
        }
        self.next = cx.time + 40;
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `RandomStroll.getTargetSwimPos`: a swimmable spot at growing distances (1, 3, 5, 6, 7, then
/// 10 blocks across and 7 up), each further one in the direction of the last, while the spots are
/// in a fluid; the last good one (or none) is the target.
fn target_swim_pos(cx: &mut Cx) -> Option<Vec3> {
    const TIERS: [(i32, i32); 6] = [(1, 1), (3, 3), (5, 5), (6, 5), (7, 7), (10, 7)];
    let mut result: Option<Vec3> = None;
    let mut candidate: Option<Vec3> = None;
    for (h, v) in TIERS {
        candidate = match result {
            None => crate::mob::path::random_swimmable_pos(cx.e, cx.m, &*cx.level, h, v),
            Some(r) => {
                let p = cx.e.position();
                Some(p + (r - p).normalize().multiply(h as f64, v as f64, h as f64))
            }
        };
        // (`GoalUtils.mobRestricted`: no axolotl has a restriction.)
        match candidate {
            Some(c) if !crate::physics::fluid_state(cx.level.block(BlockPos::containing(c.x, c.y, c.z))).is_empty() => result = Some(c),
            _ => return result,
        }
    }
    candidate
}

/// `RandomStroll.swim(speed)`: in the water, the next swim target.
fn swim_stroll(speed: f32) -> Box<dyn brain::Control> {
    shot("RandomStroll", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        if !cx.e.is_in_water() {
            return false;
        }
        match target_swim_pos(cx) {
            Some(p) => cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, speed, 0))),
            None => cx.b.mem.erase(Mem::WalkTarget),
        }
        true
    })
}

/// `SetWalkTargetFromLookTarget.create(canSetWalkTargetFromLookTarget, getSpeedModifier, 3)`: only
/// toward a spot in the same medium (water or not) as the axolotl.
fn set_walk_target_from_look_target_same_medium() -> Box<dyn brain::Control> {
    shot("SetWalkTargetFromLookTarget", &[(Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, ValuePresent)], |cx| {
        let Some(t) = cx.b.mem.look_target() else { return false };
        let Some(p) = util::tracker_block(cx, &t) else { return false };
        if is_water_fluid(&*cx.level, p) != cx.e.is_in_water() {
            return false;
        }
        let speed = speed_modifier(cx);
        cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed, close_enough: 3 }));
        true
    })
}

// ---------------------------------------------------------------------- the brain

/// `AxolotlAi.getActivities` with the sensors of `Axolotl.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Adult { any_type: false }),
        Box::new(sensors::HurtBy),
        Box::new(AxolotlAttackables),
        Box::new(sensors::Tempting::for_animal()),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![LookAtTargetSink::new(45, 90), MoveToTargetSink::new(), validate_play_dead(), CountDownCooldownTicks::new(Mem::TemptationCooldownTicks)],
    );
    let idle = ActivityData::with_priorities(
        Activity::Idle,
        vec![
            (0, SetEntityLookTargetSometimes::new(Some("minecraft:player"), 6.0, (30, 60))),
            (1, AnimalMakeLove::new("minecraft:axolotl", 0.2, 2)),
            (
                2,
                Gate::run_one(vec![
                    (FollowTemptation::new(speed_modifier), 1),
                    (baby_follow_adult((5, 16), speed_modifier_fast, Mem::NearestVisibleAdult, false), 1),
                ]),
            ),
            (3, start_attacking(|_| true, find_nearest_valid_attack_target)),
            (3, TryFindLiquid::new(6, 0.15)),
            (
                4,
                // The two `triggerIf` entries (in water, on the ground) never act.
                Gate::new(
                    "GateBehavior",
                    &[(Mem::WalkTarget, ValueAbsent)],
                    &[],
                    OrderPolicy::Ordered,
                    RunningPolicy::TryAll,
                    vec![
                        (swim_stroll(0.5), 2),
                        (stroll(0.15, StrollKind::Land { avoid_water: true }), 2),
                        (set_walk_target_from_look_target_same_medium(), 3),
                    ],
                ),
            ),
        ],
    );
    let fight = ActivityData::full(
        Activity::Fight,
        vec![
            (0, stop_attacking_if_target_invalid(|_, _| false, on_stop_attacking, true)),
            (1, set_walk_target_from_attack_target_if_out_of_reach(speed_modifier_fast)),
            (2, melee_attack(20)),
            (3, erase_if_breeding(Mem::AttackTarget)),
        ],
        &[(Mem::AttackTarget, ValuePresent)],
        &[Mem::AttackTarget],
    );
    let play_dead = ActivityData::full(
        Activity::PlayDead,
        vec![(0, Timed::new(PlayDead)), (1, erase_if_breeding(Mem::PlayDeadTicks))],
        &[(Mem::PlayDeadTicks, ValuePresent)],
        &[Mem::PlayDeadTicks],
    );
    Brain::new(&[], sensors, vec![core, idle, fight, play_dead], random)
}

/// `AxolotlAi.updateActivity`: playing dead, then fighting, then idling; leaving a fight starts
/// the 2400-tick hunting cooldown.
fn update_activity(m: &mut MobData) {
    let Some(b) = m.brain.as_mut() else { return };
    let before = b.st.active_non_core();
    if before != Some(Activity::PlayDead) {
        b.st.set_active_activity_to_first_valid(&[Activity::PlayDead, Activity::Fight, Activity::Idle]);
        if before == Some(Activity::Fight) && b.st.active_non_core() != Some(Activity::Fight) {
            b.st.mem.set_expiring(Mem::HasHuntingCooldown, Val::Bool(true), 2400);
        }
    }
}

// ---------------------------------------------------------------------- the kind

impl Kind for Axolotl {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        // `AmphibiousPathNavigation` (it cannot float), water is free to path through.
        super::tame::set_malus(m, PathType::Water, 0.0);
        m.nav.amphibious = true;
        m.air_supply_max = TOTAL_AIR_SUPPLY;
        Some(Box::new(State { variant: 0, playing_dead: false, from_bucket: false }))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:axolotl_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    /// The brain, `AxolotlAi.updateActivity`, then the playing-dead flag (`customServerAiStep`,
    /// which does not call `Animal`'s).
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        update_activity(m);
        // `getTarget` reads the brain.
        m.target = m.brain.as_ref().and_then(|b| b.st.mem.entity(Mem::AttackTarget));
        if !m.no_ai {
            let dead = m.brain.as_ref().and_then(|b| b.st.mem.int(Mem::PlayDeadTicks)).is_some_and(|t| t > 0);
            st_mut(m).playing_dead = dead;
        }
    }

    /// `AxolotlMoveControl` (`SmoothSwimmingMoveControl(85, 10, 0.1, 0.5, false)`): still while
    /// playing dead.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !is_playing_dead(m) {
            smooth_swim_move(e, m, 85, 10, 0.1, 0.5, false);
        }
        true
    }

    /// `AxolotlLookControl` (`SmoothSwimmingLookControl(20)`): still while playing dead.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !is_playing_dead(m) {
            smooth_swim_look(e, m, 20);
        }
        true
    }

    /// `travelInWater`: no water drag beyond 0.9 and no gravity.
    fn travel_in_water(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        mob::move_relative(e, m.speed, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        true
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn pushed_by_fluid(&self) -> bool {
        false
    }

    fn swim_sound(&self) -> Option<&'static str> {
        Some("minecraft:entity.axolotl.swim")
    }

    /// `getAmbientSound` (none while playing dead: `playAmbientSound`).
    fn ambient_sound(&self, e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        if is_playing_dead(m) {
            return Some(None);
        }
        Some(Some(mob::sound_event(if e.is_in_water() { "minecraft:entity.axolotl.idle_water" } else { "minecraft:entity.axolotl.idle_air" })))
    }

    /// `WaterAnimal`-like air: `Axolotl.handleAirSupply` (6000 ticks, hurts for 2 as `dryOut`).
    fn after_base_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
        if m.no_ai {
            return;
        }
        if mob::is_alive(e, m) && !in_water_or_rain(e, &*level) {
            e.air_supply = air_before - 1;
            if e.air_supply <= -20 {
                e.air_supply = 0;
                mob::hurt(e, m, level, DamageSource::of(DamageKind::DryOut), 2.0);
            }
        } else {
            e.air_supply = TOTAL_AIR_SUPPLY;
        }
    }

    /// `hurtServer`: a hit in the water now and then (or a bad one) makes it play dead.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let health = m.health;
        if !m.no_ai && e.random.next_int_bounded(3) == 0 {
            let hard = (e.random.next_int_bounded(3) as f32) < amount || health / m.max_health() < 0.5;
            if hard && amount < health && e.is_in_water() && (source.attacker.is_some() || source.direct.is_some()) && !is_playing_dead(m)
                && let Some(b) = m.brain.as_mut()
            {
                b.st.mem.set(Mem::PlayDeadTicks, Val::Int(200));
            }
        }
        Some(mob::hurt_base(e, m, level, *source, amount))
    }

    /// `playAttackSound`.
    fn after_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _t: &Living) {
        mob::play_sound(e, m, level, "minecraft:entity.axolotl.attack", 1.0, 1.0);
    }

    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        Some(!st(m).from_bucket)
    }

    /// `Axolotl.getBreedOffspring`: a 1 in 1200 chance of a blue baby, else one parent's color.
    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, _level: &mut dyn EntityLevel) {
        let v = if e.random.next_int_bounded(1200) == 0 {
            spawn_variant(&mut e.random, false)
        } else if e.random.next_bool() {
            st(m).variant
        } else {
            st(partner).variant
        };
        st_mut(child).variant = v;
        child.persistence_required = true;
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `Bucketable.bucketMobPickup`.
        if !stack.is_empty() && mob::item_name(stack) == "minecraft:water_bucket" && mob::is_alive(e, m) {
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.bucket.fill_axolotl", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
            let mut filled = ItemStack::of("minecraft:axolotl_bucket", 1)?;
            save_to_bucket(e, m, &mut filled);
            if let Some(b) = m.brain.as_mut() {
                b.st.mem.clear_all();
            }
            e.discard();
            return Some(Outcome::success(HeldChange::Fill(filled)));
        }
        // `Animal.mobInteract`; a tropical fish bucket comes back as a water bucket (`usePlayerItem`).
        let mut out = interact::animal_interact(e, m, level, who, stack);
        if !out.success {
            return None;
        }
        if !stack.is_empty() && mob::item_name(stack) == "minecraft:tropical_fish_bucket" && out.held == HeldChange::Consume(1) {
            out.held = HeldChange::Fill(ItemStack::of("minecraft:water_bucket", 1)?);
        }
        Some(out)
    }

    /// `checkAxolotlSpawnRules`: on clay (`#axolotls_spawnable_on`).
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(super::wolf::block_in_tag(view.block(pos.below()), "minecraft:axolotls_spawnable_on"))
    }

    fn placement(&self) -> Placement {
        Placement::InWater
    }

    fn spawn_in_liquids(&self) -> bool {
        true
    }

    fn spawn_ignores_light(&self) -> bool {
        true
    }

    /// `BABY_DIMENSIONS`: 0.375 by 0.21, eyes at 0.09375.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.375, 0.21, 0.09375) } else { base }
    }

    /// `Axolotl.finalizeSpawn`: a group of common variants; from the third member on, babies.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        let mut baby = false;
        let types = match group.variant {
            Some(packed) => {
                if group.ageable_group_size >= 2 {
                    baby = true;
                }
                (packed & 0xF, (packed >> 4) & 0xF)
            }
            None => {
                let (a, b) = (spawn_variant(r, true), spawn_variant(r, true));
                group.variant = Some(a | (b << 4));
                (a, b)
            }
        };
        st_mut(m).variant = if r.next_int_bounded(2) == 0 { types.0 } else { types.1 };
        if baby {
            mob::set_age(e, m, mob::breed::BABY_START_AGE);
        }
        group.ageable_group_size += 1;
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let v = r.int_or("Variant", 0);
        let from_bucket = r.bool_or("FromBucket", false);
        let s = st_mut(m);
        s.variant = if (0..VARIANTS.len() as i32).contains(&v) { v } else { 0 };
        s.from_bucket = from_bucket;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("Variant", Tag::Int(s.variant));
        o.put("FromBucket", Tag::Byte(s.from_bucket as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(kiln_data::entities::data::axolotl::VARIANT, &DataValue::Int(s.variant));
        d.set(kiln_data::entities::data::axolotl::PLAYING_DEAD, &DataValue::Boolean(s.playing_dead));
        d.set(kiln_data::entities::data::axolotl::FROM_BUCKET, &DataValue::Boolean(s.from_bucket));
    }
}

// ---------------------------------------------------------------------- buckets

/// `Axolotl.saveToBucketTag` (with `Bucketable.saveDefaultDataToBucketTag`): the variant as a
/// component, the mob's state in `bucket_entity_data`.
fn save_to_bucket(e: &Entity, m: &MobData, bucket: &mut ItemStack) {
    let mut tag: Vec<(String, Tag)> = Vec::new();
    if m.no_ai {
        tag.push(("NoAI".into(), Tag::Byte(1)));
    }
    if e.silent {
        tag.push(("Silent".into(), Tag::Byte(1)));
    }
    if e.no_gravity {
        tag.push(("NoGravity".into(), Tag::Byte(1)));
    }
    if e.invulnerable {
        tag.push(("Invulnerable".into(), Tag::Byte(1)));
    }
    if m.persistence_required {
        tag.push(("PersistenceRequired".into(), Tag::Byte(1)));
    }
    tag.push(("Health".into(), Tag::Float(m.health)));
    tag.push(("Age".into(), Tag::Int(m.age)));
    tag.push(("AgeLocked".into(), Tag::Byte(m.age_locked as i8)));
    if let Some(b) = m.brain.as_ref()
        && b.st.mem.has(Mem::HasHuntingCooldown)
    {
        tag.push(("HuntingCooldown".into(), Tag::Long(b.st.mem.time_until_expiry(Mem::HasHuntingCooldown))));
    }
    bucket.insert(kiln_item::keys::BUCKET_ENTITY_DATA, kiln_item::component::CustomData(Tag::Compound(tag)));
    if let Some(v) = kiln_item::component::variant::AxolotlVariant::ALL.get(st(m).variant as usize) {
        bucket.insert(kiln_item::keys::AXOLOTL_VARIANT, *v);
    }
}

/// `MobBucketItem.spawn` on an axolotl just made: the bucket's variant component, then
/// `loadFromBucketTag` (`Bucketable.loadDefaultDataFromBucketTag`, the age and the hunting
/// cooldown) and `setFromBucket(true)`.
pub fn apply_bucket(e: &mut Entity, bucket: &ItemStack) {
    let Some(m) = mob::data_mut(e) else { return };
    if ext::state::<State>(m).is_none() {
        return;
    }
    if let Some(v) = bucket.get(kiln_item::keys::AXOLOTL_VARIANT) {
        st_mut(m).variant = v.id();
    }
    st_mut(m).from_bucket = true;
    let tag = bucket.get(kiln_item::keys::BUCKET_ENTITY_DATA).map(|c| c.0.clone());
    if let Some(Tag::Compound(fields)) = tag {
        for (k, v) in &fields {
            let on = v.as_f64().is_some_and(|b| b != 0.0);
            match k.as_str() {
                "NoAI" => m.no_ai = on,
                "PersistenceRequired" if on => m.persistence_required = true,
                "Health" => {
                    if let Some(h) = v.as_f64() {
                        m.set_health(h as f32);
                    }
                }
                "Age" => m.age = v.as_f64().unwrap_or(0.0) as i32,
                "AgeLocked" => m.age_locked = on,
                "HuntingCooldown" => {
                    if let (Some(b), Some(t)) = (m.brain.as_mut(), v.as_f64()) {
                        b.st.mem.set_expiring(Mem::HasHuntingCooldown, Val::Bool(true), t as i64);
                    }
                }
                _ => {}
            }
        }
        for (k, v) in &fields {
            let on = v.as_f64().is_some_and(|b| b != 0.0);
            match k.as_str() {
                "Silent" => e.silent = on,
                "NoGravity" => e.no_gravity = on,
                "Invulnerable" => e.invulnerable = on,
                _ => {}
            }
        }
    }
    if let Some(m) = mob::data(e).cloned() {
        mob::refresh_dimensions(e, &m);
    }
}

/// The variant components entity predicates see (`axolotl/variant`).
pub fn variant_components(m: &MobData) -> Option<Vec<kiln_item::Component>> {
    let v = *kiln_item::component::variant::AxolotlVariant::ALL.get(variant(m) as usize)?;
    Some(vec![kiln_item::Component::AxolotlVariant(v)])
}
