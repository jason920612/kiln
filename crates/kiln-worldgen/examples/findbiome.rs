//! Finds the nearest columns (spiralling out by chunks) whose surface-level biome matches a
//! name, for a seed: handy to send a player somewhere snowy.
//!
//! usage: cargo run --release -p kiln-worldgen --example findbiome -- <biome> [seed] [radius]
//! (datapack: `$KILN_WORK/generated`)

use kiln_worldgen::{Datapack, GenScratch, Generator};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let want = args.next().expect("biome name");
    let seed: i64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let radius: i32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(256);
    let dir = PathBuf::from(std::env::var_os("KILN_WORK").expect("set KILN_WORK")).join("generated");
    let pack = Datapack::load(&dir).expect("load datapack");
    let generator = Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", seed).expect("generator");
    let mut gs = GenScratch::default();
    let id = kiln_data::synced_id("minecraft:worldgen/biome", &want).expect("known biome") as u16;
    for r in 0..=radius {
        for cx in -r..=r {
            for cz in -r..=r {
                if cx.abs() != r && cz.abs() != r {
                    continue;
                }
                // The chunk's center quart at y 64.
                let (qx, qz) = (cx * 4 + 2, cz * 4 + 2);
                if gs.noise_biome(&generator, qx, 16, qz) == id {
                    println!("{want} at chunk {cx} {cz}: block {} {}", cx * 16 + 8, cz * 16 + 8);
                    return;
                }
            }
        }
    }
    println!("no {want} within {radius} chunks");
}
