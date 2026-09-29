//! Kiln's optional native world format (design §9.4, M8): per dimension, a `native/`
//! directory of cell files (`c.<x>.<z>.kcell`, one per 8×8-chunk cell) holding chunks in
//! Kiln's container layout, entity chunks and POI chunks, each record zstd-compressed (level
//! 3) with a per-dimension dictionary; state and biome id tables in `registries/`. Anvil
//! stays the default; [`WorldFormat`] picks the format of a world, and [`convert`] moves a
//! world between the two without loss.

pub mod cellfile;
pub mod chunk;
pub mod convert;
pub mod registry;

use crate::anvil::AnvilSource;
use crate::native::cellfile::{CHUNK, CellFile, Compacted, CompactionPlan, Key, Record};
use crate::native::chunk::NativeChunk;
use crate::native::registry::{Registry, Remap};
use kiln_proto::nbt::{self, Tag};
use kiln_world::chunk::Chunk;
use kiln_world::{ChunkPos, ChunkSource, Dimension};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{info, warn};

pub use cellfile::{ENTITIES, PLUGIN, POI};

/// Chunk records: a [`NativeChunk`], or chunk NBT as stored (chunks not fully generated, and
/// chunks the native layout does not reproduce exactly).
pub const FORM_NATIVE: u8 = 0;
pub const FORM_NBT: u8 = 1;

const CODEC_STORED: u8 = 0;
const CODEC_ZSTD: u8 = 1;
pub const ZSTD_LEVEL: i32 = 3;
/// Dictionary size and the chunk records sampled to train one.
const DICT_SIZE: usize = 112 * 1024;
const DICT_SAMPLES: usize = 256;
/// Open cell files kept (least recently used ones are closed first).
const OPEN_FILES: usize = 256;

/// How a world's chunks are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldFormat {
    Anvil,
    Native,
}

/// Marks a world as native: `kiln/world_format` holding `native`.
const FORMAT_FILE: &str = "kiln/world_format";

impl WorldFormat {
    /// The format a world uses: native if marked so; otherwise Anvil, unless the world is new
    /// (no `level.dat`, no region files) and `wanted` is native, in which case it is marked.
    pub fn resolve(world: &Path, wanted: WorldFormat) -> WorldFormat {
        if Self::of(world) == WorldFormat::Native {
            return WorldFormat::Native;
        }
        if wanted == WorldFormat::Native {
            let fresh = !world.join("level.dat").exists() && !world.join("dimensions/minecraft/overworld/region").exists();
            if !fresh {
                warn!("{} is an Anvil world; it stays Anvil (convert it with `kiln world convert --to native`)", world.display());
                return WorldFormat::Anvil;
            }
            if let Err(e) = Self::mark(world, WorldFormat::Native) {
                warn!("cannot mark {} as a native world: {e}", world.display());
                return WorldFormat::Anvil;
            }
            return WorldFormat::Native;
        }
        WorldFormat::Anvil
    }

    /// The format a world is marked with (Anvil when unmarked).
    pub fn of(world: &Path) -> WorldFormat {
        match std::fs::read_to_string(world.join(FORMAT_FILE)) {
            Ok(s) if s.trim() == "native" => WorldFormat::Native,
            _ => WorldFormat::Anvil,
        }
    }

    pub fn mark(world: &Path, format: WorldFormat) -> std::io::Result<()> {
        let path = world.join(FORMAT_FILE);
        match format {
            WorldFormat::Anvil => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
            WorldFormat::Native => {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(path, "native\n")
            }
        }
    }
}

/// Timings of chunk loads from storage (read, decompress, decode), for both formats; a
/// summary is logged every [`LoadStats::REPORT_EVERY`] loads.
#[derive(Default, Clone)]
pub struct LoadStats {
    pub label: &'static str,
    /// Microseconds per load, in order (the first 1,000,000).
    pub loads_us: Vec<u32>,
    /// File opens (region or cell) and their total time.
    pub opens: u64,
    pub open_us: u64,
    /// Loads up to the last report.
    reported: usize,
}

