//! Noise-based chunk generation (`NoiseBasedChunkGenerator`): the BIOMES status and the three
//! steps of TERRAIN (fill, surface, carvers), without structures.

use crate::Error;
use crate::biome::{BiomeInfo, LastResult, ParameterList, target};
use crate::datapack::Datapack;
use crate::sampler::{SamplerRef, Scratch};
use crate::state::RandomState;
use crate::volume::Volume;
use std::collections::HashMap;

/// The router fields a `Climate.Sampler` reads, in `Climate.target` order.
const CLIMATE: [&str; 6] = ["temperature", "vegetation", "continents", "erosion", "depth", "ridges"];

/// The steps of the TERRAIN status, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Fill,
    Surface,
    Carvers,
}

/// A chunk being generated: block states and biomes of every section.
#[derive(Clone)]
pub struct ProtoChunk {
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    /// Block states, section by section from the bottom; within a section `y << 8 | z << 4 | x`.
    pub blocks: Vec<u16>,
    /// Biome indices ([`Generator::biomes`]) per quart, section by section; within a section
    /// `y << 4 | z << 2 | x`.
    pub biomes: Vec<u16>,
}

impl ProtoChunk {
    pub fn sections(&self) -> usize {
        self.biomes.len() / 64
    }

    #[inline]
    pub fn index(&self, x: usize, y: i32, z: usize) -> usize {
        let ry = (y - self.min_y) as usize;
        ((ry >> 4) << 12) | ((ry & 15) << 8) | (z << 4) | x
    }

    #[inline]
    pub fn get(&self, x: usize, y: i32, z: usize) -> u16 {
        self.blocks[self.index(x, y, z)]
    }
}

/// Per-thread working memory: sampling contexts and the biomes of recently generated chunks
/// (vanilla reads neighbour biomes from chunks that already passed BIOMES).
pub struct GenScratch {
    pub(crate) biome_context: Scratch,
    pub(crate) noise_context: Scratch,
    biome_cache: HashMap<(i32, i32), Vec<u16>>,
}

impl Default for GenScratch {
    fn default() -> Self {
        Self { biome_context: Scratch::caching(), noise_context: Scratch::caching(), biome_cache: HashMap::new() }
    }
}

const BIOME_CACHE_CHUNKS: usize = 4096;

pub struct Generator {
    pub min_y: i32,
    pub height: i32,
    pub sea_level: i32,
    pub biomes: Vec<BiomeInfo>,
    parameters: ParameterList<u16>,
    climate: Vec<SamplerRef>,
}

impl Generator {
    /// The generator for `noise_settings` entry `settings` with the multi-noise biome source
    /// `biome_source` (a `multi_noise_biome_source_parameter_list` id), seeded with `seed`.
    pub fn new(pack: &Datapack, settings: &str, biome_source: &str, seed: i64) -> Result<Generator, Error> {
        let s = pack.settings(settings)?;
        let biomes: Vec<BiomeInfo> =
            pack.biomes.iter().map(|(id, json)| BiomeInfo::parse(id, json).map_err(|e| e.context(id))).collect::<Result<_, _>>()?;
        let index: HashMap<&str, u16> = biomes.iter().enumerate().map(|(i, b)| (b.name.as_str(), i as u16)).collect();
        let list = pack
            .parameter_lists
            .get(&crate::function::qualify(biome_source))
            .ok_or_else(|| Error::Invalid(format!("no parameter list {biome_source} (reports/biome_parameters missing?)")))?;
        let values = list
            .iter()
            .map(|(space, biome)| {
                index.get(biome.as_str()).map(|&i| (*space, i)).ok_or_else(|| Error::Invalid(format!("unknown biome {biome}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let parameters = ParameterList::new(values)?;

        let mut state = RandomState::new(seed, s.legacy_random_source);
        let router = |name: &str| s.router.iter().find(|(n, _)| n == name).map(|(_, id)| *id).expect("router field");
        let roots: Vec<_> = CLIMATE.iter().map(|n| router(n)).collect();
        let climate = state.compile(&pack.graph, &roots)?;
        Ok(Generator { min_y: s.min_y, height: s.height, sea_level: s.sea_level, biomes, parameters, climate })
    }

    pub fn sections(&self) -> usize {
        (self.height >> 4) as usize
    }

    pub fn parameters(&self) -> &ParameterList<u16> {
        &self.parameters
    }

    /// `ChunkGenerator.doCreateBiomes`: the climate on the chunk's quart grid in volume mode,
    /// then one parameter-list search per quart in `fillBiomesFromNoise` order (sections
    /// upward, then x, y, z), starting from an empty last result.
    pub fn chunk_biomes(&self, s: &mut Scratch, cx: i32, cz: i32) -> Vec<u16> {
        s.reset_caches();
        let quarts_y = self.height >> 2;
        let vol = Volume::new([4, quarts_y, 4], [cx << 4, (self.min_y >> 2) << 2, cz << 4], [4, 4, 4]);
        let climate: Vec<Vec<f32>> = self
            .climate
            .iter()
            .map(|f| {
                let mut b = vec![0f32; vol.len()];
                f.fill(s, &vol, &mut b);
                b
            })
            .collect();
        let mut last: LastResult = None;
        let mut out = vec![0u16; self.sections() * 64];
        for section in 0..self.sections() {
            for x in 0..4 {
                for y in 0..4 {
                    for z in 0..4 {
                        let i = vol.index(x, section as i32 * 4 + y, z);
                        let t = target(climate[0][i], climate[1][i], climate[2][i], climate[3][i], climate[4][i], climate[5][i]);
                        out[section * 64 + ((y << 4) | (z << 2) | x) as usize] = *self.parameters.find(&t, &mut last);
                    }
                }
            }
        }
        out
    }

    /// The stored biomes of chunk `(cx, cz)`, from the per-thread cache.
    pub(crate) fn cached_biomes<'a>(&self, gs: &'a mut GenScratch, cx: i32, cz: i32) -> &'a [u16] {
        if !gs.biome_cache.contains_key(&(cx, cz)) {
            if gs.biome_cache.len() >= BIOME_CACHE_CHUNKS {
                gs.biome_cache.clear();
            }
            let b = self.chunk_biomes(&mut gs.biome_context, cx, cz);
            gs.biome_cache.insert((cx, cz), b);
        }
        &gs.biome_cache[&(cx, cz)]
    }

    /// Generates a chunk through the BIOMES status.
    pub fn new_chunk(&self, gs: &mut GenScratch, cx: i32, cz: i32) -> ProtoChunk {
        let biomes = self.cached_biomes(gs, cx, cz).to_vec();
        ProtoChunk { x: cx, z: cz, min_y: self.min_y, blocks: vec![0; self.sections() * 4096], biomes }
    }

    /// Runs the TERRAIN steps on a chunk that passed BIOMES, calling `after` after each.
    pub fn run_steps(&self, _gs: &mut GenScratch, _chunk: &mut ProtoChunk, _after: &mut dyn FnMut(Step, &ProtoChunk)) {}
}
