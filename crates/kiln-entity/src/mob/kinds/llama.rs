//! Llamas and trader llamas (`Llama`, `TraderLlama`): chested equines (see [`super::horse`], which
//! holds their state: strength, coat, caravan links, the trader's despawn delay) that spit at what
//! hurts them, fight wolves, and follow each other in caravans when led.
//!
//! Leads do not exist yet: no llama is ever leashed, so [`LlamaFollowCaravanGoal`] only runs for
//! llamas that were joined to a caravan by hand ([`join_caravan`]) and a trader llama is never led
//! by a wandering trader (`TraderLlamaDefendWanderingTraderGoal` never starts).

use super::common_a::{Avoid, NearestTargetGoal};
use super::horse::{self, st, st_mut};
use super::snow_golem::RangedAttackGoal;
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::ext::CustomGoal;
use crate::mob::goals::{self, Goal, Living, MOVE, TARGET};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{MobData, MobKind, path};
use kiln_javamath::random::RandomSource;

pub use super::horse::{LLAMA, TRADER_LLAMA};

/// `instanceof Llama` for a type (trader llamas are llamas).
pub fn is_llama(kind: MobKind) -> bool {
    matches!(kind, MobKind::Llama | MobKind::TraderLlama)
}

/// `Llama.getStrength`.
pub fn strength(m: &MobData) -> i32 {
    horse::state(m).map_or(0, |s| s.strength)
}

/// `Llama.setStrength`: 1 to 5.
pub fn set_strength(m: &mut MobData, strength: i32) {
    st_mut(m).strength = strength.clamp(1, 5);
}

/// `Llama.setRandomStrength`: one to three, now and then one to five.
pub fn set_random_strength(m: &mut MobData, r: &mut dyn RandomSource) {
    let bound = if r.next_float() < 0.04 { 5 } else { 3 };
    set_strength(m, 1 + r.next_int_bounded(bound));
}

/// `Llama.Variant.byId` (clamped).
pub fn variant_by_id(id: i32) -> i32 {
    id.clamp(0, 3)
}

/// The leash holder of `m` (`Leashable.getLeashHolder`): nobody, leads are not simulated.
pub fn leash_holder(_m: &MobData) -> Option<i32> {
    None
}

/// `isLeashed`.
pub fn is_leashed(m: &MobData) -> bool {
    leash_holder(m).is_some()
}

/// `Llama.inCaravan`.
pub fn in_caravan(m: &MobData) -> bool {
    horse::state(m).is_some_and(|s| s.caravan_head.is_some())
}

/// `Llama.hasCaravanTail`.
pub fn has_caravan_tail(m: &MobData) -> bool {
    horse::state(m).is_some_and(|s| s.caravan_tail.is_some())
}

/// `Llama.leaveCaravan`: the head forgets this llama.
pub fn leave_caravan(id: i32, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(head) = st_mut(m).caravan_head.take()
        && let Some(hm) = level.entity_mut(head).and_then(crate::mob::data_mut)
        && let Some(s) = ext_state_mut(hm)
        && s.caravan_tail == Some(id)
    {
        s.caravan_tail = None;
    }
}

/// `Llama.joinCaravan(head)`: this llama follows `head`, which now has it as its tail.
pub fn join_caravan(id: i32, m: &mut MobData, head: i32, level: &mut dyn EntityLevel) {
    st_mut(m).caravan_head = Some(head);
    if let Some(hm) = level.entity_mut(head).and_then(crate::mob::data_mut)
        && let Some(s) = ext_state_mut(hm)
    {
        s.caravan_tail = Some(id);
    }
}

fn ext_state_mut(m: &mut MobData) -> Option<&mut horse::State> {
    crate::mob::ext::state_mut::<horse::State>(m)
}