impl LoadStats {
    pub const REPORT_EVERY: usize = 2000;

    pub fn new(label: &'static str) -> LoadStats {
        LoadStats { label, ..LoadStats::default() }
    }

    pub fn record(&mut self, start: Instant) {
        if self.loads_us.len() < 1_000_000 {
            self.loads_us.push(start.elapsed().as_micros().min(u32::MAX as u128) as u32);
        }
        if self.loads_us.len() >= self.reported + Self::REPORT_EVERY {
            // The loads since the last report.
            let recent = LoadStats { loads_us: self.loads_us[self.reported..].to_vec(), ..LoadStats::default() };
            self.reported = self.loads_us.len();
            info!("{}: {} (all: {})", self.label, recent.summary(), self.summary());
        }
    }

    pub fn record_open(&mut self, start: Instant) {
        self.opens += 1;
        self.open_us += start.elapsed().as_micros() as u64;
    }

    /// "n loads, mean, p50, p99" of the loads so far.
    pub fn summary(&self) -> String {
        let mut v = self.loads_us.clone();
        if v.is_empty() {
            return "no loads".into();
        }
        v.sort_unstable();
        let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
        let p = |q: f64| v[((v.len() as f64 * q) as usize).min(v.len() - 1)];
        format!(
            "{} chunk loads: mean {:.0} µs, p50 {} µs, p99 {} µs; {} file opens, mean {:.0} µs",
            v.len(),
            mean,
            p(0.5),
            p(0.99),
            self.opens,
            if self.opens > 0 { self.open_us as f64 / self.opens as f64 } else { 0.0 }
        )
    }
}

/// Compression with the dimension's current dictionary.
pub struct Compressor {
    dict: u32,
    inner: zstd::bulk::Compressor<'static>,
}

impl Compressor {
    pub fn new(level: i32, dict: Option<(u32, &[u8])>) -> Compressor {
        let inner = match dict {
            Some((_, d)) => zstd::bulk::Compressor::with_dictionary(level, d),
            None => zstd::bulk::Compressor::new(level),
        }
        .expect("zstd context");
        Compressor { dict: dict.map_or(0, |d| d.0), inner }
    }

    pub fn record(&mut self, form: u8, raw: &[u8], registry: u32, stamp: u32) -> Record {
        let data = self.inner.compress(raw).expect("in-memory compression");
        Record { form, codec: CODEC_ZSTD, dict: self.dict, registry, stamp, raw_len: raw.len() as u32, data }
    }
}

/// Dictionaries of a dimension: `dict.<id>.zdict`, the current one named by `dict.current`.
pub struct Dictionaries {
    dir: PathBuf,
    loaded: HashMap<u32, Option<zstd::bulk::Decompressor<'static>>>,
    plain: zstd::bulk::Decompressor<'static>,
}

impl Dictionaries {
    fn new(dir: &Path) -> Dictionaries {
        Dictionaries { dir: dir.to_owned(), loaded: HashMap::new(), plain: zstd::bulk::Decompressor::new().expect("zstd context") }
    }

    pub fn current(dir: &Path) -> Option<(u32, Vec<u8>)> {
        let id = u32::from_str_radix(std::fs::read_to_string(dir.join("dict.current")).ok()?.trim(), 16).ok()?;
        Some((id, std::fs::read(dir.join(format!("dict.{id:08x}.zdict"))).ok()?))
    }

    /// Trains a dictionary on sample records, saves it and makes it current.
    pub fn train(dir: &Path, samples: &[Vec<u8>]) -> Option<(u32, Vec<u8>)> {
        let dict = zstd::dict::from_samples(samples, DICT_SIZE).map_err(|e| warn!("cannot train a dictionary: {e}")).ok()?;
        let id = crc32fast::hash(&dict).max(1);
        let save = || -> std::io::Result<()> {
            std::fs::create_dir_all(dir)?;
            let path = dir.join(format!("dict.{id:08x}.zdict"));
            std::fs::write(path.with_extension("tmp"), &dict)?;
            std::fs::rename(path.with_extension("tmp"), &path)?;
            std::fs::write(dir.join("dict.tmp"), format!("{id:08x}\n"))?;
            std::fs::rename(dir.join("dict.tmp"), dir.join("dict.current"))
        };
        save().map_err(|e| warn!("cannot save a dictionary in {}: {e}", dir.display())).ok()?;
        Some((id, dict))
    }

