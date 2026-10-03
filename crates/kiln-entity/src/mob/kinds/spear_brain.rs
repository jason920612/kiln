//! The piglin's spear behaviours (`SpearApproach`, `SpearAttack`, `SpearRetreat`): the brain's
//! version of [`super::spear_use::SpearUseGoal`], in the fight activity before the melee attack.
//! The phase is kept in the `spear_status` memory (`SpearAttack.SpearStatus`, as an int).

use super::spear_use::{charge_speed_modifier, root_vehicle};
use crate::behavior_boilerplate;
use crate::mob::brain::{Behavior, Control, Cx, Mem, Status, Timed, Tracker, Val, util};
use crate::mob::goals::Living;
use crate::mob::{MAINHAND, path, random_pos};
use crate::spear;
use kiln_item::keys;
use Status::{ValueAbsent, ValuePresent};

const APPROACH: i32 = 0;
const CHARGING: i32 = 1;
const RETREAT: i32 = 2;

fn status(cx: &Cx) -> i32 {
    cx.b.mem.int(Mem::SpearStatus).unwrap_or(APPROACH)
}

fn target(cx: &Cx) -> Option<Living> {
    cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id))
}

fn able_to_attack(cx: &Cx) -> bool {
    cx.m.equipment[MAINHAND].get(keys::KINETIC_WEAPON).is_some() && target(cx).is_some()
}

/// `Mob.chargeSpeedModifier` of the root vehicle.
fn charge(cx: &Cx) -> f64 {
    root_vehicle(cx.e, cx.m, &*cx.level).and_then(|(_, k)| k).map_or(1.0, charge_speed_modifier) as f64
}

fn look_at(cx: &mut Cx, t: &Living) {
    cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(t.id, true)));
}

fn block_of(t: &Living) -> crate::math::BlockPos {
    crate::math::BlockPos::containing(t.pos.x, t.pos.y, t.pos.z)
}

/// `SpearApproach(speedModifierWhenRepositioning, approachDistance)`.
#[derive(Clone, Debug)]
pub struct SpearApproach {
    speed: f64,
    approach_sq: f32,
}

impl SpearApproach {
    pub fn new(speed: f64, approach: f32) -> Box<dyn Control> {
        Timed::new(SpearApproach { speed, approach_sq: approach * approach })
    }

    fn far_enough(&self, cx: &Cx) -> bool {
        let Some(t) = target(cx) else { return false };
        cx.e.position().distance_to_sqr(t.pos) > self.approach_sq as f64
    }
}

impl Behavior for SpearApproach {
    fn name(&self) -> &'static str {
        "SpearApproach"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::SpearStatus, ValueAbsent)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        able_to_attack(cx) && cx.m.using_item.is_none()
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.m.set_aggressive(true);
        cx.b.mem.set(Mem::SpearStatus, Val::Int(APPROACH));
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        able_to_attack(cx) && self.far_enough(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = target(cx) else { return };
        let speed = charge(cx) * self.speed;
        look_at(cx, &t);
        path::move_to_entity(cx.e, cx.m, &*cx.level, block_of(&t), speed);
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.m.nav_mut().stop();
        cx.b.mem.set(Mem::SpearStatus, Val::Int(CHARGING));
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    behavior_boilerplate!();
}

/// `SpearAttack(speedModifierWhenCharging, speedModifierWhenRepositioning, targetInRangeRadius)`.
#[derive(Clone, Debug)]
pub struct SpearAttack {
    charging: f64,
    repositioning: f64,
    in_range_sq: f32,
}

impl SpearAttack {
    pub fn new(charging: f64, repositioning: f64, in_range: f32) -> Box<dyn Control> {
        Timed::new(SpearAttack { charging, repositioning, in_range_sq: in_range * in_range })
    }
}

