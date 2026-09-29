//! Camel (`Camel`, an `AbstractHorse` that is always tame): carries two riders, the first
//! steering once it wears a saddle; sits down and stands up now and then (refusing to move
//! meanwhile), dashes forward on the rider's jump with a 55-tick cooldown, eats cactus.
//!
//! Approximation: vanilla drives it with a `Brain` (`CamelAi`); here the same behaviours are
//! goals in the brain's priority order (panic, love, temptation, following an adult, looking
//! about, strolling or sitting down).

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event, PlayerView};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{Goal, JUMP, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, GroupData, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Camel;

pub static KIND: Camel = Camel;

/// `createBaseHorseAttributes` with the camel's own.
static INFO: Info = Info {
    head: (30, 40, 10),
    ..Info::animal(
        "minecraft:camel",
        &[(MaxHealth, 32.0), (MovementSpeed, 0.09000000357627869), (JumpStrength, 0.41999998688697815), (StepHeight, 1.5), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5)],
    )
};

const SITDOWN_TICKS: i64 = 40;
const STANDUP_TICKS: i64 = 52;

#[derive(Clone, Debug, Default)]
pub struct State {
    pub saddle: ItemStack,
    /// `LAST_POSE_CHANGE_TICK` (negative while sitting).
    pub last_pose_change: i64,
    pub dashing: bool,
    dash_cooldown: i32,
    /// The game time seen last (pose times need it outside the level).
    now: i64,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("camel state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("camel state")
}

pub fn sitting(m: &MobData) -> bool {
    st(m).last_pose_change < 0
}

fn pose_time(m: &MobData) -> i64 {
    let s = st(m);
    s.now - s.last_pose_change.abs()
}

fn in_pose_transition(m: &MobData) -> bool {
    pose_time(m) < if sitting(m) { SITDOWN_TICKS } else { STANDUP_TICKS }
}

/// `refuseToMove`.
pub fn refuse_to_move(m: &MobData) -> bool {
    sitting(m) || in_pose_transition(m)
}

fn sit_down(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if sitting(m) {
        return;
    }
    mob::make_sound(e, m, level, "minecraft:entity.camel.sit");
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    st_mut(m).last_pose_change = -level.game_time();
    mob::refresh_dimensions(e, m);
}

fn stand_up(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !sitting(m) {
        return;
    }
    mob::make_sound(e, m, level, "minecraft:entity.camel.stand");
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    st_mut(m).last_pose_change = level.game_time();
    mob::refresh_dimensions_in(e, m, level);
}

/// `standUpInstantly`.
fn stand_up_instantly(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    let was = sitting(m);
    st_mut(m).last_pose_change = (level.game_time() - STANDUP_TICKS - 1).max(0);
    if was {
        mob::refresh_dimensions_in(e, m, level);
    }
}

impl Kind for Camel {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        m.nav.can_walk_over_fences = true;
        Some(Box::new(State::default()))
    }

