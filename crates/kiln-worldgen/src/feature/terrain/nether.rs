//! Nether terrain: `ReplaceBlobsFeature` (basalt and blackstone blobs), `DeltaFeature` (lava
//! deltas) and `SteppedColumnClusterFeature` (basalt columns).

use super::{field, within_manhattan};
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{block_state, is_air, is_block, same_block};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::providers::IntProvider;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

#[derive(Debug)]
pub struct ReplaceBlobs {
    target: u16,
    state: u16,
    radius: IntProvider,
}

impl ReplaceBlobs {
    pub fn parse(json: &Json) -> Result<ReplaceBlobs, Error> {
        Ok(ReplaceBlobs {
            target: block_state(field(json, "target")?)?,
            state: block_state(field(json, "state")?)?,
            radius: IntProvider::parse(field(json, "radius")?)?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut t = origin.at_y(origin.y.clamp(r.min_y() + 1, r.max_y()));
        loop {
            if t.y <= r.min_y() + 1 {
                return false;
            }
            if same_block(r.get(t), self.target) {
                break;
            }
            t = t.below();
        }
        let (rx, ry, rz) = (self.radius.sample(random), self.radius.sample(random), self.radius.sample(random));
        let reach = rx.max(ry.max(rz));
        let mut placed = false;
        for q in within_manhattan(t, rx, ry, rz) {
            if q.dist_manhattan(t) > reach {
                break;
            }
            if same_block(r.get(q), self.target) {
                r.set_block(q, self.state);
                placed = true;
            }
        }
        placed
    }
}

#[derive(Debug)]
pub struct Delta {
    contents: u16,
    rim: u16,
    size: IntProvider,
    rim_size: IntProvider,
}

/// `DeltaFeature.CANNOT_REPLACE`.
const DELTA_CANNOT_REPLACE: [&str; 7] = [
    "minecraft:bedrock",
    "minecraft:nether_bricks",
    "minecraft:nether_brick_fence",
    "minecraft:nether_brick_stairs",
    "minecraft:nether_wart",
    "minecraft:chest",
    "minecraft:spawner",
];

impl Delta {
    pub fn parse(json: &Json) -> Result<Delta, Error> {
        Ok(Delta {
            contents: block_state(field(json, "contents")?)?,
            rim: block_state(field(json, "rim")?)?,
            size: IntProvider::parse(field(json, "size")?)?,
            rim_size: IntProvider::parse(field(json, "rim_size")?)?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let rim = random.next_double() < 0.9;
        let rx = if rim { self.rim_size.sample(random) } else { 0 };
        let rz = if rim { self.rim_size.sample(random) } else { 0 };
        let has_rim = rim && rx != 0 && rz != 0;
        let sx = self.size.sample(random);
        let sz = self.size.sample(random);
        let reach = sx.max(sz);
        let mut placed = false;
        for q in within_manhattan(origin, sx, 0, sz) {
            if q.dist_manhattan(origin) > reach {
                break;
            }
            if !self.is_clear(r, q) {
                continue;
            }
            if has_rim {
                placed = true;
                r.set_block(q, self.rim);
            }
            let inner = q.offset(rx, 0, rz);
            if self.is_clear(r, inner) {
                placed = true;
                r.set_block(inner, self.contents);
            }
        }
        placed
    }

    /// `DeltaFeature.isClear`: replaceable, solid all around and below, open above.
    fn is_clear(&self, r: &mut Region, p: BlockPos) -> bool {
        let s = r.get(p);
        if same_block(s, self.contents) || DELTA_CANNOT_REPLACE.iter().any(|b| is_block(s, b)) {
            return false;
        }
        Dir::ALL.into_iter().all(|d| r.is_air(p.relative(d)) == (d == Dir::Up))
    }
}

#[derive(Debug)]
pub struct SteppedColumnCluster {
    block: StateProvider,
    continue_through: BlockPredicate,
    can_replace: BlockPredicate,
    cannot_place_on: Arc<BlockSet>,
    cluster_reach: IntProvider,
    column_count: IntProvider,
    column_reach: IntProvider,
    height: IntProvider,
}

impl SteppedColumnCluster {
    pub fn parse(json: &Json, l: &Loader) -> Result<SteppedColumnCluster, Error> {
        let ints = |k: &str| IntProvider::parse(field(json, k)?);
        Ok(SteppedColumnCluster {
            block: StateProvider::parse(field(json, "block")?, l)?,
            continue_through: BlockPredicate::parse(field(json, "continue_through")?, l)?,
            can_replace: BlockPredicate::parse(field(json, "can_replace")?, l)?,
            cannot_place_on: l.blocks(field(json, "cannot_place_on")?)?,
            cluster_reach: ints("cluster_reach")?,
            column_count: ints("column_count")?,
            column_reach: ints("column_reach")?,
            height: ints("height")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !self.can_place_at(r, origin) {
            return false;
        }
        let height = self.height.sample(random);
        let reach = height.min(self.cluster_reach.sample(random));
        let count = self.column_count.sample(random);
        let mut placed = false;
        // `BlockPos.randomBetweenClosed` over a one-block-high box (the y draw still happens).
        for _ in 0..count {
            let x = origin.x - reach + random.next_int_bounded(2 * reach + 1);
            let y = origin.y + random.next_int_bounded(1);
            let z = origin.z - reach + random.next_int_bounded(2 * reach + 1);
            let p = BlockPos::new(x, y, z);
            let left = height - p.dist_manhattan(origin);
            if left >= 0 {
                let column_reach = self.column_reach.sample(random);
                placed |= self.place_column(r, random, p, left, column_reach);
            }
        }
        placed
    }

    fn place_column(&self, r: &mut Region, random: &mut WorldgenRandom, center: BlockPos, height: i32, reach: i32) -> bool {
        let mut placed = false;
        for p in super::between_closed(center.offset(-reach, 0, -reach), center.offset(reach, 0, reach)) {
            let d = p.dist_manhattan(center);
            let start = if self.can_replace.test(r, p) { self.find_surface(r, p, d) } else { self.find_air(r, p, d) };
            let Some(mut q) = start else { continue };
            let mut left = height - d / 2;
            while left >= 0 {
                if self.can_replace.test(r, q) {
                    let s = self.block.state(r, random, q);
                    r.set_block(q, s);
                    q = q.above();
                    placed = true;
                } else if self.continue_through.test(r, q) {
                    q = q.above();
                } else {
                    break;
                }
                left -= 1;
            }
        }
        placed
    }

    fn find_surface(&self, r: &mut Region, mut p: BlockPos, mut steps: i32) -> Option<BlockPos> {
        while p.y > r.min_y() + 1 && steps > 0 {
            steps -= 1;
            if self.can_place_at(r, p) {
                return Some(p);
            }
            p = p.below();
        }
        None
    }

    fn can_place_at(&self, r: &mut Region, p: BlockPos) -> bool {
        if !self.can_replace.test(r, p) {
            return false;
        }
        let below = r.get(p.below());
        !is_air(below) && !self.cannot_place_on.contains(below)
    }

    fn find_air(&self, r: &mut Region, mut p: BlockPos, mut steps: i32) -> Option<BlockPos> {
        while p.y <= r.max_y() && steps > 0 {
            steps -= 1;
            let s = r.get(p);
            if self.cannot_place_on.contains(s) {
                return None;
            }
            if is_air(s) {
                return Some(p);
            }
            p = p.above();
        }
        None
    }
}
