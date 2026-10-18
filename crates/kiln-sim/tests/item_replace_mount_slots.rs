//! `/item replace entity` reaches the saddle and body armour of the mounts (wp68): `saddle` and `armor.body` were "no such slot"
//! for every mob (a horse, a nautilus, a camel, a happy ghast), because the command only knew a mob's hands and four armour pieces.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn equipment_of(sim: &Sim, id: &str) -> Vec<(String, String)> {
    let t = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(id)).expect("the entity");
    match t.get("equipment") {
        Some(Tag::Compound(eq)) => eq.iter().map(|(slot, item)| (slot.clone(), item.get("id").and_then(Tag::as_str).unwrap_or("?").to_owned())).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn the_saddle_and_body_slots_of_mounts_take_items() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    for (k, kind) in ["horse", "nautilus", "camel", "happy_ghast"].into_iter().enumerate() {
        assert!(sim.step([ToSim::Console(format!("summon minecraft:{kind} {} {} {} {{PersistenceRequired:1b,NoAI:1b,Tame:1b}}", p[0] + 3.0 + k as f64 * 5.0, p[1], p[2]))]));
    }
    ticks(&mut sim, &mut client, 3);
    let run = |sim: &mut Sim, cmd: &str| assert!(sim.step([ToSim::Console(cmd.into())]));
    run(&mut sim, "item replace entity @e[type=horse,limit=1] saddle with minecraft:saddle");
    run(&mut sim, "item replace entity @e[type=horse,limit=1] armor.body with minecraft:iron_horse_armor");
    run(&mut sim, "item replace entity @e[type=nautilus,limit=1] saddle with minecraft:saddle");
    run(&mut sim, "item replace entity @e[type=nautilus,limit=1] armor.body with minecraft:iron_nautilus_armor");
    run(&mut sim, "item replace entity @e[type=camel,limit=1] saddle with minecraft:saddle");
    run(&mut sim, "item replace entity @e[type=happy_ghast,limit=1] armor.body with minecraft:blue_harness");
    ticks(&mut sim, &mut client, 2);
    let has = |eq: &[(String, String)], slot: &str, item: &str| eq.iter().any(|(s, i)| s == slot && i == item);
    let horse = equipment_of(&sim, "minecraft:horse");
    assert!(has(&horse, "saddle", "minecraft:saddle") && has(&horse, "body", "minecraft:iron_horse_armor"), "horse: {horse:?}");
    let nautilus = equipment_of(&sim, "minecraft:nautilus");
    assert!(has(&nautilus, "saddle", "minecraft:saddle") && has(&nautilus, "body", "minecraft:iron_nautilus_armor"), "nautilus: {nautilus:?}");
    let camel = equipment_of(&sim, "minecraft:camel");
    assert!(has(&camel, "saddle", "minecraft:saddle"), "camel: {camel:?}");
    let ghast = equipment_of(&sim, "minecraft:happy_ghast");
    assert!(has(&ghast, "body", "minecraft:blue_harness"), "ghast: {ghast:?}");
    // and back to nothing
    run(&mut sim, "item replace entity @e[type=horse,limit=1] saddle with minecraft:air");
    ticks(&mut sim, &mut client, 2);
    assert!(!has(&equipment_of(&sim, "minecraft:horse"), "saddle", "minecraft:saddle"));
}
