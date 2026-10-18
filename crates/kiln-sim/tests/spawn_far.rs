//! `EntityType.canSpawnFarFromPlayer` (wp79): `isValidSpawnPostitionForType` turns a position down when the nearest player is farther than the category's
//! despawn distance (128), except for types that may spawn far from players: the creature and misc categories, pillagers and shulkers. Kiln applied the
//! distance to every type, so a player flying 190 blocks above the ground (an AFK player on a high platform, a nether roof farm 60 blocks above the roof
//! while the striders live in the lava sea 160 blocks below) never had an animal or a strider spawn. Needs the vanilla datapack; skips without it.

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

#[test]
fn animals_spawn_more_than_128_blocks_below_the_player() {
    let Some(pack) = datapack() else { return };
    let file = std::env::temp_dir().join(format!("kiln-spawn-far-{}", std::process::id()));
    // SAFETY: nothing else in this test binary reads the environment concurrently.
    unsafe { std::env::set_var("KILN_SPAWN_COUNTERS", &file) };
    let mut config = SimConfig::new(4, 6, None);
    config.noise = Some(NoiseConfig { seed: 72, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 2);
    // (Plains around 8, 8 in seed 72; the player hovers at 400, far above everything: more than 128 blocks from every position animals can have.)
    let setup = ["gamemode creative Watcher", "difficulty normal", "time set 6000", "gamerule minecraft:advance_time false", "tp Watcher 8 400 8"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats);
    for _ in 0..2500 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let counters = kiln_sim::spawn_counters();
    let creature_picks: u64 = counters.iter().filter(|(k, _)| matches!(k.as_str(), "pick cow" | "pick pig" | "pick sheep" | "pick chicken" | "pick horse" | "pick donkey" | "pick rabbit" | "pick fox" | "pick wolf" | "pick goat")).map(|(_, v)| *v).sum();
    let valid: u64 = counters.iter().filter(|(k, _)| k.starts_with("validPosition ") && k.ends_with("-> true") && !k.contains("IN-BOX")).map(|(_, v)| *v).sum();
    assert!(creature_picks > 0, "the creature category was never picked from: {counters:?}");
    assert!(valid > 0, "no spawn position was ever valid (animals picked {creature_picks} times): {counters:?}");
    let _ = std::fs::remove_file(file);
}
