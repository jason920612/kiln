//! Hoglin: attacks players and throws them, breeds with crimson fungus, flees warped fungus.
//!
//! Driven by the brain of `HoglinAi` (core: looking and moving; idle: giving up near repellents,
//! breeding, avoiding repellents and piglins, attacking the nearest visible player, following
//! adults, looking and strolling; fight: melee every 40 ticks, 15 for babies; avoid: running
//! from piglins that outnumber the hoglins), on [`crate::mob::brain`]. The hit's random damage
//! and the upward throw are vanilla's `HoglinBase`. Outside the nether an adult that is not
//! immune turns into a zoglin after 300 ticks.

use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::{set_walk_target_from_attack_target_if_out_of_reach, start_attacking, stop_attacking_if_target_invalid_default};
use crate::mob::brain::memory::Val;
use crate::mob::brain::nether::*;
use crate::mob::brain::sensors;
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Control, Cx, Gate, Mem, Status, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::Living;
use crate::mob::interact::{Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, SpawnContext, mth};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::ValuePresent;

pub struct Hoglin;

pub static KIND: Hoglin = Hoglin;

static INFO: Info = Info {
    animal: true,
    ageable: true,
    sounds: Some("hoglin"),
    sound_source: "hostile",
    extends_monster: false,
    ..Info::monster("minecraft:hoglin", &[(MaxHealth, 40.0), (MovementSpeed, 0.30000001192092896), (KnockbackResistance, 0.6000000238418579), (AttackKnockback, 1.0), (AttackDamage, 6.0)])
};

/// `Hoglin.CONVERSION_TIME`.
pub const CONVERSION_TIME: i32 = 300;

/// `HoglinAi.RETREAT_DURATION` (`TimeUtil.rangeOfSeconds(5, 20)`).
const RETREAT_DURATION: (i32, i32) = seconds(5, 20);

#[derive(Clone, Debug, Default)]
pub struct HoglinState {
    pub immune_to_zombification: bool,
    pub time_in_overworld: i32,
    pub cannot_be_hunted: bool,
    pub attack_animation: i32,
    /// The value of `NEAREST_REPELLENT` (the walk target value reads it while the brain is out of
    /// the mob).
    pub repellent: Option<BlockPos>,
    /// Attackers whose hits the hoglin reacts to at the start of its next brain tick.
    pub pending_hurt: Vec<i32>,
}

pub fn state(m: &MobData) -> Option<&HoglinState> {
    ext::state::<HoglinState>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut HoglinState> {
    ext::state_mut::<HoglinState>(m)
}

/// The sensor's `NEAREST_REPELLENT` (for `getWalkTargetValue`).
pub fn remember_repellent(m: &mut MobData, p: Option<BlockPos>) {
    if let Some(s) = state_mut(m) {
        s.repellent = p;
    }
}

/// The stream vanilla's `level.getRandom()` is for a mob (see [`Cx::rng`]).
fn ai_rng<'a>(level: &'a mut dyn EntityLevel, m: &'a mut MobData) -> &'a mut LegacyRandom {
    match level.shared_ai_random() {
        Some(r) => r,
        None => &mut m.brain_random,
    }
}

/// `HoglinBase.throwTarget`: the target flies up and away, turned by a random angle (given to
/// `Vec3.yRot` in degrees-sized radians, as vanilla does).
pub fn throw_target(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, target: i32, resistance: f64) {
    let strength = m.attrs.value(AttackKnockback) - resistance;
    if strength <= 0.0 {
        return;
    }
    let Some(t) = level.entity(target).map(|t| t.position()).or_else(|| level.player(target).map(|p| p.pos)) else { return };
    let (dx, dz) = (t.x - e.x(), t.z - e.z());
    let r = ai_rng(level, m);
    let angle = (r.next_int_bounded(21) - 10) as f32;
    let horiz = strength * (r.next_float() * 0.5 + 0.2) as f64;
    let v = Vec3::new(dx, 0.0, dz).normalize().scale(horiz);
    let (c, s) = (mth::cos(angle as f64) as f64, mth::sin(angle as f64) as f64);
    let v = Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s);
    let up = strength * r.next_float() as f64 * 0.5;
    if let Some(o) = level.entity_mut(target) {
        o.delta = o.delta.add(v.x, up, v.z);
        o.needs_sync = true;
    }
}

