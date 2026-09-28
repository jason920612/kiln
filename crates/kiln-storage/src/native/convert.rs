//! `kiln world convert`: moves a world between Anvil and the native format. Chunk, entity and
//! POI data go through [`NativeChunk`] records or as stored NBT, so converting to native and
//! back gives every chunk's NBT byte for byte (and the Anvil header timestamps); every other
//! file of the world is copied unchanged. [`compare_worlds`] checks two Anvil worlds for that.

use super::cellfile::{self, CHUNK, CellFile, ENTITIES, Key, POI, Record};
use super::chunk::NativeChunk;
use super::registry::{Registry, Remap};
use super::{Compressor, Dictionaries, FORM_NATIVE, FORM_NBT, WorldFormat, ZSTD_LEVEL, cell_path};
use crate::anvil::AnvilSource;
use crate::region::{RegionFile, compress_chunk, write_region_stamped};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Anvil directories of a dimension and the record kinds they become.
const KINDS: [(&str, u8); 3] = [("region", CHUNK), ("entities", ENTITIES), ("poi", POI)];
const NATIVE_DIR: &str = "native";

#[derive(Default, Debug, Clone)]
pub struct Report {
    pub dimensions: usize,
    /// Chunks converted through the native layout.
    pub native_chunks: usize,
    /// Chunks kept as NBT: not fully generated.
    pub proto_chunks: usize,
    /// Fully generated chunks kept as NBT (the native layout would not reproduce them).
    pub nbt_chunks: usize,
    pub entity_chunks: usize,
    pub poi_chunks: usize,
    /// Chunk data that could not be read (left out, with a warning).
    pub unreadable: usize,
    pub files_copied: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub seconds: f64,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} dimensions: {} chunks native, {} not fully generated and {} other kept as NBT, {} entity chunks, {} POI chunks, {} unreadable; {} other files copied; chunk data {:.1} MiB -> {:.1} MiB in {:.1} s",
            self.dimensions,
            self.native_chunks,
            self.proto_chunks,
            self.nbt_chunks,
            self.entity_chunks,
            self.poi_chunks,
            self.unreadable,
            self.files_copied,
            self.bytes_in as f64 / 1048576.0,
            self.bytes_out as f64 / 1048576.0,
            self.seconds
        )
    }
}

/// Converts the world in `src` into a new world in `dst` stored as `to`.
pub fn convert_world(src: &Path, dst: &Path, to: WorldFormat, threads: usize) -> io::Result<Report> {
    let start = Instant::now();
    if !src.join("level.dat").exists() {
        return Err(io::Error::other(format!("{} is not a world (no level.dat)", src.display())));
    }
    if dst.exists() && std::fs::read_dir(dst)?.next().is_some() {
        return Err(io::Error::other(format!("{} exists and is not empty", dst.display())));
    }
    let from = WorldFormat::of(src);
    if from == to {
        return Err(io::Error::other(format!("{} is already {to:?}", src.display())));
    }
    let mut report = Report::default();
    let mut dims = Vec::new();
    copy_other_files(src, dst, Path::new(""), &mut dims, &mut report)?;
    report.dimensions = dims.len();
    for dim in &dims {
        let (s, d) = (src.join(dim), dst.join(dim));
        let r = match to {
            WorldFormat::Native => to_native(&s, &d, threads)?,
            WorldFormat::Anvil => to_anvil(&s, &d, threads)?,
        };
        report.native_chunks += r.native_chunks;
        report.proto_chunks += r.proto_chunks;
        report.nbt_chunks += r.nbt_chunks;
        report.entity_chunks += r.entity_chunks;
        report.poi_chunks += r.poi_chunks;
        report.unreadable += r.unreadable;
        report.bytes_in += r.bytes_in;
        report.bytes_out += r.bytes_out;
    }
    WorldFormat::mark(dst, to)?;
    report.seconds = start.elapsed().as_secs_f64();
    Ok(report)
}

/// Copies every file but chunk storage and the lock, noting dimension directories (those
/// holding `region`, `entities`, `poi` or `native`).
fn copy_other_files(src: &Path, dst: &Path, rel: &Path, dims: &mut Vec<PathBuf>, report: &mut Report) -> io::Result<()> {
    let dir = src.join(rel);
    let mut is_dim = false;
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        let name = e.file_name();
        let sub = rel.join(&name);
        if e.file_type()?.is_dir() {
            let n = name.to_string_lossy();
            if rel.starts_with("dimensions") && (KINDS.iter().any(|(k, _)| *k == n) || n == NATIVE_DIR) {
                is_dim = true;
                continue;
            }
            copy_other_files(src, dst, &sub, dims, report)?;
        } else {
            if sub == Path::new("session.lock") || sub == Path::new("kiln/world_format") {
                continue;
            }
            std::fs::create_dir_all(dst.join(rel))?;
            std::fs::copy(e.path(), dst.join(&sub))?;
            report.files_copied += 1;
        }
    }
    if is_dim {
        dims.push(rel.to_owned());
    }
    Ok(())
}

