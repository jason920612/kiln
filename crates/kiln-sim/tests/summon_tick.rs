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

#[test]
fn a_summoned_zombie_next_to_a_player_stays() {
    use kiln_sim::testing::{Client, join};
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Watcher", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("difficulty normal".into()), ToSim::Console("gamemode creative Watcher".into())]));
    let mut client = Client::new(1, stats);
    let tick = |sim: &mut Sim, client: &mut Client, extra: Vec<ToSim>| {
        let mut inbox = extra;
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    };
    for _ in 0..20 {
        tick(&mut sim, &mut client, Vec::new());
    }
    let p = client.pos;
    tick(&mut sim, &mut client, vec![ToSim::Console(format!("summon minecraft:zombie {} {} {}", p[0] + 4.0, p[1], p[2]))]);
    for _ in 0..60 {
        tick(&mut sim, &mut client, Vec::new());
    }
    assert_eq!(sim.entity_ids_of("minecraft:zombie").len(), 1, "{:?}", sim.entities());
}

#[test]
fn a_summoned_zombie_is_there_and_stays() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    assert!(sim.step([ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("forceload add 0 0 15 15".into()), ToSim::Console("difficulty normal".into())]));
    for _ in 0..20 {
        assert!(sim.step([]));
    }
    assert!(sim.step([ToSim::Console("summon minecraft:zombie 8.5 -60 8.5".into())]));
    for _ in 0..60 {
        assert!(sim.step([]));
    }
    assert_eq!(sim.entity_ids_of("minecraft:zombie").len(), 1, "{:?}", sim.entities());
}

/// A block that falls because of its scheduled tick (placed over air with `/setblock`, two ticks later) is an entity that already moved by
/// 0.04 when the tick ends: vanilla's entity tick list holds what block ticks add (real servers side by side: a falling sand at 202 was at
/// y = 201.96, motion -0.0392 after its first tick on vanilla, 202.0 and 0 on Kiln).
#[test]
fn a_falling_block_that_the_block_phase_made_has_its_first_tick_in_that_tick() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    assert!(sim.step([ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("forceload add 0 0 15 15".into())]));
    for _ in 0..20 {
        assert!(sim.step([]));
    }
    assert!(sim.step([ToSim::Console("setblock 8 -40 8 minecraft:sand".into())]));
    let mut first = None;
    for k in 1..=6 {
        if let Some((_, p)) = sim.entities().iter().find(|(t, _)| *t == "minecraft:falling_block") {
            first = Some((k, p[1]));
            break;
        }
        assert!(sim.step([]));
    }
    let (k, y) = first.expect("the sand fell");
    assert!(k <= 4, "the entity exists soon after the scheduled tick (2 ticks): tick {k}");
    assert!((y - (-40.04)).abs() < 1e-9, "the tick that made it also moved it: y = {y}");
}
