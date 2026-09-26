//! Connection and chunk lifecycles across ticks: joins and leaves in the same batch, packets
//! sent right before leaving, and edits surviving a chunk's unload and reload.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn flat_sim() -> Sim {
    Sim::new(SimConfig::new(8, 4, None))
}

fn settle(sim: &mut Sim, clients: &mut [Client], ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        for c in clients.iter_mut() {
            c.tick(None, &mut inbox);
        }
        assert!(sim.step(inbox));
    }
}

#[test]
fn a_join_and_leave_in_the_same_batch_leave_no_player_behind() {
    let mut sim = flat_sim();
    let (msg, _stats) = join(1, "Quick", 2);
    assert!(sim.step([msg, ToSim::Leave(1)]));
    assert_eq!(sim.player_count(), 0);
    for _ in 0..30 {
        assert!(sim.step([]));
    }
    assert_eq!(sim.player_count(), 0);
}

#[test]
fn packets_sent_before_leaving_still_apply() {
    let mut sim = flat_sim();
    let (msg, stats) = join(1, "Builder", 2);
    assert!(sim.step([msg]));
    let mut clients = vec![Client::new(1, stats)];
    settle(&mut sim, &mut clients, 5);
    let [x, y, z] = clients[0].pos.map(|c| c.floor() as i32);
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    let item = ItemStack { item: stone, count: 1, added: Vec::new(), removed: Vec::new() };
    let place = PlayIn::UseItemOn { hand: 0, pos: [x + 2, y - 1, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
    assert!(sim.step([
        ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) }),
        ToSim::Packet(1, place),
        ToSim::Leave(1),
    ]));
    assert_eq!(sim.player_count(), 0);
    assert_eq!(sim.block_at(x + 2, y, z), Some(kiln_data::blocks::default_state::STONE));
}

#[test]
fn edits_survive_a_chunk_leaving_memory_and_coming_back() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lifecycle-unload");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.clone())));
    let (msg, stats) = join(1, "Traveller", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Traveller".into())]));
    let mut clients = vec![Client::new(1, stats)];
    settle(&mut sim, &mut clients, 5);
    // A block next to the player, in a chunk that is certainly loaded.
    let [x, y, z] = clients[0].pos.map(|c| c.floor() as i32);
    let target = [x + 1, y + 1, z];
    // Clicking air (replaceable) places into the clicked block itself.
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    let item = ItemStack { item: stone, count: 1, added: Vec::new(), removed: Vec::new() };
    let place = PlayIn::UseItemOn { hand: 0, pos: target, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
    assert!(sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) }), ToSim::Packet(1, place)]));
    assert_eq!(sim.block_at(target[0], target[1], target[2]), Some(kiln_data::blocks::default_state::STONE));
    // Far away: the chunk unloads (checks every 20 ticks), then back again.
    assert!(sim.step([ToSim::Console(format!("tp Traveller {} {} {}", x + 5000, y, z))]));
    settle(&mut sim, &mut clients, 60);
    assert_eq!(sim.block_at(target[0], target[1], target[2]), None, "the chunk should have unloaded");
    assert!(sim.step([ToSim::Console(format!("tp Traveller {} {} {}", x, y, z))]));
    settle(&mut sim, &mut clients, 5);
    assert_eq!(sim.block_at(target[0], target[1], target[2]), Some(kiln_data::blocks::default_state::STONE));
}
