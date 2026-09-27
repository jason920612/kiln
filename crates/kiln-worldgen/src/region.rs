//! The world as features see it while a chunk is decorated (`WorldGenRegion`): the chunk and
//! its eight neighbours, owned for the duration of the step.
//!
//! Features may write into all nine chunks (FEATURES has write radius 1). Vanilla lets them
//! read up to eight chunks away; nothing in vanilla's data does, and doing so would make the
//! result depend on generation order, so reads outside the 3×3 window return `VOID_AIR` and are
//! counted in [`RegionStats`], like writes outside it (which vanilla also refuses).

use crate::block_facts::{Fluid, fluid};
use crate::blocks::state;
use crate::generator::{GenScratch, Generator, zoomed_biome};
use crate::pos::BlockPos;
use crate::proto::{GenTick, Heightmap, ProtoChunk};
use kiln_data::block_props::has_block_entity;
use kiln_javamath::random::WorldgenRandom as PositionalRandom;
use kiln_proto::nbt::Tag;

/// Accesses outside the 3×3 window, which are not vanilla-compatible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionStats {
    pub far_reads: u64,
    pub far_writes: u64,
}

/// `WorldGenRegion` over nine owned proto-chunks.
pub struct Region<'a> {
    // Boxed: chunks move between the pipeline and regions without copying their arrays.
    #[allow(clippy::vec_box)]
    chunks: Vec<Box<ProtoChunk>>,
    pub cx: i32,
    pub cz: i32,
    pub generator: &'a Generator,
    scratch: &'a mut GenScratch,
    pub stats: RegionStats,
    log: Option<Vec<(BlockPos, u16)>>,
    level_random: Option<PositionalRandom>,
}

impl<'a> Region<'a> {
    /// `chunks` are the 3×3 chunks around `(cx, cz)`, row by row from the north-west
    /// (`index = (dz + 1) * 3 + dx + 1`).
    pub fn new(chunks: Vec<Box<ProtoChunk>>, cx: i32, cz: i32, generator: &'a Generator, scratch: &'a mut GenScratch) -> Self {
        assert_eq!(chunks.len(), 9, "a region holds the 3x3 chunks around its center");
        for (i, c) in chunks.iter().enumerate() {
            debug_assert_eq!((c.x, c.z), (cx + i as i32 % 3 - 1, cz + i as i32 / 3 - 1), "chunk {i} out of place");
        }
        Self { chunks, cx, cz, generator, scratch, stats: RegionStats::default(), log: None, level_random: None }
    }

    pub fn into_chunks(self) -> Vec<Box<ProtoChunk>> {
        self.chunks
    }

    #[inline]
    fn slot(&self, cx: i32, cz: i32) -> Option<usize> {
        let (dx, dz) = (cx - self.cx, cz - self.cz);
        (dx.abs() <= 1 && dz.abs() <= 1).then(|| ((dz + 1) * 3 + dx + 1) as usize)
    }

    pub fn chunk(&self, cx: i32, cz: i32) -> Option<&ProtoChunk> {
        self.slot(cx, cz).map(|i| &*self.chunks[i])
    }

    pub fn chunk_mut(&mut self, cx: i32, cz: i32) -> Option<&mut ProtoChunk> {
        self.slot(cx, cz).map(|i| &mut *self.chunks[i])
    }

    pub fn center(&self) -> &ProtoChunk {
        &self.chunks[4]
    }

    pub fn chunks(&self) -> &[Box<ProtoChunk>] {
        &self.chunks
    }

    /// Starts recording `(position, old state)` of every changed block.
    pub fn start_log(&mut self) {
        self.log = Some(Vec::new());
    }

    /// The changes recorded since [`Region::start_log`] (or the last call), oldest first.
    pub fn take_log(&mut self) -> Vec<(BlockPos, u16)> {
        self.log.as_mut().map(std::mem::take).unwrap_or_default()
    }

    pub fn min_y(&self) -> i32 {
        self.generator.min_y
    }

    /// `LevelHeightAccessor.getMaxY`: the highest y inside the build height.
    pub fn max_y(&self) -> i32 {
        self.generator.min_y + self.generator.height - 1
    }

    pub fn height(&self) -> i32 {
        self.generator.height
    }

    pub fn sea_level(&self) -> i32 {
        self.generator.sea_level
    }

    /// The level seed (`WorldGenLevel.getSeed`).
    pub fn seed(&self) -> i64 {
        self.generator.seed
    }

    pub fn is_outside_build_height(&self, y: i32) -> bool {
        y < self.min_y() || y > self.max_y()
    }

    /// `getBlockState`.
    #[inline]
    pub fn get(&mut self, p: BlockPos) -> u16 {
        match self.slot(p.x >> 4, p.z >> 4) {
            Some(i) => self.chunks[i].get((p.x & 15) as usize, p.y, (p.z & 15) as usize),
            None => {
                self.stats.far_reads += 1;
                state::VOID_AIR
            }
        }
    }

    /// `getFluidState`.
    pub fn fluid(&mut self, p: BlockPos) -> Fluid {
        fluid(self.get(p))
    }

    /// `isStateAtPosition(pos, BlockState::isAir)`.
    pub fn is_air(&mut self, p: BlockPos) -> bool {
        crate::blocks::is_air(self.get(p))
    }

    /// `ensureCanWrite`.
    pub fn can_write(&self, p: BlockPos) -> bool {
        self.slot(p.x >> 4, p.z >> 4).is_some()
    }

