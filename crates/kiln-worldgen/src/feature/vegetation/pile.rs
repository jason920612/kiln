//! `BambooFeature` (a bamboo stalk, sometimes on a podzol disc) and `BlockPileFeature` (hay,
//! melons, snow... heaped in a rough ellipse).

use super::state_provider;
use crate::Error;
use crate::block_facts::{Dir, Support, is_face_sturdy};
use crate::blocks::{is_air, is_block, state, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::providers::float;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;

/// `BambooFeature`.
#[derive(Debug)]
pub struct Bamboo {
    probability: f32,
}

fn bamboo(leaves: &str, stage: &str) -> u16 {
    let s = with_prop(state::BAMBOO, "age", "1");
    with_prop(with_prop(s, "leaves", leaves), "stage", stage)
}

impl Bamboo {
    pub fn parse(json: &Json) -> Result<Self, Error> {
        Ok(Self { probability: float(json, "probability")? })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !is_air(r.get(origin)) {
            return false;
        }
        if !crate::survive::can_survive(state::BAMBOO, r, origin) {
            return false;
        }
        let height = random.next_int_bounded(12) + 5;
        if random.next_float() < self.probability {
            let radius = random.next_int_bounded(4) + 1;
            for x in origin.x - radius..=origin.x + radius {
                for z in origin.z - radius..=origin.z + radius {
                    let (dx, dz) = (x - origin.x, z - origin.z);
                    if dx * dx + dz * dz > radius * radius {
                        continue;
                    }
                    let p = BlockPos::new(x, r.height_at(Heightmap::WorldSurface, x, z) - 1, z);
                    if crate::vtags::is(r.get(p), "beneath_bamboo_podzol_replaceable") {
                        r.set(p, state::PODZOL, 2);
                    }
                }
            }
        }
        let mut p = origin;
        for _ in 0..height {
            if !is_air(r.get(p)) {
                break;
            }
            r.set(p, bamboo("none", "0"), 2);
            p = p.above();
        }
        if p.y - origin.y >= 3 {
            r.set(p, bamboo("large", "1"), 2);
            r.set(p.below(), bamboo("large", "0"), 2);
            r.set(p.below_n(2), bamboo("small", "0"), 2);
        }
        true
    }
}

/// `BlockPileFeature`.
#[derive(Debug)]
pub struct BlockPile {
    state: StateProvider,
}

impl BlockPile {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self { state: state_provider(json, "state_provider", l)? })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if origin.y < r.min_y() + 5 {
            return false;
        }
        let rx = 2 + random.next_int_bounded(2);
        let rz = 2 + random.next_int_bounded(2);
        for z in origin.z - rz..=origin.z + rz {
            for y in origin.y..=origin.y + 1 {
                for x in origin.x - rx..=origin.x + rx {
                    let p = BlockPos::new(x, y, z);
                    let (dx, dz) = (x - origin.x, z - origin.z);
                    let d = (dx * dx + dz * dz) as f32;
                    let a = random.next_float() * 10.0;
                    let b = random.next_float() * 6.0;
                    if d <= a - b || (random.next_float() as f64) < 0.031 {
                        self.try_place(r, random, p);
                    }
                }
            }
        }
        true
    }

    /// `tryPlaceBlock`.
    fn try_place(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) {
        if !is_air(r.get(p)) {
            return;
        }
        let below = r.get(p.below());
        let ok = if is_block(below, "minecraft:dirt_path") { random.next_bool() } else { is_face_sturdy(below, Dir::Up, Support::Full) };
        if ok {
            let s = self.state.state(r, random, p);
            r.set(p, s, 260);
        }
    }
}
