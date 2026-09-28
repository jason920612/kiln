//! `HugeFungusFeature`: crimson and warped fungus trees with weeping vines.

use super::{field, flag};
use crate::Error;
use crate::blocks::{block_state, is_air, is_block, same_block, state, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::providers::next_int;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use kiln_data::block_props::replaceable;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct HugeFungus {
    valid_base: u16,
    stem: u16,
    hat: u16,
    decor: u16,
    replaceable_blocks: BlockPredicate,
    planted: bool,
}

impl HugeFungus {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            valid_base: block_state(field(json, "valid_base_block")?)?,
            stem: block_state(field(json, "stem_state")?)?,
            hat: block_state(field(json, "hat_state")?)?,
            decor: block_state(field(json, "decor_state")?)?,
            replaceable_blocks: BlockPredicate::parse(field(json, "replaceable_blocks")?, l)?,
            planted: flag(json, "planted"),
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !same_block(r.get(origin.below()), self.valid_base) {
            return false;
        }
        let mut height = next_int(random, 4, 13);
        if random.next_int_bounded(12) == 0 {
            height *= 2;
        }
        if !self.planted && origin.y + height + 1 >= r.generator.gen_height {
            return false;
        }
        let huge = !self.planted && random.next_float() < 0.06;
        r.set(origin, state::AIR, 260);
        self.place_stem(r, random, origin, height, huge);
        self.place_hat(r, random, origin, height, huge);
        true
    }

    /// `isReplaceable`.
    fn is_replaceable(&self, r: &mut Region, p: BlockPos, check_predicate: bool) -> bool {
        replaceable(r.get(p)) || (check_predicate && self.replaceable_blocks.test(r, p))
    }

    /// `destroyBlock(pos, true)` of a planted fungus: the block is removed.
    fn clear_below(&self, r: &mut Region, p: BlockPos) {
        if self.planted && !is_air(r.get(p.below())) {
            r.set(p, state::AIR, 3);
        }
    }

    fn place_stem(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, height: i32, huge: bool) {
        let rad = huge as i32;
        for dx in -rad..=rad {
            for dz in -rad..=rad {
                let corner = huge && dx.abs() == rad && dz.abs() == rad;
                for dy in 0..height {
                    let p = origin.offset(dx, dy, dz);
                    if !self.is_replaceable(r, p, true) {
                        continue;
                    }
                    if self.planted {
                        self.clear_below(r, p);
                        r.set(p, self.stem, 3);
                    } else if corner {
                        if random.next_float() < 0.1 {
                            r.set(p, self.stem, 3);
                        }
                    } else {
                        r.set(p, self.stem, 3);
                    }
                }
            }
        }
    }

    fn place_hat(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, height: i32, huge: bool) {
        let wart = is_block(self.hat, "minecraft:nether_wart_block");
        let hat_height = (random.next_int_bounded(1 + height / 3) + 5).min(height);
        let hat_start = height - hat_height;
        for y in hat_start..=height {
            let mut rad = if y < height - random.next_int_bounded(3) { 2 } else { 1 };
            if hat_height > 8 && y < hat_start + 4 {
                rad = 3;
            }
            if huge {
                rad += 1;
            }
            for dx in -rad..=rad {
                for dz in -rad..=rad {
                    let edge_x = dx == -rad || dx == rad;
                    let edge_z = dz == -rad || dz == rad;
                    let inside = !edge_x && !edge_z && y != height;
                    let corner = edge_x && edge_z;
                    let low = y < hat_start + 3;
                    let p = origin.offset(dx, y, dz);
                    if !self.is_replaceable(r, p, false) {
                        continue;
                    }
                    self.clear_below(r, p);
                    if low {
                        if !inside {
                            self.place_hat_drop(r, random, p, wart);
                        }
                    } else if inside {
                        self.place_hat_block(r, random, p, 0.1, 0.2, if wart { 0.1 } else { 0.0 });
                    } else if corner {
                        self.place_hat_block(r, random, p, 0.01, 0.7, if wart { 0.083 } else { 0.0 });
                    } else {
                        self.place_hat_block(r, random, p, 5.0e-4, 0.98, if wart { 0.07 } else { 0.0 });
                    }
                }
            }
        }
    }

    fn place_hat_block(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, decor: f32, hat: f32, vines: f32) {
        if random.next_float() < decor {
            r.set(p, self.decor, 3);
        } else if random.next_float() < hat {
            r.set(p, self.hat, 3);
            if random.next_float() < vines {
                weeping_vines(r, random, p);
            }
        }
    }

    fn place_hat_drop(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, wart: bool) {
        if same_block(r.get(p.below()), self.hat) {
            r.set(p, self.hat, 3);
        } else if (random.next_float() as f64) < 0.15 {
            r.set(p, self.hat, 3);
            if wart && random.next_int_bounded(11) == 0 {
                weeping_vines(r, random, p);
            }
        }
    }
}

/// `tryPlaceWeepingVines` and `placeWeepingVinesColumn`.
fn weeping_vines(r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) {
    let mut q = p.below();
    if !is_air(r.get(q)) {
        return;
    }
    let mut length = next_int(random, 1, 5);
    if random.next_int_bounded(7) == 0 {
        length *= 2;
    }
    for i in 0..=length {
        if is_air(r.get(q)) {
            if i == length || !is_air(r.get(q.below())) {
                let age = next_int(random, 23, 25);
                r.set(q, with_prop(state::WEEPING_VINES, "age", &age.to_string()), 2);
                break;
            }
            r.set(q, state::WEEPING_VINES_PLANT, 2);
        }
        q = q.below();
    }
}
