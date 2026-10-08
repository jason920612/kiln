//! Noise-based chunk generation (`NoiseBasedChunkGenerator`): the BIOMES status and the three
//! steps of TERRAIN (fill, surface, carvers), without structures.

use crate::Error;
use crate::aquifer::{Aquifer, AquiferFunctions, FluidPicker, NoiseAquifer};
use crate::biome::{BiomeInfo, LastResult, Parameter, ParameterList, target};
use crate::blocks::{has_fluid, is_block, state};
use crate::carver::{Carver, CarvingMask, GenContext};
pub use crate::proto::ProtoChunk;
use crate::proto::stored_biome;
use crate::datapack::Datapack;
use crate::sampler::{SamplerRef, Scratch};
use crate::state::RandomState;
use crate::surface::{self, BiomeClimate, MaterialInputs, MaterialSystem};
use crate::volume::Volume;
use kiln_javamath::random::{LegacyRandom, PositionalRandomFactory, RandomSource};
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

/// Per-thread working memory: sampling contexts and the biomes of recently generated chunks
/// (vanilla reads neighbour biomes from chunks that already passed BIOMES).
pub struct GenScratch {
    noise_context: Scratch,
    point_context: Scratch,
    biomes: BiomeCache,
    density: Vec<f32>,
    /// Structure starts: climate sampler and base heights.
    pub structures: crate::structure::StructureScratch,
}

impl Default for GenScratch {
    fn default() -> Self {
        Self {
            noise_context: Scratch::caching(),
            point_context: Scratch::default(),
            biomes: BiomeCache { context: Scratch::caching(), chunks: HashMap::new() },
            density: Vec::new(),
            structures: Default::default(),
        }
    }
}

impl GenScratch {
    /// The biome a chunk stores for a quart (computed if the chunk is not cached).
    pub fn noise_biome(&mut self, generator: &Generator, qx: i32, qy: i32, qz: i32) -> u16 {
        stored_biome(self.biomes.get(generator, qx >> 2, qz >> 2), generator.min_y, qx, qy, qz)
    }

    /// The point-sampling context (no caches).
    pub fn point_context(&mut self) -> &mut Scratch {
        &mut self.point_context
    }

