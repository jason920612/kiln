//! Single-threaded throughput of the overworld noise router on the cell-corner grid.
//!
//! usage: cargo run --release -p kiln-worldgen --example bench [-- <datapack dir>]
//! (default datapack: `$KILN_WORK/generated`)

use kiln_worldgen::{Datapack, NoiseRouter, SamplerRef, Scratch, Volume};
use std::path::PathBuf;
use std::time::Instant;

const CHUNKS: i32 = 32;

fn corner_volume(cx: i32, cz: i32) -> Volume {
    Volume::new([5, 49, 5], [cx * 16, -64, cz * 16], [4, 8, 4])
}

fn volume_rate(samplers: &[&SamplerRef]) -> f64 {
    let mut scratch = Scratch::default();
    let mut buf = vec![0f32; 5 * 49 * 5];
    let mut best = 0f64;
    for _ in 0..3 {
        let start = Instant::now();
        let mut positions = 0usize;
        for cx in 0..CHUNKS {
            for cz in 0..CHUNKS {
                let vol = corner_volume(cx, cz);
                for s in samplers {
                    s.fill(&mut scratch, &vol, &mut buf);
                }
                positions += vol.len();
            }
        }
        best = best.max(positions as f64 / start.elapsed().as_secs_f64());
    }
    best
}

fn point_rate(sampler: &SamplerRef) -> f64 {
    let mut scratch = Scratch::default();
    let start = Instant::now();
    let mut positions = 0usize;
    let mut sink = 0f32;
    for cx in 0..CHUNKS / 4 {
        for cz in 0..CHUNKS / 4 {
            for [x, y, z] in corner_volume(cx, cz).positions() {
                sink += sampler.point(&mut scratch, x, y, z);
                positions += 1;
            }
        }
    }
    std::hint::black_box(sink);
    positions as f64 / start.elapsed().as_secs_f64()
}

fn main() {
    let dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("KILN_WORK").expect("pass a datapack dir or set KILN_WORK")).join("generated")
    });
    let start = Instant::now();
    let pack = Datapack::load(&dir).expect("load datapack");
    let loaded = start.elapsed();
    let start = Instant::now();
    let (_, router) = NoiseRouter::new(&pack, "minecraft:overworld", 0).expect("compile router");
    println!("load {:.0} ms, compile {:.0} ms", loaded.as_secs_f64() * 1e3, start.elapsed().as_secs_f64() * 1e3);

    let fields: Vec<&SamplerRef> =
        router.outputs.iter().filter(|(n, _)| !n.starts_with("aquifers/")).map(|(_, s)| s).collect();
    let final_density = router.get("final_density").unwrap();
    println!("final_density volume: {:>12.0} positions/s", volume_rate(&[final_density]));
    println!("full router volume:   {:>12.0} positions/s (all 8 fields per position)", volume_rate(&fields));
    println!("final_density point:  {:>12.0} positions/s", point_rate(final_density));
    for (name, s) in router.outputs.iter().filter(|(n, _)| !n.starts_with("aquifers/")) {
        println!("  {name:20} volume {:>12.0} positions/s", volume_rate(&[s]));
    }
}
