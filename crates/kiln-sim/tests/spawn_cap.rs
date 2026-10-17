//! The natural spawner's mob cap counts the mobs of the whole level (wp77): `NaturalSpawner.createState` adds up every loaded entity, near a
//! player or not, and `canSpawnForCategoryGlobal` compares the sum with `max × spawnable chunks / 289`. wp74 saw it on a restart at simulation
//! distance 3: a hundred mobs in loaded chunks that did not tick filled the cap and vanilla spawned nothing. Kiln counted only the mobs inside the
//! 8-chunk squares around the players.
//! Needs the vanilla datapack (`KILN_DATAPACK`, else `$KILN_WORK/generated`, else `work/generated`) for the spawn tables; skips without it.

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
fn animals_in_far_loaded_chunks_fill_the_cap() {
    let Some(pack) = datapack() else { return };
    let file = std::env::temp_dir().join(format!("kiln-spawn-counters-{}", std::process::id()));
    // SAFETY: nothing else in this test binary reads the environment concurrently.
    unsafe { std::env::set_var("KILN_SPAWN_COUNTERS", &file) };
    let mut config = SimConfig::new(100, 8, None);
    config.noise = Some(NoiseConfig { seed: 72, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 2);
    let setup = ["gamemode creative Watcher", "difficulty normal", "time set 6000", "gamerule minecraft:advance_time false", "tp Watcher 8 120 8", "forceload add 200 8"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats);
    let tick = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    tick(&mut sim, &mut client, 400);
    // Ten cows on a stone floor in the forced chunk (12, 0): 12 chunks from the player, outside the squares of the spawn range, and
    // animals never despawn.
    let mut cmds = vec!["kill @e[type=!player]".to_owned(), "fill 192 99 0 207 99 15 minecraft:stone".to_owned(), "fill 192 100 0 207 104 15 minecraft:air".to_owned()];
    for i in 0..10 {
        cmds.push(format!("summon minecraft:cow {} 100 8 {{NoAI:1b}}", 193 + i));
    }
    assert!(sim.step(cmds.into_iter().map(ToSim::Console)));
    tick(&mut sim, &mut client, 300);
    let counters = kiln_sim::spawn_counters();
    let full = counters.iter().filter(|(k, _)| k.starts_with("global CREATURE chunks=289 count=") && k.ends_with("-> false")).collect::<Vec<_>>();
    assert!(
        full.iter().any(|(k, _)| {
            let count: i32 = k["global CREATURE chunks=289 count=".len()..].split(' ').next().unwrap().parse().unwrap();
            count >= 10
        }),
        "the cows 12 chunks away are not counted: {:?}",
        counters.keys().filter(|k| k.starts_with("global")).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_file(file);
}
