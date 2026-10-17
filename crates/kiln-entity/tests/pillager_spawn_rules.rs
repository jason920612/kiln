//! wp72: where pillagers spawn naturally. They alone use `PatrollingMonster.checkPatrollingMonsterSpawnRules`: no block light above 8 at the
//! spot, nothing about the darkness a monster needs. Kiln asked them for `Monster.isDarkEnoughToSpawn` like zombies, so an outpost (torches
//! on its towers) made far fewer pillagers than on vanilla, and none by day (real servers side by side on seed 72: 18 pillagers in three
//! 14400-tick nights on vanilla, 4 in six on Kiln).

use kiln_entity::math::BlockPos;
use kiln_entity::mob::MobKind;
use kiln_entity::mob::ext::SpawnView;
use kiln_javamath::random::LegacyRandom;

/// Stone floor at y 9; the lights at the spot are set by the test.
struct Spot {
    sky: i32,
    block: i32,
}

fn state(name: &str) -> u16 {
    kiln_data::blocks_types::block_by_name(name).unwrap().default
}

impl SpawnView for Spot {
    fn block(&self, p: BlockPos) -> u16 {
        state(if p.y <= 9 { "minecraft:stone" } else { "minecraft:air" })
    }
    fn raw_brightness(&self, _: BlockPos, sky_darken: i32) -> i32 {
        (self.sky - sky_darken).max(self.block)
    }
    fn sky_darken(&self) -> i32 {
        0
    }
    fn sky_light(&self, _: BlockPos) -> i32 {
        self.sky
    }
    fn block_light(&self, _: BlockPos) -> i32 {
        self.block
    }
    fn biome(&self, _: BlockPos) -> i32 {
        0
    }
    fn difficulty(&self) -> u8 {
        2
    }
    fn world_seed(&self) -> i64 {
        0
    }
    fn moon_brightness(&self) -> f32 {
        1.0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn sea_level(&self) -> i32 {
        32
    }
}

fn can(kind: MobKind, sky: i32, block: i32) -> bool {
    let mut r = LegacyRandom::new(1);
    let spot = Spot { sky, block };
    let k = kind.ext().unwrap();
    k.check_spawn_rules(&spot, BlockPos::new(0, 10, 0), &mut r).expect("the kind has a spawn rule")
}

#[test]
fn a_pillager_spawns_in_daylight_but_not_next_to_a_torch() {
    assert!(can(MobKind::Pillager, 15, 0), "any sky light");
    assert!(can(MobKind::Pillager, 0, 0), "darkness");
    assert!(can(MobKind::Pillager, 0, 8), "block light 8 is the limit");
    assert!(!can(MobKind::Pillager, 0, 9), "a torch (14) beside the spot");
    assert!(!can(MobKind::Pillager, 15, 14));
}
