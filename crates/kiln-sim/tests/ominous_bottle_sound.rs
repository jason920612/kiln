//! wp84 (an ominous bottle drunk by a real client on both servers: vanilla sends `item.ominous_bottle.dispose` to the drinker, Kiln sent it to
//! nobody): `PlaySoundConsumeEffect.apply` is `level.playSound(null, ...)` - every player hears it, the one who drank included.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn the_drinker_of_an_ominous_bottle_hears_the_dispose_sound() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Drinker", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let log = stats.clone();
    assert!(sim.step([msg, ToSim::Console("gamemode creative Drinker".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let bottle = kiln_data::builtin_id("minecraft:item", "minecraft:ominous_bottle").unwrap();
    let stack = ItemStack { item: bottle, count: 1, added: Vec::new(), removed: Vec::new() };
    assert!(sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    assert!(sim.step([ToSim::Console("gamemode survival Drinker".into())]));
    assert!(sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: Hand::Main, sequence: 1, yaw: 0.0, pitch: 0.0 })]));
    for _ in 0..40 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    let dispose = kiln_data::builtin_id("minecraft:sound_event", "minecraft:item.ominous_bottle.dispose").expect("sound");
    let packets = log.log.lock().unwrap().clone().unwrap();
    let heard = packets
        .iter()
        .filter(|p| {
            let mut r = kiln_proto::codec::Reader::new(p);
            r.varint().ok() == Some(kiln_data::packets::play::clientbound::SOUND) && r.varint().ok() == Some(dispose + 1)
        })
        .count();
    assert_eq!(heard, 1, "the dispose sound once, to the drinker");
    assert_eq!(sim.inventory(1).unwrap()[36], None, "the bottle was drunk");
}
