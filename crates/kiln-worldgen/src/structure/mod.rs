//! Structures: structure sets and their placement, the per-chunk structure starts
//! (STRUCTURE_STARTS, `ChunkGenerator.createStructures`), references (STRUCTURE_REFERENCES)
//! and placing pieces during FEATURES.
//!
//! A chunk's starts are a function of the seed and the chunk position, so they are computed
//! on demand and cached ([`StartCache`]). Structure types plug in through [`kinds`]; types Kiln
//! does not implement generate nothing and are counted.

pub mod bbox;
pub mod beard;
pub mod jigsaw;
pub mod kinds;
pub mod legacy;
pub mod locate;
pub mod longset;
pub mod piece;
pub mod placement;
pub mod processor;
pub mod shape;
pub mod template;
pub mod templated;
pub mod transform;

use crate::Error;
use crate::biome::LastResult;
use crate::decorate::STEPS;
use crate::generator::Generator;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::sampler::Scratch;
use crate::sets::{BiomeSet, Loader};
use bbox::BoundingBox;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use piece::Piece;
use placement::Placement;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// `TerrainAdjustment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainAdjustment {
    None,
    Bury,
    BeardThin,
    BeardBox,
    Encapsulate,
}

/// A structure (`worldgen/structure`): common settings plus its type.
pub struct StructureDef {
    pub name: String,
    pub biomes: BiomeSet,
    /// Generation step (index into `GenerationStep.Decoration`) its pieces are placed in.
    pub step: usize,
    pub terrain_adaptation: TerrainAdjustment,
    pub kind: Box<dyn kinds::Kind>,
}

/// A structure set: a placement and the structures it chooses from.
pub struct StructureSet {
    pub name: String,
    pub placement: Placement,
    /// (structure index, weight).
    pub structures: Vec<(usize, i32)>,
}

/// A structure start: the pieces of one structure generated from a start chunk.
#[derive(Debug)]
pub struct Start {
    pub structure: usize,
    pub chunk: (i32, i32),
    pub references: i32,
    pub pieces: Vec<Box<dyn Piece>>,
    /// `getBoundingBox`: the pieces' box, grown by 12 for terrain adaptation.
    pub bbox: BoundingBox,
}

impl Start {
    /// `StructureStart.placeInChunk`: places the pieces reaching `chunk_box` (the chunk's
    /// writable area), pivoting on the first piece.
    pub fn place_in_chunk(
        &self,
        cx: &piece::PlaceContext,
        r: &mut crate::region::Region,
        random: &mut WorldgenRandom,
        chunk_box: &BoundingBox,
        chunk: (i32, i32),
    ) {
        let Some(first) = self.pieces.first() else { return };
        let b = first.base().bbox;
        let c = b.center();
        let pivot = BlockPos::new(c.x, b.min_y, c.z);
        for p in &self.pieces {
            if p.base().bbox.intersects(chunk_box) {
                p.place(cx, r, random, chunk_box, chunk, pivot);
            }
        }
        cx.structures.structures[self.structure].kind.after_place(cx, r, random, chunk_box, chunk, self);
    }

    /// `StructureStart.createTag`.
    pub fn save(&self, structures: &Structures) -> Tag {
        Tag::Compound(vec![
            ("id".into(), Tag::String(structures.structures[self.structure].name.clone())),
            ("ChunkX".into(), Tag::Int(self.chunk.0)),
            ("ChunkZ".into(), Tag::Int(self.chunk.1)),
            ("references".into(), Tag::Int(self.references)),
            ("Children".into(), Tag::List(self.pieces.iter().map(|p| p.save()).collect())),
        ])
    }
}

/// The union of piece boxes (`StructurePiece.createBoundingBox`).
pub fn pieces_bbox(pieces: &[Box<dyn Piece>]) -> Option<BoundingBox> {
    let mut it = pieces.iter();
    let mut b = it.next()?.base().bbox;
    for p in it {
        b.encapsulate(&p.base().bbox);
    }
    Some(b)
}