/// `Llama.registerGoals` and `TraderLlama.registerGoals`.
pub fn register_goals(m: &mut MobData, trader: bool) {
    let g = &mut m.goals;
    g.add(0, Goal::Float);
    g.add(1, Goal::Custom(Box::new(horse::run_around_like_crazy(1.2))));
    g.add(2, Goal::Custom(Box::new(LlamaFollowCaravanGoal { speed: 2.0999999046325684, dist_check_counter: 0 })));
    g.add(3, Goal::Custom(Box::new(RangedAttackGoal::with_attack("RangedAttackGoal", 1.25, 40, 40, 20.0, spit))));
    g.add(3, Goal::Panic { speed: 1.2, pos: Vec3::ZERO });
    g.add(4, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
    g.add(5, Goal::Tempt { speed: 1.25, calm_down: 0, player: None });
    g.add(6, Goal::FollowParent { speed: 1.0, parent: None, recalc: 0 });
    g.add(7, Goal::RandomStroll { speed: 0.7, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
    g.add(8, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
    g.add(9, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    m.targets.add(1, Goal::Custom(Box::new(LlamaHurtByTargetGoal { inner: super::common_a::hurt_by(false) })));
    // `LlamaAttackWolfGoal`: a wild wolf in a quarter of the follow range.
    m.targets.add(
        2,
        NearestTargetGoal::new("LlamaAttackWolfGoal", Avoid::Types(&["minecraft:wolf"]), 16, false)
            .scale(0.25)
            .selector(|_, _, level, t| !level.entity(t.id).and_then(crate::mob::data).is_some_and(super::tame::is_tame))
            .boxed(),
    );
    if trader {
        m.goals.add(1, Goal::Panic { speed: 2.0, pos: Vec3::ZERO });
        m.targets.add(1, Goal::Custom(Box::new(TraderLlamaDefendWanderingTraderGoal { timestamp: 0 })));
        // Zombies (but zombified piglins) and illagers.
        m.targets.add(
            2,
            NearestTargetGoal::new("NearestAttackableTargetGoal", Avoid::Types(&["minecraft:zombie", "minecraft:husk", "minecraft:drowned", "minecraft:zombie_villager"]), 10, true).boxed(),
        );
        m.targets.add(
            2,
            NearestTargetGoal::new("NearestAttackableTargetGoal", Avoid::Types(&["minecraft:pillager", "minecraft:vindicator", "minecraft:evoker", "minecraft:illusioner"]), 10, true).boxed(),
        );
    }
}

/// `Llama.performRangedAttack` (`spit`): a `LlamaSpit` from the front of the body toward a third
/// of the way up the target, spread by 10, and the llama's spit sound.
pub fn spit(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut spit = crate::ext_entity::llama_spit::new(id, e, m.y_body_rot, seed);
    let height = level.entity(t.id).map_or_else(|| if t.sneaking { 1.5f32 as f64 } else { 1.8f32 as f64 }, |o| o.height as f64);
    let dx = t.pos.x - e.x();
    let dy = t.pos.y + height * 0.3333333333333333 - spit.y();
    let dz = t.pos.z - e.z();
    let yo = (dx * dx + dz * dz).sqrt() * 0.20000000298023224;
    crate::mob::species::shoot(&mut spit, dx, dy + yo, dz, 1.5, 10.0);
    spit.set_old_pos_and_rot();
    level.add_entity(spit);
    if !e.silent {
        let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.llama.spit", source: "neutral", volume: 1.0, pitch });
    }
    st_mut(m).did_spit = true;
}

/// `Llama.LlamaHurtByTargetGoal`: a `HurtByTargetGoal` that lets go after the llama spat.
#[derive(Clone, Debug)]
struct LlamaHurtByTargetGoal {
    inner: Goal,
}

impl CustomGoal for LlamaHurtByTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LlamaHurtByTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut crate::mob::MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_use(&mut self.inner, e, m, level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut crate::mob::MobData, level: &mut dyn EntityLevel) -> bool {
        if st(m).did_spit {
            st_mut(m).did_spit = false;
            return false;
        }
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut crate::mob::MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut crate::mob::MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut crate::mob::MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
    }
}

/// `LlamaFollowCaravanGoal`: a llama in sight of a caravan led by a lead joins its tail and
/// follows the llama ahead of it.
#[derive(Clone, Debug)]
struct LlamaFollowCaravanGoal {
    speed: f64,
    dist_check_counter: i32,
}

/// `firstIsLeashed`: the head of the caravan, at most eight links up, is led.
fn first_is_leashed(level: &dyn EntityLevel, llama: i32, depth: i32) -> bool {
    if depth > 8 {
        return false;
    }
    let Some(m) = level.entity(llama).and_then(crate::mob::data) else { return false };
    if !in_caravan(m) {
        return false;
    }
    let head = st(m).caravan_head.unwrap();
    let Some(hm) = level.entity(head).and_then(crate::mob::data) else { return false };
    if is_leashed(hm) {
        return true;
    }
    first_is_leashed(level, head, depth + 1)
}

impl CustomGoal for LlamaFollowCaravanGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LlamaFollowCaravanGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if is_leashed(m) || in_caravan(m) {
            return false;
        }
        let area = e.bounding_box().inflate(9.0, 4.0, 9.0);
        let llamas: Vec<i32> = level
            .entities_in(&area, EntityFilter::Living, e.id)
            .into_iter()
            .filter(|&id| level.entity(id).is_some_and(|o| o.type_name == "minecraft:llama" || o.type_name == "minecraft:trader_llama"))
            .collect();
        let mut closest: Option<i32> = None;
        let mut best = f64::MAX;
        for &id in &llamas {
            let Some(o) = level.entity(id) else { continue };
            let Some(om) = crate::mob::data(o) else { continue };
            if !in_caravan(om) || has_caravan_tail(om) {
                continue;
            }
            let d = e.position().distance_to_sqr(o.position());
            if d > best {
                continue;
            }
            best = d;
            closest = Some(id);
        }
        if closest.is_none() {
            for &id in &llamas {
                let Some(o) = level.entity(id) else { continue };
                let Some(om) = crate::mob::data(o) else { continue };
                if !is_leashed(om) || has_caravan_tail(om) {
                    continue;
                }
                let d = e.position().distance_to_sqr(o.position());
                if d > best {
                    continue;
                }
                best = d;
                closest = Some(id);
            }
        }
        let Some(head) = closest else { return false };
        if best < 4.0 {
            return false;
        }
        let head_leashed = level.entity(head).and_then(crate::mob::data).is_some_and(is_leashed);
        if !head_leashed && !first_is_leashed(level, head, 1) {
            return false;
        }
        join_caravan(e.id, m, head, level);
        true
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(head) = st(m).caravan_head else { return false };
        let Some(ho) = level.entity(head) else { return false };
        if !in_caravan(m) || !ho.is_alive() || !first_is_leashed(level, e.id, 0) {
            return false;
        }
        let d = e.position().distance_to_sqr(ho.position());
        if d > 676.0 {
            if self.speed <= 3.0 {
                self.speed *= 1.2;
                self.dist_check_counter = reduced_tick_delay(40);
                return true;
            }
            if self.dist_check_counter == 0 {
                return false;
            }
        }
        if self.dist_check_counter > 0 {
            self.dist_check_counter -= 1;
        }
        true
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        leave_caravan(e.id, m, level);
        self.speed = 2.1;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(head) = st(m).caravan_head else { return };
        // (A llama led by a fence knot stays where it is: knots are not simulated.)
        let Some(ho) = level.entity(head) else { return };
        let (hx, hy, hz) = (ho.x(), ho.y(), ho.z());
        let fx = (e.x() - hx) as f32;
        let fy = (e.y() - hy) as f32;
        let fz = (e.z() - hz) as f32;
        let distance = ((fx * fx + fy * fy + fz * fz) as f32).sqrt() as f64;
        let v = Vec3::new(hx - e.x(), hy - e.y(), hz - e.z()).normalize().scale((distance - 2.0).max(0.0));
        path::move_to(e, m, level, e.x() + v.x, e.y() + v.y, e.z() + v.z, self.speed);
    }
}

/// `TraderLlama.TraderLlamaDefendWanderingTraderGoal`: fights whoever hurt the wandering trader
/// that leads the llama (which never happens: leads and wandering traders are not simulated).
#[derive(Clone, Debug)]
struct TraderLlamaDefendWanderingTraderGoal {
    timestamp: i32,
}

impl CustomGoal for TraderLlamaDefendWanderingTraderGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TraderLlamaDefendWanderingTraderGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(holder) = leash_holder(m) else { return false };
        let Some(h) = level.entity(holder).filter(|h| h.type_name == "minecraft:wandering_trader") else { return false };
        let Some(hm) = crate::mob::data(h) else { return false };
        let (by, stamp) = (hm.last_hurt_by_mob, hm.last_hurt_by_mob_timestamp);
        let Some(by) = by.and_then(|id| goals::living(level, id)) else { return false };
        stamp != self.timestamp && goals::can_attack(m, level, &by)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(holder) = leash_holder(m) else { return };
        let Some(hm) = level.entity(holder).and_then(crate::mob::data) else { return };
        let (by, stamp) = (hm.last_hurt_by_mob, hm.last_hurt_by_mob_timestamp);
        crate::mob::set_target(e, m, by);
        self.timestamp = stamp;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, None);
    }
}

