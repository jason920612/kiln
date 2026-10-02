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
            ToSim::Packet(1, PlayIn::Move { pos: Some([ground[0], y, ground[2]]), rot: None, on_ground, horizontal_collision: false }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    // 10 blocks minus the safe 3: 7 damage.
    assert_eq!(sim.health(1), Some((13.0, false)));
}

/// A totem of undying in hand: a deadly fall leaves the player at 1 health (then absorption
/// and regeneration), the totem used up; `/kill` goes through it.
#[test]
fn totems_save_players_from_death() {
    let (mut sim, mut client) = joined();
    hold(&mut sim, "minecraft:totem_of_undying", 1);
    let ground = client.pos;
    assert!(sim.step([ToSim::Console(format!("tp Faller {} {} {}", ground[0], ground[1] + 30.0, ground[2]))]));
    for _ in 0..2 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    for i in 1..=30 {
        let y = ground[1] + 30.0 - i as f64;
        assert!(sim.step([
            ToSim::Packet(1, PlayIn::Move { pos: Some([ground[0], y, ground[2]]), rot: None, on_ground: i == 30, horizontal_collision: false }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    let (health, dead) = sim.health(1).unwrap();
    // (Regeneration II may already have healed a point.)
    assert!(!dead && health <= 2.0 + 1e-3, "saved at 1 health, got {health}");
    assert_eq!(sim.inventory(1).unwrap()[36], None, "the totem is used up");
    let effects: Vec<&str> = sim.effects(1).unwrap().into_iter().map(|e| e.0).collect();
    assert!(effects.iter().any(|e| *e == "minecraft:regeneration") && effects.iter().any(|e| *e == "minecraft:absorption"), "{effects:?}");
    if let Some(done) = sim.criterion_done(1, "minecraft:adventure/totem_of_undying", "used_totem") {
        assert!(done);
    }
    hold(&mut sim, "minecraft:totem_of_undying", 1);
    assert!(sim.step([ToSim::Console("kill Faller".into())]));
    assert_eq!(sim.health(1), Some((0.0, true)), "/kill bypasses the totem");
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
            ToSim::Packet(1, PlayIn::Move { pos: Some([ground[0], y, ground[2]]), rot: None, on_ground: i == 10, horizontal_collision: false }),
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
            ToSim::Packet(1, PlayIn::Move { pos: Some([x, ground[1], ground[2]]), rot: None, on_ground: true, horizontal_collision: false }),
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
            ToSim::Packet(1, PlayIn::Move { pos: Some([x, ground[1], ground[2]]), rot: None, on_ground: true, horizontal_collision: false }),
            ToSim::Packet(1, PlayIn::ClientTickEnd),
        ]));
    }
    assert!(sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 2 })]));
    eat(&mut sim, 34);
    let bowl = kiln_data::builtin_id("minecraft:item", "minecraft:bowl").unwrap();
    assert_eq!(sim.inventory(1).unwrap()[36], Some((bowl, 1)));
    assert_eq!(sim.food(1).unwrap().0, 20);
}

#[test]
fn effects_from_commands_and_golden_apples_and_milk_clears_them() {
    let (mut sim, _client) = joined();
    assert!(sim.step([ToSim::Console("effect give Faller minecraft:poison 10 1".into())]));
    let effects = sim.effects(1).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!((effects[0].0, effects[0].1), ("minecraft:poison", 1));
    // Poison II hurts every 12 ticks.
    for _ in 0..30 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    assert!(sim.health(1).unwrap().0 < 20.0);
    hold(&mut sim, "minecraft:golden_apple", 1);
    eat(&mut sim, 34);
    let names: Vec<&str> = sim.effects(1).unwrap().iter().map(|e| e.0).collect();
    assert_eq!(names, ["minecraft:regeneration", "minecraft:poison", "minecraft:absorption"]);
    hold(&mut sim, "minecraft:milk_bucket", 1);
    eat(&mut sim, 34);
    assert_eq!(sim.effects(1).unwrap(), []);
    let bucket = kiln_data::builtin_id("minecraft:item", "minecraft:bucket").unwrap();
    assert_eq!(sim.inventory(1).unwrap()[36], Some((bucket, 1)));
}

#[test]
fn fire_burns_and_water_puts_it_out() {
    let (mut sim, client) = joined();
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    assert_eq!(sim.fire_and_air(1), Some((-20, 300)));
    assert!(sim.step([ToSim::Console(format!("setblock {x} {y} {z} minecraft:fire"))]));
    for _ in 0..30 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    let (fire, _) = sim.fire_and_air(1).unwrap();
    assert!(fire > 150, "burning for 8 seconds after 20 ticks in the fire ({fire})");
    assert!(sim.health(1).unwrap().0 < 20.0);
    assert!(sim.step([ToSim::Console(format!("setblock {x} {y} {z} minecraft:water"))]));
    assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    assert_eq!(sim.fire_and_air(1).unwrap().0, -20, "water puts the fire out");
}

#[test]
fn players_drown_with_their_head_under_water() {
    let (mut sim, client) = joined();
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    let glass = format!("fill {} {} {} {} {} {} minecraft:glass", x - 1, y, z - 1, x + 1, y + 2, z + 1);
    assert!(sim.step([ToSim::Console(glass)]));
    assert!(sim.step([ToSim::Console(format!("fill {x} {y} {z} {x} {} {z} minecraft:water", y + 2))]));
    for _ in 0..300 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    let (_, air) = sim.fire_and_air(1).unwrap();
    assert!((-20..=0).contains(&air), "air {air}");
    let before = sim.health(1).unwrap().0;
    let mut ticks = 0;
    while sim.health(1).unwrap().0 == before {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
        ticks += 1;
        assert!(ticks <= 21, "no drowning damage");
    }
    // At -20 the air resets to 0 with 2 drowning damage.
    assert_eq!(sim.health(1).unwrap().0, before - 2.0);
    assert_eq!(sim.fire_and_air(1).unwrap().1, 0);
    assert!(sim.step([ToSim::Console("effect give Faller minecraft:water_breathing 10".into())]));
    let (_, air) = sim.fire_and_air(1).unwrap();
    for _ in 0..10 {
        assert!(sim.step([ToSim::Packet(1, PlayIn::ClientTickEnd)]));
    }
    assert_eq!(sim.fire_and_air(1).unwrap().1, (air + 40).min(300), "water breathing refills the air");
}

#[test]
fn chorus_fruit_teleports() {
    let (mut sim, client) = joined();
    hold(&mut sim, "minecraft:chorus_fruit", 1);
    eat(&mut sim, 34);
    let tp = client.stats.teleport.lock().unwrap().unwrap().1;
    let d = ((tp[0] - client.pos[0]).powi(2) + (tp[2] - client.pos[2]).powi(2)).sqrt();
    assert!(d > 0.0 && d < 12.0, "teleported {d} blocks to {tp:?}");
}