/// `HoglinBase.hurtAndThrowTarget`.
pub fn hurt_and_throw_target(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> bool {
    let base = m.attrs.value(AttackDamage) as f32;
    let damage = if !m.baby() && base as i32 > 0 { base / 2.0 + ai_rng(level, m).next_int_bounded(base as i32) as f32 } else { base };
    let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    let hurt = mob::hurt_living(level, t, source, damage);
    if hurt {
        m.last_hurt_mob = Some(t.id);
        if !m.baby() {
            let resistance = level.entity(t.id).and_then(mob::data).map_or(0.0, |o| o.attrs.value(KnockbackResistance));
            throw_target(e, m, level, t.id, resistance);
        }
    }
    hurt
}

/// `ageBoundaryReached`: babies hit for 0.5.
fn update_attack_damage(m: &mut MobData) {
    let base = if m.baby() { 0.5 } else { 6.0 };
    if let Some(a) = m.attrs.get_mut(AttackDamage) {
        a.base = base;
    }
}

// ---------------------------------------------------------------------------- HoglinAi

fn is_pacified(cx: &Cx) -> bool {
    cx.b.mem.has(Mem::Pacified)
}

/// `HoglinAi.findNearestValidAttackTarget`.
fn find_nearest_valid_attack_target(cx: &mut Cx) -> Option<i32> {
    if is_pacified(cx) || cx.b.mem.has(Mem::BreedTarget) {
        return None;
    }
    cx.b.mem.entity(Mem::NearestVisibleAttackablePlayer)
}

/// `HoglinAi.piglinsOutnumberHoglins`.
fn piglins_outnumber_hoglins(cx: &Cx) -> bool {
    if cx.m.baby() {
        return false;
    }
    let piglins = cx.b.mem.int(Mem::VisibleAdultPiglinCount).unwrap_or(0);
    let hoglins = cx.b.mem.int(Mem::VisibleAdultHoglinCount).unwrap_or(0) + 1;
    piglins > hoglins
}

fn wants_to_stop_fleeing(cx: &mut Cx) -> bool {
    !cx.m.baby() && !piglins_outnumber_hoglins(cx)
}

fn is_adult(cx: &mut Cx) -> bool {
    !cx.m.baby()
}

fn is_baby(cx: &mut Cx) -> bool {
    cx.m.baby()
}

fn is_breeding(cx: &mut Cx) -> bool {
    cx.b.mem.has(Mem::BreedTarget)
}

/// `HoglinAi.setAttackTarget`.
fn set_attack_target(cx: &mut Cx, target: &Living) {
    cx.b.mem.erase(Mem::CantReachWalkTargetSince);
    cx.b.mem.erase(Mem::BreedTarget);
    cx.b.mem.set_expiring(Mem::AttackTarget, Val::Entity(target.id), 200);
}

/// `HoglinAi.setAttackTargetIfCloserThanCurrent`.
fn set_attack_target_if_closer_than_current(cx: &mut Cx, target: &Living) {
    if is_pacified(cx) {
        return;
    }
    let current = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id));
    let n = match &current {
        None => target.id,
        Some(c) => util::nearest_of(cx, c, target),
    };
    let t = if n == target.id { target.clone() } else { current.expect("current is the nearest") };
    set_attack_target(cx, &t);
}

/// `HoglinAi.broadcastAttackTarget`.
fn broadcast_attack_target(cx: &mut Cx, target: &Living) {
    let others = cx.b.mem.entities(Mem::NearestVisibleAdultHoglins).to_vec();
    for id in others {
        let t = target.clone();
        as_mob(cx, id, |c| set_attack_target_if_closer_than_current(c, &t));
    }
}

/// `HoglinAi.setAvoidTarget`.
fn set_avoid_target(cx: &mut Cx, target: &Living) {
    cx.b.mem.erase(Mem::AttackTarget);
    cx.b.mem.erase(Mem::WalkTarget);
    let n = sample(cx.rng(), RETREAT_DURATION.0, RETREAT_DURATION.1) as i64;
    cx.b.mem.set_expiring(Mem::AvoidTarget, Val::Entity(target.id), n);
}

/// `HoglinAi.retreatFromNearestTarget`.
fn retreat_from_nearest_target(cx: &mut Cx, target: &Living) {
    let mut t = target.clone();
    let avoid = cx.b.mem.entity(Mem::AvoidTarget).and_then(|id| util::living(cx, id));
    if let Some(a) = avoid
        && util::nearest_of(cx, &a, &t) != t.id
    {
        t = a;
    }
    let attack = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id));
    if let Some(a) = attack
        && util::nearest_of(cx, &a, &t) != t.id
    {
        t = a;
    }
    set_avoid_target(cx, &t);
}

