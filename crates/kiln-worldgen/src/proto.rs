//! Chunks being generated (`ProtoChunk`): block states, biomes, the heightmaps generation reads
//! and what a finished chunk carries beyond blocks (post-processing positions, scheduled ticks,
//! block entities).

use crate::block_facts::fluid;
use crate::blocks::{is_air, state};
use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// How far a chunk got, as far as its own data is concerned (`ChunkAccess.getPersistedStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// BIOMES done, TERRAIN running: writes update the worldgen heightmaps.
    Biomes,
    /// TERRAIN done: writes update the final heightmaps.
    Terrain,
    Features,
}

/// `Heightmap.Types`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heightmap {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl Heightmap {
    pub fn parse(name: &str) -> Option<Heightmap> {
        Some(match name {
            "WORLD_SURFACE_WG" => Heightmap::WorldSurfaceWg,
            "WORLD_SURFACE" => Heightmap::WorldSurface,
            "OCEAN_FLOOR_WG" => Heightmap::OceanFloorWg,
            "OCEAN_FLOOR" => Heightmap::OceanFloor,
            "MOTION_BLOCKING" => Heightmap::MotionBlocking,
            "MOTION_BLOCKING_NO_LEAVES" => Heightmap::MotionBlockingNoLeaves,
            _ => return None,
        })
    }

    /// Bit of [`heightmap_flags`] a block needs to count for this heightmap.
    fn bit(self) -> u8 {
        match self {
            Heightmap::WorldSurfaceWg | Heightmap::WorldSurface => NOT_AIR,
            Heightmap::OceanFloorWg | Heightmap::OceanFloor => MOTION,
            Heightmap::MotionBlocking => MOTION_OR_FLUID,
            Heightmap::MotionBlockingNoLeaves => NO_LEAVES_OR_FLUID,
        }
    }

    /// Index among the final heightmaps.
    fn final_index(self) -> Option<usize> {
        match self {
            Heightmap::WorldSurface => Some(0),
            Heightmap::OceanFloor => Some(1),
            Heightmap::MotionBlocking => Some(2),
            Heightmap::MotionBlockingNoLeaves => Some(3),
            _ => None,
        }
    }
}

const FINAL: [Heightmap; 4] =
    [Heightmap::WorldSurface, Heightmap::OceanFloor, Heightmap::MotionBlocking, Heightmap::MotionBlockingNoLeaves];

/// The [`heightmap_flags`] bit a block needs to count for `map`.
pub fn heightmap_bit(map: Heightmap) -> u8 {
    map.bit()
}

const NOT_AIR: u8 = 1;
const MOTION: u8 = 2;
const MOTION_OR_FLUID: u8 = 4;
const NO_LEAVES_OR_FLUID: u8 = 8;

/// Per state: which heightmap predicates it satisfies (`Heightmap.Types` in 26.3: not air;
/// `#blocks_motion_in_heightmap`; that or a fluid; `#blocks_motion_in_heightmap_no_leaves` or a
/// fluid).
pub fn heightmap_flags(s: u16) -> u8 {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let n = kiln_data::blocks::STATE_COUNT as usize;
        let tag = |name: &str| {
            let mut out = vec![false; n];
            let ids = kiln_data::registries::TAGS
                .iter()
                .find(|(r, _)| *r == "minecraft:block")
                .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == name))
                .map_or(&[][..], |(_, ids)| *ids);
            let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
            for &id in ids {
                if let Some(b) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                    out[b.first as usize..=b.last as usize].fill(true);
                }
            }
            out
        };
        let motion = tag("minecraft:blocks_motion_in_heightmap");
        let no_leaves = tag("minecraft:blocks_motion_in_heightmap_no_leaves");
        (0..n)
            .map(|i| {
                let s = i as u16;
                let f = !fluid(s).is_empty();
                (if is_air(s) { 0 } else { NOT_AIR })
                    | if motion[i] { MOTION } else { 0 }
                    | if motion[i] || f { MOTION_OR_FLUID } else { 0 }
                    | if no_leaves[i] || f { NO_LEAVES_OR_FLUID } else { 0 }
            })
            .collect()
    })[s as usize]
}

/// A tick scheduled during generation (`ProtoChunkTicks`): the block or fluid at `pos` is
/// ticked `delay` ticks after the chunk becomes full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenTick {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Registry name of the block or fluid type.
    pub kind: &'static str,
    pub delay: i32,
    pub priority: i32,
}

