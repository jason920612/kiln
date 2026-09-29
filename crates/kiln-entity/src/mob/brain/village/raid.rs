//! The villagers in danger and in raids: `VillagerPanicTrigger`, `VillagerCalmDown`, `SetRaidStatus`,
//! `ResetRaidStatus`, `ReactToBell`, `RingBell`, `LocateHidingPlace`, `SetHiddenState`,
//! `CelebrateVillagersSurvivedRaid`.

use kiln_javamath::random::RandomSource;
use super::{closer_to_center_than, gpos};
use crate::behavior_boilerplate;
use crate::level::PoiOccupancy;
use crate::math::BlockPos;
use crate::mob::brain::memory::{Val, WalkTarget};
use crate::mob::brain::{Activity, Behavior, Control, Cx, Mem, Shot, ShotBehavior, Status, Timed, shot};
use crate::mob::kinds::villager;
use Status::{Registered, ValueAbsent, ValuePresent};

/// `BehaviorBuilder.sequence(triggerIf(cond), inner)`: `inner` triggers when `cond` holds.
#[derive(Clone, Debug)]
struct SequenceIf {
    cond: fn(&Cx) -> bool,
    inner: Box<dyn Control>,
}

pub fn sequence_if(cond: fn(&Cx) -> bool, inner: Box<dyn Control>) -> Box<dyn Control> {
    Box::new(SequenceIf { cond, inner })
}

impl Control for SequenceIf {
    fn name(&self) -> &'static str {
        ""
    }
    fn running(&self) -> bool {
        false
    }
    fn required(&self, out: &mut Vec<Mem>) {
        self.inner.required(out);
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        (self.cond)(cx) && self.inner.try_start(cx)
    }
    fn tick_or_stop(&mut self, _cx: &mut Cx) {}
    fn do_stop(&mut self, _cx: &mut Cx) {}
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        self.inner.seed_gates(base, k);
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// `VillagerPanicTrigger`: hurt or with a hostile near, the villager panics (and every 100
/// ticks may call for an iron golem).
#[derive(Clone, Debug, Default)]
pub struct VillagerPanicTrigger;

impl VillagerPanicTrigger {
    pub fn new() -> Box<dyn Control> {
        Timed::new(VillagerPanicTrigger)
    }

    fn danger(cx: &Cx) -> bool {
        cx.b.mem.has(Mem::HurtBy) || cx.b.mem.has(Mem::NearestHostile)
    }
}

impl Behavior for VillagerPanicTrigger {
    fn name(&self) -> &'static str {
        "VillagerPanicTrigger"
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        Self::danger(cx)
    }
    fn start(&mut self, cx: &mut Cx) {
        if Self::danger(cx) {
            if !cx.b.is_active(Activity::Panic) {
                cx.b.mem.erase(Mem::Path);
                cx.b.mem.erase(Mem::WalkTarget);
                cx.b.mem.erase(Mem::LookTarget);
                cx.b.mem.erase(Mem::BreedTarget);
                cx.b.mem.erase(Mem::InteractionTarget);
            }
            cx.b.set_active_activity_if_possible(Activity::Panic);
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.time % 100 == 0 {
            villager::spawn_golem_if_needed(cx, 3);
        }
    }
    behavior_boilerplate!();
}

/// `VillagerCalmDown.create()`: nothing hurts or threatens the villager any more: it calms down
/// and goes back to its schedule.
pub fn villager_calm_down() -> Box<dyn Control> {
    shot("VillagerCalmDown", &[(Mem::HurtBy, Registered), (Mem::HurtByEntity, Registered), (Mem::NearestHostile, Registered)], |cx| {
        let near = |cx: &Cx, m: Mem| {
            cx.b.mem.entity(m).and_then(|id| cx.level.entity(id).map(|o| o.position())).is_some_and(|p| p.distance_to_sqr(cx.e.position()) <= 36.0)
        };
        // The memories are wired as the bytecode has them: hurt-by, then the hostile (any
        // distance counts), then the entity that hurt it if it is within 6 blocks.
        let danger = cx.b.mem.has(Mem::HurtBy) || cx.b.mem.has(Mem::NearestHostile) || near(cx, Mem::HurtByEntity);
        if !danger {
            cx.b.mem.erase(Mem::HurtBy);
            cx.b.mem.erase(Mem::HurtByEntity);
            let t = cx.time;
            cx.b.update_activity_from_schedule(t, &*cx.level);
        }
        true
    })
}

