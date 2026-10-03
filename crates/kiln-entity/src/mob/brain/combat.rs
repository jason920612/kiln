//! Combat behaviours several brains share: `MeleeAttack`, `StartAttacking`,
//! `StopAttackingIfTargetInvalid`, `SetWalkTargetFromAttackTargetIfTargetOutOfReach`, `BackUpIfTooClose`.
//! (Declarative one-shots in vanilla: they never show up as running behaviours.)

use super::memory::{Tracker, Val, WalkTarget};
use super::util;
use super::{Control, Cx, Mem, Status, shot};
use crate::mob::goals::{self, Living};
use crate::mob::{self};

use Status::{Registered, ValueAbsent, ValuePresent};

/// A bow or a crossbow in a hand (`ProjectileWeaponItem`).
fn holds_projectile_weapon(cx: &Cx) -> bool {
    [mob::MAINHAND, mob::OFFHAND].iter().any(|&i| {
        let s = &cx.m.equipment[i];
        !s.is_empty() && matches!(mob::item_name(s), "minecraft:bow" | "minecraft:crossbow")
    })
}

/// `MeleeAttack.isHoldingUsableNonMeleeWeapon` (`Mob.canUseNonMeleeWeapon` of what is held): a
/// bow or a crossbow, and for a piglin a spear (a `kinetic_weapon`: it charges instead).
fn holds_usable_non_melee(cx: &Cx) -> bool {
    holds_projectile_weapon(cx)
        || (cx.m.kind == mob::MobKind::Piglin
            && [mob::MAINHAND, mob::OFFHAND].iter().any(|&i| cx.m.equipment[i].get(kiln_item::keys::KINETIC_WEAPON).is_some()))
}

/// `MeleeAttack.create(cooldown)`: hits the attack target in reach and in sight, then cools down.
pub fn melee_attack(cooldown: i32) -> Box<dyn Control> {
    melee_attack_when(|_| true, cooldown)
}

/// `MeleeAttack.create(predicate, cooldown)`.
pub fn melee_attack_when(pred: fn(&Cx) -> bool, cooldown: i32) -> Box<dyn Control> {
    shot(
        "MeleeAttack",
        &[
            (Mem::LookTarget, Registered),
            (Mem::AttackTarget, ValuePresent),
            (Mem::AttackCoolingDown, ValueAbsent),
            (Mem::NearestVisibleLivingEntities, ValuePresent),
        ],
        move |cx| {
            let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
            let Some(t) = util::living(cx, id) else { return false };
            if pred(cx) && !holds_usable_non_melee(cx) && util::within_melee(cx, &t) && util::visible_contains(cx, id) {
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                cx.m.swing = true;
                mob::do_hurt_target(cx.e, cx.m, cx.level, &t);
                cx.b.mem.set_expiring(Mem::AttackCoolingDown, Val::Bool(true), cooldown as i64);
                return true;
            }
            false
        },
    )
}

/// `StartAttacking.create(condition, finder)`: picks an attack target.
pub fn start_attacking(cond: fn(&mut Cx) -> bool, finder: fn(&mut Cx) -> Option<i32>) -> Box<dyn Control> {
    shot("StartAttacking", &[(Mem::AttackTarget, ValueAbsent), (Mem::CantReachWalkTargetSince, Registered)], move |cx| {
        if !cond(cx) {
            return false;
        }
        let Some(id) = finder(cx) else { return false };
        let Some(t) = util::living(cx, id) else { return false };
        if !goals::can_attack(cx.m, &*cx.level, &t) {
            return false;
        }
        cx.b.mem.set(Mem::AttackTarget, Val::Entity(id));
        cx.b.mem.erase(Mem::CantReachWalkTargetSince);
        true
    })
}

