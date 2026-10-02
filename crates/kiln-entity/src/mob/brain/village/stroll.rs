//! Where villagers walk: `StrollAroundPoi`, `StrollToPoi`, `StrollToPoiList`, `InsideBrownianWalk`,
//! `GoToClosestVillage`, `VillageBoundRandomStroll`, `SetWalkTargetAwayFrom`, `MoveToSkySeeingSpot`,
//! `JumpOnBed`, and the doors they open on the way (`InteractWithDoor`).

use kiln_javamath::random::RandomSource;
use super::poi::bed_occupied_state;
use super::{closer_to_center_than, mem_pos};
use crate::behavior_boilerplate;
use crate::math::{BlockPos, Vec3};
use crate::mob::brain::memory::{GlobalPos, Val, WalkTarget};
use crate::mob::brain::util;
use crate::mob::brain::{Behavior, Control, Cx, Mem, Shot, ShotBehavior, Status, Timed, shot};
use crate::mob::kinds::wolf::block_in_tag;
use crate::mob::random_pos;
use Status::{Registered, ValueAbsent, ValuePresent};

// ---------------------------------------------------------------------------- around a point

/// The walk-target-carrying strolls that remember when they may next start.
#[derive(Clone, Debug)]
enum Kind {
    /// `StrollAroundPoi.create(memory, speed, maxDistance)`.
    Around { max_distance: i32 },
    /// `StrollToPoi.create(memory, speed, closeEnough, maxDistance)`.
    To { close_enough: i32, max_distance: i32 },
}

#[derive(Clone, Debug)]
pub struct StrollPoi {
    kind: Kind,
    mem: Mem,
    speed: f32,
    next: i64,
}

impl StrollPoi {
    /// `StrollAroundPoi.create(memory, speed, maxDistance)`: within `max_distance` of the point,
    /// a spot near it every 180 ticks.
    pub fn around(mem: Mem, speed: f32, max_distance: i32) -> Box<dyn Control> {
        Shot::new(StrollPoi { kind: Kind::Around { max_distance }, mem, speed, next: 0 })
    }

    /// `StrollToPoi.create(memory, speed, closeEnough, maxDistance)`.
    pub fn to(mem: Mem, speed: f32, close_enough: i32, max_distance: i32) -> Box<dyn Control> {
        Shot::new(StrollPoi { kind: Kind::To { close_enough, max_distance }, mem, speed, next: 0 })
    }
}

impl ShotBehavior for StrollPoi {
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Around { .. } => "StrollAroundPoi",
            Kind::To { .. } => "StrollToPoi",
        }
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        match self.mem {
            Mem::JobSite => &[(Mem::WalkTarget, Registered), (Mem::JobSite, ValuePresent)],
            Mem::MeetingPoint => &[(Mem::WalkTarget, Registered), (Mem::MeetingPoint, ValuePresent)],
            _ => panic!("stroll around {:?}", self.mem),
        }
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let Some(pos) = mem_pos(cx, self.mem) else { return false };
        let max = match self.kind {
            Kind::Around { max_distance } | Kind::To { max_distance, .. } => max_distance,
        };
        if !closer_to_center_than(pos, cx.e.position(), max as f64) {
            return false;
        }
        if cx.time > self.next {
            match self.kind {
                Kind::Around { .. } => {
                    let v = random_pos::land_pos(cx.e, cx.m, &*cx.level, 8, 6);
                    match v {
                        Some(v) => cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(v, self.speed, 1))),
                        None => cx.b.mem.erase(Mem::WalkTarget),
                    }
                    self.next = cx.time + 180;
                }
                Kind::To { close_enough, .. } => {
                    cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(pos, self.speed, close_enough)));
                    self.next = cx.time + 80;
                }
            }
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `StrollToPoiList.create(listMemory, speed, closeEnough, maxDistance, poiMemory)`.
#[derive(Clone, Debug)]
pub struct StrollToPoiList {
    speed: f32,
    close_enough: i32,
    max_distance: i32,
    next: i64,
}