    /// The brain's activities as goals.
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Panic { speed: 4.0, pos: Vec3::ZERO });
        g.add(2, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(3, Goal::Tempt { speed: 2.5, calm_down: 0, player: None });
        g.add(4, Goal::FollowParent { speed: 2.5, parent: None, recalc: 0 });
        g.add(5, Goal::Custom(Box::new(RandomSittingGoal)));
        g.add(6, Goal::RandomStroll { speed: 2.0, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:camel_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:camel_food")
    }

    fn pre_tick(&self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        st_mut(m).now = level.game_time();
    }

    /// `CamelPanic.start`: a panicking camel stands up at once.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if sitting(m) && m.goals.is_running(|g| matches!(g, Goal::Panic { .. })) {
            stand_up_instantly(e, m, level);
        }
    }

    /// `CamelMoveControl`: no steps while sitting or changing pose.
    fn tick_move(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if refuse_to_move(m) {
            mob::control::set_speed(m, 0.0);
            return true;
        }
        false
    }

    /// `Camel.travel`: standing still on the ground while it refuses to move.
    fn travel(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _input: Vec3) -> bool {
        if refuse_to_move(m) && e.on_ground {
            e.delta = e.delta.multiply(0.0, 1.0, 0.0);
            m.xxa = 0.0;
            m.zza = 0.0;
        }
        false
    }

    /// `Camel.tick` after `Mob.tick`: the dash ends on landing, its cooldown runs out with a
    /// sound; sitting in water stands it up.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st(m);
        if s.dashing && s.dash_cooldown < 50 && (e.on_ground || e.is_in_water() || e.is_in_lava() || e.vehicle.is_some()) {
            st_mut(m).dashing = false;
        }
        if st(m).dash_cooldown > 0 {
            st_mut(m).dash_cooldown -= 1;
            if st(m).dash_cooldown == 0 {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.camel.dash_ready", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
        }
        if refuse_to_move(m) {
            m.y_head_rot = m.y_body_rot;
        }
        if sitting(m) && e.is_in_water() {
            stand_up_instantly(e, m, level);
        }
    }

    fn tick_body(&self, _e: &mut Entity, m: &mut MobData) -> bool {
        refuse_to_move(m)
    }

    /// `actuallyHurt`: a hurt camel stands up at once.
    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &mob::DamageSource, _amount: f32, hurt: bool) {
        if hurt && sitting(m) {
            stand_up_instantly(e, m, level);
        }
    }

    fn steerable_by(&self, m: &MobData, _rider: &PlayerView) -> bool {
        !st(m).saddle.is_empty()
    }

    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, rider: &PlayerView) {
        if refuse_to_move(m) {
            return;
        }
        e.y_rot = rider.yaw % 360.0;
        e.x_rot = (rider.pitch * 0.5) % 360.0;
        e.y_rot_o = e.y_rot;
        m.y_body_rot = e.y_rot;
        m.y_head_rot = e.y_rot;
    }

    /// `getPassengerAttachmentPoint`: the driver at the front, the second rider behind.
    fn passenger_offset_at(&self, e: &Entity, m: &MobData, index: usize) -> Option<Vec3> {
        let scale = if m.baby() { 0.6f32 } else { 1.0 };
        let base_h = e.height as f64 - if m.baby() { 0.09375 } else { 0.375 };
        let drop = scale * 1.43 - (scale * 1.43 - scale * 0.2);
        let h = if sitting(m) && !in_pose_transition(m) { base_h + drop as f64 } else { base_h };
        let mut offset = 0.5f32;
        if e.passengers.len() > 1 && index > 0 {
            offset = -0.7;
        }
        Some(crate::ride::y_rot(Vec3::new(0.0, h, (offset * scale) as f64), -e.y_rot * 0.017453292))
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if who.sneaking && !m.baby() {
            // The inventory screen is not modelled.
            return Some(Outcome::success(HeldChange::None));
        }
        let name = if stack.is_empty() { "minecraft:air" } else { mob::item_name(stack) };
        if name == "minecraft:saddle" && st(m).saddle.is_empty() && !m.baby() && mob::is_alive(e, m) {
            let mut one = stack.clone();
            one.set_count(1);
            st_mut(m).saddle = one;
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.camel.saddle", source: "neutral", volume: 0.5, pitch: 1.0 });
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if !stack.is_empty() && self.is_food(stack.item()) {
            // `handleEating`: heal 2, love when tame and grown, grow up faster.
            let mut ate = false;
            if m.health < m.max_health() {
                let h = m.health + 2.0;
                m.set_health(h);
                ate = true;
            }
            if m.age == 0 && m.in_love <= 0 {
                mob::breed::set_in_love(e, m, level, Some(who.id));
                ate = true;
            }
            if m.baby() && !m.age_locked {
                mob::random_point(e, 1.0);
                mob::age_up(e, m, 10, false);
                ate = true;
            }
            if !ate {
                return Some(Outcome::PASS);
            }
            if !e.silent {
                let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.camel.eat", source: "neutral", volume: 1.0, pitch });
            }
            level.emit(Event::GameEvent { event: "minecraft:eat", pos: e.position(), entity: Some(e.id) });
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if e.passengers.len() < 2 && !m.baby() {
            // `doPlayerRide`.
            let mut out = Outcome::success(HeldChange::None);
            out.ride = true;
            return Some(out);
        }
        Some(Outcome::PASS)
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = st(m);
        if s.saddle.is_empty() { Vec::new() } else { vec![(7, s.saddle.clone())] }
    }

    /// `canMate`: both grown, fed and healthy (`canParent`).
    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        let parent = |x: &MobData| !x.baby() && x.health >= x.max_health() && x.in_love > 0 && !x.is_vehicle;
        parent(m) && parent(partner)
    }

    /// `AbstractHorse.finalizeSpawn`: babies at 0.2 after the first of a group.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        ext::ageable_finalize(e, m, r, group, 0.2);
        ext::mob_finalize(m, r);
    }

    /// `checkCamelSpawnRules`.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(super::wolf::block_in_tag(view.block(pos.below()), "minecraft:camels_spawnable_on") && view.raw_brightness(pos, 0) > 8)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        match (sitting(m), m.baby()) {
            (true, true) => (0.95, 0.425, 0.41),
            (true, false) => (base.0, base.1 - 1.43, 0.845),
            (false, true) => (0.95, 1.4, 1.38),
            (false, false) => base,
        }
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let saddle = match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let pose = match r.get("LastPoseTick") {
            Some(Tag::Long(t)) => *t,
            _ => 0,
        };
        let s = st_mut(m);
        if let Some(sd) = saddle {
            s.saddle = sd;
        }
        s.last_pose_change = pose;
        mob::refresh_dimensions(e, m);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        if !s.saddle.is_empty() {
            let entry = ("saddle".to_owned(), s.saddle.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        o.put("EatingHaystack", Tag::Byte(0));
        o.put("Bred", Tag::Byte(0));
        o.put("Temper", Tag::Int(0));
        o.put("Tame", Tag::Byte(1));
        o.put("LastPoseTick", Tag::Long(s.last_pose_change));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let s = st(m);
        // `AbstractHorse` flags: tame.
        d.set(data::abstract_horse::ID_FLAGS, &DataValue::Byte(2));
        d.set(data::camel::DASH, &DataValue::Boolean(s.dashing));
        d.set(data::camel::LAST_POSE_CHANGE_TICK, &DataValue::Long(s.last_pose_change));
        if sitting(m) {
            d.set(data::entity::POSE, &DataValue::Pose(kiln_data::entities::pose::SITTING));
        }
    }
}

