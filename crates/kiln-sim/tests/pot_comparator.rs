//! wp84 (a decorated pot filled by real clicks beside a comparator, both servers): vanilla's `setTheItem` is `setChanged`, so the comparator follows the pot's
//! fullness with every item put in (1 -> 10 at 44 of 64); Kiln updated it for the first item only.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_comparator_follows_a_decorated_pot_as_it_fills() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Potter", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Potter".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let p = client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 2, p[1].floor() as i32, p[2].floor() as i32);
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    assert!(sim.step([
        ToSim::Console(format!("setblock {x} {y} {z} minecraft:decorated_pot[facing=north]")),
        ToSim::Console(format!("setblock {x} {y} {} minecraft:comparator[facing=south]", z - 1)),
    ]));
    // (creative keeps the stack at 64: 44 clicks put 44 stones in)
    let held = ItemStack { item: stone, count: 64, added: Vec::new(), removed: Vec::new() };
    assert!(sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(held) })]));
    for i in 0..44 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::UseItemOn { hand: 0, pos: [x, y, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: i + 1 })]));
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    for _ in 0..6 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    let pot = sim.block_entity_nbt(x, y, z).expect("the pot");
    let count = pot.get("item").and_then(|i| i.get("count")).and_then(Tag::as_i64).unwrap_or(0);
    assert!(count >= 40, "the pot took the stones: {pot:?}");
    let comparator = sim.block_entity_nbt(x, y, z - 1).expect("the comparator");
    let out = comparator.get("OutputSignal").and_then(Tag::as_i64).unwrap_or(-1);
    let want = 1 + (count as f64 / 64.0 * 14.0).floor() as i64;
    assert_eq!(out, want, "{count} of 64 stones read as {want}: {comparator:?}");
}
