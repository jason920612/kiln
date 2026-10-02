//! The behaviours that claim, keep and use points of interest: `AcquirePoi`, `ValidateNearbyPoi`,
//! `PoiCompetitorScan`, `GoToPotentialJobSite`, `YieldJobSite`, `AssignProfessionFromJobSite`,
//! `ResetProfession`, `SetWalkTargetFromBlockMemory`, `SetClosestHomeAsWalkTarget`, `SleepInBed`,
//! `WakeUp`.

use kiln_javamath::random::RandomSource;
use super::sensors::{find_path_to_pois, valid_range};
use super::{closer_to_center_than, gpos, mem_pos};
use crate::behavior_boilerplate;
use crate::level::{Event, PoiOccupancy};
use crate::math::{BlockPos, Vec3};
use crate::mob::brain::memory::{Tracker, Val, WalkTarget};
use crate::mob::brain::util;
use crate::mob::brain::{Activity, Behavior, Control, Cx, Mem, Shot, ShotBehavior, Status, Timed, shot};
use crate::mob::kinds::villager;
use crate::mob::kinds::wolf::block_in_tag;
use crate::mob::{path, random_pos};
use Status::{Registered, ValueAbsent, ValuePresent};

// ---------------------------------------------------------------------------- AcquirePoi

/// The point of interest types an `AcquirePoi` looks for.
#[derive(Clone, Copy, Debug)]
pub enum PoiWant {
    /// `profession.acquirableJobSite()` (the profession the brain was built with).
    Job(&'static str),
    /// `profession.heldJobSite()`.
    HeldJob(&'static str),
    Home,
    Meeting,
}

impl PoiWant {
    /// The `minecraft:point_of_interest_type` names to ask the level for.
    pub fn types(self) -> &'static [&'static str] {
        match self {
            PoiWant::Job("minecraft:none") => &["#minecraft:acquirable_job_site"],
            PoiWant::Job("minecraft:nitwit") => &[],
            PoiWant::Job(p) | PoiWant::HeldJob(p) => job_type(p),
            PoiWant::Home => &["minecraft:home"],
            PoiWant::Meeting => &["minecraft:meeting"],
        }
    }
}

fn job_type(profession: &str) -> &'static [&'static str] {
    macro_rules! t {
        ($($n:literal),*) => {
            match profession {
                $($n => &[$n],)*
                _ => &[],
            }
        };
    }
    t!(
        "minecraft:armorer",
        "minecraft:butcher",
        "minecraft:cartographer",
        "minecraft:cleric",
        "minecraft:farmer",
        "minecraft:fisherman",
        "minecraft:fletcher",
        "minecraft:leatherworker",
        "minecraft:librarian",
        "minecraft:mason",
        "minecraft:shepherd",
        "minecraft:toolsmith",
        "minecraft:weaponsmith"
    )
}

/// `AcquirePoi.JitteredLinearRetry`.
#[derive(Clone, Debug)]
struct Retry {
    previous: i64,
    next: i64,
    delay: i32,
}

impl Retry {
    fn new(cx: &mut Cx, time: i64) -> Retry {
        let mut r = Retry { previous: 0, next: 0, delay: 0 };
        r.mark_attempt(cx, time);
        r
    }

    fn mark_attempt(&mut self, cx: &mut Cx, time: i64) {
        self.previous = time;
        let d = self.delay + cx.rng().next_int_bounded(40) + 40;
        self.delay = d.min(400);
        self.next = time + self.delay as i64;
    }

    fn still_valid(&self, time: i64) -> bool {
        time - self.previous < 400
    }

    fn should_retry(&self, time: i64) -> bool {
        time >= self.next
    }
}

