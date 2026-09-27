//! `GeodeFeature`: nested shells around a few random points, shaped by noise, with an optional
//! crack and crystal buds on the inner layer.

use super::{between_closed, field, opt, safe_set};
use crate::Error;
use crate::block_facts::{Dir, fluid};
use crate::blocks::{block_state, has_prop, is_air, is_water, state, with_prop};
use crate::json::Json;
use crate::noise::NormalNoiseParams;
use crate::pos::BlockPos;
use crate::providers::IntProvider;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::state_provider::StateProvider;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

#[derive(Debug)]
pub struct Geode {
    filling: StateProvider,
    inner_layer: StateProvider,
    alternate_inner_layer: StateProvider,
    middle_layer: StateProvider,
    outer_layer: StateProvider,
    inner_placements: Vec<u16>,
    cannot_replace: Arc<BlockSet>,
    invalid_blocks: Arc<BlockSet>,
    /// `GeodeLayerSettings`: filling, inner, middle, outer.
    layers: [f64; 4],
    generate_crack_chance: f64,
    base_crack_size: f64,
    crack_point_offset: i32,
    use_potential_placements_chance: f64,
    use_alternate_layer0_chance: f64,
    placements_require_layer0_alternate: bool,
    outer_wall_distance: IntProvider,
    distribution_points: IntProvider,
    point_offset: IntProvider,
    min_gen_offset: i32,
    max_gen_offset: i32,
    noise_multiplier: f64,
    invalid_blocks_threshold: i32,
}

fn int_provider(json: &Json, key: &str, min: i32, max: i32) -> Result<IntProvider, Error> {
    match json.get(key) {
        None => Ok(IntProvider::Uniform { min, max }),
        Some(v) => IntProvider::parse(v),
    }
}

