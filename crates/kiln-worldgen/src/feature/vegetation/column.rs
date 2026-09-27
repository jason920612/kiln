//! Straight runs of blocks: `BlockColumnFeature` (kelp, sugar cane, cave vines...),
//! `SingleBlockPillarFeature`, and the scattered fills `ProjectedRandomPatchySquare` and
//! `RandomNeighborSpreadFeature`.

use super::{dir, field, flag, placed_opt, predicate_or_true, state_provider};
use crate::Error;
use crate::block_facts::Dir;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::providers::{IntProvider, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

/// `BlockColumnFeature`.
#[derive(Debug)]
pub struct BlockColumn {
    layers: Vec<(IntProvider, StateProvider)>,
    direction: Dir,
    allowed: BlockPredicate,
    prioritize_tip: bool,
}

impl BlockColumn {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        let layers = field(json, "layers")?
            .as_array()
            .ok_or_else(|| Error::Invalid("layers must be a list".into()))?
            .iter()
            .map(|e| Ok((IntProvider::parse(field(e, "height")?)?, state_provider(e, "provider", l)?)))
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            layers,
            direction: dir(json, "direction")?,
            allowed: BlockPredicate::parse(field(json, "allowed_placement")?, l)?,
            prioritize_tip: flag(json, "prioritize_tip"),
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut heights: Vec<i32> = self.layers.iter().map(|(h, _)| h.sample(random)).collect();
        let total: i32 = heights.iter().sum();
        if total == 0 {
            return false;
        }
        let mut check = origin.relative(self.direction);
        for i in 0..total {
            if !self.allowed.test(r, check) {
                truncate(&mut heights, total, i, self.prioritize_tip);
                break;
            }
            check = check.relative(self.direction);
        }
        let mut p = origin;
        for (i, &n) in heights.iter().enumerate() {
            for _ in 0..n {
                let s = self.layers[i].1.state(r, random, p);
                r.set(p, s, 2);
                p = p.relative(self.direction);
            }
        }
        true
    }
}

/// `BlockColumnFeature.truncate`: removes `total - fit` blocks from the layers, from the first
/// layer on when the tip has priority, else from the last.
fn truncate(heights: &mut [i32], total: i32, fit: i32, prioritize_tip: bool) {
    let mut excess = total - fit;
    let n = heights.len() as i32;
    let (step, start, end) = if prioritize_tip { (1, 0, n) } else { (-1, n - 1, -1) };
    let mut i = start;
    while i != end && excess > 0 {
        let cut = heights[i as usize].min(excess);
        excess -= cut;
        heights[i as usize] -= cut;
        i += step;
    }
}

/// `SingleBlockPillarFeature`.
#[derive(Debug)]
pub struct Pillar {
    block: StateProvider,
    can_replace: BlockPredicate,
    direction: Dir,
    chance_to_continue: f32,
    cap: Option<usize>,
}

impl Pillar {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            block: state_provider(json, "block", l)?,
            can_replace: predicate_or_true(json, "can_replace", l)?,
            direction: dir(json, "direction")?,
            chance_to_continue: json.get("chance_to_continue").and_then(Json::as_f32).unwrap_or(1.0),
            cap: placed_opt(json, "cap_feature", f, l)?,
        })
    }

    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut p = origin;
        while self.can_replace.test(r, p) && random.next_float() < self.chance_to_continue && !r.is_outside_build_height(p.y) {
            let s = self.block.state(r, random, p);
            r.set(p, s, 2);
            p = p.relative(self.direction);
        }
        p = p.relative(self.direction.opposite());
        if let Some(cap) = self.cap {
            f.place_placed(cap, r, random, p, false);
        }
        true
    }

    pub fn nested(&self) -> Vec<usize> {
        self.cap.into_iter().collect()
    }
}

/// `ProjectedRandomPatchySquare`: a rough square (sparser towards the corners) whose blocks
/// drop down through `project_through`.
#[derive(Debug)]
pub struct PatchySquare {
    block: StateProvider,
    project_through: BlockPredicate,
    size: IntProvider,
    max_projection_height: i32,
}

impl PatchySquare {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            block: state_provider(json, "block", l)?,
            project_through: BlockPredicate::parse(field(json, "project_through")?, l)?,
            size: IntProvider::parse(field(json, "size")?)?,
            max_projection_height: int(json, "max_projection_height")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let size = self.size.sample(random);
        let bound = size * size + 1;
        for dx in -size..=size {
            for dz in -size..=size {
                let corner = dx.abs() * dz.abs();
                if random.next_int_bounded(bound) >= bound - corner {
                    continue;
                }
                let mut p = origin.offset(dx, 0, dz);
                let mut left = self.max_projection_height;
                while self.project_through.test(r, p.below()) {
                    p = p.below();
                    left -= 1;
                    if left <= 0 {
                        break;
                    }
                }
                if let Some(s) = self.block.optional_state(r, random, p) {
                    r.set(p, s, 2);
                }
            }
        }
        true
    }
}

/// `RandomNeighborSpreadFeature`: blocks grown next to exactly one accepted neighbour.
#[derive(Debug)]
pub struct NeighborSpread {
    block: StateProvider,
    accepted: Arc<BlockSet>,
    can_replace: BlockPredicate,
    attempts: IntProvider,
    xz_offset: IntProvider,
    y_offset: IntProvider,
}

impl NeighborSpread {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            block: state_provider(json, "block", l)?,
            accepted: l.blocks(field(json, "accepted_neighbors")?)?,
            can_replace: BlockPredicate::parse(field(json, "can_replace")?, l)?,
            attempts: IntProvider::parse(field(json, "attempts")?)?,
            xz_offset: IntProvider::parse(field(json, "xz_offset")?)?,
            y_offset: IntProvider::parse(field(json, "y_offset")?)?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let s = self.block.state(r, random, origin);
        r.set(origin, s, 2);
        let attempts = self.attempts.sample(random);
        for _ in 0..attempts {
            let dx = self.xz_offset.sample(random);
            let dy = self.y_offset.sample(random);
            let dz = self.xz_offset.sample(random);
            let p = origin.offset(dx, dy, dz);
            if !self.can_replace.test(r, p) {
                continue;
            }
            let mut neighbors = 0;
            for d in Dir::ALL {
                if self.accepted.contains(r.get(p.relative(d))) {
                    neighbors += 1;
                }
                if neighbors > 1 {
                    break;
                }
            }
            if neighbors == 1 {
                let s = self.block.state(r, random, p);
                r.set(p, s, 2);
            }
        }
        true
    }
}
