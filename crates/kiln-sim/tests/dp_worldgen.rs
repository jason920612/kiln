//! wp85: a world with worldgen data packs runs on what the packs define: the biomes' spawn lists, `/locate`, `/place`, the dimension's
//! biome source. (Block-by-block generation against vanilla is `kiln-worldgen`'s `packs` test and `wp85_vanilla`.) Needs the vanilla data
//! (`KILN_DATAPACK`, else `$KILN_WORK/generated`); skipped without.
//!
//! The levels and registries are process-wide: every test takes `live::exclusive()`.

mod support;

use kiln_sim::{NoiseConfig, Sim, SimConfig};
use std::path::{Path, PathBuf};
use support::wp85_world;

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

fn start(tag: &str, packs: &[&str], seed: i64) -> Option<(Sim, PathBuf)> {
    let datapack = vanilla_data()?;
    let dir = wp85_world(tag, packs);
    let mut config = SimConfig::new(4, 4, Some(dir.clone()));
    config.noise = Some(NoiseConfig { seed, datapack, threads: 4 });
    let mut sim = Sim::new(config);
    assert!(sim.step([]));
    Some((sim, dir))
}

#[test]
fn a_replaced_biome_spawns_what_the_pack_says() {
    let _x = kiln_data::live::exclusive();
    let Some((sim, dir)) = start("spawns", &["kiln85_desert", "kiln85_crystal"], 85) else { return };
    // The pack's desert has cows and zombies only; vanilla's has rabbits, camels, skeletons, husks...
    assert_eq!(sim.spawn_list("minecraft:desert", "creature"), ["minecraft:cow"]);
    assert_eq!(sim.spawn_list("minecraft:desert", "monster"), ["minecraft:zombie"]);
    // A biome of a pack is there for the natural spawner too; the plains are untouched.
    assert_eq!(sim.spawn_list("kiln85:crystal_flats", "creature"), ["minecraft:sheep"]);
    assert_eq!(sim.spawn_list("kiln85:crystal_flats", "monster"), ["minecraft:spider"]);
    assert!(sim.spawn_list("minecraft:plains", "creature").contains(&"minecraft:horse"));
    drop(sim);
    kiln_data::levels::reset();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn locate_and_place_use_the_structures_and_templates_of_the_packs() {
    let _x = kiln_data::live::exclusive();
    let Some((mut sim, dir)) = start("locate", &["kiln85_structure"], 85) else { return };
    // Seed 85 has huts around (-64, -50) (a plains/desert/forest/meadow start of the pack's structure set).
    let found = sim.locate_structure_near("minecraft:overworld", [-64, 70, -50], &["kiln85:hut"]);
    let (pos, id) = found.expect("a hut near the origin");
    assert_eq!(id, "kiln85:hut");
    assert!((pos[0] + 64).abs() < 100 && (pos[2] + 50).abs() < 100, "{pos:?}");
    // The structure and its template exist for the commands.
    sim.console_now("forceload add 0 0 15 15");
    sim.console_now("place template kiln85:hut 4 200 4");
    use kiln_data::blocks::default_state as d;
    assert_eq!(sim.block_at(4, 200, 4), Some(d::OAK_PLANKS), "the floor of the pack's template");
    assert_eq!(sim.block_at(4, 201, 4), Some(d::COBBLESTONE), "its wall");
    assert_eq!(sim.block_at(6, 201, 4), Some(d::AIR), "its doorway");
    drop(sim);
    kiln_data::levels::reset();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_overworld_uses_the_biome_list_of_the_pack() {
    let _x = kiln_data::live::exclusive();
    let Some((mut sim, dir)) = start("biomes", &["kiln85_crystal"], 85) else { return };
    sim.console_now("forceload add -128 -128 127 127");
    let crystal = kiln_data::synced_id("minecraft:worldgen/biome", "kiln85:crystal_flats").unwrap() as u16;
    let allowed: Vec<u16> = ["kiln85:crystal_flats", "minecraft:plains", "minecraft:desert", "minecraft:ocean"]
        .iter()
        .map(|b| kiln_data::synced_id("minecraft:worldgen/biome", b).unwrap() as u16)
        .collect();
    let (mut seen_crystal, mut others) = (false, 0);
    for x in (-128..128).step_by(4) {
        for z in (-128..128).step_by(4) {
            for y in (0..120).step_by(8) {
                let b = sim.biome_in("minecraft:overworld", x, y, z).expect("loaded");
                seen_crystal |= b == crystal;
                if !allowed.contains(&b) {
                    others += 1;
                }
            }
        }
    }
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for x in (-128..128).step_by(16) {
        for z in (-128..128).step_by(16) {
            let b = sim.biome_in("minecraft:overworld", x, 64, z).expect("loaded");
            *counts.entry(kiln_data::live::synced_name("minecraft:worldgen/biome", b as i32).unwrap()).or_default() += 1;
        }
    }
    assert!(seen_crystal, "the pack's biome is in the world: {counts:?}");
    assert_eq!(others, 0, "only the biomes of the pack's list are");
    drop(sim);
    kiln_data::levels::reset();
    let _ = std::fs::remove_dir_all(dir);
}
