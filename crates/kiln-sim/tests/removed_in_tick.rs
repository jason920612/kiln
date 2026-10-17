//! An entity that another one discards after its own turn in the tick is gone at the end of that tick (vanilla removes at once). It used
//! to be seen as removed in its next turn only, a tick late, when the one that killed it came later in the list: a wind charge made after
//! the armor stand it hits.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn count(sim: &Sim, id: &str) -> usize {
    sim.entity_nbt().iter().filter(|t| t.get("id").and_then(|i| i.as_str()) == Some(id)).count()
}

#[test]
fn a_stand_the_charge_made_after_it_kills_is_gone_in_the_tick_of_the_hit() {
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
    let (y, z) = (p[1] + 2.0, p[2] + 8.0);
    // The stand is made first (so it ticks first) and stands farther east than the charge, which flies at it.
    let mut inbox = vec![
        ToSim::Console(format!("summon minecraft:armor_stand {} {y} {z} {{NoAI:1b,NoGravity:1b}}", p[0] + 4.5)),
        ToSim::Console(format!("summon minecraft:wind_charge {} {} {z} {{Motion:[1.0d,0.0d,0.0d]}}", p[0] + 0.5, y + 0.5)),
    ];
    client.tick(None, &mut inbox);
    assert!(sim.step(inbox));
    let mut hit = None;
    for tick in 0..10 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let (charges, stands) = (count(&sim, "minecraft:wind_charge"), count(&sim, "minecraft:armor_stand"));
        if charges == 0 && hit.is_none() {
            hit = Some(tick);
            assert_eq!(stands, 0, "the charge is gone in tick {tick} and took the stand with it");
        }
    }
    assert!(hit.is_some(), "the charge hit something");
}
