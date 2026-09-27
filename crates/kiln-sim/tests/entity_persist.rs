//! Entities are saved with their chunk: dropped items leave with a chunk that unloads and come
//! back with it, and survive a restart of the server on the same world, keeping their UUID,
//! stack and age.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;

fn settle(sim: &mut Sim, client: &mut Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

/// (UUID, item id, count, age) of every item entity.
fn items(sim: &Sim) -> Vec<(Tag, String, i64, i64)> {
    let mut out: Vec<_> = sim
        .entity_nbt()
        .into_iter()
        .filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item"))
        .map(|t| {
            let item = t.get("Item").expect("an item");
            (
                t.get("UUID").cloned().expect("a UUID"),
                item.get("id").and_then(Tag::as_str).unwrap().to_owned(),
                item.get("count").and_then(Tag::as_i64).unwrap_or(1),
                t.get("Age").and_then(Tag::as_i64).unwrap(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn stack(name: &str, count: i32) -> ItemStack {
    let item = kiln_data::builtin_id("minecraft:item", name).unwrap();
    ItemStack { item, count, added: Vec::new(), removed: Vec::new() }
}

fn world(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A player standing on a stone floor in an empty (void) world, having dropped a stone and
/// all their diamonds, which lie on the floor.
fn dropped_items(dir: &std::path::Path) -> (Sim, Client, [i32; 3]) {
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.to_owned())));
    let (msg, stats) = join(1, "Dropper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Dropper".into())]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    assert!(sim.step([ToSim::Console(format!("fill {} {} {} {} {} {} stone", x - 6, y - 1, z - 6, x + 6, y - 1, z + 6))]));
    assert!(sim.step([
        ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack("minecraft:stone", 3)) }),
        ToSim::Packet(1, PlayIn::PlayerAction { action: 5, pos: [0, 0, 0], face: 0, sequence: 0 }),
        ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 37, item: Some(stack("minecraft:diamond", 7)) }),
        ToSim::Packet(1, PlayIn::SetCarriedItem { slot: 1 }),
        ToSim::Packet(1, PlayIn::PlayerAction { action: 4, pos: [0, 0, 0], face: 0, sequence: 0 }),
    ]));
    settle(&mut sim, &mut client, 40);
    let found = items(&sim);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!((found[0].1.as_str(), found[0].2), ("minecraft:diamond", 7));
    assert_eq!((found[1].1.as_str(), found[1].2), ("minecraft:stone", 1));
    for e in sim.entities() {
        assert!((e.1[1] - y as f64).abs() < 1e-6, "items rest on the floor: {e:?}");
    }
    // Dropped with the drop key: the player is the thrower.
    let player = uuid::Uuid::from_u64_pair(0x6b69_6c6e, 1).as_u128();
    for t in sim.entity_nbt() {
        assert_eq!(t.get("Thrower").and_then(kiln_entity::persist::uuid_from_tag), Some(player), "{t:?}");
    }
    (sim, client, [x, y, z])
}

#[test]
fn items_leave_and_come_back_with_their_chunk() {
    let dir = world("entity-persist-unload");
    let (mut sim, mut client, [x, y, z]) = dropped_items(&dir);
    let before = items(&sim);
    assert!(sim.step([ToSim::Console(format!("tp Dropper {} {} {}", x + 5000, y, z))]));
    settle(&mut sim, &mut client, 60);
    assert!(items(&sim).is_empty(), "the items should have unloaded with their chunk");
    assert!(sim.step([ToSim::Console(format!("tp Dropper {x} {y} {z}"))]));
    settle(&mut sim, &mut client, 3);
    let after = items(&sim);
    assert_eq!(after.len(), 2, "{after:?}");
    for (b, a) in before.iter().zip(&after) {
        assert_eq!((&b.0, &b.1, b.2), (&a.0, &a.1, a.2), "same entity and stack");
        // They age while loaded (a tick or two around the teleports), not while away.
        assert!(a.3 >= b.3 && a.3 < b.3 + 10, "age {} -> {}", b.3, a.3);
    }
}

#[test]
fn items_survive_a_restart() {
    let dir = world("entity-persist-restart");
    let (mut sim, _client, _) = dropped_items(&dir);
    let before = items(&sim);
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!sim.step([ToSim::Shutdown { done }]));
    drop(sim);
    let region = dir.join("dimensions/minecraft/overworld/entities");
    assert!(std::fs::read_dir(&region).unwrap().any(|f| f.unwrap().file_name().to_string_lossy().ends_with(".mca")));

    // The same player comes back to where they were; the items load with the chunk.
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.clone())));
    let (msg, stats) = join(1, "Dropper", 2);
    assert!(sim.step([msg]));
    let after = items(&sim);
    assert_eq!(after.len(), 2, "{after:?}");
    for (b, a) in before.iter().zip(&after) {
        assert_eq!((&b.0, &b.1, b.2), (&a.0, &a.1, a.2), "same entity and stack");
        assert!(a.3 - b.3 <= 1 && a.3 >= b.3, "age {} -> {}", b.3, a.3);
    }
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    assert_eq!(items(&sim).len(), 2);
}