impl Geode {
    pub fn parse(json: &Json, l: &Loader) -> Result<Geode, Error> {
        let blocks = field(json, "blocks")?;
        let provider = |k: &str| StateProvider::parse(field(blocks, k)?, l);
        let layers = field(json, "layers")?;
        let crack = field(json, "crack")?;
        let f64_or = |j: &Json, k: &str, d: f64| opt(j, k, d, Json::as_f64);
        Ok(Geode {
            filling: provider("filling_provider")?,
            inner_layer: provider("inner_layer_provider")?,
            alternate_inner_layer: provider("alternate_inner_layer_provider")?,
            middle_layer: provider("middle_layer_provider")?,
            outer_layer: provider("outer_layer_provider")?,
            inner_placements: field(blocks, "inner_placements")?
                .as_array()
                .ok_or_else(|| Error::Invalid("inner_placements must be a list".into()))?
                .iter()
                .map(block_state)
                .collect::<Result<_, _>>()?,
            cannot_replace: l.blocks(field(blocks, "cannot_replace")?)?,
            invalid_blocks: l.blocks(field(blocks, "invalid_blocks")?)?,
            layers: [
                f64_or(layers, "filling", 1.7)?,
                f64_or(layers, "inner_layer", 2.2)?,
                f64_or(layers, "middle_layer", 3.2)?,
                f64_or(layers, "outer_layer", 4.2)?,
            ],
            generate_crack_chance: f64_or(crack, "generate_crack_chance", 1.0)?,
            base_crack_size: f64_or(crack, "base_crack_size", 2.0)?,
            crack_point_offset: opt(crack, "crack_point_offset", 2, Json::as_i32)?,
            use_potential_placements_chance: f64_or(json, "use_potential_placements_chance", 0.35)?,
            use_alternate_layer0_chance: f64_or(json, "use_alternate_layer0_chance", 0.0)?,
            placements_require_layer0_alternate: opt(json, "placements_require_layer0_alternate", true, Json::as_bool)?,
            outer_wall_distance: int_provider(json, "outer_wall_distance", 4, 5)?,
            distribution_points: int_provider(json, "distribution_points", 3, 4)?,
            point_offset: int_provider(json, "point_offset", 1, 2)?,
            min_gen_offset: opt(json, "min_gen_offset", -16, Json::as_i32)?,
            max_gen_offset: opt(json, "max_gen_offset", 16, Json::as_i32)?,
            noise_multiplier: f64_or(json, "noise_multiplier", 0.05)?,
            invalid_blocks_threshold: crate::providers::int(json, "invalid_blocks_threshold")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let can_replace = |s: u16| !self.cannot_replace.contains(s);
        let points_n = self.distribution_points.sample(random);
        let noise = NormalNoiseParams::parity(-4, &[1.0]).create(&mut WorldgenRandom::legacy(r.seed()));
        let d = points_n as f64 / self.outer_wall_distance.max_value() as f64;
        let [filling, inner, middle, outer] = self.layers;
        let filling = 1.0 / filling.sqrt();
        let inner = 1.0 / (inner + d).sqrt();
        let middle = 1.0 / (middle + d).sqrt();
        let outer = 1.0 / (outer + d).sqrt();
        let crack_size = 1.0 / (self.base_crack_size + random.next_double() / 2.0 + if points_n > 3 { d } else { 0.0 }).sqrt();
        let crack = (random.next_float() as f64) < self.generate_crack_chance;
        let mut invalid = 0;
        let mut points = Vec::with_capacity(points_n.max(0) as usize);
        for _ in 0..points_n {
            let x = self.outer_wall_distance.sample(random);
            let y = self.outer_wall_distance.sample(random);
            let z = self.outer_wall_distance.sample(random);
            let p = origin.offset(x, y, z);
            let s = r.get(p);
            if is_air(s) || self.invalid_blocks.contains(s) {
                invalid += 1;
                if invalid > self.invalid_blocks_threshold {
                    return false;
                }
            }
            points.push((p, self.point_offset.sample(random) as f64));
        }
        let mut crack_points = Vec::new();
        if crack {
            let side = points_n * 2 + 1;
            let (x, z) = match random.next_int_bounded(4) {
                0 => (side, 0),
                1 => (0, side),
                2 => (side, side),
                _ => (0, 0),
            };
            crack_points.extend([7, 5, 1].map(|y| origin.offset(x, y, z)));
        }
        let crack_offset = self.crack_point_offset as f64;
        let mut potential = Vec::new();
        let (lo, hi) = (self.min_gen_offset, self.max_gen_offset);
        for p in between_closed(origin.offset(lo, lo, lo), origin.offset(hi, hi, hi)) {
            let n = noise.get3(p.x as f64, p.y as f64, p.z as f64) as f64 * self.noise_multiplier;
            let mut dist = 0.0;
            for &(q, offset) in &points {
                dist += inv_sqrt(p.dist_sqr(q) + offset) + n;
            }
            if dist < outer {
                continue;
            }
            if dist >= filling {
                let s = self.filling.state(r, random, p);
                safe_set(r, p, s, can_replace);
                continue;
            }
            let mut crack_dist = 0.0;
            for &q in &crack_points {
                crack_dist += inv_sqrt(p.dist_sqr(q) + crack_offset) + n;
            }
            if crack && crack_dist >= crack_size {
                safe_set(r, p, state::AIR, can_replace);
                for dir in Dir::ALL {
                    let q = p.relative(dir);
                    let f = r.fluid(q);
                    if !f.is_empty() {
                        r.schedule_fluid_tick(q, f.name(), 0);
                    }
                }
            } else if dist >= inner {
                let alternate = (random.next_float() as f64) < self.use_alternate_layer0_chance;
                let layer = if alternate { &self.alternate_inner_layer } else { &self.inner_layer };
                let s = layer.state(r, random, p);
                safe_set(r, p, s, can_replace);
                if (!self.placements_require_layer0_alternate || alternate)
                    && (random.next_float() as f64) < self.use_potential_placements_chance
                {
                    potential.push(p);
                }
            } else if dist >= middle {
                let s = self.middle_layer.state(r, random, p);
                safe_set(r, p, s, can_replace);
            } else {
                let s = self.outer_layer.state(r, random, p);
                safe_set(r, p, s, can_replace);
            }
        }
        for p in potential {
            let mut bud = self.inner_placements[random.next_int_bounded(self.inner_placements.len() as i32) as usize];
            for dir in Dir::ALL {
                if has_prop(bud, "facing") {
                    bud = with_prop(bud, "facing", dir.name());
                }
                let q = p.relative(dir);
                let there = r.get(q);
                if has_prop(bud, "waterlogged") {
                    bud = with_prop(bud, "waterlogged", if fluid(there).source { "true" } else { "false" });
                }
                if can_cluster_grow_at(there) {
                    safe_set(r, q, bud, can_replace);
                    break;
                }
            }
        }
        true
    }
}

/// `Mth.invSqrt(double)`.
fn inv_sqrt(x: f64) -> f64 {
    1.0 / x.sqrt()
}

/// `BuddingAmethystBlock.canClusterGrowAtState`: air or a full water block.
fn can_cluster_grow_at(s: u16) -> bool {
    is_air(s) || (is_water(s) && fluid(s).amount == 8)
}