/// A chunk being generated.
#[derive(Clone)]
pub struct ProtoChunk {
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    /// Block states, section by section from the bottom; within a section `y << 8 | z << 4 | x`.
    pub blocks: Vec<u16>,
    /// Biome indices ([`crate::Generator::biomes`]) per quart, section by section; within a
    /// section `y << 4 | z << 2 | x`.
    pub biomes: Vec<u16>,
    /// `WORLD_SURFACE_WG` per column (`z << 4 | x`): the lowest y above every non-air block.
    pub surface: [i32; 256],
    /// `OCEAN_FLOOR_WG` per column.
    pub ocean_floor: [i32; 256],
    pub status: Status,
    /// Final heightmaps (world surface, ocean floor, motion blocking, motion blocking without
    /// leaves), from TERRAIN on.
    heights: Box<[[i32; 256]; 4]>,
    /// Non-air blocks per section (`LevelChunkSection.nonEmptyBlockCount`).
    non_air: Vec<u16>,
    /// Positions to update when the chunk becomes full (`ProtoChunk.postProcessing`), per
    /// section as packed `x | y << 4 | z << 8` offsets. Fluids there start flowing.
    pub post_processing: Vec<Vec<u16>>,
    pub block_ticks: Vec<GenTick>,
    pub fluid_ticks: Vec<GenTick>,
    /// Block entity data by block index (`y - min_y << 8 | z << 4 | x`), in saved form without
    /// position fields.
    pub block_entities: BTreeMap<u32, Tag>,
    /// Entities generation added (`ProtoChunk.addEntity`: end crystals, end city shulkers and
    /// item frames...), in saved form (`id`, `Pos`, ...), in the order they were added.
    pub entities: Vec<Tag>,
}

impl ProtoChunk {
    /// An empty chunk (all air) at BIOMES.
    pub fn new(x: i32, z: i32, min_y: i32, sections: usize, biomes: Vec<u16>) -> Self {
        Self {
            x,
            z,
            min_y,
            blocks: vec![state::AIR; sections * 4096],
            biomes,
            surface: [min_y; 256],
            ocean_floor: [min_y; 256],
            status: Status::Biomes,
            heights: Box::new([[min_y; 256]; 4]),
            non_air: vec![0; sections],
            post_processing: vec![Vec::new(); sections],
            block_ticks: Vec::new(),
            fluid_ticks: Vec::new(),
            block_entities: BTreeMap::new(),
            entities: Vec::new(),
        }
    }

    pub fn sections(&self) -> usize {
        self.biomes.len() / 64
    }

    pub fn max_y(&self) -> i32 {
        self.min_y + (self.sections() as i32) * 16 - 1
    }

    #[inline]
    pub fn index(&self, x: usize, y: i32, z: usize) -> usize {
        let ry = (y - self.min_y) as usize;
        ((ry >> 4) << 12) | ((ry & 15) << 8) | (z << 4) | x
    }

    /// The block at a column-local position; `VOID_AIR` outside the build height.
    #[inline]
    pub fn get(&self, x: usize, y: i32, z: usize) -> u16 {
        if y < self.min_y || y > self.max_y() { state::VOID_AIR } else { self.blocks[self.index(x, y, z)] }
    }

    /// `ChunkAccess.getHeight(WORLD_SURFACE_WG, x, z) + 1`.
    #[inline]
    pub fn surface_height(&self, x: usize, z: usize) -> i32 {
        self.surface[(z << 4) | x]
    }

    /// `ChunkAccess.getHeight(type, x, z) + 1`: the lowest y above every block counting for
    /// the heightmap (`min_y` for an empty column).
    pub fn height(&self, map: Heightmap, x: usize, z: usize) -> i32 {
        let col = (z << 4) | x;
        match map {
            Heightmap::WorldSurfaceWg => self.surface[col],
            Heightmap::OceanFloorWg => self.ocean_floor[col],
            m => {
                if self.status < Status::Terrain {
                    self.scan(m, x, z)
                } else {
                    self.heights[m.final_index().unwrap()][col]
                }
            }
        }
    }

    /// The heightmap value computed from the blocks (`Heightmap.primeHeightmaps`).
    fn scan(&self, map: Heightmap, x: usize, z: usize) -> i32 {
        let bit = map.bit();
        for y in (self.min_y..=self.max_y()).rev() {
            if heightmap_flags(self.blocks[self.index(x, y, z)]) & bit != 0 {
                return y + 1;
            }
        }
        self.min_y
    }

    /// TERRAIN is done: from now on writes maintain the final heightmaps, primed from the
    /// blocks (vanilla primes them on first use, which gives the same values).
    pub fn finish_terrain(&mut self) {
        self.prime_heightmaps();
        self.status = Status::Terrain;
    }

    /// Recomputes the final heightmaps from the blocks (`Heightmap.primeHeightmaps`).
    pub fn prime_heightmaps(&mut self) {
        for (i, m) in FINAL.iter().enumerate() {
            for z in 0..16 {
                for x in 0..16 {
                    self.heights[i][(z << 4) | x] = self.scan(*m, x, z);
                }
            }
        }
    }

