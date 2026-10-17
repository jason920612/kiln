//! An entity a console command summons ticks in the very next tick (wp72). Vanilla adds it to the level at once, between two ticks, so its first
//! tick is the next one; Kiln queued the spawn and made the entity at the end of that next tick, so everything summoned by a command or a
//! function was one tick behind (a real client side by side: an item summoned with `Motion:[0,0,0]` had y = start - 0.04 after one tick on vanilla
//! and the start on Kiln; falling blocks and items landed a tick later).

use kiln_link::ToSim;
use kiln_sim::{Sim, SimConfig};

fn y_of(sim: &Sim, kind: &str) -> f64 {
    sim.entities().iter().find(|(t, _)| *t == kind).expect("the entity").1[1]
}

#[test]
fn a_summoned_item_has_its_first_tick_in_the_next_tick() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    assert!(sim.step([ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("forceload add 0 0 15 15".into())]));
    for _ in 0..20 {
        assert!(sim.step([]));
    }
    assert!(sim.step([ToSim::Console("summon minecraft:item 0.5 100 0.5 {Item:{id:\"minecraft:stone\",count:1},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767s}".into())]));
    // gravity: delta.y = -0.04, the item moves by it: y = 99.96 after the first tick (the one that ran the command's tick start)
    let y = y_of(&sim, "minecraft:item");
    assert!((y - 99.96).abs() < 1e-9, "first tick done: y = {y}");
    assert!(sim.step([]));
    let y2 = y_of(&sim, "minecraft:item");
    // (vanilla: 201.96, 201.8808 for a falling block summoned at 202)
    assert!((y2 - 99.8808).abs() < 1e-6, "second tick: y = {y2}");
}