    pub fn decompress(&mut self, rec: &Record) -> std::io::Result<Vec<u8>> {
        match rec.codec {
            CODEC_STORED => Ok(rec.data.clone()),
            CODEC_ZSTD => {
                let d = if rec.dict == 0 {
                    &mut self.plain
                } else {
                    let dir = &self.dir;
                    let entry = self.loaded.entry(rec.dict).or_insert_with(|| {
                        let bytes = std::fs::read(dir.join(format!("dict.{:08x}.zdict", rec.dict))).ok()?;
                        zstd::bulk::Decompressor::with_dictionary(&bytes).ok()
                    });
                    entry.as_mut().ok_or_else(|| std::io::Error::other(format!("missing dictionary {:08x}", rec.dict)))?
                };
                let out = d.decompress(&rec.data, rec.raw_len as usize)?;
                if out.len() != rec.raw_len as usize {
                    return Err(std::io::Error::other("record length mismatch"));
                }
                Ok(out)
            }
            c => Err(std::io::Error::other(format!("unknown record codec {c}"))),
        }
    }
}

pub fn cell_of(pos: ChunkPos) -> ((i32, i32), u8) {
    ((pos.x >> 3, pos.z >> 3), (((pos.z & 7) << 3) | (pos.x & 7)) as u8)
}

pub fn cell_path(dir: &Path, (cx, cz): (i32, i32)) -> PathBuf {
    dir.join(format!("c.{cx}.{cz}.kcell"))
}

/// Compactions running at once per store (more wait for a later flush).
const MAX_COMPACTIONS: usize = 2;

/// Where a store rewrites cell files that hold mostly stale records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionMode {
    /// On a background thread, one task per cell; the flush that noticed the waste only takes a
    /// snapshot, and a later flush swaps the compacted copy in (a rename plus the records
    /// appended meanwhile).
    Background,
    /// In the flush itself (the original behaviour; for comparisons).
    Inline,
}

/// What compaction has done in a store.
#[derive(Clone, Debug, Default)]
pub struct CompactionStats {
    pub started: u64,
    pub finished: u64,
    pub failed: u64,
    /// Results dropped because their file was replaced meanwhile.
    pub discarded: u64,
    /// Bytes of files before and after compaction.
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// Time the flushing thread spent on compaction: the snapshot and thread start, and the
    /// swap (in `Inline` mode all of it), in microseconds; and the worst single piece.
    pub owner_us: u64,
    pub owner_max_us: u64,
    /// Time the background threads spent copying, in microseconds.
    pub background_us: u64,
}

impl CompactionStats {
    fn owner(&mut self, start: Instant) {
        let us = start.elapsed().as_micros() as u64;
        self.owner_us += us;
        self.owner_max_us = self.owner_max_us.max(us);
    }
}

/// A finished background copy: its cell, the copy (or why there is none), the size of the file
/// it was taken from and the microseconds it took.
type CompactionResult = ((i32, i32), std::io::Result<Compacted>, u64, u64);

