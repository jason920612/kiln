//! The triggers wp53 added, against advancements written for the test (no vanilla advancement uses `any_block_use`,
//! `default_block_use` or `used_ender_eye`; the others have vanilla ones, which the interaction vectors cover for the bee nest).

use crate::testing::{Client, join};
use crate::{Sim, SimConfig};
use kiln_item::ItemStack;
use kiln_link::{PlayIn, ToSim};

fn pack(name: &str, advancements: &[(&str, &str, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-wp53-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let adv = dir.join("data/test/advancement");
    std::fs::create_dir_all(&adv).unwrap();
    for (id, trigger, conditions) in advancements {
        std::fs::write(adv.join(format!("{id}.json")), format!(r#"{{"criteria":{{"c":{{"trigger":"{trigger}","conditions":{conditions}}}}}}}"#)).unwrap();
    }
    dir
}

fn sim_with(name: &str, advancements: &[(&str, &str, &str)]) -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(2, 2, None));
    let (msg, stats) = join(1, "Trig", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:show_advancement_messages false".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    sim.load_advancements(&[pack(name, advancements)]);
    (sim, client)
}

fn done(sim: &Sim) -> Vec<String> {
    sim.players.get(&1).unwrap().advancements.done_criteria()
}

#[test]
fn spear_mobs_counts_up_to_the_stabbed() {
    let (mut sim, _c) = sim_with("spear", &[("three", "minecraft:spear_mobs", r#"{"count":3}"#), ("any", "minecraft:spear_mobs", "{}")]);
    sim.players.get_mut(&1).unwrap().spear_mobs(2);
    assert_eq!(done(&sim), ["test:any/c"]);
    sim.players.get_mut(&1).unwrap().spear_mobs(3);
    assert_eq!(done(&sim), ["test:any/c", "test:three/c"]);
}

#[test]
fn used_ender_eye_compares_the_horizontal_distance_squared() {
    let (mut sim, _c) = sim_with(
        "eye",
        &[("near", "minecraft:used_ender_eye", r#"{"distance":{"max":10}}"#), ("far", "minecraft:used_ender_eye", r#"{"distance":{"min":50}}"#)],
    );
    let p = sim.players.get_mut(&1).unwrap();
    let at = [p.pos[0] as i32 + 30, 1000, p.pos[2] as i32 + 40];
    // 50 blocks away (the height does not count): only `far`.
    p.used_ender_eye(at);
    assert_eq!(done(&sim), ["test:far/c"]);
    let p = sim.players.get_mut(&1).unwrap();
    let at = [p.pos[0] as i32 + 3, -5, p.pos[2] as i32 + 4];
    p.used_ender_eye(at);
    assert_eq!(done(&sim), ["test:far/c", "test:near/c"]);
}

#[test]
fn a_bee_nest_destroyed_needs_its_block_bees_and_tool() {
    let nest = kiln_data::blocks::default_state::BEE_NEST;
    let hive = kiln_data::blocks::default_state::BEEHIVE;
    let (mut sim, _c) = sim_with(
        "nest",
        &[("nest", "minecraft:bee_nest_destroyed", r#"{"blocks":"minecraft:bee_nest","num_bees_inside":3,"item":{"items":"minecraft:diamond_pickaxe"}}"#)],
    );
    let pick = ItemStack::of("minecraft:diamond_pickaxe", 1).unwrap();
    let p = sim.players.get_mut(&1).unwrap();
    p.bee_nest_destroyed(hive, &pick, 3);
    p.bee_nest_destroyed(nest, &pick, 2);
    p.bee_nest_destroyed(nest, &ItemStack::of("minecraft:stick", 1).unwrap(), 3);
    assert!(done(&sim).is_empty(), "{:?}", done(&sim));
    sim.players.get_mut(&1).unwrap().bee_nest_destroyed(nest, &pick, 3);
    assert_eq!(done(&sim), ["test:nest/c"]);
}

#[test]
fn what_a_player_threw_and_an_entity_took_is_told_to_the_thrower() {
    use kiln_entity::level::{Criterion, Seen};
    let (mut sim, _c) = sim_with(
        "thrown",
        &[
            (
                "piglin",
                "minecraft:thrown_item_picked_up_by_entity",
                r#"{"item":{"items":"minecraft:gold_ingot"},"entity":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:piglin"}}}"#,
            ),
            (
                "cow",
                "minecraft:thrown_item_picked_up_by_entity",
                r#"{"entity":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:cow"}}}"#,
            ),
        ],
    );
    let piglin = kiln_entity::mob::new(kiln_entity::mob::MobKind::Piglin, 77, 77, 1);
    let seen = Seen::of(&piglin);
    let p = sim.players.get_mut(&1).unwrap();
    p.entity_criterion("minecraft:overworld", &Criterion::ThrownItemPickedUp { item: ItemStack::of("minecraft:stick", 1).unwrap(), entity: seen.clone() });
    assert!(done(&sim).is_empty(), "wrong item");
    let p = sim.players.get_mut(&1).unwrap();
    p.entity_criterion("minecraft:overworld", &Criterion::ThrownItemPickedUp { item: ItemStack::of("minecraft:gold_ingot", 1).unwrap(), entity: seen });
    assert_eq!(done(&sim), ["test:piglin/c"], "a piglin took a gold ingot");
}

#[test]
fn an_allay_dropping_cake_on_a_note_block() {
    use kiln_entity::level::Criterion;
    let cake = r#"{"location":{"type":"minecraft:all_of","terms":[{"type":"minecraft:location_check","predicate":{"block":{"blocks":"minecraft:note_block"}}},{"type":"minecraft:match_tool","predicate":{"items":"minecraft:cake"}}]}}"#;
    let (mut sim, _c) = sim_with("allay", &[("cake", "minecraft:allay_drop_item_on_block", cake)]);
    let note = kiln_data::blocks::default_state::NOTE_BLOCK;
    let at = kiln_entity::math::BlockPos::new(0, 64, 0);
    let p = sim.players.get_mut(&1).unwrap();
    p.entity_criterion("minecraft:overworld", &Criterion::AllayDropItem { pos: at, state: kiln_data::blocks::default_state::STONE, item: ItemStack::of("minecraft:cake", 1).unwrap() });
    p.entity_criterion("minecraft:overworld", &Criterion::AllayDropItem { pos: at, state: note, item: ItemStack::of("minecraft:apple", 1).unwrap() });
    assert!(done(&sim).is_empty(), "{:?}", done(&sim));
    sim.players
        .get_mut(&1)
        .unwrap()
        .entity_criterion("minecraft:overworld", &Criterion::AllayDropItem { pos: at, state: note, item: ItemStack::of("minecraft:cake", 1).unwrap() });
    assert_eq!(done(&sim), ["test:cake/c"]);
}

/// `default_block_use` is the empty hand's click that a block takes; `any_block_use` is any click that did something.
#[test]
fn block_uses_fire_any_and_default() {
    let lever = r#"{"location":{"type":"minecraft:location_check","predicate":{"block":{"blocks":"minecraft:lever"}}}}"#;
    let (mut sim, mut client) = sim_with(
        "uses",
        &[("any_lever", "minecraft:any_block_use", lever), ("default_lever", "minecraft:default_block_use", lever), ("any", "minecraft:any_block_use", "{}")],
    );
    assert!(sim.step([
        ToSim::Console("setblock 2 99 0 minecraft:stone".into()),
        ToSim::Console("setblock 2 100 0 minecraft:lever[face=floor,facing=north,powered=false]".into()),
    ]));
    {
        let p = sim.players.get_mut(&1).unwrap();
        p.pos = [0.5, 100.0, 0.5];
        p.on_ground = true;
        p.game_mode = 0;
    }
    let mut inbox = vec![ToSim::Packet(1, PlayIn::UseItemOn { hand: 0, pos: [2, 100, 0], face: 1, cursor: [0.5, 0.5, 0.5], inside: false, sequence: 1 })];
    client.tick(None, &mut inbox);
    assert!(sim.step(inbox));
    // Nothing in hand: the lever takes the click (both triggers; the lever is the block).
    assert_eq!(done(&sim), ["test:any/c", "test:any_lever/c", "test:default_lever/c"]);
}
