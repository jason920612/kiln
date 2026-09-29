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
}

impl PoiStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { native: None, dir: dir.into(), regions: HashMap::new(), pending: HashMap::new() }
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
        let data = if let Some(store) = &self.native {
            store.lock().unwrap().read(POI, pos).map(|(_, raw, _)| raw)?
        } else {
            let (key, local) = Self::local(pos);
            if let Some((_, _, payload)) = self.pending.get(&key).and_then(|p| p.iter().find(|(x, z, _)| (*x, *z) == local)) {
                crate::region::decompress_chunk(payload.as_ref()?).map_err(|e| warn!("poi chunk {pos:?}: {e}")).ok()?
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
        let nbt = tag.map(|t| {
            let mut buf = bytes::BytesMut::new();
            t.write_named("", &mut buf);
            buf
        });
        if let Some(store) = &self.native {
            let mut store = store.lock().unwrap();
            if nbt.is_some() || store.contains(POI, pos) {
                store.write(POI, pos, FORM_NBT, nbt.as_deref());
            }
            return;
        }
        let payload = nbt.map(|b| crate::region::compress_chunk(&b));
        let (key, (lx, lz)) = Self::local(pos);
        let entry = self.pending.entry(key).or_default();
        entry.retain(|(x, z, _)| (*x, *z) != (lx, lz));
        entry.push((lx, lz, payload));
    }

    /// Writes the queued chunks into their region files.
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
            if !path.exists() && updates.iter().all(|(_, _, p)| p.is_none()) {
                continue;
            }
            written += updates.len();
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
}