/// A dimension's native store, shared by its chunk source and entity store.
pub struct NativeStore {
    dir: PathBuf,
    files: HashMap<(i32, i32), (Option<CellFile>, u64)>,
    clock: u64,
    /// Writes not flushed yet (`None` deletes), per cell; loads read them first.
    pending: HashMap<(i32, i32), BTreeMap<Key, Option<Record>>>,
    compressor: Compressor,
    dicts: Dictionaries,
    /// Native chunk records kept to train the dimension's first dictionary.
    samples: Option<Vec<Vec<u8>>>,
    remaps: HashMap<u32, Option<Arc<Remap>>>,
    /// Sync each flush to disk.
    pub sync: bool,
    pub stats: LoadStats,
    pub mode: CompactionMode,
    pub compaction: CompactionStats,
    /// Cells being compacted on a thread, with the file size they started from.
    inflight: HashMap<(i32, i32), u64>,
    /// Cells whose file was replaced or removed while compacting: their result is dropped.
    stale: HashSet<(i32, i32)>,
    done_tx: Sender<CompactionResult>,
    done_rx: Receiver<CompactionResult>,
}

impl NativeStore {
    pub fn open(dir: impl Into<PathBuf>) -> NativeStore {
        let dir = dir.into();
        let dict = Dictionaries::current(&dir);
        let compressor = Compressor::new(ZSTD_LEVEL, dict.as_ref().map(|(id, d)| (*id, &d[..])));
        remove_stale_copies(&dir);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        NativeStore {
            samples: dict.is_none().then(Vec::new),
            dicts: Dictionaries::new(&dir),
            dir,
            files: HashMap::new(),
            clock: 0,
            pending: HashMap::new(),
            compressor,
            remaps: HashMap::new(),
            sync: true,
            stats: LoadStats::new("native chunk storage"),
            mode: CompactionMode::Background,
            compaction: CompactionStats::default(),
            inflight: HashMap::new(),
            stale: HashSet::new(),
            done_tx,
            done_rx,
        }
    }

