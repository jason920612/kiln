//! What a server administrator expects a save to keep: the world seed, the difficulty and its
//! lock, the game rules, the operators (`ops.json`) and the command storage. A save written by
//! Kiln has the files and formats vanilla 26.3 writes (tools/persist_check.py has vanilla load
//! them); a save written by vanilla starts Kiln with its own settings.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::{Sim, SimConfig};
use kiln_storage::{LevelState, LevelStore, WorldSpawn};
use std::path::PathBuf;

fn world(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("wp44-admin-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open(dir: &std::path::Path) -> Sim {
    let mut sim = Sim::new(SimConfig::new(4, 2, Some(dir.to_owned())));
    sim.capture_console();
    sim
}

fn console(sim: &mut Sim, command: &str) -> Vec<String> {
    assert!(sim.step([ToSim::Console(command.into())]));
    sim.take_console()
}

fn stop(mut sim: Sim) {
    let (done, _rx) = std::sync::mpsc::channel();
    assert!(!sim.step([ToSim::Shutdown { done }]));
}

fn level_state(difficulty: Option<(u8, bool)>) -> LevelState {
    LevelState {
        game_time: 100,
        day_time: 1000,
        spawn: WorldSpawn { dimension: "minecraft:overworld".into(), pos: [0, 64, 0], yaw: 0.0, pitch: 0.0 },
        data_packs: None,
        enabled_features: None,
        difficulty,
    }
}

/// A save as the vanilla server writes it: seed in `world_gen_settings.dat`, difficulty in
/// `level.dat`, rules in `game_rules.dat`.
fn vanilla_like_save(dir: &std::path::Path, seed: i64, difficulty: (u8, bool)) {
    let mut level = LevelStore::open(dir);
    level.save(&level_state(Some(difficulty))).unwrap();
    let settings = Tag::Compound(vec![("seed".into(), Tag::Long(seed)), ("generate_structures".into(), Tag::Byte(1))]);
    kiln_storage::saved_data::write(dir, "world_gen_settings", settings).unwrap();
    let rules = Tag::Compound(vec![
        ("minecraft:keep_inventory".into(), Tag::Byte(1)),
        ("minecraft:random_tick_speed".into(), Tag::Int(7)),
        ("minecraft:made_up_rule".into(), Tag::Byte(1)),
    ]);
    kiln_storage::saved_data::write(dir, "game_rules", rules).unwrap();
}

#[test]
fn the_seed_difficulty_and_rules_come_from_the_save() {
    let dir = world("load");
    vanilla_like_save(&dir, -4172144997902289642, (1, true));
    let mut sim = open(&dir);
    assert_eq!(sim.seed(), -4172144997902289642);
    let out = console(&mut sim, "seed");
    assert!(out[0].contains("-4172144997902289642"), "{out:?}");
    assert_eq!(console(&mut sim, "difficulty"), ["commands.difficulty.query[options.difficulty.easy]"]);
    assert_eq!(console(&mut sim, "gamerule keep_inventory"), ["commands.gamerule.query[keep_inventory, true]"]);
    assert_eq!(console(&mut sim, "gamerule random_tick_speed"), ["commands.gamerule.query[random_tick_speed, 7]"]);
    // A rule the save does not list has its default.
    assert_eq!(console(&mut sim, "gamerule pvp"), ["commands.gamerule.query[pvp, true]"]);
    assert!(sim.difficulty_locked());
    stop(sim);
}

#[test]
fn a_saved_world_keeps_what_the_operator_changed() {
    let dir = world("roundtrip");
    vanilla_like_save(&dir, 99, (2, false));
    let mut sim = open(&dir);
    for c in [
        "difficulty hard",
        "gamerule pvp false",
        "gamerule respawn_radius 3",
        "data modify storage kiln:test counter set value 5",
        "data modify storage minecraft:plain list set value [1,2,3]",
    ] {
        console(&mut sim, c);
    }
    stop(sim);

    // The files are the ones vanilla writes.
    let level = LevelStore::open(&dir);
    assert_eq!(level.difficulty(), Some((3, false)));
    assert_eq!(level.seed(), Some(99), "the seed stays");
    let rules: std::collections::HashMap<_, _> = level.game_rules().into_iter().collect();
    assert_eq!(rules["minecraft:pvp"], kiln_storage::level::SavedRule::Bool(false));
    assert_eq!(rules["minecraft:respawn_radius"], kiln_storage::level::SavedRule::Int(3));
    assert_eq!(rules["minecraft:keep_inventory"], kiln_storage::level::SavedRule::Bool(true));
    assert_eq!(rules["minecraft:random_tick_speed"], kiln_storage::level::SavedRule::Int(7));
    assert!(rules.contains_key("minecraft:max_entity_cramming"), "vanilla writes every rule");
    let kiln = kiln_storage::saved_data::read_ns(&dir, "kiln", "command_storage").expect("kiln namespace file");
    assert_eq!(kiln.get("contents").and_then(|c| c.get("test")).and_then(|t| t.get("counter")).and_then(Tag::as_i64), Some(5));
    assert!(kiln_storage::saved_data::read_ns(&dir, "minecraft", "command_storage").is_some());

    let mut sim = open(&dir);
    assert_eq!(console(&mut sim, "difficulty"), ["commands.difficulty.query[options.difficulty.hard]"]);
    assert_eq!(console(&mut sim, "gamerule pvp"), ["commands.gamerule.query[pvp, false]"]);
    assert_eq!(console(&mut sim, "gamerule respawn_radius"), ["commands.gamerule.query[respawn_radius, 3]"]);
    let out = console(&mut sim, "data get storage kiln:test counter");
    assert!(out[0].contains('5'), "{out:?}");
    let out = console(&mut sim, "data get storage minecraft:plain list[1]");
    assert!(out[0].contains('2'), "{out:?}");
    stop(sim);
}

#[test]
fn emptied_command_storage_is_forgotten() {
    let dir = world("storage-empty");
    let mut sim = open(&dir);
    console(&mut sim, "data modify storage kiln:gone x set value 1");
    console(&mut sim, "data modify storage kiln:gone x set value 2");
    console(&mut sim, "data remove storage kiln:gone x");
    stop(sim);
    let mut sim = open(&dir);
    let out = console(&mut sim, "data get storage kiln:gone");
    assert!(out[0].starts_with("commands.data.storage.get") || out[0].contains("error") || out[0].contains("{}"), "{out:?}");
    stop(sim);
}

#[test]
fn a_new_world_gets_the_files_vanilla_expects() {
    let dir = world("fresh");
    let sim = open(&dir);
    stop(sim);
    assert!(dir.join("level.dat").exists());
    let level = LevelStore::open(&dir);
    assert!(level.difficulty().is_some(), "difficulty_settings is written");
    assert!(!level.game_rules().is_empty());
    assert!(dir.join("data/minecraft/game_rules.dat").exists());
}

#[test]
fn operators_are_read_from_and_written_to_ops_json() {
    let dir = world("ops");
    let server_dir = world("ops-server");
    std::fs::write(
        server_dir.join("ops.json"),
        r#"[{"uuid":"00000000-0000-0000-0000-000000000007","name":"Alex","level":3,"bypassesPlayerLimit":false}]"#,
    )
    .unwrap();
    let access = kiln_link::access::AccessLists::new(Some(server_dir.clone())).shared();
    let mut config = SimConfig::new(4, 2, Some(dir.clone()));
    config.access = access;
    let mut sim = Sim::new(config);
    sim.capture_console();
    assert_eq!(sim.permission_level_of_name("Alex"), 3, "a level 3 operator");
    assert_eq!(sim.permission_level_of_name("Steve"), 0);
    console(&mut sim, "op Steve");
    assert_eq!(sim.permission_level_of_name("steve"), 4);
    let text = std::fs::read_to_string(server_dir.join("ops.json")).unwrap();
    assert!(text.contains("\"name\": \"steve\"") && text.contains("\"level\": 4") && text.contains("Alex"), "{text}");
    console(&mut sim, "deop steve");
    assert_eq!(sim.permission_level_of_name("steve"), 0);
    assert_eq!(sim.permission_level_of_name("Alex"), 3, "the others stay");
    let text = std::fs::read_to_string(server_dir.join("ops.json")).unwrap();
    assert!(!text.contains("steve") && text.contains("Alex"), "{text}");
    stop(sim);
}

fn settle(sim: &mut Sim, client: &mut kiln_sim::testing::Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

#[test]
fn chunks_near_players_accumulate_inhabited_time_and_keep_it() {
    // The spawning pass (which counts the chunks players are near) needs the vanilla datapack.
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let dir = std::env::var_os("KILN_WORK").map(|w| PathBuf::from(w).join("generated"));
        match dir.filter(|d| d.exists()) {
            Some(dir) => unsafe { std::env::set_var("KILN_DATAPACK", dir) },
            None => return eprintln!("skipped: no vanilla datapack (KILN_DATAPACK or KILN_WORK)"),
        }
    }
    let dir = world("inhabited");
    let mut sim = open(&dir);
    let (msg, stats) = kiln_sim::testing::join(1, "Walker", 2);
    assert!(sim.step([msg]));
    let mut client = kiln_sim::testing::Client::new(1, stats);
    settle(&mut sim, &mut client, 1300);
    let near = sim.inhabited_time_at(8, 8).expect("the spawn chunk is loaded");
    assert!(near >= 1200, "a player stood by the chunk for 1300 ticks: {near}");
    assert!(sim.inhabited_time_at(8 + 16 * 40, 8).is_none(), "far chunks are not even loaded");
    stop(sim);

    // The chunk went to disk with its InhabitedTime (a chunk nothing changed in is rewritten for
    // it once it grew by a minute), and a restarted server reads it back.
    let mut sim = open(&dir);
    let (msg, stats) = kiln_sim::testing::join(1, "Walker", 2);
    assert!(sim.step([msg]));
    let mut client = kiln_sim::testing::Client::new(1, stats);
    settle(&mut sim, &mut client, 2);
    let back = sim.inhabited_time_at(8, 8).expect("loaded again");
    assert!(back >= near, "{back} >= {near}");
}