/// All structures and structure sets of a datapack, for one world.
pub struct Structures {
    pub structures: Vec<StructureDef>,
    ids: HashMap<String, usize>,
    pub sets: Vec<StructureSet>,
    /// Sets that can generate with the biome source (`possibleStructureSets`), in order.
    possible: Vec<usize>,
    /// Structure indices per generation step, registry order (`applyBiomeDecoration`).
    pub by_step: Vec<Vec<usize>>,
    pub seed: i64,
    /// Unimplemented structure types: attempts skipped.
    gaps: Vec<(String, AtomicU64)>,
    rings: placement::Rings,
    /// `StructureTemplateManager`.
    pub templates: Arc<template::TemplateManager>,
}

impl Structures {
    pub fn load(generator: &Generator, l: &Loader) -> Result<Structures, Error> {
        let empty = Vec::new();
        let structure_json = l.pack.registries.get("structure").unwrap_or(&empty);
        let mut gaps: Vec<(String, AtomicU64)> = Vec::new();
        let mut structures = Vec::new();
        let mut ids = HashMap::new();
        let jigsaw_pools = jigsaw::LoadScope::new(l)?;
        for (id, json) in structure_json {
            let def = parse_structure(id, json, l, &mut gaps).map_err(|e| e.context(id))?;
            ids.insert(id.clone(), structures.len());
            structures.push(def);
        }
        drop(jigsaw_pools);
        let set_json = l.pack.registries.get("structure_set").unwrap_or(&empty);
        let set_ids: HashMap<&str, usize> = set_json.iter().enumerate().map(|(i, (id, _))| (id.as_str(), i)).collect();
        let mut sets = Vec::new();
        for (id, json) in set_json {
            let placement = Placement::parse(json.get("placement").ok_or_else(|| Error::Invalid("set without placement".into()))?, l, &set_ids)
                .map_err(|e| e.context(id))?;
            let entries = json
                .get("structures")
                .and_then(Json::as_array)
                .ok_or_else(|| Error::Invalid(format!("{id}: set without structures")))?
                .iter()
                .map(|e| {
                    let name = e.get("structure").and_then(Json::as_str).unwrap_or("");
                    let i = *ids.get(&crate::function::qualify(name)).ok_or_else(|| Error::Invalid(format!("unknown structure {name}")))?;
                    Ok((i, e.get("weight").and_then(Json::as_i32).unwrap_or(1)))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            sets.push(StructureSet { name: id.clone(), placement, structures: entries });
        }
        // Biomes the source can produce.
        let mut possible_biomes = vec![false; generator.biomes.len()];
        for b in generator.possible_biomes() {
            possible_biomes[b as usize] = true;
        }
        let possible: Vec<usize> = (0..sets.len())
            .filter(|&i| {
                sets[i].structures.iter().any(|&(s, _)| (0..generator.biomes.len()).any(|b| possible_biomes[b] && structures[s].biomes.contains(b as u16)))
            })
            .collect();
        let mut by_step = vec![Vec::new(); STEPS];
        for (i, s) in structures.iter().enumerate() {
            by_step[s.step].push(i);
        }
        let rings = placement::Rings::new(&sets, &structures, &possible_biomes);
        let templates = Arc::new(template::TemplateManager::near(&l.pack.root));
        Ok(Structures { structures, ids, sets, possible, by_step, seed: generator.seed, gaps, rings, templates })
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.ids.get(name).copied()
    }

    /// Skipped attempts per unimplemented structure type.
    pub fn gaps(&self) -> BTreeMap<String, u64> {
        self.gaps.iter().map(|(n, c)| (n.clone(), c.load(Ordering::Relaxed))).filter(|(_, c)| *c > 0).collect()
    }

    /// `StructurePlacement.isStructureChunk` for set `set`.
    pub fn is_structure_chunk(&self, generator: &Generator, set: usize, cx: i32, cz: i32) -> bool {
        self.sets[set].placement.is_structure_chunk(self, generator, set, cx, cz)
    }

    /// Ring positions of a concentric-rings set (computed once).
    pub fn ring_positions(&self, generator: &Generator, set: usize) -> Option<&[(i32, i32)]> {
        self.rings.positions(self, generator, set)
    }

    /// `ChunkGenerator.createStructures`: the starts generated from chunk `(cx, cz)`, in set
    /// order. Every attempt of a chunk shares one caching climate sampler and biome search
    /// history, as vanilla's `createStructures` call does (Kiln starts each chunk's history
    /// empty; vanilla's is a thread-local).
    pub fn create_starts(&self, generator: &Generator, scratch: &mut StructureScratch, cx: i32, cz: i32) -> Vec<Start> {
        scratch.climate.reset_caches();
        let mut last: LastResult = None;
        let mut out: Vec<Start> = Vec::new();
        for &set in &self.possible {
            let s = &self.sets[set];
            if s.structures.iter().any(|&(st, _)| out.iter().any(|o| o.structure == st)) {
                continue;
            }
            if !self.is_structure_chunk(generator, set, cx, cz) {
                continue;
            }
            if s.structures.len() == 1 {
                if let Some(start) = self.try_generate(generator, scratch, &mut last, s.structures[0].0, cx, cz) {
                    out.push(start);
                }
                continue;
            }
            let mut list = s.structures.clone();
            let mut random = WorldgenRandom::legacy(0);
            random.set_large_feature_seed(self.seed, cx, cz);
            let mut total: i32 = list.iter().map(|e| e.1).sum();
            while !list.is_empty() {
                let mut r = random.next_int_bounded(total);
                let mut i = 0;
                for e in &list {
                    r -= e.1;
                    if r < 0 {
                        break;
                    }
                    i += 1;
                }
                let (st, w) = list[i];
                if let Some(start) = self.try_generate(generator, scratch, &mut last, st, cx, cz) {
                    out.push(start);
                    break;
                }
                list.remove(i);
                total -= w;
            }
        }
        out
    }

    /// `ChunkGenerator.tryGenerateStructure` / `Structure.generate`.
    fn try_generate(&self, generator: &Generator, scratch: &mut StructureScratch, last: &mut LastResult, st: usize, cx: i32, cz: i32) -> Option<Start> {
        self.generate_at(generator, scratch, last, st, cx, cz, true)
    }

    /// `Structure.generate(...)` with `biome -> true`, as `/place structure` calls it: the start
    /// of structure `st` generated from chunk `(cx, cz)` wherever its biomes are not.
    pub fn generate_anywhere(&self, generator: &Generator, scratch: &mut StructureScratch, st: usize, cx: i32, cz: i32) -> Option<Start> {
        scratch.climate.reset_caches();
        let mut last: LastResult = None;
        self.generate_at(generator, scratch, &mut last, st, cx, cz, false)
    }

    /// `JigsawPlacement.generateJigsaw`'s pieces (`/place jigsaw`): `pool` grown from its
    /// `target` jigsaw at `pos` to `max_depth`, with the generation random of the chunk holding
    /// `pos`. `like` names a jigsaw structure whose loaded pools are used.
    pub fn generate_jigsaw(
        &self,
        generator: &Generator,
        scratch: &mut StructureScratch,
        pool: &str,
        target: &str,
        max_depth: i32,
        pos: BlockPos,
    ) -> Option<Vec<Box<dyn Piece>>> {
        let (st, base) = self.structures.iter().enumerate().find_map(|(i, d)| d.kind.as_jigsaw().map(|j| (i, j)))?;
        let s = jigsaw::JigsawStructure {
            pools: base.pools.clone(),
            start_pool: pool.to_owned(),
            start_jigsaw_name: Some(target.to_owned()),
            max_depth,
            start_height: base.start_height.clone(),
            use_expansion_hack: false,
            project_start_to_heightmap: None,
            max_distance: (128, 128),
            aliases: Vec::new(),
            padding: None,
            liquid: crate::structure::template::LiquidSettings::ApplyWaterlogging,
        };
        scratch.climate.reset_caches();
        let mut last: LastResult = None;
        let (cx, cz) = (pos.x >> 4, pos.z >> 4);
        let mut random = WorldgenRandom::legacy(0);
        random.set_large_feature_seed(self.seed, cx, cz);
        let mut ctx = GenCtx {
            generator,
            structures: self,
            structure: st,
            seed: self.seed,
            chunk: (cx, cz),
            random,
            climate: &mut scratch.climate,
            last: &mut last,
            heights: &mut scratch.heights,
        };
        let stub = jigsaw::placement::add_pieces(&s, &mut ctx, pos)?;
        let mut pieces: Vec<Box<dyn Piece>> = Vec::new();
        (stub.build)(&mut ctx, &mut pieces);
        (!pieces.is_empty()).then_some(pieces)
    }

    fn generate_at(
        &self,
        generator: &Generator,
        scratch: &mut StructureScratch,
        last: &mut LastResult,
        st: usize,
        cx: i32,
        cz: i32,
        check_biome: bool,
    ) -> Option<Start> {
        let def = &self.structures[st];
        if let Some(gap) = def.kind.gap() {
            self.gaps[gap].1.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let mut random = WorldgenRandom::legacy(0);
        random.set_large_feature_seed(self.seed, cx, cz);
        let mut ctx = GenCtx {
            generator,
            structures: self,
            structure: st,
            seed: self.seed,
            chunk: (cx, cz),
            random,
            climate: &mut scratch.climate,
            last,
            heights: &mut scratch.heights,
        };
        let stub = def.kind.find(&mut ctx)?;
        if check_biome && !ctx.is_valid_biome(stub.pos) {
            return None;
        }
        let mut pieces: Vec<Box<dyn Piece>> = Vec::new();
        (stub.build)(&mut ctx, &mut pieces);
        let b = pieces_bbox(&pieces)?;
        let bbox = if def.terrain_adaptation != TerrainAdjustment::None { b.inflated(12, 12, 12) } else { b };
        Some(Start { structure: st, chunk: (cx, cz), references: 0, pieces, bbox })
    }
}

/// Per-thread working memory of structure generation.
pub struct StructureScratch {
    /// The caching climate sampler of one `createStructures` call.
    pub climate: Scratch,
    /// For base heights (each `iterateNoiseColumn` gets a fresh caching context).
    pub heights: Scratch,
}

impl Default for StructureScratch {
    fn default() -> Self {
        Self { climate: Scratch::caching(), heights: Scratch::caching() }
    }
}

/// `Structure.GenerationStub`: where a structure starts and how to build its pieces.
pub struct Stub<'k> {
    pub pos: BlockPos,
    #[allow(clippy::type_complexity)]
    pub build: Box<dyn FnOnce(&mut GenCtx, &mut Vec<Box<dyn Piece>>) + 'k>,
}

/// `Structure.GenerationContext`.
pub struct GenCtx<'a> {
    pub generator: &'a Generator,
    pub structures: &'a Structures,
    pub structure: usize,
    pub seed: i64,
    pub chunk: (i32, i32),
    /// `WorldgenRandom` with `setLargeFeatureSeed(seed, chunkX, chunkZ)`.
    pub random: WorldgenRandom,
    climate: &'a mut Scratch,
    last: &'a mut LastResult,
    heights: &'a mut Scratch,
}

impl GenCtx<'_> {
    pub fn min_y(&self) -> i32 {
        self.generator.min_y
    }

