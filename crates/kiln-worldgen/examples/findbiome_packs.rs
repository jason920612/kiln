//! Finds the nearest columns (spiralling out by chunks) whose biome at a height matches a name, for a seed, in a dimension of the
//! data packs: `findbiome` for worlds with packs (wp85).
//!
//! usage: cargo run --release -p kiln-worldgen --example findbiome_packs -- <dimension> <biome> <seed> <radius> [pack dir ...]
//! (the vanilla data: `$KILN_WORK/generated`; the packs are directories holding `data/`)

use kiln_worldgen::generator::BiomeSourceKind;
use kiln_worldgen::stem::{StemGenerator, stems_of};
use kiln_worldgen::{Datapack, GenScratch, Generator};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let dimension = args.next().expect("dimension");
    let want = args.next().expect("biome name");
    let seed: i64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let radius: i32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(256);
    let mut roots = vec![PathBuf::from(std::env::var_os("KILN_WORK").expect("set KILN_WORK")).join("generated")];
    roots.extend(args.map(PathBuf::from));
    let mut pack = Datapack::load_stack(&roots).expect("load the packs");
    let stems = stems_of(&pack);
    let (_, json) = stems.iter().find(|(k, _)| *k == dimension).expect("a dimension");
    let stem = pack.parse_stem(&dimension, json).expect("stem");
    let StemGenerator::Noise { settings, source } = &stem.generator else { panic!("not a noise generator") };
    let kind: BiomeSourceKind = source.kind();
    let generator = Generator::for_dimension(&pack, settings, kind, &stem.dimension_type, seed).expect("generator");
    let mut gs = GenScratch::default();
    let id = generator.biomes.iter().position(|b| b.name == want).expect("a biome of the packs") as u16;
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