/// `Wolf.WolfAvoidEntityGoal` for llamas: a wild wolf runs from a llama (24 blocks) when the
/// llama's strength is at least a roll of 0 to 4, and drops its target while it does.
#[derive(Clone, Debug)]
pub struct WolfAvoidEntityGoal {
    inner: super::common_a::AvoidEntityGoal,
}

impl WolfAvoidEntityGoal {
    pub fn new() -> WolfAvoidEntityGoal {
        WolfAvoidEntityGoal { inner: super::common_a::AvoidEntityGoal::new("WolfAvoidEntityGoal", Avoid::Types(&["minecraft:llama", "minecraft:trader_llama"]), 24.0, 1.5, 1.5) }
    }
}

impl Default for WolfAvoidEntityGoal {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomGoal for WolfAvoidEntityGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "WolfAvoidEntityGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !self.inner.can_use(e, m, level) {
            return false;
        }
        let Some(llama) = self.inner.to_avoid.and_then(|id| level.entity(id)).and_then(crate::mob::data) else { return false };
        // `!wolf.isTame() && avoidLlama(llama)`.
        if super::tame::is_tame(m) {
            return false;
        }
        strength(llama) >= e.random.next_int_bounded(5)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.inner.can_continue(e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, None);
        self.inner.start(e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.inner.stop(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, None);
        self.inner.tick(e, m, level);
    }
}
