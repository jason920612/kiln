//! Armadillo: rolls up into its shell when something scary is near (undead, whoever hurt it,
//! sprinting or riding players within 7 blocks), peeks out now and then and unrolls once the
//! danger has been gone 80 ticks; hurt while rolled up it takes (damage - 1) / 2. It sheds a
//! scute every 5 to 10 minutes and gives one to a brush.
//!
//! Approximation: vanilla drives it with a `Brain` (`ArmadilloAi`); here the same behaviours are
//! goals in the brain's priority order (panic, the ball-up, love, temptation, following an
//! adult, looking about, strolling). The scare sensor runs every 5 ticks from the mob's tick.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, JUMP, LOOK, MOVE};
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
    /// `DANGER_DETECTED_RECENTLY`'s expiry (game time).
    danger_until: Option<i64>,
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
fn can_stay_rolled_up(e: &Entity, m: &MobData) -> bool {
    let panicking = m.goals.is_running(|g| matches!(g, Goal::Panic { .. }));
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
fn scared_by(e: &Entity, m: &MobData, level: &dyn EntityLevel, id: i32) -> bool {
    let area = e.bounding_box().inflate(7.0, 2.0, 7.0);
    if let Some(p) = level.player(id) {
        let t = goals::living_player(&p);
        return t.bb.intersects(&area) && !p.spectator && (m.last_hurt_by_mob == Some(id) || p.vehicle.is_some());
    }
    let Some(o) = level.entity(id) else { return false };
    if !o.bounding_box().intersects(&area) {
        return false;
    }
    mob::entity_type_tag(o.type_name, "minecraft:undead") || m.last_hurt_by_mob == Some(id)
}

impl Kind for Armadillo {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        Some(Box::new(State { state: IDLE, in_state_ticks: 0, scute_time: pick_scute_time(random), danger_until: None }))
    }

    /// The brain's activities as goals.
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Panic { speed: 2.0, pos: Vec3::ZERO });
        g.add(2, Goal::Custom(Box::new(BallUpGoal { next_peek: 0, danger_was_around: false })));
        g.add(3, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(4, Goal::Tempt { speed: 1.25, calm_down: 0, player: None });
        g.add(5, Goal::FollowParent { speed: 1.25, parent: None, recalc: 0 });
        g.add(6, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:armadillo_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:armadillo_food")
    }


    /// The scare sensor (every 5 ticks), the danger running out (`ARMADILLO_ROLLING_OUT`) and
    /// the scute (`customServerAiStep`).
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let now = level.game_time();
        if e.tick_count % 5 == 0 {
            if !can_stay_rolled_up(e, m) {
                st_mut(m).danger_until = None;
            } else {
                let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
                let scary = level.entities_in(&area, crate::level::EntityFilter::Living, e.id).into_iter().any(|id| scared_by(e, m, level, id));
                if scary {
                    st_mut(m).danger_until = Some(now + 80);
                }
            }
        }
        if st(m).danger_until.is_some_and(|t| t <= now) {
            st_mut(m).danger_until = None;
        }
        if st(m).danger_until.is_none() && is_scared(m) {
            roll_out(e, m, level);
        }
        let s = st_mut(m);
        s.scute_time -= 1;
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

    /// `hurtServer`: the shell halves the damage (less one); `actuallyHurt`: an attacker scares
    /// it, fire and the like make it unroll.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let amount = if is_scared(m) { (amount - 1.0) / 2.0 } else { amount };
        let r = mob::hurt_base(e, m, level, *source, amount);
        if r && !m.no_ai && !m.is_dead_or_dying() {
            if source.attacker.is_some_and(|a| goals::living(level, a).is_some()) {
                st_mut(m).danger_until = Some(level.game_time() + 80);
                if can_stay_rolled_up(e, m) {
                    roll_up(e, m, level);
                }
            } else if source.kind.is_tag("minecraft:panic_environmental_causes") {
                roll_out(e, m, level);
            }
        }
        Some(r)
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

/// `ArmadilloAi.ArmadilloBallUp`: while danger was detected recently, on the ground and dry:
/// rolled up, peeking out, unrolling when the danger fades.
#[derive(Clone, Debug)]
struct BallUpGoal {
    next_peek: i32,
    danger_was_around: bool,
}

impl BallUpGoal {
    fn pick_peek(e: &mut Entity) -> i32 {
        STATES[SCARED as usize].2 as i32 + mob::mth::next_int_between(&mut e.random, 100, 400)
    }
}

impl CustomGoal for BallUpGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ArmadilloBallUp"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        st(m).danger_until.is_some_and(|t| t > level.game_time()) && e.on_ground && !e.is_in_water() && !e.is_in_lava()
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        STATES[st(m).state as usize].1
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        roll_up(e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !can_stay_rolled_up(e, m) {
            roll_out(e, m, level);
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.next_peek > 0 {
            self.next_peek -= 1;
        }
        let s = st(m);
        if s.state == ROLLING && s.in_state_ticks > STATES[ROLLING as usize].2 {
            switch_to(m, SCARED);
            if e.on_ground && !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.armadillo.land", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
            return;
        }
        let left = st(m).danger_until.map_or(0, |t| t - level.game_time());
        let danger = left > 75;
        if danger != self.danger_was_around {
            self.next_peek = Self::pick_peek(e);
        }
        self.danger_was_around = danger;
        match st(m).state {
            SCARED => {
                if self.next_peek == 0 && e.on_ground && danger {
                    level.emit(Event::EntityEvent { entity: e.id, event: 64 });
                    self.next_peek = Self::pick_peek(e);
                }
                if left < STATES[UNROLLING as usize].2 {
                    if !e.silent {
                        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.armadillo.unroll_start", source: "neutral", volume: 1.0, pitch: 1.0 });
                    }
                    switch_to(m, UNROLLING);
                }
            }
            UNROLLING if left > STATES[UNROLLING as usize].2 => switch_to(m, SCARED),
            _ => {}
        }
    }
}
