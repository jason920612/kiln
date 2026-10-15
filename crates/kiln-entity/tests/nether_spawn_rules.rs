//! wp65: the natural-spawn rules of the nether's mobs that a real nether run found missing
//! (hoglins never spawned, striders never spawned): the walk target `PathfinderMob.checkSpawnRules` tests
//! and `Strider.checkStriderSpawnRules` / `IN_LAVA`, checked on a hand-made column of blocks.

use kiln_entity::math::BlockPos;
use kiln_entity::mob::MobKind;
use kiln_entity::mob::ext::{Placement, SpawnView};
use kiln_javamath::random::LegacyRandom;
use std::collections::HashMap;

struct Blocks(HashMap<(i32, i32, i32), u16>);

fn state(name: &str) -> u16 {
    kiln_data::blocks_types::block_by_name(name).unwrap().default
}

impl Blocks {
    fn new() -> Blocks {
        Blocks(HashMap::new())
    }
    fn set(&mut self, x: i32, y: i32, z: i32, name: &str) {
        self.0.insert((x, y, z), state(name));
    }
}

impl SpawnView for Blocks {
    fn block(&self, p: BlockPos) -> u16 {
        *self.0.get(&(p.x, p.y, p.z)).unwrap_or(&state("minecraft:air"))
    }
    fn raw_brightness(&self, _: BlockPos, _: i32) -> i32 {
        0
    }
    fn sky_darken(&self) -> i32 {
        0
    }
    fn sky_light(&self, _: BlockPos) -> i32 {
        0
    }
    fn block_light(&self, _: BlockPos) -> i32 {
        0
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

#[test]
fn a_hoglin_walk_target_is_ten_on_crimson_nylium_and_zero_elsewhere() {
    // `Hoglin.getWalkTargetValue` (not `Animal`'s light-based one: a dark nether floor would be negative).
    let hoglin = MobKind::Hoglin.ext().unwrap();
    let mut w = Blocks::new();
    w.set(0, 9, 0, "minecraft:crimson_nylium");
    w.set(5, 9, 0, "minecraft:netherrack");
    assert_eq!(hoglin.spawn_walk_target(&w, BlockPos::new(0, 10, 0)), Some(10.0));
    assert_eq!(hoglin.spawn_walk_target(&w, BlockPos::new(5, 10, 0)), Some(0.0));
}

#[test]
fn enderman_pillager_and_warden_walk_targets_are_zero_in_any_light() {
    let w = Blocks::new();
    for k in [MobKind::Enderman, MobKind::Pillager, MobKind::Warden] {
        assert_eq!(k.ext().unwrap().spawn_walk_target(&w, BlockPos::new(0, 10, 0)), Some(0.0), "{k:?}");
    }
}

#[test]
fn a_strider_spawns_in_lava_under_air() {
    let strider = MobKind::Strider.ext().unwrap();
    assert_eq!(strider.placement(), Placement::InLava);
    assert!(strider.spawn_in_liquids() && strider.spawn_ignores_light());
    let mut w = Blocks::new();
    let mut r = LegacyRandom::new(0);
    // Two lava blocks, air above: the surface of a lava lake.
    w.set(0, 10, 0, "minecraft:lava");
    w.set(0, 11, 0, "minecraft:lava");
    assert_eq!(strider.check_spawn_rules(&w, BlockPos::new(0, 10, 0), &mut r), Some(true));
    // The column ends in rock: no.
    w.set(0, 12, 0, "minecraft:netherrack");
    assert_eq!(strider.check_spawn_rules(&w, BlockPos::new(0, 10, 0), &mut r), Some(false));
}