impl StrollToPoiList {
    /// Over `SECONDARY_JOB_SITE` and `JOB_SITE`.
    pub fn new(speed: f32, close_enough: i32, max_distance: i32) -> Box<dyn Control> {
        Shot::new(StrollToPoiList { speed, close_enough, max_distance, next: 0 })
    }
}

impl ShotBehavior for StrollToPoiList {
    fn name(&self) -> &'static str {
        "StrollToPoiList"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, Registered), (Mem::SecondaryJobSite, ValuePresent), (Mem::JobSite, ValuePresent)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let list: Vec<GlobalPos> = cx.b.mem.positions(Mem::SecondaryJobSite).to_vec();
        let Some(site) = mem_pos(cx, Mem::JobSite) else { return false };
        if list.is_empty() {
            return false;
        }
        let i = cx.rng().next_int_bounded(list.len() as i32) as usize;
        let chosen = &list[i];
        if !closer_to_center_than(site, cx.e.position(), self.max_distance as f64) {
            return false;
        }
        if cx.time > self.next {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(chosen.pos, self.speed, self.close_enough)));
            self.next = cx.time + 100;
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

// ---------------------------------------------------------------------------- indoors and villages

/// `InsideBrownianWalk.create(speed)`: indoors, a random open block next to the villager. (Vanilla
/// shuffles with an unseeded `java.util.Random`; the level's stream stands in.)
pub fn inside_brownian_walk(speed: f32) -> Box<dyn Control> {
    shot("InsideBrownianWalk", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        if cx.level.can_see_sky(cx.e.block_position()) {
            return false;
        }
        let c = cx.e.block_position();
        let mut list = Vec::with_capacity(27);
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    list.push(c.offset(x, y, z));
                }
            }
        }
        // `Collections.shuffle`.
        for i in (1..list.len()).rev() {
            let j = cx.rng().next_int_bounded(i as i32 + 1) as usize;
            list.swap(i, j);
        }
        let pick = list.into_iter().find(|p| {
            !cx.level.can_see_sky(*p) && loaded_and_entity_can_stand_on(cx, *p) && crate::collision::no_collision(&*cx.level, &cx.e.collision_context(), cx.e.id, &box_at(cx, *p))
        });
        if let Some(p) = pick {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(p, speed, 0)));
        }
        true
    })
}

/// The mob's box moved so that its feet are at the bottom centre of `p` (`Level.noCollision(entity, box)`
/// asks about the box at the position: `getBoundingBox()` translated).
fn box_at(cx: &Cx, p: BlockPos) -> crate::math::Aabb {
    let bb = cx.e.bounding_box();
    let (dx, dy, dz) = (p.x as f64 + 0.5 - cx.e.x(), p.y as f64 - cx.e.y(), p.z as f64 + 0.5 - cx.e.z());
    // `ServerLevel.noCollision(BlockPos)` (an entity `getBoundingBox` at the block): the mob's box
    // as it is now (vanilla passes the mob and the block; the block is only where the box
    // is computed from).
    let _ = (dx, dy, dz);
    bb
}

/// `PathfinderMob.loadedAndEntityCanStandOn(pos)`: the block below has a full top face.
fn loaded_and_entity_can_stand_on(cx: &Cx, p: BlockPos) -> bool {
    cx.level.is_loaded(p) && crate::mob::path::is_stable_destination(&*cx.level, p) && kiln_data::block_props::solid_render(cx.level.block(p.below()))
}

/// `GoToClosestVillage.create(speed, closeEnough)`.
pub fn go_to_closest_village(speed: f32, close_enough: i32) -> Box<dyn Control> {
    shot("GoToClosestVillage", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        let here = cx.e.block_position();
        if cx.level.is_village(here) {
            return false;
        }
        let best_now = cx.level.sections_to_village(here);
        let mut best: Option<Vec3> = None;
        for _ in 0..5 {
            let level = &*cx.level;
            let weight = |p: BlockPos| -(level.sections_to_village(p) as f64);
            let Some(v) = random_pos::land_pos_weighted(cx.e, cx.m, level, 15, 7, &weight) else { continue };
            let s = cx.level.sections_to_village(BlockPos::containing(v.x, v.y, v.z));
            if s < best_now {
                best = Some(v);
                break;
            }
            if s == best_now {
                best = Some(v);
            }
        }
        if let Some(v) = best {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(v, speed, close_enough)));
        }
        true
    })
}