impl Behavior for SpearAttack {
    fn name(&self) -> &'static str {
        "SpearAttack"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::SpearStatus, ValuePresent)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        status(cx) == CHARGING && able_to_attack(cx) && cx.m.using_item.is_none()
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.m.set_aggressive(true);
        let duration = cx.m.equipment[MAINHAND].get(keys::KINETIC_WEAPON).map_or(0, spear::damage_use_duration);
        cx.b.mem.set(Mem::SpearEngageTime, Val::Int(duration));
        cx.b.mem.erase(Mem::SpearChargePosition);
        cx.m.start_using_item();
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.int(Mem::SpearEngageTime).unwrap_or(0) > 0 && able_to_attack(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = target(cx) else { return };
        let d = cx.e.position().distance_to_sqr(t.pos);
        let charge = charge(cx);
        let extra: i32 = if cx.e.vehicle.is_some() { 2 } else { 0 };
        look_at(cx, &t);
        let engage = cx.b.mem.int(Mem::SpearEngageTime).unwrap_or(0) - 1;
        cx.b.mem.set(Mem::SpearEngageTime, Val::Int(engage));
        if let Some(p) = cx.b.mem.vec3(Mem::SpearChargePosition) {
            path::move_to(cx.e, cx.m, &*cx.level, p.x, p.y, p.z, charge * self.repositioning);
            if cx.m.nav_ref().is_done() {
                cx.b.mem.erase(Mem::SpearChargePosition);
            }
        } else {
            path::move_to_entity(cx.e, cx.m, &*cx.level, block_of(&t), charge * self.charging);
            if d < self.in_range_sq as f64 || cx.m.nav_ref().is_done() {
                let dist = d.sqrt();
                let away = random_pos::land_pos_away_between(cx.e, cx.m, &*cx.level, (6 + extra) as f64 - dist, (7 + extra) as f64 - dist, 7, t.pos);
                cx.b.mem.set_opt(Mem::SpearChargePosition, away.map(Val::Vec3));
            }
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.m.nav_mut().stop();
        cx.m.stop_using_item();
        cx.b.mem.erase(Mem::SpearChargePosition);
        cx.b.mem.erase(Mem::SpearEngageTime);
        cx.b.mem.set(Mem::SpearStatus, Val::Int(RETREAT));
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    behavior_boilerplate!();
}

/// `SpearRetreat(speedModifierWhenRepositioning)`.
#[derive(Clone, Debug)]
pub struct SpearRetreat {
    speed: f64,
}

impl SpearRetreat {
    pub fn new(speed: f64) -> Box<dyn Control> {
        Timed::new(SpearRetreat { speed })
    }
}

impl Behavior for SpearRetreat {
    fn name(&self) -> &'static str {
        "SpearRetreat"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::SpearStatus, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (100, 100)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if !able_to_attack(cx) || cx.m.using_item.is_some() {
            return false;
        }
        if status(cx) != RETREAT {
            return false;
        }
        let Some(t) = target(cx) else { return false };
        let d = cx.e.position().distance_to_sqr(t.pos);
        let extra: i32 = if cx.e.vehicle.is_some() { 2 } else { 0 };
        let dist = d.sqrt();
        let away = random_pos::land_pos_away_between(cx.e, cx.m, &*cx.level, 0.0f64.max((9 + extra) as f64 - dist), 1.0f64.max((11 + extra) as f64 - dist), 7, t.pos);
        let Some(away) = away else { return false };
        cx.b.mem.set(Mem::SpearFleeingPosition, Val::Vec3(away));
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.m.set_aggressive(true);
        cx.b.mem.set(Mem::SpearFleeingTime, Val::Int(0));
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.int(Mem::SpearFleeingTime).unwrap_or(100) < 100 && cx.b.mem.has(Mem::SpearFleeingPosition) && !cx.m.nav_ref().is_done() && able_to_attack(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = target(cx) else { return };
        let speed = charge(cx) * self.speed;
        look_at(cx, &t);
        let time = cx.b.mem.int(Mem::SpearFleeingTime).unwrap_or(0) + 1;
        cx.b.mem.set(Mem::SpearFleeingTime, Val::Int(time));
        if let Some(p) = cx.b.mem.vec3(Mem::SpearFleeingPosition) {
            path::move_to(cx.e, cx.m, &*cx.level, p.x, p.y, p.z, speed);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.m.nav_mut().stop();
        cx.m.set_aggressive(false);
        cx.m.stop_using_item();
        cx.b.mem.erase(Mem::SpearFleeingTime);
        cx.b.mem.erase(Mem::SpearFleeingPosition);
        cx.b.mem.erase(Mem::SpearStatus);
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    behavior_boilerplate!();
}
