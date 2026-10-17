//! A player's saved `attributes` (`AttributeMap.pack`, wp77), read off a 26.3 server with `data get entity`: the four attributes a player is made with
//! (block and entity interaction range, movement speed, waypoint transmit range; no max health), then the ones something changed or asked to change
//! (a worn armor piece, a held weapon, `/attribute`, a potion effect on the speed), each with its base and its permanent modifiers only: an effect's
//! (`minecraft:effect.speed`) and the ones `/attribute modifier add` made are saved, the equipment's are not. They come back after a restart.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;

fn world(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settle(sim: &mut Sim, client: &mut Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

/// id -> (base, modifier ids) of the saved attributes.
fn attrs(sim: &Sim) -> Vec<(String, f64, Vec<String>)> {
    let nbt = sim.saved_player_nbt(1).expect("the player");
    let Some(Tag::List(list)) = nbt.get("attributes") else { panic!("no attributes in {nbt:?}") };
    let mut out: Vec<_> = list
        .iter()
        .map(|a| {
            let a = a.unwrap_list_element();
            let mods = match a.get("modifiers") {
                Some(Tag::List(m)) => m.iter().map(|m| m.unwrap_list_element().get("id").and_then(Tag::as_str).unwrap().to_owned()).collect(),
                _ => Vec::new(),
            };
            (a.get("id").and_then(Tag::as_str).unwrap().trim_start_matches("minecraft:").to_owned(), a.get("base").and_then(Tag::as_f64).unwrap(), mods)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn ids(v: &[(String, f64, Vec<String>)]) -> Vec<&str> {
    v.iter().map(|a| a.0.as_str()).collect()
}

#[test]
fn a_player_saves_the_attributes_that_exist_and_their_permanent_modifiers() {
    let dir = world("player_attributes");
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.clone())));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode survival Keeper".into())]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    let start = attrs(&sim);
    assert_eq!(ids(&start), ["block_interaction_range", "entity_interaction_range", "movement_speed", "waypoint_transmit_range"], "{start:?}");
    assert_eq!(start.iter().find(|a| a.0 == "movement_speed").unwrap().1, 0.10000000149011612);
    assert_eq!(start.iter().find(|a| a.0 == "waypoint_transmit_range").unwrap().1, 6.0e7);

    // Querying an attribute makes no instance; a base, a modifier, an effect and the equipment do.
    for c in [
        "attribute Keeper minecraft:scale get",
        "attribute Keeper minecraft:luck base set 2",
        "attribute Keeper minecraft:luck modifier add test:lucky 3 add_value",
        "effect give Keeper minecraft:speed 1000 1",
        "item replace entity Keeper armor.chest with minecraft:iron_chestplate",
        "item replace entity Keeper weapon.mainhand with minecraft:diamond_sword",
    ] {
        assert!(sim.step([ToSim::Console(c.into())]));
    }
    settle(&mut sim, &mut client, 5);
    let now = attrs(&sim);
    assert_eq!(
        ids(&now),
        ["armor", "armor_toughness", "attack_damage", "attack_speed", "block_interaction_range", "entity_interaction_range", "luck", "movement_speed", "waypoint_transmit_range"],
        "{now:?}"
    );
    let get = |n: &str| now.iter().find(|a| a.0 == n).unwrap();
    assert_eq!(get("luck").1, 2.0);
    assert_eq!(get("luck").2, ["test:lucky"], "a modifier of /attribute is permanent");
    assert_eq!(get("movement_speed").2, ["minecraft:effect.speed"], "an effect's modifier is saved");
    assert!(get("armor").2.is_empty() && get("attack_damage").2.is_empty() && get("armor_toughness").2.is_empty(), "equipment modifiers are transient: {now:?}");
    assert_eq!(get("attack_damage").1, 1.0);

    // A restart: the base, the modifier and the instances come back (the effect brings its own modifier back).
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!sim.step([ToSim::Shutdown { done }]));
    drop(sim);
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir)));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    let after = attrs(&sim);
    let get = |n: &str| after.iter().find(|a| a.0 == n).unwrap_or_else(|| panic!("{n} in {after:?}"));
    assert_eq!(get("luck").1, 2.0);
    assert_eq!(get("luck").2, ["test:lucky"]);
    assert_eq!(get("movement_speed").2, ["minecraft:effect.speed"]);
    assert!(ids(&after).contains(&"armor"), "an instance stays: {after:?}");
}