/// `StopAttackingIfTargetInvalid.create(stopCondition, onErased, canGrowTired)`.
pub fn stop_attacking_if_target_invalid(
    stop: fn(&mut Cx, &Living) -> bool,
    erased: fn(&mut Cx, i32),
    can_grow_tired: bool,
) -> Box<dyn Control> {
    shot("StopAttackingIfTargetInvalid", &[(Mem::AttackTarget, ValuePresent), (Mem::CantReachWalkTargetSince, Registered)], move |cx| {
        let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
        let t = util::living(cx, id);
        let invalid = match &t {
            None => true,
            Some(t) => {
                !goals::can_attack(cx.m, &*cx.level, t)
                    || (can_grow_tired && tired_of_trying_to_reach(cx))
                    || !t.alive
                    || stop(cx, t)
            }
        };
        if invalid {
            erased(cx, id);
            cx.b.mem.erase(Mem::AttackTarget);
        }
        true
    })
}

/// `StopAttackingIfTargetInvalid.create()`: never stops for its own reasons.
pub fn stop_attacking_if_target_invalid_default() -> Box<dyn Control> {
    stop_attacking_if_target_invalid(|_, _| false, |_, _| {}, true)
}

/// `isTiredOfTryingToReachTarget`: it could not reach the target for over 200 ticks.
fn tired_of_trying_to_reach(cx: &Cx) -> bool {
    cx.b.mem.long(Mem::CantReachWalkTargetSince).is_some_and(|since| cx.time - since > 200)
}

/// `SetWalkTargetFromAttackTargetIfTargetOutOfReach.create(speed)`.
pub fn set_walk_target_from_attack_target_if_out_of_reach(speed: fn(&Cx) -> f32) -> Box<dyn Control> {
    shot(
        "SetWalkTargetFromAttackTargetIfTargetOutOfReach",
        &[
            (Mem::WalkTarget, Registered),
            (Mem::LookTarget, Registered),
            (Mem::AttackTarget, ValuePresent),
            (Mem::NearestVisibleLivingEntities, Registered),
        ],
        move |cx| {
            let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
            let Some(t) = util::living(cx, id) else { return false };
            let visible = cx.b.mem.has(Mem::NearestVisibleLivingEntities) && util::visible_contains(cx, id);
            if visible && within_attack_range(cx, &t, 1) {
                cx.b.mem.erase(Mem::WalkTarget);
            } else {
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                let w = WalkTarget { target: Tracker::entity(id, false), speed: speed(cx), close_enough: 0 };
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(w));
            }
            true
        },
    )
}

/// `BehaviorUtils.isWithinAttackRange(mob, target, extra)`: ranged weapons shoot from range
/// (the holder's default range less `extra`), everything else from melee reach.
pub fn within_attack_range(cx: &Cx, t: &Living, extra: i32) -> bool {
    if holds_projectile_weapon(cx) {
        // `getDefaultProjectileRange`: bows 15, crossbows 8 (`CROSSBOW_RANGE`), less the extra.
        let range = if cx.m.holding_bow() { 15 } else { 8 } - extra;
        return cx.e.position().distance_to_sqr(t.pos) < (range as f64) * (range as f64);
    }
    util::within_melee(cx, t)
}

/// `BackUpIfTooClose.create(tooCloseDistance, speed)`.
pub fn back_up_if_too_close(too_close: i32, speed: f32) -> Box<dyn Control> {
    shot(
        "BackUpIfTooClose",
        &[
            (Mem::WalkTarget, ValueAbsent),
            (Mem::LookTarget, Registered),
            (Mem::AttackTarget, ValuePresent),
            (Mem::NearestVisibleLivingEntities, ValuePresent),
        ],
        move |cx| {
            let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
            let Some(t) = util::living(cx, id) else { return false };
            let d = cx.e.position().distance_to_sqr(t.pos);
            if d < (too_close as f64) * (too_close as f64) && util::visible_contains(cx, id) {
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                cx.m.mov.strafe(-speed, 0.0);
                let head = cx.m.y_head_rot;
                cx.e.y_rot = mob::mth::rotate_if_necessary(cx.e.y_rot, head, 0.0);
                return true;
            }
            false
        },
    )
}
