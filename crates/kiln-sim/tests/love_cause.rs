//! wp80 (found feeding wheat to a cow in both servers and comparing the cow's saved data): `Animal.loveCause`, the player who fed it, is saved as `LoveCause`
//! (a UUID) and still credits `bred_animals` after the animal was loaded back.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_cow_fed_wheat_remembers_the_player_who_fed_it() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Farmer", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode survival Farmer".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, 5);
    let p = client.pos;
    assert!(sim.step([
        ToSim::Console("give Farmer minecraft:wheat 2".into()),
        ToSim::Console(format!("summon minecraft:cow {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] + 1.5, p[1], p[2])),
    ]));
    tick(&mut sim, 2);
    let id = sim.entity_ids_of("minecraft:cow")[0];
    assert!(sim.step([ToSim::Packet(1, PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false })]));
    tick(&mut sim, 2);
    let cow = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:cow")).expect("the cow");
    assert!(cow.get("InLove").and_then(Tag::as_i64).is_some_and(|v| v > 0), "in love: {cow:?}");
    let cause = cow.get("LoveCause");
    assert!(matches!(cause, Some(Tag::IntArray(v)) if v.len() == 4), "LoveCause is the feeder's UUID: {cow:?}");
}
