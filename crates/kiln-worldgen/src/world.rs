//! Generated chunks as `kiln-world` chunks: [`FullChunks`] (the whole pipeline through FULL)
//! and [`NoiseChunks`] (TERRAIN only) plug into a `kiln_world::ChunkProvider` for every chunk
//! storage does not have.

use crate::generator::{GenScratch, Generator};
use crate::pipeline::Pipeline;
use crate::proto::ProtoChunk;
use kiln_world::block_entity::BlockEntity;
use kiln_world::chunk::{Chunk, PendingUpdates};
use kiln_world::section::{BlockContainer, Biomes, Section};
use kiln_world::{ChunkGenerator, ChunkPos, Dimension};
use std::sync::Arc;

/// Network (synchronized registry) id of each generator biome.
fn network_biome_ids(generator: &Generator) -> Arc<Vec<u16>> {
    Arc::new(
        generator
            .biomes
            .iter()
            .map(|b| kiln_data::synced_id("minecraft:worldgen/biome", &b.name).unwrap_or(0) as u16)
            .collect(),
    )
}

/// Converts a generated chunk: paletted sections, network biome ids, block entities (a
/// placeholder left by generation becomes the block's default block entity) and the updates
/// owed when the chunk becomes full.
pub fn to_chunk(p: &ProtoChunk, biome_ids: &[u16]) -> Chunk {
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
                Biomes::Single(biome_ids[quarts[0] as usize])
            } else {
                Biomes::Cells(Box::new(std::array::from_fn(|i| biome_ids[quarts[i] as usize])))
            };
            Section::new(container, biomes)
        })
        .collect();
    let mut chunk = Chunk::new(sections, p.min_y);
    for (&key, tag) in &p.block_entities {
        let (x, y, z) = ((key & 15) as usize, (key >> 8) as i32 + p.min_y, ((key >> 4) & 15) as usize);
        let state = p.get(x, y, z);
        let Some(kind) = kiln_data::block_props::block_entity_type(state) else { continue };
        // Generation leaves `id: DUMMY` placeholders (as vanilla's WorldGenRegion does), possibly
        // with fields features and structures set (loot tables...).
        let placeholder = tag.get("id").and_then(|t| t.as_str()) == Some("DUMMY");
        let be = if placeholder {
            let mut be = BlockEntity::new(kind);
            if let (kiln_proto::nbt::Tag::Compound(dst), kiln_proto::nbt::Tag::Compound(src)) = (&mut be.nbt, tag) {
                dst.extend(src.iter().filter(|(k, _)| k != "id").cloned());
            }
            Some(be)
        } else {
            BlockEntity::from_saved(tag.clone())
        };
        if let Some(be) = be {
            chunk.load_block_entity(x, y, z, be);
        }
    }
    let mut pending = PendingUpdates::default();
    for (s, list) in p.post_processing.iter().enumerate() {
        for &packed in list {
            let (x, y, z) = ((packed & 15) as i32, ((packed >> 4) & 15) as i32, ((packed >> 8) & 15) as i32);
            pending.post_process.push([(p.x << 4) + x, p.min_y + s as i32 * 16 + y, (p.z << 4) + z]);
        }
    }
    pending.block_ticks = p.block_ticks.iter().map(|t| ([t.x, t.y, t.z], t.kind, t.delay)).collect();
    pending.fluid_ticks = p.fluid_ticks.iter().map(|t| ([t.x, t.y, t.z], t.kind, t.delay)).collect();
    chunk.set_pending_updates(pending);
    chunk
}

/// A `ChunkGenerator` running the whole pipeline ([`Pipeline`]): FULL chunks with features.
/// Forks share the pipeline, so generation threads share proto-chunks and work.
pub struct FullChunks {
    pipeline: Arc<Pipeline>,
    scratch: GenScratch,
    biome_ids: Arc<Vec<u16>>,
}

impl FullChunks {
    pub fn new(pipeline: Arc<Pipeline>) -> Self {
        let biome_ids = network_biome_ids(&pipeline.world().generator);
        Self { pipeline, scratch: GenScratch::default(), biome_ids }
    }

    pub fn pipeline(&self) -> &Arc<Pipeline> {
        &self.pipeline
    }
}

impl ChunkGenerator for FullChunks {
    fn generate(&mut self, pos: ChunkPos, dimension: Dimension) -> Chunk {
        let g = &self.pipeline.world().generator;
        debug_assert_eq!((dimension.min_y, dimension.height), (g.min_y, g.height));
        let p = self.pipeline.full(&mut self.scratch, pos.x, pos.z);
        let mut chunk = to_chunk(&p, &self.biome_ids);
        chunk.structures = Some(Box::new(self.pipeline.structure_data(&mut self.scratch, pos.x, pos.z)));
        chunk
    }

    fn fork(&self) -> Box<dyn ChunkGenerator> {
        Box::new(FullChunks { pipeline: self.pipeline.clone(), scratch: GenScratch::default(), biome_ids: self.biome_ids.clone() })
    }
}

/// A `ChunkGenerator` running BIOMES and TERRAIN only (no structures, features or spawning).
pub struct NoiseChunks {
    generator: Arc<Generator>,
    scratch: GenScratch,
    biome_ids: Arc<Vec<u16>>,
}

impl NoiseChunks {
    pub fn new(generator: Arc<Generator>) -> Self {
        let biome_ids = network_biome_ids(&generator);
        Self { generator, scratch: GenScratch::default(), biome_ids }
    }

    /// Converts a generated chunk (see [`to_chunk`]).
    pub fn to_chunk(&self, p: &ProtoChunk) -> Chunk {
        to_chunk(p, &self.biome_ids)
    }
}

impl ChunkGenerator for NoiseChunks {
    fn generate(&mut self, pos: ChunkPos, dimension: Dimension) -> Chunk {
        debug_assert_eq!((dimension.min_y, dimension.height), (self.generator.min_y, self.generator.height));
        let p = self.generator.generate(&mut self.scratch, pos.x, pos.z);
        self.to_chunk(&p)
    }

    fn fork(&self) -> Box<dyn ChunkGenerator> {
        Box::new(NoiseChunks { generator: self.generator.clone(), scratch: GenScratch::default(), biome_ids: self.biome_ids.clone() })
    }
}
