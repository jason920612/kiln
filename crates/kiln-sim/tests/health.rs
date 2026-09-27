//! Fall damage, death by /kill with the inventory scattered, respawning, and food: healing
//! when well fed and exhaustion from sprinting.

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

#[test]
fn well_fed_players_heal() {
    let (mut sim, mut client) = joined();
    let ground = client.pos;
    assert!(sim.step([ToSim::Console(format!("tp Faller {} {} {}", ground[0], ground[1] + 10.0, ground[2]))]));
    for _ in 0..2 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    for i in 1..=10 {
        let y = ground[1] + 10.0 - i as f64;
        assert!(sim.step([
            ToSim::Packet(1, PlayIn::Move { pos: Some([ground[0], y, ground[2]]), rot: None, on_ground: i == 10 }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    assert_eq!(sim.health(1), Some((13.0, false)));
    // Full food with saturation: saturation / 6 health every 10 ticks, using saturation.
    for _ in 0..40 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    let (health, _) = sim.health(1).unwrap();
    assert!(health > 14.0, "healed to {health}");
    let (food, saturation) = sim.food(1).unwrap();
    assert_eq!(food, 20);
    assert!(saturation < 5.0, "healing used saturation ({saturation})");
}

#[test]
fn sprinting_uses_saturation() {
    let (mut sim, client) = joined();
    let ground = client.pos;
    // Sprinting 0.28 blocks a tick for 200 ticks: 56 blocks, 5.6 exhaustion, one saturation point.
    let (_, before) = sim.food(1).unwrap();
    assert!(sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 1 })]));
    let mut x = ground[0];
    for _ in 0..200 {
        x += 0.28;
        assert!(sim.step([
            ToSim::Packet(1, PlayIn::Move { pos: Some([x, ground[1], ground[2]]), rot: None, on_ground: true }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    let (_, after) = sim.food(1).unwrap();
    assert_eq!((before, after), (5.0, 4.0), "5.6 exhaustion takes one saturation point");
}

fn hold(sim: &mut Sim, item: &str, count: i32) {
    let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
    assert!(sim.step([ToSim::Console("gamemode creative Faller".into())]));
    let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
    assert!(sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    assert!(sim.step([ToSim::Console("gamemode survival Faller".into())]));
}

fn eat(sim: &mut Sim, ticks: usize) {
    use kiln_proto::packets::serverbound::Hand;
    assert!(sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: Hand::Main, sequence: 1, yaw: 0.0, pitch: 0.0 })]));
    for _ in 0..ticks {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
}

#[test]
fn golden_apples_are_always_edible() {
    let (mut sim, _client) = joined();
    hold(&mut sim, "minecraft:golden_apple", 2);
    eat(&mut sim, 34);
    let apple = kiln_data::builtin_id("minecraft:item", "minecraft:golden_apple").unwrap();
    assert_eq!(sim.inventory(1).unwrap()[36], Some((apple, 1)));
    let (food, saturation) = sim.food(1).unwrap();
    assert_eq!(food, 20);
    assert!(saturation > 14.0, "saturation {saturation}");
}

#[test]
fn stew_is_eaten_only_when_hungry_and_leaves_a_bowl() {
    let (mut sim, client) = joined();
    hold(&mut sim, "minecraft:mushroom_stew", 1);
    let stew = kiln_data::builtin_id("minecraft:item", "minecraft:mushroom_stew").unwrap();
    eat(&mut sim, 40);
    assert_eq!(sim.inventory(1).unwrap()[36], Some((stew, 1)), "not hungry: not eaten");
    // Sprint until hungry.
    let ground = client.pos;
    assert!(sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 1 })]));
    let mut x = ground[0];
    while sim.food(1).unwrap().0 == 20 {
        x += 0.28;
        assert!(sim.step([
            ToSim::Packet(1, PlayIn::Move { pos: Some([x, ground[1], ground[2]]), rot: None, on_ground: true }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    assert!(sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 2 })]));
    eat(&mut sim, 34);
    let bowl = kiln_data::builtin_id("minecraft:item", "minecraft:bowl").unwrap();
    assert_eq!(sim.inventory(1).unwrap()[36], Some((bowl, 1)));
    assert_eq!(sim.food(1).unwrap().0, 20);
}
