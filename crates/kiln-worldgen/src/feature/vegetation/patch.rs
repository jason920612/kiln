//! `VegetationPatchFeature` and `WaterloggedVegetationPatchFeature`: a patch of ground blocks
//! on a cave floor or ceiling with vegetation placed on it.

use super::jhash::PosSet;
use super::{field, placed, state_provider};
use crate::Error;
use crate::blocks::{has_prop, is_air, prop, same_block, with_prop};
use crate::block_facts::{Dir, Support, is_face_sturdy};
use crate::feature::Features;
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
pub struct VegetationPatch {
    replaceable: Arc<BlockSet>,
    ground: StateProvider,
    vegetation: usize,
    /// `CaveSurface.getDirection`: down for the floor, up for the ceiling.
    surface: Dir,
    depth: IntProvider,
    extra_bottom_block_chance: f32,
    vertical_range: i32,
    vegetation_chance: f32,
    xz_radius: IntProvider,
    extra_edge_column_chance: f32,
    waterlogged: bool,
}

impl VegetationPatch {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader, waterlogged: bool) -> Result<Self, Error> {
        let surface = match field(json, "surface")?.as_str() {
            Some("floor") => Dir::Down,
            Some("ceiling") => Dir::Up,
            s => return Err(Error::Invalid(format!("bad cave surface {s:?}"))),
        };
        Ok(Self {
            replaceable: l.blocks(field(json, "replaceable")?)?,
            ground: state_provider(json, "ground_state", l)?,
            vegetation: placed(json, "vegetation_feature", f, l)?,
            surface,
            depth: IntProvider::parse(field(json, "depth")?)?,
            extra_bottom_block_chance: float(json, "extra_bottom_block_chance")?,
            vertical_range: int(json, "vertical_range")?,
            vegetation_chance: float(json, "vegetation_chance")?,
            xz_radius: IntProvider::parse(field(json, "xz_radius")?)?,
            extra_edge_column_chance: float(json, "extra_edge_column_chance")?,
            waterlogged,
        })
    }

    pub fn nested(&self) -> Vec<usize> {
        vec![self.vegetation]
    }

    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let rx = self.xz_radius.sample(random) + 1;
        let rz = self.xz_radius.sample(random) + 1;
        let mut ground = self.place_ground_patch(r, random, origin, rx, rz);
        if self.waterlogged {
            ground = waterlog(r, ground);
        }
        for p in ground.iter_order() {
            if self.vegetation_chance > 0.0 && random.next_float() < self.vegetation_chance {
                self.place_vegetation(f, r, random, p);
            }
        }
        !ground.is_empty()
    }

    /// `placeGroundPatch`.
    fn place_ground_patch(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, rx: i32, rz: i32) -> PosSet {
        let inward = self.surface;
        let outward = inward.opposite();
        let mut out = PosSet::new();
        for dx in -rx..=rx {
            let edge_x = dx == -rx || dx == rx;
            for dz in -rz..=rz {
                let edge_z = dz == -rz || dz == rz;
                let corner = edge_x && edge_z;
                let edge = (edge_x || edge_z) && !corner;
                if corner {
                    continue;
                }
                if edge && (self.extra_edge_column_chance == 0.0 || random.next_float() > self.extra_edge_column_chance) {
                    continue;
                }
                let mut p = origin.offset(dx, 0, dz);
                let mut i = 0;
                while is_air(r.get(p)) && i < self.vertical_range {
                    p = p.relative(inward);
                    i += 1;
                }
                let mut i = 0;
                while !is_air(r.get(p)) && i < self.vertical_range {
                    p = p.relative(outward);
                    i += 1;
                }
                let below = p.relative(inward);
                let below_state = r.get(below);
                if is_air(r.get(p)) && is_face_sturdy(below_state, outward, Support::Full) {
                    let depth = self.depth.sample(random);
                    let extra = self.extra_bottom_block_chance > 0.0 && random.next_float() < self.extra_bottom_block_chance;
                    let depth = depth + extra as i32;
                    if self.place_ground(r, random, below, depth) {
                        out.insert(below);
                    }
                }
            }
        }
        out
    }

    /// `placeGround`: `depth` ground blocks from `p` into the surface.
    fn place_ground(&self, r: &mut Region, random: &mut WorldgenRandom, mut p: BlockPos, depth: i32) -> bool {
        for i in 0..depth {
            let s = self.ground.state(r, random, p);
            let here = r.get(p);
            if same_block(s, here) {
                continue;
            }
            if !self.replaceable.contains(here) {
                return i != 0;
            }
            r.set(p, s, 2);
            p = p.relative(self.surface);
        }
        true
    }

    /// `placeVegetation`.
    fn place_vegetation(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        if !self.waterlogged {
            return f.place_placed(self.vegetation, r, random, p.relative(self.surface.opposite()), false);
        }
        if !f.place_placed(self.vegetation, r, random, p.below().relative(self.surface.opposite()), false) {
            return false;
        }
        let s = r.get(p);
        if has_prop(s, "waterlogged") && prop(s, "waterlogged") == Some("false") {
            r.set(p, with_prop(s, "waterlogged", "true"), 2);
        }
        true
    }
}

/// `WaterloggedVegetationPatchFeature.placeGroundPatch`: the ground positions enclosed on all
/// sides and below become water.
fn waterlog(r: &mut Region, ground: PosSet) -> PosSet {
    let mut kept = PosSet::new();
    for p in ground.iter_order() {
        let exposed = [Dir::North, Dir::East, Dir::South, Dir::West, Dir::Down]
            .into_iter()
            .any(|d| !is_face_sturdy(r.get(p.relative(d)), d.opposite(), Support::Full));
        if !exposed {
            kept.insert(p);
        }
    }
    for p in kept.iter_order() {
        r.set(p, crate::blocks::state::WATER, 2);
    }
    kept
}
