//! Generated chunks as `kiln-world` chunks: [`NoiseChunks`] plugs the generator into a
//! `kiln_world::ChunkProvider` for every chunk storage does not have.

use crate::generator::{GenScratch, Generator, ProtoChunk};
use kiln_world::chunk::Chunk;
use kiln_world::section::{BlockContainer, Biomes, Section};
use kiln_world::{ChunkGenerator, ChunkPos, Dimension};
use std::sync::Arc;

/// A `ChunkGenerator` running BIOMES and TERRAIN (no structures, features or spawning).
pub struct NoiseChunks {
    generator: Arc<Generator>,
    scratch: GenScratch,
    /// Network (synchronized registry) id of each generator biome.
    biome_ids: Vec<u16>,
}

impl NoiseChunks {
    pub fn new(generator: Arc<Generator>) -> Self {
        let biome_ids = generator
            .biomes
            .iter()
            .map(|b| kiln_data::synced_id("minecraft:worldgen/biome", &b.name).unwrap_or(0) as u16)
            .collect();
        Self { generator, scratch: GenScratch::default(), biome_ids }
    }

    /// Converts a generated chunk: paletted sections and network biome ids.
    pub fn to_chunk(&self, p: &ProtoChunk) -> Chunk {
        let mut palette: Vec<u16> = Vec::with_capacity(16);
        let mut index = vec![0u8; 4096];
        let sections = (0..p.sections())
            .map(|s| {
                let blocks = &p.blocks[s << 12..(s + 1) << 12];
                palette.clear();
                let mut direct = false;
                for (i, &b) in blocks.iter().enumerate() {
                    let k = match palette.iter().position(|&q| q == b) {
                        Some(k) => k,
                        None => {
                            palette.push(b);
                            palette.len() - 1
                        }
                    };
                    if k > 255 {
                        direct = true;
                        break;
                    }
                    index[i] = k as u8;
                }
                let container = if direct {
                    BlockContainer::from_palette(blocks, |i| i)
                } else {
                    BlockContainer::from_palette(&palette, |i| index[i] as usize)
                };
                let quarts = &p.biomes[s * 64..(s + 1) * 64];
                let biomes = if quarts.iter().all(|&b| b == quarts[0]) {
                    Biomes::Single(self.biome_ids[quarts[0] as usize])
                } else {
                    Biomes::Cells(Box::new(std::array::from_fn(|i| self.biome_ids[quarts[i] as usize])))
                };
                Section::new(container, biomes)
            })
            .collect();
        Chunk::new(sections, p.min_y)
    }
}

impl ChunkGenerator for NoiseChunks {
    fn generate(&mut self, pos: ChunkPos, dimension: Dimension) -> Chunk {
        debug_assert_eq!((dimension.min_y, dimension.height), (self.generator.min_y, self.generator.height));
        let p = self.generator.generate(&mut self.scratch, pos.x, pos.z);
        self.to_chunk(&p)
    }
}
