//! `RootPlacer`s (only `MangroveRootPlacer` exists): roots grown below and around the trunk.

use super::tree::{Ctx, Part, valid_tree_pos};
use super::field;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{has_prop, is_air, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::{IntProvider, float, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

#[derive(Debug)]
pub struct RootPlacer {
    trunk_offset_y: IntProvider,
    root_provider: StateProvider,
    /// `AboveRootPlacement`: provider and chance.
    above: Option<(StateProvider, f32)>,
    can_grow_through: Arc<BlockSet>,
    muddy_roots_in: Arc<BlockSet>,
    muddy_roots_provider: StateProvider,
    max_root_width: i32,
    max_root_length: i32,
    random_skew_chance: f32,
}

impl RootPlacer {
    pub fn parse(json: &Json, l: &Loader) -> Result<RootPlacer, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        if ty.strip_prefix("minecraft:").unwrap_or(ty) != "mangrove_root_placer" {
            return Err(Error::Invalid(format!("unknown root placer {ty}")));
        }
        let above = match json.get("above_root_placement") {
            Some(a) => Some((StateProvider::parse(field(a, "above_root_provider")?, l)?, float(a, "above_root_placement_chance")?)),
            None => None,
        };
        let m = field(json, "mangrove_root_placement")?;
        Ok(RootPlacer {
            trunk_offset_y: IntProvider::parse(field(json, "trunk_offset_y")?)?,
            root_provider: StateProvider::parse(field(json, "root_provider")?, l)?,
            above,
            can_grow_through: l.blocks(field(m, "can_grow_through")?)?,
            muddy_roots_in: l.blocks(field(m, "muddy_roots_in")?)?,
            muddy_roots_provider: StateProvider::parse(field(m, "muddy_roots_provider")?, l)?,
            max_root_width: int(m, "max_root_width")?,
            max_root_length: int(m, "max_root_length")?,
            random_skew_chance: float(m, "random_skew_chance")?,
        })
    }

    /// `RootPlacer.getTrunkOrigin`.
    pub fn trunk_origin(&self, origin: BlockPos, random: &mut WorldgenRandom) -> BlockPos {
        origin.above_n(self.trunk_offset_y.sample(random))
    }

    /// `MangroveRootPlacer.canPlaceRoot`.
    fn can_place_root(&self, r: &mut Region, p: BlockPos) -> bool {
        valid_tree_pos(r, p) || self.can_grow_through.contains(r.get(p))
    }

    /// `MangroveRootPlacer.placeRoots`.
    pub fn place_roots(&self, cx: &mut Ctx, origin: BlockPos, trunk_origin: BlockPos) -> bool {
        let mut positions = Vec::new();
        let mut p = origin;
        while p.y < trunk_origin.y {
            if !self.can_place_root(cx.r, p) {
                return false;
            }
            p = p.above();
        }
        positions.push(trunk_origin.below());
        for d in Dir::HORIZONTAL {
            let start = trunk_origin.relative(d);
            let mut roots = Vec::new();
            if !self.simulate(cx.r, cx.random, start, d, trunk_origin, &mut roots, 0) {
                return false;
            }
            positions.extend(roots);
            positions.push(trunk_origin.relative(d));
        }
        for p in positions {
            self.place_root(cx, p);
        }
        true
    }

    /// `MangroveRootPlacer.simulateRoots`.
    #[allow(clippy::too_many_arguments)]
    fn simulate(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, d: Dir, trunk: BlockPos, out: &mut Vec<BlockPos>, depth: i32) -> bool {
        if depth == self.max_root_length || out.len() as i32 > self.max_root_length {
            return false;
        }
        for q in self.potential_positions(p, d, random, trunk) {
            if self.can_place_root(r, q) {
                out.push(q);
                if !self.simulate(r, random, q, d, trunk, out, depth + 1) {
                    return false;
                }
            }
        }
        true
    }

    /// `MangroveRootPlacer.potentialRootPositions`.
    fn potential_positions(&self, p: BlockPos, d: Dir, random: &mut WorldgenRandom, trunk: BlockPos) -> Vec<BlockPos> {
        let below = p.below();
        let side = p.relative(d);
        let dist = p.dist_manhattan(trunk);
        let width = self.max_root_width;
        if dist > width - 3 && dist <= width {
            return if random.next_float() < self.random_skew_chance { vec![below, side.below()] } else { vec![below] };
        }
        if dist > width {
            return vec![below];
        }
        if random.next_float() < self.random_skew_chance {
            return vec![below];
        }
        if random.next_bool() { vec![side] } else { vec![below] }
    }

    /// `MangroveRootPlacer.placeRoot`.
    fn place_root(&self, cx: &mut Ctx, p: BlockPos) {
        if self.muddy_roots_in.contains(cx.r.get(p)) {
            let s = self.muddy_roots_provider.state(cx.r, cx.random, p);
            let s = waterlogged(cx.r, p, s);
            cx.put(Part::Roots, p, s);
            return;
        }
        if !self.can_place_root(cx.r, p) {
            return;
        }
        let s = self.root_provider.state(cx.r, cx.random, p);
        let s = waterlogged(cx.r, p, s);
        cx.put(Part::Roots, p, s);
        if let Some((provider, chance)) = &self.above {
            let up = p.above();
            if cx.random.next_float() < *chance && is_air(cx.r.get(up)) {
                let s = provider.state(cx.r, cx.random, up);
                let s = waterlogged(cx.r, up, s);
                cx.put(Part::Roots, up, s);
            }
        }
    }
}

/// `RootPlacer.getPotentiallyWaterloggedState`.
fn waterlogged(r: &mut Region, p: BlockPos, s: u16) -> u16 {
    if has_prop(s, "waterlogged") {
        let water = r.fluid(p).is_water();
        with_prop(s, "waterlogged", if water { "true" } else { "false" })
    } else {
        s
    }
}
