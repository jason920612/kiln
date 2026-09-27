//! `UnderwaterMagmaFeature`: magma blocks scattered around the floor below a water column,
//! only where no face is exposed.

use super::between_closed;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{is_air, is_water, state};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::{float, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_data::block_props::face_full;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct UnderwaterMagma {
    floor_search_range: i32,
    placement_radius_around_floor: i32,
    placement_probability_per_valid_position: f32,
}

impl UnderwaterMagma {
    pub fn parse(json: &Json) -> Result<UnderwaterMagma, Error> {
        Ok(UnderwaterMagma {
            floor_search_range: int(json, "floor_search_range")?,
            placement_radius_around_floor: int(json, "placement_radius_around_floor")?,
            placement_probability_per_valid_position: float(json, "placement_probability_per_valid_position")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let Some(floor) = self.floor_y(r, origin) else { return false };
        let center = origin.at_y(floor);
        let n = self.placement_radius_around_floor;
        let mut placed = 0;
        for p in between_closed(center.offset(-n, -n, -n), center.offset(n, n, n)) {
            if random.next_float() < self.placement_probability_per_valid_position && valid_placement(r, p) {
                r.set(p, state::MAGMA_BLOCK, 2);
                placed += 1;
            }
        }
        placed > 0
    }

    /// `Column.scan(level, origin, range, is water, is not water).getFloor()`.
    fn floor_y(&self, r: &mut Region, origin: BlockPos) -> Option<i32> {
        if !is_water(r.get(origin)) {
            return None;
        }
        let mut p = origin;
        for _ in 1..self.floor_search_range {
            if !is_water(r.get(p)) {
                break;
            }
            p = p.below();
        }
        (!is_water(r.get(p))).then_some(p.y)
    }
}

fn valid_placement(r: &mut Region, p: BlockPos) -> bool {
    let s = r.get(p);
    if is_water(s) || is_air(s) || visible_from_outside(r, p.below(), Dir::Up) {
        return false;
    }
    !Dir::HORIZONTAL.into_iter().any(|d| visible_from_outside(r, p.relative(d), d.opposite()))
}

/// Whether the face of the block at `p` toward `d` does not fully occlude.
fn visible_from_outside(r: &mut Region, p: BlockPos, d: Dir) -> bool {
    !face_full(r.get(p), d as u8)
}
