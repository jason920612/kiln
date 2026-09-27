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
use crate::generator::{GenScratch, Generator};
use crate::order;
use crate::proto::{ProtoChunk, Status};
use crate::region::Region;
use crate::sets::Loader;
use crate::structure::{ChunkStarts, StartCache, Structures};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};

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
    decorated: HashSet<(i32, i32)>,
    /// Handed out by [`Pipeline::full`].
    finished: HashSet<(i32, i32)>,
}

/// Counts for monitoring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineStats {
    /// Proto-chunks held (terrain or decorated, not yet handed out).
    pub held: usize,
    pub decorated: usize,
    pub finished: usize,
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
    pub fn full(&self, gs: &mut GenScratch, x: i32, z: i32) -> ProtoChunk {
        if self.state.lock().unwrap().finished.contains(&(x, z)) {
            return Pipeline::new(self.world.clone()).full(gs, x, z);
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.decorate(gs, x + dx, z + dz);
            }
        }
        let mut s = self.state.lock().unwrap();
        loop {
            if s.finished.contains(&(x, z)) {
                drop(s);
                return Pipeline::new(self.world.clone()).full(gs, x, z);
            }
            match s.chunks.remove(&(x, z)) {
                Some(Slot::Ready(c)) => {
                    s.finished.insert((x, z));
                    return *c;
                }
                Some(other) => {
                    s.chunks.insert((x, z), other);
                    s = self.wake.wait(s).unwrap();
                }
                None => unreachable!("decorated neighbourhood without the chunk"),
            }
        }
    }

    /// Makes sure the chunk is at TERRAIN or later.
    fn terrain(&self, gs: &mut GenScratch, x: i32, z: i32) {
        let mut s = self.state.lock().unwrap();
        loop {
            debug_assert!(!s.finished.contains(&(x, z)), "terrain requested for a finished chunk {x},{z}");
            match s.chunks.get(&(x, z)) {
                Some(Slot::Ready(_) | Slot::Borrowed) => return,
                Some(Slot::Generating) => s = self.wake.wait(s).unwrap(),
                None => break,
            }
        }
        s.chunks.insert((x, z), Slot::Generating);
        drop(s);
        let chunk = Box::new(self.world.generator.generate(gs, x, z));
        let mut s = self.state.lock().unwrap();
        s.chunks.insert((x, z), Slot::Ready(chunk));
        self.wake.notify_all();
    }

    /// FEATURES for one chunk, after the chunks that must precede it in canonical order.
    fn decorate(&self, gs: &mut GenScratch, x: i32, z: i32) {
        if self.state.lock().unwrap().decorated.contains(&(x, z)) {
            return;
        }
        for (px, pz) in order::predecessors(x, z) {
            self.decorate(gs, px, pz);
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.terrain(gs, x + dx, z + dz);
            }
        }
        let window = {
            let mut s = self.state.lock().unwrap();
            loop {
                if s.decorated.contains(&(x, z)) {
                    return;
                }
                let free = (0..9).all(|i| matches!(s.chunks.get(&(x + i % 3 - 1, z + i / 3 - 1)), Some(Slot::Ready(_))));
                if free {
                    break;
                }
                s = self.wake.wait(s).unwrap();
            }
            debug_assert!(order::predecessors(x, z).all(|p| s.decorated.contains(&p)), "decorating {x},{z} out of order");
            (0..9)
                .map(|i| match s.chunks.insert((x + i % 3 - 1, z + i / 3 - 1), Slot::Borrowed) {
                    Some(Slot::Ready(c)) => c,
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        };
        let starts = self.world.generate_structures.then(|| {
            ChunkStarts::new(&self.world.structures, &self.world.generator, &self.starts, &mut gs.structures, x, z)
        });
        let mut region = Region::new(window, x, z, &self.world.generator, gs);
        let structures = starts.as_ref().map(|s| (&self.world.structures, s));
        self.world.decorator.decorate(&mut region, structures, &mut ());
        let chunks = region.into_chunks();
        let mut s = self.state.lock().unwrap();
        for (i, mut c) in chunks.into_iter().enumerate() {
            if i == 4 {
                c.status = Status::Features;
            }
            s.chunks.insert((c.x, c.z), Slot::Ready(c));
        }
        s.decorated.insert((x, z));
        self.wake.notify_all();
    }
}
