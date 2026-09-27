//! `LakeFeature`: a blob of fluid (air above its middle) in a 16×8×16 box, lined with a barrier.

use super::{field, mark_above_for_post_processing, should_freeze};
use crate::Error;
use crate::block_facts::{fluid, is_solid};
use crate::blocks::{is_air, is_block, state};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct Lake {
    fluid: StateProvider,
    barrier: StateProvider,
    can_place_feature: BlockPredicate,
    can_replace_with_air_or_fluid: BlockPredicate,
    can_replace_with_barrier: BlockPredicate,
}

/// Index into the 16×16×8 shape grid.
fn idx(x: i32, z: i32, y: i32) -> usize {
    ((x * 16 + z) * 8 + y) as usize
}

/// `BlockState.liquid()`: the water, lava and bubble column blocks.
fn liquid(s: u16) -> bool {
    is_block(s, "minecraft:water") || is_block(s, "minecraft:lava") || is_block(s, "minecraft:bubble_column")
}

impl Lake {
    pub fn parse(json: &Json, l: &Loader) -> Result<Lake, Error> {
        let pred = |k: &str| BlockPredicate::parse(field(json, k)?, l);
        Ok(Lake {
            fluid: StateProvider::parse(field(json, "fluid")?, l)?,
            barrier: StateProvider::parse(field(json, "barrier")?, l)?,
            can_place_feature: pred("can_place_feature")?,
            can_replace_with_air_or_fluid: pred("can_replace_with_air_or_fluid")?,
            can_replace_with_barrier: pred("can_replace_with_barrier")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if origin.y <= r.min_y() + 4 {
            return false;
        }
        let o = origin.offset(-8, -4, -8);
        let mut grid = [false; 2048];
        let blobs = random.next_int_bounded(4) + 4;
        for _ in 0..blobs {
            let sx = random.next_double() * 6.0 + 3.0;
            let sy = random.next_double() * 4.0 + 2.0;
            let sz = random.next_double() * 6.0 + 3.0;
            let cx = random.next_double() * (16.0 - sx - 2.0) + 1.0 + sx / 2.0;
            let cy = random.next_double() * (8.0 - sy - 4.0) + 2.0 + sy / 2.0;
            let cz = random.next_double() * (16.0 - sz - 2.0) + 1.0 + sz / 2.0;
            for x in 1..15 {
                for z in 1..15 {
                    for y in 1..7 {
                        let dx = (x as f64 - cx) / (sx / 2.0);
                        let dy = (y as f64 - cy) / (sy / 2.0);
                        let dz = (z as f64 - cz) / (sz / 2.0);
                        if dx * dx + dy * dy + dz * dz < 1.0 {
                            grid[idx(x, z, y)] = true;
                        }
                    }
                }
            }
        }
        let edge = |x: i32, z: i32, y: i32| {
            !grid[idx(x, z, y)]
                && (x < 15 && grid[idx(x + 1, z, y)]
                    || x > 0 && grid[idx(x - 1, z, y)]
                    || z < 15 && grid[idx(x, z + 1, y)]
                    || z > 0 && grid[idx(x, z - 1, y)]
                    || y < 7 && grid[idx(x, z, y + 1)]
                    || y > 0 && grid[idx(x, z, y - 1)])
        };
        let fluid_state = self.fluid.state(r, random, o);
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..8 {
                    if !edge(x, z, y) {
                        continue;
                    }
                    let p = o.offset(x, y, z);
                    let s = r.get(p);
                    if y >= 4 && liquid(s) {
                        return false;
                    }
                    if y < 4 && !is_solid(s) && s != fluid_state {
                        return false;
                    }
                    if !self.can_place_feature.test(r, p) {
                        return false;
                    }
                }
            }
        }
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..8 {
                    if !grid[idx(x, z, y)] {
                        continue;
                    }
                    let p = o.offset(x, y, z);
                    if !self.can_replace_with_air_or_fluid.test(r, p) {
                        continue;
                    }
                    let air = y >= 4;
                    r.set(p, if air { state::CAVE_AIR } else { fluid_state }, 2);
                    if air {
                        r.schedule_block_tick(p, "minecraft:cave_air", 0);
                        mark_above_for_post_processing(r, p);
                    }
                }
            }
        }
        let barrier = self.barrier.state(r, random, o);
        if !is_air(barrier) {
            for x in 0..16 {
                for z in 0..16 {
                    for y in 0..8 {
                        if !edge(x, z, y) || (y >= 4 && random.next_int_bounded(2) == 0) {
                            continue;
                        }
                        let p = o.offset(x, y, z);
                        if is_solid(r.get(p)) && self.can_replace_with_barrier.test(r, p) {
                            r.set(p, barrier, 2);
                            mark_above_for_post_processing(r, p);
                        }
                    }
                }
            }
        }
        if fluid(fluid_state).is_water() {
            for x in 0..16 {
                for z in 0..16 {
                    let p = o.offset(x, 4, z);
                    if should_freeze(r, p, p) && self.can_replace_with_air_or_fluid.test(r, p) {
                        r.set(p, state::ICE, 2);
                    }
                }
            }
        }
        true
    }
}