    /// The stored biomes of a chunk (BIOMES status), from the per-thread cache.
    pub fn chunk_biomes(&mut self, generator: &Generator, cx: i32, cz: i32) -> &[u16] {
        self.biomes.get(generator, cx, cz)
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

/// Which biome source a generator uses.
#[derive(Clone, Copy, Debug)]
pub enum BiomeSourceKind<'a> {
    /// `MultiNoiseBiomeSource` from a `multi_noise_biome_source_parameter_list` id.
    MultiNoise(&'a str),
    /// `TheEndBiomeSource`.
    TheEnd,
}

/// A generator's biome source.
pub enum BiomeSource {
    MultiNoise(ParameterList<u16>),
    /// `TheEndBiomeSource`: `the_end` within 64 chunks of the origin, elsewhere chosen by the
    /// router's erosion (the end islands function) at the section's center column.
    TheEnd { end: u16, highlands: u16, midlands: u16, islands: u16, barrens: u16 },
}

impl BiomeSource {
    /// `BiomeSource.possibleBiomes` in order (first occurrence).
    pub fn possible(&self) -> Vec<u16> {
        match self {
            BiomeSource::MultiNoise(list) => {
                let mut order: Vec<u16> = Vec::new();
                for (_, b) in list.values() {
                    if !order.contains(b) {
                        order.push(*b);
                    }
                }
                order
            }
            BiomeSource::TheEnd { end, highlands, midlands, islands, barrens } => vec![*end, *highlands, *midlands, *islands, *barrens],
        }
    }
}

/// Index of the erosion field in [`CLIMATE`].
const EROSION: usize = 3;

pub struct Generator {
    /// The level's geometry (`DimensionType` `min_y`/`height`): the chunks' sections.
    pub min_y: i32,
    pub height: i32,
    /// The generation context (`WorldGenerationContext`: the noise settings clamped to the
    /// level): what TERRAIN fills, what anchors and carvers resolve against.
    pub gen_min_y: i32,
    pub gen_height: i32,
    pub sea_level: i32,
    /// `disable_mob_generation` of the noise settings: no initial animals.
    pub disable_mob_generation: bool,
    pub biomes: Vec<BiomeInfo>,
    source: BiomeSource,
    climate: Vec<SamplerRef>,
    pub zoom_seed: i64,
    final_density: SamplerRef,
    default_block: u16,
    fluid_picker: FluidPicker,
    aquifer: Option<AquiferFunctions>,
    material: MaterialSystem,
    pub seed: i64,
    /// `RandomState.getOrCreateRandomFactory(worldgen_region_random)`: `WorldGenRegion.random`.
    pub region_random: PositionalRandomFactory,
    carvers: Vec<Carver>,
    /// Carver indices per biome.
    biome_carvers: Vec<Vec<usize>>,
    /// The carvers every biome of the source shares, if they all agree (then no biome lookup
    /// is needed to find a chunk's carvers).
    uniform_carvers: Option<Vec<usize>>,
    /// States of `#minecraft:uncarvable` blocks.
    uncarvable: HashSet<u16>,
    /// `spawn_target` points: (density function, climate parameter) pairs.
    pub(crate) spawn_target: Vec<Vec<(SamplerRef, Parameter)>>,
}

impl Generator {
    /// The generator for `noise_settings` entry `settings` with the multi-noise biome source
    /// `biome_source` (a `multi_noise_biome_source_parameter_list` id), seeded with `seed`,
    /// building chunks as tall as the noise settings (the overworld's geometry).
    pub fn new(pack: &Datapack, settings: &str, biome_source: &str, seed: i64) -> Result<Generator, Error> {
        let s = pack.settings(settings)?;
        Self::for_level(pack, settings, BiomeSourceKind::MultiNoise(biome_source), (s.min_y, s.height), seed)
    }

    /// The generator for noise settings `settings` and `biome_source` in a level of
    /// `dimension_type` (whose geometry the chunks get), seeded with `seed`.
    pub fn for_dimension(
        pack: &Datapack,
        settings: &str,
        biome_source: BiomeSourceKind,
        dimension_type: &str,
        seed: i64,
    ) -> Result<Generator, Error> {
        let level = *pack
            .dimension_types
            .get(&crate::function::qualify(dimension_type))
            .ok_or_else(|| Error::Invalid(format!("unknown dimension type {dimension_type}")))?;
        Self::for_level(pack, settings, biome_source, level, seed)
    }

    fn for_level(pack: &Datapack, settings: &str, biome_source: BiomeSourceKind, level: (i32, i32), seed: i64) -> Result<Generator, Error> {
        let s = pack.settings(settings)?;
        // `NoiseSettings.clampToHeightAccessor`.
        let gen_min_y = s.min_y.max(level.0);
        let gen_height = (s.min_y + s.height).min(level.0 + level.1) - gen_min_y;
        let biomes: Vec<BiomeInfo> =
            pack.biomes.iter().map(|(id, json)| BiomeInfo::parse(id, json).map_err(|e| e.context(id))).collect::<Result<_, _>>()?;
        let index: HashMap<&str, u16> = biomes.iter().enumerate().map(|(i, b)| (b.name.as_str(), i as u16)).collect();
        let biome = |name: &str| index.get(name).copied().ok_or_else(|| Error::Invalid(format!("unknown biome {name}")));
        let source = match biome_source {
            BiomeSourceKind::MultiNoise(id) => {
                let list = pack
                    .parameter_lists
                    .get(&crate::function::qualify(id))
                    .ok_or_else(|| Error::Invalid(format!("no parameter list {id} (reports/biome_parameters missing?)")))?;
                let values = list.iter().map(|(space, b)| Ok((*space, biome(b)?))).collect::<Result<Vec<_>, Error>>()?;
                BiomeSource::MultiNoise(ParameterList::new(values)?)
            }
            BiomeSourceKind::TheEnd => BiomeSource::TheEnd {
                end: biome("minecraft:the_end")?,
                highlands: biome("minecraft:end_highlands")?,
                midlands: biome("minecraft:end_midlands")?,
                islands: biome("minecraft:small_end_islands")?,
                barrens: biome("minecraft:end_barrens")?,
            },
        };

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
        roots.extend(s.spawn_target.iter().flatten().map(|(id, _)| *id));
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
        let densities: HashMap<_, _> = rule_densities.iter().copied().zip(compiled.by_ref()).collect();
        let spawn_target = s
            .spawn_target
            .iter()
            .map(|point| point.iter().map(|(_, p)| (compiled.next().expect("spawn target function"), *p)).collect())
            .collect();
        let region_random = state.factory().from_hash_of("minecraft:worldgen_region_random").fork_positional();
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
                min_y: gen_min_y,
                height: gen_height,
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
        let mut source_biomes: Vec<u16> = source.possible();
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
            min_y: level.0,
            height: level.1,
            gen_min_y,
            gen_height,
            sea_level: s.sea_level,
            disable_mob_generation: s.disable_mob_generation,
            biomes,
            source,
            climate,
            zoom_seed: obfuscate_seed(seed),
            seed,
            region_random,
            carvers,
            biome_carvers,
            uniform_carvers,
            uncarvable,
            spawn_target,
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

    /// `WorldGenerationContext` for this generator's level.
    pub fn gen_context(&self) -> GenContext {
        GenContext { min_y: self.gen_min_y, height: self.gen_height, sea_level: self.sea_level }
    }

    /// The multi-noise parameter list (`None` for other biome sources).
    pub fn parameters(&self) -> Option<&ParameterList<u16>> {
        match &self.source {
            BiomeSource::MultiNoise(list) => Some(list),
            BiomeSource::TheEnd { .. } => None,
        }
    }

    pub fn biome_source(&self) -> &BiomeSource {
        &self.source
    }

    /// `BiomeSource.possibleBiomes`, in the source's order.
    pub fn possible_biomes(&self) -> Vec<u16> {
        self.source.possible()
    }

    /// `ChunkGenerator.doCreateBiomes`: for multi-noise the climate on the chunk's quart grid
    /// in volume mode, then one parameter-list search per quart in `fillBiomesFromNoise` order
    /// (sections upward, then x, y, z), starting from an empty last result; other sources
    /// resolve each quart on its own (caching context).
    pub fn chunk_biomes(&self, s: &mut Scratch, cx: i32, cz: i32) -> Vec<u16> {
        s.reset_caches();
        if !matches!(self.source, BiomeSource::MultiNoise(_)) {
            let mut last: LastResult = None;
            let mut out = vec![0u16; self.sections() * 64];
            let (qx, qz, qy0) = (cx << 2, cz << 2, self.min_y >> 2);
            for section in 0..self.sections() {
                for i in 0..64 {
                    let (x, y, z) = (i & 3, i >> 4, (i >> 2) & 3);
                    out[section * 64 + i as usize] = self.point_biome(s, &mut last, qx + x, qy0 + section as i32 * 4 + y, qz + z);
                }
            }
            return out;
        }
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
                        out[section * 64 + ((y << 4) | (z << 2) | x) as usize] = self.find(&t, &mut last);
                    }
                }
            }
        }
        out
    }

    /// Generates a chunk through the BIOMES status.
    pub fn new_chunk(&self, gs: &mut GenScratch, cx: i32, cz: i32) -> ProtoChunk {
        let biomes = gs.biomes.get(self, cx, cz).to_vec();
        ProtoChunk::new(cx, cz, self.min_y, self.sections(), biomes)
    }

    /// Generates a chunk through BIOMES and TERRAIN, without structures.
    pub fn generate(&self, gs: &mut GenScratch, cx: i32, cz: i32) -> ProtoChunk {
        self.generate_with(gs, cx, cz, None)
    }

    /// Generates a chunk through BIOMES and TERRAIN, its terrain adapted to nearby structures
    /// by `beard` (`Beardifier.forStructuresInChunk`; `None` is `Beardifier.EMPTY`).
    pub fn generate_with(
        &self,
        gs: &mut GenScratch,
        cx: i32,
        cz: i32,
        beard: Option<std::sync::Arc<crate::structure::beard::Beardifier>>,
    ) -> ProtoChunk {
        let mut chunk = self.new_chunk(gs, cx, cz);
        gs.noise_context.beard = beard;
        self.run_steps(gs, &mut chunk, &mut |_, _| {});
        gs.noise_context.beard = None;
        chunk.finish_terrain();
        chunk
    }

    /// Runs the TERRAIN steps on a chunk that passed BIOMES, calling `after` after each.
    pub fn run_steps(&self, gs: &mut GenScratch, chunk: &mut ProtoChunk, after: &mut dyn FnMut(Step, &ProtoChunk)) {
        let vol = Volume::blocks([16, self.gen_height, 16], [chunk.x << 4, self.gen_min_y, chunk.z << 4]);
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
        let mut possible = vec![false; self.biomes.len()];
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &b in biomes.get(self, chunk.x + dx, chunk.z + dz) {
                    possible[b as usize] = true;
                }
            }
        }
        self.material.build_surface(s, &mut |x, y, z| biomes.zoomed(self, x, y, z), &possible, chunk);
        after(Step::Surface, chunk);

        self.carve(s, &mut gs.point_context, &mut aquifer, chunk);
        after(Step::Carvers, chunk);
    }

    /// `NoiseBasedChunkGenerator.iterateNoiseColumn`: the terrain of one column as TERRAIN's
    /// fill step would place it (final density over a 1×height×1 volume, then the aquifer),
    /// before surface rules, carvers and structures. States from `gen_min_y` up (the
    /// generation depth; vanilla's `NoiseColumn` reads air outside it).
    pub fn base_column(&self, s: &mut Scratch, x: i32, z: i32) -> Vec<u16> {
        let vol = Volume::blocks([1, self.gen_height, 1], [x, self.gen_min_y, z]);
        s.reset_caches();
        let mut aquifer = match &self.aquifer {
            Some(f) => Aquifer::Noise(Box::new(NoiseAquifer::new(f, self.fluid_picker, s, &vol))),
            None => Aquifer::Disabled(self.fluid_picker),
        };
        let mut density = vec![0f32; vol.len()];
        self.final_density.fill(s, &vol, &mut density);
        let mut out = vec![state::AIR; self.gen_height as usize];
        for yi in (0..vol.size[1]).rev() {
            let y = vol.block_y(yi);
            out[yi as usize] = aquifer.compute_substance(s, x, y, z, density[vol.index(0, yi, 0)] as f64).unwrap_or(self.default_block);
        }
        out
    }

    /// `ChunkGenerator.getBaseHeight`: one above the highest block of [`Self::base_column`]
    /// counting for the heightmap (`min_y` if none). Vanilla samples the column top down and
    /// stops at the first match; the values sampled are the same.
    pub fn base_height(&self, s: &mut Scratch, x: i32, z: i32, map: crate::proto::Heightmap) -> i32 {
        let column = self.base_column(s, x, z);
        let bit = crate::proto::heightmap_bit(map);
        column
            .iter()
            .rposition(|&b| crate::proto::heightmap_flags(b) & bit != 0)
            .map_or(self.gen_min_y, |i| self.gen_min_y + i as i32 + 1)
    }

    /// `MultiNoiseBiomeSource.createResolverForChunk`'s sampling: the six climate values on a
    /// volume of quarts (`size` quarts from quart `q`), in volume order.
    pub fn climate_volume(&self, s: &mut Scratch, q: [i32; 3], size: [i32; 3]) -> (Volume, [Vec<f32>; 6]) {
        let vol = Volume::new(size, [q[0] << 2, q[1] << 2, q[2] << 2], [4, 4, 4]);
        let climate = std::array::from_fn(|i| {
            let mut b = vec![0f32; vol.len()];
            self.climate[i].fill(s, &vol, &mut b);
            b
        });
        (vol, climate)
    }

    fn find(&self, t: &crate::biome::Target, last: &mut LastResult) -> u16 {
        match &self.source {
            BiomeSource::MultiNoise(list) => *list.find(t, last),
            BiomeSource::TheEnd { .. } => panic!("the end biome source has no parameter list"),
        }
    }

    /// The biomes of the quarts `q0..=q1` of a quart column, as `couldStructureExistInColumn`
    /// samples them (multi-noise: the climate in volume mode, then the parameter list).
    pub fn column_biomes(&self, s: &mut Scratch, last: &mut LastResult, qx: i32, qz: i32, q0: i32, q1: i32) -> Vec<u16> {
        if !matches!(self.source, BiomeSource::MultiNoise(_)) {
            return (q0..=q1).map(|qy| self.point_biome(s, last, qx, qy, qz)).collect();
        }
        let (vol, c) = self.climate_volume(s, [qx, q0, qz], [1, q1 - q0 + 1, 1]);
        (q0..=q1)
            .map(|qy| {
                let i = vol.index(0, qy - q0, 0);
                self.find(&target(c[0][i], c[1][i], c[2][i], c[3][i], c[4][i], c[5][i]), last)
            })
            .collect()
    }

    /// The biome of a quart from point-sampled climate (`createResolver`/
    /// `createUncachedResolver`); `TheEndBiomeSource.getNoiseBiome` for the end.
    pub fn point_biome(&self, s: &mut Scratch, last: &mut LastResult, qx: i32, qy: i32, qz: i32) -> u16 {
        let (x, y, z) = (qx << 2, qy << 2, qz << 2);
        if let BiomeSource::TheEnd { end, highlands, midlands, islands, barrens } = self.source {
            let (sx, sz) = (x >> 4, z >> 4);
            if (sx as i64) * (sx as i64) + (sz as i64) * (sz as i64) <= 4096 {
                return end;
            }
            let e = self.climate[EROSION].point(s, (sx * 2 + 1) * 8, y, (sz * 2 + 1) * 8) as f64;
            return if e > 0.25 {
                highlands
            } else if e >= -0.0625 {
                midlands
            } else if e < -0.21875 {
                islands
            } else {
                barrens
            };
        }
        let c: Vec<f32> = self.climate.iter().map(|f| f.point(s, x, y, z)).collect();
        self.find(&target(c[0], c[1], c[2], c[3], c[4], c[5]), last)
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
        let g = self.gen_context();
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
                if aquifer.should_schedule_fluid_update() && has_fluid(new) {
                    chunk.mark_post_processing(bx, y, bz);
                }
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
                        if has_fluid(top) {
                            chunk.mark_post_processing(bx, y - 1, bz);
                        }
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
                        if aquifer.should_schedule_fluid_update() && has_fluid(block) {
                            chunk.mark_post_processing(bx, by, bz);
                        }
                    }
                }
            }
        }
    }
}
