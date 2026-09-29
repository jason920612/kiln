//! `BehaviorUtils`, `TargetingConditions` and `NearestVisibleLivingEntities` for behaviours.

use super::memory::{NearestVisible, Tracker, Val, WalkTarget};
use super::{Cx, Mem};
use crate::level::EntityFilter;
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::goals::{self, Living};
use kiln_javamath::random::RandomSource;

/// `LivingEntity` view of an entity or player id (`None` when it is gone or not living).
pub fn living(cx: &Cx, id: i32) -> Option<Living> {
    goals::living(&*cx.level, id)
}

/// A living entity that is alive, or a player that is (the `isAlive` filters).
pub fn alive(cx: &Cx, id: i32) -> Option<Living> {
    living(cx, id).filter(|l| l.alive)
}

/// `BlockPos.distManhattan`.
pub fn dist_manhattan(a: BlockPos, b: BlockPos) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs()
}

/// `Vec3i.distSqr` between two block positions.
pub fn dist_sqr_pos(a: BlockPos, b: BlockPos) -> f64 {
    let (dx, dy, dz) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
    dx * dx + dy * dy + dz * dz
}

/// `BlockPos.closerThan(vec, dist)`-style: squared distance from a block's centre... use
/// [`crate::math::Vec3::distance_to_sqr`] for entity positions.
pub fn dist_sqr_to_entity(cx: &Cx, pos: Vec3) -> f64 {
    cx.e.position().distance_to_sqr(pos)
}

/// `TargetingConditions` (`forNonCombat` / `forCombat`, `range`, `ignoreLineOfSight`,
/// `ignoreInvisibilityTesting`).
#[derive(Clone, Copy, Debug)]
pub struct Targeting {
    pub combat: bool,
    pub range: f64,
    pub ignore_los: bool,
    pub test_invisible: bool,
}

impl Targeting {
    pub const fn non_combat() -> Targeting {
        Targeting { combat: false, range: -1.0, ignore_los: false, test_invisible: true }
    }

    pub const fn combat() -> Targeting {
        Targeting { combat: true, range: -1.0, ignore_los: false, test_invisible: true }
    }

    pub const fn range(mut self, r: f64) -> Targeting {
        self.range = r;
        self
    }

    pub const fn ignore_line_of_sight(mut self) -> Targeting {
        self.ignore_los = true;
        self
    }

    pub const fn ignore_invisibility_testing(mut self) -> Targeting {
        self.test_invisible = false;
        self
    }

    /// `TargetingConditions.test(level, mob, target)`.
    pub fn test(&self, cx: &mut Cx, t: &Living) -> bool {
        if t.id == cx.e.id || !t.alive || t.spectator {
            return false;
        }
        if self.combat && !goals::can_attack(cx.m, &*cx.level, t) {
            return false;
        }
        if self.range > 0.0 {
            let mut vis = 1.0;
            if self.test_invisible {
                // `Entity.getVisibilityPercent`.
                if t.sneaking {
                    vis *= 0.8;
                }
                if t.invisible {
                    vis *= 0.7 * t.armor_cover.max(0.1) as f64;
                }
                vis = crate::mob::mth::clamp_d(vis, 0.0, 10.0);
            }
            let d = (self.range * vis).max(2.0);
            if cx.e.position().distance_to_sqr(t.pos) > d * d {
                return false;
            }
        }
        if !self.ignore_los && !crate::mob::has_line_of_sight_cached(cx.e, cx.m, &*cx.level, t) {
            return false;
        }
        true
    }
}

/// The follow range: what `Sensor.updateTargetingConditionRanges` sets before a scan.
pub fn follow_range(cx: &Cx) -> f64 {
    cx.m.attrs.value(crate::mob::attributes::Attr::FollowRange)
}

/// `Sensor.isEntityTargetable(level, body, target)`.
pub fn is_entity_targetable(cx: &mut Cx, t: &Living) -> bool {
    let c = if cx.b.mem.is(Mem::AttackTarget, &Val::Entity(t.id)) {
        Targeting::non_combat().range(follow_range(cx)).ignore_invisibility_testing()
    } else {
        Targeting::non_combat().range(follow_range(cx))
    };
    c.test(cx, t)
}

