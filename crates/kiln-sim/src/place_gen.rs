//! `/place feature`, `/place structure` and `/place jigsaw`: the generator's feature and
//! structure placement run over the live chunks.
//!
//! Vanilla hands the `ServerLevel` itself to `Feature.place` and `StructurePiece.postProcess`.
//! Kiln's placement works on a [`Region`] of proto-chunks, so the loaded chunks around the spot
//! are copied into one, the placement runs there, and every block it changed (and the block
//! entity data and entities that came with them) is written back with the level's own block
//! machinery, in the chunks that are loaded. A placement that fails halfway keeps the blocks
//! it already placed, as vanilla's does.

use crate::{DimId, Sim};
use kiln_command::{CommandError, Host, UpdateFlags, tr};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_world::{Blocks, ChunkPos};
use kiln_world::section::BlockContainer;
use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::pipeline::Worldgen;
use kiln_worldgen::pos::BlockPos;
use kiln_worldgen::proto::ProtoChunk;
use kiln_worldgen::random::WorldgenRandom;
use kiln_worldgen::region::Region;
use kiln_worldgen::structure::bbox::BoundingBox;
use kiln_worldgen::structure::piece::PlaceContext;
use std::collections::HashSet;
use std::sync::Arc;

impl Sim {
    /// A random source seeded from the level random (`level.getRandom()`).
    fn level_worldgen_random(&mut self) -> WorldgenRandom {
        WorldgenRandom::legacy(self.world.level_random.next_long())
    }

    /// The loaded chunk `(cx, cz)` of `dim` as a proto-chunk (an empty one when it is not
    /// loaded), with the generator's biomes.
    fn live_proto(&self, dim: DimId, g: &kiln_worldgen::generator::Generator, gs: &mut GenScratch, cx: i32, cz: i32) -> (Box<ProtoChunk>, bool) {
        let biomes = gs.chunk_biomes(g, cx, cz).to_vec();
        let mut p = ProtoChunk::new(cx, cz, g.min_y, g.sections(), biomes);
        let live = self.dims[dim].regions.chunk(ChunkPos::new(cx, cz));
        if let Some(chunk) = live {
            for (si, section) in chunk.sections.iter().enumerate().take(g.sections()) {
                if matches!(section.blocks, BlockContainer::Single(s) if s == kiln_data::blocks::default_state::AIR) {
                    continue;
                }
                let base = g.min_y + si as i32 * 16;
                for i in 0..4096 {
                    let s = section.blocks.get(i);
                    if s != kiln_data::blocks::default_state::AIR {
                        // (`set` keeps the generation heightmaps, which `finish_terrain` then primes.)
                        p.set(i & 15, base + (i >> 8) as i32, (i >> 4) & 15, s);
                    }
                }
            }
        }
        p.finish_terrain();
        (Box::new(p), live.is_some())
    }

    /// Runs `f` over a window of `(2 * radius + 1)` chunks around `center` and writes what it
    /// changed back into the loaded ones. `None` when the level has no generator.
    fn with_live_region<R>(&mut self, dim: DimId, center: (i32, i32), radius: i32, flags: u32, f: impl FnOnce(&mut Region, &Worldgen) -> R) -> Option<R> {
        let pipeline = self.world.pipelines.get(dim).cloned().flatten()?;
        let world: Arc<Worldgen> = pipeline.world().clone();
        self.with_live_region_in(dim, center, radius, flags, &world.generator, |r| f(r, &world))
    }

