//! NBT encoding (and for region files compression) on a thread of its own, for the chunk data
//! the tick hands over when chunks unload: entities and points of interest (encoding them cost
//! the tick several milliseconds per unload batch, 73 ms when many players left at once).
//!
//! The tick hands a chunk's compound over and goes on. Until its encoding is back, the newest
//! data of that chunk is the handed-over compound, and the store reads it from there: nothing
//! ever waits for the thread, which may get no core for a while (a client rendering in software
//! on the same machine). A flush encodes whatever is still out on the calling thread.

use kiln_proto::nbt::Tag;
use kiln_world::ChunkPos;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

/// A chunk's compound handed to the thread.
struct Job {
    pos: ChunkPos,
    seq: u64,
    root: Arc<Tag>,
}

/// The jobs the thread has not taken yet, and whether the store is gone.
type Queue = (Mutex<(VecDeque<Job>, bool)>, Condvar);

pub(crate) struct Encoder {
    queue: Arc<Queue>,
    done: std::sync::mpsc::Receiver<(ChunkPos, u64, Vec<u8>)>,
    /// Chunks handed over and not back: the newest job's number and compound (older jobs of the
    /// chunk, still out, come back to nothing).
    out: HashMap<ChunkPos, (u64, Arc<Tag>)>,
    next: u64,
    compress: bool,
}

/// A chunk's compound as stored: plain NBT for a native store, compressed for region files.
pub(crate) fn encode(root: &Tag, compress: bool) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    root.write_named("", &mut buf);
    if compress { crate::region::compress_chunk(&buf) } else { buf.to_vec() }
}

impl Encoder {
    pub(crate) fn start(name: &str, compress: bool) -> Encoder {
        let queue: Arc<Queue> = Arc::default();
        let (tx, done) = std::sync::mpsc::channel();
        let q = queue.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                loop {
                    let job = {
                        let mut jobs = q.0.lock().unwrap();
                        loop {
                            if let Some(job) = jobs.0.pop_front() {
                                break job;
                            }
                            if jobs.1 {
                                return;
                            }
                            jobs = q.1.wait(jobs).unwrap();
                        }
                    };
                    if tx.send((job.pos, job.seq, encode(&job.root, compress))).is_err() {
                        return;
                    }
                }
            })
            .expect("spawning an encoding thread");
        Encoder { queue, done, out: HashMap::new(), next: 0, compress }
    }

    /// Hands a chunk's compound over (newer than anything the store has of it).
    pub(crate) fn submit(&mut self, pos: ChunkPos, root: Tag) {
        self.next += 1;
        let root = Arc::new(root);
        self.out.insert(pos, (self.next, root.clone()));
        self.queue.0.lock().unwrap().0.push_back(Job { pos, seq: self.next, root });
        self.queue.1.notify_one();
    }

    /// The chunk's compound if it is out (the newest data of the chunk then).
    pub(crate) fn newest(&self, pos: ChunkPos) -> Option<&Arc<Tag>> {
        self.out.get(&pos).map(|(_, root)| root)
    }

    /// The store has something newer for the chunk: what is out comes back to nothing.
    pub(crate) fn forget(&mut self, pos: ChunkPos) {
        self.out.remove(&pos);
    }

    /// The finished encodings that are still the newest of their chunk.
    pub(crate) fn finished(&mut self) -> Vec<(ChunkPos, Vec<u8>)> {
        let mut got = Vec::new();
        for (pos, seq, payload) in self.done.try_iter() {
            if self.out.get(&pos).is_some_and(|(s, _)| *s == seq) {
                self.out.remove(&pos);
                got.push((pos, payload));
            }
        }
        got
    }

    /// Everything still out, encoded here: jobs the thread has not taken leave its queue, and one
    /// it is in the middle of is encoded again (its result comes back to nothing).
    pub(crate) fn drain(&mut self) -> Vec<(ChunkPos, Vec<u8>)> {
        let mut got = self.finished();
        self.queue.0.lock().unwrap().0.clear();
        let mut rest: Vec<(ChunkPos, Arc<Tag>)> = self.out.drain().map(|(pos, (_, root))| (pos, root)).collect();
        rest.sort_unstable_by_key(|(p, _)| (p.x, p.z));
        let compress = self.compress;
        got.extend(rest.into_iter().map(|(pos, root)| (pos, encode(&root, compress))));
        got
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.queue.0.lock().unwrap().1 = true;
        self.queue.1.notify_all();
    }
}