/// `Sensor.isEntityAttackable`.
pub fn is_entity_attackable(cx: &mut Cx, t: &Living) -> bool {
    let c = if cx.b.mem.is(Mem::AttackTarget, &Val::Entity(t.id)) {
        Targeting::combat().range(follow_range(cx)).ignore_invisibility_testing()
    } else {
        Targeting::combat().range(follow_range(cx))
    };
    c.test(cx, t)
}

/// `Sensor.isEntityAttackableIgnoringLineOfSight`.
pub fn is_entity_attackable_ignoring_los(cx: &mut Cx, t: &Living) -> bool {
    let c = if cx.b.mem.is(Mem::AttackTarget, &Val::Entity(t.id)) {
        Targeting::combat().range(follow_range(cx)).ignore_line_of_sight().ignore_invisibility_testing()
    } else {
        Targeting::combat().range(follow_range(cx)).ignore_line_of_sight()
    };
    c.test(cx, t)
}

/// One entity's line of sight test of the `NEAREST_VISIBLE_LIVING_ENTITIES` memory, asked once
/// per scan (`Object2BooleanOpenHashMap.computeIfAbsent`).
fn visible_test(cx: &mut Cx, nv: &mut NearestVisible, id: i32) -> bool {
    if let Some(&(_, v)) = nv.seen.iter().find(|(i, _)| *i == id) {
        return v;
    }
    let v = match living(cx, id) {
        Some(t) => is_entity_targetable(cx, &t),
        None => false,
    };
    nv.seen.push((id, v));
    v
}

/// Runs `f` with the `NEAREST_VISIBLE_LIVING_ENTITIES` value taken out of the memory (so the
/// tests can use the rest of `cx`), then puts it back with what was learned.
pub fn with_visible<R>(cx: &mut Cx, f: impl FnOnce(&mut Cx, &mut NearestVisible) -> R) -> Option<R> {
    let mut nv = match cx.b.mem.get_mut(Mem::NearestVisibleLivingEntities) {
        Some(Val::Visible(v)) => std::mem::take(v),
        _ => return None,
    };
    let r = f(cx, &mut nv);
    if let Some(Val::Visible(v)) = cx.b.mem.get_mut(Mem::NearestVisibleLivingEntities) {
        *v = nv;
    }
    Some(r)
}

/// `NearestVisibleLivingEntities.findClosest(predicate)`: the first nearby entity (nearest first)
/// that passes `pred` and the line of sight test. `None` also when the memory is absent.
pub fn find_closest_visible(cx: &mut Cx, mut pred: impl FnMut(&mut Cx, i32) -> bool) -> Option<i32> {
    with_visible(cx, |cx, nv| {
        for i in 0..nv.nearby.len() {
            let id = nv.nearby[i];
            if pred(cx, id) && visible_test(cx, nv, id) {
                return Some(id);
            }
        }
        None
    })
    .flatten()
}

/// `NearestVisibleLivingEntities.find(predicate)`: all that pass, nearest first.
pub fn find_all_visible(cx: &mut Cx, mut pred: impl FnMut(&mut Cx, i32) -> bool) -> Vec<i32> {
    with_visible(cx, |cx, nv| {
        let mut out = Vec::new();
        for i in 0..nv.nearby.len() {
            let id = nv.nearby[i];
            if pred(cx, id) && visible_test(cx, nv, id) {
                out.push(id);
            }
        }
        out
    })
    .unwrap_or_default()
}

/// `NearestVisibleLivingEntities.contains(entity)`.
pub fn visible_contains(cx: &mut Cx, id: i32) -> bool {
    with_visible(cx, |cx, nv| nv.nearby.contains(&id) && visible_test(cx, nv, id)).unwrap_or(false)
}

/// `BehaviorUtils.entityIsVisible`.
pub fn entity_is_visible(cx: &mut Cx, id: i32) -> bool {
    visible_contains(cx, id)
}

/// `PositionTracker.currentPosition`.
pub fn tracker_pos(cx: &Cx, t: &Tracker) -> Option<Vec3> {
    match *t {
        Tracker::Block { center, .. } => Some(center),
        Tracker::Entity { id, track_eye, .. } => {
            let l = living(cx, id)?;
            Some(if track_eye { Vec3::new(l.pos.x, l.eye_y, l.pos.z) } else { l.pos })
        }
    }
}