    pub fn shared(dir: impl Into<PathBuf>) -> Arc<Mutex<NativeStore>> {
        Arc::new(Mutex::new(NativeStore::open(dir)))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&mut self, cell: (i32, i32)) -> Option<&mut CellFile> {
        self.clock += 1;
        let clock = self.clock;
        if !self.files.contains_key(&cell) {
            if self.files.len() >= OPEN_FILES {
                let oldest = self.files.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| *k);
                if let Some(k) = oldest {
                    self.files.remove(&k);
                }
            }
            let start = Instant::now();
            let path = cell_path(&self.dir, cell);
            let f = CellFile::open(&path).unwrap_or_else(|e| {
                warn!("cannot open {}: {e}", path.display());
                // A corrupt file is moved aside, so the next write starts a new one.
                if e.kind() == std::io::ErrorKind::InvalidData {
                    if self.inflight.contains_key(&cell) {
                        self.stale.insert(cell);
                    }
                    let aside = path.with_extension(format!("kcell.corrupt-{}", now()));
                    match std::fs::rename(&path, &aside) {
                        Ok(()) => warn!("moved to {}", aside.display()),
                        Err(e) => warn!("cannot move {}: {e}", path.display()),
                    }
                }
                None
            });
            self.stats.record_open(start);
            self.files.insert(cell, (f, clock));
        }
        let (f, t) = self.files.get_mut(&cell).unwrap();
        *t = clock;
        f.as_mut()
    }

    /// A record's payload, decompressed, and the id table it refers to.
    pub fn read(&mut self, kind: u8, pos: ChunkPos) -> Option<(u8, Vec<u8>, u32)> {
        let (cell, slot) = cell_of(pos);
        let key = Key { kind, slot };
        let rec = match self.pending.get(&cell).and_then(|p| p.get(&key)) {
            Some(r) => r.clone()?,
            None => match self.file(cell)?.read(key) {
                Ok(r) => r?,
                Err(e) => {
                    warn!("record {kind} of chunk {pos:?}: {e}");
                    return None;
                }
            },
        };
        match self.dicts.decompress(&rec) {
            Ok(raw) => Some((rec.form, raw, rec.registry)),
            Err(e) => {
                warn!("record {kind} of chunk {pos:?}: {e}");
                None
            }
        }
    }

    /// Whether a record exists (pending or stored).
    pub fn contains(&mut self, kind: u8, pos: ChunkPos) -> bool {
        let (cell, slot) = cell_of(pos);
        let key = Key { kind, slot };
        match self.pending.get(&cell).and_then(|p| p.get(&key)) {
            Some(r) => r.is_some(),
            None => self.file(cell).is_some_and(|f| f.contains(key)),
        }
    }

    /// Queues a record for the next flush (`None` deletes it).
    pub fn write(&mut self, kind: u8, pos: ChunkPos, form: u8, raw: Option<&[u8]>) {
        let (cell, slot) = cell_of(pos);
        let rec = raw.map(|raw| {
            if kind == CHUNK && form == FORM_NATIVE {
                self.sample(raw);
            }
            let registry = if kind == CHUNK && form == FORM_NATIVE { Registry::current().fingerprint } else { 0 };
            self.compressor.record(form, raw, registry, now())
        });
        self.pending.entry(cell).or_default().insert(Key { kind, slot }, rec);
    }

    /// Keeps chunk records until there are enough to train the dimension's dictionary.
    fn sample(&mut self, raw: &[u8]) {
        let Some(samples) = &mut self.samples else { return };
        samples.push(raw.to_vec());
        if samples.len() < DICT_SAMPLES {
            return;
        }
        let samples = self.samples.take().unwrap();
        let start = Instant::now();
        if let Some((id, dict)) = Dictionaries::train(&self.dir, &samples) {
            info!("{}: trained a {} KiB dictionary in {:.0} ms", self.dir.display(), dict.len() / 1024, start.elapsed().as_secs_f64() * 1e3);
            self.compressor = Compressor::new(ZSTD_LEVEL, Some((id, &dict)));
        }
    }

    /// The remap for records written against another id table.
    pub fn remap(&mut self, fingerprint: u32) -> Option<Arc<Remap>> {
        if fingerprint == 0 || fingerprint == Registry::current().fingerprint {
            return None;
        }
        let dir = &self.dir;
        self.remaps
            .entry(fingerprint)
            .or_insert_with(|| match Registry::load(dir, fingerprint) {
                Some(r) => Some(Arc::new(Remap::to_current(&r))),
                None => {
                    warn!("{}: id table {fingerprint:08x} is missing; its chunks load with this build's ids", dir.display());
                    None
                }
            })
            .clone()
    }

    /// The plugin cell data of Anvil region `(rx, rz)`.
    pub fn read_sidecar(&mut self, rx: i32, rz: i32) -> Option<Vec<u8>> {
        self.read(PLUGIN, ChunkPos::new(rx * 32, rz * 32)).map(|(_, raw, _)| raw)
    }

    /// Stores (or with `None` deletes) the plugin cell data of Anvil region `(rx, rz)` and
    /// flushes.
    pub fn write_sidecar(&mut self, rx: i32, rz: i32, data: Option<&[u8]>) -> std::io::Result<()> {
        let pos = ChunkPos::new(rx * 32, rz * 32);
        if data.is_some() || self.contains(PLUGIN, pos) {
            self.write(PLUGIN, pos, FORM_NBT, data);
            self.flush()?;
        }
        Ok(())
    }

    /// Writes the queued records, one append per cell file.
    pub fn flush(&mut self) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            return Ok(0);
        }
        std::fs::create_dir_all(&self.dir)?;
        Registry::save_current(&self.dir)?;
        self.poll_compactions();
        let mut written = 0;
        let mut cells: Vec<_> = std::mem::take(&mut self.pending).into_iter().collect();
        cells.sort_unstable_by_key(|(c, _)| *c);
        for (cell, updates) in cells {
            written += updates.len();
            let sync = self.sync;
            let path = cell_path(&self.dir, cell);
            if self.file(cell).is_none() {
                if updates.values().all(Option::is_none) {
                    continue;
                }
                if path.exists() {
                    // It could not be opened (and is not corrupt): never replace it; retry later.
                    warn!("{} is unavailable; its writes wait for the next save", path.display());
                    self.files.remove(&cell);
                    self.pending.insert(cell, updates);
                    continue;
                }
                if self.inflight.contains_key(&cell) {
                    self.stale.insert(cell);
                }
                let f = CellFile::create(&path)?;
                self.files.insert(cell, (Some(f), self.clock));
            }
            let file = self.file(cell).expect("just created");
            file.commit(updates, sync)?;
            self.compact_if_wasteful(cell)?;
        }
        Ok(written)
    }

    /// After a commit: an emptied file is deleted, and a file that is mostly stale records is
    /// compacted, in the background unless the store is in `Inline` mode.
    fn compact_if_wasteful(&mut self, cell: (i32, i32)) -> std::io::Result<()> {
        let sync = self.sync;
        let running = self.inflight.contains_key(&cell);
        let (empty, wasteful, before) = {
            let file = self.file(cell).expect("just committed");
            (file.is_empty(), file.wants_compaction(), file.file_len())
        };
        // A copy in progress finds an emptied file empty when it ends, and removes it.
        if running || !(empty || wasteful) {
            return Ok(());
        }
        let start = Instant::now();
        if self.mode == CompactionMode::Inline || empty {
            let (f, t) = self.files.remove(&cell).unwrap();
            let f = f.unwrap().compact(sync)?;
            self.compaction.started += 1;
            self.compaction.finished += 1;
            self.compaction.bytes_before += before;
            self.compaction.bytes_after += f.as_ref().map_or(0, CellFile::file_len);
            self.files.insert(cell, (f, t));
        } else if self.inflight.len() < MAX_COMPACTIONS {
            let plan = self.file(cell).expect("just committed").plan_compaction();
            let tx = self.done_tx.clone();
            let spawned = std::thread::Builder::new().name("kiln-compact".into()).spawn(move || {
                let start = Instant::now();
                let copy = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_plan(plan, sync)))
                    .unwrap_or_else(|_| Err(std::io::Error::other("compaction panicked")));
                let _ = tx.send((cell, copy, before, start.elapsed().as_micros() as u64));
            });
            match spawned {
                Ok(_) => {
                    self.inflight.insert(cell, before);
                    self.compaction.started += 1;
                }
                Err(e) => warn!("cannot start a compaction thread: {e}"),
            }
        }
        self.compaction.owner(start);
        Ok(())
    }

    /// Swaps in the compacted copies that background threads have finished.
    pub fn poll_compactions(&mut self) {
        while let Ok(done) = self.done_rx.try_recv() {
            self.finish_compaction(done);
        }
    }

    /// Waits for the running compactions and swaps their copies in (shutdown, tests).
    pub fn finish_compactions(&mut self) {
        while !self.inflight.is_empty() {
            match self.done_rx.recv() {
                Ok(done) => self.finish_compaction(done),
                Err(_) => break,
            }
        }
    }

    /// Cells being compacted on background threads.
    pub fn compactions_running(&self) -> usize {
        self.inflight.len()
    }

    fn finish_compaction(&mut self, (cell, copy, before, took_us): CompactionResult) {
        let start = Instant::now();
        self.inflight.remove(&cell);
        self.compaction.background_us += took_us;
        let sync = self.sync;
        let path = cell_path(&self.dir, cell);
        let copy = match copy {
            Ok(c) => c,
            Err(e) => {
                warn!("compacting {}: {e}", path.display());
                self.compaction.failed += 1;
                self.stale.remove(&cell);
                return;
            }
        };
        if self.stale.remove(&cell) {
            self.compaction.discarded += 1;
            copy.discard();
            return;
        }
        // The file as it is now: the records written since the snapshot come along.
        self.file(cell);
        let (file, t) = self.files.remove(&cell).unwrap_or((None, self.clock));
        let Some(file) = file else {
            copy.discard();
            self.files.insert(cell, (None, t));
            return;
        };
        match file.finish_compaction(copy, sync) {
            Ok(f) => {
                self.compaction.finished += 1;
                self.compaction.bytes_before += before;
                self.compaction.bytes_after += f.as_ref().map_or(0, CellFile::file_len);
                info!(
                    "compacted {}: {} KiB to {} KiB ({:.1} ms in the background, {:.2} ms to swap in)",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    before / 1024,
                    f.as_ref().map_or(0, CellFile::file_len) / 1024,
                    took_us as f64 / 1e3,
                    start.elapsed().as_secs_f64() * 1e3
                );
                self.files.insert(cell, (f, t));
            }
            Err(e) => {
                // The file stays as it was; it is opened again on next use.
                warn!("swapping in the compacted {}: {e}", path.display());
                self.compaction.failed += 1;
            }
        }
        self.compaction.owner(start);
    }
}