/// `AcquirePoi.create(...)`: every 20-odd ticks, claims one of the five closest free points of
/// interest of the wanted type (that it can path to) into a memory.
#[derive(Clone, Debug)]
pub struct AcquirePoi {
    want: PoiWant,
    /// `acquirePoiMemory` (the memory that must be absent) and `poiMemory` (the one set).
    acquire: Mem,
    poi: Mem,
    only_if_adult: bool,
    event: Option<u8>,
    validate_bed: bool,
    next_start: i64,
    cache: Vec<(i64, Retry)>,
}

impl AcquirePoi {
    pub fn new(want: PoiWant, acquire: Mem, poi: Mem, only_if_adult: bool, event: Option<u8>, validate_bed: bool) -> Box<dyn Control> {
        Shot::new(AcquirePoi { want, acquire, poi, only_if_adult, event, validate_bed, next_start: 0, cache: Vec::new() })
    }
}

impl ShotBehavior for AcquirePoi {
    fn name(&self) -> &'static str {
        "AcquirePoi"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        // The outer builder (job sites: JOB_SITE absent) and the inner (POTENTIAL_JOB_SITE absent).
        match (self.acquire, self.poi) {
            (Mem::JobSite, Mem::PotentialJobSite) => &[(Mem::JobSite, ValueAbsent), (Mem::PotentialJobSite, ValueAbsent)],
            (Mem::Home, _) => &[(Mem::Home, ValueAbsent)],
            (Mem::MeetingPoint, _) => &[(Mem::MeetingPoint, ValueAbsent)],
            _ => &[],
        }
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        if self.only_if_adult && cx.m.baby() {
            return false;
        }
        let time = cx.time;
        if self.next_start == 0 {
            self.next_start = time + cx.rng().next_int_bounded(20) as i64;
            return false;
        }
        if time < self.next_start {
            return false;
        }
        self.next_start = time + 20 + cx.rng().next_int_bounded(20) as i64;
        self.cache.retain(|(_, r)| r.still_valid(time));
        let types = self.want.types();
        let center = cx.e.block_position();
        // `findAllWithType(.., posPredicate, ..)`: the retry cache judges every record in range.
        let mut found: Vec<BlockPos> = Vec::new();
        for p in cx.level.poi_in_range(types, center, 48, PoiOccupancy::HasSpace) {
            match self.cache.iter().position(|(k, _)| *k == p.as_long()) {
                None => found.push(p),
                Some(i) => {
                    if self.cache[i].1.should_retry(time) {
                        let mut r = self.cache[i].1.clone();
                        r.mark_attempt(cx, time);
                        self.cache[i].1 = r;
                        found.push(p);
                    }
                }
            }
        }
        // `.sorted(by distance).limit(5).filter(validate)`.
        found.sort_by(|a, b| super::dist_sqr(*a, center).total_cmp(&super::dist_sqr(*b, center)));
        found.truncate(5);
        if self.validate_bed {
            found.retain(|p| validate_bed_poi(cx, *p));
        }
        let path = find_path_to_pois(cx, &found);
        match path {
            Some(p) if p.reached => {
                let target = p.target;
                if cx.level.poi_type(target).is_some() {
                    cx.level.poi_take(types, target, 1, &|_, q| q == target);
                    cx.b.mem.set(self.poi, gpos(target));
                    if let Some(ev) = self.event {
                        let id = cx.e.id;
                        cx.level.emit(Event::EntityEvent { entity: id, event: ev });
                    }
                    self.cache.clear();
                }
            }
            _ => {
                for p in found {
                    if !self.cache.iter().any(|(k, _)| *k == p.as_long()) {
                        let r = Retry::new(cx, time);
                        self.cache.push((p.as_long(), r));
                    }
                }
            }
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `VillagerGoalPackages.validateBedPoi`: a bed villagers sleep on that nobody lies in.
fn validate_bed_poi(cx: &Cx, pos: BlockPos) -> bool {
    let s = cx.level.block(pos);
    block_in_tag(s, "minecraft:villagers_can_sleep_on_bed") && !bed_occupied_state(s)
}

/// `BedBlock.OCCUPIED`.
pub fn bed_occupied_state(state: u16) -> bool {
    kiln_data::blocks_types::block_of(state).property(state, "occupied") == Some("true")
}

// ---------------------------------------------------------------------------- ValidateNearbyPoi

/// `ValidateNearbyPoi.create(typePredicate, memory)`: forgets a point of interest that is gone
/// (or, for a bed, taken by someone else).
pub fn validate_nearby_poi(want: PoiWant, mem: Mem) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match mem {
        Mem::JobSite => &[(Mem::JobSite, ValuePresent)],
        Mem::PotentialJobSite => &[(Mem::PotentialJobSite, ValuePresent)],
        Mem::Home => &[(Mem::Home, ValuePresent)],
        Mem::MeetingPoint => &[(Mem::MeetingPoint, ValuePresent)],
        _ => panic!("ValidateNearbyPoi over {mem:?}"),
    };
    shot("ValidateNearbyPoi", entry, move |cx| {
        let Some(pos) = mem_pos(cx, mem) else { return false };
        if !closer_to_center_than(pos, cx.e.position(), 16.0) {
            return false;
        }
        let exists = cx.level.poi_type(pos).is_some_and(|t| want_accepts(want, t));
        if !exists {
            cx.b.mem.erase(mem);
        } else if bed_is_occupied(cx, pos) {
            cx.b.mem.erase(mem);
            if !bed_occupied_by_villager(cx, pos) {
                cx.level.poi_release(pos);
            }
        }
        true
    })
}

/// A `PoiType` predicate as `PoiWant` states it.
fn want_accepts(want: PoiWant, poi: &str) -> bool {
    match want {
        PoiWant::Job(p) => villager::acquirable_job_site(p, poi),
        PoiWant::HeldJob(p) => villager::held_job_site(p, poi),
        PoiWant::Home => poi == "minecraft:home",
        PoiWant::Meeting => poi == "minecraft:meeting",
    }
}

/// `ValidateNearbyPoi.bedIsOccupied`.
fn bed_is_occupied(cx: &Cx, pos: BlockPos) -> bool {
    let s = cx.level.block(pos);
    block_in_tag(s, "minecraft:villagers_can_sleep_on_bed") && bed_occupied_state(s) && !villager::is_sleeping(cx.m)
}

/// `ValidateNearbyPoi.bedIsOccupiedByVillager`: a sleeping villager whose box holds the position.
fn bed_occupied_by_villager(cx: &Cx, pos: BlockPos) -> bool {
    let area = crate::math::Aabb::of_block(pos);
    cx.level.entities_in(&area, crate::level::EntityFilter::Living, cx.e.id).into_iter().any(|id| {
        cx.level
            .entity(id)
            .is_some_and(|o| o.type_name == "minecraft:villager" && crate::mob::data(o).is_some_and(|d| villager::is_sleeping(d)))
    })
}

// ---------------------------------------------------------------------------- job sites

/// `PoiCompetitorScan`: two villagers that hold the same job site with the same profession:
/// the one with more experience keeps it.
pub fn poi_competitor_scan() -> Box<dyn Control> {
    shot("PoiCompetitorScan", &[(Mem::JobSite, ValuePresent), (Mem::NearestLivingEntities, ValuePresent)], |cx| {
        let Some(site) = cx.b.mem.global_pos(Mem::JobSite).cloned() else { return true };
        let Some(ty) = cx.level.poi_type(site.pos) else { return true };
        let my_id = cx.e.id;
        let mut winner_id = my_id;
        let mut winner_xp = villager::state(cx.m).map_or(0, |s| s.xp);
        // (Nothing in the loop changes the list: it is read by index, not copied.)
        for i in 0..cx.b.mem.entities(Mem::NearestLivingEntities).len() {
            let id = cx.b.mem.entities(Mem::NearestLivingEntities)[i];
            if id == my_id {
                continue;
            }
            let Some(o) = cx.level.entity(id) else { continue };
            if o.type_name != "minecraft:villager" || !o.is_alive() {
                continue;
            }
            let Some(om) = crate::mob::data(o) else { continue };
            let Some(ost) = villager::state(om) else { continue };
            let competes = om.brain.as_ref().and_then(|b| b.st.mem.global_pos(Mem::JobSite)).is_some_and(|g| *g == site) && villager::held_job_site(ost.profession, ty);
            if !competes {
                continue;
            }
            // `selectWinner(winner, other)`: more experience wins (ties go to the other one).
            let other_xp = ost.xp;
            let (loser, next_winner, next_xp) = if winner_xp > other_xp { (id, winner_id, winner_xp) } else { (winner_id, id, other_xp) };
            erase_job_site(cx, loser);
            winner_id = next_winner;
            winner_xp = next_xp;
        }
        true
    })
}

/// `getBrain().eraseMemory(JOB_SITE)` on villager `id` (the ticking one or another).
fn erase_job_site(cx: &mut Cx, id: i32) {
    if id == cx.e.id {
        cx.b.mem.erase(Mem::JobSite);
    } else if let Some(o) = cx.level.entity_mut(id)
        && let Some(om) = crate::mob::data_mut(o)
        && let Some(b) = om.brain.as_mut()
    {
        b.st.mem.erase(Mem::JobSite);
    }
}

/// `GoToPotentialJobSite(speed)`: walks to the claimed job site for at most 1200 ticks.
#[derive(Clone, Debug)]
pub struct GoToPotentialJobSite {
    pub speed: f32,
}

impl GoToPotentialJobSite {
    pub fn new(speed: f32) -> Box<dyn Control> {
        Timed::new(GoToPotentialJobSite { speed })
    }
}

impl Behavior for GoToPotentialJobSite {
    fn name(&self) -> &'static str {
        "GoToPotentialJobSite"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::PotentialJobSite, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (1200, 1200)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.b.active_non_core().is_none_or(|a| matches!(a, Activity::Idle | Activity::Work | Activity::Play))
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::PotentialJobSite)
    }
    fn tick(&mut self, cx: &mut Cx) {
        if let Some(pos) = mem_pos(cx, Mem::PotentialJobSite) {
            util::set_walk_and_look(cx, Tracker::block(pos), self.speed, 1);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        if let Some(pos) = mem_pos(cx, Mem::PotentialJobSite) {
            if cx.level.poi_type(pos).is_some() {
                cx.level.poi_release(pos);
            }
        }
        cx.b.mem.erase(Mem::PotentialJobSite);
    }
    behavior_boilerplate!();
}

