//! Base heights and columns (`ChunkGenerator.getBaseHeight`/`getBaseColumn`, what structures
//! stand on) against vanilla, dumped by `tools/feature_vectors.py --regions 0 --heights N`.
//! Runs with `KILN_PARITY=1`; `KILN_HEIGHT_VECTORS` defaults to `<work>/wp4-features/heights`.

use kiln_worldgen::generator::Generator;
use kiln_worldgen::proto::Heightmap;
use kiln_worldgen::{Datapack, Scratch};
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn base_heights_match_vanilla() {
    if std::env::var_os("KILN_PARITY").is_none_or(|v| v != "1") {
        eprintln!("skipping: set KILN_PARITY=1 to run it");
        return;
    }
    let work = match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    };
    let dir = std::env::var_os("KILN_HEIGHT_VECTORS").map(PathBuf::from).unwrap_or_else(|| work.join("wp4-features/heights"));
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        eprintln!("skipping: no height vectors in {}", dir.display());
        return;
    }
    let pack = Datapack::load(&work.join("generated")).expect("datapack");
    let mut bad = 0;
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let seed: i64 = name.trim_start_matches("heights_").trim_end_matches(".bin").parse().unwrap();
        let generator = Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", seed).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"KWGH");
        let mut p = 4;
        let i32_at = |p: &mut usize| {
            let v = i32::from_le_bytes(bytes[*p..*p + 4].try_into().unwrap());
            *p += 4;
            v
        };
        let n = i32_at(&mut p);
        let mut s = Scratch::caching();
        let (mut columns_bad, mut heights_bad) = (0, 0);
        for _ in 0..n {
            let (x, z) = (i32_at(&mut p), i32_at(&mut p));
            let (surface, floor) = (i32_at(&mut p), i32_at(&mut p));
            let len = i16::from_le_bytes(bytes[p..p + 2].try_into().unwrap()) as usize;
            p += 2;
            let column: Vec<u16> = (0..len).map(|i| u16::from_le_bytes(bytes[p + 2 * i..p + 2 * i + 2].try_into().unwrap())).collect();
            p += 2 * len;
            let mine = generator.base_column(&mut s, x, z);
            if mine != column {
                columns_bad += 1;
                if columns_bad <= 3 {
                    let y = mine.iter().zip(&column).position(|(a, b)| a != b).unwrap();
                    eprintln!("  column {x},{z}: first difference at y {}", generator.min_y + y as i32);
                }
            }
            let (ms, mf) =
                (generator.base_height(&mut s, x, z, Heightmap::WorldSurfaceWg), generator.base_height(&mut s, x, z, Heightmap::OceanFloorWg));
            if (ms, mf) != (surface, floor) {
                heights_bad += 1;
                if heights_bad <= 3 {
                    eprintln!("  heights {x},{z}: vanilla {surface}/{floor}, kiln {ms}/{mf}");
                }
            }
        }
        eprintln!("{name}: {n} columns, {columns_bad} columns and {heights_bad} height pairs differ");
        bad += columns_bad + heights_bad;
    }
    assert_eq!(bad, 0);
}
