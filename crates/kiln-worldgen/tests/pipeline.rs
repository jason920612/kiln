//! The generation pipeline gives the same chunks whatever the thread count and request order.
//! Needs the datapack (`cargo xtask data`); skips without it. The multi-threaded comparison is
//! slow and only runs with `KILN_PARITY=1`.

use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::{Datapack, Pipeline, Worldgen};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn world(seed: i64) -> Option<Arc<Worldgen>> {
    let work = match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    };
    let generated = work.join("generated");
    if !generated.join("reports/biome_parameters").is_dir() {
        eprintln!("skipping: need {} (cargo xtask data)", generated.display());
        return None;
    }
    let pack = Datapack::load(&generated).expect("load datapack");
    Some(Arc::new(Worldgen::overworld(&pack, seed).expect("worldgen")))
}

#[test]
fn chunks_do_not_depend_on_thread_count_or_order() {
    if std::env::var_os("KILN_PARITY").is_none_or(|v| v != "1") {
        eprintln!("skipping: set KILN_PARITY=1 to run it");
        return;
    }
    let Some(world) = world(4242) else { return };
    let targets: Vec<(i32, i32)> = (0..3).flat_map(|x| (0..3).map(move |z| (x - 20, z + 7))).collect();

    // Reference: one thread, row order.
    let single = Pipeline::new(world.clone());
    let mut gs = GenScratch::default();
    let reference: Vec<_> = targets.iter().map(|&(x, z)| single.full(&mut gs, x, z)).collect();

    // Several threads, each asking for the targets in a different order.
    let shared = Arc::new(Pipeline::new(world.clone()));
    let results: Vec<Vec<(i32, i32, Vec<u16>)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let (shared, targets) = (shared.clone(), targets.clone());
                s.spawn(move || {
                    let mut gs = GenScratch::default();
                    let mut mine: Vec<(i32, i32)> = targets.iter().copied().skip(t).step_by(4).collect();
                    mine.reverse();
                    mine.into_iter().map(|(x, z)| (x, z, shared.full(&mut gs, x, z).blocks)).collect()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut compared = 0;
    for (x, z, blocks) in results.into_iter().flatten() {
        let i = targets.iter().position(|&t| t == (x, z)).unwrap();
        assert!(reference[i].blocks == blocks, "chunk {x},{z} differs between runs");
        compared += 1;
    }
    assert_eq!(compared, targets.len());
    // Asking again recomputes the same chunk.
    let again = shared.full(&mut gs, targets[4].0, targets[4].1);
    assert!(again.blocks == reference[4].blocks);
    eprintln!("{compared} chunks identical; pipeline {:?}", shared.stats());
}
