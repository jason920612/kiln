//! wp80: beacons light up with pyramids of every height, wherever they stand (found with a pyramid built by commands in the sky of a running server: vanilla's
//! beacon read 4 levels, Kiln's stayed at 0).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn levels_of_pyramid(levels: i32, up: i32, near_chunk_border: bool, blocker: Option<(i32, i32, i32)>) -> i32 {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        if dir.join("data/minecraft/recipe").is_dir() {
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let p = client.pos;
    let (bx, by, bz) = (p[0].floor() as i32 + 3, p[1].floor() as i32 + up, p[2].floor() as i32 + 3);
    // On a chunk border the pyramid's layers stand in more than one chunk.
    let bx = if near_chunk_border { (bx >> 4) * 16 + 1 } else { bx };
    let bz = if near_chunk_border { (bz >> 4) * 16 + 1 } else { bz };
    let run = |sim: &mut Sim, c: String| assert!(sim.step([ToSim::Console(c)]));
    run(&mut sim, format!("fill {} {} {} {} {} {} minecraft:air", bx - 8, by - 6, bz - 8, bx + 8, by + 6, bz + 8));
    for l in 1..=levels {
        run(&mut sim, format!("fill {} {} {} {} {} {} minecraft:iron_block", bx - l, by - l, bz - l, bx + l, by - l, bz + l));
    }
    if let Some((dx, dy, dz)) = blocker {
        run(&mut sim, format!("setblock {} {} {} minecraft:stone", bx + dx, by + dy, bz + dz));
    }
    run(&mut sim, format!("setblock {bx} {by} {bz} minecraft:beacon"));
    for _ in 0..200 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    sim.container_at([bx, by, bz]).map_or(-1, |(_, data)| data[0])
}

#[test]
fn a_beacon_reads_its_pyramid_whatever_its_height_and_wherever_it_stands() {
    assert_eq!(levels_of_pyramid(1, 6, false, None), 1);
    assert_eq!(levels_of_pyramid(4, 6, false, None), 4, "four levels floating in the air");
    assert_eq!(levels_of_pyramid(4, 6, true, None), 4, "four levels across chunk borders");
    assert_eq!(levels_of_pyramid(4, 6, false, Some((2, -3, 0))), 2, "a stone in the third layer");
    assert_eq!(levels_of_pyramid(3, 6, false, Some((0, 3, 0))), 0, "a stone in the beam");
}
