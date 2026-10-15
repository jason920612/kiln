//! wp66: when the `#minecraft:tick` functions run in a game tick. `ServerFunctionManager.tick` is the first thing of
//! `MinecraftServer.tickChildren`: before the levels tick (so before the overworld's game time goes up by one and the
//! scheduled functions of `/schedule` come due), and only while the game runs (not frozen, unless stepping).
//! Found by a function that logged the game time of each golem a farm spawned: Kiln read one tick more than vanilla.

use kiln_link::ToSim;
use kiln_sim::{Sim, SimConfig};

fn world(tag: &str, tick_function: &str) -> (Sim, std::path::PathBuf) {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("functions-tick-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("datapacks/t");
    std::fs::create_dir_all(pack.join("data/t/function")).unwrap();
    std::fs::create_dir_all(pack.join("data/minecraft/tags/function")).unwrap();
    std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"t","min_format":121,"max_format":121}}"#).unwrap();
    std::fs::write(pack.join("data/minecraft/tags/function/tick.json"), r#"{"values":["t:tick"]}"#).unwrap();
    std::fs::write(pack.join("data/t/function/tick.mcfunction"), tick_function).unwrap();
    let sim = Sim::new(SimConfig::new(4, 2, Some(dir.clone())));
    (sim, dir)
}

fn stone_at(sim: &Sim) -> bool {
    sim.block_at(0, 80, 0) == Some(kiln_data::blocks::default_state::DIAMOND_BLOCK)
}

/// A tick function sees the game time as it was before the tick: the tick in which it sees `T` ends with the game time
/// `T + 1`. (The chunk is loaded in the 60 ticks it takes to get there.)
#[test]
fn a_tick_function_runs_before_the_game_time_goes_up() {
    let (mut sim, dir) = world("order", "execute store result score #g t run time query gametime\nexecute if score #g t matches 60 run setblock 0 80 0 minecraft:diamond_block\n");
    assert!(sim.step([ToSim::Console("scoreboard objectives add t dummy".into()), ToSim::Console("forceload add 0 0".into())]));
    for _ in 0..80 {
        assert!(sim.step([]));
        if stone_at(&sim) {
            break;
        }
    }
    assert!(stone_at(&sim), "the function saw game time 60");
    assert_eq!(sim.game_time(), 61, "it ran in the tick that makes the game time 61, before the increment");
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

/// A frozen game runs no tick functions (`ServerFunctionManager.tick` returns unless `runsNormally`), and a stepped tick does.
#[test]
fn a_frozen_game_runs_no_tick_functions_but_a_step_does() {
    let (mut sim, dir) = world("frozen", "scoreboard players add #n t 1
execute if score #n t matches 1.. run setblock 0 80 0 minecraft:diamond_block
");
    assert!(sim.step([ToSim::Console("scoreboard objectives add t dummy".into()), ToSim::Console("forceload add 0 0".into())]));
    // Let the chunk load, then freeze (the tick of the command itself may still run them), and start counting from nothing.
    for _ in 0..30 {
        assert!(sim.step([]));
    }
    assert!(sim.step([ToSim::Console("setblock 0 80 0 minecraft:air".into()), ToSim::Console("tick freeze".into())]));
    for _ in 0..3 {
        assert!(sim.step([]));
    }
    assert!(sim.step([ToSim::Console("setblock 0 80 0 minecraft:air".into()), ToSim::Console("scoreboard players set #n t 0".into())]));
    for _ in 0..40 {
        assert!(sim.step([]));
    }
    assert!(!stone_at(&sim), "no function ran in 40 frozen ticks");
    // One stepped tick runs them once.
    assert!(sim.step([ToSim::Console("tick step 1".into())]));
    for _ in 0..3 {
        assert!(sim.step([]));
    }
    assert!(stone_at(&sim), "a stepped tick runs them");
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}