fn region_files(dir: &Path) -> io::Result<Vec<(i32, i32)>> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return Ok(out) };
    for e in rd {
        let name = e?.file_name();
        let name = name.to_string_lossy();
        let Some(mid) = name.strip_prefix("r.").and_then(|n| n.strip_suffix(".mca")) else { continue };
        let mut it = mid.split('.');
        if let (Some(Ok(x)), Some(Ok(z)), None) = (it.next().map(str::parse), it.next().map(str::parse), it.next()) {
            out.push((x, z));
        }
    }
    Ok(out)
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

/// Runs `work` over `items` on `threads` threads.
fn parallel<T: Sync>(items: &[T], threads: usize, work: impl Fn(&T) -> io::Result<()> + Sync) -> io::Result<()> {
    let next = AtomicUsize::new(0);
    let error = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                while let Some(item) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let Err(e) = work(item) {
                        error.lock().unwrap().get_or_insert(e);
                        next.store(items.len(), Ordering::Relaxed);
                    }
                }
            });
        }
    });
    match error.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Stored chunk NBT of every chunk of region files, by kind: (kind, global x, global z,
/// timestamp, NBT).
/// A chunk for a region file: local x, local z, compressed payload, timestamp.
type RegionEntry = (usize, usize, Vec<u8>, u32);

type Chunks = Vec<(u8, i32, i32, u32, Vec<u8>)>;

fn read_region(dim: &Path, (rx, rz): (i32, i32), unreadable: &AtomicUsize) -> io::Result<(Chunks, u64)> {
    let mut out = Vec::new();
    let mut bytes = 0;
    for (sub, kind) in KINDS {
        let path = dim.join(sub).join(format!("r.{rx}.{rz}.mca"));
        if !path.exists() {
            continue;
        }
        bytes += file_size(&path);
        let mut f = RegionFile::open(&path).map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
        for lz in 0..32 {
            for lx in 0..32 {
                if !f.contains(lx, lz) {
                    continue;
                }
                let (x, z) = (rx * 32 + lx as i32, rz * 32 + lz as i32);
                match f.read(lx, lz) {
                    Ok(Some(nbt)) => out.push((kind, x, z, f.timestamp(lx, lz), nbt)),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("{} chunk {x},{z}: {e}; left out", path.display());
                        unreadable.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    Ok((out, bytes))
}

fn to_native(src: &Path, dst: &Path, threads: usize) -> io::Result<Report> {
    let out_dir = dst.join(NATIVE_DIR);
    std::fs::create_dir_all(&out_dir)?;
    Registry::save_current(&out_dir)?;
    let mut regions: Vec<(i32, i32)> = KINDS.iter().map(|(k, _)| region_files(&src.join(k))).collect::<io::Result<Vec<_>>>()?.concat();
    regions.sort_unstable();
    regions.dedup();

    // The dictionary, trained on native records spread over the dimension.
    let mut samples = Vec::new();
    let codec = AnvilSource::new(PathBuf::new());
    let ignored = AtomicUsize::new(0);
    let step = regions.len().div_ceil(8).max(1);
    for r in regions.iter().step_by(step) {
        let (chunks, _) = read_region(src, *r, &ignored)?;
        let per = chunks.len().div_ceil(super::DICT_SAMPLES / 8).max(1);
        for (kind, .., nbt) in chunks.iter().step_by(per) {
            if *kind == CHUNK
                && let Some(c) = NativeChunk::from_nbt(nbt, &codec)
            {
                samples.push(c.encode());
            }
        }
    }
    let dict = if samples.len() >= 16 { Dictionaries::train(&out_dir, &samples) } else { None };
    let registry = Registry::current().fingerprint;

    let totals = Mutex::new(Report::default());
    let unreadable = AtomicUsize::new(0);
    parallel(&regions, threads, |&r| {
        let (chunks, bytes_in) = read_region(src, r, &unreadable)?;
        let mut compressor = Compressor::new(ZSTD_LEVEL, dict.as_ref().map(|(id, d)| (*id, &d[..])));
        let codec = AnvilSource::new(PathBuf::new());
        let mut local = Report { bytes_in, ..Report::default() };
        let mut cells: BTreeMap<(i32, i32), Vec<(Key, Record)>> = BTreeMap::new();
        for (kind, x, z, stamp, nbt) in chunks {
            let (cell, slot) = super::cell_of(kiln_world::ChunkPos::new(x, z));
            let rec = match kind {
                CHUNK => match NativeChunk::from_nbt(&nbt, &codec) {
                    Some(c) => {
                        local.native_chunks += 1;
                        compressor.record(FORM_NATIVE, &c.encode(), registry, stamp)
                    }
                    None => {
                        if is_full(&nbt) {
                            local.nbt_chunks += 1;
                        } else {
                            local.proto_chunks += 1;
                        }
                        compressor.record(FORM_NBT, &nbt, 0, stamp)
                    }
                },
                _ => {
                    if kind == ENTITIES {
                        local.entity_chunks += 1;
                    } else {
                        local.poi_chunks += 1;
                    }
                    compressor.record(FORM_NBT, &nbt, 0, stamp)
                }
            };
            cells.entry(cell).or_default().push((Key { kind, slot }, rec));
        }
        for (cell, records) in cells {
            let path = cell_path(&out_dir, cell);
            cellfile::write_new(&path, records, true)?;
            local.bytes_out += file_size(&path);
        }
        let mut t = totals.lock().unwrap();
        t.native_chunks += local.native_chunks;
        t.proto_chunks += local.proto_chunks;
        t.nbt_chunks += local.nbt_chunks;
        t.entity_chunks += local.entity_chunks;
        t.poi_chunks += local.poi_chunks;
        t.bytes_in += local.bytes_in;
        t.bytes_out += local.bytes_out;
        Ok(())
    })?;
    let mut report = totals.into_inner().unwrap();
    report.unreadable = unreadable.into_inner();
    for f in std::fs::read_dir(&out_dir)? {
        let f = f?;
        if f.file_type()?.is_file() && !f.file_name().to_string_lossy().ends_with(".kcell") {
            report.bytes_out += f.metadata()?.len();
        }
    }
    Ok(report)
}

fn is_full(nbt: &[u8]) -> bool {
    kiln_proto::nbt::read_named(nbt).ok().is_some_and(|(_, t)| {
        t.get("Status").or_else(|| t.get("status")).and_then(kiln_proto::nbt::Tag::as_str) == Some("minecraft:full")
    })
}

fn cell_files(dir: &Path) -> io::Result<Vec<(i32, i32)>> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return Ok(out) };
    for e in rd {
        let name = e?.file_name();
        let name = name.to_string_lossy();
        let Some(mid) = name.strip_prefix("c.").and_then(|n| n.strip_suffix(".kcell")) else { continue };
        let mut it = mid.split('.');
        if let (Some(Ok(x)), Some(Ok(z)), None) = (it.next().map(str::parse), it.next().map(str::parse), it.next()) {
            out.push((x, z));
        }
    }
    Ok(out)
}

fn to_anvil(src: &Path, dst: &Path, threads: usize) -> io::Result<Report> {
    let in_dir = src.join(NATIVE_DIR);
    let mut regions: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    for c in cell_files(&in_dir)? {
        regions.entry((c.0.div_euclid(4), c.1.div_euclid(4))).or_default().push(c);
    }
    let mut regions: Vec<_> = regions.into_iter().collect();
    regions.sort_unstable();
    let remaps: Mutex<HashMap<u32, Option<std::sync::Arc<Remap>>>> = Mutex::new(HashMap::new());
    let totals = Mutex::new(Report::default());
    parallel(&regions, threads, |((rx, rz), cells)| {
        let mut dicts = Dictionaries::new(&in_dir);
        let mut local = Report::default();
        let mut out: HashMap<u8, Vec<RegionEntry>> = HashMap::new();
        for &cell in cells {
            let path = cell_path(&in_dir, cell);
            local.bytes_in += file_size(&path);
            let Some(mut f) = CellFile::open(&path)? else { continue };
            for key in f.keys().collect::<Vec<_>>() {
                let rec = f.read(key)?.expect("listed");
                let raw = dicts.decompress(&rec)?;
                let nbt = if key.kind == CHUNK && rec.form == FORM_NATIVE {
                    let mut c = NativeChunk::decode(&raw).ok_or_else(|| io::Error::other(format!("{}: corrupt chunk record", path.display())))?;
                    if rec.registry != Registry::current().fingerprint {
                        let m = remaps
                            .lock()
                            .unwrap()
                            .entry(rec.registry)
                            .or_insert_with(|| Registry::load(&in_dir, rec.registry).map(|r| std::sync::Arc::new(Remap::to_current(&r))))
                            .clone();
                        match m {
                            Some(m) => c.remap(&m),
                            None => return Err(io::Error::other(format!("{}: id table {:08x} missing", in_dir.display(), rec.registry))),
                        }
                    }
                    local.native_chunks += 1;
                    let mut b = bytes::BytesMut::new();
                    c.to_nbt().write_named("", &mut b);
                    b.to_vec()
                } else {
                    match key.kind {
                        CHUNK if is_full(&raw) => local.nbt_chunks += 1,
                        CHUNK => local.proto_chunks += 1,
                        ENTITIES => local.entity_chunks += 1,
                        _ => local.poi_chunks += 1,
                    }
                    raw
                };
                let (x, z) = (cell.0 * 8 + (key.slot & 7) as i32, cell.1 * 8 + (key.slot >> 3) as i32);
                out.entry(key.kind).or_default().push(((x & 31) as usize, (z & 31) as usize, compress_chunk(&nbt), rec.stamp));
            }
        }
        for (sub, kind) in KINDS {
            let Some(chunks) = out.get(&kind) else { continue };
            let dir = dst.join(sub);
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("r.{rx}.{rz}.mca"));
            let updates: Vec<(usize, usize, &[u8], u32)> = chunks.iter().map(|(x, z, p, s)| (*x, *z, &p[..], *s)).collect();
            write_region_stamped(&path, &updates).map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
            local.bytes_out += file_size(&path);
        }
        let mut t = totals.lock().unwrap();
        t.native_chunks += local.native_chunks;
        t.proto_chunks += local.proto_chunks;
        t.nbt_chunks += local.nbt_chunks;
        t.entity_chunks += local.entity_chunks;
        t.poi_chunks += local.poi_chunks;
        t.bytes_in += local.bytes_in;
        t.bytes_out += local.bytes_out;
        Ok(())
    })?;
    Ok(totals.into_inner().unwrap())
}

/// Differences between two Anvil worlds: chunk NBT compared per chunk (bytes, after
/// decompression) and header timestamps; every other file byte for byte. `session.lock` and
/// Kiln's format marker are ignored.
pub fn compare_worlds(a: &Path, b: &Path) -> io::Result<(usize, Vec<String>)> {
    let mut files = BTreeMap::new();
    list_files(a, Path::new(""), &mut files, 0)?;
    list_files(b, Path::new(""), &mut files, 1)?;
    let mut diffs = Vec::new();
    let mut chunks = 0;
    for (rel, seen) in files {
        if rel == Path::new("session.lock") || rel == Path::new("kiln/world_format") {
            continue;
        }
        if seen != [true, true] {
            diffs.push(format!("{}: only in {}", rel.display(), if seen[0] { "the first" } else { "the second" }));
            continue;
        }
        let (pa, pb) = (a.join(&rel), b.join(&rel));
        if rel.extension().is_some_and(|e| e == "mca") {
            let open = |p: &Path| RegionFile::open(p).map_err(|e| io::Error::other(format!("{}: {e}", p.display())));
            let (mut ra, mut rb) = (open(&pa)?, open(&pb)?);
            for i in 0..1024 {
                let (x, z) = (i & 31, i >> 5);
                let (ca, cb) = (ra.read(x, z).ok().flatten(), rb.read(x, z).ok().flatten());
                if ca.is_some() {
                    chunks += 1;
                }
                if ca != cb {
                    diffs.push(format!("{} chunk {x},{z}: NBT differs", rel.display()));
                } else if ca.is_some() && ra.timestamp(x, z) != rb.timestamp(x, z) {
                    // (Vanilla leaves the timestamp of a chunk it deleted.)
                    diffs.push(format!("{} chunk {x},{z}: timestamp differs", rel.display()));
                }
            }
        } else if std::fs::read(&pa)? != std::fs::read(&pb)? {
            diffs.push(format!("{}: contents differ", rel.display()));
        }
    }
    Ok((chunks, diffs))
}

fn list_files(root: &Path, rel: &Path, out: &mut BTreeMap<PathBuf, [bool; 2]>, side: usize) -> io::Result<()> {
    for e in std::fs::read_dir(root.join(rel))? {
        let e = e?;
        let sub = rel.join(e.file_name());
        if e.file_type()?.is_dir() {
            list_files(root, &sub, out, side)?;
        } else if !e.file_name().to_string_lossy().ends_with(".mcc") {
            // External chunks are compared through their region file.
            out.entry(sub).or_default()[side] = true;
        }
    }
    Ok(())
}