    pub fn max_y(&self) -> i32 {
        self.generator.min_y + self.generator.height - 1
    }

    /// `ChunkGenerator.getFirstFreeHeight` (= `getBaseHeight`).
    pub fn first_free_height(&mut self, x: i32, z: i32, map: Heightmap) -> i32 {
        self.generator.base_height(self.heights, x, z, map)
    }

    /// `ChunkGenerator.getFirstOccupiedHeight`.
    pub fn first_occupied_height(&mut self, x: i32, z: i32, map: Heightmap) -> i32 {
        self.first_free_height(x, z, map) - 1
    }

    /// `ChunkGenerator.getBaseColumn`: states from `min_y` up.
    pub fn base_column(&mut self, x: i32, z: i32) -> Vec<u16> {
        self.generator.base_column(self.heights, x, z)
    }

    /// The biome at a quart from the caching climate sampler (`BiomeSource.createResolver`).
    pub fn biome(&mut self, qx: i32, qy: i32, qz: i32) -> u16 {
        self.generator.point_biome(self.climate, self.last, qx, qy, qz)
    }

    /// `GenerationContext.isValidBiome`: the structure's biomes contain the biome at `p`.
    pub fn is_valid_biome(&mut self, p: BlockPos) -> bool {
        let b = self.biome(p.x >> 2, p.y >> 2, p.z >> 2);
        self.structures.structures[self.structure].biomes.contains(b)
    }

