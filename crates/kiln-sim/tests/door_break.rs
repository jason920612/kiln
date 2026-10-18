//! `BreakDoorGoal` (wp77): a zombie that can break doors, held back by a wooden door in a glass wall between it and a player, hacks the door away
//! in 240 ticks on hard difficulty and leaves it alone on normal; a vindicator in a raid opens doors (`RaiderOpenDoorGoal`) instead of waiting
//! in front of them. Kiln's zombies walked into closed doors and stayed there.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn door_after(difficulty: &str, ticks_to_run: usize) -> (bool, Vec<u16>) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Victim", 2);
    assert!(sim.step([
        msg,
        ToSim::Console("gamemode survival Victim".into()),
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console(format!("difficulty {difficulty}")),
    ]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, &mut client, 5);
    let p = client.pos;
    let [x, y, z] = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let run = |sim: &mut Sim, c: String| assert!(sim.step([ToSim::Console(c)]));
    // A glass wall across the way (eyes can see through it), a door in it.
    run(&mut sim, format!("fill {} {} {} {} {} {} minecraft:glass", x + 3, y, z - 6, x + 3, y + 3, z + 6));
    run(&mut sim, format!("setblock {} {y} {z} minecraft:oak_door[facing=east,half=lower,open=false]", x + 3));
    run(&mut sim, format!("setblock {} {} {z} minecraft:oak_door[facing=east,half=upper,open=false]", x + 3, y + 1));
    run(&mut sim, format!("summon minecraft:zombie {} {y} {} {{CanBreakDoors:1b,PersistenceRequired:1b}}", x + 6, z as f64 + 0.5));
    tick(&mut sim, &mut client, ticks_to_run);
    let door = sim.block_at(x + 3, y, z).expect("loaded");
    let air = kiln_data::blocks_types::block_by_name("minecraft:air").map(|b| b.first).expect("air");
    let states = (y..y + 2).map(|yy| sim.block_at(x + 3, yy, z).unwrap()).collect();
    (door == air, states)
}

#[test]
fn a_zombie_breaks_a_door_on_hard_only() {
    let (broken, states) = door_after("hard", 900);
    assert!(broken, "the door is gone on hard: {states:?}");
    let (broken, _) = door_after("normal", 900);
    assert!(!broken, "and stays on normal");
}
