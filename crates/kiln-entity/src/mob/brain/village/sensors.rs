//! The villagers' own sensors: `NearestBedSensor`, `VillagerHostilesSensor`,
//! `VillagerBabiesSensor`, `SecondaryPoiSensor`, `GolemSensor`, `NearestItemSensor`.

use kiln_javamath::random::RandomSource;
use crate::level::{EntityFilter, PoiOccupancy};
use crate::math::{Aabb, BlockPos};
use crate::mob::brain::memory::Val;
use crate::mob::brain::util;
use crate::mob::brain::{Cx, Mem, Sensor};
use crate::mob::goals::Living;
use crate::mob::kinds::villager;
use crate::mob::path;
use crate::sensor_boilerplate;

/// `NearestBedSensor`: for babies, the closest bed they can walk to (they jump on it).
#[derive(Clone, Debug, Default)]
pub struct NearestBed;

impl Sensor for NearestBed {
    fn name(&self) -> &'static str {
        "NearestBedSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestBed]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        if !cx.m.baby() {
            return;
        }
        // `lastUpdate = gameTime + level.getRandom().nextInt(20)` (only ever read to expire a cache
        // nothing fills).
        let _ = cx.rng().next_int_bounded(20);
        let center = cx.e.block_position();
        // The position filter lets the first four through (`++triedCount < 5`).
        let found: Vec<BlockPos> = cx.level.poi_in_range(&["minecraft:home"], center, 48, PoiOccupancy::Any).into_iter().take(4).collect();
        if let Some(p) = find_path_to_pois(cx, &found)
            && p.reached
            && cx.level.poi_type(p.target).is_some()
        {
            cx.b.mem.set(Mem::NearestBed, Val::Block(p.target));
        }
    }
    sensor_boilerplate!();
}

/// `AcquirePoi.findPathToPois`: a path to the best of several points of interest (reaching any
/// within the largest valid range), `None` without any.
pub fn find_path_to_pois(cx: &mut Cx, pois: &[BlockPos]) -> Option<path::Path> {
    if pois.is_empty() {
        return None;
    }
    let mut range = 1;
    for p in pois {
        if let Some(t) = cx.level.poi_type(*p) {
            range = range.max(valid_range(t));
        }
    }
    path::create_path_multi(cx.e, cx.m, &*cx.level, pois, range)
}

/// `PoiType.validRange`.
pub fn valid_range(poi: &str) -> i32 {
    if poi == "minecraft:meeting" { 6 } else { 1 }
}

/// `VillagerHostilesSensor`: the closest visible hostile within its own distance.
#[derive(Clone, Debug, Default)]
pub struct VillagerHostiles;

impl Sensor for VillagerHostiles {
    fn name(&self) -> &'static str {
        "VillagerHostilesSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestHostile, Mem::NearestVisibleLivingEntities]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        if !cx.b.mem.has(Mem::NearestVisibleLivingEntities) {
            cx.b.mem.erase(Mem::NearestHostile);
            return;
        }
        let found = util::find_closest_visible(cx, |cx, id| {
            let Some(l) = util::living(cx, id) else { return false };
            hostile_distance(l.type_name).is_some_and(|r| cx.e.position().distance_to_sqr(l.pos) <= (r * r) as f64)
        });
        cx.b.mem.set_opt(Mem::NearestHostile, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

/// `VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES`.
pub fn hostile_distance(type_name: &str) -> Option<f32> {
    Some(match type_name {
        "minecraft:drowned" | "minecraft:husk" | "minecraft:vex" | "minecraft:zombie" | "minecraft:zombie_villager" => 8.0,
        "minecraft:evoker" | "minecraft:illusioner" | "minecraft:ravager" => 12.0,
        "minecraft:pillager" => 15.0,
        "minecraft:vindicator" | "minecraft:zoglin" => 10.0,
        _ => return None,
    })
}

/// `VillagerBabiesSensor`: the visible baby villagers.
#[derive(Clone, Debug, Default)]
pub struct VillagerBabies;

impl Sensor for VillagerBabies {
    fn name(&self) -> &'static str {
        "VillagerBabiesSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::VisibleVillagerBabies]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let babies = util::find_all_visible(cx, |cx, id| {
            cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:villager" && crate::mob::data(o).is_some_and(|d| d.baby()))
        });
        cx.b.mem.set(Mem::VisibleVillagerBabies, Val::Entities(babies));
    }
    sensor_boilerplate!();
}

