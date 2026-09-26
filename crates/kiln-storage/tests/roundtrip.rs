//! Load -> save -> load on a copy of the reference world (skipped if absent).

use kiln_data::blocks::default_state as block;
use kiln_storage::AnvilSource;
use kiln_world::{Blocks, ChunkPos, ChunkSource, OVERWORLD, Terrain, World};
use std::path::PathBuf;

/// `KILN_WORK` or `<workspace>/work`.
fn work_dir() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

fn copy_regions(tag: &str) -> Option<PathBuf> {
    let src = work_dir().join("vanilla-world/world/dimensions/minecraft/overworld/region");
    if !src.exists() {
        eprintln!("no reference world; run tools/gen_vanilla_world.py");
        return None;
    }
    let dst = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("roundtrip-{tag}"));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
    }
    Some(dst)
}

#[test]
fn unmodified_chunks_survive_save_and_reload() {
    let Some(dir) = copy_regions("unmodified") else { return };
    let mut a = AnvilSource::new(&dir);
    let positions: Vec<ChunkPos> = (-12..12).flat_map(|x| (-12..12).map(move |z| ChunkPos::new(x, z))).collect();
    let originals: Vec<_> = positions.iter().map(|&p| a.load(p, OVERWORLD).unwrap()).collect();
    for (p, c) in positions.iter().zip(&originals) {
        a.save(*p, c);
    }
    a.flush().unwrap();

    let mut b = AnvilSource::new(&dir);
    for (p, orig) in positions.iter().zip(&originals) {
        let back = b.load(*p, OVERWORLD).unwrap();
        for y in -64..320 {
            for x in 0..16 {
                for z in 0..16 {
                    assert_eq!(orig.get(x, y, z), back.get(x, y, z), "block at {p:?} {x},{y},{z}");
                }
            }
        }
        for li in 0..orig.sky_light().len() {
            assert_eq!(orig.sky_light()[li].to_bytes(), back.sky_light()[li].to_bytes(), "sky light {p:?} {li}");
            assert_eq!(orig.block_light()[li].to_bytes(), back.block_light()[li].to_bytes(), "block light {p:?} {li}");
        }
        let mut orig_sections = orig.sections.iter();
        for s in &back.sections {
            let o = orig_sections.next().unwrap();
            let biomes = |sec: &kiln_world::section::Section| {
                let mut b = bytes::BytesMut::new();
                sec.biomes.encode(&mut b, 100);
                b
            };
            assert_eq!(biomes(o), biomes(s), "biomes at {p:?}");
        }
    }
}

#[test]
fn edits_persist_through_the_world_api() {
    let Some(dir) = copy_regions("edits") else { return };
    let mut w = World::with_source(OVERWORLD, Box::new(AnvilSource::new(&dir)), Terrain::Void, 0, 67);
    w.set_block(96, 150, -32, block::GOLD_BLOCK);
    w.set_block(1000, 100, 1000, block::DIAMOND_BLOCK); // outside the generated area: a new chunk
    let saved = w.save().unwrap();
    assert!(saved >= 2);

    let mut w2 = World::with_source(OVERWORLD, Box::new(AnvilSource::new(&dir)), Terrain::Void, 0, 67);
    w2.load_chunk(ChunkPos::of_block(96, -32));
    w2.load_chunk(ChunkPos::of_block(1000, 1000));
    assert_eq!(w2.get_block(96, 150, -32), Some(block::GOLD_BLOCK));
    assert_eq!(w2.get_block(1000, 100, 1000), Some(block::DIAMOND_BLOCK));
}
