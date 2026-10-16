//! Entity chunks: a dimension's `entities/r.<x>.<z>.mca`, as vanilla `EntityStorage` keeps
//! them (`DataVersion`, `Position` as `[x, z]`, `Entities` with each entity's saved compound).

use crate::anvil::DATA_VERSION;
use crate::region::RegionFile;
use kiln_proto::nbt::{self, Tag};
use kiln_world::ChunkPos;
use std::collections::{HashMap, HashSet, VecDeque};
use crate::native::{ENTITIES, FORM_NBT, NativeStore};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use tracing::warn;

/// A pending update: the compressed chunk, or `None` to delete it (vanilla stores `null` for
/// a chunk whose entities are all gone).
type Update = (usize, usize, Option<Vec<u8>>);

pub struct EntityStore {
    /// Entity records of a native world's cell files instead of region files.
    native: Option<Arc<Mutex<NativeStore>>>,
    dir: PathBuf,
    regions: HashMap<(i32, i32), Option<RegionFile>>,
    /// Writes waiting for `flush`, per region; loads read them first.
    pending: HashMap<(i32, i32), Vec<Update>>,
    /// Region writes on a thread of its own, once [`Self::set_background`] asked.
    writer: Option<crate::region::BackgroundWriter>,
    /// Chunks known to have no entities stored (`EntityStorage.emptyChunks`), so saving
    /// them empty again writes nothing.
    empty: HashSet<ChunkPos>,
    warned_version: bool,
    /// Encoding on a thread of its own, once [`Self::set_background`] asked.
    encoder: Option<Encoder>,
}

/// A chunk's entity data (`DataVersion`, `Entities`, `Position`) handed to the encoder.
struct Job {
    pos: ChunkPos,
    seq: u64,
    root: Arc<Tag>,
}

/// The jobs the encoder thread has not taken yet, and whether the store is gone.
type Queue = (Mutex<(VecDeque<Job>, bool)>, Condvar);

/// Entity chunks encoded (NBT, compressed for region files) on a thread of its own: unloading
/// chunks hands their entities over and goes on (encoding and compressing them cost the tick
/// 14–18 ms on a flight, 73 ms when many players left at once). Until a chunk's encoding is
/// back, its newest entities are the handed-over ones, and loads read them from there: nothing
/// ever waits for the thread, which may get no core for a while. A flush encodes what is still
/// out itself.
struct Encoder {
    queue: Arc<Queue>,
    done: std::sync::mpsc::Receiver<(ChunkPos, u64, Vec<u8>)>,
    /// Chunks handed over and not back: the newest job's number and data (older jobs of the
    /// chunk, still out, come back to nothing).
    out: HashMap<ChunkPos, (u64, Arc<Tag>)>,
    next: u64,
}

/// A chunk's entities as stored: plain NBT for a native store, compressed for region files.
fn encode(root: &Tag, compress: bool) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    root.write_named("", &mut buf);
    if compress { crate::region::compress_chunk(&buf) } else { buf.to_vec() }
}

impl Encoder {
    fn start(compress: bool) -> Encoder {
        let queue: Arc<Queue> = Arc::default();
        let (tx, done) = std::sync::mpsc::channel();
        let q = queue.clone();
        std::thread::Builder::new()
            .name("kiln-encode-entities".into())
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
            .expect("spawning the entity encoding thread");
        Encoder { queue, done, out: HashMap::new(), next: 0 }
    }

    fn submit(&mut self, pos: ChunkPos, root: Tag) {
        self.next += 1;
        let root = Arc::new(root);
        self.out.insert(pos, (self.next, root.clone()));
        self.queue.0.lock().unwrap().0.push_back(Job { pos, seq: self.next, root });
        self.queue.1.notify_one();
    }