/// `YieldJobSite.create(speed)`: an unemployed villager gives the job site it wanted to a
/// neighbour whose profession matches it.
pub fn yield_job_site(speed: f32) -> Box<dyn Control> {
    shot(
        "YieldJobSite",
        &[
            (Mem::PotentialJobSite, ValuePresent),
            (Mem::JobSite, ValueAbsent),
            (Mem::NearestLivingEntities, ValuePresent),
            (Mem::WalkTarget, Registered),
            (Mem::LookTarget, Registered),
        ],
        move |cx| {
            if cx.m.baby() {
                return false;
            }
            if villager::state(cx.m).is_none_or(|s| s.profession != "minecraft:none") {
                return false;
            }
            let Some(pos) = mem_pos(cx, Mem::PotentialJobSite) else { return false };
            let Some(ty) = cx.level.poi_type(pos) else { return true };
            let my_id = cx.e.id;
            let mut chosen = None;
            for id in cx.b.mem.entities(Mem::NearestLivingEntities).to_vec() {
                if id == my_id {
                    continue;
                }
                let Some(o) = cx.level.entity(id) else { continue };
                if o.type_name != "minecraft:villager" || !o.is_alive() {
                    continue;
                }
                if nearby_wants_jobsite(cx, id, ty, pos) {
                    chosen = Some(id);
                    break;
                }
            }
            if let Some(id) = chosen {
                cx.b.mem.erase(Mem::WalkTarget);
                cx.b.mem.erase(Mem::LookTarget);
                cx.b.mem.erase(Mem::PotentialJobSite);
                if let Some(o) = cx.level.entity_mut(id)
                    && let Some(om) = crate::mob::data_mut(o)
                    && let Some(b) = om.brain.as_mut()
                    && !b.st.mem.has(Mem::JobSite)
                {
                    let t = Tracker::block(pos);
                    b.st.mem.set(Mem::LookTarget, Val::Look(t));
                    b.st.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed, close_enough: 1 }));
                    b.st.mem.set(Mem::PotentialJobSite, gpos(pos));
                }
            }
            true
        },
    )
}

