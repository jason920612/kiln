//! wp71: a strolling villager's targets (`LandRandomPos.getPos(mob, 10, 7)`, what `VillageBoundRandomStroll` asks for) depend on the ground *below* the pen.
//! A candidate block is taken when the block under it is solid (`PathNavigation.isStableDestination`) and then lifted out of any solid block it is in, so
//! with solid ground below the floor most candidates under the floor are good (lifted to the floor's level) and the search finds something in all but
//! about 2 of 100 asks; with nothing below the floor only the candidates at the floor's level and in the walls are, and about 28 of 100 asks find nothing.
//! Measured on the vanilla 26.3 server (`tools/wp71`, the agent counts the returns of `LandRandomPos.getPos`): 560 of 27,635 asks (2.0 %) in the flat world's
//! farm, 31,990 of 111,981 (28.6 %) with the three layers below the pen made air. The wp66 and wp69 farm comparisons put vanilla on a flat world and Kiln in
//! a void world and so compared these two numbers, not the servers: the golem counts follow them (vanilla 1018 / 871, Kiln 1016 / 866 per 24 pens and 6 days).

use kiln_data::blocks::default_state::{STONE, STONE_BRICKS};
use kiln_entity::level::EntityLevel;
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::{self, MobKind};
use kiln_javamath::random::LegacyRandom;

const FLOOR: i32 = 64;

/// The wp66 pen: a stone floor (and `deep` more layers of stone under it), a 9 x 9 room with walls of stone bricks three blocks high.
fn pen(deep: i32) -> MemoryLevel {
    let mut level = MemoryLevel::new(-64, 7);
    for x in -12..=24 {
        for z in -12..=20 {
            for y in FLOOR - deep..=FLOOR {
                level.blocks.insert(BlockPos::new(x, y, z), STONE);
            }
        }
    }
    for x in -1..=12 {
        for z in -1..=9 {
            for y in FLOOR + 1..=FLOOR + 3 {
                let wall = !(0..=8).contains(&x) || !(0..=8).contains(&z) || y == FLOOR + 3;
                if wall {
                    level.blocks.insert(BlockPos::new(x, y, z), STONE_BRICKS);
                }
            }
        }
    }
    level
}

/// How many of `n` asks for a stroll target find none, for a villager standing in the pen.
fn none_of(level: &MemoryLevel, n: i64) -> i64 {
    let mut e = mob::new(MobKind::Villager, 10, 0, 1);
    e.set_pos(Vec3::new(6.5, (FLOOR + 1) as f64, 3.5));
    let m = mob::data(&e).unwrap().clone();
    let level: &dyn EntityLevel = level;
    (0..n)
        .filter(|&seed| {
            e.random = LegacyRandom::new(seed * 7919 + 3);
            mob::random_pos::land_pos(&mut e, &m, level, 10, 7).is_none()
        })
        .count() as i64
}

#[test]
fn with_solid_ground_below_the_pen_a_stroll_nearly_always_finds_a_target() {
    let none = none_of(&pen(3), 4000);
    eprintln!("solid ground below: {none} of 4000 asks found nothing");
    assert!(none < 4000 * 6 / 100, "{none} of 4000 asks found nothing (vanilla: 2 in 100)");
}

#[test]
fn with_nothing_below_the_pen_a_stroll_often_finds_none() {
    let none = none_of(&pen(0), 4000);
    eprintln!("nothing below: {none} of 4000 asks found nothing");
    assert!((4000 * 15 / 100..4000 * 45 / 100).contains(&none), "{none} of 4000 asks found nothing (vanilla: 29 in 100)");
}