impl Drop for NativeStore {
    fn drop(&mut self) {
        self.finish_compactions();
    }
}

/// Runs a compaction plan, removing its copy again when it fails.
fn run_plan(plan: CompactionPlan, sync: bool) -> std::io::Result<Compacted> {
    let tmp = plan.temp_file();
    let r = plan.run(sync);
    if r.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    r
}

/// Copies left behind by a compaction that a crash cut short are of no use: the cell files
/// they were made from were never touched.
fn remove_stale_copies(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        if name.to_str().is_some_and(|n| n.ends_with(".kcell.compact")) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn now() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32)
}

/// Chunks from a dimension's native store.
pub struct NativeSource {
    store: Arc<Mutex<NativeStore>>,
    codec: AnvilSource,
    /// Loaded chunks' fields other than sections and block entities, written back.
    preserved: HashMap<ChunkPos, Tag>,
}

impl NativeSource {
    pub fn new(store: Arc<Mutex<NativeStore>>) -> NativeSource {
        NativeSource { store, codec: AnvilSource::new(PathBuf::new()), preserved: HashMap::new() }
    }

    pub fn store(&self) -> &Arc<Mutex<NativeStore>> {
        &self.store
    }
}

impl ChunkSource for NativeSource {
    fn load(&mut self, pos: ChunkPos, dim: Dimension) -> Option<Chunk> {
        let start = Instant::now();
        let mut store = self.store.lock().unwrap();
        let (form, raw, registry) = store.read(CHUNK, pos)?;
        let decoded = match form {
            FORM_NATIVE => match NativeChunk::decode(&raw) {
                Some(mut c) => {
                    if let Some(m) = store.remap(registry) {
                        c.remap(&m);
                    }
                    c.into_chunk(&mut self.codec, dim)
                }
                None => {
                    warn!("chunk {pos:?}: corrupt native record");
                    return None;
                }
            },
            _ => self.codec.decode(&raw, dim).map(|c| {
                let preserved = match nbt::read_named(&raw) {
                    Ok((_, Tag::Compound(mut f))) => {
                        f.retain(|(k, _)| k != "sections" && k != "block_entities");
                        Tag::Compound(f)
                    }
                    _ => Tag::Compound(Vec::new()),
                };
                (c, preserved)
            }),
        };
        if decoded.is_ok() {
            store.stats.record(start);
        }
        match decoded {
            Ok((mut chunk, preserved)) => {
                // Structure starts and references stay readable (location predicates).
                if let Some(s) = preserved.get("structures") {
                    chunk.structures = Some(Box::new(s.clone()));
                }
                self.preserved.insert(pos, preserved);
                Some(chunk)
            }
            Err(crate::anvil::ChunkError::NotFull(_)) => None,
            Err(e) => {
                warn!("chunk {pos:?}: {e}");
                None
            }
        }
    }

    fn save(&mut self, pos: ChunkPos, chunk: &Chunk) {
        let raw = NativeChunk::encode_chunk(pos, chunk, self.preserved.get(&pos));
        self.store.lock().unwrap().write(CHUNK, pos, FORM_NATIVE, Some(&raw));
    }

    fn unloaded(&mut self, pos: ChunkPos) {
        self.preserved.remove(&pos);
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut store = self.store.lock().unwrap();
        store.flush().map(|_| ())
    }
}
