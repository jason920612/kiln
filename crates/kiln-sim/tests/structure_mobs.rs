//! The mobs structure generation places (elder guardians, vindicators, witches...) exist in the running
//! simulation, not only in the generated chunk. Needs the vanilla datapack (`KILN_DATAPACK`, else
//! `$KILN_WORK/generated`, else `work/generated`); skips without it.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{NoiseConfig, Sim, SimConfig};
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

/// A player standing at `at`, waiting until the chunks around are loaded; the types and positions of the
/// entities within `radius` blocks.
fn visit(seed: i64, pack: PathBuf, at: [i32; 3], radius: f64, settle_ticks: usize, world: Option<&str>) -> Vec<(&'static str, [f64; 3])> {
    let dir = world.map(|name| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    });
    let mut config = SimConfig::new(100, 10, dir);
    config.noise = Some(NoiseConfig { seed, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Visitor", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Visitor".into()), ToSim::Console(format!("tp Visitor {} {} {}", at[0], at[1], at[2]))]));
    let mut client = Client::new(1, stats);
    for _ in 0..settle_ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    sim.entities()
        .into_iter()
        .filter(|(t, p)| *t != "minecraft:player" && ((p[0] - at[0] as f64).powi(2) + (p[2] - at[2] as f64).powi(2)).sqrt() < radius)
        .collect()
}

/// wp60: the real client found no elder guardian in any monument of seed 60, though generation made them
/// (`kiln-worldgen` `every_ocean_monument_of_seed_60_has_three_elder_guardians`).
#[test]
fn an_ocean_monument_in_the_running_world_has_its_elder_guardians() {
    let Some(pack) = datapack() else { return };
    // `/locate structure minecraft:monument` in seed 60: (-352, 160); the monument spans 58 blocks from there.
    let found = visit(60, pack, [-352 + 29, 80, 160 + 29], 70.0, 1500, Some("monument_world"));
    let elders = found.iter().filter(|(t, _)| *t == "minecraft:elder_guardian").count();
    let mut kinds = std::collections::BTreeMap::new();
    for (t, _) in &found {
        *kinds.entry(*t).or_insert(0) += 1;
    }
    assert!(elders >= 2, "elder guardians in the loaded monument: {kinds:?}");
}

/// How many elder guardians a visit finds with a given view distance (wp60 bisect).
#[test]
fn bisect_view_distance() {
    let Some(pack) = datapack() else { return };
    for view in [3u8, 4, 5, 6, 8, 10] {
        let mut config = SimConfig::new(100, view, None);
        config.noise = Some(NoiseConfig { seed: 60, datapack: pack.clone(), threads: 4 });
        let mut sim = Sim::new(config);
        let (msg, stats) = join(1, "Visitor", 2);
        let at = [-352 + 29, 80, 160 + 29];
        assert!(sim.step([msg, ToSim::Console("gamemode creative Visitor".into()), ToSim::Console(format!("tp Visitor {} {} {}", at[0], at[1], at[2]))]));
        let mut client = Client::new(1, stats);
        for _ in 0..1200 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let elders = sim.entities().into_iter().filter(|(t, _)| *t == "minecraft:elder_guardian").count();
        eprintln!("view {view}: {elders} elders, {} chunks", sim.loaded_chunks().iter().sum::<usize>());
    }
}
