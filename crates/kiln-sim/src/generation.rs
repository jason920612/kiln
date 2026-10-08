//! Chunk generation off the tick thread (design §4.2 gen pool, §5.4–5.5): worker threads with
//! their own generator instances; finished chunks wait until B0 installs them.
//!
//! Two queues: the chunks the players' views ask for, and urgent ones (the chunk a player
//! joins or is teleported into, which waits for it in a loading state). The threads take
//! urgent chunks first.

use crate::chunkstats;
use crossbeam_channel::{Receiver, Sender};
use kiln_world::chunk::Chunk;
use kiln_world::{ChunkGenerator, ChunkPos, Dimension};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Chunks queued or being generated at most; further requests wait for the next tick
/// (urgent requests do not count against it).
const MAX_IN_FLIGHT: usize = 256;

/// How many chunks the generation threads have made since start, and the time they spent on
/// them (summed over the threads), for load tools.
pub fn generation_totals() -> (u64, std::time::Duration) {
    use std::sync::atomic::Ordering::Relaxed;
    (chunkstats::GEN_DONE.load(Relaxed), std::time::Duration::from_nanos(chunkstats::GEN_BUSY_NS.load(Relaxed)))
}

/// Generation threads run only on CPU time nothing else wants (`SCHED_IDLE` on Linux, below
/// normal priority on Windows): ticks and other programs go first, and idle cores generate.
fn background_priority() {
    #[cfg(target_os = "linux")]
    {
        #[repr(C)]
        struct SchedParam {
            priority: i32,
        }
        unsafe extern "C" {
            fn sched_setscheduler(pid: i32, policy: i32, param: *const SchedParam) -> i32;
        }
        const SCHED_IDLE: i32 = 5;
        // SAFETY: pid 0 is the calling thread; the parameter outlives the call.
        unsafe {
            sched_setscheduler(0, SCHED_IDLE, &SchedParam { priority: 0 });
        }
    }
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        // SAFETY: the current-thread pseudo-handle and a documented priority value.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}

/// Whether a requested chunk is still waiting in a queue or a thread took it. A chunk can sit
/// in both queues (made urgent after it was queued): the first thread to take it generates it,
/// the other copy is skipped, as is a copy whose result the tick thread already collected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Claim {
    Queued,
    Taken,
}

pub(crate) struct GenPool {
    requests: Sender<ChunkPos>,
    urgent: Sender<ChunkPos>,
    results: Receiver<(ChunkPos, Chunk)>,
    /// Queued or running chunks and when each was requested.
    in_flight: HashMap<ChunkPos, Instant>,
    /// Chunks sent to the urgent queue (a subset of `in_flight`).
    urgent_sent: std::collections::HashSet<ChunkPos>,
    /// Finished chunks not installed yet (still in `in_flight`, so not asked for again).
    ready: std::collections::BTreeMap<ChunkPos, Chunk>,
    claims: Arc<Mutex<HashMap<ChunkPos, Claim>>>,
    /// A fork of the generator kept for its statistics.
    probe: Box<dyn ChunkGenerator>,
}

impl GenPool {
    pub fn new(generator: &dyn ChunkGenerator, dimension: Dimension, threads: usize) -> Self {
        let (requests, jobs) = crossbeam_channel::unbounded::<ChunkPos>();
        let (urgent, urgent_jobs) = crossbeam_channel::unbounded::<ChunkPos>();
        let (done, results) = crossbeam_channel::unbounded();
        let claims: Arc<Mutex<HashMap<ChunkPos, Claim>>> = Arc::default();
        for i in 0..threads.max(1) {
            let (jobs, urgent_jobs, done, claims) = (jobs.clone(), urgent_jobs.clone(), done.clone(), claims.clone());
            let mut generator = generator.fork();
            std::thread::Builder::new()
                .name(format!("kiln-gen-{i}"))
                .spawn(move || {
                    background_priority();
                    loop {
                        // Urgent chunks first; otherwise whichever queue has work.
                        let pos = match urgent_jobs.try_recv() {
                            Ok(pos) => pos,
                            Err(_) => crossbeam_channel::select! {
                                recv(urgent_jobs) -> p => match p { Ok(p) => p, Err(_) => break },
                                recv(jobs) -> p => match p { Ok(p) => p, Err(_) => break },
                            },
                        };
                        {
                            let mut claims = claims.lock().unwrap();
                            match claims.get_mut(&pos) {
                                Some(c @ Claim::Queued) => *c = Claim::Taken,
                                _ => continue,
                            }
                        }
                        let started = Instant::now();
                        let mut chunk = generator.generate(pos, dimension);
                        chunkstats::count(&chunkstats::GEN_DONE);
                        chunkstats::add_ns(&chunkstats::GEN_BUSY_NS, started.elapsed());
                        chunk.mark_new();
                        if done.send((pos, chunk)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawning a generation thread");
        }
        Self { requests, urgent, results, in_flight: HashMap::new(), urgent_sent: Default::default(), ready: Default::default(), claims, probe: generator.fork() }
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
        self.claims.lock().unwrap().insert(pos, Claim::Queued);
        let _ = self.requests.send(pos);
        true
    }

    /// Queues `pos` ahead of everything else (whatever the queue holds), or moves it ahead if it
    /// is queued already.
    pub fn request_urgent(&mut self, pos: ChunkPos) {
        if !self.urgent_sent.insert(pos) {
            return;
        }
        if !self.in_flight.contains_key(&pos) {
            self.in_flight.insert(pos, Instant::now());
            self.claims.lock().unwrap().insert(pos, Claim::Queued);
        }
        let _ = self.urgent.send(pos);
    }

    /// Chunks waiting in a queue (not taken by a thread, not finished).
    pub fn queued(&self) -> Vec<ChunkPos> {
        let claims = self.claims.lock().unwrap();
        self.in_flight.keys().filter(|p| claims.get(p) == Some(&Claim::Queued)).copied().collect()
    }

    /// Drops a chunk still waiting in a queue (a thread that took it already finishes it);
    /// whether it was dropped.
    pub fn cancel(&mut self, pos: ChunkPos) -> bool {
        let mut claims = self.claims.lock().unwrap();
        if claims.get(&pos) != Some(&Claim::Queued) {
            return false;
        }
        claims.remove(&pos);
        drop(claims);
        self.in_flight.remove(&pos);
        self.urgent_sent.remove(&pos);
        true
    }

    /// Unfinished chunks the generator holds (see [`ChunkGenerator::held`]).
    pub fn held(&self) -> usize {
        self.probe.held()
    }

    /// Chunks queued or being generated.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub fn is_queued(&self, pos: ChunkPos) -> bool {
        self.in_flight.contains_key(&pos)
    }

    /// Collects what the threads finished since the last call; a result for a position no
    /// longer in flight is a copy and goes.
    pub fn collect(&mut self) {
        for (pos, chunk) in self.results.try_iter() {
            if self.in_flight.contains_key(&pos) && !self.ready.contains_key(&pos) {
                self.ready.insert(pos, chunk);
            }
        }
    }

    /// Finished chunks waiting to be taken, in position order.
    pub fn ready(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.ready.keys().copied()
    }

    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Hands out a finished chunk; it is no longer in flight.
    pub fn take(&mut self, pos: ChunkPos) -> Option<Chunk> {
        let chunk = self.ready.remove(&pos)?;
        if let Some(at) = self.in_flight.remove(&pos) {
            chunkstats::GEN_LATENCY.add(at.elapsed());
        }
        self.claims.lock().unwrap().remove(&pos);
        self.urgent_sent.remove(&pos);
        Some(chunk)
    }
}
