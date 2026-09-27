//! Chunk generation off the tick thread (design §4.2 gen pool, §5.4–5.5): worker threads with
//! their own generator instances; finished chunks wait until B0 installs them.

use crossbeam_channel::{Receiver, Sender};
use kiln_world::chunk::Chunk;
use kiln_world::{ChunkGenerator, ChunkPos, Dimension};
use std::collections::HashSet;

/// Chunks queued or being generated at most; further requests wait for the next tick.
const MAX_IN_FLIGHT: usize = 256;

pub(crate) struct GenPool {
    requests: Sender<ChunkPos>,
    results: Receiver<(ChunkPos, Chunk)>,
    in_flight: HashSet<ChunkPos>,
}

impl GenPool {
    pub fn new(generator: &dyn ChunkGenerator, dimension: Dimension, threads: usize) -> Self {
        let (requests, jobs) = crossbeam_channel::unbounded::<ChunkPos>();
        let (done, results) = crossbeam_channel::unbounded();
        for i in 0..threads.max(1) {
            let (jobs, done) = (jobs.clone(), done.clone());
            let mut generator = generator.fork();
            std::thread::Builder::new()
                .name(format!("kiln-gen-{i}"))
                .spawn(move || {
                    for pos in jobs {
                        let mut chunk = generator.generate(pos, dimension);
                        chunk.mark_new();
                        if done.send((pos, chunk)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawning a generation thread");
        }
        Self { requests, results, in_flight: HashSet::new() }
    }

    /// Queues `pos` unless it is already queued; `false` when the queue is full.
    pub fn request(&mut self, pos: ChunkPos) -> bool {
        if self.in_flight.contains(&pos) {
            return true;
        }
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return false;
        }
        self.in_flight.insert(pos);
        let _ = self.requests.send(pos);
        true
    }

    pub fn is_queued(&self, pos: ChunkPos) -> bool {
        self.in_flight.contains(&pos)
    }

    /// Chunks finished since the last call, in position order.
    pub fn finished(&mut self) -> Vec<(ChunkPos, Chunk)> {
        let mut out: Vec<_> = self.results.try_iter().collect();
        for (pos, _) in &out {
            self.in_flight.remove(pos);
        }
        out.sort_unstable_by_key(|(pos, _)| *pos);
        out
    }
}