/// `BehaviorUtils.findSectionClosestToVillage(level, section, 2)`: the section within 2 (a cube,
/// x fastest) nearest to a village, the section itself when none is nearer.
fn find_section_closest_to_village(cx: &Cx, section: (i32, i32, i32), radius: i32) -> (i32, i32, i32) {
    // One scan for the village sections around, when the level lists them: the distance of a
    // section is then the nearest of them by cube distance, 7 when none is within 6.
    let centers = cx.level.village_centers_near(section, radius + 6);
    let distance = |x: i32, y: i32, z: i32| match &centers {
        Some(c) => c.iter().map(|&(cx, cy, cz)| (cx - x).abs().max((cy - y).abs()).max((cz - z).abs())).min().map_or(7, |d| d.min(7)),
        None => cx.level.sections_to_village(BlockPos::new(x << 4, y << 4, z << 4)),
    };
    let here = distance(section.0, section.1, section.2);
    let mut best: Option<((i32, i32, i32), i32)> = None;
    for z in section.2 - radius..=section.2 + radius {
        for y in section.1 - radius..=section.1 + radius {
            for x in section.0 - radius..=section.0 + radius {
                let d = distance(x, y, z);
                if d < here && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some(((x, y, z), d));
                }
            }
        }
    }
    best.map_or(section, |(s, _)| s)
}

/// `VillageBoundRandomStroll.create(speed, maxXZ, maxY)`: strolls in the village, or toward the
/// nearest one.
pub fn village_bound_random_stroll(speed: f32, max_xz: i32, max_y: i32) -> Box<dyn Control> {
    shot("VillageBoundRandomStroll", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        let here = cx.e.block_position();
        let v = if cx.level.is_village(here) {
            random_pos::land_pos(cx.e, cx.m, &*cx.level, max_xz, max_y)
        } else {
            let section = (here.x >> 4, here.y >> 4, here.z >> 4);
            let best = find_section_closest_to_village(cx, section, 2);
            if best == section {
                random_pos::land_pos(cx.e, cx.m, &*cx.level, max_xz, max_y)
            } else {
                // `SectionPos.center()` then `Vec3.atBottomCenterOf`.
                let c = BlockPos::new((best.0 << 4) + 8, (best.1 << 4) + 8, (best.2 << 4) + 8);
                let target = Vec3::new(c.x as f64 + 0.5, c.y as f64, c.z as f64 + 0.5);
                random_pos::default_pos_towards(cx.e, cx.m, &*cx.level, max_xz, max_y, target, 1.5707963705062866)
            }
        };
        match v {
            Some(v) => cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(v, speed, 0))),
            None => cx.b.mem.erase(Mem::WalkTarget),
        }
        true
    })
}

// ---------------------------------------------------------------------------- away and up

/// `SetWalkTargetAwayFrom.entity(memory, speed, closeEnough, invalidate)`: runs from the entity
/// in `mem`.
pub fn walk_away_from_entity(mem: Mem, speed: f32, close_enough: i32, invalidate: bool) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match mem {
        Mem::NearestHostile => &[(Mem::WalkTarget, Registered), (Mem::NearestHostile, ValuePresent)],
        Mem::HurtByEntity => &[(Mem::WalkTarget, Registered), (Mem::HurtByEntity, ValuePresent)],
        _ => panic!("SetWalkTargetAwayFrom over {mem:?}"),
    };
    shot("SetWalkTargetAwayFrom", entry, move |cx| {
        let walk = cx.b.mem.walk_target();
        if walk.is_some() && !invalidate {
            return false;
        }
        let here = cx.e.position();
        let Some(from_id) = cx.b.mem.entity(mem) else { return false };
        let Some(from) = util::living(cx, from_id).map(|l| l.pos) else { return false };
        if !(here.distance_to_sqr(from) < (close_enough * close_enough) as f64) {
            return false;
        }
        if let Some(w) = walk
            && w.speed == speed
            && let Some(tp) = util::tracker_pos(cx, &w.target)
        {
            let a = tp - here;
            let b = from - here;
            if a.x * b.x + a.y * b.y + a.z * b.z < 0.0 {
                return false;
            }
        }
        for _ in 0..10 {
            if let Some(p) = random_pos::land_pos_away(cx.e, cx.m, &*cx.level, 16, 7, from) {
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, speed, 0)));
                break;
            }
        }
        true
    })
}