/// `YieldJobSite.nearbyWantsJobsite(type, villager, pos)` for villager `id`.
fn nearby_wants_jobsite(cx: &mut Cx, id: i32, ty: &'static str, pos: BlockPos) -> bool {
    let Some(o) = cx.level.entity(id) else { return false };
    let Some(om) = crate::mob::data(o) else { return false };
    let Some(b) = om.brain.as_ref() else { return false };
    if b.st.mem.has(Mem::PotentialJobSite) {
        return false;
    }
    let profession = villager::state(om).map_or("minecraft:none", |s| s.profession);
    if !villager::held_job_site(profession, ty) {
        return false;
    }
    match b.st.mem.global_pos(Mem::JobSite) {
        None => can_reach_pos(cx, id, pos, ty),
        Some(g) => g.pos == pos,
    }
}

/// `YieldJobSite.canReachPos`: whether villager `id`'s navigation reaches the position (the
/// navigation's own bookkeeping is left alone: the path is made on a copy).
fn can_reach_pos(cx: &Cx, id: i32, pos: BlockPos, ty: &str) -> bool {
    let Some(o) = cx.level.entity(id) else { return false };
    let Some(mut m) = crate::mob::data(o).cloned() else { return false };
    let p = path::create_path(o, &mut m, &*cx.level, pos, valid_range(ty));
    p.is_some_and(|p| p.reached)
}

