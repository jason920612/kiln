//! Chunk generation off the tick thread (design §4.2 gen pool, §5.4–5.5): worker threads with
//! their own generator instances; finished chunks wait until B0 installs them.

use crossbeam_channel::{Receiver, Sender};
use kiln_world::chunk::Chunk;
use kiln_world::{ChunkGenerator, ChunkPos, Dimension};
use crate::chunkstats;
use std::collections::HashMap;
use std::time::Instant;

/// Chunks queued or being generated at most; further requests wait for the next tick.
const MAX_IN_FLIGHT: usize = 256;

/// Chunks the generation threads finished and the time they took (all levels, since start).
static GENERATED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GENERATING_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many chunks the generation threads have made since start, and the time they spent on
/// them (summed over the threads), for load tools.
pub fn generation_totals() -> (u64, std::time::Duration) {
    use std::sync::atomic::Ordering::Relaxed;
    (GENERATED.load(Relaxed), std::time::Duration::from_nanos(GENERATING_NS.load(Relaxed)))
}

pub(crate) struct GenPool {
    requests: Sender<ChunkPos>,
    results: Receiver<(ChunkPos, Chunk)>,
    /// Queued or running chunks and when each was requested.
    in_flight: HashMap<ChunkPos, Instant>,
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
                        let started = Instant::now();
                        let mut chunk = generator.generate(pos, dimension);
                        chunkstats::count(&chunkstats::GEN_DONE);
                        chunkstats::add_ns(&chunkstats::GEN_BUSY_NS, started.elapsed());
                        chunk.mark_new();
                        GENERATING_NS.fetch_add(started.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
                        GENERATED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if done.send((pos, chunk)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawning a generation thread");
        }
        Self { requests, results, in_flight: HashMap::new() }
    }

    /// Queues `pos` unless it is already queued; `false` when the queue is full.
    pub fn request(&mut self, pos: ChunkPos) -> bool {
        if self.in_flight.contains_key(&pos) {
            return true;
        }
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return false;
        }
        self.in_flight.insert(pos, Instant::now());
        let _ = self.requests.send(pos);
        true
    }

    /// Chunks queued or being generated.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub fn is_queued(&self, pos: ChunkPos) -> bool {
        self.in_flight.contains_key(&pos)
    }

    /// Chunks finished since the last call, in position order.
    pub fn finished(&mut self) -> Vec<(ChunkPos, Chunk)> {
        let mut out: Vec<_> = self.results.try_iter().collect();
        for (pos, _) in &out {
            if let Some(at) = self.in_flight.remove(pos) {
                chunkstats::GEN_LATENCY.add(at.elapsed());
            }
        }
        out.sort_unstable_by_key(|(pos, _)| *pos);
        out
    }
}
