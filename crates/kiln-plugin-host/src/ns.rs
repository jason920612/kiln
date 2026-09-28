//! Owned namespaces (design §11.3) and their files under `kiln/plugins/`:
//!
//! - `players/<uuid>.bin`: a player's data of every plugin;
//! - `cells/<namespace>/<level>/r.<x>.<z>.bin`: cell data, one sidecar per Anvil region file
//!   (32x32 chunks, so 4x4 cells of 8x8 chunks);
//! - `global.bin`: every plugin's global namespace.
//!
//! Data of plugins that are not loaded is kept and written back unchanged.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// One plugin's keys in a namespace.
pub(crate) type Kv = BTreeMap<String, Vec<u8>>;

/// A namespace's data by plugin (index in load order), plus data of unknown plugins by id.
#[derive(Clone, Default, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Ns {
    pub by_plugin: Vec<Kv>,
    pub unknown: BTreeMap<String, Kv>,
}

impl Ns {
    pub fn get(&self, plugin: usize, key: &str) -> Option<&Vec<u8>> {
        self.by_plugin.get(plugin)?.get(key)
    }

    pub fn put(&mut self, plugin: usize, key: String, val: Option<Vec<u8>>) {
        if self.by_plugin.len() <= plugin {
            self.by_plugin.resize_with(plugin + 1, Kv::new);
        }
        match val {
            Some(v) => self.by_plugin[plugin].insert(key, v),
            None => self.by_plugin[plugin].remove(&key),
        };
    }

    fn is_empty(&self) -> bool {
        self.by_plugin.iter().all(Kv::is_empty) && self.unknown.values().all(Kv::is_empty)
    }
}

/// A global value (typed so atomic operations have meaning).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GlobalValue {
    Int(i64),
    Bytes(Vec<u8>),
}

/// Every plugin's global namespace.
#[derive(Clone, Default, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Globals {
    pub by_plugin: Vec<BTreeMap<String, GlobalValue>>,
    pub unknown: BTreeMap<String, BTreeMap<String, GlobalValue>>,
}

impl Globals {
    pub fn get(&self, plugin: usize, key: &str) -> Option<&GlobalValue> {
        self.by_plugin.get(plugin)?.get(key)
    }

    pub fn entry(&mut self, plugin: usize) -> &mut BTreeMap<String, GlobalValue> {
        if self.by_plugin.len() <= plugin {
            self.by_plugin.resize_with(plugin + 1, BTreeMap::new);
        }
        &mut self.by_plugin[plugin]
    }
}

/// A cell: 8x8 chunks (128x128 blocks) of a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey {
    pub dim: u32,
    pub x: i32,
    pub z: i32,
}

impl CellKey {
    pub fn of_block(dim: u32, x: i32, z: i32) -> CellKey {
        CellKey { dim, x: x >> 7, z: z >> 7 }
    }

    /// The Anvil region file holding the cell.
    fn region(self) -> (u32, i32, i32) {
        (self.dim, self.x.div_euclid(4), self.z.div_euclid(4))
    }
}

/// Cell data of every level, loaded one sidecar at a time.
#[derive(Default)]
pub(crate) struct CellTable {
    pub cells: HashMap<CellKey, Ns>,
    /// Sidecars read (or found missing) already.
    loaded: HashSet<(u32, i32, i32)>,
}

/// Where cell data is kept instead of sidecar files: a world in Kiln's native format keeps it
/// in its cell files. Sidecars are per Anvil region (32x32 chunks) of a level.
pub trait CellSidecars: Send + Sync {
    fn read(&self, level: &str, rx: i32, rz: i32) -> Option<Vec<u8>>;
    /// Stores the region's data (`None` deletes it).
    fn write(&self, level: &str, rx: i32, rz: i32, data: Option<&[u8]>);
}

/// Where the namespaces are saved, and how plugin ids map to load order.
#[derive(Clone)]
pub(crate) struct Persist {
    pub root: PathBuf,
    /// Level keys by dimension index.
    pub levels: Vec<String>,
    pub ids: Vec<String>,
    /// Cell data store replacing the sidecar files.
    pub sidecars: Option<std::sync::Arc<dyn CellSidecars>>,
}

impl Persist {
    fn index(&self, id: &str) -> Option<usize> {
        self.ids.iter().position(|i| i == id)
    }

    pub fn player_path(&self, uuid: u128) -> PathBuf {
        self.root.join("players").join(format!("{}.bin", uuid::Uuid::from_u128(uuid).hyphenated()))
    }

    fn sidecar_path(&self, (dim, rx, rz): (u32, i32, i32)) -> PathBuf {
        let level = self.levels.get(dim as usize).map_or("unknown:level", String::as_str);
        let (ns, path) = level.split_once(':').unwrap_or(("minecraft", level));
        self.root.join("cells").join(ns).join(path).join(format!("r.{rx}.{rz}.bin"))
    }