    /// The finished encodings that are still the newest of their chunk.
    fn finished(&mut self) -> Vec<(ChunkPos, Vec<u8>)> {
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
    fn drain(&mut self, compress: bool) -> Vec<(ChunkPos, Vec<u8>)> {
        let mut got = self.finished();
        self.queue.0.lock().unwrap().0.clear();
        let mut rest: Vec<(ChunkPos, Arc<Tag>)> = self.out.drain().map(|(pos, (_, root))| (pos, root)).collect();
        rest.sort_unstable_by_key(|(p, _)| (p.x, p.z));
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

impl EntityStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { native: None, dir: dir.into(), regions: HashMap::new(), pending: HashMap::new(), writer: None, empty: HashSet::new(), warned_version: false, encoder: None }
    }

    /// Entity chunks kept in a native store.
    pub fn native(store: Arc<Mutex<NativeStore>>) -> Self {
        let mut s = Self::new(PathBuf::new());
        s.native = Some(store);
        s
    }

    fn local(pos: ChunkPos) -> ((i32, i32), (usize, usize)) {
        ((pos.x >> 5, pos.z >> 5), ((pos.x & 31) as usize, (pos.z & 31) as usize))
    }

    fn read_nbt(&mut self, pos: ChunkPos) -> Option<Vec<u8>> {
        if let Some(store) = &self.native {
            return store.lock().unwrap().read(ENTITIES, pos).map(|(_, raw, _)| raw);
        }
        let (key, local) = Self::local(pos);
        // A write not flushed yet is newer than the region file.
        if let Some((_, _, payload)) = self.pending.get(&key).and_then(|p| p.iter().find(|(x, z, _)| (*x, *z) == local)) {
            return payload.as_ref().and_then(|p| crate::region::decompress_chunk(p).map_err(|e| warn!("entity chunk {pos:?}: {e}")).ok());
        }
        if let Some(w) = self.writer.as_mut() {
            for k in w.written() {
                self.regions.remove(&k);
            }
            if let Some(p) = w.find(key, local) {
                return (!p.is_empty()).then(|| crate::region::decompress_chunk(p).map_err(|e| warn!("entity chunk {pos:?}: {e}")).ok()).flatten();
            }
        }
        let dir = &self.dir;
        let region = self.regions.entry(key).or_insert_with(|| {
            let path = dir.join(format!("r.{}.{}.mca", key.0, key.1));
            path.exists().then(|| RegionFile::open(&path)).and_then(|r| r.map_err(|e| warn!("cannot open {}: {e}", path.display())).ok())
        });
        match region.as_mut()?.read(local.0, local.1) {
            Ok(data) => data,
            Err(e) => {
                warn!("entity chunk {pos:?}: {e}");
                None
            }
        }
    }

    /// `EntityStorage.loadEntities`: the saved entity compounds of a chunk (empty if it has
    /// none or its data is unreadable).
    pub fn load(&mut self, pos: ChunkPos) -> Vec<Tag> {
        if self.empty.contains(&pos) {
            return Vec::new();
        }
        // Handed to the encoder and not back: the newest there is.
        if let Some((_, root)) = self.encoder.as_ref().and_then(|e| e.out.get(&pos)) {
            return root.get("Entities").and_then(Tag::as_list).unwrap_or(&[]).iter().map(|t| t.unwrap_list_element().clone()).collect();
        }
        self.collect();
        let Some(data) = self.read_nbt(pos) else {
            self.empty.insert(pos);
            return Vec::new();
        };
        let root = match nbt::read_named(&data) {
            Ok((_, root @ Tag::Compound(_))) => root,
            Ok(_) | Err(_) => {
                warn!("entity chunk {pos:?} is not a compound");
                return Vec::new();
            }
        };
        match root.get("Position") {
            Some(Tag::IntArray(p)) if p.len() == 2 && (p[0], p[1]) != (pos.x, pos.z) => {
                warn!("entity chunk file at {pos:?} is in the wrong location (got [{}, {}])", p[0], p[1]);
                return Vec::new();
            }
            Some(Tag::IntArray(p)) if p.len() == 2 => {}
            _ => {
                warn!("entity chunk {pos:?}: no position");
                return Vec::new();
            }
        }
        let version = root.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0);
        if version != DATA_VERSION && !self.warned_version {
            warn!("entity data version {version} differs from {DATA_VERSION}; upgrade old worlds with vanilla --forceUpgrade");
            self.warned_version = true;
        }
        match root.get("Entities").and_then(Tag::as_list) {
            Some(list) => list.iter().map(|t| t.unwrap_list_element().clone()).filter(|t| matches!(t, Tag::Compound(_))).collect(),
            None => Vec::new(),
        }
    }

