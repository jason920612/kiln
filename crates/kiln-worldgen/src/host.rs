//! Worldgen features for a running level: the [`FeatureHost`] kiln-blocks asks to place a tree,
//! a huge mushroom or a flower where something grew.
//!
//! A feature is placed exactly as generation places it (same code, same random draws), on a
//! copy of the level's blocks around the origin: the 3x3 chunks around the origin's chunk
//! (`WorldGenRegion`'s write radius) and the sections from one below the origin's to four
//! above it (80 or more blocks, more than any tree needs; the copy ends at the build height).
//! The copy is read through the level's `getBlockState`, so a tree sees the blocks that are
//! there. Everything the feature does to the copy is recorded ([`crate::region::RegionOp`]) and
//! handed back for the level to replay through its own `setBlock`, which makes block updates,
//! neighbour reactions and scheduled ticks the level's, not the copy's.
//!
//! Approximations: a feature that reads further than 80 blocks above the origin or a chunk
//! beyond the 3x3 finds `VOID_AIR` there (vanilla's level would answer with the real blocks);
//! `nextGaussian` draws on the level random lose their cached second value between calls.

use crate::blocks::state;
use crate::feature::Feature;
use crate::feature::trees::Kind as TreeKind;
use crate::feature::vegetation::Kind as VegKind;
use crate::generator::GenScratch;
use crate::pipeline::Worldgen;
use crate::proto::{Heightmap, ProtoChunk};
use crate::random::WorldgenRandom;
use crate::region::{Region, RegionOp};
use kiln_blocks::BlockPos as KPos;
use kiln_blocks::feature_host::{FeatureHost, FeatureOp, FeatureRef, Placed};
use kiln_data::block_props::has_block_entity;
use kiln_javamath::random::LegacyRandom;
use kiln_proto::nbt::Tag;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    /// Working memory of the region (biome lookups outside its chunks; features here have none).
    static SCRATCH: RefCell<GenScratch> = RefCell::new(GenScratch::default());
}

/// A level's worldgen, for what grows in it.
pub struct WorldgenHost {
    world: Arc<Worldgen>,
}

impl WorldgenHost {
    pub fn new(world: Arc<Worldgen>) -> Self {
        Self { world }
    }

    /// The copy of the blocks around `origin` as a region's nine proto-chunks.
    fn window(&self, origin: KPos, biome: u16, read: &mut dyn FnMut(KPos) -> u16) -> Vec<Box<ProtoChunk>> {
        let g = &self.world.generator;
        let (cx, cz) = (origin.x >> 4, origin.z >> 4);
        let (min_section, max_section) = (g.min_y >> 4, (g.min_y + g.height - 1) >> 4);
        let first = ((origin.y >> 4) - 1).max(min_section);
        let last = ((origin.y >> 4) + 4).min(max_section);
        let sections = (last - first + 1) as usize;
        let y0 = first * 16;
        let mut chunks = Vec::with_capacity(9);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (x0, z0) = ((cx + dx) << 4, (cz + dz) << 4);
                let mut c = ProtoChunk::new(cx + dx, cz + dz, y0, sections, vec![biome; sections * 64]);
                for y in y0..y0 + sections as i32 * 16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            let s = read(KPos::new(x0 + x as i32, y, z0 + z as i32));
                            if s != state::AIR {
                                c.set_raw(x, y, z, s);
                            }
                        }
                    }
                }
                // The heightmaps of a level that is being played in: every one primed from the blocks.
                c.finish_terrain();
                for z in 0..16 {
                    for x in 0..16 {
                        c.surface[(z << 4) | x] = c.height(Heightmap::WorldSurface, x, z);
                        c.ocean_floor[(z << 4) | x] = c.height(Heightmap::OceanFloor, x, z);
                    }
                }
                chunks.push(Box::new(c));
            }
        }
        chunks
    }
}

fn kpos(p: crate::pos::BlockPos) -> KPos {
    KPos::new(p.x, p.y, p.z)
}

impl FeatureHost for WorldgenHost {
    fn place(&self, feature: FeatureRef<'_>, origin: KPos, biome: Option<&str>, read: &mut dyn FnMut(KPos) -> u16, random: &mut LegacyRandom) -> Placed {
        let w = &*self.world;
        let features = &w.decorator.features;
        let configured = match feature {
            FeatureRef::Configured(name) => features.feature_id(name).map(Ok),
            FeatureRef::Placed(name) => features.placed_id(name).map(Err),
        };
        let Some(which) = configured else { return Placed::default() };
        let biome = biome.and_then(|b| w.generator.biomes.iter().position(|x| x.name == b)).unwrap_or(0) as u16;
        let chunks = self.window(origin, biome, read);
        let at = crate::pos::BlockPos::new(origin.x, origin.y, origin.z);
        SCRATCH.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            let mut region = Region::new(chunks, origin.x >> 4, origin.z >> 4, &w.generator, &mut scratch);
            region.start_ops();
            let mut wr = WorldgenRandom::from_legacy(std::mem::replace(random, LegacyRandom::new(0)));
            let ok = match which {
                Ok(id) => features.place_feature(id, &mut region, &mut wr, at),
                Err(id) => features.place_placed(id, &mut region, &mut wr, at, false),
            };
            *random = wr.into_legacy().expect("a legacy random stays legacy");
            let ops = region.take_ops();
            let mut out: Vec<FeatureOp> = Vec::with_capacity(ops.len());
            let mut entities: Vec<crate::pos::BlockPos> = Vec::new();
            for op in &ops {
                out.push(match *op {
                    RegionOp::Set(p, s, f) => {
                        if has_block_entity(s) && !entities.contains(&p) {
                            entities.push(p);
                        }
                        FeatureOp::Set { pos: kpos(p), state: s, flags: f as u32 }
                    }
                    RegionOp::BlockTick(p, block, delay) => FeatureOp::BlockTick { pos: kpos(p), block, delay },
                    RegionOp::FluidTick(p, fluid, delay) => FeatureOp::FluidTick { pos: kpos(p), fluid, delay },
                });
            }
            // What features filled into block entities (a bee nest's bees), as it stands at the end.
            for p in entities {
                if let Some(Tag::Compound(fields)) = region.block_entity_mut(p).map(|t| t.clone()) {
                    let data: Vec<_> = fields.into_iter().filter(|(k, _)| k != "id").collect();
                    if !data.is_empty() {
                        out.push(FeatureOp::BlockEntity { pos: kpos(p), data: Tag::Compound(data) });
                    }
                }
            }
            Placed { ok, ops: out }
        })
    }

    fn bone_meal_features(&self, biome: &str) -> Vec<String> {
        let w = &*self.world;
        let Some(i) = w.generator.biomes.iter().position(|b| b.name == biome) else { return Vec::new() };
        w.decorator.bone_meal[i].iter().map(|&f| w.decorator.features.configured[f].name.clone()).collect()
    }

    fn tree_base_height(&self, feature: &str) -> Option<i32> {
        let f = &self.world.decorator.features;
        match &f.configured[f.feature_id(feature)?].feature {
            Feature::Trees(TreeKind::Tree(t)) => Some(t.base_height()),
            _ => None,
        }
    }

    fn huge_mushroom_radius(&self, feature: &str) -> Option<i32> {
        let f = &self.world.decorator.features;
        match &f.configured[f.feature_id(feature)?].feature {
            Feature::Vegetation(VegKind::HugeBrownMushroom(m) | VegKind::HugeRedMushroom(m)) => Some(m.foliage_radius()),
            _ => None,
        }
    }
}
