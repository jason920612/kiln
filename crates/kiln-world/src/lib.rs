//! World storage: chunks grouped into 8×8-chunk cells (the unit regions own), block and
//! light access over any set of cells, chunk loading and generation, and cached chunk packets.
//!
//! Block, light and chunk operations are written once against [`CellStore`] (through the
//! [`Blocks`] extension trait), so they run the same on a standalone [`World`], on the cells
//! one region owns, or on every region of a dimension. They only ever see loaded chunks;
//! loading and generating is the [`ChunkProvider`]'s job.

pub mod block_entity;
pub mod chunk;
pub mod light;
pub mod section;
pub mod spawn;

use bytes::Bytes;
use chunk::Chunk;
use kiln_data::blocks::default_state as block;
pub use kiln_region::CellPos;
use kiln_region::{CELL_SHIFT, CellSet, Regions};
use section::Section;
use std::collections::HashMap;

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
        CellPos::of_chunk(self.x, self.z)
    }

    fn cell_index(self) -> usize {
        let m = (1 << CELL_SHIFT) - 1;
        (((self.z & m) << CELL_SHIFT) | (self.x & m)) as usize
    }

    /// The chunk at index `i` of `cell`.
    fn in_cell(cell: CellPos, i: usize) -> Self {
        let m = (1 << CELL_SHIFT) - 1;
        Self::new((cell.x << CELL_SHIFT) | (i as i32 & m), (cell.z << CELL_SHIFT) | ((i as i32 >> CELL_SHIFT) & m))
    }
}

/// The loaded chunks of one cell.
pub struct Cell {
    chunks: [Option<Box<Chunk>>; CELL_CHUNKS],
    /// Chunks handed out mutably since the last [`Cell::take_touched`] (one bit per chunk),
    /// so per-tick scans such as light changes skip chunks nothing touched.
    touched: u64,
}

impl Default for Cell {
    fn default() -> Self {
        Self { chunks: std::array::from_fn(|_| None), touched: 0 }
    }
}

impl Cell {
    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks[pos.cell_index()].as_deref()
    }

    pub fn chunk_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        let i = pos.cell_index();
        self.touched |= 1 << i;
        self.chunks[i].as_deref_mut()
    }

    /// Loaded chunks touched through [`Cell::chunk_mut`] since the last call.
    pub fn take_touched(&mut self, cell: CellPos) -> impl Iterator<Item = (ChunkPos, &mut Chunk)> {
        let touched = std::mem::take(&mut self.touched);
        self.chunks.iter_mut().enumerate().filter(move |(i, _)| touched & (1 << i) != 0).filter_map(
            move |(i, c)| Some((ChunkPos::in_cell(cell, i), c.as_deref_mut()?)),
        )
    }

    /// Installs a chunk (`pos` must lie in this cell); returns the one it replaced.
    pub fn insert(&mut self, pos: ChunkPos, chunk: Chunk) -> Option<Box<Chunk>> {
        self.chunks[pos.cell_index()].replace(Box::new(chunk))
    }

    pub fn remove(&mut self, pos: ChunkPos) -> Option<Box<Chunk>> {
        self.chunks[pos.cell_index()].take()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.iter().all(Option::is_none)
    }

    pub fn len(&self) -> usize {
        self.chunks.iter().filter(|c| c.is_some()).count()
    }

    /// Loaded chunks with their positions, given this cell's position.
    pub fn chunks(&self, cell: CellPos) -> impl Iterator<Item = (ChunkPos, &Chunk)> {
        self.chunks.iter().enumerate().filter_map(move |(i, c)| Some((ChunkPos::in_cell(cell, i), c.as_deref()?)))
    }

    pub fn chunks_mut(&mut self, cell: CellPos) -> impl Iterator<Item = (ChunkPos, &mut Chunk)> {
        self.chunks
            .iter_mut()
            .enumerate()
            .filter_map(move |(i, c)| Some((ChunkPos::in_cell(cell, i), c.as_deref_mut()?)))
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

    /// Queues a chunk for writing; data reaches storage on `flush`, and until then `load`
    /// must return the queued version.
    fn save(&mut self, _pos: ChunkPos, _chunk: &Chunk) {}

    /// The chunk left memory (after `save`, if it needed one): per-chunk state kept since
    /// `load` can go.
    fn unloaded(&mut self, _pos: ChunkPos) {}

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Generates chunks, e.g. vanilla's noise-based generation (`kiln-worldgen`). Called for
/// every chunk the chunk source does not have; heightmaps and light are derived from the
/// returned blocks.
pub trait ChunkGenerator: Send {
    fn generate(&mut self, pos: ChunkPos, dimension: Dimension) -> Chunk;

    /// An independent instance for another thread (same world, its own scratch space), for
    /// generating off the tick thread.
    fn fork(&self) -> Box<dyn ChunkGenerator>;
}

/// What to create where the chunk source has nothing (unless a [`ChunkGenerator`] is set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrain {
    Flat,
    Void,
}