/// `AssignProfessionFromJobSite`: arriving at the potential job site makes it the job site and
/// gives an unemployed villager the site's profession.
pub fn assign_profession_from_job_site() -> Box<dyn Control> {
    shot("AssignProfessionFromJobSite", &[(Mem::PotentialJobSite, ValuePresent), (Mem::JobSite, Registered)], |cx| {
        let Some(g) = cx.b.mem.global_pos(Mem::PotentialJobSite).cloned() else { return false };
        if !closer_to_center_than(g.pos, cx.e.position(), 2.0) {
            return false;
        }
        cx.b.mem.erase(Mem::PotentialJobSite);
        cx.b.mem.set(Mem::JobSite, Val::Pos(g.clone()));
        let id = cx.e.id;
        cx.level.emit(Event::EntityEvent { entity: id, event: 14 });
        if villager::state(cx.m).is_none_or(|s| s.profession != "minecraft:none") {
            return true;
        }
        if let Some(ty) = cx.level.poi_type(g.pos)
            && let Some(p) = villager::profession_of_job_site(ty)
            && let Some(st) = villager::state_mut(cx.m)
        {
            st.set_profession(p);
            cx.b.refresh_requested = true;
        }
        true
    })
}

/// `ResetProfession`: without a job site a villager that has not traded loses its profession.
pub fn reset_profession() -> Box<dyn Control> {
    shot("ResetProfession", &[(Mem::JobSite, ValueAbsent)], |cx| {
        let Some(st) = villager::state_mut(cx.m) else { return false };
        let has_profession = st.profession != "minecraft:none" && st.profession != "minecraft:nitwit";
        if has_profession && st.xp == 0 && st.level <= 1 {
            st.set_profession("minecraft:none");
            cx.b.refresh_requested = true;
            return true;
        }
        false
    })
}

// ---------------------------------------------------------------------------- walking to a memory

