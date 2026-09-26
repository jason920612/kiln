//! Loads the reference world made by tools/gen_vanilla_world.py (skipped if absent).

use kiln_data::blocks_types::block_of;
use kiln_storage::AnvilSource;
use kiln_world::{ChunkPos, ChunkSource, OVERWORLD};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// `KILN_WORK` or `<workspace>/work`.
fn work_dir() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

fn world_dir() -> Option<PathBuf> {
    let root = work_dir().join("vanilla-world/world");
    root.join("level.dat").exists().then_some(root)
}

#[test]
fn loads_every_generated_chunk() {
    let Some(dir) = world_dir() else {
        eprintln!("no reference world; run tools/gen_vanilla_world.py");
        return;
    };
    let mut src = AnvilSource::new(dir.join("dimensions/minecraft/overworld/region"));
    let mut loaded = 0;
    let mut blocks: BTreeMap<&str, u64> = BTreeMap::new();
    for x in -12..12 {
        for z in -12..12 {
            let chunk = src.load(ChunkPos::new(x, z), OVERWORLD).unwrap_or_else(|| panic!("chunk {x},{z} missing"));
            loaded += 1;
            for y in (-64..320).step_by(7) {
                *blocks.entry(block_of(chunk.get(3, y, 5)).name).or_default() += 1;
            }
        }
    }
    assert_eq!(loaded, 576);
    for expected in ["minecraft:bedrock", "minecraft:stone", "minecraft:deepslate", "minecraft:air"] {
        assert!(blocks.contains_key(expected), "no {expected} sampled: {blocks:?}");
    }
    let spawn = kiln_storage::read_spawn(&dir).expect("spawn in level.dat");
    assert_eq!(spawn, [96, 145, -32]);
    println!("{loaded} chunks, {} block types sampled", blocks.len());
}
