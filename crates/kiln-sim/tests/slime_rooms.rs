//! Slimes look for room for a size-4 one before they spawn (wp72). `EntityType.getSpawnAABB` is the type's box times its
//! `spawnDimensionsScale` (4 for slimes and magma cubes): `NaturalSpawner.isValidSpawnPostion` asks `noCollision` of that 2.08-block
//! box, so a room two blocks high never gets a slime and one three high does. Kiln checked the unscaled 0.52 box: real servers side by
//! side on seed 72 (13 flat floors, 2 high, in nine slime chunks, 14400 ticks) had 22 to 28 slimes on Kiln and none on vanilla (a
//! java agent counted 32 `checkSlimeSpawnRules` successes there that the box then refused).
//! Needs the vanilla datapack (`KILN_DATAPACK`, else `$KILN_WORK/generated`, else `work/generated`) for the spawn tables; skips without it.

use kiln_entity::mob::kinds::slime::is_slime_chunk;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{NoiseConfig, Sim, SimConfig};
use std::path::{Path, PathBuf};

const SEED: i64 = 72;

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

/// Sealed rooms `height` blocks high at y 1.. in the slime chunks `chunks`, run for `ticks` ticks with a player far above them; the
/// slimes found in the rooms.
fn run(pack: PathBuf, height: i32, chunks: &[(i32, i32)], ticks: usize) -> usize {
    let mut config = SimConfig::new(100, 8, None);
    config.noise = Some(NoiseConfig { seed: SEED, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 2);
    let setup = ["gamerule minecraft:random_tick_speed 0", "gamemode creative Watcher", "difficulty normal", "time set 6000", "gamerule minecraft:advance_time false", "tp Watcher 8 120 8"];
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
    // wait for the chunks around the player
    tick(&mut sim, &mut client, 600);
    for &(cx, cz) in chunks {
        let (x0, z0) = (cx * 16, cz * 16);
        let cmds = [
            format!("fill {x0} -1 {z0} {} 6 {} minecraft:stone", x0 + 15, z0 + 15),
            format!("fill {} 1 {} {} {} {} minecraft:air", x0 + 1, z0 + 1, x0 + 14, height, z0 + 14),
            "kill @e[type=!player]".to_owned(),
        ];
        assert!(sim.step(cmds.map(ToSim::Console)));
    }
    let mut seen = 0;
    for _ in 0..ticks / 600 {
        tick(&mut sim, &mut client, 600);
        // the slimes of this stretch (not the ones that came from splitting: those are in the same rooms)
        seen += sim
            .entities()
            .iter()
            .filter(|(t, p)| *t == "minecraft:slime" && p[1] < 8.0 && chunks.iter().any(|&(cx, cz)| (p[0].floor() as i32).div_euclid(16) == cx && (p[2].floor() as i32).div_euclid(16) == cz))
            .count();
        assert!(sim.step([ToSim::Console("kill @e[type=minecraft:slime]".into())]));
    }
    seen
}

fn slime_chunks_near_the_player() -> Vec<(i32, i32)> {
    let mut found = Vec::new();
    for cx in -4i32..=4 {
        for cz in -4i32..=4 {
            if (cx.abs() >= 2 || cz.abs() >= 2) && is_slime_chunk(SEED, cx, cz) {
                found.push((cx, cz));
            }
        }
    }
    found
}

#[test]
fn a_room_too_low_for_a_size_4_slime_gets_no_slimes_and_a_higher_one_does() {
    let Some(pack) = datapack() else { return };
    let chunks = slime_chunks_near_the_player();
    assert!(chunks.len() >= 5, "slime chunks around the spawn: {chunks:?}");
    let low = run(pack.clone(), 2, &chunks, 12000);
    let high = run(pack, 3, &chunks, 12000);
    assert_eq!(low, 0, "two blocks of room are not enough for the 2.08 block spawn box");
    assert!(high > 0, "three blocks are: {high} slimes in {chunks:?}");
}
