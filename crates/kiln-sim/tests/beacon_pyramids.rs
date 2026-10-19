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
    sim.block_entity_data("minecraft:overworld", bx, by, bz).and_then(|be| be.get("Levels").and_then(kiln_proto::nbt::Tag::as_i64)).map_or(-1, |v| v as i32)
}

#[test]
fn a_beacon_reads_its_pyramid_whatever_its_height_and_wherever_it_stands() {
    assert_eq!(levels_of_pyramid(1, 6, false, None), 1);
    assert_eq!(levels_of_pyramid(4, 6, false, None), 4, "four levels floating in the air");
    assert_eq!(levels_of_pyramid(4, 6, true, None), 4, "four levels across chunk borders");
    assert_eq!(levels_of_pyramid(4, 6, false, Some((2, -3, 0))), 2, "a stone in the third layer");
    assert_eq!(levels_of_pyramid(3, 6, false, Some((0, 3, 0))), 0, "a stone in the beam");
}

#[test]
fn data_get_block_shows_the_levels_a_beacon_worked_out() {
    use kiln_proto::nbt::Tag;
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
    let (bx, by, bz) = (p[0].floor() as i32 + 3, p[1].floor() as i32 + 6, p[2].floor() as i32 + 3);
    for c in [
        format!("fill {} {} {} {} {} {} minecraft:air", bx - 8, by - 6, bz - 8, bx + 8, by + 6, bz + 8),
        format!("fill {} {} {} {} {} {} minecraft:iron_block", bx - 1, by - 1, bz - 1, bx + 1, by - 1, bz + 1),
        format!("fill {} {} {} {} {} {} minecraft:iron_block", bx - 2, by - 2, bz - 2, bx + 2, by - 2, bz + 2),
        format!("setblock {bx} {by} {bz} minecraft:beacon"),
    ] {
        assert!(sim.step([ToSim::Console(c)]));
    }
    for _ in 0..200 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let be = sim.block_entity_data("minecraft:overworld", bx, by, bz).expect("a block entity");
    assert_eq!(be.get("Levels").and_then(Tag::as_i64), Some(2), "the two layers the beacon counted: {be:?}");
}

#[test]
fn data_get_block_of_a_crafter_placed_by_a_command_shows_its_saved_fields() {
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
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32 + 3, p[2].floor() as i32 + 3);
    assert!(sim.step([ToSim::Console(format!("setblock {x} {y} {z} minecraft:crafter"))]));
    for _ in 0..30 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let be = sim.block_entity_data("minecraft:overworld", x, y, z).expect("a block entity");
    for key in ["crafting_ticks_remaining", "triggered", "disabled_slots", "Items"] {
        assert!(be.get(key).is_some(), "{key} in {be:?}");
    }
}

#[test]
fn a_data_merge_keeps_the_levels_and_the_beam_a_beacon_worked_out() {
    use kiln_proto::nbt::Tag;
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, 5);
    let (bx, by, bz) = (7, 100, 7);
    for c in [
        format!("fill {} {} {} {} {} {} minecraft:air", bx - 8, by - 6, bz - 8, bx + 8, by + 6, bz + 8),
        format!("fill {} {} {} {} {} {} minecraft:iron_block", bx - 1, by - 1, bz - 1, bx + 1, by - 1, bz + 1),
        format!("setblock {bx} {by} {bz} minecraft:beacon"),
    ] {
        assert!(sim.step([ToSim::Console(c)]));
    }
    tick(&mut sim, 200);
    assert!(sim.step([ToSim::Console(format!("data merge block {bx} {by} {bz} {{primary_effect:\"minecraft:speed\"}}"))]));
    let be = sim.block_entity_data("minecraft:overworld", bx, by, bz).expect("a block entity");
    assert_eq!(be.get("Levels").and_then(Tag::as_i64), Some(1), "the levels survive the merge: {be:?}");
    assert!(be.get("primary_effect").is_some());
}
