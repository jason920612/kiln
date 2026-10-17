//! Point of interest chunks: a dimension's `poi/r.<x>.<z>.mca`, one compound per chunk as
//! vanilla's `SectionStorage` writes it (`Sections` by section y, `DataVersion`), or the POI
//! records of a native world's cell files.

use crate::native::{FORM_NBT, NativeStore, POI};
use crate::region::RegionFile;
use kiln_proto::nbt::{self, Tag};
use kiln_world::ChunkPos;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::warn;

type Update = (usize, usize, Option<Vec<u8>>);

pub struct PoiStore {
    native: Option<Arc<Mutex<NativeStore>>>,
    dir: PathBuf,
    regions: HashMap<(i32, i32), Option<RegionFile>>,
    /// Writes waiting for `flush`, per region; loads read them first.
    pending: HashMap<(i32, i32), Vec<Update>>,
    /// Region writes on a thread of its own, once [`Self::set_background`] asked.
    writer: Option<crate::region::BackgroundWriter>,
    /// Encoding on a thread of its own, once [`Self::set_background`] asked.
    encoder: Option<crate::encoder::Encoder>,
}

impl PoiStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { native: None, dir: dir.into(), regions: HashMap::new(), pending: HashMap::new(), writer: None, encoder: None }
    }

    /// Point of interest chunks kept in a native store.
    pub fn native(store: Arc<Mutex<NativeStore>>) -> Self {
        let mut s = Self::new(PathBuf::new());
        s.native = Some(store);
        s
    }

    fn local(pos: ChunkPos) -> ((i32, i32), (usize, usize)) {
        ((pos.x >> 5, pos.z >> 5), ((pos.x & 31) as usize, (pos.z & 31) as usize))
    }

    /// The saved compound of a chunk, if any.
    pub fn load(&mut self, pos: ChunkPos) -> Option<Tag> {
        // Handed to the encoder and not back: the newest there is.
        if let Some(root) = self.encoder.as_ref().and_then(|e| e.newest(pos)) {
            return Some(root.as_ref().clone());
        }
        self.collect();
        let data = if let Some(store) = &self.native {
            store.lock().unwrap().read(POI, pos).map(|(_, raw, _)| raw)?
        } else {
            let (key, local) = Self::local(pos);
            let mut being_written = None;
            if let Some(w) = self.writer.as_mut() {
                for k in w.written() {
                    self.regions.remove(&k);
                }
                being_written = w.find(key, local).cloned();
            }
            if let Some((_, _, payload)) = self.pending.get(&key).and_then(|p| p.iter().find(|(x, z, _)| (*x, *z) == local)) {
                crate::region::decompress_chunk(payload.as_ref()?).map_err(|e| warn!("poi chunk {pos:?}: {e}")).ok()?
            } else if let Some(p) = being_written {
                if p.is_empty() {
                    return None;
                }
                crate::region::decompress_chunk(&p).map_err(|e| warn!("poi chunk {pos:?}: {e}")).ok()?
            } else {
                let dir = &self.dir;
                let region = self.regions.entry(key).or_insert_with(|| {
                    let path = dir.join(format!("r.{}.{}.mca", key.0, key.1));
                    path.exists().then(|| RegionFile::open(&path)).and_then(|r| r.map_err(|e| warn!("cannot open {}: {e}", path.display())).ok())
                });
                match region.as_mut()?.read(local.0, local.1) {
                    Ok(data) => data?,
                    Err(e) => {
                        warn!("poi chunk {pos:?}: {e}");
                        return None;
                    }
                }
            }
        };
        match nbt::read_named(&data) {
            Ok((_, root @ Tag::Compound(_))) => Some(root),
            _ => {
                warn!("poi chunk {pos:?} is not a compound");
                None
            }
        }
    }

    /// Queues a chunk's compound for the next `flush` (`None` deletes it).
    pub fn store(&mut self, pos: ChunkPos, tag: Option<Tag>) {
        match (tag, self.encoder.as_mut()) {
            (Some(tag), Some(e)) => e.submit(pos, tag),
            (tag, encoder) => {
                // (Newer than whatever is still with the encoder.)
                if let Some(e) = encoder {
                    e.forget(pos);
                }
                let encoded = tag.map(|t| crate::encoder::encode(&t, self.native.is_none()));
                self.put(pos, encoded);
            }
        }
    }

    /// Queues a chunk's encoded compound (`None`: delete it) for the next `flush`.
    fn put(&mut self, pos: ChunkPos, encoded: Option<Vec<u8>>) {
        if let Some(store) = &self.native {
            let mut store = store.lock().unwrap();
            if encoded.is_some() || store.contains(POI, pos) {
                store.write(POI, pos, FORM_NBT, encoded.as_deref());
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

    /// Writes the queued chunks into their region files.
    /// Region files written on a thread of its own from now on (`flush` returns before they
    /// are written; [`Self::sync`] waits). Native stores keep writing in place.
    pub fn set_background(&mut self, on: bool) {
        if on && self.writer.is_none() && self.native.is_none() {
            self.writer = Some(crate::region::BackgroundWriter::start("kiln-write-poi"));
        }
        if on && self.encoder.is_none() {
            self.encoder = Some(crate::encoder::Encoder::start("kiln-encode-poi", self.native.is_none()));
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
        if let Some(e) = self.encoder.as_mut() {
            for (pos, encoded) in e.drain() {
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

    #[test]
    fn store_flush_load() {
        let dir = std::env::temp_dir().join(format!("kiln-poi-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = ChunkPos { x: -3, z: 40 };
        let mut pois = kiln_world::poi::ChunkPois::default();
        pois.add([-40, 70, 650], kiln_world::poi::type_index("minecraft:meeting").unwrap());
        let tag = pois.to_nbt(crate::anvil::DATA_VERSION as i32);
        let mut s = PoiStore::new(&dir);
        assert!(s.load(a).is_none());
        s.store(a, Some(tag.clone()));
        assert_eq!(s.load(a), Some(tag.clone()));
        s.flush().unwrap();
        let mut s = PoiStore::new(&dir);
        let back = s.load(a).unwrap();
        assert_eq!(kiln_world::poi::ChunkPois::from_nbt(&back).sections, pois.sections);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// With the encoder thread, reads see the newest compound of a chunk whether its encoding is
    /// out or back, and a sync writes the newest.
    #[test]
    fn newest_compound_while_encoding() {
        let dir = std::env::temp_dir().join(format!("kiln-poi-encoder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = PoiStore::new(&dir);
        s.set_background(true);
        let tag = |n: i32| {
            let mut pois = kiln_world::poi::ChunkPois::default();
            for k in 0..n {
                pois.add([k, 70, 650], kiln_world::poi::type_index("minecraft:meeting").unwrap());
            }
            pois.to_nbt(crate::anvil::DATA_VERSION as i32)
        };
        let chunks: Vec<ChunkPos> = (0..12).map(|i| ChunkPos { x: i, z: 40 }).collect();
        let sections = |t: &Option<Tag>| t.as_ref().map(|t| kiln_world::poi::ChunkPois::from_nbt(t).sections);
        let mut newest = HashMap::new();
        for round in 0..20 {
            for (i, &c) in chunks.iter().enumerate() {
                let t = ((round + i) % 4 != 0).then(|| tag((round + i) as i32 % 5 + 1));
                s.store(c, t.clone());
                newest.insert(c, t);
                assert_eq!(sections(&s.load(c)), sections(&newest[&c]), "round {round}, chunk {c:?}");
            }
            if round % 7 == 6 {
                s.flush().unwrap();
            }
        }
        s.sync().unwrap();
        let mut fresh = PoiStore::new(&dir);
        for &c in &chunks {
            assert_eq!(sections(&fresh.load(c)), sections(&newest[&c]), "after the sync, chunk {c:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
