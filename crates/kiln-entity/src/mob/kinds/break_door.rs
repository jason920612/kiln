//! `BreakDoorGoal` and `OpenDoorGoal` on top of `DoorInteractGoal`: a mob that has walked into a closed wooden door (its navigation may open doors, its
//! path leads through one) hacks at it for 240 ticks and then removes it, or opens it. Zombies (and husks, zombie villagers) get the break goal from
//! `setCanBreakDoors` and break doors on hard difficulty only; a vindicator in a raid has its own version (normal and hard) and opens doors.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::brain::village::stroll::{apply_door_open, door_open_changes};
use crate::mob::ext::CustomGoal;
use crate::mob::goals::MOVE;
use crate::mob::kinds::raider;
use crate::mob::mth::reduced_tick_delay;
use crate::mob::MobData;
use kiln_javamath::random::RandomSource;

/// `DEFAULT_DOOR_BREAK_TIME`; `getDoorBreakTime` is `Math.max(240, doorBreakTime)` and the vindicator's 6 never reaches it.
const BREAK_TIME: i32 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// `Zombie.DOOR_BREAKING_PREDICATE`: hard only.
    Zombie,
    /// `Vindicator.VindicatorBreakDoorGoal`: in a raid, one try in ten, normal and hard.
    Vindicator,
}

/// `DoorInteractGoal`'s state: the door the mob walks into and whether it has passed it.
#[derive(Clone, Debug)]
struct Interact {
    door_pos: BlockPos,
    has_door: bool,
    passed: bool,
    open_dir_x: f32,
    open_dir_z: f32,
}

impl Interact {
    fn new() -> Interact {
        Interact { door_pos: BlockPos::default(), has_door: false, passed: false, open_dir_x: 0.0, open_dir_z: 0.0 }
    }

    /// `DoorBlock.isWoodenDoor`: a door that opens by hand (not the iron one).
    fn is_wooden_door(level: &dyn EntityLevel, pos: BlockPos) -> bool {
        let name = crate::blocks::block_name(level.block(pos));
        name.ends_with("_door") && name != "minecraft:iron_door"
    }

    /// `DoorInteractGoal.isOpen`.
    fn is_open(&mut self, level: &dyn EntityLevel) -> bool {
        if !self.has_door {
            return false;
        }
        let state = level.block(self.door_pos);
        if !crate::blocks::block_name(state).ends_with("_door") {
            self.has_door = false;
            return false;
        }
        kiln_data::blocks_types::block_of(state).property(state, "open") == Some("true")
    }

    /// `DoorInteractGoal.canUse`.
    fn can_use(&mut self, e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
        if m.nav.water_bound || !e.horizontal_collision {
            return false;
        }
        let Some(path) = m.nav.path.as_ref().filter(|p| !p.is_done()) else { return false };
        for i in 0..(path.next + 2).min(path.nodes.len()) {
            let n = &path.nodes[i];
            self.door_pos = BlockPos::new(n.x, n.y + 1, n.z);
            let (dx, dz) = (n.x as f64 - e.x(), n.z as f64 - e.z());
            if dx * dx + dz * dz > 2.25 {
                continue;
            }
            self.has_door = Self::is_wooden_door(level, self.door_pos);
            if self.has_door {
                return true;
            }
        }
        let at = e.block_position();
        self.door_pos = BlockPos::new(at.x, at.y + 1, at.z);
        self.has_door = Self::is_wooden_door(level, self.door_pos);
        self.has_door
    }

    fn start(&mut self, e: &Entity) {
        self.passed = false;
        self.open_dir_x = (self.door_pos.x as f64 + 0.5 - e.x()) as f32;
        self.open_dir_z = (self.door_pos.z as f64 + 0.5 - e.z()) as f32;
    }

    /// `DoorInteractGoal.tick`: past the door once it lies behind.
    fn tick(&mut self, e: &Entity) {
        let dx = (self.door_pos.x as f64 + 0.5 - e.x()) as f32;
        let dz = (self.door_pos.z as f64 + 0.5 - e.z()) as f32;
        if self.open_dir_x * dx + self.open_dir_z * dz < 0.0 {
            self.passed = true;
        }
    }

    /// `DoorInteractGoal.setOpen`.
    fn set_open(&self, e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, open: bool) {
        if self.has_door && door_open_changes(&*level, self.door_pos, open) {
            let r = match level.shared_ai_random() {
                Some(r) => r.next_float(),
                None => m.brain_random.next_float(),
            };
            apply_door_open(level, e.id, self.door_pos, open, r * 0.1 + 0.9);
        }
    }
}