/// `PositionTracker.currentBlockPosition`.
pub fn tracker_block(cx: &Cx, t: &Tracker) -> Option<BlockPos> {
    match *t {
        Tracker::Block { pos, .. } => Some(pos),
        Tracker::Entity { id, target_eye, .. } => {
            let l = living(cx, id)?;
            Some(if target_eye { BlockPos::containing(l.pos.x, l.eye_y, l.pos.z) } else { BlockPos::containing(l.pos.x, l.pos.y, l.pos.z) })
        }
    }
}

/// `PositionTracker.isVisibleBy(entity)`.
pub fn tracker_visible(cx: &mut Cx, t: &Tracker) -> bool {
    match *t {
        Tracker::Block { .. } => true,
        Tracker::Entity { id, .. } => match living(cx, id) {
            None => true,
            Some(l) if !l.alive => false,
            Some(_) => visible_contains(cx, id),
        },
    }
}

/// `BehaviorUtils.lookAtEntity`.
pub fn look_at_entity(cx: &mut Cx, id: i32) {
    cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
}

/// `BehaviorUtils.setWalkAndLookTargetMemories(entity, tracker, speed, closeEnough)`.
pub fn set_walk_and_look(cx: &mut Cx, t: Tracker, speed: f32, close_enough: i32) {
    let w = WalkTarget { target: t, speed, close_enough };
    cx.b.mem.set(Mem::LookTarget, Val::Look(t));
    cx.b.mem.set(Mem::WalkTarget, Val::Walk(w));
}

/// `BehaviorUtils.canSee`.
pub fn can_see(cx: &mut Cx, id: i32) -> bool {
    if !cx.b.mem.has(Mem::NearestVisibleLivingEntities) {
        return false;
    }
    visible_contains(cx, id)
}

/// `BehaviorUtils.isBreeding`.
pub fn is_breeding(cx: &Cx) -> bool {
    cx.b.mem.has(Mem::BreedTarget)
}

/// `BehaviorUtils.isWithinAttackRange(mob, target, extra)` for a mob that is not holding a
/// projectile weapon (melee reach).
pub fn within_melee(cx: &Cx, t: &Living) -> bool {
    crate::mob::within_melee_range(cx.e, t)
}

/// `BehaviorUtils.isOtherTargetMuchFurtherAwayThanCurrentAttackTarget`.
pub fn other_target_much_further(cx: &Cx, other: &Living, extra: f64) -> bool {
    let Some(cur) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| living(cx, id)) else { return false };
    let d = cx.e.position().distance_to_sqr(cur.pos);
    let o = cx.e.position().distance_to_sqr(other.pos);
    o > d + extra * extra
}

/// `BehaviorUtils.getTargetNearestMe`.
pub fn nearest_of(cx: &Cx, a: &Living, b: &Living) -> i32 {
    let p = cx.e.position();
    if p.distance_to_sqr(a.pos) < p.distance_to_sqr(b.pos) { a.id } else { b.id }
}

/// `IntProvider.sample` for `UniformInt.of(min, max)`: `Mth.randomBetweenInclusive`.
pub fn uniform(r: &mut dyn RandomSource, min: i32, max: i32) -> i32 {
    r.next_int_bounded(max - min + 1) + min
}

/// The living entities and players around, nearest to `from` first (ties in listing order), as
/// `getEntitiesOfClass(LivingEntity.class, box, filter)` then a stable sort by distance.
pub fn living_in_box(cx: &Cx, area: &Aabb) -> Vec<i32> {
    let mut v: Vec<(f64, i32)> = Vec::new();
    let me = cx.e.id;
    for id in cx.level.entities_in(area, EntityFilter::Living, me) {
        if let Some(l) = living(cx, id)
            && l.alive
        {
            v.push((cx.e.position().distance_to_sqr(l.pos), id));
        }
    }
    for p in cx.level.players_in(area) {
        if p.id != me && p.alive {
            v.push((cx.e.position().distance_to_sqr(p.pos), p.id));
        }
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, id)| id).collect()
}
