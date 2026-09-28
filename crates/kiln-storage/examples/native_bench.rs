//! Native format against Anvil: conversion, load and save throughput, and disk size.
//!
//! usage:
//!   cargo run --release -p kiln-storage --example native_bench -- gen <world> <regions-x> <regions-z> [threads]
//!       generates regions of vanilla noise terrain (FULL chunks, lit) into a new Anvil world
//!       (datapack from `$KILN_WORK/generated`, seed 12345)
//!   cargo run --release -p kiln-storage --example native_bench -- bench <anvil-world> [threads]
//!       converts the world to native and back (checking every chunk), then loads and saves
//!       every overworld chunk through each format on one thread
//!
//! Work files go next to the world (`<world>-bench/`).

use kiln_storage::native::convert::{compare_worlds, convert_world};
use kiln_storage::{AnvilSource, NativeSource, NativeStore, WorldFormat};
use kiln_world::chunk::Chunk;
use kiln_world::{ChunkGenerator, ChunkPos, ChunkSource, Dimension, OVERWORLD, Terrain, World};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const OW: &str = "dimensions/minecraft/overworld";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    match args.first().map(String::as_str) {
        Some("gen") => {
            let n = |i: usize| args.get(i).and_then(|a| a.parse::<usize>().ok());
            gen_world(Path::new(&args[1]), n(2).unwrap_or(4) as i32, n(3).unwrap_or(5) as i32, n(4).unwrap_or(cores))
        }
        Some("bench") => bench(Path::new(&args[1]), args.get(2).and_then(|a| a.parse().ok()).unwrap_or(cores)),
        _ => eprintln!("usage: native_bench gen <world> <rx> <rz> [threads] | bench <world> [threads]"),
    }
}

/// Hands out chunks generated beforehand.
struct Pregenerated(HashMap<ChunkPos, Chunk>);

impl ChunkGenerator for Pregenerated {
    fn generate(&mut self, pos: ChunkPos, _: Dimension) -> Chunk {
        self.0.remove(&pos).expect("pregenerated chunk")
    }
    fn fork(&self) -> Box<dyn ChunkGenerator> {
        unimplemented!()
    }
}

fn gen_world(world: &Path, rx: i32, rz: i32, threads: usize) {
    let work = PathBuf::from(std::env::var_os("KILN_WORK").expect("set KILN_WORK"));
    let pack = kiln_worldgen::Datapack::load(&work.join("generated")).expect("datapack");
    let wg = std::sync::Arc::new(kiln_worldgen::Worldgen::overworld(&pack, 12345, true).expect("worldgen"));
    let pipeline = std::sync::Arc::new(kiln_worldgen::Pipeline::new(wg));
    let full = kiln_worldgen::FullChunks::new(pipeline);
    let _ = std::fs::remove_dir_all(world);
    std::fs::create_dir_all(world).unwrap();
    // A level.dat so the world converts (and a vanilla server could open it).
    std::fs::copy(work.join("vanilla-world/world/level.dat"), world.join("level.dat")).expect("reference level.dat");
    let start = Instant::now();
    let mut total = 0;
    for x in 0..rx {
        for z in 0..rz {
            // One region at a time: generated in parallel, then lit (neighbours inside the
            // region) and saved.
            let positions: Vec<ChunkPos> = (0..1024).map(|i| ChunkPos::new(x * 32 + (i & 31), z * 32 + (i >> 5))).collect();
            let next = std::sync::atomic::AtomicUsize::new(0);
            let chunks = std::sync::Mutex::new(HashMap::new());
            std::thread::scope(|s| {
                for _ in 0..threads {
                    let mut g = full.fork();
                    let (next, chunks, positions) = (&next, &chunks, &positions);
                    s.spawn(move || {
                        while let Some(&p) = positions.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                            let c = g.generate(p, OVERWORLD);
                            chunks.lock().unwrap().insert(p, c);
                        }
                    });
                }
            });
            let provider = kiln_world::ChunkProvider::with_source(
                OVERWORLD,
                Box::new(AnvilSource::new(world.join(OW).join("region"))),
                Terrain::Void,
                0,
                67,
            )
            .with_generator(Box::new(Pregenerated(chunks.into_inner().unwrap())));
            let mut w = World::new(provider);
            for &p in &positions {
                w.load_chunk(p);
            }
            for &p in &positions {
                kiln_world::light::light_new_chunk(&mut w, p);
            }
            total += w.save().unwrap();
            println!("region {x},{z}: {total} chunks, {:.0} s", start.elapsed().as_secs_f64());
        }
    }
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p).map_or(0, |rd| {
        rd.flatten().map(|e| if e.file_type().unwrap().is_dir() { dir_size(&e.path()) } else { std::fs::metadata(e.path()).unwrap().len() }).sum()
    })
}