    /// `couldStructureExistInColumn(x, z, minY, maxY)`: any valid biome in the column, from a
    /// volume sample of its quarts.
    pub fn could_exist_in_column(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let (qx, qz, q0, q1) = (x >> 2, z >> 2, min_y >> 2, max_y >> 2);
        let biomes = self.generator.column_biomes(self.climate, self.last, qx, qz, q0, q1);
        biomes.into_iter().any(|b| self.structures.structures[self.structure].biomes.contains(b))
    }

    /// `couldValidBiomeExistOnTopOfChunkCenter`.
    pub fn could_exist_at_chunk_center(&mut self) -> bool {
        let (x, z) = ((self.chunk.0 << 4) + 8, (self.chunk.1 << 4) + 8);
        let (lo, hi) = (self.min_y() - 1, self.max_y());
        self.could_exist_in_column(x, z, lo, hi)
    }

    /// `Structure.onTopOfChunkCenter`: at the chunk center on the heightmap, if a valid biome
    /// could exist in that column.
    pub fn on_top_of_chunk_center(&mut self, map: Heightmap) -> Option<BlockPos> {
        if !self.could_exist_at_chunk_center() {
            return None;
        }
        Some(self.chunk_center_on(map))
    }

    /// `onTopOfChunkCenterWithoutBiomeCheck`.
    pub fn chunk_center_on(&mut self, map: Heightmap) -> BlockPos {
        let (x, z) = ((self.chunk.0 << 4) + 8, (self.chunk.1 << 4) + 8);
        let y = self.first_occupied_height(x, z, map);
        BlockPos::new(x, y, z)
    }