    /// `EntityStorage.storeEntities`: queues a chunk's entities for the next `flush`. An empty
    /// list deletes the chunk's data unless it is already known to be empty.
    pub fn store(&mut self, pos: ChunkPos, entities: Vec<Tag>) {
        if entities.is_empty() {
            if !self.empty.insert(pos) {
                return;
            }
            // (Newer than whatever is still with the encoder.)
            if let Some(e) = self.encoder.as_mut() {
                e.out.remove(&pos);
            }
            self.put(pos, None);
            return;
        }
        self.empty.remove(&pos);
        let root = Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(DATA_VERSION as i32)),
            ("Entities".into(), Tag::List(entities)),
            ("Position".into(), Tag::IntArray(vec![pos.x, pos.z])),
        ]);
        match self.encoder.as_mut() {
            Some(e) => e.submit(pos, root),
            None => {
                let encoded = encode(&root, self.native.is_none());
                self.put(pos, Some(encoded));
            }
        }
    }

    /// Queues a chunk's encoded entities (`None`: it has none) for the next `flush`.
    fn put(&mut self, pos: ChunkPos, encoded: Option<Vec<u8>>) {
        if let Some(store) = &self.native {
            let mut store = store.lock().unwrap();
            // Deleting what is not stored changes nothing.
            if encoded.is_some() || store.contains(ENTITIES, pos) {
                store.write(ENTITIES, pos, FORM_NBT, encoded.as_deref());
            }
            return;
        }
        let (key, (lx, lz)) = Self::local(pos);
        let entry = self.pending.entry(key).or_default();
        entry.retain(|(x, z, _)| (*x, *z) != (lx, lz));
        entry.push((lx, lz, encoded));
    }

    /// Takes in what the encoder finished.
    fn collect(&mut self) {
        let Some(e) = self.encoder.as_mut() else { return };
        for (pos, encoded) in e.finished() {
            self.put(pos, Some(encoded));
        }
    }

    /// Forgets what is known about a chunk that unloaded (its data stays pending until
    /// `flush`).
    pub fn unloaded(&mut self, pos: ChunkPos) {
        self.empty.remove(&pos);
    }

    /// Writes the queued chunks into their region files.
    /// Region files written on a thread of its own from now on (`flush` returns before they
    /// are written; [`Self::sync`] waits). Native stores keep writing in place.
    pub fn set_background(&mut self, on: bool) {
        if on && self.writer.is_none() && self.native.is_none() {
            self.writer = Some(crate::region::BackgroundWriter::start("kiln-write-entities"));
        }
        if on && self.encoder.is_none() {
            self.encoder = Some(Encoder::start(self.native.is_none()));
        }
    }

    /// Flushes and waits until everything is written.
    pub fn sync(&mut self) -> std::io::Result<usize> {
        if let Some(store) = &self.native {
            return store.lock().unwrap().flush_now();
        }
        let n = self.flush()?;
        if let Some(w) = self.writer.as_mut() {
            for k in w.wait() {
                self.regions.remove(&k);
            }
        }
        Ok(n)
    }

    pub fn flush(&mut self) -> std::io::Result<usize> {
        // Everything still with the encoder, encoded here (no waiting for its thread).
        let compress = self.native.is_none();
        if let Some(e) = self.encoder.as_mut() {
            for (pos, encoded) in e.drain(compress) {
                self.put(pos, Some(encoded));
            }
        }
        if let Some(store) = &self.native {
            return store.lock().unwrap().flush();
        }
        if self.pending.is_empty() {
            return Ok(0);
        }
        std::fs::create_dir_all(&self.dir)?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
        let mut written = 0;
        for ((rx, rz), updates) in std::mem::take(&mut self.pending) {
            let path = self.dir.join(format!("r.{rx}.{rz}.mca"));
            // Deleting from a region that does not exist changes nothing.
            if !path.exists() && updates.iter().all(|(_, _, p)| p.is_none()) && !self.writer.as_ref().is_some_and(|w| w.writing((rx, rz))) {
                continue;
            }
            written += updates.len();
            if let Some(w) = self.writer.as_mut() {
                let updates = updates.into_iter().map(|(x, z, p)| (x, z, crate::region::Payload::from(p.unwrap_or_default()))).collect();
                w.submit((rx, rz), path, updates);
                continue;
            }
            self.regions.remove(&(rx, rz));
            let updates: Vec<(usize, usize, Vec<u8>)> = updates.into_iter().map(|(x, z, p)| (x, z, p.unwrap_or_default())).collect();
            crate::region::write_region(&path, &updates, now).map_err(std::io::Error::other)?;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(n: i32) -> Tag {
        Tag::Compound(vec![("id".into(), Tag::String("minecraft:pig".into())), ("n".into(), Tag::Int(n))])
    }

    #[test]
    fn store_flush_load_and_delete() {
        let dir = std::env::temp_dir().join(format!("kiln-entity-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = ChunkPos { x: 3, z: -40 };
        let b = ChunkPos { x: -1, z: 0 };
        let mut s = EntityStore::new(&dir);
        assert!(s.load(a).is_empty());
        s.store(a, vec![entity(1), entity(2)]);
        s.store(b, vec![entity(3)]);
        // Pending writes are served before they reach the disk.
        assert_eq!(s.load(a), vec![entity(1), entity(2)]);
        s.flush().unwrap();
        let mut s = EntityStore::new(&dir);
        assert_eq!(s.load(a), vec![entity(1), entity(2)]);
        assert_eq!(s.load(b), vec![entity(3)]);
        // Raw chunk layout as vanilla writes it.
        let mut region = RegionFile::open(&dir.join("r.0.-2.mca")).unwrap();
        let (_, root) = nbt::read_named(&region.read(3, 24).unwrap().unwrap()).unwrap();
        assert_eq!(root.get("Position"), Some(&Tag::IntArray(vec![3, -40])));
        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(DATA_VERSION as i32)));
        s.store(a, Vec::new());
        assert!(s.load(a).is_empty());
        s.flush().unwrap();
        let mut s = EntityStore::new(&dir);
        assert!(s.load(a).is_empty());
        assert_eq!(s.load(b), vec![entity(3)]);
        // Storing an already empty chunk empty again queues nothing.
        s.store(a, Vec::new());
        assert_eq!(s.flush().unwrap(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// With the encoder thread, a chunk read back at any moment (its encoding out or back, flushed
    /// or not) has its newest entities, never an older copy, and a flush writes the newest.
    #[test]
    fn entities_read_back_while_encoding_are_the_newest() {
        let dir = std::env::temp_dir().join(format!("kiln-entity-encoder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = EntityStore::new(&dir);
        s.set_background(true);
        let chunks: Vec<ChunkPos> = (0..40).map(|i| ChunkPos { x: i % 7 - 3, z: i / 7 - 40 }).collect();
        let mut newest: HashMap<ChunkPos, Vec<Tag>> = HashMap::new();
        for round in 0..30 {
            for (i, &c) in chunks.iter().enumerate() {
                // Some rounds leave a chunk empty, some store more entities than before.
                let n = (round * 7 + i) % 5;
                let list: Vec<Tag> = (0..n as i32).map(|k| entity(round as i32 * 100 + k)).collect();
                s.store(c, list.clone());
                newest.insert(c, list);
                if (round + i) % 3 == 0 {
                    assert_eq!(s.load(c), newest[&c], "round {round}, chunk {c:?}");
                }
            }
            if round % 10 == 9 {
                s.flush().unwrap();
            }
            for &c in &chunks {
                assert_eq!(s.load(c), newest[&c], "round {round}, chunk {c:?}");
            }
        }
        s.sync().unwrap();
        let mut fresh = EntityStore::new(&dir);
        for &c in &chunks {
            assert_eq!(fresh.load(c), newest[&c], "after the flush, chunk {c:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