/// `MoveToSkySeeingSpot.hasNoBlocksAbove`.
pub fn has_no_blocks_above(cx: &Cx, p: BlockPos) -> bool {
    cx.level.can_see_sky(p) && cx.level.heightmap(p.x, p.z, false) as f64 <= cx.e.y()
}

/// `MoveToSkySeeingSpot.create(speed)`: indoors, walks to a spot under the sky.
pub fn move_to_sky_seeing_spot(speed: f32) -> Box<dyn Control> {
    shot("MoveToSkySeeingSpot", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        if cx.level.can_see_sky(cx.e.block_position()) {
            return false;
        }
        let center = cx.e.block_position();
        for _ in 0..10 {
            let dx = cx.e.random.next_int_bounded(20) - 10;
            let dy = cx.e.random.next_int_bounded(6) - 3;
            let dz = cx.e.random.next_int_bounded(20) - 10;
            let p = center.offset(dx, dy, dz);
            if has_no_blocks_above(cx, p) {
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5), speed, 0)));
                break;
            }
        }
        true
    })
}

// ---------------------------------------------------------------------------- JumpOnBed

/// `JumpOnBed(speed)`: a baby near a bed walks to it and jumps on it 3 to 6 times.
#[derive(Clone, Debug)]
pub struct JumpOnBed {
    speed: f32,
    target_bed: Option<BlockPos>,
    remaining_time_to_reach: i32,
    remaining_jumps: i32,
    remaining_cooldown: i32,
}

impl JumpOnBed {
    pub fn new(speed: f32) -> Box<dyn Control> {
        Timed::new(JumpOnBed { speed, target_bed: None, remaining_time_to_reach: 0, remaining_jumps: 0, remaining_cooldown: 0 })
    }

    fn jumpable(cx: &Cx, p: BlockPos) -> bool {
        block_in_tag(cx.level.block(p), "minecraft:villager_babies_can_jump_on_bed")
    }

    fn on_or_over_bed(cx: &Cx) -> bool {
        let p = cx.e.block_position();
        Self::jumpable(cx, p) || Self::jumpable(cx, p.below())
    }

    fn nearest_bed(cx: &Cx) -> Option<BlockPos> {
        match cx.b.mem.get(Mem::NearestBed) {
            Some(Val::Block(p)) => Some(*p),
            Some(Val::Pos(g)) => Some(g.pos),
            _ => None,
        }
    }
}

