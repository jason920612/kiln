//! `SnowAndFreezeFeature` (snow and ice on top of a chunk) and `BlueIceFeature`.

use super::{should_freeze, should_snow};
use crate::block_facts::Dir;
use crate::blocks::{has_prop, is_air, is_block, is_water, state, with_prop};
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_javamath::random::RandomSource;

/// `SnowAndFreezeFeature.place`.
pub fn freeze_top_layer(r: &mut Region, origin: BlockPos) -> bool {
    for dx in 0..16 {
        for dz in 0..16 {
            let (x, z) = (origin.x + dx, origin.z + dz);
            let top = BlockPos::new(x, r.height_at(Heightmap::MotionBlocking, x, z), z);
            let below = top.below();
            if should_freeze(r, top, below) {
                r.set(below, state::ICE, 2);
            }
            if should_snow(r, top, top) {
                r.set(top, state::SNOW, 2);
                let s = r.get(below);
                if has_prop(s, "snowy") {
                    r.set(below, with_prop(s, "snowy", "true"), 2);
                }
            }
        }
    }
    true
}

/// `BlueIceFeature.place`.
pub fn blue_ice(r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    if origin.y > r.sea_level() - 1 {
        return false;
    }
    if !is_water(r.get(origin)) && !is_water(r.get(origin.below())) {
        return false;
    }
    let near_packed_ice = Dir::ALL
        .into_iter()
        .filter(|&d| d != Dir::Down)
        .any(|d| is_block(r.get(origin.relative(d)), "minecraft:packed_ice"));
    if !near_packed_ice {
        return false;
    }
    r.set(origin, state::BLUE_ICE, 2);
    for _ in 0..200 {
        let dy = random.next_int_bounded(5) - random.next_int_bounded(6);
        let mut spread = 3;
        if dy < 2 {
            spread += dy / 2;
        }
        if spread < 1 {
            continue;
        }
        let dx = random.next_int_bounded(spread) - random.next_int_bounded(spread);
        let dz = random.next_int_bounded(spread) - random.next_int_bounded(spread);
        let p = origin.offset(dx, dy, dz);
        let s = r.get(p);
        let replaceable = is_air(s) || is_water(s) || is_block(s, "minecraft:packed_ice") || is_block(s, "minecraft:ice");
        if replaceable && Dir::ALL.into_iter().any(|d| is_block(r.get(p.relative(d)), "minecraft:blue_ice")) {
            r.set(p, state::BLUE_ICE, 2);
        }
    }
    true
}
