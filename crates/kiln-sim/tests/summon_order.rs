//! Entities made by commands get their ids in the order the commands ran (vanilla numbers them as they are made). They used to be
//! numbered by where they stood, which gave an item at rest another phase of its every-fourth-tick move (`tickCount + id`).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn summoned_entities_get_their_ids_in_command_order() {
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
    // Each to the west of the one before: by position they would come in the opposite order.
    let xs = [p[0] + 3.0, p[0] + 2.0, p[0] + 1.0, p[0] - 1.0, p[0] - 2.0];
    let mut inbox: Vec<ToSim> = xs.iter().map(|x| ToSim::Console(format!("summon minecraft:item {x} {} {} {{Item:{{id:\"minecraft:stone\",count:1}},NoGravity:1b}}", p[1] + 2.0, p[2]))).collect();
    client.tick(None, &mut inbox);
    assert!(sim.step(inbox));
    let items: Vec<f64> = sim.entities().into_iter().filter(|(k, _)| *k == "minecraft:item").map(|(_, pos)| pos[0]).collect();
    assert_eq!(items.len(), xs.len());
    // (`entities()` lists by id.)
    for (got, want) in items.iter().zip(&xs) {
        assert!((got - want).abs() < 0.5, "ids in command order: {items:?} for {xs:?}");
    }
}