/// `SecondaryPoiSensor`: the blocks around that the profession works with besides its job site
/// (a farmer's farmland).
#[derive(Clone, Debug, Default)]
pub struct SecondaryPois;

impl Sensor for SecondaryPois {
    fn name(&self) -> &'static str {
        "SecondaryPoiSensor"
    }
    fn scan_rate(&self) -> i32 {
        40
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::SecondaryJobSite]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let Some(st) = villager::state(cx.m) else { return };
        let secondary = villager::secondary_poi_block(st.profession);
        let mut found = Vec::new();
        if let Some(block) = secondary {
            let center = cx.e.block_position();
            for dx in -4..=4 {
                for dy in -2..=2 {
                    for dz in -4..=4 {
                        let p = center.offset(dx, dy, dz);
                        if crate::blocks::block_name(cx.level.block(p)) == block {
                            found.push(crate::mob::brain::GlobalPos::new(super::DIM, p));
                        }
                    }
                }
            }
        }
        if found.is_empty() {
            cx.b.mem.erase(Mem::SecondaryJobSite);
        } else {
            cx.b.mem.set(Mem::SecondaryJobSite, Val::Positions(found));
        }
    }
    sensor_boilerplate!();
}

/// `GolemSensor`: an iron golem among the nearest living entities was detected recently.
#[derive(Clone, Debug, Default)]
pub struct GolemDetected;

impl Sensor for GolemDetected {
    fn name(&self) -> &'static str {
        "GolemSensor"
    }
    fn scan_rate(&self) -> i32 {
        200
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestLivingEntities, Mem::GolemDetectedRecently]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        if !cx.b.mem.has(Mem::NearestLivingEntities) {
            return;
        }
        let golem = cx.b.mem.entities(Mem::NearestLivingEntities).iter().any(|&id| cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:iron_golem"));
        if golem {
            golem_detected(&mut cx.b.mem);
        }
    }
    sensor_boilerplate!();
}

/// `GolemSensor.golemDetected`: 599 ticks.
pub fn golem_detected(mem: &mut crate::mob::brain::Memories) {
    mem.set_expiring(Mem::GolemDetectedRecently, Val::Bool(true), 599);
}

/// `NearestItemSensor`: the closest item within 32 blocks the mob wants and can see.
#[derive(Clone, Debug, Default)]
pub struct NearestItems;

impl Sensor for NearestItems {
    fn name(&self) -> &'static str {
        "NearestItemSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestVisibleWantedItem]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let area: Aabb = cx.e.bounding_box().inflate(32.0, 16.0, 32.0);
        let mut items: Vec<(f64, i32)> = Vec::new();
        for id in cx.level.entities_in(&area, EntityFilter::Item, cx.e.id) {
            if let Some(o) = cx.level.entity(id) {
                items.push((cx.e.position().distance_to_sqr(o.position()), id));
            }
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut found = None;
        for (d, id) in items {
            let Some(o) = cx.level.entity(id) else { continue };
            let crate::entity::EntityKind::Item(data) = &o.kind else { continue };
            if !villager::wants_to_pick_up(cx.m, &data.stack) || d >= 32.0 * 32.0 {
                continue;
            }
            let target = Living {
                id,
                type_name: o.type_name,
                pos: o.position(),
                eye_y: o.eye_y(),
                alive: true,
                player: false,
                creative: false,
                spectator: false,
                invulnerable: false,
                sneaking: false,
                invisible: false,
                armor_cover: 0.0,
                bb: o.bounding_box(),
            };
            if crate::mob::has_line_of_sight_cached(cx.e, cx.m, &*cx.level, &target) {
                found = Some(id);
                break;
            }
        }
        cx.b.mem.set_opt(Mem::NearestVisibleWantedItem, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}