/// Loads chunks from storage, generates the missing ones and writes changed ones back.
pub struct ChunkProvider {
    pub dimension: Dimension,
    source: Option<Box<dyn ChunkSource>>,
    generator: Option<Box<dyn ChunkGenerator>>,
    terrain: Terrain,
    biome: u16,
    /// Size of the biome registry (the width of direct biome palettes on the wire).
    pub biome_count: usize,
}

impl ChunkProvider {
    pub fn flat(dimension: Dimension, biome: u16, biome_count: usize) -> Self {
        Self { dimension, source: None, generator: None, terrain: Terrain::Flat, biome, biome_count }
    }

    /// Stored chunks, with `fallback` terrain where the source has none.
    pub fn with_source(
        dimension: Dimension,
        source: Box<dyn ChunkSource>,
        fallback: Terrain,
        biome: u16,
        biome_count: usize,
    ) -> Self {
        Self { dimension, source: Some(source), generator: None, terrain: fallback, biome, biome_count }
    }

    /// Generates missing chunks with `generator` instead of the fallback terrain.
    pub fn with_generator(mut self, generator: Box<dyn ChunkGenerator>) -> Self {
        self.generator = Some(generator);
        self
    }

    /// Y coordinate a player stands at on top of the flat terrain.
    pub fn flat_surface_y(&self) -> f64 {
        (self.dimension.min_y + FLAT_LAYERS.len() as i32) as f64
    }

