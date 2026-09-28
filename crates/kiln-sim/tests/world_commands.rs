//! The level-wide commands' effects: the world border hurts players outside it and is saved
//! with the level, `/tick freeze` and `step` stop and run the game clock, `/forceload` keeps
//! a chunk loaded with nobody near and saves it.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const OVERWORLD: &str = "minecraft:overworld";

fn settle(sim: &mut Sim, client: &mut Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

fn joined(world: Option<std::path::PathBuf>) -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(4, 4, world));
    let (msg, stats) = join(1, "Walker", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode survival Walker".into())]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    (sim, client)
}

#[test]
fn players_outside_the_border_take_damage() {
    let (mut sim, mut client) = joined(None);
    assert_eq!(sim.health(1), Some((20.0, false)));
    // The player stands near 8, 8: a 10 block border around 100, 100 is ~87 blocks away,
    // 82 past the 5 block buffer: floor(82 * 0.2) = 16 damage.
    assert!(sim.step([ToSim::Console("worldborder center 100 100".into()), ToSim::Console("worldborder set 10".into())]));
    settle(&mut sim, &mut client, 1);
    let (health, _) = sim.health(1).unwrap();
    assert!(health <= 4.0, "hurt by the border: {health}");
    settle(&mut sim, &mut client, 30);
    assert!(sim.health(1).unwrap().1, "the border kills");
}

#[test]
fn inside_the_buffer_is_safe() {
    let (mut sim, mut client) = joined(None);
    let [x, _, z] = client.pos;
    // The border's edge 3 blocks away: within the 5 block buffer.
    let edge = x.floor() - 3.0;
    let size = 2.0 * (x.floor() - edge);
    let cmds = [format!("worldborder center {} {}", edge - size / 2.0, z.floor()), format!("worldborder set {size}")];
    assert!(sim.step(cmds.map(ToSim::Console)));
    settle(&mut sim, &mut client, 40);
    assert_eq!(sim.health(1).map(|h| h.0), Some(20.0));
}

#[test]
fn frozen_levels_keep_their_time_until_stepped() {
    let (mut sim, mut client) = joined(None);
    assert!(sim.step([ToSim::Console("tick freeze".into())]));
    settle(&mut sim, &mut client, 1);
    let frozen_at = sim.game_time();
    settle(&mut sim, &mut client, 10);
    assert_eq!(sim.game_time(), frozen_at);
    assert!(!sim.runs_normally());
    assert!(sim.step([ToSim::Console("tick step 5".into())]));
    settle(&mut sim, &mut client, 10);
    assert_eq!(sim.game_time(), frozen_at + 5, "five stepped ticks");
    assert!(sim.step([ToSim::Console("tick unfreeze".into())]));
    settle(&mut sim, &mut client, 3);
    // The unfreeze arrives after the tick rate manager ticked: the next three ticks run.
    assert_eq!(sim.game_time(), frozen_at + 5 + 3);
}

#[test]
fn border_and_forced_chunks_are_saved_with_the_level() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("world-commands-save");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    {
        let (mut sim, mut client) = joined(Some(dir.clone()));
        let cmds = ["worldborder set 1000", "worldborder set 500 100", "forceload add 400 400"].map(|c| ToSim::Console(c.into()));
        assert!(sim.step(cmds));
        settle(&mut sim, &mut client, 10);
        let (_, size, left) = sim.world_border_of(OVERWORLD).unwrap();
        // The border ticks in the step that set it, then ten more.
        assert_eq!(left, 89);
        assert_eq!(size, 1000.0 - 500.0 * 11.0 / 100.0);
        // The forced chunk stays loaded with nobody near.
        assert!(sim.block_at(400, -64, 400).is_some());
        let (done, _rx) = std::sync::mpsc::channel();
        assert!(!sim.step([ToSim::Shutdown { done }]));
    }
    let sim = Sim::new(SimConfig::new(4, 4, Some(dir.clone())));
    let (_, size, left) = sim.world_border_of(OVERWORLD).unwrap();
    assert_eq!((size, left), (945.0, 89), "the move carries on");
    assert_eq!(sim.forced_chunks_of(OVERWORLD), vec![[25, 25]]);
}
