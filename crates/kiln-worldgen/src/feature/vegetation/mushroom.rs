//! `HugeBrownMushroomFeature` and `HugeRedMushroomFeature` (`AbstractHugeMushroomFeature`).

use super::{field, state_provider};
use crate::Error;
use crate::blocks::{has_prop, is_air, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct HugeMushroom {
    cap: StateProvider,
    stem: StateProvider,
    foliage_radius: i32,
    can_place_on: BlockPredicate,
    red: bool,
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

impl HugeMushroom {
    pub fn parse(json: &Json, l: &Loader, red: bool) -> Result<Self, Error> {
        Ok(Self {
            cap: state_provider(json, "cap_provider", l)?,
            stem: state_provider(json, "stem_provider", l)?,
            foliage_radius: json.get("foliage_radius").and_then(Json::as_i32).unwrap_or(2),
            can_place_on: BlockPredicate::parse(field(json, "can_place_on")?, l)?,
            red,
        })
    }

    /// `AbstractHugeMushroomFeature.foliageRadius()`.
    pub fn foliage_radius(&self) -> i32 {
        self.foliage_radius
    }

    /// `getTreeRadiusForHeight(-1, -1, foliageRadius, y)`, as `isValidPosition` calls it.
    fn radius_for_height(&self, y: i32) -> i32 {
        if self.red {
            // The red variant compares y with the tree height, passed as -1: never within.
            0
        } else if y <= 3 {
            0
        } else {
            self.foliage_radius
        }
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut height = random.next_int_bounded(3) + 4;
        if random.next_int_bounded(12) == 0 {
            height *= 2;
        }
        if !self.valid_position(r, origin, height) {
            return false;
        }
        if self.red {
            self.red_cap(r, random, origin, height);
        } else {
            self.brown_cap(r, random, origin, height);
        }
        for i in 0..height {
            let s = self.stem.state(r, random, origin);
            place_block(r, origin.above_n(i), s);
        }
        true
    }

    /// `isValidPosition`.
    fn valid_position(&self, r: &mut Region, origin: BlockPos, height: i32) -> bool {
        let y = origin.y;
        if y < r.min_y() + 1 || y + height + 1 > r.max_y() {
            return false;
        }
        if !self.can_place_on.test(r, origin.below()) {
            return false;
        }
        for dy in 0..=height {
            let rad = self.radius_for_height(dy);
            for dx in -rad..=rad {
                for dz in -rad..=rad {
                    let s = r.get(origin.offset(dx, dy, dz));
                    if !is_air(s) && !crate::vtags::is(s, "leaves") {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `HugeBrownMushroomFeature.makeCap`: a flat square with cut corners on top.
    fn brown_cap(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, height: i32) {
        let rad = self.foliage_radius;
        for dx in -rad..=rad {
            for dz in -rad..=rad {
                let (min_x, max_x, min_z, max_z) = (dx == -rad, dx == rad, dz == -rad, dz == rad);
                let edge_x = min_x || max_x;
                let edge_z = min_z || max_z;
                if edge_x && edge_z {
                    continue;
                }
                let p = origin.offset(dx, height, dz);
                let west = min_x || (edge_z && dx == 1 - rad);
                let east = max_x || (edge_z && dx == rad - 1);
                let north = min_z || (edge_x && dz == 1 - rad);
                let south = max_z || (edge_x && dz == rad - 1);
                let mut s = self.cap.state(r, random, origin);
                if ["west", "east", "north", "south"].iter().all(|k| has_prop(s, k)) {
                    s = with_prop(s, "west", bool_str(west));
                    s = with_prop(s, "east", bool_str(east));
                    s = with_prop(s, "north", bool_str(north));
                    s = with_prop(s, "south", bool_str(south));
                }
                place_block(r, p, s);
            }
        }
    }

    /// `HugeRedMushroomFeature.makeCap`: a dome over the top three blocks of the stem.
    fn red_cap(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, height: i32) {
        for y in height - 3..=height {
            let rad = if y < height { self.foliage_radius } else { self.foliage_radius - 1 };
            let inner = self.foliage_radius - 2;
            for dx in -rad..=rad {
                for dz in -rad..=rad {
                    let edge_x = dx == -rad || dx == rad;
                    let edge_z = dz == -rad || dz == rad;
                    if y < height && edge_x == edge_z {
                        continue;
                    }
                    let p = origin.offset(dx, y, dz);
                    let mut s = self.cap.state(r, random, origin);
                    if ["west", "east", "north", "south", "up"].iter().all(|k| has_prop(s, k)) {
                        s = with_prop(s, "up", bool_str(y >= height - 1));
                        s = with_prop(s, "west", bool_str(dx < -inner));
                        s = with_prop(s, "east", bool_str(dx > inner));
                        s = with_prop(s, "north", bool_str(dz < -inner));
                        s = with_prop(s, "south", bool_str(dz > inner));
                    }
                    place_block(r, p, s);
                }
            }
        }
    }
}

/// `placeMushroomBlock`: only into air or blocks mushrooms replace.
fn place_block(r: &mut Region, p: BlockPos, s: u16) {
    let here = r.get(p);
    if is_air(here) || crate::vtags::is(here, "replaceable_by_mushrooms") {
        r.set(p, s, 3);
    }
}
