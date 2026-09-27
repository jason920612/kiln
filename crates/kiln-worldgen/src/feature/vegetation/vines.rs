//! `VinesFeature` (one vine on the first supporting side) and `MultifaceGrowthFeature` (glow
//! lichen and sculk veins on nearby cave surfaces).

use super::field;
use super::multiface::{Spreader, shuffle, state_for_placement};
use super::shape::can_attach_to;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{block_state, is_air, is_block, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

/// `VinesFeature.place`.
pub fn place_vine(r: &mut Region, p: BlockPos) -> bool {
    if !is_air(r.get(p)) {
        return false;
    }
    for d in Dir::ALL {
        if d != Dir::Down && can_attach_to(r.get(p.relative(d)), d) {
            r.set(p, with_prop(crate::blocks::state::VINE, d.name(), "true"), 2);
            return true;
        }
    }
    false
}

/// `MultifaceGrowthFeature`.
#[derive(Debug)]
pub struct MultifaceGrowth {
    spreader: Spreader,
    search_range: i32,
    floor: bool,
    ceiling: bool,
    wall: bool,
    chance_of_spreading: f32,
    can_be_placed_on: Arc<BlockSet>,
}

fn air_or_water(s: u16) -> bool {
    is_air(s) || is_block(s, "minecraft:water")
}

impl MultifaceGrowth {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        let flag = |k: &str| json.get(k).and_then(Json::as_bool).unwrap_or(false);
        Ok(Self {
            spreader: Spreader::of(block_state(field(json, "block")?)?),
            search_range: json.get("search_range").and_then(Json::as_i32).unwrap_or(10),
            floor: flag("can_place_on_floor"),
            ceiling: flag("can_place_on_ceiling"),
            wall: flag("can_place_on_wall"),
            chance_of_spreading: json.get("chance_of_spreading").and_then(Json::as_f32).unwrap_or(0.5),
            can_be_placed_on: l.blocks(field(json, "can_be_placed_on")?)?,
        })
    }

    /// `validDirections`.
    fn directions(&self) -> Vec<Dir> {
        let mut v = Vec::with_capacity(6);
        if self.ceiling {
            v.push(Dir::Up);
        }
        if self.floor {
            v.push(Dir::Down);
        }
        if self.wall {
            v.extend(Dir::HORIZONTAL);
        }
        v
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !air_or_water(r.get(origin)) {
            return false;
        }
        let mut dirs = self.directions();
        shuffle(&mut dirs, random);
        let here = r.get(origin);
        if self.place_if_possible(r, origin, here, random, &dirs) {
            return true;
        }
        for &d in &dirs {
            let mut rest: Vec<Dir> = self.directions().into_iter().filter(|&e| e != d.opposite()).collect();
            shuffle(&mut rest, random);
            // Vanilla offsets from the origin each step, so every step looks at the same block.
            for _ in 0..self.search_range {
                let p = origin.relative(d);
                let s = r.get(p);
                if !air_or_water(s) && !crate::blocks::same_block(s, self.spreader.block) {
                    break;
                }
                if self.place_if_possible(r, p, s, random, &rest) {
                    return true;
                }
            }
        }
        false
    }

    /// `placeGrowthIfPossible`.
    fn place_if_possible(&self, r: &mut Region, p: BlockPos, s: u16, random: &mut WorldgenRandom, dirs: &[Dir]) -> bool {
        for &d in dirs {
            if !self.can_be_placed_on.contains(r.get(p.relative(d))) {
                continue;
            }
            let Some(new) = state_for_placement(self.spreader.block, r, s, p, d) else { return false };
            r.set(p, new, 3);
            r.mark_post_processing(p);
            if random.next_float() < self.chance_of_spreading {
                self.spreader.spread_from_face_toward_random_direction(r, new, p, d, random, true);
            }
            return true;
        }
        false
    }
}
