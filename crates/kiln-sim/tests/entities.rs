//! Items thrown by a player fall, lie on the ground and come back when picked up.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_dropped_item_falls_and_is_picked_up_again() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Thrower", 2);
    // No natural mobs (slimes spawn in the superflat world's slime chunks).
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    let item = ItemStack { item: stone, count: 3, added: Vec::new(), removed: Vec::new() };
    assert!(sim.step([
        ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) }),
        ToSim::Packet(1, PlayIn::PlayerAction { action: 5, pos: [0, 0, 0], face: 0, sequence: 0 }),
    ]));
    assert_eq!(sim.inventory(1).unwrap()[36], Some((stone, 2)));
    let entities = sim.entities();
    assert_eq!(entities.len(), 1, "{entities:?}");
    let (kind, start) = entities[0];
    assert_eq!(kind, "minecraft:item");
    let ground = client.pos[1];
    assert!(start[1] > ground + 1.0, "thrown from the eyes: {start:?}");

    // It falls onto the ground within a second.
    for _ in 0..20 {
        assert!(sim.step([]));
    }
    let (_, landed) = sim.entities()[0];
    assert!((landed[1] - ground).abs() < 1e-6, "rests on the surface at {ground}: {landed:?}");

    // The thrower walks over; after the 40-tick pickup delay it takes the item back.
    let mut inbox = Vec::new();
    client.tick(Some([landed[0], ground, landed[2] - 0.5]), &mut inbox);
    assert!(sim.step(inbox));
    for _ in 0..30 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    assert!(sim.entities().is_empty());
    assert_eq!(sim.inventory(1).unwrap()[36], Some((stone, 3)));
}
