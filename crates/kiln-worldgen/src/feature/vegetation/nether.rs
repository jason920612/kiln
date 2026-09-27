//! `SteppedColumnClusterFeature` (basalt column clusters) and `ChorusPlantFeature`
//! (`ChorusFlowerBlock.generatePlant`).

use super::{field, state_provider};
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{is_air, is_block, state, with_prop};
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

/// `SteppedColumnClusterFeature`.
#[derive(Debug)]
pub struct SteppedColumns {
    block: StateProvider,
    continue_through: BlockPredicate,
    can_replace: BlockPredicate,
    cannot_place_on: Arc<BlockSet>,
    cluster_reach: IntProvider,
    column_count: IntProvider,
    column_reach: IntProvider,
    height: IntProvider,
}

impl SteppedColumns {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        let int = |k: &str| IntProvider::parse(field(json, k)?);
        Ok(Self {
            block: state_provider(json, "block", l)?,
            continue_through: BlockPredicate::parse(field(json, "continue_through")?, l)?,
            can_replace: BlockPredicate::parse(field(json, "can_replace")?, l)?,
            cannot_place_on: l.blocks(field(json, "cannot_place_on")?)?,
            cluster_reach: int("cluster_reach")?,
            column_count: int("column_count")?,
            column_reach: int("column_reach")?,
            height: int("height")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !self.can_place_at(r, origin) {
            return false;
        }
        let height = self.height.sample(random);
        let reach = height.min(self.cluster_reach.sample(random));
        let count = self.column_count.sample(random);
        let (w, h, d) = (2 * reach + 1, 1, 2 * reach + 1);
        let mut placed = false;
        for _ in 0..count {
            let x = origin.x - reach + random.next_int_bounded(w);
            let y = origin.y + random.next_int_bounded(h);
            let z = origin.z - reach + random.next_int_bounded(d);
            let p = BlockPos::new(x, y, z);
            let column_height = height - p.dist_manhattan(origin);
            if column_height >= 0 {
                let column_reach = self.column_reach.sample(random);
                placed |= self.place_column(r, random, p, column_height, column_reach);
            }
        }
        placed
    }

    /// `placeColumn`: a stepped column around `p`, lower toward its edge.
    fn place_column(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, height: i32, reach: i32) -> bool {
        let mut placed = false;
        for z in p.z - reach..=p.z + reach {
            for x in p.x - reach..=p.x + reach {
                let q = BlockPos::new(x, p.y, z);
                let d = q.dist_manhattan(p);
                let start = if self.can_replace.test(r, q) { self.find_surface(r, q, d) } else { self.find_air(r, q, d) };
                let Some(mut c) = start else { continue };
                let mut left = height - d / 2;
                while left >= 0 {
                    if self.can_replace.test(r, c) {
                        let s = self.block.state(r, random, c);
                        r.set(c, s, 3);
                        c = c.above();
                        placed = true;
                    } else if self.continue_through.test(r, c) {
                        c = c.above();
                    } else {
                        break;
                    }
                    left -= 1;
                }
            }
        }
        placed
    }

    /// `findSurface`: down to the first position the column can stand on.
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

    /// `canPlaceAt`.
    fn can_place_at(&self, r: &mut Region, p: BlockPos) -> bool {
        if !self.can_replace.test(r, p) {
            return false;
        }
        let below = r.get(p.below());
        !is_air(below) && !self.cannot_place_on.contains(below)
    }

    /// `findAir`: up to the first air, unless a block the column cannot stand on is in the way.
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

/// `ChorusPlantFeature.place`.
pub fn place_chorus(r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    if !is_air(r.get(origin)) || !crate::vtags::is(r.get(origin.below()), "supports_chorus_plant") {
        return false;
    }
    let s = chorus_connections(r, origin);
    r.set(origin, s, 2);
    grow_chorus(r, origin, random, origin, 8, 0);
    true
}

/// `ChorusPlantBlock.getStateWithConnections` for a chorus plant.
fn chorus_connections(r: &mut Region, p: BlockPos) -> u16 {
    let connects = |s: u16| is_block(s, "minecraft:chorus_plant") || is_block(s, "minecraft:chorus_flower");
    let below = r.get(p.below());
    let mut s = with_prop(
        state::CHORUS_PLANT,
        "down",
        if connects(below) || crate::vtags::is(below, "supports_chorus_plant") { "true" } else { "false" },
    );
    for d in [Dir::Up, Dir::North, Dir::East, Dir::South, Dir::West] {
        let n = r.get(p.relative(d));
        s = with_prop(s, d.name(), if connects(n) { "true" } else { "false" });
    }
    s
}

/// `ChorusFlowerBlock.allNeighborsEmpty`.
fn neighbors_empty(r: &mut Region, p: BlockPos, except: Option<Dir>) -> bool {
    Dir::HORIZONTAL.into_iter().all(|d| Some(d) == except || is_air(r.get(p.relative(d))))
}

/// `ChorusFlowerBlock.growTreeRecursive`.
fn grow_chorus(r: &mut Region, p: BlockPos, random: &mut WorldgenRandom, root: BlockPos, max_distance: i32, depth: i32) {
    let mut height = random.next_int_bounded(4) + 1;
    if depth == 0 {
        height += 1;
    }
    for i in 0..height {
        let q = p.above_n(i + 1);
        if !neighbors_empty(r, q, None) {
            return;
        }
        let s = chorus_connections(r, q);
        r.set(q, s, 2);
        let s = chorus_connections(r, q.below());
        r.set(q.below(), s, 2);
    }
    let mut branched = false;
    if depth < 4 {
        let mut branches = random.next_int_bounded(4);
        if depth == 0 {
            branches += 1;
        }
        for _ in 0..branches {
            let d = Dir::HORIZONTAL[random.next_int_bounded(4) as usize];
            let q = p.above_n(height).relative(d);
            if (q.x - root.x).abs() < max_distance
                && (q.z - root.z).abs() < max_distance
                && is_air(r.get(q))
                && is_air(r.get(q.below()))
                && neighbors_empty(r, q, Some(d.opposite()))
            {
                branched = true;
                let s = chorus_connections(r, q);
                r.set(q, s, 2);
                let back = q.relative(d.opposite());
                let s = chorus_connections(r, back);
                r.set(back, s, 2);
                grow_chorus(r, q, random, root, max_distance, depth + 1);
            }
        }
    }
    if !branched {
        r.set(p.above_n(height), with_prop(state::CHORUS_FLOWER, "age", "5"), 2);
    }
}