    /// `WorldGenRegion.setBlock(pos, state, flags)`. Returns false when the position is
    /// outside the write radius.
    pub fn set(&mut self, p: BlockPos, s: u16, flags: i32) -> bool {
        let Some(i) = self.slot(p.x >> 4, p.z >> 4) else {
            self.stats.far_writes += 1;
            return false;
        };
        let chunk = &mut self.chunks[i];
        let (lx, lz) = ((p.x & 15) as usize, (p.z & 15) as usize);
        let before = chunk.get(lx, p.y, lz);
        let old = chunk.set(lx, p.y, lz, s);
        let inside = !(p.y < chunk.min_y || p.y > chunk.max_y());
        if inside {
            let key = (((p.y - chunk.min_y) as u32) << 8) | ((lz as u32) << 4) | lx as u32;
            if has_block_entity(s) {
                chunk.block_entities.insert(key, Tag::Compound(vec![("id".into(), Tag::String("DUMMY".into()))]));
            } else if has_block_entity(old) {
                chunk.block_entities.remove(&key);
            }
        }
        let after = chunk.get(lx, p.y, lz);
        if after != before
            && let Some(log) = &mut self.log
        {
            log.push((p, before));
        }
        if flags & 16 == 0
            && let Some(pp) = crate::postprocess::post_process_pos(s, self, p)
        {
            self.mark_post_processing(pp);
        }
        true
    }

    /// `Feature.setBlock`: `setBlock(pos, state, 3)`.
    pub fn set_block(&mut self, p: BlockPos, s: u16) {
        self.set(p, s, 3);
    }

    /// A direct section write (`BulkSectionAccess`, used by ore features): no heightmap
    /// update, no block entity or post-processing bookkeeping.
    pub fn set_raw(&mut self, p: BlockPos, s: u16) {
        let Some(i) = self.slot(p.x >> 4, p.z >> 4) else {
            self.stats.far_writes += 1;
            return;
        };
        let old = self.chunks[i].set_raw((p.x & 15) as usize, p.y, (p.z & 15) as usize, s);
        if old != s
            && let Some(log) = &mut self.log
        {
            log.push((p, old));
        }
    }

    /// `WorldGenRegion.getHeight(type, x, z)`: the lowest y above the column's blocks that
    /// count for the heightmap.
    pub fn height_at(&mut self, map: Heightmap, x: i32, z: i32) -> i32 {
        match self.slot(x >> 4, z >> 4) {
            Some(i) => self.chunks[i].height(map, (x & 15) as usize, (z & 15) as usize),
            None => {
                self.stats.far_reads += 1;
                self.min_y()
            }
        }
    }

    /// The biome index at a block position (`LevelReader.getBiome`: the voronoi zoom over the
    /// chunks' stored biomes).
    pub fn biome(&mut self, p: BlockPos) -> u16 {
        let generator = self.generator;
        let (chunks, scratch, (cx, cz)) = (&self.chunks, &mut *self.scratch, (self.cx, self.cz));
        zoomed_biome(generator.zoom_seed, p.x, p.y, p.z, &mut |qx, qy, qz| {
            let (dx, dz) = ((qx >> 2) - cx, (qz >> 2) - cz);
            if dx.abs() <= 1 && dz.abs() <= 1 {
                chunks[((dz + 1) * 3 + dx + 1) as usize].quart_biome(qx, qy, qz)
            } else {
                scratch.noise_biome(generator, qx, qy, qz)
            }
        })
    }

    /// `ChunkAccess.markPosForPostProcessing` on the chunk holding `p`.
    pub fn mark_post_processing(&mut self, p: BlockPos) {
        if let Some(i) = self.slot(p.x >> 4, p.z >> 4) {
            self.chunks[i].mark_post_processing(p.x, p.y, p.z);
        }
    }

    /// `LevelAccessor.scheduleTick(pos, block, delay)`. A proto-chunk keeps one tick per
    /// position and type and drops the delay (`ProtoChunkTicks.schedule` saves 0).
    pub fn schedule_block_tick(&mut self, p: BlockPos, block: &'static str, _delay: i32) {
        if let Some(i) = self.slot(p.x >> 4, p.z >> 4) {
            schedule(&mut self.chunks[i].block_ticks, p, block);
        }
    }

    /// `LevelAccessor.scheduleTick(pos, fluid, delay)`, like [`Self::schedule_block_tick`].
    pub fn schedule_fluid_tick(&mut self, p: BlockPos, fluid: &'static str, _delay: i32) {
        if let Some(i) = self.slot(p.x >> 4, p.z >> 4) {
            schedule(&mut self.chunks[i].fluid_ticks, p, fluid);
        }
    }

    /// The block entity data at a position (`getBlockEntity`), if the block has one.
    pub fn block_entity_mut(&mut self, p: BlockPos) -> Option<&mut Tag> {
        let i = self.slot(p.x >> 4, p.z >> 4)?;
        let chunk = &mut self.chunks[i];
        if p.y < chunk.min_y || p.y > chunk.max_y() {
            return None;
        }
        let key = (((p.y - chunk.min_y) as u32) << 8) | (((p.z & 15) as u32) << 4) | (p.x & 15) as u32;
        chunk.block_entities.get_mut(&key)
    }

    /// `WorldGenRegion.getRandom()`: a positional Xoroshiro source for the center chunk.
    pub fn level_random(&mut self) -> &mut PositionalRandom {
        let (x, z) = (self.cx << 4, self.cz << 4);
        let factory = self.generator.region_random;
        self.level_random.get_or_insert_with(|| factory.at(x, 0, z))
    }
}

/// `ProtoChunkTicks.schedule`: appends unless the position already has a tick of that type.
fn schedule(ticks: &mut Vec<GenTick>, p: BlockPos, kind: &'static str) {
    if !ticks.iter().any(|t| t.x == p.x && t.y == p.y && t.z == p.z && t.kind == kind) {
        ticks.push(GenTick { x: p.x, y: p.y, z: p.z, kind, delay: 0, priority: 0 });
    }
}
