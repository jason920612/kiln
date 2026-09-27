//! Noise-based chunk generation (`NoiseBasedChunkGenerator`): the BIOMES status and the three
//! steps of TERRAIN (fill, surface, carvers), without structures.

use crate::Error;
use crate::aquifer::{Aquifer, AquiferFunctions, FluidPicker, NoiseAquifer};
use crate::biome::{BiomeInfo, LastResult, ParameterList, target};
use crate::blocks::{has_fluid, is_air, is_block, state};
use crate::carver::{Carver, CarvingMask, GenContext};
use crate::datapack::Datapack;
use crate::sampler::{SamplerRef, Scratch};
use crate::state::RandomState;
use crate::surface::{self, BiomeClimate, MaterialInputs, MaterialSystem};
use crate::volume::Volume;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::collections::{HashMap, HashSet};

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
    pub(crate) noise_context: Scratch,
    point_context: Scratch,
    biomes: BiomeCache,
    density: Vec<f32>,
}

impl Default for GenScratch {
    fn default() -> Self {
        Self {
            noise_context: Scratch::caching(),
            point_context: Scratch::default(),
            biomes: BiomeCache { context: Scratch::caching(), chunks: HashMap::new() },
            density: Vec::new(),
        }
    }
}

/// Stored biomes of chunks by position, computed on demand.
struct BiomeCache {
    context: Scratch,
    chunks: HashMap<(i32, i32), Vec<u16>>,
}

const BIOME_CACHE_CHUNKS: usize = 4096;

impl BiomeCache {
    fn get(&mut self, generator: &Generator, cx: i32, cz: i32) -> &[u16] {
        if !self.chunks.contains_key(&(cx, cz)) {
            if self.chunks.len() >= BIOME_CACHE_CHUNKS {
                self.chunks.clear();
            }
            let b = generator.chunk_biomes(&mut self.context, cx, cz);
            self.chunks.insert((cx, cz), b);
        }
        &self.chunks[&(cx, cz)]
    }

    /// `BiomeManager.getBiome` over stored chunk biomes.
    fn zoomed(&mut self, generator: &Generator, x: i32, y: i32, z: i32) -> u16 {
        zoomed_biome(generator.zoom_seed, x, y, z, &mut |qx, qy, qz| {
            stored_biome(self.get(generator, qx >> 2, qz >> 2), generator.min_y, qx, qy, qz)
        })
    }
}

/// `BiomeManager.obfuscateSeed`: the first 8 bytes (little-endian) of the SHA-256 of the
/// seed's little-endian bytes.
pub fn obfuscate_seed(seed: i64) -> i64 {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(seed.to_le_bytes());
    i64::from_le_bytes(hash[..8].try_into().unwrap())
}

/// `WorldgenRandom.setLargeFeatureSeed` on a legacy source.
fn large_feature_random(seed: i64, x: i32, z: i32) -> LegacyRandom {
    let mut r = LegacyRandom::new(seed);
    let a = r.next_long();
    let b = r.next_long();
    LegacyRandom::new((x as i64).wrapping_mul(a) ^ (z as i64).wrapping_mul(b) ^ seed)
}

/// `LinearCongruentialGenerator.next`.
#[inline]
fn lcg(a: i64, b: i64) -> i64 {
    a.wrapping_mul(a.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407)).wrapping_add(b)
}

/// `BiomeManager.getFiddle`.
#[inline]
fn fiddle(l: i64) -> f64 {
    let d = (l >> 24).rem_euclid(1024) as f64 / 1024.0;
    (d - 0.5) * 0.9
}

/// `BiomeManager.getBiome`: the quart whose jittered corner is nearest (voronoi zoom).
pub fn zoomed_biome(seed: i64, x: i32, y: i32, z: i32, noise_biome: &mut dyn FnMut(i32, i32, i32) -> u16) -> u16 {
    let (x, y, z) = (x - 2, y - 2, z - 2);
    let (qx, qy, qz) = (x >> 2, y >> 2, z >> 2);
    let (fx, fy, fz) = ((x & 3) as f64 / 4.0, (y & 3) as f64 / 4.0, (z & 3) as f64 / 4.0);
    let mut best = 0;
    let mut best_distance = f64::INFINITY;
    for i in 0..8 {
        let (ex, ey, ez) = (i & 4 == 0, i & 2 == 0, i & 1 == 0);
        let cx = if ex { qx } else { qx + 1 };
        let cy = if ey { qy } else { qy + 1 };
        let cz = if ez { qz } else { qz + 1 };
        let dx = if ex { fx } else { fx - 1.0 };
        let dy = if ey { fy } else { fy - 1.0 };
        let dz = if ez { fz } else { fz - 1.0 };
        let mut l = seed;
        for v in [cx, cy, cz, cx, cy, cz] {
            l = lcg(l, v as i64);
        }
        let f1 = fiddle(l);
        l = lcg(l, seed);
        let f2 = fiddle(l);
        l = lcg(l, seed);
        let f3 = fiddle(l);
        let d = (dz + f3) * (dz + f3) + (dy + f2) * (dy + f2) + (dx + f1) * (dx + f1);
        if best_distance > d {
            best = i;
            best_distance = d;
        }
    }
    let cx = if best & 4 == 0 { qx } else { qx + 1 };
    let cy = if best & 2 == 0 { qy } else { qy + 1 };
    let cz = if best & 1 == 0 { qz } else { qz + 1 };
    noise_biome(cx, cy, cz)
}