/// `SetWalkTargetFromBlockMemory.create(memory, speed, closeEnough, tooFar, tooLongUnreachable)`.
pub fn set_walk_target_from_block_memory(mem: Mem, speed: f32, close_enough: i32, too_far: i32, too_long_unreachable: i32) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match mem {
        Mem::JobSite => &[(Mem::CantReachWalkTargetSince, Registered), (Mem::WalkTarget, ValueAbsent), (Mem::JobSite, ValuePresent)],
        Mem::Home => &[(Mem::CantReachWalkTargetSince, Registered), (Mem::WalkTarget, ValueAbsent), (Mem::Home, ValuePresent)],
        Mem::MeetingPoint => &[(Mem::CantReachWalkTargetSince, Registered), (Mem::WalkTarget, ValueAbsent), (Mem::MeetingPoint, ValuePresent)],
        _ => panic!("SetWalkTargetFromBlockMemory over {mem:?}"),
    };
    shot("SetWalkTargetFromBlockMemory", entry, move |cx| {
        let Some(gp) = mem_pos(cx, mem) else { return false };
        let cant_reach = cx.b.mem.long(Mem::CantReachWalkTargetSince);
        if cant_reach.is_some_and(|t| cx.time - t > too_long_unreachable as i64) {
            release_poi(cx, mem);
            cx.b.mem.erase(mem);
            cx.b.mem.set(Mem::CantReachWalkTargetSince, Val::Long(cx.time));
            return true;
        }
        let here = cx.e.block_position();
        if util::dist_manhattan(gp, here) > too_far {
            let mut pos: Option<Vec3> = None;
            let mut tries = 0;
            loop {
                if let Some(p) = pos
                    && util::dist_manhattan(BlockPos::containing(p.x, p.y, p.z), here) <= too_far
                {
                    break;
                }
                let target = Vec3::new(gp.x as f64 + 0.5, gp.y as f64, gp.z as f64 + 0.5);
                pos = random_pos::default_pos_towards(cx.e, cx.m, &*cx.level, 15, 7, target, 1.5707963705062866);
                tries += 1;
                if tries == 1000 {
                    release_poi(cx, mem);
                    cx.b.mem.erase(mem);
                    cx.b.mem.set(Mem::CantReachWalkTargetSince, Val::Long(cx.time));
                    return true;
                }
            }
            if let Some(p) = pos {
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, speed, close_enough)));
            }
        } else if util::dist_manhattan(gp, here) > close_enough {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(gp, speed, close_enough)));
        }
        true
    })
}

/// `Villager.releasePoi(memory)`: gives the ticket of the point of interest in the memory back.
pub fn release_poi(cx: &mut Cx, mem: Mem) {
    let Some(pos) = mem_pos(cx, mem) else { return };
    let Some(ty) = cx.level.poi_type(pos) else { return };
    let profession = villager::state(cx.m).map_or("minecraft:none", |s| s.profession);
    let ok = match mem {
        Mem::Home => ty == "minecraft:home",
        Mem::JobSite => villager::held_job_site(profession, ty),
        Mem::PotentialJobSite => villager::acquirable_job_site("minecraft:none", ty),
        Mem::MeetingPoint => ty == "minecraft:meeting",
        _ => false,
    };
    if ok {
        cx.level.poi_release(pos);
    }
}

/// `SetClosestHomeAsWalkTarget.create(speed)`: no home yet: walks toward the closest bed.
#[derive(Clone, Debug)]
pub struct SetClosestHomeAsWalkTarget {
    speed: f32,
    next: i64,
    /// `batchCache` (never filled in vanilla).
    _cache: (),
}

impl SetClosestHomeAsWalkTarget {
    pub fn new(speed: f32) -> Box<dyn Control> {
        Shot::new(SetClosestHomeAsWalkTarget { speed, next: 0, _cache: () })
    }
}

