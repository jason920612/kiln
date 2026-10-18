//! Natural spawning picks its chunks from the block-ticking ring (`ChunkMap.collectSpawningChunks`: `ChunkHolder.getTickingChunk()` of the chunks within 8 of a
//! player): one chunk further than entities tick. With a simulation distance of 3 vanilla attempts the chunks up to 4 chunks from the player (measured with the
//! `chunkAttempt` counters of the java agent on a simulation distance of 6: 15x15 chunks, not 13x13), none at 5. Needs the vanilla datapack; skips without it.

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
fn spawn_attempts_reach_one_chunk_beyond_the_simulation_distance() {
    let Some(pack) = datapack() else { return };
    let file = std::env::temp_dir().join(format!("kiln-spawn-ring-{}", std::process::id()));
    // SAFETY: nothing else in this test binary reads the environment concurrently.
    unsafe {
        std::env::set_var("KILN_SPAWN_COUNTERS", &file);
        std::env::set_var("KILN_SPAWN_BOX", "-300,0,-300,300,255,300");
    }
    let mut config = SimConfig::new(4, 8, None);
    config.simulation_distance = 3;
    config.noise = Some(NoiseConfig { seed: 72, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 2);
    let setup = ["gamemode creative Watcher", "difficulty normal", "time set 18000", "gamerule minecraft:advance_time false", "tp Watcher 8 100 8"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats);
    // (Until the generated chunks around are all in, then a stretch of attempts.)
    let mut waited = 0;
    while sim.chunk_backlog() > 0 && waited < 6000 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        std::thread::sleep(std::time::Duration::from_millis(2));
        waited += 1;
    }
    for _ in 0..400 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let counters = kiln_sim::spawn_counters();
    let attempts = |cx: i32, cz: i32| counters.get(&format!("chunkAttempt MONSTER {cx},{cz}")).copied().unwrap_or(0);
    assert!(attempts(0, 0) > 0, "the player's own chunk: {:?}", counters.keys().filter(|k| k.starts_with("chunkAttempt")).collect::<Vec<_>>());
    let ring = |d: i32| attempts(d, 0) + attempts(0, d) + attempts(-d, 0) + attempts(0, -d);
    assert!(attempts(3, 0) > 0 && attempts(0, -3) > 0, "within the simulation distance");
    assert!(attempts(4, 0) > 0 && attempts(0, 4) > 0 && attempts(-4, 0) > 0 && attempts(0, -4) > 0, "one chunk beyond it: {:?}", (0..=6).map(|d| (d, attempts(d, 0), attempts(0, d), attempts(-d, 0), attempts(0, -d))).collect::<Vec<_>>());
    assert_eq!(ring(5) + ring(6), 0, "two beyond it");
    let _ = std::fs::remove_file(file);
}
