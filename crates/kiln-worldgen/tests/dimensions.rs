//! Smoke test for the Nether and End pipelines: FULL chunks come out with the dimension's
//! height and blocks. Needs the datapack (`cargo xtask data`); skips without it.

use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::{Datapack, Pipeline, Worldgen};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn pack() -> Option<Datapack> {
    let work = match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    };
    let generated = work.join("generated");
    if !generated.join("reports/biome_parameters").is_dir() {
        eprintln!("skipping: need {} (cargo xtask data)", generated.display());
        return None;
    }
    Some(Datapack::load(&generated).expect("load datapack"))
}

fn count(blocks: &[u16], name: &str) -> usize {
    let b = kiln_data::blocks_types::block_by_name(name).unwrap();
    blocks.iter().filter(|&&s| (b.first..=b.last).contains(&s)).count()
}

#[test]
fn nether_chunks_have_netherrack_lava_and_bedrock() {
    let Some(pack) = pack() else { return };
    let world = Arc::new(Worldgen::nether(&pack, 12345, true).expect("nether"));
    assert_eq!((world.generator.min_y, world.generator.height), (0, 256));
    let pipeline = Pipeline::new(world);
    let mut gs = GenScratch::default();
    let c = pipeline.full(&mut gs, 0, 0);
    assert!(count(&c.blocks, "minecraft:netherrack") > 10_000);
    assert!(count(&c.blocks, "minecraft:bedrock") >= 256 * 2);
    assert!(count(&c.blocks, "minecraft:lava") > 0);
}

#[test]
fn end_chunks_have_the_main_island() {
    let Some(pack) = pack() else { return };
    let world = Arc::new(Worldgen::end(&pack, 12345, true).expect("end"));
    assert_eq!((world.generator.min_y, world.generator.height), (0, 256));
    assert_eq!(world.generator.possible_biomes().len(), 5);
    let pipeline = Pipeline::new(world);
    let mut gs = GenScratch::default();
    let c = pipeline.full(&mut gs, 0, 0);
    assert!(count(&c.blocks, "minecraft:end_stone") > 5_000);
    assert_eq!(count(&c.blocks, "minecraft:bedrock"), 0);
}