    fn generate(&mut self, pos: ChunkPos) -> Chunk {
        if let Some(g) = &mut self.generator {
            return g.generate(pos, self.dimension);
        }
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

    pub fn load_or_generate(&mut self, pos: ChunkPos) -> Chunk {
        self.load(pos).unwrap_or_else(|| {
            let mut c = self.generate(pos);
            c.mark_new();
            c
        })
    }

    /// The stored chunk at `pos`, if the source has one.
    pub fn load(&mut self, pos: ChunkPos) -> Option<Chunk> {
        let dim = self.dimension;
        self.source.as_mut().and_then(|s| s.load(pos, dim))
    }

    /// A generator instance for another thread, when missing chunks come from a generator
    /// (not the cheap superflat or void fill).
    pub fn fork_generator(&self) -> Option<Box<dyn ChunkGenerator>> {
        self.generator.as_ref().map(|g| g.fork())
    }

    /// Queues `chunk` for writing if it changed (or was generated) since it was last saved.
    /// Returns whether it was queued.
    pub fn save(&mut self, pos: ChunkPos, chunk: &mut Chunk) -> bool {
        let Some(source) = self.source.as_mut() else { return false };
        if !chunk.needs_save() {
            return false;
        }
        source.save(pos, chunk);
        chunk.mark_saved();
        true
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.source.as_mut().map_or(Ok(()), |s| s.flush())
    }

    /// Saves a chunk that is leaving memory if it needs it, and lets the source forget it.
    pub fn unload(&mut self, pos: ChunkPos, chunk: &mut Chunk) -> bool {
        let saved = self.save(pos, chunk);
        if let Some(source) = self.source.as_mut() {
            source.unloaded(pos);
        }
        saved
    }

    /// Whether unloaded chunks can be written somewhere (a superflat test world cannot).
    pub fn stores(&self) -> bool {
        self.source.is_some()
    }

    /// Queues every changed chunk in `cells` and flushes; returns how many were written.
    pub fn save_all<S: CellStore + ?Sized>(&mut self, cells: &mut S) -> std::io::Result<usize> {
        let mut saved = 0;
        cells.for_each_cell_mut(&mut |pos, cell| {
            for (chunk_pos, chunk) in cell.chunks_mut(pos) {
                saved += self.save(chunk_pos, chunk) as usize;
            }
        });
        self.flush()?;
        Ok(saved)
    }
}

/// Cells that block, light and chunk operations run on.
pub trait CellStore {
    fn cell(&self, pos: CellPos) -> Option<&Cell>;
    fn cell_mut(&mut self, pos: CellPos) -> Option<&mut Cell>;
    fn for_each_cell<'a>(&'a self, f: &mut dyn FnMut(CellPos, &'a Cell));
    fn for_each_cell_mut(&mut self, f: &mut dyn FnMut(CellPos, &mut Cell));
}

/// The cells one region owns.
impl CellStore for CellSet<Cell> {
    fn cell(&self, pos: CellPos) -> Option<&Cell> {
        self.get(pos)
    }
    fn cell_mut(&mut self, pos: CellPos) -> Option<&mut Cell> {
        self.get_mut(pos)
    }
    fn for_each_cell<'a>(&'a self, f: &mut dyn FnMut(CellPos, &'a Cell)) {
        self.iter().for_each(|(p, c)| f(p, c));
    }
    fn for_each_cell_mut(&mut self, f: &mut dyn FnMut(CellPos, &mut Cell)) {
        self.iter_mut().for_each(|(p, c)| f(p, c));
    }
}

/// Every region of a dimension (serial phases only).
impl<P> CellStore for Regions<Cell, P> {
    fn cell(&self, pos: CellPos) -> Option<&Cell> {
        self.at(pos)?.cells().get(pos)
    }
    fn cell_mut(&mut self, pos: CellPos) -> Option<&mut Cell> {
        self.at_mut(pos)?.cells_mut().get_mut(pos)
    }
    fn for_each_cell<'a>(&'a self, f: &mut dyn FnMut(CellPos, &'a Cell)) {
        self.iter().for_each(|r| r.cells().for_each_cell(f));
    }
    fn for_each_cell_mut(&mut self, f: &mut dyn FnMut(CellPos, &mut Cell)) {
        self.iter_mut().for_each(|r| r.cells_mut().for_each_cell_mut(f));
    }
}

impl CellStore for HashMap<CellPos, Box<Cell>> {
    fn cell(&self, pos: CellPos) -> Option<&Cell> {
        self.get(&pos).map(|c| &**c)
    }
    fn cell_mut(&mut self, pos: CellPos) -> Option<&mut Cell> {
        self.get_mut(&pos).map(|c| &mut **c)
    }
    fn for_each_cell<'a>(&'a self, f: &mut dyn FnMut(CellPos, &'a Cell)) {
        self.iter().for_each(|(p, c)| f(*p, c));
    }
    fn for_each_cell_mut(&mut self, f: &mut dyn FnMut(CellPos, &mut Cell)) {
        self.iter_mut().for_each(|(p, c)| f(*p, c));
    }
}

/// Block, light and chunk access over loaded chunks; unloaded chunks read as `None`.
pub trait Blocks: CellStore {
    fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.cell(pos.cell())?.chunk(pos)
    }

    fn chunk_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.cell_mut(pos.cell())?.chunk_mut(pos)
    }

    fn get_block(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let c = self.chunk(ChunkPos::of_block(x, z))?;
        Some(c.get((x & 15) as usize, y, (z & 15) as usize))
    }