pub struct Generator {
    pub min_y: i32,
    pub height: i32,
    pub sea_level: i32,
    pub biomes: Vec<BiomeInfo>,
    parameters: ParameterList<u16>,
    climate: Vec<SamplerRef>,
    zoom_seed: i64,
    final_density: SamplerRef,
    default_block: u16,
    fluid_picker: FluidPicker,
    aquifer: Option<AquiferFunctions>,
    material: MaterialSystem,
    seed: i64,
    carvers: Vec<Carver>,
    /// Carver indices per biome.
    biome_carvers: Vec<Vec<usize>>,
    /// The carvers every biome of the source shares, if they all agree (then no biome lookup
    /// is needed to find a chunk's carvers).
    uniform_carvers: Option<Vec<usize>>,
    /// States of `#minecraft:uncarvable` blocks.
    uncarvable: HashSet<u16>,
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
        let mut rule_densities = Vec::new();
        surface::rule_densities(pack, &s.material_rule, &mut rule_densities)?;
        let mut roots: Vec<_> = CLIMATE.iter().map(|n| router(n)).collect();
        roots.push(router("final_density"));
        roots.push(router("chunk_surface_level"));
        roots.extend(s.aquifers.iter().map(|(_, id)| *id));
        roots.extend(&rule_densities);
        let mut compiled = state.compile(&pack.graph, &roots)?.into_iter();
        let climate: Vec<SamplerRef> = compiled.by_ref().take(CLIMATE.len()).collect();
        let final_density = compiled.next().expect("final density");
        let chunk_surface_level = compiled.next().expect("chunk surface level");
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
        let densities: HashMap<_, _> = rule_densities.iter().copied().zip(compiled).collect();
        let default_block = s.default_block.resolve()?;
        let default_fluid = s.default_fluid.resolve()?;
        let biome_names: Vec<String> = biomes.iter().map(|b| b.name.clone()).collect();
        let material = MaterialSystem::new(
            &mut state,
            MaterialInputs {
                pack,
                rule: &s.material_rule,
                default_block,
                sea_level: s.sea_level,
                min_y: s.min_y,
                height: s.height,
                preliminary_surface: chunk_surface_level,
                densities,
                biome_names: &biome_names,
                climate: biomes.iter().map(|b| BiomeClimate { temperature: b.temperature, frozen: b.frozen }).collect(),
            },
        )?;
        let mut carver_ids: Vec<String> = Vec::new();
        let mut biome_carvers = Vec::new();
        for b in &biomes {
            let mut list = Vec::new();
            for id in &b.carvers {
                let i = match carver_ids.iter().position(|c| c == id) {
                    Some(i) => i,
                    None => {
                        carver_ids.push(id.clone());
                        carver_ids.len() - 1
                    }
                };
                list.push(i);
            }
            biome_carvers.push(list);
        }
        let carvers = carver_ids
            .iter()
            .map(|id| {
                let json = pack.carvers.get(id).ok_or_else(|| Error::Invalid(format!("unknown carver {id}")))?;
                Carver::parse(json).map_err(|e| e.context(id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut source_biomes: Vec<u16> = parameters.values().iter().map(|(_, b)| *b).collect();
        source_biomes.sort_unstable();
        source_biomes.dedup();
        let uniform_carvers = source_biomes
            .windows(2)
            .all(|w| biome_carvers[w[0] as usize] == biome_carvers[w[1] as usize])
            .then(|| biome_carvers[source_biomes[0] as usize].clone());
        let mut uncarvable = HashSet::new();
        for name in pack.block_tag("minecraft:uncarvable").unwrap_or_default() {
            if let Some(b) = kiln_data::blocks_types::block_by_name(&name) {
                uncarvable.extend(b.first..=b.last);
            }
        }
        Ok(Generator {
            min_y: s.min_y,
            height: s.height,
            sea_level: s.sea_level,
            biomes,
            parameters,
            climate,
            zoom_seed: obfuscate_seed(seed),
            seed,
            carvers,
            biome_carvers,
            uniform_carvers,
            uncarvable,
            final_density,
            default_block,
            fluid_picker: FluidPicker::new(s.sea_level, default_fluid),
            aquifer,
            material,
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

    /// Generates a chunk through the BIOMES status.
    pub fn new_chunk(&self, gs: &mut GenScratch, cx: i32, cz: i32) -> ProtoChunk {
        let biomes = gs.biomes.get(self, cx, cz).to_vec();
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

        let biomes = &mut gs.biomes;
        self.material.build_surface(s, &mut |x, y, z| biomes.zoomed(self, x, y, z), chunk);
        after(Step::Surface, chunk);

        self.carve(s, &mut gs.point_context, &mut aquifer, chunk);
        after(Step::Carvers, chunk);
    }

    /// The biome of a quart from point-sampled climate (`createUncachedResolver`).
    fn point_biome(&self, s: &mut Scratch, last: &mut LastResult, qx: i32, qy: i32, qz: i32) -> u16 {
        let (x, y, z) = (qx << 2, qy << 2, qz << 2);
        let c: Vec<f32> = self.climate.iter().map(|f| f.point(s, x, y, z)).collect();
        *self.parameters.find(&target(c[0], c[1], c[2], c[3], c[4], c[5]), last)
    }

    /// The carvers of the biome at a chunk's origin (`getBiomeGenerationSettingsForCarver`).
    fn chunk_carvers(&self, s: &mut Scratch, last: &mut LastResult, cx: i32, cz: i32) -> &[usize] {
        match &self.uniform_carvers {
            Some(list) => list,
            None => &self.biome_carvers[self.point_biome(s, last, cx << 2, 0, cz << 2) as usize],
        }
    }

    /// `NoiseBasedChunkGenerator.generateCarvers`: carvers started within 8 chunks mark a
    /// carving mask, whose positions the aquifer then fills.
    fn carve(&self, s: &mut Scratch, point: &mut Scratch, aquifer: &mut Aquifer, chunk: &mut ProtoChunk) {
        let g = GenContext { min_y: self.min_y, height: self.height, sea_level: self.sea_level };
        let mut mask = CarvingMask::new(g.min_y + 1, g.min_y + g.height - 1 - 7);
        let mut last: LastResult = None;
        let (cx, cz) = (chunk.x, chunk.z);
        for dx in -8..=8 {
            for dz in -8..=8 {
                let (sx, sz) = (cx + dx, cz + dz);
                for (index, &c) in self.chunk_carvers(point, &mut last, sx, sz).iter().enumerate() {
                    let mut r = large_feature_random(self.seed.wrapping_add(index as i64), sx, sz);
                    let carver = &self.carvers[c];
                    if carver.is_start_chunk(&mut r) {
                        carver.carve(&g, &mut r, cx, cz, sx, sz, &mut mask);
                    }
                }
            }
        }
        if mask.is_empty() {
            return;
        }
        let mut point_last: LastResult = None;
        mask.visit(&mut |x, z, y0, y1| {
            let (lx, lz) = (x as usize, z as usize);
            let (bx, bz) = ((cx << 4) + x, (cz << 4) + z);
            let mut exposed_grass = false;
            for y in (y0..=y1).rev() {
                let old = chunk.get(lx, y, lz);
                if self.uncarvable.contains(&old) {
                    continue;
                }
                if is_block(old, "minecraft:grass_block") || is_block(old, "minecraft:mycelium") {
                    exposed_grass = true;
                }
                let Some(new) = aquifer.compute_substance(s, bx, y, bz, 0.0) else { continue };
                chunk.set(lx, y, lz, new);
                if exposed_grass && chunk.get(lx, y - 1, lz) == state::DIRT {
                    let top = self.material.top_material(
                        s,
                        &mut |x, y, z| {
                            zoomed_biome(self.zoom_seed, x, y, z, &mut |qx, qy, qz| self.point_biome(point, &mut point_last, qx, qy, qz))
                        },
                        chunk,
                        bx,
                        y - 1,
                        bz,
                        has_fluid(new),
                    );
                    if let Some(top) = top {
                        chunk.set(lx, y - 1, lz, top);
                    }
                }
            }
        });
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