#[derive(Clone, Debug)]
pub struct BreakDoorGoal {
    who: Who,
    door: Interact,
    break_time: i32,
    last_progress: i32,
    /// The tick the arm last swung in (`Mob.isSwinging` for the default 6 ticks).
    swung_at: i32,
}

impl BreakDoorGoal {
    pub fn new(who: Who) -> BreakDoorGoal {
        BreakDoorGoal { who, door: Interact::new(), break_time: 0, last_progress: -1, swung_at: i32::MIN / 2 }
    }

    /// `validDifficulties.test(level.getDifficulty())`.
    fn valid_difficulty(&self, level: &dyn EntityLevel) -> bool {
        match self.who {
            Who::Zombie => level.difficulty() == 3,
            Who::Vindicator => matches!(level.difficulty(), 2 | 3),
        }
    }

    fn progress(&self, level: &mut dyn EntityLevel, e: &Entity, stage: i32) {
        level.emit(Event::BlockDestruction { entity: e.id, pos: self.door.door_pos, stage });
    }
}

impl CustomGoal for BreakDoorGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        match self.who {
            Who::Zombie => "BreakDoorGoal",
            Who::Vindicator => "VindicatorBreakDoorGoal",
        }
    }
    fn flags(&self) -> u8 {
        match self.who {
            Who::Zombie => 0,
            Who::Vindicator => MOVE,
        }
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.who == Who::Vindicator {
            // `hasActiveRaid() && random.nextInt(reducedTickDelay(10)) == 0 && super.canUse()`.
            if !raider::has_active_raid(m, level) || e.random.next_int_bounded(reduced_tick_delay(10)) != 0 {
                return false;
            }
        }
        if !self.door.can_use(e, m, level) || !level.mob_griefing() {
            return false;
        }
        self.valid_difficulty(level) && !self.door.is_open(level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.who == Who::Vindicator && !raider::has_active_raid(m, level) {
            return false;
        }
        let p = self.door.door_pos;
        let c = Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
        self.break_time <= BREAK_TIME && !self.door.is_open(level) && c.distance_to_sqr(e.position()) < 4.0 && self.valid_difficulty(level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.door.start(e);
        self.break_time = 0;
        if self.who == Who::Vindicator {
            m.no_action_time = 0;
        }
    }
    fn stop(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        self.progress(level, e, -1);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.door.tick(e);
        if e.random.next_int_bounded(20) == 0 {
            level.emit(Event::LevelEvent { event: 1019, pos: self.door.door_pos, data: 0 });
            if e.tick_count - self.swung_at >= 6 {
                self.swung_at = e.tick_count;
                m.swing = true;
            }
        }
        self.break_time += 1;
        let progress = (self.break_time as f32 / BREAK_TIME as f32 * 10.0) as i32;
        if progress != self.last_progress {
            self.progress(level, e, progress);
            self.last_progress = progress;
        }
        if self.break_time == BREAK_TIME && self.valid_difficulty(level) {
            let pos = self.door.door_pos;
            level.set_block(pos, 0, 3);
            level.emit(Event::LevelEvent { event: 1021, pos, data: 0 });
            // The state read after the removal (air): vanilla's particles are of nothing.
            let id = level.block(pos) as i32;
            level.emit(Event::LevelEvent { event: 2001, pos, data: id });
        }
    }
}

/// `AbstractIllager.RaiderOpenDoorGoal` (an `OpenDoorGoal` that does not close the door behind it, in a raid): the door opens as the goal starts and
/// shuts as it stops, which is the next time the goals run since it never continues.
#[derive(Clone, Debug)]
pub struct RaiderOpenDoorGoal {
    door: Interact,
}

impl RaiderOpenDoorGoal {
    pub fn new() -> RaiderOpenDoorGoal {
        RaiderOpenDoorGoal { door: Interact::new() }
    }
}

impl CustomGoal for RaiderOpenDoorGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RaiderOpenDoorGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.door.can_use(e, m, level) && raider::has_active_raid(m, level)
    }
    /// `closeDoor` is false: `canContinueToUse` never holds.
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.door.start(e);
        self.door.set_open(e, m, level, true);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.door.set_open(e, m, level, false);
    }
    fn tick(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.door.tick(e);
    }
}