    /// Sets a block in a loaded chunk and updates light. Returns the previous state, or
    /// `None` if the chunk is not loaded or `y` is outside the world.
    fn set_block(&mut self, x: i32, y: i32, z: i32, state: u16) -> Option<u16> {
        let old = self.chunk_mut(ChunkPos::of_block(x, z))?.set((x & 15) as usize, y, (z & 15) as usize, state)?;
        if old != state {
            light::update_light(self, x, y, z, old, state);
        }
        Some(old)
    }

    /// Type and update tag of the block entity at a position for a Block Entity Data packet,
    /// if vanilla sends one when that block changes (an empty update tag is an empty compound).
    fn block_entity_data(&self, x: i32, y: i32, z: i32) -> Option<(u16, kiln_proto::nbt::Tag)> {
        let c = self.chunk(ChunkPos::of_block(x, z))?;
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let be = c.block_entity(lx, y, lz).filter(|be| block_entity::sends_updates(be.kind))?;
        let tag = be.update_tag(c.get(lx, y, lz)).unwrap_or(kiln_proto::nbt::Tag::Compound(Vec::new()));
        Some((be.kind, tag))
    }

    fn light_at(&self, layer: chunk::LightLayer, x: i32, y: i32, z: i32) -> Option<u8> {
        light::light_at(self, layer, x, y, z)
    }

    /// Light Data for an Update Light packet covering the given sections of a loaded chunk.
    fn light_update_body(&self, pos: ChunkPos, sky: u64, block: u64) -> Option<Bytes> {
        let mut b = bytes::BytesMut::new();
        self.chunk(pos)?.encode_light_update(sky, block, &mut b);
        Some(b.freeze())
    }

    /// Loaded chunks with light changes since the last call, with their section masks.
    fn take_light_changes(&mut self) -> Vec<(ChunkPos, u64, u64)> {
        let mut out = Vec::new();
        self.for_each_cell_mut(&mut |pos, cell| {
            for (chunk_pos, chunk) in cell.take_touched(pos) {
                let (sky, block) = chunk.take_light_dirty();
                if sky | block != 0 {
                    out.push((chunk_pos, sky, block));
                }
            }
        });
        out
    }

    fn loaded_chunks(&self) -> usize {
        let mut n = 0;
        self.for_each_cell(&mut |_, cell| n += cell.len());
        n
    }

