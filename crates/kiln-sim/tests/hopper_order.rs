//! Block entities tick in the order they were made (vanilla's `Level.blockEntityTickers`), so
//! a stack of hoppers moves its items differently when it was built top first or bottom first.
//! The expected rows are what vanilla 26.3 did with the same stack
//! (`ContainerVectors` `hopper_stack_made_top_first` / `hopper_stack_made_bottom_first`): per
//! tick, the upper hopper's transfer cooldown and logs, the lower hopper's, the logs in the
//! chest below and in the chest above.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const BASE: [i32; 3] = [0, 100, 0];

fn logs(sim: &Sim, y: i32) -> i32 {
    sim.container_at([BASE[0], BASE[1] + y, BASE[2]])
        .map_or(0, |(items, _)| items.iter().filter(|(_, n, _)| *n == "minecraft:oak_log").map(|(_, _, c)| *c).sum())
}

fn cooldown(sim: &Sim, y: i32) -> i32 {
    sim.block_entity_nbt(BASE[0], BASE[1] + y, BASE[2]).and_then(|t| t.get("TransferCooldown").and_then(|v| v.as_i64())).unwrap_or(0) as i32
}

/// Builds the stack in the order of `heights` (all in the first tick, as the vanilla vectors do) and returns per tick
/// (upper hopper cooldown, its logs, lower hopper cooldown, its logs, logs in the chest below, logs in the chest above).
fn run(heights: [i32; 4], ticks: usize) -> Vec<(i32, i32, i32, i32, i32, i32)> {
    let mut sim = Sim::new(SimConfig::new(2, 3, None));
    let (msg, stats) = join(1, "Hoppers", 3);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator Hoppers".into()), ToSim::Console("difficulty easy".into()), ToSim::Console(format!("tp Hoppers {} {} {}", BASE[0], BASE[1] + 8, BASE[2]))]));
    let mut client = Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let block = |y: i32| match y {
        3 => "minecraft:chest{Items:[{Slot:0b,id:\"minecraft:oak_log\",count:5},{Slot:13b,id:\"minecraft:dirt\",count:2}]}",
        2 => "minecraft:hopper[facing=down]{Items:[{Slot:0b,id:\"minecraft:oak_log\",count:3}]}",
        1 => "minecraft:hopper[facing=down]",
        _ => "minecraft:chest",
    };
    let mut rows = Vec::new();
    for t in 0..ticks {
        let mut inbox = Vec::new();
        if t == 0 {
            for y in heights {
                inbox.push(ToSim::Console(format!("setblock {} {} {} {}", BASE[0], BASE[1] + y, BASE[2], block(y))));
            }
        }
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        rows.push((cooldown(&sim, 2), logs(&sim, 2), cooldown(&sim, 1), logs(&sim, 1), logs(&sim, 0), logs(&sim, 3)));
    }
    rows
}

#[test]
fn a_stack_of_hoppers_made_top_first_ticks_top_first() {
    let want = [
        (8, 3, 7, 1, 0, 4), (7, 3, 6, 1, 0, 4), (6, 3, 5, 1, 0, 4), (5, 3, 4, 1, 0, 4), (4, 3, 3, 1, 0, 4), (3, 3, 2, 1, 0, 4),
        (2, 3, 1, 1, 0, 4), (1, 2, 8, 1, 1, 4), (8, 2, 7, 2, 1, 3), (7, 2, 6, 2, 1, 3), (6, 2, 5, 2, 1, 3), (5, 2, 4, 2, 1, 3),
        (4, 2, 3, 2, 1, 3), (3, 2, 2, 2, 1, 3), (2, 2, 1, 2, 1, 3), (1, 1, 8, 2, 2, 3),
    ];
    assert_eq!(run([3, 2, 1, 0], want.len()), want);
}

#[test]
fn a_stack_of_hoppers_made_bottom_first_ticks_bottom_first() {
    let want = [
        (8, 2, 8, 2, 0, 4), (7, 2, 7, 2, 0, 4), (6, 2, 6, 2, 0, 4), (5, 2, 5, 2, 0, 4), (4, 2, 4, 2, 0, 4), (3, 2, 3, 2, 0, 4),
        (2, 2, 2, 2, 0, 4), (1, 2, 1, 2, 0, 4), (8, 1, 8, 3, 1, 3), (7, 1, 7, 3, 1, 3), (6, 1, 6, 3, 1, 3), (5, 1, 5, 3, 1, 3),
        (4, 1, 4, 3, 1, 3), (3, 1, 3, 3, 1, 3), (2, 1, 2, 3, 1, 3), (1, 1, 1, 3, 1, 3),
    ];
    assert_eq!(run([0, 1, 2, 3], want.len()), want);
}
