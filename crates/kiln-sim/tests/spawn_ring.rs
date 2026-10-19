//! Natural spawning picks its chunks from those whose status is block ticking or better (`ChunkMap.collectSpawningChunks`: `ChunkHolder.getTickingChunk()`, which
//! follows the loading tickets, so within the *server's view distance* of a player, not its simulation distance), tries them when entities tick there
//! (`ServerLevel.canSpawnEntitiesInChunk`: the view distance) and a player is within 128 blocks of the chunk's centre. wp79 took the ring one chunk beyond the simulation
//! distance for it (13x13 -> 15x15); lock step on the nether roof with a simulation distance of 6 and a view distance of 8 (`tools/wp81/farm81.py nether`) showed vanilla trying
//! 207.6 chunks a tick for a category with no cap, the whole circle of 128 blocks, Kiln 195.9 (the ring at 7). With a simulation distance of 3 and a view distance of 8 the chunks
//! up to 8 chunks from the player are tried, none at 9. Needs the vanilla datapack; skips without it.

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
fn spawn_attempts_reach_the_view_distance_not_the_simulation_distance() {
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
    let around = |d: i32| (d, attempts(d, 0), attempts(0, d), attempts(-d, 0), attempts(0, -d));
    for d in [4, 5, 6] {
        assert!(attempts(d, 0) > 0 && attempts(0, d) > 0 && attempts(-d, 0) > 0 && attempts(0, -d) > 0, "beyond the simulation distance, within the view distance: {:?}", around(d));
    }
    assert_eq!(ring(9), 0, "beyond the view distance: {:?}", around(9));
    let _ = std::fs::remove_file(file);
}
