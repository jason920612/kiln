//! wp86: the `server.properties` settings that are process-wide in the simulation (`max-world-size`,
//! `entity-broadcast-range-percentage`) and `force-gamemode`/`op-permission-level`, which need a restart or a console.
//! Kept in one test binary and one test at a time each: a `Sim` applies its settings when it is made.

use kiln_link::{ConnId, ToSim};
use kiln_proto::Reader;
use kiln_sim::testing::{self, Client};
use kiln_sim::{Sim, SimConfig};
use std::sync::Mutex;

/// Tests here set process-wide values: one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

const ADD_ENTITY: i32 = kiln_data::packets::play::clientbound::ADD_ENTITY;
const INITIALIZE_BORDER: i32 = kiln_data::packets::play::clientbound::INITIALIZE_BORDER;

fn id_of(p: &[u8]) -> i32 {
    Reader::new(p).varint().unwrap()
}

#[test]
fn max_world_size_is_the_borders_absolute_maximum() {
    let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for size in [1000, 29_999_984] {
        let mut config = SimConfig::new(4, 2, None);
        config.server.max_world_size = size;
        let mut sim = Sim::new(config);
        let (msg, stats) = testing::join(1, "Alice", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg]));
        let log = stats.log.lock().unwrap().clone().unwrap();
        let pkt = log.iter().find(|p| id_of(p) == INITIALIZE_BORDER).expect("the border was sent");
        let mut r = Reader::new(pkt);
        r.varint().unwrap();
        for _ in 0..4 {
            r.f64().unwrap();
        }
        r.varlong().unwrap();
        assert_eq!(r.varint().unwrap(), size, "absolute max size in Initialize Border");
    }
}

/// How many `Add Entity` packets for cows a player at the origin got with the broadcast percentage at `percent`.
fn cows_seen(percent: i32) -> usize {
    let mut config = SimConfig::new(4, 2, None);
    config.server.entity_broadcast_range_percentage = percent;
    let mut sim = Sim::new(config);
    let (msg, stats) = testing::join(1, "Alice", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let mut client = Client::new(1, stats.clone());
    assert!(sim.step([msg]));
    for _ in 0..30 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let p = client.pos;
    // 20 blocks away: inside the view distance (2 chunks) and a cow's tracking range, outside 10% of it.
    sim.console_now(&format!("summon minecraft:cow {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] + 20.0, p[1], p[2]));
    for _ in 0..10 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let log = stats.log.lock().unwrap().clone().unwrap();
    log.iter().filter(|p| id_of(p) == ADD_ENTITY).count()
}

#[test]
fn entity_broadcast_range_percentage_scales_what_players_see() {
    let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // (The player's own entity is not sent to itself; other players are not here: the cow is the one Add Entity.)
    assert_eq!(cows_seen(100), 1, "a cow 20 blocks away is seen at 100%");
    assert_eq!(cows_seen(10), 0, "and not at 10% of its range (16 blocks)");
    assert_eq!(cows_seen(100), 1);
}

#[test]
fn force_gamemode_puts_returning_players_in_the_default_mode() {
    let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wp86-force-gamemode");
    let _ = std::fs::remove_dir_all(&dir);
    let play = |force: bool, default: u8, then: Option<&str>| -> u8 {
        let mut config = SimConfig::new(4, 2, Some(dir.clone()));
        config.server.default_game_mode = Some(default);
        config.server.force_gamemode = force;
        let mut sim = Sim::new(config);
        let (msg, _stats) = testing::join(1, "Alice", 2);
        assert!(sim.step([msg]));
        if let Some(command) = then {
            sim.console_now(command);
        }
        let mode = sim.game_mode(1 as ConnId).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(!sim.step([ToSim::Shutdown { done: tx }]));
        rx.recv().unwrap();
        mode
    };
    assert_eq!(play(false, 1, Some("gamemode survival Alice")), 0, "the first visit takes the default and then the command");
    assert_eq!(play(false, 2, None), 0, "the saved mode stays when the default changes");
    assert_eq!(play(true, 2, None), 2, "force-gamemode: the default again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn op_permission_level_is_what_op_grants() {
    let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = SimConfig::new(4, 2, None);
    config.server.op_permission_level = 3;
    let mut sim = Sim::new(config);
    sim.console_now("op Boss");
    assert_eq!(sim.permission_level_of_name("Boss"), 3);
    sim.console_now("deop Boss");
    assert_eq!(sim.permission_level_of_name("Boss"), 0);
}