    /// [`Self::with_live_region`] over a generator of the caller's (structure blocks place templates in levels that generate no
    /// terrain of their own).
    pub(crate) fn with_live_region_in<R>(
        &mut self,
        dim: DimId,
        center: (i32, i32),
        radius: i32,
        flags: u32,
        g: &kiln_worldgen::generator::Generator,
        f: impl FnOnce(&mut Region) -> R,
    ) -> Option<R> {
        let mut gs = GenScratch::default();
        let (cx, cz) = center;
        let mut chunks = Vec::new();
        let mut loaded = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let (p, is_loaded) = self.live_proto(dim, g, &mut gs, cx + dx, cz + dz);
                chunks.push(p);
                loaded.push(is_loaded);
            }
        }
        let mut region = Region::with_radius(chunks, cx, cz, radius, g, &mut gs);
        region.start_log();
        let out = f(&mut region);
        let log = region.take_log();
        let chunks = region.into_chunks();
        let w = 2 * radius + 1;
        let dimension = crate::DIMENSIONS[dim].0;
        let mut seen: HashSet<(i32, i32, i32)> = HashSet::new();
        for (pos, old) in log {
            if !seen.insert((pos.x, pos.y, pos.z)) {
                continue;
            }
            let (dx, dz) = ((pos.x >> 4) - (cx - radius), (pos.z >> 4) - (cz - radius));
            if !(0..w).contains(&dx) || !(0..w).contains(&dz) {
                continue;
            }
            let i = (dz * w + dx) as usize;
            if !loaded[i] {
                continue;
            }
            let chunk = &chunks[i];
            let (lx, lz) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
            let new = chunk.get(lx, pos.y, lz);
            let key = (((pos.y - chunk.min_y) as u32) << 8) | ((lz as u32) << 4) | lx as u32;
            let nbt = chunk.block_entities.get(&key).map(|t| match t {
                // Generation leaves `id: DUMMY` placeholders for the block's own entity.
                Tag::Compound(fields) if fields.iter().any(|(k, v)| k == "id" && v.as_str() == Some("DUMMY")) => {
                    Tag::Compound(fields.iter().filter(|(k, _)| k != "id").cloned().collect())
                }
                other => other.clone(),
            });
            if new == old && nbt.is_none() {
                continue;
            }
            Host::set_block(self, dimension, [pos.x, pos.y, pos.z], new, nbt.as_ref(), UpdateFlags(flags));
        }
        for (i, chunk) in chunks.iter().enumerate() {
            if loaded[i] && !chunk.entities.is_empty() {
                let at = ChunkPos::new(chunk.x, chunk.z);
                self.dims[dim].add_saved_entities(at, chunk.entities.clone());
            }
        }
        Some(out)
    }

    /// `/place feature`: the configured feature `id` placed at `pos` with the level random.
    pub(crate) fn place_generated_feature(&mut self, dim: DimId, id: Option<&str>, pos: [i32; 3]) -> Result<(), CommandError> {
        let Some(id) = id else { return Err(CommandError::unsupported("place feature (inline)")) };
        let Some(pipeline) = self.world.pipelines.get(dim).cloned().flatten() else { return Err(CommandError::unsupported("place feature")) };
        let Some(feature) = pipeline.world().decorator.features.feature_id(id) else { return Err(CommandError::new(tr!("commands.place.feature.failed"))) };
        let mut random = self.level_worldgen_random();
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let placed = self
            .with_live_region(dim, (pos[0] >> 4, pos[2] >> 4), 1, 3, |r, w| w.decorator.features.place_feature(feature, r, &mut random, at))
            .unwrap_or(false);
        if placed { Ok(()) } else { Err(CommandError::new(tr!("commands.place.feature.failed"))) }
    }

    /// `/place structure`: the structure's start generated from the chunk holding `pos` (any
    /// biome is good), every chunk its box touches loaded, its pieces placed chunk by chunk.
    pub(crate) fn place_generated_structure(&mut self, dim: DimId, id: &str, pos: [i32; 3]) -> Result<(), CommandError> {
        let Some(pipeline) = self.world.pipelines.get(dim).cloned().flatten() else { return Err(CommandError::unsupported("place structure")) };
        let world = pipeline.world().clone();
        let failed = || CommandError::new(tr!("commands.place.structure.failed"));
        let Some(st) = world.structures.id(id) else { return Err(failed()) };
        let mut gs = GenScratch::default();
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let Some(start) = world.structures.generate_anywhere(&world.generator, &mut gs.structures, st, cx, cz) else { return Err(failed()) };
        let Some(b) = kiln_worldgen::structure::pieces_bbox(&start.pieces) else { return Err(failed()) };
        let (min, max) = ((b.min_x >> 4, b.min_z >> 4), (b.max_x >> 4, b.max_z >> 4));
        let dimension = crate::DIMENSIONS[dim].0;
        for z in min.1..=max.1 {
            for x in min.0..=max.0 {
                if !Host::is_chunk_loaded(self, dimension, x, z) {
                    return Err(CommandError::pos_unloaded());
                }
            }
        }
        let mut random = self.level_worldgen_random();
        for z in min.1..=max.1 {
            for x in min.0..=max.0 {
                self.with_live_region(dim, (x, z), 1, 2, |r, w| {
                    let cx = PlaceContext { structures: &w.structures, generator: &w.generator, features: Some(&w.decorator.features) };
                    let chunk_box = BoundingBox::new(x << 4, r.min_y(), z << 4, (x << 4) + 15, r.max_y() + 1, (z << 4) + 15);
                    start.place_in_chunk(&cx, r, &mut random, &chunk_box, (x, z));
                });
            }
        }
        Ok(())
    }

    /// `/place jigsaw`: pieces grown from `pool` at `pos` (their `target` jigsaw on it) to
    /// `max_depth`, all placed at once, in the loaded chunks they reach.
    pub(crate) fn place_generated_jigsaw(&mut self, dim: DimId, pool: &str, target: &str, max_depth: i32, pos: [i32; 3], keep_jigsaws: bool) -> Result<(), CommandError> {
        let Some(pipeline) = self.world.pipelines.get(dim).cloned().flatten() else { return Err(CommandError::unsupported("place jigsaw")) };
        let world = pipeline.world().clone();
        let failed = || CommandError::new(tr!("commands.place.jigsaw.failed"));
        let mut gs = GenScratch::default();
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let Some(pieces) = world.structures.generate_jigsaw(&world.generator, &mut gs.structures, pool, target, max_depth, at, keep_jigsaws) else {
            return Err(failed());
        };
        let mut b = pieces[0].base().bbox;
        for p in &pieces {
            b.encapsulate(&p.base().bbox);
        }
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let reach = |lo: i32, hi: i32, c: i32| (c - (lo >> 4)).abs().max(((hi >> 4) - c).abs());
        let radius = (reach(b.min_x, b.max_x, cx).max(reach(b.min_z, b.max_z, cz)) + 1).min(12);
        let mut random = self.level_worldgen_random();
        self.with_live_region(dim, (cx, cz), radius, 2, |r, w| {
            let cxt = PlaceContext { structures: &w.structures, generator: &w.generator, features: Some(&w.decorator.features) };
            for p in &pieces {
                p.place(&cxt, r, &mut random, &BoundingBox::infinite(), (cx, cz), at);
            }
        });
        Ok(())
    }
}
