//! wp87: a superflat dimension of a data pack that asks for more than layers (`features`, `lakes`, `structure_overrides`) generates it in the running server: trees, grass
//! and flowers on its grass layer, as the biome decorates it. (Block by block against vanilla it is `kiln-worldgen`'s `wp87_flat`.) Needs the vanilla data; skipped without.
//!
//! The levels and registries are process-wide: every test takes `live::exclusive()`.

mod support;

use kiln_sim::{NoiseConfig, Sim, SimConfig};
use std::path::{Path, PathBuf};
use support::wp87_world;

fn vanilla_data() -> Option<PathBuf> {
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
fn a_flat_dimension_with_features_is_decorated() {
    let _x = kiln_data::live::exclusive();
    let Some(datapack) = vanilla_data() else { return };
    let dir = wp87_world("flatfeat", &["kiln87_flat"]);
    let mut config = SimConfig::new(4, 4, Some(dir.clone()));
    config.noise = Some(NoiseConfig { seed: 87, datapack, threads: 4 });
    let mut sim = Sim::new(config);
    assert!(sim.step([]));
    let level = "kiln87:flatfeat";
    sim.console_now(&format!("execute in {level} run forceload add 0 0 63 63"));
    let mut waited = 0;
    while (sim.chunk_backlog() > 0 || sim.block_in(level, 8, -5, 8).is_none()) && waited < 20000 {
        assert!(sim.step([]));
        std::thread::sleep(std::time::Duration::from_millis(1));
        waited += 1;
    }
    for _ in 0..10 {
        assert!(sim.step([]));
    }
    use kiln_data::blocks::default_state as d;
    let (mut grass, mut other_surface, mut logs, mut stone) = (0, 0, 0, 0);
    for x in 0..64 {
        for z in 0..64 {
            // The layers: bedrock at -64, stone to -5, dirt, grass at -2 (the last of 63 layers): the surface is -3 here.
            if sim.block_in(level, x, -64, z) != Some(d::BEDROCK) {
                stone += 1;
            }
            for y in -3..=-1 {
                match sim.block_in(level, x, y, z) {
                    Some(b) if kiln_data::blocks_types::block_of(b).name == "minecraft:short_grass" => grass += 1,
                    Some(b) if kiln_data::blocks_types::block_of(b).name == "minecraft:oak_log" => logs += 1,
                    Some(b) if b != d::AIR && y == -1 => other_surface += 1,
                    _ => {}
                }
            }
        }
    }
    eprintln!("short grass {grass}, oak logs {logs}, other blocks at the top {other_surface}");
    assert_eq!(stone, 0, "bedrock at the bottom");
    assert!(grass > 50, "the plains' vegetation grows on the grass layer: {grass}");
    assert!(logs > 0, "trees too: {logs}");
    drop(sim);
    kiln_data::levels::reset();
    let _ = std::fs::remove_dir_all(dir);
}