/// `SetRaidStatus.create()`.
pub fn set_raid_status() -> Box<dyn Control> {
    shot("SetRaidStatus", &[], |cx| {
        if cx.rng().next_int_bounded(20) != 0 {
            return false;
        }
        let raid = cx.level.raid_at(cx.e.block_position()).map(|r| (r.groups_spawned > 0, r.between_waves));
        if let Some((first, between)) = raid {
            if !first || between {
                cx.b.set_default_activity(Activity::PreRaid);
                cx.b.set_active_activity_if_possible(Activity::PreRaid);
            } else {
                cx.b.set_default_activity(Activity::Raid);
                cx.b.set_active_activity_if_possible(Activity::Raid);
            }
        }
        true
    })
}

/// `ResetRaidStatus.create()`.
pub fn reset_raid_status() -> Box<dyn Control> {
    shot("ResetRaidStatus", &[], |cx| {
        if cx.rng().next_int_bounded(20) != 0 {
            return false;
        }
        let over = match cx.level.raid_at(cx.e.block_position()) {
            None => true,
            Some(r) => (!r.active && !r.over) || r.loss,
        };
        if over {
            cx.b.set_default_activity(Activity::Idle);
            let t = cx.time;
            cx.b.update_activity_from_schedule(t, &*cx.level);
        }
        true
    })
}

/// `ReactToBell.create()`: outside a raid, hearing a bell sends the villager hiding.
pub fn react_to_bell() -> Box<dyn Control> {
    shot("ReactToBell", &[(Mem::HeardBellTime, ValuePresent)], |cx| {
        if cx.level.raid_at(cx.e.block_position()).is_none() {
            cx.b.set_active_activity_if_possible(Activity::Hide);
        }
        true
    })
}

/// `RingBell.create()`: at the bell, now and then, a villager rings it.
pub fn ring_bell() -> Box<dyn Control> {
    shot("RingBell", &[(Mem::MeetingPoint, ValuePresent)], |cx| {
        if cx.rng().next_float() <= 0.95 {
            return false;
        }
        let Some(pos) = super::mem_pos(cx, Mem::MeetingPoint) else { return false };
        let here = cx.e.block_position();
        if super::dist_sqr(pos, here) < 9.0 && crate::blocks::block_name(cx.level.block(pos)) == "minecraft:bell" {
            villager::ring_bell(cx, pos);
        }
        true
    })
}

/// `LocateHidingPlace.create(radius, speed, closeEnough)`.
#[derive(Clone, Debug)]
pub struct LocateHidingPlace {
    radius: i32,
    speed: f32,
    close_enough: i32,
}

impl LocateHidingPlace {
    pub fn new(radius: i32, speed: f32, close_enough: i32) -> Box<dyn Control> {
        Shot::new(LocateHidingPlace { radius, speed, close_enough })
    }
}