fn mib(b: u64) -> String {
    format!("{:.1} MiB", b as f64 / 1048576.0)
}

fn chunk_positions(region_dir: &Path) -> Vec<ChunkPos> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(region_dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(mid) = name.strip_prefix("r.").and_then(|n| n.strip_suffix(".mca")) else { continue };
        let (x, z) = mid.split_once('.').unwrap();
        let (rx, rz): (i32, i32) = (x.parse().unwrap(), z.parse().unwrap());
        let f = kiln_storage::region::RegionFile::open(&e.path()).unwrap();
        for i in 0..1024 {
            if f.contains(i & 31, i >> 5) {
                out.push(ChunkPos::new(rx * 32 + (i & 31) as i32, rz * 32 + (i >> 5) as i32));
            }
        }
    }
    out.sort_by_key(|p| (p.z, p.x));
    out
}

/// Loads every chunk; returns the chunks and the source's load summary.
fn load_all(src: &mut dyn ChunkSource, positions: &[ChunkPos]) -> (Vec<(ChunkPos, Chunk)>, f64) {
    let start = Instant::now();
    let chunks: Vec<_> = positions.iter().filter_map(|&p| src.load(p, OVERWORLD).map(|c| (p, c))).collect();
    (chunks, start.elapsed().as_secs_f64())
}

fn save_all(src: &mut dyn ChunkSource, chunks: &[(ChunkPos, Chunk)]) -> f64 {
    let start = Instant::now();
    for (p, c) in chunks {
        src.save(*p, c);
    }
    src.flush().unwrap();
    start.elapsed().as_secs_f64()
}

