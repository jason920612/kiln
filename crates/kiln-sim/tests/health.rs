//! Fall damage, death by /kill with the inventory scattered, and respawning.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::ClientCommand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn joined() -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Faller", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode survival Faller".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    (sim, client)
}

#[test]
fn landing_after_a_long_fall_hurts() {
    let (mut sim, mut client) = joined();
    let ground = client.pos;
    // Up 10 blocks (a teleport so the move check allows it), then fall back down.
    assert!(sim.step([ToSim::Console(format!("tp Faller {} {} {}", ground[0], ground[1] + 10.0, ground[2]))]));
    for _ in 0..2 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    for i in 1..=10 {
        let y = ground[1] + 10.0 - i as f64;
        let on_ground = i == 10;
        assert!(sim.step([
            ToSim::Packet(1, PlayIn::Move { pos: Some([ground[0], y, ground[2]]), rot: None, on_ground }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    // 10 blocks minus the safe 3: 7 damage.
    assert_eq!(sim.health(1), Some((13.0, false)));
}

#[test]
fn killed_players_drop_their_items_and_respawn() {
    let (mut sim, mut client) = joined();
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    assert!(sim.step([ToSim::Console("gamemode creative Faller".into())]));
    let item = ItemStack { item: stone, count: 5, added: Vec::new(), removed: Vec::new() };
    assert!(sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) })]));
    assert!(sim.step([ToSim::Console("gamemode survival Faller".into()), ToSim::Console("kill Faller".into())]));
    assert_eq!(sim.health(1), Some((0.0, true)));
    assert!(sim.step([]));
    assert_eq!(sim.entities().len(), 1, "the stone is scattered");
    assert_eq!(sim.inventory(1).unwrap()[36], None);

    assert!(sim.step([ToSim::Packet(1, PlayIn::ClientCommand(ClientCommand::PerformRespawn))]));
    assert_eq!(sim.health(1), Some((20.0, false)));
    let mut inbox = Vec::new();
    client.tick(None, &mut inbox);
    assert!(sim.step(inbox));
}
