//! Slimes look for room for a size-4 one before they spawn (wp72). `EntityType.getSpawnAABB` is the type's box times its
//! `spawnDimensionsScale` (4 for slimes and magma cubes): `NaturalSpawner.isValidSpawnPostion` asks `noCollision` of that 2.08-block
//! box, so a room two blocks high never gets a slime and one three high does. Kiln checked the unscaled 0.52 box: real servers side by
//! side on seed 72 (13 flat floors, 2 high, in nine slime chunks, 14400 ticks) had 22 to 28 slimes on Kiln and none on vanilla (a
//! java agent counted 32 `checkSlimeSpawnRules` successes there that the box then refused).

use kiln_entity::mob::kinds::slime::is_slime_chunk;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

/// Sealed rooms `height` blocks high on the flat ground of the slime chunks `chunks`, run for `ticks` ticks with a player a few chunks away;
/// the slimes found in each room.
fn run(height: i32, chunks: &[(i32, i32)], ticks: usize) -> Vec<usize> {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Watcher", 2);
    let setup = ["gamerule minecraft:random_tick_speed 0", "gamemode creative Watcher", "difficulty normal", "time set 6000", "gamerule minecraft:advance_time false"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, &mut client, 10);
    let ground = client.pos[1].floor() as i32 - 1;
    for &(cx, cz) in chunks {
        let (x0, z0) = (cx * 16, cz * 16);
        let cmds = [
            format!("forceload add {x0} {z0} {} {}", x0 + 15, z0 + 15),
            format!("fill {x0} {ground} {z0} {} {} {} minecraft:stone", x0 + 15, ground + height + 1, z0 + 15),
            format!("fill {} {} {} {} {} {} minecraft:air", x0 + 1, ground + 1, z0 + 1, x0 + 14, ground + height, z0 + 14),
        ];
        assert!(sim.step(cmds.map(ToSim::Console)));
    }
    tick(&mut sim, &mut client, ticks);
    chunks
        .iter()
        .map(|&(cx, cz)| {
            sim.entities().iter().filter(|(t, p)| *t == "minecraft:slime" && (p[0].floor() as i32).div_euclid(16) == cx && (p[2].floor() as i32).div_euclid(16) == cz).count()
        })
        .collect()
}

fn slime_chunks_near_the_player() -> Vec<(i32, i32)> {
    let mut found = Vec::new();
    for cx in -3..=3 {
        for cz in -3..=3 {
            if (cx.abs() >= 2 || cz.abs() >= 2) && is_slime_chunk(0, cx, cz) {
                found.push((cx, cz));
            }
        }
    }
    found
}

#[test]
fn a_room_too_low_for_a_size_4_slime_gets_no_slimes_and_a_higher_one_does() {
    let chunks = slime_chunks_near_the_player();
    assert!(chunks.len() >= 3, "slime chunks around the flat world's spawn: {chunks:?}");
    let low: usize = run(2, &chunks, 6000).iter().sum();
    let high: usize = run(3, &chunks, 6000).iter().sum();
    assert_eq!(low, 0, "two blocks of room are not enough for the 2.08 block spawn box");
    assert!(high > 0, "three blocks are: {high} slimes in {chunks:?}");
}