fn bench(world: &Path, threads: usize) {
    let work = world.with_file_name(format!("{}-bench", world.file_name().unwrap().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let (native, back) = (work.join("native"), work.join("back"));
    let r = convert_world(world, &native, WorldFormat::Native, threads).unwrap();
    println!("to native ({threads} threads): {r}; {:.0} chunks/s", r.native_chunks as f64 / r.seconds);
    let r = convert_world(&native, &back, WorldFormat::Anvil, threads).unwrap();
    println!("to Anvil ({threads} threads): {r}; {:.0} chunks/s", r.native_chunks as f64 / r.seconds);
    let start = Instant::now();
    let (n, diffs) = compare_worlds(world, &back).unwrap();
    println!("compared {n} chunks in {:.1} s: {} differences", start.elapsed().as_secs_f64(), diffs.len());
    for d in diffs.iter().take(10) {
        println!("  {d}");
    }

    let region = world.join(OW).join("region");
    let native_dir = native.join(OW).join("native");
    println!(
        "disk: Anvil region/ {} ({} files), native/ {} ({} files)",
        mib(dir_size(&region)),
        std::fs::read_dir(&region).unwrap().count(),
        mib(dir_size(&native_dir)),
        std::fs::read_dir(&native_dir).unwrap().count()
    );

    let positions = chunk_positions(&region);
    println!("{} overworld chunks; one thread from here", positions.len());
    let mut anvil = AnvilSource::new(&region);
    let (chunks, secs) = load_all(&mut anvil, &positions);
    println!("load Anvil:  {:.0} chunks/s; {}", chunks.len() as f64 / secs, anvil.stats.summary());
    let store = NativeStore::shared(&native_dir);
    let mut src = NativeSource::new(store.clone());
    let (nchunks, secs) = load_all(&mut src, &positions);
    println!("load native: {:.0} chunks/s; {}", nchunks.len() as f64 / secs, store.lock().unwrap().stats.summary());
    assert_eq!(chunks.len(), nchunks.len());
    drop(nchunks);
    profile_native_load(&native_dir, &region, &positions);

    // Saving every chunk into an empty world, as a full autosave would.
    let anvil_out = work.join("save-anvil");
    let mut a = AnvilSource::new(&anvil_out);
    let secs = save_all(&mut a, &chunks);
    println!("save Anvil:  {:.0} chunks/s, {}", chunks.len() as f64 / secs, mib(dir_size(&anvil_out)));
    for sync in [true, false] {
        let native_out = work.join(format!("save-native-{sync}"));
        // Seed the dictionary as the converted world has it.
        std::fs::create_dir_all(&native_out).unwrap();
        for e in std::fs::read_dir(&native_dir).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("dict.") {
                std::fs::copy(e.path(), native_out.join(&n)).unwrap();
            }
        }
        let store = NativeStore::shared(&native_out);
        store.lock().unwrap().sync = sync;
        let mut n = NativeSource::new(store);
        let secs = save_all(&mut n, &chunks);
        println!("save native (fsync {sync}): {:.0} chunks/s, {}", chunks.len() as f64 / secs, mib(dir_size(&native_out)));
    }
    // An incremental autosave: a tenth of the chunks again, appended to the cell files.
    let store = NativeStore::shared(work.join("save-native-true"));
    let mut n = NativeSource::new(store);
    let some: Vec<_> = chunks.iter().step_by(10).map(|(p, c)| (*p, c)).collect();
    let start = Instant::now();
    for (p, c) in &some {
        n.save(*p, c);
    }
    n.flush().unwrap();
    let secs = start.elapsed().as_secs_f64();
    println!("resave 10% native: {:.0} chunks/s, {}", some.len() as f64 / secs, mib(dir_size(&work.join("save-native-true"))));
    let mut a = AnvilSource::new(&anvil_out);
    let start = Instant::now();
    for (p, c) in &some {
        a.save(*p, c);
    }
    a.flush().unwrap();
    let secs = start.elapsed().as_secs_f64();
    println!("resave 10% Anvil:  {:.0} chunks/s, {}", some.len() as f64 / secs, mib(dir_size(&anvil_out)));
}

/// Where a load's time goes: reading and decompressing, parsing, building the chunk.
fn profile_native_load(native_dir: &Path, region: &Path, positions: &[ChunkPos]) {
    use kiln_storage::native::chunk::NativeChunk;
    let mut store = NativeStore::open(native_dir);
    let mut codec = AnvilSource::new(PathBuf::new());
    let (mut read, mut parse, mut build, mut n) = (0.0, 0.0, 0.0, 0);
    for &p in positions {
        let t = Instant::now();
        let Some((_, raw, _)) = store.read(kiln_storage::native::cellfile::CHUNK, p) else { continue };
        let t1 = Instant::now();
        let c = NativeChunk::decode(&raw).unwrap();
        let t2 = Instant::now();
        if c.into_chunk(&mut codec, OVERWORLD).is_ok() {
            n += 1;
        }
        let t3 = Instant::now();
        read += (t1 - t).as_secs_f64();
        parse += (t2 - t1).as_secs_f64();
        build += (t3 - t2).as_secs_f64();
    }
    let k = positions.len() as f64 / 1e6;
    println!("native per chunk: read+unzstd {:.0} µs, decode {:.0} µs, build {:.0} µs ({n} full)", read / k, parse / k, build / k);
    let mut regions: HashMap<(i32, i32), kiln_storage::region::RegionFile> = HashMap::new();
    let (mut read, mut decode) = (0.0, 0.0);
    for &p in positions {
        let t = Instant::now();
        let f = regions
            .entry((p.x >> 5, p.z >> 5))
            .or_insert_with(|| kiln_storage::region::RegionFile::open(&region.join(format!("r.{}.{}.mca", p.x >> 5, p.z >> 5))).unwrap());
        let raw = f.read((p.x & 31) as usize, (p.z & 31) as usize).unwrap().unwrap();
        let t1 = Instant::now();
        let _ = codec.decode(&raw, OVERWORLD);
        let t2 = Instant::now();
        read += (t1 - t).as_secs_f64();
        decode += (t2 - t1).as_secs_f64();
    }
    println!("anvil per chunk: read+inflate {:.0} µs, decode+build {:.0} µs", read / k, decode / k);
}