impl ShotBehavior for LocateHidingPlace {
    fn name(&self) -> &'static str {
        "LocateHidingPlace"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::WalkTarget, ValueAbsent),
            (Mem::Home, Registered),
            (Mem::HidingPlace, Registered),
            (Mem::Path, Registered),
            (Mem::LookTarget, Registered),
            (Mem::BreedTarget, Registered),
            (Mem::InteractionTarget, Registered),
        ]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let center = cx.e.block_position();
        // `poi.find(home, any, center, closeEnough + 1, ANY)` (the first one), kept if it is
        // within `closeEnough` of the villager ...
        let first = cx.level.poi_in_range(&["minecraft:home"], center, self.close_enough + 1, PoiOccupancy::Any).into_iter().next();
        let mut found = first.filter(|p| closer_to_center_than(*p, cx.e.position(), self.close_enough as f64));
        // ... else a random home within `radius` (`getRandom`: a shuffled list) ...
        if found.is_none() {
            let mut list = cx.level.poi_in_range(&["minecraft:home"], center, self.radius, PoiOccupancy::Any);
            for j in (2..=list.len()).rev() {
                let k = cx.e.random.next_int_bounded(j as i32) as usize;
                list.swap(j - 1, k);
            }
            found = list.first().copied();
        }
        // ... else the home the villager has.
        if found.is_none() {
            found = super::mem_pos(cx, Mem::Home);
        }
        if let Some(pos) = found {
            cx.b.mem.erase(Mem::Path);
            cx.b.mem.erase(Mem::LookTarget);
            cx.b.mem.erase(Mem::BreedTarget);
            cx.b.mem.erase(Mem::InteractionTarget);
            cx.b.mem.set(Mem::HidingPlace, gpos(pos));
            if !closer_to_center_than(pos, cx.e.position(), self.close_enough as f64) {
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(pos, self.speed, self.close_enough)));
            }
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `SetHiddenState.create(radius, hideBase)`: hides for at most 300 ticks after the bell (or
/// 20 * radius ticks spent at the hiding place).
#[derive(Clone, Debug)]
pub struct SetHiddenState {
    max_counter: i32,
    close: i32,
    counter: i32,
}

impl SetHiddenState {
    pub fn new(radius: i32, close: i32) -> Box<dyn Control> {
        Shot::new(SetHiddenState { max_counter: radius * 20, close, counter: 0 })
    }
}

impl ShotBehavior for SetHiddenState {
    fn name(&self) -> &'static str {
        "SetHiddenState"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::HidingPlace, ValuePresent), (Mem::HeardBellTime, ValuePresent)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let heard = cx.b.mem.long(Mem::HeardBellTime).unwrap_or(0);
        let timed_out = heard + 300 <= cx.time;
        if self.counter > self.max_counter || timed_out {
            cx.b.mem.erase(Mem::HeardBellTime);
            cx.b.mem.erase(Mem::HidingPlace);
            let t = cx.time;
            cx.b.update_activity_from_schedule(t, &*cx.level);
            self.counter = 0;
            return true;
        }
        if let Some(p) = super::mem_pos(cx, Mem::HidingPlace)
            && super::dist_sqr(p, cx.e.block_position()) < (self.close * self.close) as f64
        {
            self.counter += 1;
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `CelebrateVillagersSurvivedRaid(600, 600)`.
#[derive(Clone, Debug, Default)]
pub struct CelebrateVillagersSurvivedRaid {
    raid: Option<i32>,
}

impl CelebrateVillagersSurvivedRaid {
    pub fn new() -> Box<dyn Control> {
        Timed::new(CelebrateVillagersSurvivedRaid::default())
    }
}

impl Behavior for CelebrateVillagersSurvivedRaid {
    fn name(&self) -> &'static str {
        "CelebrateVillagersSurvivedRaid"
    }
    fn duration(&self) -> (i32, i32) {
        (600, 600)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        let pos: BlockPos = cx.e.block_position();
        let raid = cx.level.raid_at(pos).map(|r| (r.id, r.over && !r.loss));
        self.raid = raid.map(|(id, _)| id);
        raid.is_some_and(|(_, victory)| victory) && super::stroll::has_no_blocks_above(cx, pos)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        self.raid.is_some_and(|id| cx.level.raid(id).is_some_and(|r| r.active || r.over))
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.e.random.next_int_bounded(100) == 0 {
            villager::play_celebrate_sound(cx.e, cx.m, cx.level);
        }
        if cx.e.random.next_int_bounded(200) == 0 && super::stroll::has_no_blocks_above(cx, cx.e.block_position()) {
            let color = cx.e.random.next_int_bounded(16);
            let flight = cx.e.random.next_int_bounded(3);
            villager::launch_firework(cx, color, flight);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        self.raid = None;
        let t = cx.time;
        cx.b.update_activity_from_schedule(t, &*cx.level);
    }
    behavior_boilerplate!();
}
