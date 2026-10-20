//! The generation status DAG (vanilla 26.3 `ChunkStatus`/`ChunkPyramid`) as a thread-safe
//! scheduler that owns proto-chunks and hands out finished ones.
//!
//! | status | work | needs |
//! |---|---|---|
//! | STRUCTURE_STARTS, STRUCTURE_REFERENCES | (structures: not generated yet) | starts within 8 |
//! | BIOMES | [`Generator::chunk_biomes`] (pure; neighbour biomes are recomputed on demand) | — |
//! | TERRAIN | fill, surface, carvers ([`Generator::generate`]) | BIOMES within 1 |
//! | FEATURES | [`Decorator::decorate`] on the 3×3 window (write radius 1) | TERRAIN within 1 |
//! | INITIALIZE_LIGHT, LIGHT | sky light from heights ([`kiln_world::chunk::Chunk::new`]) | — |
//! | SPAWN | (initial mobs: not generated) | — |
//! | FULL | conversion to a [`kiln_world::chunk::Chunk`] | FEATURES within 1 |
//!
//! Everything is a function of the seed and the chunk position: TERRAIN is computed per chunk
//! with no shared state, and FEATURES runs in Kiln's canonical order ([`crate::order`]): a
//! chunk is decorated only after the lower-ranked chunks within distance 2, and while it is
//! the only decoration touching its 3×3 window. Threads calling [`Pipeline::full`] share the
//! work: each runs the steps it needs on its own thread and waits when another thread holds
//! a chunk it needs. Nothing here reads simulation state.
//!
//! A chunk is final once all chunks within distance 1 are decorated; [`Pipeline::full`] then
//! removes it from the pipeline and returns it. Asking again for a chunk already handed out
//! recomputes it from scratch in a private pipeline (same result, since generation is a
//! pure function of the position).

use crate::Error;
use crate::datapack::Datapack;
use crate::decorate::Decorator;
use crate::generator::{BiomeSourceKind, GenScratch, Generator};
use crate::order;
use crate::proto::{ProtoChunk, Status};
use crate::region::Region;
use crate::sets::Loader;
use crate::stem::{Stem, StemGenerator};
use crate::structure::beard::Beardifier;
use crate::structure::{ChunkStarts, StartCache, Structures};
use kiln_proto::nbt::Tag;
use std::collections::{HashMap, HashSet};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Everything overworld generation needs, shared by all generation threads.
pub struct Worldgen {
    pub generator: Generator,
    pub decorator: Decorator,
    pub structures: Structures,
    /// `WorldOptions.generateStructures`.
    pub generate_structures: bool,
}