    /// Feeds the position and block states of every loaded chunk with block edits to `h`, in
    /// an order that depends neither on container layout nor on how the cells are split
    /// between regions (for determinism tests). Unedited chunks are the generator's or the
    /// save's and which of them happen to be loaded is not part of the state.
    fn hash_blocks<H: std::hash::Hasher>(&self, h: &mut H)
    where
        Self: Sized,
    {
        use std::hash::Hash;
        let mut cells: Vec<(CellPos, &Cell)> = Vec::new();
        self.for_each_cell(&mut |pos, cell| cells.push((pos, cell)));
        cells.sort_by_key(|(pos, _)| *pos);
        for (cell_pos, cell) in cells {
            for (pos, chunk) in cell.chunks(cell_pos).filter(|(_, c)| c.edited()) {
                (pos.x, pos.z).hash(h);
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
}

impl<S: CellStore + ?Sized> Blocks for S {}

/// A standalone world: cells in a map, loading chunks on demand (tools and tests; the
/// simulation keeps its cells in regions).
pub struct World {
    pub dimension: Dimension,
    cells: HashMap<CellPos, Box<Cell>>,
    provider: ChunkProvider,
}

impl World {
    pub fn flat(dimension: Dimension, biome: u16, biome_count: usize) -> Self {
        Self::new(ChunkProvider::flat(dimension, biome, biome_count))
    }

    /// A world backed by stored chunks, with `fallback` terrain where the source has none.
    pub fn with_source(
        dimension: Dimension,
        source: Box<dyn ChunkSource>,
        fallback: Terrain,
        biome: u16,
        biome_count: usize,
    ) -> Self {
        Self::new(ChunkProvider::with_source(dimension, source, fallback, biome, biome_count))
    }

    pub fn new(provider: ChunkProvider) -> Self {
        Self { dimension: provider.dimension, cells: HashMap::new(), provider }
    }

    /// Y coordinate a player stands at on top of the flat terrain.
    pub fn flat_surface_y(&self) -> f64 {
        self.provider.flat_surface_y()
    }

    /// Writes every changed or newly generated chunk to the chunk source.
    /// Returns how many chunks were saved.
    pub fn save(&mut self) -> std::io::Result<usize> {
        self.provider.save_all(&mut self.cells)
    }

    /// The chunk at `pos`, loading or generating it first if needed.
    pub fn load_chunk(&mut self, pos: ChunkPos) -> &mut Chunk {
        if self.chunk(pos).is_none() {
            let chunk = self.provider.load_or_generate(pos);
            self.cells.entry(pos.cell()).or_default().insert(pos, chunk);
        }
        Blocks::chunk_mut(self, pos).unwrap()
    }

    pub fn chunk_body(&mut self, pos: ChunkPos) -> Bytes {
        let biome_count = self.provider.biome_count;
        self.load_chunk(pos).packet_body(biome_count)
    }

    /// Sets a block, loading its chunk if needed, and updates light. Returns the previous
    /// state, or `None` if `y` is outside the world.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: u16) -> Option<u16> {
        self.load_chunk(ChunkPos::of_block(x, z));
        Blocks::set_block(self, x, y, z, state)
    }
}

impl CellStore for World {
    fn cell(&self, pos: CellPos) -> Option<&Cell> {
        self.cells.cell(pos)
    }
    fn cell_mut(&mut self, pos: CellPos) -> Option<&mut Cell> {
        self.cells.cell_mut(pos)
    }
    fn for_each_cell<'a>(&'a self, f: &mut dyn FnMut(CellPos, &'a Cell)) {
        self.cells.for_each_cell(f)
    }
    fn for_each_cell_mut(&mut self, f: &mut dyn FnMut(CellPos, &mut Cell)) {
        self.cells.for_each_cell_mut(f)
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
        Blocks::chunk_mut(&mut w, ChunkPos::new(0, 0)).unwrap().set_block_entity(2, 0, 2, marked(chest));
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
    fn unloaded_chunks_are_not_edited_through_blocks() {
        let mut w = World::flat(OVERWORLD, 0, 67);
        assert_eq!(Blocks::set_block(&mut w, 5, 10, 5, block::STONE), None);
        assert_eq!(w.get_block(5, 10, 5), None);
        assert_eq!(w.loaded_chunks(), 0);
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

    #[test]
    fn generator_fills_missing_chunks() {
        struct Checker;
        impl ChunkGenerator for Checker {
            fn generate(&mut self, pos: ChunkPos, dimension: Dimension) -> Chunk {
                let n = (dimension.height / 16) as usize;
                let mut sections = vec![Section::filled(block::AIR, 0); n];
                let state = if (pos.x + pos.z) & 1 == 0 { block::STONE } else { block::DIRT };
                sections[4].set(1, 2, 3, state);
                Chunk::new(sections, dimension.min_y)
            }
            fn fork(&self) -> Box<dyn ChunkGenerator> {
                Box::new(Checker)
            }
        }
        let mut w = World::new(ChunkProvider::flat(OVERWORLD, 0, 67).with_generator(Box::new(Checker)));
        w.load_chunk(ChunkPos::new(0, 0));
        w.load_chunk(ChunkPos::new(-3, 0));
        assert_eq!(w.get_block(1, 2, 3), Some(block::STONE));
        assert_eq!(w.get_block(-47, 2, 3), Some(block::DIRT));
        assert_eq!(w.get_block(0, -64, 0), Some(block::AIR), "no flat layers under a generator");
    }

    #[test]
    fn chunk_positions_round_trip_through_cells() {
        for &(x, z) in &[(0, 0), (7, 7), (8, -1), (-9, 17), (-1_875_000, 1_875_000)] {
            let pos = ChunkPos::new(x, z);
            assert_eq!(ChunkPos::in_cell(pos.cell(), pos.cell_index()), pos);
        }
    }
}
