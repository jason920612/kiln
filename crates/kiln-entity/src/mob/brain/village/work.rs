//! What villagers do at work: `WorkAtPoi`, `WorkAtComposter`, `HarvestFarmland`, `UseBonemeal`.

use kiln_javamath::random::RandomSource;
use super::closer_to_center_than;
use crate::behavior_boilerplate;
use crate::math::BlockPos;
use crate::mob::brain::memory::{Tracker, Val, WalkTarget};
use crate::mob::brain::{Behavior, Control, Cx, Mem, Status, Timed};
use crate::mob::kinds::villager;
use crate::mob::MAINHAND;
use Status::{Registered, ValueAbsent, ValuePresent};

/// `WorkAtPoi` and its farmer subclass `WorkAtComposter`.
#[derive(Clone, Debug)]
pub struct WorkAtPoi {
    composter: bool,
    last_check: i64,
}

impl WorkAtPoi {
    pub fn new(composter: bool) -> Box<dyn Control> {
        Timed::new(WorkAtPoi { composter, last_check: 0 })
    }
}

impl Behavior for WorkAtPoi {
    fn name(&self) -> &'static str {
        if self.composter { "WorkAtComposter" } else { "WorkAtPoi" }
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::JobSite, ValuePresent), (Mem::LookTarget, Registered)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if cx.time - self.last_check < 300 {
            return false;
        }
        if cx.rng().next_int_bounded(2) != 0 {
            return false;
        }
        self.last_check = cx.time;
        match super::mem_pos(cx, Mem::JobSite) {
            Some(p) => closer_to_center_than(p, cx.e.position(), 1.73),
            None => false,
        }
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.set(Mem::LastWorkedAtPoi, Val::Long(cx.time));
        if let Some(p) = super::mem_pos(cx, Mem::JobSite) {
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(p)));
        }
        villager::play_work_sound(cx.e, cx.m, cx.level);
        if self.composter {
            villager::use_composter(cx);
        }
        if villager::should_restock(cx.e, cx.m, cx.level) {
            villager::restock(cx.e, cx.m, cx.level);
        }
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        match super::mem_pos(cx, Mem::JobSite) {
            Some(p) => closer_to_center_than(p, cx.e.position(), 1.73),
            None => false,
        }
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- farming

/// `CropBlock`: the state's block is one and `isMaxAge`.
fn crop_state(state: u16) -> Option<bool> {
    let info = kiln_data::blocks_types::block_of(state);
    if !matches!(info.name, "minecraft:wheat" | "minecraft:carrots" | "minecraft:potatoes" | "minecraft:beetroots") {
        return None;
    }
    let age: i32 = info.property(state, "age")?.parse().ok()?;
    let max = info.properties.iter().find(|p| p.name == "age")?.values.len() as i32 - 1;
    Some(age >= max)
}

fn is_farmland(state: u16) -> bool {
    kiln_data::blocks_types::block_of(state).name == "minecraft:farmland"
}

/// `HarvestFarmland`: a farmer harvests ripe crops around it and plants seeds on empty farmland.
#[derive(Clone, Debug, Default)]
pub struct HarvestFarmland {
    above_farmland: Option<BlockPos>,
    next_ok_start: i64,
    time_worked: i32,
    valid: Vec<BlockPos>,
}

impl HarvestFarmland {
    pub fn new() -> Box<dyn Control> {
        Timed::new(HarvestFarmland::default())
    }

    fn valid_pos(cx: &Cx, p: BlockPos) -> bool {
        let s = cx.level.block(p);
        let below = cx.level.block(p.below());
        crop_state(s) == Some(true) || (kiln_data::blocks_types::is_air(s) && is_farmland(below))
    }

    fn pick(&self, cx: &mut Cx) -> Option<BlockPos> {
        if self.valid.is_empty() {
            None
        } else {
            let i = cx.rng().next_int_bounded(self.valid.len() as i32) as usize;
            Some(self.valid[i])
        }
    }
}