    fn corner_heights(&mut self, x: i32, w: i32, z: i32, d: i32) -> [i32; 4] {
        [
            self.first_occupied_height(x, z, Heightmap::WorldSurfaceWg),
            self.first_occupied_height(x, z + d, Heightmap::WorldSurfaceWg),
            self.first_occupied_height(x + w, z, Heightmap::WorldSurfaceWg),
            self.first_occupied_height(x + w, z + d, Heightmap::WorldSurfaceWg),
        ]
    }

    /// `Structure.getMeanFirstOccupiedHeight`.
    pub fn mean_first_occupied_height(&mut self, x: i32, w: i32, z: i32, d: i32) -> i32 {
        let h = self.corner_heights(x, w, z, d);
        (h[0] + h[1] + h[2] + h[3]) / 4
    }

    /// `Structure.getLowestY(context, width, depth)` from the chunk's corner.
    pub fn lowest_y(&mut self, w: i32, d: i32) -> i32 {
        let (x, z) = (self.chunk.0 << 4, self.chunk.1 << 4);
        self.lowest_y_at(x, z, w, d)
    }

    /// `Structure.getLowestY(context, x, z, width, depth)`.
    pub fn lowest_y_at(&mut self, x: i32, z: i32, w: i32, d: i32) -> i32 {
        let h = self.corner_heights(x, w, z, d);
        h[0].min(h[1]).min(h[2].min(h[3]))
    }