impl Behavior for JumpOnBed {
    fn name(&self) -> &'static str {
        "JumpOnBed"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::NearestBed, ValuePresent), (Mem::WalkTarget, ValueAbsent)]
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.m.baby() && (Self::on_or_over_bed(cx) || Self::nearest_bed(cx).is_some())
    }
    fn start(&mut self, cx: &mut Cx) {
        if let Some(bed) = Self::nearest_bed(cx) {
            self.target_bed = Some(bed);
            self.remaining_time_to_reach = 100;
            self.remaining_jumps = 3 + cx.rng().next_int_bounded(4);
            self.remaining_cooldown = 0;
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(bed, self.speed, 0)));
        }
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let Some(bed) = self.target_bed else { return false };
        let on = Self::on_or_over_bed(cx);
        cx.m.baby() && Self::jumpable(cx, bed) && !(!on && self.remaining_time_to_reach <= 0) && !(on && self.remaining_jumps <= 0)
    }
    fn tick(&mut self, cx: &mut Cx) {
        if !Self::on_or_over_bed(cx) {
            self.remaining_time_to_reach -= 1;
            return;
        }
        if self.remaining_cooldown > 0 {
            self.remaining_cooldown -= 1;
            return;
        }
        if Self::jumpable(cx, cx.e.block_position()) {
            cx.m.jump.jump = true;
            self.remaining_jumps -= 1;
            self.remaining_cooldown = 5;
        }
    }
    fn stop(&mut self, _cx: &mut Cx) {
        self.target_bed = None;
        self.remaining_time_to_reach = 0;
        self.remaining_jumps = 0;
        self.remaining_cooldown = 0;
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- doors

/// `InteractWithDoor.create()`: opens the doors on the path, and closes the ones passed through.
#[derive(Clone, Debug, Default)]
pub struct InteractWithDoor {
    last_node: Option<BlockPos>,
    countdown: i32,
}

impl InteractWithDoor {
    pub fn new() -> Box<dyn Control> {
        Shot::new(InteractWithDoor::default())
    }
}

/// `DoorBlock.isOpen` and whether the block is a mob-interactable door.
fn door_open(state: u16) -> Option<bool> {
    if !block_in_tag(state, "minecraft:mob_interactable_doors") {
        return None;
    }
    // (`s.getBlock() instanceof DoorBlock`: doors, not trapdoors or gates.)
    let info = kiln_data::blocks_types::block_of(state);
    info.property(state, "half")?;
    info.property(state, "open").map(|v| v == "true")
}

/// `DoorBlock.setOpen(entity, level, state, pos, open)`: the door (both halves), its sound (the
/// pitch draws from the level's random) and the game event.
fn set_door_open(cx: &mut Cx, pos: BlockPos, open: bool) {
    let state = cx.level.block(pos);
    let info = kiln_data::blocks_types::block_of(state);
    if door_open(state) == Some(open) || door_open(state).is_none() {
        return;
    }
    let value = if open { "true" } else { "false" };
    let Some(new) = info.with_property(state, "open", value) else { return };
    cx.level.set_block(pos, new, 10);
    // The other half follows (`updateShape`).
    let other = if info.property(state, "half") == Some("lower") { pos.above() } else { pos.below() };
    let os = cx.level.block(other);
    if kiln_data::blocks_types::block_of(os).name == info.name
        && let Some(n2) = info.with_property(os, "open", value)
    {
        cx.level.set_block(other, n2, 10);
    }
    let _pitch = cx.rng().next_float() * 0.1 + 0.9;
    let (sound_open, sound_close) = door_sounds(info.name);
    let p = pos.center();
    let sound = if open { sound_open } else { sound_close };
    cx.level.emit(crate::level::Event::Sound { pos: p, sound, source: "blocks", volume: 1.0, pitch: _pitch });
    let id = cx.e.id;
    cx.level.emit(crate::level::Event::GameEvent { event: if open { "minecraft:block_open" } else { "minecraft:block_close" }, pos: p, entity: Some(id) });
}

/// A door's block set type sounds (`BlockSetType.doorOpen/doorClose`).
fn door_sounds(name: &str) -> (&'static str, &'static str) {
    match name {
        "minecraft:iron_door" => ("minecraft:block.iron_door.open", "minecraft:block.iron_door.close"),
        n if n.contains("copper") => ("minecraft:block.copper_door.open", "minecraft:block.copper_door.close"),
        n if n.contains("bamboo") => ("minecraft:block.bamboo_wood_door.open", "minecraft:block.bamboo_wood_door.close"),
        n if n.contains("cherry") => ("minecraft:block.cherry_wood_door.open", "minecraft:block.cherry_wood_door.close"),
        n if n.contains("crimson") || n.contains("warped") => ("minecraft:block.nether_wood_door.open", "minecraft:block.nether_wood_door.close"),
        _ => ("minecraft:block.wooden_door.open", "minecraft:block.wooden_door.close"),
    }
}

/// `InteractWithDoor.closeDoorsThatIHaveOpenedOrPassedThrough(level, mob, prev, next, doors, nearby)`.
pub fn close_doors_passed_through(cx: &mut Cx, prev: Option<BlockPos>, next: Option<BlockPos>) {
    let mut doors: Vec<GlobalPos> = match cx.b.mem.get(Mem::DoorsToClose) {
        Some(Val::Positions(v)) => v.clone(),
        _ => return,
    };
    let mut i = 0;
    while i < doors.len() {
        let pos = doors[i].pos;
        if prev == Some(pos) || next == Some(pos) {
            i += 1;
            continue;
        }
        let too_far = !closer_to_center_than(pos, cx.e.position(), 3.0);
        let state = cx.level.block(pos);
        let remove = if too_far {
            true
        } else {
            match door_open(state) {
                None => true,
                Some(false) => true,
                Some(true) => {
                    if !other_mobs_coming_through(cx, pos) {
                        set_door_open(cx, pos, false);
                    }
                    true
                }
            }
        };
        if remove {
            doors.remove(i);
        } else {
            i += 1;
        }
    }
    // The set is changed in place: it stays in the memory even when it ends up empty.
    if let Some(Val::Positions(v)) = cx.b.mem.get_mut(Mem::DoorsToClose) {
        *v = doors;
    }
}

/// `InteractWithDoor.areOtherMobsComingThroughDoor`.
fn other_mobs_coming_through(cx: &Cx, pos: BlockPos) -> bool {
    let ids = cx.b.mem.entities(Mem::NearestLivingEntities);
    ids.iter().any(|&id| {
        let Some(o) = cx.level.entity(id) else { return false };
        if o.type_name != cx.e.type_name || !closer_to_center_than(pos, o.position(), 2.0) {
            return false;
        }
        let Some(om) = crate::mob::data(o) else { return false };
        let Some(b) = om.brain.as_ref() else { return false };
        if !b.st.mem.has(Mem::Path) {
            return false;
        }
        match &om.nav.path {
            Some(p) if !p.is_done() && p.next > 0 => p.nodes[p.next - 1].pos() == pos || p.nodes[p.next].pos() == pos,
            _ => false,
        }
    })
}

impl ShotBehavior for InteractWithDoor {
    fn name(&self) -> &'static str {
        "InteractWithDoor"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::Path, ValuePresent), (Mem::DoorsToClose, Registered), (Mem::NearestLivingEntities, Registered)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let Some(path) = cx.m.nav.path.as_ref() else { return false };
        // `notStarted()` (no node passed yet) or `isDone()`.
        if path.next == 0 || path.is_done() {
            return false;
        }
        let next = path.nodes[path.next].pos();
        let prev = path.nodes[path.next - 1].pos();
        if self.last_node == Some(next) {
            self.countdown = 20;
        } else {
            self.countdown -= 1;
            if self.countdown > 0 {
                return false;
            }
        }
        self.last_node = Some(next);
        for p in [prev, next] {
            let state = cx.level.block(p);
            if door_open(state).is_some() {
                if door_open(state) == Some(false) {
                    set_door_open(cx, p, true);
                    remember_door(cx, p);
                } else if p == prev {
                    // The door behind is open already: remembered all the same.
                    remember_door(cx, p);
                }
            }
        }
        if matches!(cx.b.mem.get(Mem::DoorsToClose), Some(Val::Positions(_))) {
            close_doors_passed_through(cx, Some(prev), Some(next));
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `InteractWithDoor.rememberDoorToClose`: the door joins the set in `DOORS_TO_CLOSE`.
fn remember_door(cx: &mut Cx, pos: BlockPos) {
    let g = GlobalPos::new(super::DIM, pos);
    match cx.b.mem.get_mut(Mem::DoorsToClose) {
        Some(Val::Positions(v)) => {
            if !v.contains(&g) {
                v.push(g);
            }
        }
        _ => cx.b.mem.set(Mem::DoorsToClose, Val::Positions(vec![g])),
    }
}

/// Whether a bed block is occupied (re-exported for the social behaviours' use).
pub fn is_bed_occupied(state: u16) -> bool {
    bed_occupied_state(state)
}
