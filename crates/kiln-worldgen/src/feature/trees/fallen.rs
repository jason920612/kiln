//! `FallenTreeFeature`: a one-block stump and a log lying next to it.

use super::decorator::{DecoCtx, Decorator};
use super::field;
use super::jset::JHashSet;
use super::tree::valid_tree_pos;
use super::trunk::{along, random_horizontal};
use crate::Error;
use crate::block_facts::{Dir, Support, is_face_sturdy};
use crate::blocks::is_air;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::IntProvider;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct FallenTree {
    trunk_provider: StateProvider,
    log_length: IntProvider,
    pub stump_decorators: Vec<Decorator>,
    pub log_decorators: Vec<Decorator>,
}

fn decorators(json: &Json, key: &str, f: &mut Features, l: &Loader) -> Result<Vec<Decorator>, Error> {
    match json.get(key).and_then(Json::as_array) {
        Some(list) => list.iter().map(|d| Decorator::parse(d, f, l)).collect(),
        None => Ok(Vec::new()),
    }
}

/// `FallenTreeFeature.isOverSolidGround`.
fn over_solid_ground(r: &mut Region, p: BlockPos) -> bool {
    is_face_sturdy(r.get(p.below()), Dir::Up, Support::Full)
}

impl FallenTree {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader) -> Result<FallenTree, Error> {
        Ok(FallenTree {
            trunk_provider: StateProvider::parse(field(json, "trunk_provider")?, l)?,
            log_length: IntProvider::parse(field(json, "log_length")?)?,
            stump_decorators: decorators(json, "stump_decorators", f, l)?,
            log_decorators: decorators(json, "log_decorators", f, l)?,
        })
    }

    /// `FallenTreeFeature.place`.
    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let stump = self.place_log(r, random, origin, |s| s);
        let mut set = JHashSet::new();
        set.insert(stump);
        self.decorate(f, r, random, &set, &self.stump_decorators);
        let dir = random_horizontal(random);
        let length = self.log_length.sample(random) - 2;
        let mut p = origin.relative_n(dir, 2 + random.next_int_bounded(2));
        p = self.ground_start(r, p);
        if self.can_place_log(r, length, p, dir) {
            let mut logs = JHashSet::new();
            for _ in 0..length {
                logs.insert(self.place_log(r, random, p, |s| along(s, dir)));
                p = p.relative(dir);
            }
            self.decorate(f, r, random, &logs, &self.log_decorators);
        }
        true
    }

    /// `FallenTreeFeature.setGroundHeightForFallenLogStartPos`.
    fn ground_start(&self, r: &mut Region, p: BlockPos) -> BlockPos {
        let mut p = p.above();
        for _ in 0..6 {
            if valid_tree_pos(r, p) && over_solid_ground(r, p) {
                return p;
            }
            p = p.below();
        }
        p
    }

    /// `FallenTreeFeature.canPlaceEntireFallenLog`.
    fn can_place_log(&self, r: &mut Region, length: i32, start: BlockPos, dir: Dir) -> bool {
        let mut gap = 0;
        let mut p = start;
        for _ in 0..length {
            if !valid_tree_pos(r, p) {
                return false;
            }
            if !over_solid_ground(r, p) {
                gap += 1;
                if gap > 2 {
                    return false;
                }
            } else {
                gap = 0;
            }
            p = p.relative(dir);
        }
        true
    }

    /// `FallenTreeFeature.placeLogBlock`: `setBlockAndUpdate`, then marks the blocks above.
    fn place_log(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, modify: impl Fn(u16) -> u16) -> BlockPos {
        let s = modify(self.trunk_provider.state(r, random, p));
        r.set(p, s, 3);
        let mut q = p;
        for _ in 0..2 {
            q = q.above();
            if is_air(r.get(q)) {
                break;
            }
            r.mark_post_processing(q);
        }
        p
    }

    /// `FallenTreeFeature.decorateLogs`.
    fn decorate(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, logs: &JHashSet, list: &[Decorator]) {
        if list.is_empty() {
            return;
        }
        let empty = JHashSet::new();
        let mut d = DecoCtx::new(f, r, random, logs, &empty, &empty, None);
        for decorator in list {
            decorator.place(&mut d);
        }
    }
}
