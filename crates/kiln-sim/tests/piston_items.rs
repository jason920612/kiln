//! A moving piston pushes with the progress it had before the tick (`PistonMovingBlockEntity.tick` calls `moveCollidedEntities`
//! and only then sets the new progress), so an item standing on the extending head is lifted again in the tick the head arrives,
//! not stuck against the head's final shape. The rows are what vanilla 26.3 did with the same setup (`ContainerVectors`
//! `piston_items/45`).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const BASE: [i32; 3] = [0, 100, 0];

fn y_of_item(sim: &Sim) -> Option<f64> {
    sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item")).and_then(|t| match t.get("Pos") {
        Some(Tag::List(l)) => l.get(1).and_then(Tag::as_f64),
        _ => None,
    })
}

#[test]
fn an_item_on_an_extending_piston_head_is_pushed_in_the_tick_the_head_arrives() {
    let mut sim = Sim::new(SimConfig::new(2, 3, None));
    let (msg, stats) = join(1, "Pistons", 3);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator Pistons".into()), ToSim::Console(format!("tp Pistons {} {} {}", BASE[0], BASE[1] + 8, BASE[2]))]));
    let mut client = Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let mut ys = Vec::new();
    for tick in 1..=5 {
        let mut inbox = Vec::new();
        if tick == 1 {
            inbox.push(ToSim::Console(format!("setblock {} {} {} minecraft:sticky_piston[facing=up,extended=false]", BASE[0], BASE[1], BASE[2])));
            inbox.push(ToSim::Console(format!(
                "summon minecraft:item 0.19119657245048902 101.0759616132101735 0.5149571187546473 {{Item:{{id:\"minecraft:diamond\",count:1}},Motion:[0d,0d,0d],PickupDelay:32767s}}"
            )));
        }
        if tick == 3 {
            inbox.push(ToSim::Console(format!("setblock {} {} {} minecraft:redstone_block", BASE[0] + 1, BASE[1], BASE[2])));
        }
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        ys.push(y_of_item(&sim).expect("the item"));
    }
    // Falls onto the piston, is lifted by the head twice (0.51 each: the second push is the one the final shape used to stop), lands on the head.
    assert_eq!(ys, vec![101.03596161321016, 101.0, 101.51, 102.01, 102.0]);
}
