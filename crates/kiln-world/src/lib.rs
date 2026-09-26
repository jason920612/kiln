//! World storage: chunks grouped into 8×8-chunk cells (the unit regions will own), a
//! superflat generator, block access and cached chunk packets.

pub mod chunk;
pub mod section;

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

    /// Sets a block, generating its chunk if needed. Returns the previous state,
    /// or `None` if `y` is outside the world.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: u16) -> Option<u16> {
        self.chunk_mut(ChunkPos::of_block(x, z)).set((x & 15) as usize, y, (z & 15) as usize, state)
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