    fn level(&self, dim: u32) -> &str {
        self.levels.get(dim as usize).map_or("unknown:level", String::as_str)
    }

    fn read_sidecar(&self, region: (u32, i32, i32)) -> Option<Vec<u8>> {
        match &self.sidecars {
            Some(s) => s.read(self.level(region.0), region.1, region.2),
            None => std::fs::read(self.sidecar_path(region)).ok(),
        }
    }

    fn global_path(&self) -> PathBuf {
        self.root.join("global.bin")
    }

    pub fn load_player(&self, uuid: u128) -> Ns {
        match std::fs::read(self.player_path(uuid)) {
            Ok(b) => Reader(&b).ns(self).unwrap_or_else(|| {
                tracing::warn!("plugin data of player {} is corrupt; starting empty", uuid::Uuid::from_u128(uuid));
                Ns::default()
            }),
            Err(_) => Ns::default(),
        }
    }

    pub fn save_player(&self, uuid: u128, ns: &Ns) {
        let path = self.player_path(uuid);
        if ns.is_empty() && !path.exists() {
            return;
        }
        let mut w = Vec::new();
        write_ns(&mut w, ns, self);
        write_file(&path, &w);
    }

    pub fn load_globals(&self) -> Globals {
        let Ok(b) = std::fs::read(self.global_path()) else { return Globals::default() };
        let mut r = Reader(&b);
        let mut g = Globals::default();
        let ok = (|| {
            let n = r.u32()?;
            for _ in 0..n {
                let id = r.str()?;
                let count = r.u32()?;
                let mut kv = BTreeMap::new();
                for _ in 0..count {
                    let key = r.str()?;
                    let val = match r.u8()? {
                        0 => GlobalValue::Int(i64::from_le_bytes(r.take(8)?.try_into().ok()?)),
                        _ => GlobalValue::Bytes(r.bytes()?),
                    };
                    kv.insert(key, val);
                }
                match self.index(&id) {
                    Some(i) => *g.entry(i) = kv,
                    None => drop(g.unknown.insert(id, kv)),
                }
            }
            Some(())
        })();
        if ok.is_none() {
            tracing::warn!("plugin global data is corrupt; starting empty");
            return Globals::default();
        }
        g
    }

    pub fn save_globals(&self, g: &Globals) {
        let mut w = Vec::new();
        let known = g.by_plugin.iter().enumerate().filter_map(|(i, kv)| Some((self.ids.get(i)?.as_str(), kv)));
        let all: Vec<(&str, &BTreeMap<String, GlobalValue>)> = known.chain(g.unknown.iter().map(|(k, v)| (k.as_str(), v))).collect();
        put_u32(&mut w, all.len() as u32);
        for (id, kv) in all {
            put_str(&mut w, id);
            put_u32(&mut w, kv.len() as u32);
            for (k, v) in kv {
                put_str(&mut w, k);
                match v {
                    GlobalValue::Int(i) => {
                        w.push(0);
                        w.extend_from_slice(&i.to_le_bytes());
                    }
                    GlobalValue::Bytes(b) => {
                        w.push(1);
                        put_bytes(&mut w, b);
                    }
                }
            }
        }
        write_file(&self.global_path(), &w);
    }

    /// Reads the sidecar holding `cell` into the table, once.
    pub fn ensure_cell(&self, table: &mut CellTable, cell: CellKey) {
        let region = cell.region();
        if !table.loaded.insert(region) {
            return;
        }
        let Some(b) = self.read_sidecar(region) else { return };
        let mut r = Reader(&b);
        let read = (|| {
            let n = r.u32()?;
            let mut out = Vec::new();
            for _ in 0..n {
                let x = r.u32()? as i32;
                let z = r.u32()? as i32;
                out.push((CellKey { dim: cell.dim, x, z }, r.ns(self)?));
            }
            Some(out)
        })();
        match read {
            Some(cells) => {
                for (k, ns) in cells {
                    table.cells.entry(k).or_insert(ns);
                }
            }
            None => tracing::warn!("plugin cell data {} is corrupt; ignored", self.sidecar_path(region).display()),
        }
    }