impl Worldgen {
    /// The overworld of `pack` for `seed` (noise settings and biome source `minecraft:overworld`).
    pub fn overworld(pack: &Datapack, seed: i64, generate_structures: bool) -> Result<Worldgen, Error> {
        let generator = Generator::new(pack, "minecraft:overworld", "minecraft:overworld", seed)?;
        let loader = Loader::new(pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let decorator = Decorator::new(&generator, &loader)?;
        let structures = Structures::load(&generator, &loader)?;
        Ok(Worldgen { generator, decorator, structures, generate_structures })
    }

    /// The Nether of `pack` for `seed` (noise settings and multi-noise biome source
    /// `minecraft:nether`).
    pub fn nether(pack: &Datapack, seed: i64, generate_structures: bool) -> Result<Worldgen, Error> {
        let generator = Generator::for_dimension(pack, "minecraft:nether", BiomeSourceKind::MultiNoise("minecraft:nether"), "minecraft:the_nether", seed)?;
        let loader = Loader::new(pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let decorator = Decorator::new(&generator, &loader)?;
        let structures = Structures::load(&generator, &loader)?;
        Ok(Worldgen { generator, decorator, structures, generate_structures })
    }

    /// A noise-generated dimension of `pack`: `stem` (a parsed `LevelStem`, see [`Datapack::parse_stem`]) in the
    /// level `stem.dimension_type` describes, for `seed`. Flat and debug stems have no noise pipeline (`None`).
    pub fn from_stem(pack: &Datapack, stem: &Stem, seed: i64, generate_structures: bool) -> Result<Option<Worldgen>, Error> {
        if let StemGenerator::Flat(flat) = &stem.generator {
            return Self::flat(pack, flat, &stem.dimension_type, seed, generate_structures).map(Some);
        }
        let StemGenerator::Noise { settings, source } = &stem.generator else { return Ok(None) };
        let generator = Generator::for_dimension(pack, settings, source.kind(), &stem.dimension_type, seed)?;
        let loader = Loader::new(pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let decorator = Decorator::new(&generator, &loader)?;
        let structures = Structures::load(&generator, &loader)?;
        Ok(Some(Worldgen { generator, decorator, structures, generate_structures }))
    }

    /// A `minecraft:flat` dimension that generates more than its layers (see [`crate::stem::FlatSettings::needs_pipeline`]): the layers as the terrain,
    /// the biome's decoration as `features` and `lakes` ask, and the structure sets of `structure_overrides`.
    pub fn flat(pack: &Datapack, flat: &crate::stem::FlatSettings, dimension_type: &str, seed: i64, generate_structures: bool) -> Result<Worldgen, Error> {
        let layers: Vec<u16> = flat.layers.clone();
        let generator = Generator::for_flat(pack, &flat.biome, layers, dimension_type, seed)?;
        let loader = Loader::new(pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let void_gen = flat.layers.iter().all(|&s| s == crate::blocks::state::AIR) && flat.biome != "minecraft:the_void";
        let decorator = Decorator::new_flat(&generator, &loader, crate::decorate::FlatDecoration { features: flat.features, lakes: flat.lakes, void_gen })?;
        let structures = Structures::load_only(&generator, &loader, flat.structure_overrides.as_deref())?;
        Ok(Worldgen { generator, decorator, structures, generate_structures })
    }

    /// The End of `pack` for `seed` (noise settings `minecraft:end`, the end biome source).
    pub fn end(pack: &Datapack, seed: i64, generate_structures: bool) -> Result<Worldgen, Error> {
        let generator = Generator::for_dimension(pack, "minecraft:end", BiomeSourceKind::TheEnd, "minecraft:the_end", seed)?;
        let loader = Loader::new(pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let decorator = Decorator::new(&generator, &loader)?;
        let structures = Structures::load(&generator, &loader)?;
        Ok(Worldgen { generator, decorator, structures, generate_structures })
    }
}

/// A step [`Pipeline::full`] can take.
enum Work {
    Decorate((i32, i32)),
    Terrain((i32, i32)),
}

enum Slot {
    /// TERRAIN in progress on some thread.
    Generating,
    Ready(Box<ProtoChunk>),
    /// Part of a decoration window in progress.
    Borrowed,
}

#[derive(Default)]
struct State {
    chunks: HashMap<(i32, i32), Slot>,
    /// When each waiting chunk was last put back ([`State::ops`] then): the long-idle ones
    /// are parked (compressed).
    stamps: HashMap<(i32, i32), u64>,
    ops: u64,
    decorated: HashSet<(i32, i32)>,
    /// Handed out by [`Pipeline::full`].
    finished: HashSet<(i32, i32)>,
    /// Chunks whose terrain requests will soon need: threads that would wait generate these.
    wanted: VecDeque<(i32, i32)>,
}

/// Counts for monitoring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineStats {
    /// Proto-chunks held (terrain or decorated, not yet handed out).
    pub held: usize,
    pub decorated: usize,
    pub finished: usize,
}

/// Terrain prefetch radius around a requested chunk (see [`Pipeline::wait_or_help`]).
const PREFETCH: i32 = 3;

/// Thread time spent generating terrain, decorating and waiting for other threads (nanoseconds,
/// summed over every pipeline and thread), for load tools: how much of the generation threads'
/// time is work.
static TERRAIN_NS: AtomicU64 = AtomicU64::new(0);
static DECORATE_NS: AtomicU64 = AtomicU64::new(0);
static WAIT_NS: AtomicU64 = AtomicU64::new(0);

/// `(terrain, decorate, wait)` thread time so far.
pub fn timing() -> (Duration, Duration, Duration) {
    let d = |a: &AtomicU64| Duration::from_nanos(a.load(Relaxed));
    (d(&TERRAIN_NS), d(&DECORATE_NS), d(&WAIT_NS))
}

fn add(a: &AtomicU64, since: Instant) {
    a.fetch_add(since.elapsed().as_nanos() as u64, Relaxed);
}

/// The chunk generation scheduler. `Sync`: share it (`Arc`) between generation threads, each
/// with its own [`GenScratch`].
pub struct Pipeline {
    world: Arc<Worldgen>,
    state: Mutex<State>,
    /// Structure starts by chunk (STRUCTURE_STARTS), shared by all threads.
    starts: StartCache,
    wake: Condvar,
}

impl Pipeline {
    pub fn new(world: Arc<Worldgen>) -> Self {
        Self { world, state: Mutex::new(State::default()), starts: StartCache::default(), wake: Condvar::new() }
    }

    pub fn world(&self) -> &Arc<Worldgen> {
        &self.world
    }

    pub fn stats(&self) -> PipelineStats {
        let s = self.state.lock().unwrap();
        PipelineStats { held: s.chunks.len(), decorated: s.decorated.len(), finished: s.finished.len() }
    }

    /// Generates the chunk through FULL: its 3×3 neighbourhood decorated, so nothing will
    /// write to it any more. Blocks until done.
    ///
    /// The calling thread works through the decorations the chunk needs (its 3×3 and their
    /// predecessors, [`order::decoration_order`]), each time taking whichever is ready (its
    /// predecessors decorated, its window's terrain there and nobody else's decoration using it),
    /// or else a missing terrain of one of their windows; it waits only when there is none.
    /// Threads asking for nearby chunks share these decorations and each takes a different
    /// ready one (instead of descending the same predecessor chain and waiting for each other).
    pub fn full(&self, gs: &mut GenScratch, x: i32, z: i32) -> ProtoChunk {
        {
            let mut s = self.state.lock().unwrap();
            if s.finished.contains(&(x, z)) {
                drop(s);
                return Pipeline::new(self.world.clone()).full(gs, x, z);
            }
            for dz in -PREFETCH..=PREFETCH {
                for dx in -PREFETCH..=PREFETCH {
                    let p = (x + dx, z + dz);
                    if !s.chunks.contains_key(&p) && !s.finished.contains(&p) {
                        s.wanted.push_back(p);
                    }
                }
            }
        }
        let mut todo = order::decoration_order(&[(x, z)]);
        let mut s = self.state.lock().unwrap();
        loop {
            todo.retain(|p| !s.decorated.contains(p));
            if todo.is_empty() {
                break;
            }
            s = match Self::pick(&s, &todo) {
                Some(Work::Decorate(d)) => self.decorate(s, gs, d),
                Some(Work::Terrain(p)) => {
                    s.chunks.insert(p, Slot::Generating);
                    drop(s);
                    let chunk = self.generate_terrain(gs, p.0, p.1);
                    let mut s = self.state.lock().unwrap();
                    Self::put_back(&mut s, p, chunk);
                    self.wake.notify_all();
                    s
                }
                None => self.wait_or_help(s, gs),
            };
        }
        if s.finished.contains(&(x, z)) {
            drop(s);
            return Pipeline::new(self.world.clone()).full(gs, x, z);
        }
        // Every decoration that could borrow it is done: it is back and nothing takes it again.
        match s.chunks.remove(&(x, z)) {
            Some(Slot::Ready(c)) => {
                s.finished.insert((x, z));
                s.stamps.remove(&(x, z));
                Self::park_idle(&mut s);
                drop(s);
                let mut c = *c;
                c.unpark();
                c
            }
            _ => unreachable!("decorated neighbourhood without the chunk {x},{z}"),
        }
    }

    /// The next step for `todo` (decorations not done, in canonical order): the first one that can
    /// run now, else the first missing terrain of their windows.
    fn pick(s: &State, todo: &[(i32, i32)]) -> Option<Work> {
        let mut terrain = None;
        for &(x, z) in todo {
            let mut free = true;
            for i in 0..9 {
                let w = (x + i % 3 - 1, z + i / 3 - 1);
                match s.chunks.get(&w) {
                    Some(Slot::Ready(_)) => {}
                    Some(_) => free = false,
                    None => {
                        free = false;
                        terrain = terrain.or(Some(w));
                    }
                }
            }
            if free && order::predecessors(x, z).all(|p| s.decorated.contains(&p)) {
                return Some(Work::Decorate((x, z)));
            }
        }
        terrain.map(Work::Terrain)
    }

    /// The chunk's `structures` NBT (`SerializableChunkData.packStructureData`): the starts
    /// placed at this chunk by their `StructureStart.createTag`, and per structure the start
    /// chunks whose pieces reach it (`References`).
    pub fn structure_data(&self, gs: &mut GenScratch, x: i32, z: i32) -> Tag {
        let (mut starts, mut references) = (Vec::new(), Vec::new());
        if self.world.generate_structures {
            let (w, s) = (&self.world, &mut gs.structures);
            for start in self.starts.get(&w.structures, &w.generator, s, x, z).iter() {
                starts.push((w.structures.structures[start.structure].name.clone(), start.save(&w.structures)));
            }
            for (id, list) in ChunkStarts::new(&w.structures, &w.generator, &self.starts, s, x, z).references(&w.structures) {
                references.push((id, Tag::LongArray(list)));
            }
        }
        Tag::Compound(vec![("References".into(), Tag::Compound(references)), ("starts".into(), Tag::Compound(starts))])
    }

    /// `ChunkGenerator.findNearestMapStructure(level, structures, origin, radius, false)`:
    /// the nearest start of any of the structures (by name) around `origin`, with the
    /// structure's name. `None` when structures are off, none of the names is a structure
    /// that can generate here, or nothing is within `radius`.
    pub fn find_nearest_structure(&self, gs: &mut GenScratch, names: &[String], origin: [i32; 3], radius: i32) -> Option<([i32; 3], String)> {
        self.find_nearest_structure_claiming(gs, names, origin, radius, &mut |_, _| true)
    }

    /// [`Self::find_nearest_structure`] for a map that skips known structures: `claim(chunk, structure name)` is `StructureManager.tryAddReference`.
    pub fn find_nearest_structure_claiming(
        &self,
        gs: &mut GenScratch,
        names: &[String],
        origin: [i32; 3],
        radius: i32,
        claim: &mut dyn FnMut((i32, i32), &str) -> bool,
    ) -> Option<([i32; 3], String)> {
        let w = &self.world;
        if !w.generate_structures {
            return None;
        }
        let wanted: Vec<usize> = names.iter().filter_map(|n| w.structures.id(n)).collect();
        let found = w.structures.find_nearest_claiming(
            &w.generator,
            &mut |x, z| self.starts.get(&w.structures, &w.generator, &mut gs.structures, x, z),
            &wanted,
            origin,
            radius,
            &mut |chunk, st| claim(chunk, &w.structures.structures[st].name),
        )?;
        Some((found.pos, w.structures.structures[found.structure].name.clone()))
    }

    /// A chunk back in the pipeline, waiting; stamped for parking.
    fn put_back(s: &mut State, key: (i32, i32), chunk: Box<ProtoChunk>) {
        s.ops += 1;
        let ops = s.ops;
        s.stamps.insert(key, ops);
        s.chunks.insert(key, Slot::Ready(chunk));
    }

    /// Every so often, with many chunks waiting, compresses those idle for a while (their
    /// neighbourhood is done or far from what is being asked for).
    fn park_idle(s: &mut State) {
        const EVERY: u64 = 64;
        const MIN_HELD: usize = 1024;
        const IDLE_OPS: u64 = 2048;
        s.ops += 1;
        if s.ops % EVERY != 0 || s.chunks.len() < MIN_HELD {
            return;
        }
        let horizon = s.ops.saturating_sub(IDLE_OPS);
        let idle: Vec<(i32, i32)> = s.stamps.iter().filter(|&(_, &t)| t < horizon).map(|(&k, _)| k).collect();
        for k in idle {
            s.stamps.remove(&k);
            if let Some(Slot::Ready(c)) = s.chunks.get_mut(&k) {
                c.park();
            }
        }
    }

    /// BIOMES and TERRAIN (with the chunk's beardifier when structures generate).
    fn generate_terrain(&self, gs: &mut GenScratch, x: i32, z: i32) -> Box<ProtoChunk> {
        let started = Instant::now();
        let chunk = self.generate_terrain_untimed(gs, x, z);
        add(&TERRAIN_NS, started);
        chunk
    }

    fn generate_terrain_untimed(&self, gs: &mut GenScratch, x: i32, z: i32) -> Box<ProtoChunk> {
        let beard = self.world.generate_structures.then(|| {
            let starts = ChunkStarts::new(&self.world.structures, &self.world.generator, &self.starts, &mut gs.structures, x, z);
            Beardifier::for_chunk(&self.world.structures, &starts, x, z)
        });
        Box::new(self.world.generator.generate_with(gs, x, z, beard.flatten()))
    }

    /// Instead of idling until another thread makes progress, generates the terrain of a
    /// wanted chunk nobody started; waits only when there is none.
    fn wait_or_help<'a>(&'a self, mut s: MutexGuard<'a, State>, gs: &mut GenScratch) -> MutexGuard<'a, State> {
        while let Some(p) = s.wanted.pop_front() {
            if s.chunks.contains_key(&p) || s.finished.contains(&p) {
                continue;
            }
            s.chunks.insert(p, Slot::Generating);
            drop(s);
            let chunk = self.generate_terrain(gs, p.0, p.1);
            let mut s = self.state.lock().unwrap();
            Self::put_back(&mut s, p, chunk);
            self.wake.notify_all();
            return s;
        }
        let started = Instant::now();
        let s = self.wake.wait(s).unwrap();
        add(&WAIT_NS, started);
        s
    }

    /// FEATURES for one chunk whose predecessors are decorated and whose window is free
    /// ([`Pipeline::pick`]): borrows the window, decorates it unlocked, and puts it back.
    fn decorate<'a>(&'a self, mut s: MutexGuard<'a, State>, gs: &mut GenScratch, (x, z): (i32, i32)) -> MutexGuard<'a, State> {
        debug_assert!(order::predecessors(x, z).all(|p| s.decorated.contains(&p)), "decorating {x},{z} out of order");
        let window: Vec<Box<ProtoChunk>> = (0..9)
            .map(|i| match s.chunks.insert((x + i % 3 - 1, z + i / 3 - 1), Slot::Borrowed) {
                Some(Slot::Ready(c)) => c,
                _ => unreachable!(),
            })
            .collect();
        drop(s);
        let mut window = window;
        for c in &mut window {
            c.unpark();
        }
        let starts = self.world.generate_structures.then(|| {
            ChunkStarts::new(&self.world.structures, &self.world.generator, &self.starts, &mut gs.structures, x, z)
        });
        let started = Instant::now();
        let mut region = Region::new(window, x, z, &self.world.generator, gs);
        let structures = starts.as_ref().map(|s| (&self.world.structures, s));
        self.world.decorator.decorate(&mut region, structures, &mut ());
        let chunks = region.into_chunks();
        add(&DECORATE_NS, started);
        let mut s = self.state.lock().unwrap();
        for (i, mut c) in chunks.into_iter().enumerate() {
            if i == 4 {
                c.status = Status::Features;
            }
            Self::put_back(&mut s, (c.x, c.z), c);
        }
        s.decorated.insert((x, z));
        self.wake.notify_all();
        s
    }
}