/// `HoglinAi.broadcastRetreat`.
fn broadcast_retreat(cx: &mut Cx, target: &Living) {
    let others = cx.b.mem.entities(Mem::NearestVisibleAdultHoglins).to_vec();
    for id in others {
        let t = target.clone();
        as_mob(cx, id, |c| retreat_from_nearest_target(c, &t));
    }
}

/// `HoglinAi.onHitTarget`.
pub fn on_hit_target(cx: &mut Cx, target: &Living) {
    if cx.m.baby() {
        return;
    }
    if target.type_name == PIGLIN && piglins_outnumber_hoglins(cx) {
        set_avoid_target(cx, target);
        broadcast_retreat(cx, target);
        return;
    }
    broadcast_attack_target(cx, target);
}

/// `HoglinAi.maybeRetaliate`.
fn maybe_retaliate(cx: &mut Cx, attacker: &Living) {
    if cx.b.is_active(Activity::Avoid) && attacker.type_name == PIGLIN {
        return;
    }
    if attacker.type_name == HOGLIN {
        return;
    }
    if util::other_target_much_further(cx, attacker, 4.0) {
        return;
    }
    if !util::is_entity_attackable(cx, attacker) {
        return;
    }
    set_attack_target(cx, attacker);
    broadcast_attack_target(cx, attacker);
}

/// `HoglinAi.wasHurtBy`.
pub fn was_hurt_by(cx: &mut Cx, attacker: &Living) {
    cx.b.mem.erase(Mem::Pacified);
    cx.b.mem.erase(Mem::BreedTarget);
    if cx.m.baby() {
        retreat_from_nearest_target(cx, attacker);
        return;
    }
    maybe_retaliate(cx, attacker);
}

/// The hoglin's melee (`MeleeAttack.create(cooldown)`), which tells the brain about the hit
/// (`HoglinAi.onHitTarget`) between the attack sound and the damage.
fn melee(cooldown: i32) -> Box<dyn Control> {
    shot(
        "MeleeAttack",
        &[
            (Mem::LookTarget, Status::Registered),
            (Mem::AttackTarget, ValuePresent),
            (Mem::AttackCoolingDown, Status::ValueAbsent),
            (Mem::NearestVisibleLivingEntities, ValuePresent),
        ],
        move |cx| {
            let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
            let Some(t) = util::living(cx, id) else { return false };
            if util::within_melee(cx, &t) && util::visible_contains(cx, id) {
                cx.b.mem.set(Mem::LookTarget, Val::Look(brain::Tracker::entity(id, true)));
                cx.m.swing = true;
                // `Hoglin.doHurtTarget`.
                if let Some(s) = state_mut(cx.m) {
                    s.attack_animation = 10;
                }
                cx.level.emit(Event::EntityEvent { entity: cx.e.id, event: 4 });
                mob::make_sound(cx.e, cx.m, cx.level, mob::sound_event("minecraft:entity.hoglin.attack"));
                on_hit_target(cx, &t);
                hurt_and_throw_target(cx.e, cx.m, cx.level, &t);
                cx.b.mem.set_expiring(Mem::AttackCoolingDown, Val::Bool(true), cooldown as i64);
                return true;
            }
            false
        },
    )
}

/// `HoglinAi.createIdleMovementBehaviors`.
fn idle_movement_behaviors() -> Box<dyn Control> {
    Gate::run_one(vec![
        (stroll(0.4, StrollKind::Land { avoid_water: true }), 2),
        (set_walk_target_from_look_target(0.4, 3), 2),
        (DoNothing::new(30, 60), 1),
    ])
}

fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Players),
        Box::new(sensors::Adult { any_type: false }),
        Box::new(HoglinSpecific),
    ];
    let core = ActivityData::create(Activity::Core, 0, vec![LookAtTargetSink::new(45, 90), MoveToTargetSink::new()]);
    let idle = ActivityData::create(
        Activity::Idle,
        10,
        vec![
            become_passive_if_memory_present(Mem::NearestRepellent, 200),
            AnimalMakeLove::new(HOGLIN, 0.6, 2),
            SetWalkTargetAwayFrom::pos(Mem::NearestRepellent, 1.0, 8, true),
            start_attacking(|_| true, find_nearest_valid_attack_target),
            trigger_if(is_adult, SetWalkTargetAwayFrom::entity(Mem::NearestVisibleAdultPiglin, 0.4, 8, false)),
            SetEntityLookTargetSometimes::new(None, 8.0, (30, 60)),
            baby_follow_adult((5, 16), |_| 0.6, Mem::NearestVisibleAdult, false),
            idle_movement_behaviors(),
        ],
    );
    let fight = ActivityData::full(
        Activity::Fight,
        numbered(
            10,
            vec![
                become_passive_if_memory_present(Mem::NearestRepellent, 200),
                AnimalMakeLove::new(HOGLIN, 0.6, 2),
                set_walk_target_from_attack_target_if_out_of_reach(|_| 1.0),
                trigger_if(is_adult, melee(40)),
                trigger_if(is_baby, melee(15)),
                stop_attacking_if_target_invalid_default(),
                erase_memory_if(is_breeding, Mem::AttackTarget),
            ],
        ),
        &[(Mem::AttackTarget, ValuePresent)],
        &[Mem::AttackTarget],
    );
    let avoid = ActivityData::full(
        Activity::Avoid,
        numbered(
            10,
            vec![
                SetWalkTargetAwayFrom::entity(Mem::AvoidTarget, 1.3, 15, false),
                idle_movement_behaviors(),
                SetEntityLookTargetSometimes::new(None, 8.0, (30, 60)),
                erase_memory_if(wants_to_stop_fleeing, Mem::AvoidTarget),
            ],
        ),
        &[(Mem::AvoidTarget, ValuePresent)],
        &[Mem::AvoidTarget],
    );
    Brain::new(&[], sensors, vec![core, idle, fight, avoid], random)
}

pub(crate) fn numbered(start: i32, list: Vec<Box<dyn Control>>) -> Vec<(i32, Box<dyn Control>)> {
    list.into_iter().enumerate().map(|(i, b)| (start + i as i32, b)).collect()
}

/// `Hoglin.isConverting`.
fn is_converting(m: &MobData, level: &dyn EntityLevel) -> bool {
    !state(m).is_some_and(|s| s.immune_to_zombification) && !m.no_ai && level.piglins_zombify()
}

/// `HoglinAi.getSoundForActivity`.
fn sound_for_activity(m: &MobData, level: &dyn EntityLevel, mem: &brain::Memories, a: Activity) -> &'static str {
    let s = |n: &str| mob::sound_event(n);
    if a == Activity::Avoid || is_converting(m, level) {
        return s("minecraft:entity.hoglin.retreat");
    }
    if a == Activity::Fight {
        return s("minecraft:entity.hoglin.angry");
    }
    if mem.has(Mem::NearestRepellent) {
        return s("minecraft:entity.hoglin.retreat");
    }
    s("minecraft:entity.hoglin.ambient")
}

/// `HoglinAi.updateActivity`.
fn update_activity(cx: &mut Cx) {
    let old = cx.b.active_non_core();
    cx.b.set_active_activity_to_first_valid(&[Activity::Fight, Activity::Avoid, Activity::Idle]);
    let new = cx.b.active_non_core();
    if old != new
        && let Some(a) = new
    {
        let sound = sound_for_activity(cx.m, &*cx.level, &cx.b.mem, a);
        mob::make_sound(cx.e, cx.m, cx.level, sound);
    }
    let aggressive = cx.b.mem.has(Mem::AttackTarget);
    cx.m.set_aggressive(aggressive);
}

fn process_pending_hurt(cx: &mut Cx) {
    let pending = state_mut(cx.m).map(|s| std::mem::take(&mut s.pending_hurt)).unwrap_or_default();
    for id in pending {
        if let Some(a) = util::living(cx, id) {
            was_hurt_by(cx, &a);
        }
    }
}

fn sync_target(m: &mut MobData) {
    m.target = m.brain.as_ref().and_then(|b| b.st.mem.entity(Mem::AttackTarget));
}

