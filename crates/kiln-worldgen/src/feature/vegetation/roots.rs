//! `RootSystemFeature`: a tree on the surface above a cave, rooted dirt down its column and
//! hanging roots below.

use super::{field, placed, state_provider};
use crate::Error;
use crate::block_facts::{Dir, Support, fluid, is_face_sturdy, is_solid};
use crate::blocks::is_air;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::proto::Heightmap;
use crate::providers::int;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

#[derive(Debug)]
pub struct RootSystem {
    tree: usize,
    required_vertical_space: i32,
    level_test_distance: i32,
    max_level_deviation: i32,
    root_radius: i32,
    root_replaceable: Arc<BlockSet>,
    root_state: StateProvider,
    root_attempts: i32,
    root_column_max_height: i32,
    hanging_radius: i32,
    hanging_span: i32,
    hanging_state: StateProvider,
    hanging_attempts: i32,
    allowed_vertical_water: i32,
    allowed_tree_position: BlockPredicate,
}

impl RootSystem {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            tree: placed(json, "feature", f, l)?,
            required_vertical_space: int(json, "required_vertical_space_for_tree")?,
            level_test_distance: int(json, "level_test_distance")?,
            max_level_deviation: int(json, "max_level_deviation")?,
            root_radius: int(json, "root_radius")?,
            root_replaceable: l.blocks(field(json, "root_replaceable")?)?,
            root_state: state_provider(json, "root_state_provider", l)?,
            root_attempts: int(json, "root_placement_attempts")?,
            root_column_max_height: int(json, "root_column_max_height")?,
            hanging_radius: int(json, "hanging_root_radius")?,
            hanging_span: int(json, "hanging_roots_vertical_span")?,
            hanging_state: state_provider(json, "hanging_root_state_provider", l)?,
            hanging_attempts: int(json, "hanging_root_placement_attempts")?,
            allowed_vertical_water: int(json, "allowed_vertical_water_for_tree")?,
            allowed_tree_position: BlockPredicate::parse(field(json, "allowed_tree_position")?, l)?,
        })
    }

    pub fn nested(&self) -> Vec<usize> {
        vec![self.tree]
    }

    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !is_air(r.get(origin)) {
            return false;
        }
        if self.place_dirt_and_tree(f, r, random, origin) {
            self.place_roots(r, random, origin);
        }
        true
    }

    /// `spaceForTree`.
    fn space_for_tree(&self, r: &mut Region, p: BlockPos) -> bool {
        for i in 1..=self.required_vertical_space {
            let s = r.get(p.above_n(i));
            let allowed = is_air(s) || (i + 1 <= self.allowed_vertical_water && fluid(s).is_water());
            if !allowed {
                return false;
            }
        }
        if self.level_test_distance > 0 {
            for i in 0..4 {
                let q = p.relative_n(Dir::from_2d(i), self.level_test_distance);
                if !is_air(r.get(q.below_n(self.max_level_deviation))) || !is_air(r.get(q.above_n(self.max_level_deviation))) {
                    return false;
                }
            }
        }
        true
    }

    /// `placeDirtAndTree`.
    fn place_dirt_and_tree(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut p = origin;
        for i in 0..self.root_column_max_height {
            p = p.above();
            if r.height_at(Heightmap::WorldSurface, p.x, p.z) < p.y {
                return false;
            }
            if self.allowed_tree_position.test(r, p) && self.space_for_tree(r, p) {
                let below = r.get(p.below());
                if fluid(below).is_lava() || !is_solid(below) {
                    return false;
                }
                if f.place_placed(self.tree, r, random, p, false) {
                    self.place_dirt(r, random, origin, origin.y + i);
                    return true;
                }
            }
        }
        false
    }

    /// `placeDirt`: rooted dirt around each block of the column below the tree.
    fn place_dirt(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, top: i32) {
        for y in origin.y..top {
            for _ in 0..self.root_attempts {
                let dx = random.next_int_bounded(self.root_radius) - random.next_int_bounded(self.root_radius);
                let dz = random.next_int_bounded(self.root_radius) - random.next_int_bounded(self.root_radius);
                let p = BlockPos::new(origin.x + dx, y, origin.z + dz);
                if self.root_replaceable.contains(r.get(p)) {
                    let s = self.root_state.state(r, random, p);
                    r.set(p, s, 2);
                }
            }
        }
    }

    /// `placeRoots`: hanging roots under the origin.
    fn place_roots(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) {
        for _ in 0..self.hanging_attempts {
            let dx = random.next_int_bounded(self.hanging_radius) - random.next_int_bounded(self.hanging_radius);
            let dy = random.next_int_bounded(self.hanging_span) - random.next_int_bounded(self.hanging_span);
            let dz = random.next_int_bounded(self.hanging_radius) - random.next_int_bounded(self.hanging_radius);
            let p = origin.offset(dx, dy, dz);
            if !is_air(r.get(p)) {
                continue;
            }
            let s = self.hanging_state.state(r, random, p);
            if crate::survive::can_survive(s, r, p) && is_face_sturdy(r.get(p.above()), Dir::Down, Support::Full) {
                r.set(p, s, 2);
            }
        }
    }
}
