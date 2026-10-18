//! wp77: `MoveThroughVillageGoal` (zombies) and `FoxStrollThroughVillageGoal`, which Kiln never started: at night a mob within six sections of a village
//! (an occupied point of interest) that sees nothing walks towards it. A bell with a taken ticket is the village; the mob starts 23 blocks away (a spot within 15 of it must have the point of interest within 10).

use kiln_entity::level::EntityLevel;
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::attributes::Attr;
use kiln_entity::mob::{self, MobKind};

fn world(village: bool) -> MemoryLevel {
    let mut level = MemoryLevel::new(-64, 42);
    level.difficulty = 2;
    level.sky_darken = 11;
    level.day_time = 18000;
    level.immediate_adds = true;
    for x in -80..=80 {
        for z in -80..=80 {
            level.blocks.insert(BlockPos::new(x, 99, z), kiln_data::blocks::default_state::STONE);
        }
    }
    if village {
        level.blocks.insert(BlockPos::new(0, 100, 0), kiln_data::blocks_types::block_by_name("minecraft:bell").unwrap().default);
        // A villager took a ticket of the bell's meeting point: the village is occupied.
        assert!(level.poi_take(&["minecraft:meeting"], BlockPos::new(0, 100, 0), 4, &|_, _| true).is_some());
    }
    level
}

/// The closest a mob that starts 23 blocks north of the bell and cannot see anything (a follow range of 2) comes to it in `ticks` ticks.
fn closest(kind: MobKind, village: bool, ticks: usize) -> f64 {
    let mut level = world(village);
    let mut e = mob::new(kind, 10, 0, 7);
    e.set_pos(Vec3::new(0.5, 100.0, -22.5));
    let m = mob::data_mut(&mut e).unwrap();
    m.attrs.get_mut(Attr::FollowRange).unwrap().base = 2.0;
    level.insert(e);
    level.set_next_entity_id(11);
    let mut best = f64::MAX;
    for _ in 0..ticks {
        level.tick();
        let p = level.entity(10).unwrap().position();
        best = best.min(((p.x - 0.5).powi(2) + (p.z - 0.5).powi(2)).sqrt());
    }
    best
}

#[test]
fn a_zombie_at_night_walks_to_a_village() {
    let near = closest(MobKind::Zombie, true, 1500);
    let far = closest(MobKind::Zombie, false, 1500);
    assert!(near < 10.0, "it reached the bell: {near:.1}");
    assert!(far > 14.0, "without the village it only strolls: {far:.1}");
}

#[test]
fn a_fox_at_night_strolls_towards_a_village() {
    let near = closest(MobKind::Fox, true, 6000);
    let far = closest(MobKind::Fox, false, 6000);
    assert!(near < far - 8.0, "closer with a village: {near:.1} against {far:.1}");
}