impl Kind for Hoglin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(HoglinState::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn ai_step_before(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if let Some(st) = state_mut(m)
            && st.attack_animation > 0
        {
            st.attack_animation -= 1;
        }
    }

    fn post_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        update_attack_damage(m);
    }

    /// The brain, `HoglinAi.updateActivity`, then the zoglin conversion.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                process_pending_hurt(&mut cx);
            }
            m.brain = Some(b);
        }
        brain::tick_brain(e, m, level);
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                update_activity(&mut cx);
            }
            m.brain = Some(b);
        }
        sync_target(m);
        let converting = is_converting(m, &*level);
        let Some(st) = state_mut(m) else { return };
        if converting {
            st.time_in_overworld += 1;
            if st.time_in_overworld > CONVERSION_TIME {
                mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.hoglin.converted_to_zombified"));
                finish_conversion(e, m, level);
            }
        } else {
            st.time_in_overworld = 0;
        }
    }

    /// `Hoglin.hurtServer`: a hit by a living entity is the brain's business.
    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if !hurt {
            return;
        }
        let Some(a) = source.attacker else { return };
        match crate::mob::goals::living(&*level, a) {
            None => {
                if let Some(s) = state_mut(m) {
                    s.pending_hurt.push(a);
                }
            }
            Some(attacker) => {
                if let Some(mut brain) = m.brain.take() {
                    let time = level.game_time();
                    {
                        let mut cx = Cx { e, m, level, b: &mut brain.st, time };
                        was_hurt_by(&mut cx, &attacker);
                    }
                    m.brain = Some(brain);
                }
            }
        }
        sync_target(m);
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        if let Some(st) = state_mut(m) {
            st.attack_animation = 10;
        }
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.hoglin.attack"));
        Some(hurt_and_throw_target(e, m, level, t))
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        if r.next_float() < 0.2 {
            mob::set_age(e, m, mob::breed::BABY_START_AGE);
        }
        ext::ageable_finalize(e, m, r, group, 0.05);
        update_attack_damage(m);
        ext::mob_finalize(m, r);
    }

    fn breed_offspring(&self, _e: &mut Entity, _m: &mut MobData, _partner: &MobData, child: &mut MobData, _level: &mut dyn EntityLevel) {
        // `Hoglin.getBreedOffspring`: `setPersistenceRequired`.
        child.persistence_required = true;
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let immune = r.bool_or("IsImmuneToZombification", false);
        let time = r.int_or("TimeInOverworld", 0);
        let hunted = r.bool_or("CannotBeHunted", false);
        update_attack_damage(m);
        let Some(st) = state_mut(m) else { return };
        st.immune_to_zombification = immune;
        st.time_in_overworld = time;
        st.cannot_be_hunted = hunted;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = state(m) else { return };
        if st.immune_to_zombification {
            o.put("IsImmuneToZombification", Tag::Byte(1));
        }
        o.put("TimeInOverworld", Tag::Int(st.time_in_overworld));
        if st.cannot_be_hunted {
            o.put("CannotBeHunted", Tag::Byte(1));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let immune = state(m).is_some_and(|s| s.immune_to_zombification);
        d.set(kiln_data::entities::data::hoglin::IMMUNE_TO_ZOMBIFICATION, &DataValue::Boolean(immune));
    }

    fn interact(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `Hoglin.mobInteract`: feeding (the shared animal code) also makes it persistent; a
        // pacified adult does not fall in love (`canFallInLove`).
        if stack.is_empty() || !self.is_food(stack.item()) {
            return None;
        }
        let pacified = m.brain.as_ref().is_some_and(|b| b.st.mem.has(Mem::Pacified));
        if m.age >= 0 && pacified {
            return Some(Outcome::PASS);
        }
        if (m.age == 0 && m.in_love <= 0) || (m.age < 0 && !m.age_locked) {
            m.persistence_required = true;
        }
        None
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:hoglin_food")
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.75, 0.85, 0.625) } else { base }
    }

    /// `Hoglin.getWalkTargetValue`: near a repellent -1, crimson nylium 10.
    fn walk_target_value(&self, m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        if state(m).and_then(|s| s.repellent).is_some_and(|r| util::dist_sqr_pos(r, p) < 64.0) {
            return Some(-1.0);
        }
        Some(if crate::blocks::block_name(level.block(p.below())) == "minecraft:crimson_nylium" { 10.0 } else { 0.0 })
    }

    fn experience(&self, _e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(if m.baby() { 3 } else { 5 })
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(true)
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let b = m.brain.as_ref()?;
        let a = b.st.active_non_core()?;
        Some(Some(sound_for_activity(m, level, &b.st.mem, a)))
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(crate::blocks::block_name(view.block(pos.below())) != "minecraft:nether_wart_block")
    }
}

/// `Hoglin.finishConversion`: a zoglin takes its place (equipment kept, the baby flag carried
/// over), with nausea.
fn finish_conversion(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let baby = m.baby();
    crate::mob::convert::convert_to(e, m, level, MobKind::Zoglin, true, false, move |ne, nm, level| {
        if baby {
            crate::mob::kinds::zoglin::set_baby(ne, nm, true);
        }
        if let Some(fx) = crate::effect::Effect::named("minecraft:nausea", 200, 0) {
            crate::mob::effects::add(ne, nm, level, fx, None);
        }
    });
}
