//! wp83: every loot table id the Rust sources name (structure chests, dispensers, gifts, shearing, equipment...) must be a table of the vanilla data. A
//! misspelt (or since renamed) id rolls nothing, and nothing says so: the jungle temple's dispensers used `dispensers/jungle_temple`, which 26.3 calls
//! `chests/jungle_temple_dispenser`. Needs the vanilla datapack (`KILN_DATAPACK`, else `work/generated`); skips without it.

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // (The generated data tables hold registry names, not loot tables.)
            if path.file_name().is_some_and(|n| n == "gen" || n == "target") {
                continue;
            }
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The `"minecraft:<dir>/<path>"` literals of `text` for the loot table directories.
fn table_literals(text: &str) -> Vec<String> {
    const DIRS: [&str; 13] =
        ["chests", "dispensers", "archaeology", "barrels", "pots", "harvest", "carve", "till", "shearing", "spawners", "equipment", "brush", "charged_creeper"];
    let mut out = Vec::new();
    for quoted in text.split('"').skip(1).step_by(2) {
        let Some(path) = quoted.strip_prefix("minecraft:") else { continue };
        let Some((dir, rest)) = path.split_once('/') else { continue };
        if DIRS.contains(&dir) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '/') && !rest.ends_with('/') {
            out.push(path.to_owned());
        }
    }
    out
}

#[test]
fn every_loot_table_the_sources_name_exists() {
    let pack = std::env::var_os("KILN_DATAPACK").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/generated"), PathBuf::from);
    let tables = pack.join("data/minecraft/loot_table");
    if !tables.is_dir() {
        eprintln!("no vanilla datapack: skipped");
        return;
    }
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join(".."), &mut files);
    assert!(files.len() > 100, "found the sources: {}", files.len());
    let mut missing = Vec::new();
    for file in files {
        // (This file names tables only to explain.)
        if file.ends_with("tests/loot_table_names.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for id in table_literals(&text) {
            if !tables.join(format!("{id}.json")).is_file() {
                missing.push(format!("{} in {}", id, file.display()));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "loot tables named in the sources that the vanilla data does not have:\n{}", missing.join("\n"));
}
