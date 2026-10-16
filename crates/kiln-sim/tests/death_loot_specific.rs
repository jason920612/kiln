//! A killed sheep drops its wool and a killed small slime its slime balls (wp68). The death loot tables ask the dying mob's class
//! (`type_specific/sheep` `sheared: false` for the wool, `type_specific/cube_mob` `size` for the slime balls and the magma cream),
//! and Kiln's death context answered "no" to every class predicate: no wool from a killed sheep, no slime balls, no magma cream.
//! Found by killing the same 12 sheep and slimes on Kiln and on a vanilla server.
//!
//! Loot tables come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/recipe").is_dir();
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        let found = dir.join("data/minecraft/recipe").is_dir();
        if found {
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
        found
    })
}

fn items_dropped(sim: &Sim, id: &str) -> usize {
    sim.entity_nbt()
        .into_iter()
        .filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item"))
        .filter(|t| matches!(t.get("Item"), Some(i) if i.get("id").and_then(Tag::as_str) == Some(id)))
        .count()
}

#[test]
fn killed_sheep_drop_wool_and_killed_small_slimes_drop_slime_balls() {
    if !have_datapack() {
        eprintln!("no vanilla datapack: skipped");
        return;
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let mut ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    for _ in 0..16 {
        assert!(sim.step([ToSim::Console(format!("summon minecraft:sheep {} {} {} {{NoAI:1b,Color:0b}}", p[0] + 4.0, p[1], p[2]))]));
        assert!(sim.step([ToSim::Console(format!("summon minecraft:slime {} {} {} {{NoAI:1b,Size:0}}", p[0] - 4.0, p[1], p[2]))]));
    }
    ticks(&mut sim, &mut client, 3);
    assert!(sim.step([ToSim::Console("kill @e[type=sheep]".into()), ToSim::Console("kill @e[type=slime]".into())]));
    ticks(&mut sim, &mut client, 3);
    assert!(items_dropped(&sim, "minecraft:white_wool") > 0, "wool from killed white sheep");
    assert!(items_dropped(&sim, "minecraft:mutton") > 0, "mutton from killed sheep");
    assert!(items_dropped(&sim, "minecraft:slime_ball") > 0, "slime balls from killed small slimes");
}

/// A creeper killed by a skeleton drops a music disc (`entity_properties` of the `attacker`: `#minecraft:skeletons`), which the death
/// context could not answer for a mob killer.
#[test]
fn a_creeper_killed_by_a_skeleton_drops_a_music_disc() {
    if !have_datapack() {
        eprintln!("no vanilla datapack: skipped");
        return;
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let mut ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    assert!(sim.step([ToSim::Console(format!("summon minecraft:skeleton {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] + 4.0, p[1], p[2]))]));
    for _ in 0..10 {
        assert!(sim.step([ToSim::Console(format!("summon minecraft:creeper {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] - 4.0, p[1], p[2]))]));
        ticks(&mut sim, &mut client, 1);
        assert!(sim.step([ToSim::Console("damage @e[type=creeper,limit=1] 100 minecraft:arrow by @e[type=skeleton,limit=1]".into())]));
        ticks(&mut sim, &mut client, 2);
    }
    let discs = sim
        .entity_nbt()
        .into_iter()
        .filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item"))
        .filter(|t| t.get("Item").and_then(|i| i.get("id")).and_then(Tag::as_str).is_some_and(|id| id.contains("music_disc")))
        .count();
    assert_eq!(discs, 10, "a disc from each creeper the skeleton killed");
    // ... and none from the creepers a command kills
    assert!(sim.step([ToSim::Console("kill @e[type=item]".into())]));
    for _ in 0..5 {
        assert!(sim.step([ToSim::Console(format!("summon minecraft:creeper {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] - 4.0, p[1], p[2]))]));
        ticks(&mut sim, &mut client, 1);
        assert!(sim.step([ToSim::Console("kill @e[type=creeper]".into())]));
        ticks(&mut sim, &mut client, 2);
    }
    let discs = sim.entity_nbt().into_iter().filter(|t| t.get("Item").and_then(|i| i.get("id")).and_then(Tag::as_str).is_some_and(|id| id.contains("music_disc"))).count();
    assert_eq!(discs, 0);
}

/// A mob that dies burning drops its meat cooked (`furnace_smelt` under `entity_properties` `flags.is_on_fire`): the loot context
/// could not smelt at all, so burning cows dropped raw beef.
#[test]
fn burning_cows_drop_cooked_beef() {
    if !have_datapack() {
        eprintln!("no vanilla datapack: skipped");
        return;
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let mut ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    for _ in 0..12 {
        assert!(sim.step([ToSim::Console(format!("summon minecraft:cow {} {} {} {{NoAI:1b,Fire:200s}}", p[0] + 4.0, p[1], p[2]))]));
    }
    ticks(&mut sim, &mut client, 3);
    assert!(sim.step([ToSim::Console("kill @e[type=cow]".into())]));
    ticks(&mut sim, &mut client, 3);
    assert!(items_dropped(&sim, "minecraft:cooked_beef") > 0, "cooked beef from burning cows");
    assert_eq!(items_dropped(&sim, "minecraft:beef"), 0, "no raw beef from burning cows");
}
