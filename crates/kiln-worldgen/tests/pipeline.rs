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
    Some(Arc::new(Worldgen::overworld(&pack, seed, true).expect("worldgen")))
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

/// Entity ids (`id` fields) of the generated entities of the chunks in the window.
fn generated_ids(world: &Arc<Worldgen>, x0: i32, x1: i32, z0: i32, z1: i32) -> Vec<String> {
    use kiln_proto::nbt::Tag;
    let pipeline = Pipeline::new(world.clone());
    let mut gs = GenScratch::default();
    let mut ids = Vec::new();
    for x in x0..=x1 {
        for z in z0..=z1 {
            for e in &pipeline.full(&mut gs, x, z).entities {
                if let Tag::Compound(fields) = e
                    && let Some((_, Tag::String(id))) = fields.iter().find(|(k, _)| k == "id")
                {
                    ids.push(id.clone());
                }
            }
        }
    }
    ids
}

/// wp59: the real client found a monument with no elder guardians: generation only drew their
/// random numbers. Seed 59 has a monument at (320, -224) (`/locate structure minecraft:monument`).
#[test]
fn an_ocean_monument_comes_with_its_three_elder_guardians() {
    let Some(world) = world(59) else { return };
    let ids = generated_ids(&world, 15, 25, -19, -9);
    let elders = ids.iter().filter(|i| *i == "minecraft:elder_guardian").count();
    assert_eq!(elders, 3, "two wing rooms and the penthouse: {ids:?}");
}

/// wp59: a woodland mansion has its evokers and vindicators (`Mage` and `Warrior` data markers;
/// the allay groups only come with some rooms); seed 59 has one at (368, -928).
#[test]
fn a_woodland_mansion_is_inhabited() {
    let Some(world) = world(59) else { return };
    let ids = generated_ids(&world, 16, 31, -66, -50);
    let count = |t: &str| ids.iter().filter(|i| *i == t).count();
    assert!(count("minecraft:evoker") >= 1, "{ids:?}");
    assert!(count("minecraft:vindicator") >= 1, "{ids:?}");
}