impl ShotBehavior for SetClosestHomeAsWalkTarget {
    fn name(&self) -> &'static str {
        "SetClosestHomeAsWalkTarget"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, ValueAbsent), (Mem::Home, ValueAbsent)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        if cx.time - self.next < 20 {
            return false;
        }
        let center = cx.e.block_position();
        let closest = cx
            .level
            .poi_in_range(&["minecraft:home"], center, 48, PoiOccupancy::Any)
            .into_iter()
            .min_by(|a, b| super::dist_sqr(*a, center).total_cmp(&super::dist_sqr(*b, center)));
        let Some(closest) = closest else { return false };
        if super::dist_sqr(closest, center) <= 4.0 {
            return false;
        }
        self.next = cx.time + cx.rng().next_int_bounded(20) as i64;
        // The position filter lets the first four through (`incrementAndGet() < 5`).
        let found: Vec<BlockPos> = cx.level.poi_in_range(&["minecraft:home"], center, 48, PoiOccupancy::Any).into_iter().take(4).collect();
        if let Some(p) = find_path_to_pois(cx, &found)
            && p.reached
            && cx.level.poi_type(p.target).is_some()
        {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(p.target, self.speed, 1)));
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

// ---------------------------------------------------------------------------- sleeping

/// `SleepInBed`: lies down in the home bed once close enough, and stays while it is time to rest.
#[derive(Clone, Debug, Default)]
pub struct SleepInBed {
    next_ok_start: i64,
}

impl SleepInBed {
    pub fn new() -> Box<dyn Control> {
        Timed::new(SleepInBed::default())
    }
}

impl Behavior for SleepInBed {
    fn name(&self) -> &'static str {
        "SleepInBed"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::Home, ValuePresent),
            (Mem::LastWoken, Registered),
            (Mem::LastSlept, Registered),
            (Mem::WalkTarget, Registered),
            (Mem::CantReachWalkTargetSince, Registered),
        ]
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if cx.e.vehicle.is_some() {
            return false;
        }
        let Some(home) = mem_pos(cx, Mem::Home) else { return false };
        if let Some(woken) = cx.b.mem.long(Mem::LastWoken) {
            let since = cx.time - woken;
            if since > 0 && since < 100 {
                return false;
            }
        }
        let s = cx.level.block(home);
        closer_to_center_than(home, cx.e.position(), 2.0) && block_in_tag(s, "minecraft:villagers_can_sleep_on_bed") && !bed_occupied_state(s)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let Some(home) = mem_pos(cx, Mem::Home) else { return false };
        cx.b.is_active(Activity::Rest) && cx.e.y() > home.y as f64 + 0.4 && closer_to_center_than(home, cx.e.position(), 1.14)
    }
    fn start(&mut self, cx: &mut Cx) {
        if cx.time > self.next_ok_start {
            if cx.b.mem.has(Mem::DoorsToClose) {
                super::stroll::close_doors_passed_through(cx, None, None);
            }
            if let Some(home) = mem_pos(cx, Mem::Home)
                && villager::start_sleeping(cx.e, cx.m, cx.level, home)
            {
                cx.b.mem.set(Mem::LastSlept, Val::Long(cx.time));
            }
            cx.b.mem.erase(Mem::WalkTarget);
            cx.b.mem.erase(Mem::CantReachWalkTargetSince);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        if villager::is_sleeping(cx.m) {
            villager::stop_sleeping(cx.e, cx.m, cx.level, Some(&mut *cx.b));
            self.next_ok_start = cx.time + 40;
        }
    }
    behavior_boilerplate!();
}

/// `WakeUp.create()`: outside the rest activity a sleeping villager wakes.
pub fn wake_up() -> Box<dyn Control> {
    shot("WakeUp", &[], |cx| {
        if cx.b.is_active(Activity::Rest) || !villager::is_sleeping(cx.m) {
            return false;
        }
        villager::stop_sleeping(cx.e, cx.m, cx.level, Some(&mut *cx.b));
        true
    })
}