    pub fn save_cells(&self, table: &CellTable) {
        type Sidecars<'a> = BTreeMap<(u32, i32, i32), Vec<(&'a CellKey, &'a Ns)>>;
        let mut by_region: Sidecars = BTreeMap::new();
        for (k, ns) in &table.cells {
            by_region.entry(k.region()).or_default().push((k, ns));
        }
        for region in &table.loaded {
            by_region.entry(*region).or_default();
        }
        for (region, mut cells) in by_region {
            cells.retain(|(_, ns)| !ns.is_empty());
            let path = self.sidecar_path(region);
            if cells.is_empty() {
                match &self.sidecars {
                    Some(s) => s.write(self.level(region.0), region.1, region.2, None),
                    None => drop(std::fs::remove_file(&path)),
                }
                continue;
            }
            cells.sort_by_key(|(k, _)| **k);
            let mut w = Vec::new();
            put_u32(&mut w, cells.len() as u32);
            for (k, ns) in cells {
                put_u32(&mut w, k.x as u32);
                put_u32(&mut w, k.z as u32);
                write_ns(&mut w, ns, self);
            }
            match &self.sidecars {
                Some(s) => s.write(self.level(region.0), region.1, region.2, Some(&w)),
                None => write_file(&path, &w),
            }
        }
    }
}

/// Writes through a temporary file so a crash never leaves half a file.
fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("bin.tmp");
    let r = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = r {
        tracing::warn!("cannot save plugin data {}: {e}", path.display());
    }
}

fn put_u32(w: &mut Vec<u8>, v: u32) {
    w.extend_from_slice(&v.to_le_bytes());
}

fn put_bytes(w: &mut Vec<u8>, b: &[u8]) {
    put_u32(w, b.len() as u32);
    w.extend_from_slice(b);
}

fn put_str(w: &mut Vec<u8>, s: &str) {
    put_bytes(w, s.as_bytes());
}

fn write_ns(w: &mut Vec<u8>, ns: &Ns, p: &Persist) {
    let known = ns.by_plugin.iter().enumerate().filter_map(|(i, kv)| Some((p.ids.get(i)?.as_str(), kv)));
    let all: Vec<(&str, &Kv)> = known.chain(ns.unknown.iter().map(|(k, v)| (k.as_str(), v))).filter(|(_, kv)| !kv.is_empty()).collect();
    put_u32(w, all.len() as u32);
    for (id, kv) in all {
        put_str(w, id);
        put_u32(w, kv.len() as u32);
        for (k, v) in kv {
            put_str(w, k);
            put_bytes(w, v);
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        Some(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }
    fn ns(&mut self, p: &Persist) -> Option<Ns> {
        let mut ns = Ns::default();
        let n = self.u32()?;
        for _ in 0..n {
            let id = self.str()?;
            let count = self.u32()?;
            let mut kv = Kv::new();
            for _ in 0..count {
                let k = self.str()?;
                kv.insert(k, self.bytes()?);
            }
            match p.index(&id) {
                Some(i) => {
                    if ns.by_plugin.len() <= i {
                        ns.by_plugin.resize_with(i + 1, Kv::new);
                    }
                    ns.by_plugin[i] = kv;
                }
                None => drop(ns.unknown.insert(id, kv)),
            }
        }
        Some(ns)
    }
}

/// Hashes a namespace map in key order (for determinism tests).
pub(crate) fn hash_sorted<K: Ord + Hash, V: Hash, H: Hasher>(map: &HashMap<K, V>, h: &mut H) {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in entries {
        k.hash(h);
        v.hash(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persist(dir: &Path) -> Persist {
        Persist { root: dir.to_owned(), levels: vec!["minecraft:overworld".into()], ids: vec!["a".into(), "b".into()], sidecars: None }
    }

    #[test]
    fn namespaces_round_trip_and_keep_unknown_plugins() {
        let dir = std::env::temp_dir().join(format!("kiln-ns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = persist(&dir);
        let mut ns = Ns::default();
        ns.put(1, "k".into(), Some(vec![1, 2, 3]));
        ns.unknown.insert("gone".into(), Kv::from([("x".to_owned(), vec![9])]));
        p.save_player(7, &ns);
        assert_eq!(p.load_player(7), ns);

        let mut table = CellTable::default();
        let cell = CellKey::of_block(0, -200, 300);
        p.ensure_cell(&mut table, cell);
        table.cells.entry(cell).or_default().put(0, "claim".into(), Some(vec![4]));
        p.save_cells(&table);
        let mut again = CellTable::default();
        p.ensure_cell(&mut again, cell);
        assert_eq!(again.cells.get(&cell), table.cells.get(&cell));

        let mut g = Globals::default();
        g.entry(0).insert("n".into(), GlobalValue::Int(-5));
        g.entry(1).insert("b".into(), GlobalValue::Bytes(vec![1]));
        p.save_globals(&g);
        assert_eq!(p.load_globals(), g);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cells_map_to_anvil_regions() {
        assert_eq!(CellKey::of_block(0, -1, 0), CellKey { dim: 0, x: -1, z: 0 });
        assert_eq!(CellKey { dim: 0, x: -1, z: 4 }.region(), (0, -1, 1));
        assert_eq!(CellKey { dim: 0, x: 3, z: -4 }.region(), (0, 0, -1));
    }
}
