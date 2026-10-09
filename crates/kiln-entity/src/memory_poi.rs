//! Points of interest for [`crate::memory::MemoryLevel`] (the parity tests): read off the level's
//! blocks (`PoiTypes.forState`), with tickets that villagers take and give back.
//!
//! Only what the entity brains ask for: the job sites, homes (beds, by their head half) and meeting
//! points (bells). Records are listed in position order (vanilla lists them chunk by chunk, section
//! by section, in a hash map's order: tests use points of interest at different distances).

use crate::level::PoiOccupancy;
use crate::math::BlockPos;
use crate::memory::MemoryLevel;

/// The job site types (`#minecraft:acquirable_job_site`).
pub const JOB_SITES: [&str; 13] = [
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
    "minecraft:weaponsmith",
];

/// `PoiTypes.forState` for the types brains use.
pub fn type_of_state(state: u16) -> Option<&'static str> {
    let name = crate::blocks::block_name(state);
    let path = name.strip_prefix("minecraft:")?;
    Some(match path {
        "blast_furnace" => "minecraft:armorer",
        "smoker" => "minecraft:butcher",
        "cartography_table" => "minecraft:cartographer",
        "brewing_stand" => "minecraft:cleric",
        "composter" => "minecraft:farmer",
        "barrel" => "minecraft:fisherman",
        "fletching_table" => "minecraft:fletcher",
        "cauldron" | "lava_cauldron" | "water_cauldron" | "powder_snow_cauldron" => "minecraft:leatherworker",
        "lectern" => "minecraft:librarian",
        "stonecutter" => "minecraft:mason",
        "loom" => "minecraft:shepherd",
        "smithing_table" => "minecraft:toolsmith",
        "grindstone" => "minecraft:weaponsmith",
        "bell" => "minecraft:meeting",
        "beehive" => "minecraft:beehive",
        "bee_nest" => "minecraft:bee_nest",
        // The head half of each bed.
        p if p.ends_with("_bed") => {
            if kiln_data::blocks_types::block_of(state).property(state, "part") != Some("head") {
                return None;
            }
            "minecraft:home"
        }
        _ => return None,
    })
}

/// `PoiType.maxTickets`.
pub fn max_tickets(t: &str) -> i32 {
    match t {
        "minecraft:meeting" => 32,
        "minecraft:beehive" | "minecraft:bee_nest" => 0,
        _ => 1,
    }
}

fn wanted(types: &[&str], t: &str) -> bool {
    types.iter().any(|n| match *n {
        "#minecraft:acquirable_job_site" => JOB_SITES.contains(&t),
        "#minecraft:village" => JOB_SITES.contains(&t) || t == "minecraft:home" || t == "minecraft:meeting",
        "#minecraft:bee_home" => t == "minecraft:beehive" || t == "minecraft:bee_nest",
        n => n == t,
    })
}

impl MemoryLevel {
    /// The points of interest of the level's blocks, sorted by position.
    fn poi_records(&self) -> Vec<(BlockPos, &'static str)> {
        let mut v: Vec<(BlockPos, &'static str)> = self.blocks.iter().filter_map(|(p, s)| type_of_state(*s).map(|t| (*p, t))).collect();
        v.sort_by_key(|(p, _)| (p.x, p.y, p.z));
        v
    }

    fn poi_free(&self, pos: BlockPos, t: &str) -> i32 {
        max_tickets(t) - self.poi_taken.get(&pos).copied().unwrap_or(0)
    }

    pub(crate) fn poi_in_range_impl(&self, types: &[&str], center: BlockPos, radius: i32, occupancy: PoiOccupancy) -> Vec<BlockPos> {
        let r2 = radius as i64 * radius as i64;
        self.poi_records()
            .into_iter()
            .filter(|(p, t)| {
                let free = self.poi_free(*p, t);
                let occ_ok = match occupancy {
                    PoiOccupancy::HasSpace => free > 0,
                    PoiOccupancy::IsOccupied => free != max_tickets(t),
                    PoiOccupancy::Any => true,
                };
                let (dx, dy, dz) = ((p.x - center.x) as i64, (p.y - center.y) as i64, (p.z - center.z) as i64);
                wanted(types, t) && occ_ok && dx.abs() <= radius as i64 && dz.abs() <= radius as i64 && dx * dx + dy * dy + dz * dz <= r2
            })
            .map(|(p, _)| p)
            .collect()
    }

    pub(crate) fn poi_take_impl(&mut self, types: &[&str], center: BlockPos, radius: i32, accept: &dyn Fn(&str, BlockPos) -> bool) -> Option<BlockPos> {
        let found = self.poi_in_range_impl(types, center, radius, PoiOccupancy::HasSpace).into_iter().find(|p| {
            let t = self.blocks.get(p).and_then(|s| type_of_state(*s));
            t.is_some_and(|t| accept(t, *p))
        })?;
        *self.poi_taken.entry(found).or_insert(0) += 1;
        Some(found)
    }

    pub(crate) fn poi_release_impl(&mut self, pos: BlockPos) {
        if let Some(t) = self.poi_taken.get_mut(&pos)
            && *t > 0
        {
            *t -= 1;
        }
    }

    pub(crate) fn poi_type_impl(&self, pos: BlockPos) -> Option<&'static str> {
        type_of_state(self.blocks.get(&pos).copied().unwrap_or(0))
    }

    /// `PoiManager.sectionsToVillage`: sections (moving to any of the 26 neighbours) to the
    /// nearest section with an occupied village point of interest, 7 when none within 6.
    pub(crate) fn sections_to_village_impl(&self, pos: BlockPos) -> i32 {
        let (sx, sy, sz) = (pos.x >> 4, pos.y >> 4, pos.z >> 4);
        let mut best = 7;
        for (p, t) in self.poi_records() {
            if !(JOB_SITES.contains(&t) || t == "minecraft:home" || t == "minecraft:meeting") || self.poi_free(p, t) == max_tickets(t) {
                continue;
            }
            let d = ((p.x >> 4) - sx).abs().max(((p.y >> 4) - sy).abs()).max(((p.z >> 4) - sz).abs());
            best = best.min(d);
        }
        best.min(7)
    }
}
