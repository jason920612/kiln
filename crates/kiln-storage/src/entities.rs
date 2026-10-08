//! Entity chunks: a dimension's `entities/r.<x>.<z>.mca`, as vanilla `EntityStorage` keeps
//! them (`DataVersion`, `Position` as `[x, z]`, `Entities` with each entity's saved compound).

use crate::anvil::DATA_VERSION;
use crate::region::RegionFile;
use kiln_proto::nbt::{self, Tag};
use kiln_world::ChunkPos;
use std::collections::{HashMap, HashSet};
use crate::native::{ENTITIES, FORM_NBT, NativeStore};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
}

impl EntityStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { native: None, dir: dir.into(), regions: HashMap::new(), pending: HashMap::new(), writer: None, empty: HashSet::new(), warned_version: false }
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
        let nbt = if entities.is_empty() {
            if !self.empty.insert(pos) {
                return;
            }
            None
        } else {
            let root = Tag::Compound(vec![
                ("DataVersion".into(), Tag::Int(DATA_VERSION as i32)),
                ("Entities".into(), Tag::List(entities)),
                ("Position".into(), Tag::IntArray(vec![pos.x, pos.z])),
            ]);
            let mut buf = bytes::BytesMut::new();
            root.write_named("", &mut buf);
            self.empty.remove(&pos);
            Some(buf)
        };
        if let Some(store) = &self.native {
            let mut store = store.lock().unwrap();
            // Deleting what is not stored changes nothing.
            if nbt.is_some() || store.contains(ENTITIES, pos) {
                store.write(ENTITIES, pos, FORM_NBT, nbt.as_deref());
            }
            return;
        }
        let payload = nbt.map(|b| crate::region::compress_chunk(&b));
        let (key, (lx, lz)) = Self::local(pos);
        let entry = self.pending.entry(key).or_default();
        entry.retain(|(x, z, _)| (*x, *z) != (lx, lz));
        entry.push((lx, lz, payload));
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
}
