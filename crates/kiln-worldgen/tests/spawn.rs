//! The initial world spawn against the spawn vanilla 26.3 stored in `level.dat` for worlds it
//! created with these seeds (structures off). Needs the datapack; generates FULL chunks, so
//! it only runs with `KILN_PARITY=1`.

use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::spawn::{initial_spawn, spawn_pos_in_chunk};
use kiln_worldgen::{Datapack, Pipeline, Worldgen};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// (seed, vanilla spawn).
const VANILLA: [(i64, [i32; 3]); 6] = [
    (0, [-32, 69, 0]),
    (12345, [96, 145, -32]),
    (101, [-272, 71, -112]),
    (102, [0, 75, -64]),
    (201, [-32, 64, -48]),
    (301, [0, 94, 0]),
];

#[test]
fn initial_spawn_matches_vanilla() {
    if std::env::var_os("KILN_PARITY").is_none_or(|v| v != "1") {
        eprintln!("skipping: set KILN_PARITY=1 to run it");
        return;
    }
    let work = match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    };
    let generated = work.join("generated");
    if !generated.join("reports/biome_parameters").is_dir() {
        eprintln!("skipping: need {} (cargo xtask data)", generated.display());
        return;
    }
    let pack = Datapack::load(&generated).expect("load datapack");
    let (mut bad, mut exact_bad) = (0, 0);
    for (seed, vanilla) in VANILLA {
        let world = Arc::new(Worldgen::overworld(&pack, seed, false).expect("worldgen"));
        let pipeline = Pipeline::new(world.clone());
        let mut gs = GenScratch::default();
        let origin = world.generator.spawn_origin(&mut gs);
        let mut chunks = 0;
        let p = initial_spawn(origin, |cx, cz| {
            chunks += 1;
            spawn_pos_in_chunk(&pipeline.full(&mut gs, cx, cz))
        });
        let kiln = [p.x, p.y, p.z];
        let chunk_ok = (kiln[0] >> 4, kiln[2] >> 4) == (vanilla[0] >> 4, vanilla[2] >> 4);
        eprintln!("seed {seed}: origin chunk {origin:?}, {chunks} chunks searched, kiln {kiln:?}, vanilla {vanilla:?}");
        if !chunk_ok {
            bad += 1;
        } else if kiln != vanilla {
            exact_bad += 1;
            eprintln!("  same chunk, different column/height (vanilla decorated its spawn chunks in its own order, MC-55596)");
        }
    }
    eprintln!("{} seeds: spawn chunk differs in {bad}, exact position differs in {exact_bad} more", VANILLA.len());
    assert_eq!(bad, 0, "{bad} of {} spawn chunks differ", VANILLA.len());
}