    /// `Structure.getLowestYIn5by5Box`.
    pub fn lowest_y_in_5x5(&mut self, rotation: transform::Rotation) -> BlockPos {
        let (mut w, mut d) = (5, 5);
        match rotation {
            transform::Rotation::Clockwise90 => w = -5,
            transform::Rotation::Clockwise180 => {
                w = -5;
                d = -5;
            }
            transform::Rotation::CounterClockwise90 => d = -5,
            transform::Rotation::None => {}
        }
        let (x, z) = ((self.chunk.0 << 4) + 7, (self.chunk.1 << 4) + 7);
        let y = self.lowest_y_at(x, z, w, d);
        BlockPos::new(x, y, z)
    }
}

/// Reads the settings common to all structures and hands the rest to the type.
fn parse_structure(id: &str, json: &Json, l: &Loader, gaps: &mut Vec<(String, AtomicU64)>) -> Result<StructureDef, Error> {
    let ty = json.get("type").and_then(Json::as_str).ok_or_else(|| Error::Invalid("structure without type".into()))?;
    let step_name = json.get("step").and_then(Json::as_str).unwrap_or("surface_structures");
    let step = [
        "raw_generation",
        "lakes",
        "local_modifications",
        "underground_structures",
        "surface_structures",
        "strongholds",
        "underground_ores",
        "underground_decoration",
        "fluid_springs",
        "vegetal_decoration",
        "top_layer_modification",
    ]
    .iter()
    .position(|s| *s == step_name)
    .ok_or_else(|| Error::Invalid(format!("unknown step {step_name}")))?;
    let terrain_adaptation = match json.get("terrain_adaptation").and_then(Json::as_str).unwrap_or("none") {
        "none" => TerrainAdjustment::None,
        "bury" => TerrainAdjustment::Bury,
        "beard_thin" => TerrainAdjustment::BeardThin,
        "beard_box" => TerrainAdjustment::BeardBox,
        "encapsulate" => TerrainAdjustment::Encapsulate,
        t => return Err(Error::Invalid(format!("unknown terrain adaptation {t}"))),
    };
    let biomes = l.biomes(json.get("biomes").ok_or_else(|| Error::Invalid("structure without biomes".into()))?)?;
    let ty = ty.strip_prefix("minecraft:").unwrap_or(ty);
    let kind = match kinds::parse(ty, json, l)? {
        Some(k) => k,
        None => {
            let name = format!("minecraft:{ty}");
            let i = gaps.iter().position(|(n, _)| *n == name).unwrap_or_else(|| {
                gaps.push((name, AtomicU64::new(0)));
                gaps.len() - 1
            });
            Box::new(kinds::Unsupported(i))
        }
    };
    Ok(StructureDef { name: id.to_string(), biomes, step, terrain_adaptation, kind })
}

/// Starts of each chunk, computed on demand and kept (a pure function of the position).
/// A chunk's starts, shared between the cache and the chunks referencing them.
pub type SharedStarts = Arc<Vec<Start>>;

#[derive(Default)]
pub struct StartCache {
    starts: Mutex<HashMap<(i32, i32), SharedStarts>>,
}

impl StartCache {
    pub fn get(&self, structures: &Structures, generator: &Generator, scratch: &mut StructureScratch, cx: i32, cz: i32) -> Arc<Vec<Start>> {
        if let Some(s) = self.starts.lock().unwrap().get(&(cx, cz)) {
            return s.clone();
        }
        let starts = Arc::new(structures.create_starts(generator, scratch, cx, cz));
        self.starts.lock().unwrap().entry((cx, cz)).or_insert(starts).clone()
    }

