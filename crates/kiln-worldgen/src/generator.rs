//! Noise-based chunk generation (`NoiseBasedChunkGenerator`): the BIOMES status and the three
//! steps of TERRAIN (fill, surface, carvers), without structures.

use crate::Error;
use crate::aquifer::{Aquifer, AquiferFunctions, FluidPicker, NoiseAquifer};
use crate::biome::{BiomeInfo, LastResult, ParameterList, target};
use crate::blocks::{is_air, state};
use crate::datapack::Datapack;
use crate::sampler::{SamplerRef, Scratch};
use crate::state::RandomState;
use crate::volume::Volume;
use kiln_javamath::random::RandomSource;
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

/// A chunk being generated: block states and biomes of every section, plus the
/// `WORLD_SURFACE_WG` heightmap generation reads.
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
    /// Per column (`z << 4 | x`): the lowest y above every non-air block.
    pub surface: [i32; 256],
}

impl ProtoChunk {
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

    /// `ProtoChunk.setBlockState`: sets a block and updates the heightmap (`Heightmap.update`).
    pub fn set(&mut self, x: usize, y: i32, z: usize, s: u16) {
        if y < self.min_y || y > self.max_y() {
            return;
        }
        let i = self.index(x, y, z);
        self.blocks[i] = s;
        let first = self.surface[(z << 4) | x];
        if y <= first - 2 {
            return;
        }
        if !is_air(s) {
            if y >= first {
                self.surface[(z << 4) | x] = y + 1;
            }
        } else if first - 1 == y {
            let mut h = self.min_y;
            for yy in (self.min_y..y).rev() {
                if !is_air(self.blocks[self.index(x, yy, z)]) {
                    h = yy + 1;
                    break;
                }
            }
            self.surface[(z << 4) | x] = h;
        }
    }

    /// The biome stored for a quart of this chunk (clamped to the build height).
    pub fn quart_biome(&self, qx: i32, qy: i32, qz: i32) -> u16 {
        stored_biome(&self.biomes, self.min_y, qx, qy, qz)
    }
}

/// A chunk's stored biome at a quart (`ChunkAccess.getNoiseBiome`: y clamped to the chunk).
fn stored_biome(biomes: &[u16], min_y: i32, qx: i32, qy: i32, qz: i32) -> u16 {
    let sections = (biomes.len() / 64) as i32;
    let ry = (qy - (min_y >> 2)).clamp(0, sections * 4 - 1);
    biomes[((ry >> 2) * 64 + (((ry & 3) << 4) | ((qz & 3) << 2) | (qx & 3))) as usize]
}

/// Per-thread working memory: sampling contexts and the biomes of recently generated chunks
/// (vanilla reads neighbour biomes from chunks that already passed BIOMES).
pub struct GenScratch {
    pub(crate) biome_context: Scratch,
    pub(crate) noise_context: Scratch,
    biome_cache: HashMap<(i32, i32), Vec<u16>>,
    density: Vec<f32>,
}

impl Default for GenScratch {
    fn default() -> Self {
        Self {
            biome_context: Scratch::caching(),
            noise_context: Scratch::caching(),
            biome_cache: HashMap::new(),
            density: Vec::new(),
        }
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
    final_density: SamplerRef,
    default_block: u16,
    fluid_picker: FluidPicker,
    aquifer: Option<AquiferFunctions>,
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

        // One compilation for everything a chunk samples, so prepared caches are shared the
        // way vanilla's per-RandomState compiler shares them.
        let mut state = RandomState::new(seed, s.legacy_random_source);
        let router = |name: &str| s.router.iter().find(|(n, _)| n == name).map(|(_, id)| *id).expect("router field");
        let mut roots: Vec<_> = CLIMATE.iter().map(|n| router(n)).collect();
        roots.push(router("final_density"));
        roots.extend(s.aquifers.iter().map(|(_, id)| *id));
        let mut compiled = state.compile(&pack.graph, &roots)?.into_iter();
        let climate: Vec<SamplerRef> = compiled.by_ref().take(CLIMATE.len()).collect();
        let final_density = compiled.next().expect("final density");
        let aquifer = if s.aquifers.is_empty() {
            None
        } else {
            let mut next = || compiled.next().expect("aquifer function");
            Some(AquiferFunctions {
                barrier: next(),
                floodedness: next(),
                spread: next(),
                lava: next(),
                exclusion: next(),
                surface_level: next(),
                random: state.factory().from_hash_of("minecraft:aquifer").fork_positional(),
            })
        };
        let default_fluid = s.default_fluid.resolve()?;
        Ok(Generator {
            min_y: s.min_y,
            height: s.height,
            sea_level: s.sea_level,
            biomes,
            parameters,
            climate,
            final_density,
            default_block: s.default_block.resolve()?,
            fluid_picker: FluidPicker::new(s.sea_level, default_fluid),
            aquifer,
        })
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
        ProtoChunk { x: cx, z: cz, min_y: self.min_y, blocks: vec![state::AIR; self.sections() * 4096], biomes, surface: [self.min_y; 256] }
    }

    /// Generates a chunk through BIOMES and TERRAIN.
    pub fn generate(&self, gs: &mut GenScratch, cx: i32, cz: i32) -> ProtoChunk {
        let mut chunk = self.new_chunk(gs, cx, cz);
        self.run_steps(gs, &mut chunk, &mut |_, _| {});
        chunk
    }

    /// Runs the TERRAIN steps on a chunk that passed BIOMES, calling `after` after each.
    pub fn run_steps(&self, gs: &mut GenScratch, chunk: &mut ProtoChunk, after: &mut dyn FnMut(Step, &ProtoChunk)) {
        let vol = Volume::blocks([16, self.height, 16], [chunk.x << 4, self.min_y, chunk.z << 4]);
        // The NoiseChunk: a fresh caching context, then the aquifer (whose constructor samples
        // the surface level).
        let s = &mut gs.noise_context;
        s.reset_caches();
        let mut aquifer = match &self.aquifer {
            Some(f) => Aquifer::Noise(Box::new(NoiseAquifer::new(f, self.fluid_picker, s, &vol))),
            None => Aquifer::Disabled(self.fluid_picker),
        };
        self.fill(s, &mut gs.density, &mut aquifer, &vol, chunk);
        after(Step::Fill, chunk);
    }

    /// `NoiseBasedChunkGenerator.doFill`: final density over the whole chunk, then the
    /// aquifer decides every position, columns top down.
    fn fill(&self, s: &mut Scratch, density: &mut Vec<f32>, aquifer: &mut Aquifer, vol: &Volume, chunk: &mut ProtoChunk) {
        density.clear();
        density.resize(vol.len(), 0.0);
        self.final_density.fill(s, vol, density);
        for zi in 0..16 {
            let bz = vol.block_z(zi);
            for xi in 0..16 {
                let bx = vol.block_x(xi);
                for yi in (0..vol.size[1]).rev() {
                    let by = vol.block_y(yi);
                    let d = density[vol.index(xi, yi, zi)];
                    let block = aquifer.compute_substance(s, bx, by, bz, d as f64).unwrap_or(self.default_block);
                    if block != state::AIR {
                        chunk.set(xi as usize, by, zi as usize, block);
                    }
                }
            }
        }
    }
}
