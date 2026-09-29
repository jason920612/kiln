//! A villager's points of interest: its bed (`minecraft:home`), job site and meeting point
//! (bell), claimed in the level's POI manager and kept as brain memories (`AcquirePoi`,
//! `ValidateNearbyPoi`, `AssignProfessionFromJobSite`, `ResetProfession` and
//! `Villager.releaseAllPois` as goal-free stand-ins for the brain behaviours).
//!
//! Approximations: villagers claim the first free point of interest within 48 blocks every
//! second (vanilla walks there first and checks a path), an unemployed villager takes the job
//! site's profession as soon as it claims it, and a first-level villager without trades done
//! loses its profession with its job site.

use super::villager::VillagerState;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::BlockPos;
use crate::mob::MobData;
use crate::persist::{Input, Output};
use kiln_proto::nbt::Tag;

/// The claimed points of interest (brain memories `home`, `job_site`, `meeting_point`), and
/// the rest of the saved brain as it was.
#[derive(Clone, Debug, Default)]
pub struct VillagerPois {
    pub home: Option<(String, BlockPos)>,
    pub job_site: Option<(String, BlockPos)>,
    pub meeting_point: Option<(String, BlockPos)>,
    pub brain: Option<Tag>,
}

const MEMORIES: [&str; 3] = ["minecraft:home", "minecraft:job_site", "minecraft:meeting_point"];

fn slot(p: &mut VillagerPois, i: usize) -> &mut Option<(String, BlockPos)> {
    match i {
        0 => &mut p.home,
        1 => &mut p.job_site,
        _ => &mut p.meeting_point,
    }
}

/// The overworld: villagers of other levels are rare; the saved dimension is kept.
const DEFAULT_DIMENSION: &str = "minecraft:overworld";

/// Claims and checks, once a second for each villager.
pub fn tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if m.no_ai || m.baby() || (level.game_time() + e.id as i64) % 20 != 0 {
        return;
    }
    let Some(st) = super::villager::state_mut(m) else { return };
    let pos = e.block_position();
    // `ValidateNearbyPoi`: a point of interest whose block is gone is forgotten.
    for i in 0..3 {
        let expected: &[&str] = match i {
            0 => &["minecraft:home"],
            1 => &[],
            _ => &["minecraft:meeting"],
        };
        let Some((_, p)) = slot(&mut st.pois, i).clone() else { continue };
        let ok = match level.poi_type(p) {
            Some(t) if i == 1 => kiln_world_job(t),
            Some(t) => expected.contains(&t),
            None => false,
        };
        if !ok {
            *slot(&mut st.pois, i) = None;
            if i == 1 && st.level == 1 && st.xp == 0 && st.profession != "minecraft:nitwit" {
                // `ResetProfession`.
                st.set_profession("minecraft:none");
            }
        }
    }
    // `AcquirePoi` for the bed, the job site of an unemployed villager, the meeting point.
    if st.pois.home.is_none()
        && let Some(p) = level.poi_take(&["minecraft:home"], pos, 48, &|_, _| true)
    {
        st.pois.home = Some((DEFAULT_DIMENSION.into(), p));
    }
    let unemployed = st.profession == "minecraft:none";
    if st.pois.job_site.is_none() && unemployed {
        let found = level.poi_take(&["#minecraft:acquirable_job_site"], pos, 48, &|_, _| true);
        if let Some(p) = found {
            st.pois.job_site = Some((DEFAULT_DIMENSION.into(), p));
            // `AssignProfessionFromJobSite`: the job site's profession (same names).
            if let Some(t) = level.poi_type(p)
                && let Some(prof) = kiln_data::builtin_entries("minecraft:villager_profession").and_then(|e| e.iter().find(|x| **x == t).copied())
            {
                st.set_profession(prof);
            }
        }
    }
    if st.pois.meeting_point.is_none()
        && let Some(p) = level.poi_take(&["minecraft:meeting"], pos, 48, &|_, _| true)
    {
        st.pois.meeting_point = Some((DEFAULT_DIMENSION.into(), p));
    }
}

fn kiln_world_job(t: &str) -> bool {
    matches!(
        t,
        "minecraft:armorer"
            | "minecraft:butcher"
            | "minecraft:cartographer"
            | "minecraft:cleric"
            | "minecraft:farmer"
            | "minecraft:fisherman"
            | "minecraft:fletcher"
            | "minecraft:leatherworker"
            | "minecraft:librarian"
            | "minecraft:mason"
            | "minecraft:shepherd"
            | "minecraft:toolsmith"
            | "minecraft:weaponsmith"
    )
}

/// `Villager.releaseAllPois` (on death).
pub fn release_all(level: &mut dyn EntityLevel, st: &mut VillagerState) {
    for i in 0..3 {
        if let Some((_, p)) = slot(&mut st.pois, i).take() {
            level.poi_release(p);
        }
    }
}

fn global_pos(t: &Tag) -> Option<(String, BlockPos)> {
    let v = t.get("value")?;
    let dim = v.get("dimension").and_then(Tag::as_str).unwrap_or(DEFAULT_DIMENSION).to_owned();
    match v.get("pos") {
        Some(Tag::IntArray(p)) if p.len() == 3 => Some((dim, BlockPos::new(p[0], p[1], p[2]))),
        _ => None,
    }
}

/// Reads the brain's point of interest memories.
pub fn load(st: &mut VillagerState, r: &mut Input) {
    let Some(brain) = r.get("Brain") else { return };
    st.pois.brain = Some(brain.clone());
    if let Some(mem) = brain.get("memories") {
        for (i, key) in MEMORIES.iter().enumerate() {
            *slot(&mut st.pois, i) = mem.get(key).and_then(global_pos);
        }
    }
}

/// Writes the brain with the point of interest memories (other memories as they were read).
pub fn save(st: &VillagerState, o: &mut Output) {
    let mut memories: Vec<(String, Tag)> = match st.pois.brain.as_ref().and_then(|b| b.get("memories")) {
        Some(Tag::Compound(m)) => m.iter().filter(|(k, _)| !MEMORIES.contains(&k.as_str())).cloned().collect(),
        _ => Vec::new(),
    };
    let mut p = st.pois.clone();
    for (i, key) in MEMORIES.iter().enumerate() {
        if let Some((dim, pos)) = slot(&mut p, i).clone() {
            let value = Tag::Compound(vec![("dimension".into(), Tag::String(dim)), ("pos".into(), Tag::IntArray(vec![pos.x, pos.y, pos.z]))]);
            memories.push(((*key).to_owned(), Tag::Compound(vec![("value".into(), value)])));
        }
    }
    o.put("Brain", Tag::Compound(vec![("memories".into(), Tag::Compound(memories))]));
}
