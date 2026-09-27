//! Throughput of the generation pipeline through FULL (terrain, structure starts, features).
//!
//! usage: cargo run --release -p kiln-worldgen --example fullbench [-- <side> <threads>... [--no-structures]]
//! Each run generates a fresh `side`×`side` square of FULL chunks (default 16×16) with a new
//! pipeline shared by the given thread counts (default 1 and all cores), threads taking chunks
//! in row order from a shared counter. Datapack from `$KILN_WORK/generated`.

use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::{Datapack, Pipeline, Worldgen};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let structures = !args.iter().any(|a| a == "--no-structures");
    let nums: Vec<usize> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let side = nums.first().copied().unwrap_or(16) as i32;
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let threads: Vec<usize> = if nums.len() > 1 { nums[1..].to_vec() } else { vec![1, cores] };
    let dir = PathBuf::from(std::env::var_os("KILN_WORK").expect("set KILN_WORK")).join("generated");
    let pack = Datapack::load(&dir).expect("load datapack");
    let start = Instant::now();
    let world = Arc::new(Worldgen::overworld(&pack, 12345, structures).expect("worldgen"));
    println!("worldgen setup {:.0} ms; structures {structures}", start.elapsed().as_secs_f64() * 1e3);
    for (run, &t) in threads.iter().enumerate() {
        // A different square per run, so no run reuses another's work.
        let origin = (run as i32 * 1000, 500);
        let targets: Vec<(i32, i32)> = (0..side * side).map(|i| (origin.0 + i % side, origin.1 + i / side)).collect();
        let pipeline = Arc::new(Pipeline::new(world.clone()));
        let next = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..t {
                let (pipeline, next, targets) = (pipeline.clone(), next.clone(), &targets);
                s.spawn(move || {
                    let mut gs = GenScratch::default();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(x, z)) = targets.get(i) else { break };
                        std::hint::black_box(pipeline.full(&mut gs, x, z));
                    }
                });
            }
        });
        let secs = start.elapsed().as_secs_f64();
        let stats = pipeline.stats();
        println!(
            "{t:>3} threads: {} FULL chunks in {secs:.2} s = {:.1} chunks/s ({:.1} ms/chunk per thread); {} decorated, {} held",
            targets.len(),
            targets.len() as f64 / secs,
            secs * 1e3 * t as f64 / targets.len() as f64,
            stats.decorated,
            stats.held
        );
    }
    let gaps = world.decorator.features.gaps();
    println!("unimplemented feature types hit (skipped): {:?}", gaps.keys().collect::<Vec<_>>());
    println!("unimplemented structure types hit (skipped): {:?}", world.structures.gaps().keys().collect::<Vec<_>>());
}
