//! Single-threaded chunk generation throughput per step (BIOMES, then TERRAIN's fill,
//! surface and carvers), comparable to `tools/chunk_vectors.py --bench`.
//!
//! usage: cargo run --release -p kiln-worldgen --example chunkgen [-- <chunks> [seed]]
//! (datapack: `$KILN_WORK/generated`)

use kiln_worldgen::generator::Step;
use kiln_worldgen::{Datapack, GenScratch, Generator, Scratch};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(256);
    let seed: i64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let dir = PathBuf::from(std::env::var_os("KILN_WORK").expect("set KILN_WORK")).join("generated");
    let start = Instant::now();
    let pack = Datapack::load(&dir).expect("load datapack");
    let generator = Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", seed).expect("generator");
    eprintln!("datapack + generator setup: {:.0} ms", start.elapsed().as_secs_f64() * 1e3);

    // The vanilla harness's first 8x8 regions (ChunkVectors.FIXED_REGIONS), so both
    // benchmarks generate the same chunks.
    const REGIONS: [(i32, i32); 13] = [
        (0, 0),
        (-8, -8),
        (100, -300),
        (-2000, 1500),
        (10000, 10000),
        (-62500, 62500),
        (1048572, -1048580),
        (1874930, 1874930),
        (-1874938, -1874938),
        (1874930, -1874938),
        (-1874938, 1874930),
        (123456, -654321),
        (-777777, 333333),
    ];
    let chunks: Vec<(i32, i32)> = (0..n.min(REGIONS.len() * 64))
        .map(|i| {
            let (rx, rz) = REGIONS[i / 64];
            (rx + (i % 8) as i32, rz + ((i / 8) % 8) as i32)
        })
        .collect();
    let n = chunks.len();
    let names = ["biomes", "fill (incl. aquifer)", "surface", "carvers"];
    let mut best = [Duration::MAX; 4];
    for _ in 0..3 {
        let mut t = [Duration::ZERO; 4];
        let mut s = Scratch::caching();
        let a = Instant::now();
        for &(x, z) in &chunks {
            std::hint::black_box(generator.chunk_biomes(&mut s, x, z));
        }
        t[0] = a.elapsed();
        let mut gs = GenScratch::default();
        // Neighbour biomes exist before a chunk's TERRAIN in vanilla too.
        for &(x, z) in &chunks {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    generator.new_chunk(&mut gs, x + dx, z + dz);
                }
            }
        }
        for &(x, z) in &chunks {
            let mut chunk = generator.new_chunk(&mut gs, x, z);
            let mut last = Instant::now();
            generator.run_steps(&mut gs, &mut chunk, &mut |step, _| {
                let now = Instant::now();
                let i = match step {
                    Step::Fill => 1,
                    Step::Surface => 2,
                    Step::Carvers => 3,
                };
                t[i] += now - last;
                last = now;
            });
            std::hint::black_box(&chunk);
        }
        for i in 0..4 {
            best[i] = best[i].min(t[i]);
        }
    }
    let mut total = 0.0;
    for (name, b) in names.iter().zip(best) {
        let ms = b.as_secs_f64() * 1e3 / n as f64;
        total += ms;
        println!("kiln {name:22} {ms:8.3} ms/chunk");
    }
    println!("kiln total                  {total:8.3} ms/chunk = {:.1} chunks/s single-threaded", 1000.0 / total);
}