impl Behavior for HarvestFarmland {
    fn name(&self) -> &'static str {
        "HarvestFarmland"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::SecondaryJobSite, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (60, 60)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if !cx.level.mob_griefing() {
            return false;
        }
        if villager::state(cx.m).is_none_or(|s| s.profession != "minecraft:farmer") {
            return false;
        }
        self.valid.clear();
        let (x, y, z) = (cx.e.x(), cx.e.y(), cx.e.z());
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let p = BlockPos::containing(x + dx as f64, y + dy as f64, z + dz as f64);
                    if Self::valid_pos(cx, p) {
                        self.valid.push(p);
                    }
                }
            }
        }
        self.above_farmland = self.pick(cx);
        self.above_farmland.is_some()
    }
    fn start(&mut self, cx: &mut Cx) {
        if cx.time > self.next_ok_start
            && let Some(p) = self.above_farmland
        {
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(p)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(p, 0.5, 1)));
        }
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        self.time_worked < 200
    }
    fn tick(&mut self, cx: &mut Cx) {
        if let Some(p) = self.above_farmland
            && !closer_to_center_than(p, cx.e.position(), 1.0)
        {
            return;
        }
        if let Some(p) = self.above_farmland
            && cx.time > self.next_ok_start
        {
            let s = cx.level.block(p);
            let below = cx.level.block(p.below());
            let crop = crop_state(s);
            if crop == Some(true) {
                cx.level.destroy_block(p, true);
            }
            // (The state is the one read before the harvest: a crop just broken is not planted
            // again this tick.)
            if kiln_data::blocks_types::is_air(s) && is_farmland(below) && villager::has_farm_seeds(cx.m) {
                villager::plant_seed(cx, p);
            }
            if crop.is_some() && crop != Some(true) {
                self.valid.retain(|q| *q != p);
                self.above_farmland = self.pick(cx);
                if let Some(np) = self.above_farmland {
                    self.next_ok_start = cx.time + 20;
                    cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(np, 0.5, 1)));
                    cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(np)));
                }
            }
        }
        self.time_worked += 1;
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::LookTarget);
        cx.b.mem.erase(Mem::WalkTarget);
        self.time_worked = 0;
        self.next_ok_start = cx.time + 40;
    }
    behavior_boilerplate!();
}

/// `UseBonemeal`: a farmer with bone meal grows the crops around it.
#[derive(Clone, Debug, Default)]
pub struct UseBonemeal {
    next_work_cycle: i64,
    last_session: i64,
    time_worked: i32,
    crop: Option<BlockPos>,
}

impl UseBonemeal {
    pub fn new() -> Box<dyn Control> {
        Timed::new(UseBonemeal::default())
    }

    fn valid_pos(cx: &Cx, p: BlockPos) -> bool {
        crop_state(cx.level.block(p)) == Some(false)
    }

    fn pick_next(cx: &mut Cx) -> Option<BlockPos> {
        let base = cx.e.block_position();
        let mut found = None;
        let mut n = 0;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let p = base.offset(dx, dy, dz);
                    if Self::valid_pos(cx, p) {
                        n += 1;
                        if cx.rng().next_int_bounded(n) == 0 {
                            found = Some(p);
                        }
                    }
                }
            }
        }
        found
    }

    fn set_target(cx: &mut Cx, p: Option<BlockPos>) {
        if let Some(p) = p {
            let t = Tracker::block(p);
            cx.b.mem.set(Mem::LookTarget, Val::Look(t));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed: 0.5, close_enough: 1 }));
        }
    }
}

impl Behavior for UseBonemeal {
    fn name(&self) -> &'static str {
        "UseBonemeal"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if cx.e.tick_count % 10 != 0 || (self.last_session != 0 && self.last_session + 160 > cx.e.tick_count as i64) {
            return false;
        }
        if villager::count_item(cx.m, "minecraft:bone_meal") <= 0 {
            return false;
        }
        self.crop = Self::pick_next(cx);
        self.crop.is_some()
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        self.time_worked < 80 && self.crop.is_some()
    }
    fn start(&mut self, cx: &mut Cx) {
        Self::set_target(cx, self.crop);
        cx.m.equipment[MAINHAND] = kiln_item::ItemStack::of("minecraft:bone_meal", 1).unwrap_or_else(kiln_item::ItemStack::empty);
        self.next_work_cycle = cx.time;
        self.time_worked = 0;
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(p) = self.crop else { return };
        if cx.time < self.next_work_cycle || !closer_to_center_than(p, cx.e.position(), 1.0) {
            return;
        }
        if villager::count_item(cx.m, "minecraft:bone_meal") > 0 && villager::grow_crop(cx, p) {
            cx.level.emit(crate::level::Event::LevelEvent { event: 1505, pos: p, data: 15 });
            self.crop = Self::pick_next(cx);
            Self::set_target(cx, self.crop);
            self.next_work_cycle = cx.time + 40;
        }
        self.time_worked += 1;
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.m.equipment[MAINHAND] = kiln_item::ItemStack::empty();
        self.last_session = cx.e.tick_count as i64;
    }
    behavior_boilerplate!();
}

/// `GoToWantedItem.create(speed, false, maxDistance)`: walks to the item the sensor found.
pub fn go_to_wanted_item(speed: f32, max_distance: i32) -> Box<dyn Control> {
    crate::mob::brain::shot(
        "GoToWantedItem",
        &[
            (Mem::LookTarget, Registered),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::NearestVisibleWantedItem, ValuePresent),
            (Mem::ItemPickupCooldownTicks, Registered),
        ],
        move |cx| {
            let Some(id) = cx.b.mem.entity(Mem::NearestVisibleWantedItem) else { return false };
            let Some(o) = cx.level.entity(id) else { return false };
            let d = o.position().distance_to_sqr(cx.e.position());
            if !cx.b.mem.has(Mem::ItemPickupCooldownTicks) && d < (max_distance * max_distance) as f64 && cx.m.can_pick_up_loot {
                let walk = WalkTarget { target: Tracker::entity3(id, false, false), speed, close_enough: 0 };
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(walk));
                return true;
            }
            false
        },
    )
}
