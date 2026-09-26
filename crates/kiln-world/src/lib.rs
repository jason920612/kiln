//! World storage: chunks grouped into 8×8-chunk cells (the unit regions will own), a
//! superflat generator, block access and cached chunk packets.

pub mod block_entity;
pub mod chunk;
pub mod light;
pub mod section;
mod spawn;

use bytes::Bytes;
use chunk::Chunk;
use kiln_data::blocks::default_state as block;
use section::Section;
use std::collections::HashMap;

pub const CELL_SHIFT: i32 = 3;
const CELL_CHUNKS: usize = 1 << (2 * CELL_SHIFT);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub fn of_block(x: i32, z: i32) -> Self {
        Self { x: x >> 4, z: z >> 4 }
    }

    pub fn cell(self) -> CellPos {
        CellPos { x: self.x >> CELL_SHIFT, z: self.z >> CELL_SHIFT }
    }

    fn cell_index(self) -> usize {
        let m = (1 << CELL_SHIFT) - 1;
        (((self.z & m) << CELL_SHIFT) | (self.x & m)) as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellPos {
    pub x: i32,
    pub z: i32,
}

pub struct Cell {
    chunks: [Option<Box<Chunk>>; CELL_CHUNKS],
}

impl Default for Cell {
    fn default() -> Self {
        Self { chunks: std::array::from_fn(|_| None) }
    }
}

/// Dimension parameters the world needs (from the dimension type).
#[derive(Debug, Clone, Copy)]
pub struct Dimension {
    pub min_y: i32,
    pub height: i32,
}

pub const OVERWORLD: Dimension = Dimension { min_y: -64, height: 384 };

/// Layers from the bottom of the world up (superflat "classic").
pub const FLAT_LAYERS: [u16; 4] = [block::BEDROCK, block::DIRT, block::DIRT, block::GRASS_BLOCK];

/// Stored chunks, e.g. an Anvil world save.
pub trait ChunkSource: Send {
    /// Loads the chunk at `pos`, or `None` if the source has none there.
    fn load(&mut self, pos: ChunkPos, dimension: Dimension) -> Option<Chunk>;

    /// Queues a chunk for writing; data reaches storage on `flush`.
    fn save(&mut self, _pos: ChunkPos, _chunk: &Chunk) {}

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What to create where the chunk source has nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrain {
    Flat,
    Void,
}

pub struct World {
    pub dimension: Dimension,
    cells: HashMap<CellPos, Box<Cell>>,
    source: Option<Box<dyn ChunkSource>>,
    terrain: Terrain,
    biome: u16,
    biome_count: usize,
}

impl World {
    pub fn flat(dimension: Dimension, biome: u16, biome_count: usize) -> Self {
        Self { dimension, cells: HashMap::new(), source: None, terrain: Terrain::Flat, biome, biome_count }
    }

    /// A world backed by stored chunks, with `fallback` terrain where the source has none.
    pub fn with_source(
        dimension: Dimension,
        source: Box<dyn ChunkSource>,
        fallback: Terrain,
        biome: u16,
        biome_count: usize,
    ) -> Self {
        Self { dimension, cells: HashMap::new(), source: Some(source), terrain: fallback, biome, biome_count }
    }

    /// Y coordinate a player stands at on top of the flat terrain.
    pub fn flat_surface_y(&self) -> f64 {
        (self.dimension.min_y + FLAT_LAYERS.len() as i32) as f64
    }

    fn generate(&self) -> Chunk {
        let n = (self.dimension.height / 16) as usize;
        let mut sections = vec![Section::filled(block::AIR, self.biome); n];
        if self.terrain == Terrain::Flat {
            for (y, &layer) in FLAT_LAYERS.iter().enumerate() {
                for x in 0..16 {
                    for z in 0..16 {
                        sections[y >> 4].set(x, y & 15, z, layer);
                    }
                }
            }
        }
        Chunk::new(sections, self.dimension.min_y)
    }

    fn load_or_generate(&mut self, pos: ChunkPos) -> Chunk {
        let dim = self.dimension;
        self.source.as_mut().and_then(|s| s.load(pos, dim)).unwrap_or_else(|| {
            let mut c = self.generate();
            c.mark_new();
            c
        })
    }

    /// Writes every changed or newly generated chunk to the chunk source.
    /// Returns how many chunks were saved.
    pub fn save(&mut self) -> std::io::Result<usize> {
        let Some(source) = self.source.as_mut() else { return Ok(0) };
        let mut saved = 0;
        for (cell_pos, cell) in &mut self.cells {
            for (i, slot) in cell.chunks.iter_mut().enumerate() {
                let Some(chunk) = slot.as_deref_mut() else { continue };
                if !chunk.needs_save() {
                    continue;
                }
                let m = (1 << CELL_SHIFT) - 1;
                let pos = ChunkPos::new(
                    (cell_pos.x << CELL_SHIFT) | (i as i32 & m),
                    (cell_pos.z << CELL_SHIFT) | ((i as i32 >> CELL_SHIFT) & m),
                );
                source.save(pos, chunk);
                chunk.mark_saved();
                saved += 1;
            }
        }
        source.flush()?;
        Ok(saved)
    }

    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.cells.get(&pos.cell())?.chunks[pos.cell_index()].as_deref()
    }

    pub fn chunk_mut(&mut self, pos: ChunkPos) -> &mut Chunk {
        if self.chunk(pos).is_none() {
            let chunk = Box::new(self.load_or_generate(pos));
            let cell = self.cells.entry(pos.cell()).or_default();
            cell.chunks[pos.cell_index()] = Some(chunk);
        }
        self.cells.get_mut(&pos.cell()).unwrap().chunks[pos.cell_index()].as_deref_mut().unwrap()
    }

    pub fn chunk_body(&mut self, pos: ChunkPos) -> Bytes {
        let biome_count = self.biome_count;
        self.chunk_mut(pos).packet_body(biome_count)
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let c = self.chunk(ChunkPos::of_block(x, z))?;
        Some(c.get((x & 15) as usize, y, (z & 15) as usize))
    }

    /// Sets a block, generating its chunk if needed, and updates light. Returns the
    /// previous state, or `None` if `y` is outside the world.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: u16) -> Option<u16> {
        let old = self.chunk_mut(ChunkPos::of_block(x, z)).set((x & 15) as usize, y, (z & 15) as usize, state)?;
        if old != state {
            self.update_light(x, y, z, old, state);
        }
        Some(old)
    }

    /// Type and update tag of the block entity at a position for a Block Entity Data packet,
    /// if vanilla sends one when that block changes (an empty update tag is an empty compound).
    pub fn block_entity_data(&self, x: i32, y: i32, z: i32) -> Option<(u16, kiln_proto::nbt::Tag)> {
        let c = self.chunk(ChunkPos::of_block(x, z))?;
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let be = c.block_entity(lx, y, lz).filter(|be| block_entity::sends_updates(be.kind))?;
        let tag = be.update_tag(c.get(lx, y, lz)).unwrap_or(kiln_proto::nbt::Tag::Compound(Vec::new()));
        Some((be.kind, tag))
    }

    /// The chunk at `pos` if it is loaded (never loads or generates).
    pub fn chunk_mut_loaded(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.cells.get_mut(&pos.cell())?.chunks[pos.cell_index()].as_deref_mut()
    }

    /// Light Data for an Update Light packet covering the given sections of a loaded chunk.
    pub fn light_update_body(&self, pos: ChunkPos, sky: u64, block: u64) -> Option<Bytes> {
        let mut b = bytes::BytesMut::new();
        self.chunk(pos)?.encode_light_update(sky, block, &mut b);
        Some(b.freeze())
    }

    /// Loaded chunks with light changes since the last call, with their section masks.
    pub fn take_light_changes(&mut self) -> Vec<(ChunkPos, u64, u64)> {
        let mut out = Vec::new();
        for (cell_pos, cell) in &mut self.cells {
            for (i, slot) in cell.chunks.iter_mut().enumerate() {
                let Some(chunk) = slot.as_deref_mut() else { continue };
                let (sky, block) = chunk.take_light_dirty();
                if sky | block != 0 {
                    let m = (1 << CELL_SHIFT) - 1;
                    let pos = ChunkPos::new(
                        (cell_pos.x << CELL_SHIFT) | (i as i32 & m),
                        (cell_pos.z << CELL_SHIFT) | ((i as i32 >> CELL_SHIFT) & m),
                    );
                    out.push((pos, sky, block));
                }
            }
        }
        out
    }

    /// Feeds every loaded chunk's position and block states to `h` in a fixed order that does
    /// not depend on container layout (for determinism tests).
    pub fn hash_blocks<H: std::hash::Hasher>(&self, h: &mut H) {
        use std::hash::Hash;
        let mut cells: Vec<_> = self.cells.iter().collect();
        cells.sort_by_key(|(pos, _)| **pos);
        for (cell_pos, cell) in cells {
            for (i, chunk) in cell.chunks.iter().enumerate() {
                let Some(chunk) = chunk else { continue };
                (cell_pos.x, cell_pos.z, i).hash(h);
                // Runs of equal states, so a single-state container and a paletted one with
                // the same contents hash alike.
                for section in &chunk.sections {
                    let blocks = &section.blocks;
                    if let section::BlockContainer::Single(state) = blocks {
                        (*state, 4096u16).hash(h);
                        continue;
                    }
                    let (mut run_state, mut run) = (blocks.get(0), 0u16);
                    for i in 0..4096 {
                        let state = blocks.get(i);
                        if state != run_state {
                            (run_state, run).hash(h);
                            (run_state, run) = (state, 0);
                        }
                        run += 1;
                    }
                    (run_state, run).hash(h);
                }
            }
        }
    }

    pub fn loaded_chunks(&self) -> usize {
        self.cells.values().map(|c| c.chunks.iter().filter(|c| c.is_some()).count()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_across_chunks_and_cells() {
        let mut w = World::flat(OVERWORLD, 0, 67);
        for &(x, z) in &[(0, 0), (-1, -1), (129, -300), (1000, 1000)] {
            assert_eq!(w.set_block(x, 10, z, block::STONE), Some(block::AIR));
            assert_eq!(w.get_block(x, 10, z), Some(block::STONE));
            assert_eq!(w.get_block(x, -64, z), Some(block::BEDROCK));
        }
        assert_eq!(w.set_block(0, 320, 0, block::STONE), None);
        assert_eq!(w.loaded_chunks(), 4);
    }

    #[test]
    fn block_entities_follow_block_changes() {
        use kiln_proto::nbt::Tag;
        let mut w = World::flat(OVERWORLD, 0, 67);
        let kind = |w: &World, x: i32, y, z: i32| {
            let c = w.chunk(ChunkPos::of_block(x, z)).unwrap();
            c.block_entity((x & 15) as usize, y, (z & 15) as usize).map(|be| block_entity::type_name(be.kind))
        };
        w.set_block(1, 0, 1, block::CHEST);
        assert_eq!(kind(&w, 1, 0, 1), Some("minecraft:chest"));
        w.set_block(1, 0, 1, block::STONE);
        assert_eq!(kind(&w, 1, 0, 1), None);

        // Contents survive a state change of the same block and oxidation of a copper chest.
        let marked = |kind| {
            let mut be = block_entity::BlockEntity::new(kind);
            if let Tag::Compound(f) = &mut be.nbt {
                f.push(("Items".into(), Tag::List(vec![Tag::Int(7)])));
            }
            be
        };
        w.set_block(2, 0, 2, block::COPPER_CHEST);
        let chest = block_entity::type_id("minecraft:chest").unwrap();
        w.chunk_mut(ChunkPos::new(0, 0)).set_block_entity(2, 0, 2, marked(chest));
        let facing = kiln_data::blocks_types::block_of(block::COPPER_CHEST);
        w.set_block(2, 0, 2, facing.with_property(block::COPPER_CHEST, "facing", "east").unwrap());
        w.set_block(2, 0, 2, block::EXPOSED_COPPER_CHEST);
        let be = w.chunk(ChunkPos::new(0, 0)).unwrap().block_entity(2, 0, 2).unwrap();
        assert_eq!(be, &marked(chest));
        // A different block of the same type without the keep rule starts over.
        w.set_block(2, 0, 2, block::CHEST);
        assert_eq!(w.chunk(ChunkPos::new(0, 0)).unwrap().block_entity(2, 0, 2), Some(&block_entity::BlockEntity::new(chest)));
        w.set_block(2, 0, 2, block::TRAPPED_CHEST);
        assert_eq!(kind(&w, 2, 0, 2), Some("minecraft:trapped_chest"));

        w.set_block(3, 0, 3, block::OAK_SIGN);
        let (sign, tag) = w.block_entity_data(3, 0, 3).unwrap();
        assert_eq!((block_entity::type_name(sign), tag), ("minecraft:sign", Tag::Compound(Vec::new())));
        assert_eq!(w.block_entity_data(2, 0, 2), None, "chests send no block entity data");
    }

    #[test]
    fn packet_body_cache_invalidates_on_change() {
        let mut w = World::flat(OVERWORLD, 0, 67);
        let p = ChunkPos::new(0, 0);
        let a = w.chunk_body(p);
        let b = w.chunk_body(p);
        assert_eq!(a.as_ptr(), b.as_ptr(), "second call must reuse the cached body");
        w.set_block(3, 5, 3, block::STONE);
        let c = w.chunk_body(p);
        assert_ne!(a, c);
    }
}
