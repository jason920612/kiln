//! `MoveThroughVillageGoal`: at night a zombie close to a village walks to a point of interest of the village (job sites, homes, bells) it has not
//! been to lately, through a door if that is the way and it can break doors. (Kiln's zombies never started it: `Goal::Never` stood in.)

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, PoiOccupancy};
use crate::math::{BlockPos, Vec3};
use crate::mob::ext::CustomGoal;
use crate::mob::goals::MOVE;
use crate::memory_poi::JOB_SITES;
use crate::mob::path::{self, Path};
use crate::mob::{MobData, random_pos};

/// `PoiTypeTags.VILLAGE`: `#minecraft:acquirable_job_site`, `minecraft:home`, `minecraft:meeting`.
fn village_types() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = JOB_SITES.to_vec();
    v.push("minecraft:home");
    v.push("minecraft:meeting");
    v
}

fn closer_to_center(p: BlockPos, v: Vec3, dist: f64) -> bool {
    let (dx, dy, dz) = (p.x as f64 + 0.5 - v.x, p.y as f64 + 0.5 - v.y, p.z as f64 + 0.5 - v.z);
    dx * dx + dy * dy + dz * dz < dist * dist
}

fn dist_sqr(a: BlockPos, b: BlockPos) -> f64 {
    let (dx, dy, dz) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
    dx * dx + dy * dy + dz * dz
}

#[derive(Clone, Debug)]
pub struct MoveThroughVillageGoal {
    speed: f64,
    only_at_night: bool,
    distance_to_poi: i32,
    /// `canDealWithDoors`: the mob breaks doors.
    breaks_doors: fn(&MobData) -> bool,
    path: Option<Path>,
    poi: BlockPos,
    visited: Vec<BlockPos>,
}

impl MoveThroughVillageGoal {
    pub fn new(speed: f64, only_at_night: bool, distance_to_poi: i32, breaks_doors: fn(&MobData) -> bool) -> MoveThroughVillageGoal {
        MoveThroughVillageGoal { speed, only_at_night, distance_to_poi, breaks_doors, path: None, poi: BlockPos::default(), visited: Vec::new() }
    }

    /// `PoiManager.find(village, hasNotVisited, center, 10, IS_OCCUPIED)`: the first in storage order.
    fn find(&self, level: &dyn EntityLevel, center: BlockPos) -> Option<BlockPos> {
        level.poi_in_range(&village_types(), center, 10, PoiOccupancy::IsOccupied).into_iter().find(|p| !self.visited.contains(p))
    }
}

impl CustomGoal for MoveThroughVillageGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "MoveThroughVillageGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.nav.water_bound {
            return false;
        }
        // `updateVisited`.
        if self.visited.len() > 15 {
            self.visited.remove(0);
        }
        if self.only_at_night && level.is_bright_outside() {
            return false;
        }
        let mob_pos = e.block_position();
        // `isCloseToVillage(pos, 6)`.
        if level.sections_to_village(mob_pos) > 6 {
            return false;
        }
        let target = {
            let lv: &dyn EntityLevel = &*level;
            let this: &MoveThroughVillageGoal = self;
            random_pos::land_pos_weighted(e, m, lv, 15, 7, &|p| {
                if !lv.is_village(p) {
                    return f64::NEG_INFINITY;
                }
                this.find(lv, p).map_or(f64::NEG_INFINITY, |found| -dist_sqr(found, mob_pos))
            })
        };
        let Some(target) = target else { return false };
        let at = BlockPos::containing(target.x, target.y, target.z);
        let Some(poi) = self.find(&*level, at) else { return false };
        self.poi = poi;
        let doors = (self.breaks_doors)(m);
        m.nav.can_open_doors = doors;
        self.path = path::create_path(e, m, &*level, poi, 0);
        m.nav.can_open_doors = true;
        if self.path.is_none() {
            let to = Vec3::new(poi.x as f64 + 0.5, poi.y as f64, poi.z as f64 + 0.5);
            let Some(p) = random_pos::default_pos_towards(e, m, &*level, 10, 7, to, 1.5707963705062866) else { return false };
            m.nav.can_open_doors = doors;
            self.path = path::create_path(e, m, &*level, BlockPos::containing(p.x, p.y, p.z), 0);
            m.nav.can_open_doors = true;
            if self.path.is_none() {
                return false;
            }
        }
        // A closed wooden door on the way: the path to the door itself.
        if let Some(path) = &self.path {
            for n in &path.nodes {
                let door = BlockPos::new(n.x, n.y + 1, n.z);
                let name = crate::blocks::block_name(level.block(door));
                if name.ends_with("_door") && name != "minecraft:iron_door" {
                    self.path = path::create_path(e, m, &*level, BlockPos::new(n.x, n.y, n.z), 0);
                    break;
                }
            }
        }
        self.path.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if m.nav_ref().is_done() {
            return false;
        }
        !closer_to_center(self.poi, e.position(), (e.width + self.distance_to_poi as f32) as f64)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to_path(e, m, &*level, self.path.clone(), self.speed);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if m.nav_ref().is_done() || closer_to_center(self.poi, e.position(), self.distance_to_poi as f64) {
            self.visited.push(self.poi);
        }
    }
}
