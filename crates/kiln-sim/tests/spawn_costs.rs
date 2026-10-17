//! Spawn potentials (`spawn_costs`, wp72): a warped forest lets an enderman spawn only where the charges of the endermen already there add up to
//! no more than 0.12 (`NaturalSpawner.SpawnState.canSpawn`, `PotentialCalculator`); soul sand valleys do the same for skeletons, ghasts, endermen
//! and striders. Kiln did not simulate it: a real nether run side by side on seed 72 (nether wastes with warped and crimson forests around) filled
//! the monster cap with endermen (725 of 1268 mobs in sixteen 100-tick rounds against 189 of 1285 on vanilla).
//! Needs the vanilla datapack (`KILN_DATAPACK`, else `$KILN_WORK/generated`, else `work/generated`); skips without it.

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

/// The endermen the nether around (0, 50, -32) of seed 72 (a warped forest 30 blocks east and south of the player, nether wastes beside it) has
/// after 100 ticks, summed over `rounds` rounds that start from an empty nether.
fn endermen(pack: PathBuf, rounds: usize) -> usize {
    let mut config = SimConfig::new(100, 8, None);
    config.noise = Some(NoiseConfig { seed: 72, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 2);
    let setup = ["gamemode creative Watcher", "difficulty normal", "execute in minecraft:the_nether run tp Watcher -32 50 -32", "effect give Watcher minecraft:slow_falling 99999 0 true"];
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
    tick(&mut sim, &mut client, 900);
    let mut total = 0;
    for _ in 0..rounds {
        assert!(sim.step([ToSim::Console("kill @e[type=!player]".into())]));
        tick(&mut sim, &mut client, 100);
        total += sim.entity_ids_of("minecraft:enderman").len();
    }
    total
}

#[test]
fn the_endermen_of_a_warped_forest_are_spaced_by_their_spawn_cost() {
    let Some(pack) = datapack() else { return };
    let found = endermen(pack, 4);
    // vanilla: about 12 a round (189 in 16 rounds); without the costs about 45
    assert!(found < 120, "{found} endermen in 4 rounds of 100 ticks");
}