/// `handleStartJump` on a saddled camel (the rider's jump key): the dash with its sound, the
/// 55-tick cooldown (the dash's motion is the rider's client's).
pub fn start_jump(m: &mut MobData) -> Option<&'static str> {
    let s = ext::state::<State>(m)?;
    if s.saddle.is_empty() || s.dash_cooldown > 0 || refuse_to_move(m) {
        return None;
    }
    let s = st_mut(m);
    s.dashing = true;
    s.dash_cooldown = 55;
    Some("minecraft:entity.camel.dash")
}

/// `CamelAi.RandomSitting`: after 20 seconds in a pose, on the ground, out of the water and
/// without a rider, it sits down (or stands back up).
#[derive(Clone, Debug)]
struct RandomSittingGoal;

impl CustomGoal for RandomSittingGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RandomSitting"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        // One pick of four in the brain's `RunOne` of idle moves.
        if e.random.next_int_bounded(mob::mth::reduced_tick_delay(120) * 4) != 0 {
            return false;
        }
        !e.is_in_water() && pose_time(m) >= 400 && e.on_ground && e.passengers.is_empty()
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if sitting(m) {
            stand_up(e, m, level);
        } else if !m.goals.is_running(|g| matches!(g, Goal::Panic { .. })) {
            sit_down(e, m, level);
        }
    }
}

