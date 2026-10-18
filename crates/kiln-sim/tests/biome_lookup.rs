//! `Level.getBiome(pos)` is `BiomeManager.getBiome`: the stored biome of the quart nearest to the position after the seeded jitter of the voronoi zoom, not the stored
//! biome of the quart holding it (they differ along biome borders). `execute if biome` asks for it; Kiln answered with the stored one, which is also not what
//! natural spawning sees (it always used the zoomed lookup). The vectors are vanilla 26.3's answers (`tools/wp79/biome79.py`, `BIOME_VECTORS`): a grid
//! around (-64, 64) in the nether of seed 77, 676 points, 22 of them on a border where the two lookups differ. Needs the vanilla datapack and `KILN_BIOME_VECTORS`.

use kiln_sim::{NoiseConfig, Sim, SimConfig};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn datapack() -> Option<PathBuf> {
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let work = match std::env::var_os("KILN_WORK") {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
            };
            work.join("generated")
        }
    };
    dir.join("reports/biome_parameters").is_dir().then_some(dir)
}

#[test]
fn if_biome_answers_with_the_zoomed_lookup_of_vanilla() {
    let (Some(datapack), Some(vectors)) = (datapack(), std::env::var_os("KILN_BIOME_VECTORS")) else {
        eprintln!("skipped: no datapack or KILN_BIOME_VECTORS (tools/wp79/biome79.py)");
        return;
    };
    let rows: Vec<Value> = std::fs::read_to_string(vectors).unwrap().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect();
    let mut config = SimConfig::new(2, 4, None);
    config.noise = Some(NoiseConfig { seed: 77, datapack, threads: 4 });
    let mut sim = Sim::new(config);
    assert!(sim.step([]));
    sim.capture_console();
    sim.console_now("execute in minecraft:the_nether run forceload add -176 -48 48 176");
    sim.take_console();
    let mut wrong = Vec::new();
    for row in &rows {
        let (x, y, z) = (row["x"].as_i64().unwrap(), row["y"].as_i64().unwrap(), row["z"].as_i64().unwrap());
        let Some(biome) = row["vanilla"].as_str() else { continue };
        sim.console_now(&format!("execute in minecraft:the_nether if biome {x} {y} {z} minecraft:{biome}"));
        let said = sim.take_console().join("\n");
        if !said.contains("conditional.pass") {
            wrong.push(format!("{x} {y} {z}: vanilla says {biome}, Kiln: {said}"));
        }
    }
    let on_border = rows.iter().filter(|r| r["vanilla"] != r["stored"]).count();
    assert!(on_border > 0, "the vectors have points where the zoomed and the stored lookup differ");
    assert!(wrong.is_empty(), "{} of {} points differ from vanilla: {:?}", wrong.len(), rows.len(), &wrong[..wrong.len().min(8)]);
}