    /// `ProtoChunk.setBlockState`: sets a block and updates the heightmaps of the chunk's
    /// status. Setting `AIR` in a section of air only changes nothing. Returns the old state.
    pub fn set(&mut self, x: usize, y: i32, z: usize, s: u16) -> u16 {
        if y < self.min_y || y > self.max_y() {
            return state::VOID_AIR;
        }
        let section = ((y - self.min_y) >> 4) as usize;
        if self.non_air[section] == 0 && s == state::AIR {
            return s;
        }
        let i = self.index(x, y, z);
        let old = self.blocks[i];
        self.blocks[i] = s;
        match (is_air(old), is_air(s)) {
            (true, false) => self.non_air[section] += 1,
            (false, true) => self.non_air[section] -= 1,
            _ => {}
        }
        let col = (z << 4) | x;
        if self.status < Status::Terrain {
            update(&mut self.surface[col], NOT_AIR, &self.blocks, self.min_y, x, y, z, s);
            update(&mut self.ocean_floor[col], MOTION, &self.blocks, self.min_y, x, y, z, s);
        } else {
            for (i, m) in FINAL.iter().enumerate() {
                let mut h = self.heights[i][col];
                update(&mut h, m.bit(), &self.blocks, self.min_y, x, y, z, s);
                self.heights[i][col] = h;
            }
        }
        old
    }

    /// Writes a block without touching heightmaps or section counts' side effects on them
    /// (`LevelChunkSection.setBlockState` through `BulkSectionAccess`, as ore features do).
    pub fn set_raw(&mut self, x: usize, y: i32, z: usize, s: u16) -> u16 {
        if y < self.min_y || y > self.max_y() {
            return state::VOID_AIR;
        }
        let section = ((y - self.min_y) >> 4) as usize;
        let i = self.index(x, y, z);
        let old = self.blocks[i];
        self.blocks[i] = s;
        match (is_air(old), is_air(s)) {
            (true, false) => self.non_air[section] += 1,
            (false, true) => self.non_air[section] -= 1,
            _ => {}
        }
        old
    }

    /// `LevelChunkSection.hasOnlyAir` for the section holding `y`.
    pub fn section_is_air(&self, y: i32) -> bool {
        let section = ((y - self.min_y) >> 4) as usize;
        self.non_air.get(section).is_none_or(|&n| n == 0)
    }

    /// `ProtoChunk.markPosForPostProcessing` (absolute coordinates).
    pub fn mark_post_processing(&mut self, x: i32, y: i32, z: i32) {
        if y < self.min_y || y > self.max_y() {
            return;
        }
        let section = ((y - self.min_y) >> 4) as usize;
        self.post_processing[section].push(((x & 15) | ((y & 15) << 4) | ((z & 15) << 8)) as u16);
    }

    /// The biome stored for a quart of this chunk (clamped to the build height).
    pub fn quart_biome(&self, qx: i32, qy: i32, qz: i32) -> u16 {
        stored_biome(&self.biomes, self.min_y, qx, qy, qz)
    }

    /// Biome indices present in the chunk (`PalettedContainer.getAll` over every section).
    pub fn present_biomes(&self, out: &mut Vec<u16>) {
        for &b in &self.biomes {
            if !out.contains(&b) {
                out.push(b);
            }
        }
    }
}

/// `Heightmap.update` on one column value.
#[allow(clippy::too_many_arguments)]
#[inline]
fn update(first: &mut i32, bit: u8, blocks: &[u16], min_y: i32, x: usize, y: i32, z: usize, s: u16) {
    if y <= *first - 2 {
        return;
    }
    if heightmap_flags(s) & bit != 0 {
        if y >= *first {
            *first = y + 1;
        }
    } else if *first - 1 == y {
        let at = |yy: i32| {
            let ry = (yy - min_y) as usize;
            blocks[((ry >> 4) << 12) | ((ry & 15) << 8) | (z << 4) | x]
        };
        *first = (min_y..y).rev().find(|&yy| heightmap_flags(at(yy)) & bit != 0).map_or(min_y, |yy| yy + 1);
    }
}

/// A chunk's stored biome at a quart (`ChunkAccess.getNoiseBiome`: y clamped to the chunk).
pub fn stored_biome(biomes: &[u16], min_y: i32, qx: i32, qy: i32, qz: i32) -> u16 {
    let sections = (biomes.len() / 64) as i32;
    let ry = (qy - (min_y >> 2)).clamp(0, sections * 4 - 1);
    biomes[((ry >> 2) * 64 + (((ry & 3) << 4) | ((qz & 3) << 2) | (qx & 3))) as usize]
}