    pub fn len(&self) -> usize {
        self.starts.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The starts a chunk references, per structure, in placement order
/// (`StructureManager.startsForStructure`).
#[derive(Default, Clone)]
pub struct ChunkStarts {
    by_structure: Vec<(usize, Vec<(SharedStarts, usize)>)>,
}

impl ChunkStarts {
    /// Resolves a chunk's references to the starts (each start chunk has at most one start
    /// per structure).
    pub fn new(structures: &Structures, generator: &Generator, cache: &StartCache, scratch: &mut StructureScratch, cx: i32, cz: i32) -> Self {
        let refs = references(structures, generator, cache, scratch, cx, cz);
        let by_structure = refs
            .into_iter()
            .map(|(st, list)| {
                let starts = list
                    .into_iter()
                    .filter_map(|(sx, sz)| {
                        let all = cache.get(structures, generator, scratch, sx, sz);
                        let i = all.iter().position(|s| s.structure == st)?;
                        Some((all, i))
                    })
                    .collect();
                (st, starts)
            })
            .collect();
        ChunkStarts { by_structure }
    }

    /// Starts of structure `st` reaching the chunk.
    pub fn of(&self, st: usize) -> impl Iterator<Item = &Start> {
        self.by_structure.iter().filter(move |(s, _)| *s == st).flat_map(|(_, v)| v.iter().map(|(a, i)| &a[*i]))
    }

    /// Every referenced start (for the beardifier).
    pub fn all(&self) -> impl Iterator<Item = &Start> {
        self.by_structure.iter().flat_map(|(_, v)| v.iter().map(|(a, i)| &a[*i]))
    }

    /// References as saved in chunk NBT (`structures.References`): structure id and packed
    /// start chunk positions.
    pub fn references(&self, structures: &Structures) -> Vec<(String, Vec<i64>)> {
        self.by_structure
            .iter()
            .map(|(st, v)| {
                (
                    structures.structures[*st].name.clone(),
                    v.iter().map(|(a, i)| (a[*i].chunk.0 as i64 & 0xFFFF_FFFF) | ((a[*i].chunk.1 as i64 & 0xFFFF_FFFF) << 32)).collect(),
                )
            })
            .collect()
    }
}

/// `ChunkGenerator.createReferences`: per structure, the start chunks within 8 whose box
/// reaches this chunk, in vanilla's `LongOpenHashSet` iteration order.
pub fn references(
    structures: &Structures,
    generator: &Generator,
    cache: &StartCache,
    scratch: &mut StructureScratch,
    cx: i32,
    cz: i32,
) -> Vec<(usize, Vec<(i32, i32)>)> {
    let (x0, z0) = (cx << 4, cz << 4);
    let mut sets: Vec<(usize, longset::LongSet)> = Vec::new();
    for sx in cx - 8..=cx + 8 {
        for sz in cz - 8..=cz + 8 {
            let starts = cache.get(structures, generator, scratch, sx, sz);
            for s in starts.iter() {
                if s.bbox.intersects_xz(x0, z0, x0 + 15, z0 + 15) {
                    let packed = (sx as i64 & 0xFFFF_FFFF) | ((sz as i64 & 0xFFFF_FFFF) << 32);
                    match sets.iter_mut().find(|(st, _)| *st == s.structure) {
                        Some((_, set)) => {
                            set.add(packed);
                        }
                        None => {
                            let mut set = longset::LongSet::default();
                            set.add(packed);
                            sets.push((s.structure, set));
                        }
                    }
                }
            }
        }
    }
    sets.into_iter()
        .map(|(st, set)| (st, set.iter().map(|p| (p as i32, (p >> 32) as i32)).collect()))
        .collect()
}
