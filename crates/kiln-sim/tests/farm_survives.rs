//! wp66: an observer + piston sugar cane farm must keep working however the world around it changes: chunks that
//! unload and load again, regions that merge and split, a forced chunk nobody stands near. (One run in fourteen on a
//! loaded machine had none of its observers fire for good while `hasScheduledTick` would have told them a tick was
//! pending.) Every window of the run must see the observers fire.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct Farm {
    sim: Sim,
    clients: Vec<Client>,
    /// The farm's observers.
    observers: Vec<[i32; 3]>,
}

fn observer_powered(sim: &Sim, p: [i32; 3]) -> Option<bool> {
    let s = sim.block_at(p[0], p[1], p[2])?;
    kiln_blocks::state::is(s, kiln_data::blocks::default_state::OBSERVER).then(|| kiln_blocks::state::get_bool(s, "powered"))
}

impl Farm {
    fn new(dir: Option<std::path::PathBuf>, players: &[(&str, i32)]) -> Farm {
        let mut sim = Sim::new(SimConfig::new(8, 4, dir));
        let mut clients = Vec::new();
        for (i, (name, _)) in players.iter().enumerate() {
            let (msg, stats) = join(i as u64 + 1, name, 2);
            assert!(sim.step([msg, ToSim::Console(format!("gamemode creative {name}")), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
            clients.push(Client::new(i as u64 + 1, stats));
        }
        let mut f = Farm { sim, clients, observers: Vec::new() };
        f.ticks(5);
        f
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            for c in self.clients.iter_mut() {
                c.tick(None, &mut inbox);
            }
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, cmds: &[String]) {
        assert!(self.sim.step(cmds.iter().map(|c| ToSim::Console(c.clone()))));
    }

    /// `units` observer farms in a row along z starting at `(x0, y, z0)` (the wp65 layout: sand, water, cane, piston, observer
    /// looking down, a stone and a dust that carry its signal to the piston).
    fn build(&mut self, x0: i32, y: i32, z0: i32, units: i32) {
        let mut cmds = vec![format!("forceload add {} {} {} {}", x0 - 4, z0 - 4, x0 + 8, z0 + 4 * units + 4)];
        self.run(&cmds);
        self.ticks(30);
        cmds.clear();
        for i in 0..units {
            let z = z0 + 4 * i;
            for (dx, dy, dz, b) in [
                (0, 0, 0, "sand"),
                (1, 0, 0, "water"),
                (1, 1, 0, "stone"),
                (2, 0, 0, "stone"),
                (1, 0, -1, "stone"),
                (1, 0, 1, "stone"),
                (0, 1, 0, "sugar_cane"),
                (-1, 2, 0, "piston[facing=east]"),
                (0, 3, 0, "observer[facing=down]"),
                (0, 4, 0, "stone"),
                (-1, 3, 0, "stone"),
                (-1, 4, 0, "redstone_wire"),
            ] {
                cmds.push(format!("setblock {} {} {} minecraft:{b}", x0 + dx, y + dy, z + dz));
            }
            self.observers.push([x0, y + 3, z]);
        }
        self.run(&cmds);
        self.run(&["gamerule minecraft:random_tick_speed 2000".to_owned()]);
    }

    /// How many observer pulses (a tick with the observer powered) there were in the next `n` ticks.
    fn pulses(&mut self, n: usize) -> usize {
        let mut count = 0;
        for _ in 0..n {
            self.ticks(1);
            count += self.observers.clone().iter().filter(|&&p| observer_powered(&self.sim, p) == Some(true)).count();
        }
        count
    }
}

/// The farm sits next to a player: the baseline every other test compares with.
#[test]
fn a_farm_next_to_a_player_pulses() {
    let mut f = Farm::new(None, &[("Alpha", 0)]);
    let p = f.clients[0].pos.map(|c| c.floor() as i32);
    f.build(p[0] + 10, p[1] - 1, p[2] + 6, 4);
    let n = f.pulses(300);
    assert!(n >= 8 * 2, "{n} observer pulses in 300 ticks");
}

/// Unloading and loading the farm's chunks again and again (the player goes far and comes back): each time it is back it
/// works again within a few hundred ticks, and its pending ticks were not lost on the way.
#[test]
fn a_farm_works_after_its_chunks_unloaded_and_came_back_again() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("farm-unload");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = Farm::new(Some(dir), &[("Alpha", 0)]);
    let p = f.clients[0].pos.map(|c| c.floor() as i32);
    // (A forced chunk would stay; the farm's chunks are only near the player.)
    f.build(p[0] + 10, p[1] - 1, p[2] + 6, 4);
    f.run(&["forceload remove all".to_owned()]);
    assert!(f.pulses(300) >= 16, "it works at first");
    for round in 0..4 {
        // Away until the chunks are gone (the check runs every 20 ticks), then back, at different moments each round.
        f.ticks(round * 3);
        f.run(&[format!("tp Alpha {} {} {}", p[0] + 5000, p[1], p[2])]);
        f.ticks(60);
        assert!(f.sim.block_at(p[0] + 10, p[1] + 2, p[2] + 6).is_none(), "round {round}: the farm's chunk should have unloaded");
        f.run(&[format!("tp Alpha {} {} {}", p[0], p[1], p[2])]);
        let n = f.pulses(400);
        assert!(n >= 16, "round {round}: {n} observer pulses in 400 ticks after the chunks came back");
    }
}

/// Another player far away joins the farm's region (they merge when the other comes near) and leaves again (the region splits):
/// the observers keep firing through both.
#[test]
fn a_farm_works_through_region_merges_and_splits() {
    let mut f = Farm::new(None, &[("Alpha", 0), ("Beta", 0)]);
    let p = f.clients[0].pos.map(|c| c.floor() as i32);
    f.run(&[format!("tp Beta {} {} {}", p[0] + 600, p[1], p[2] + 600)]);
    f.ticks(10);
    f.build(p[0] + 10, p[1] - 1, p[2] + 6, 4);
    assert!(f.pulses(200) >= 16, "it works at first");
    for round in 0..3 {
        // Beta walks over (a merge) and back out of reach (a split).
        f.run(&[format!("tp Beta {} {} {}", p[0] + 30, p[1], p[2] + 30)]);
        let merged = f.pulses(200);
        assert!(merged >= 12, "round {round}: {merged} pulses after the regions merged");
        f.run(&[format!("tp Beta {} {} {}", p[0] + 600, p[1], p[2] + 600)]);
        let split = f.pulses(200);
        assert!(split >= 12, "round {round}: {split} pulses after the regions split");
    }
}

/// A forced chunk with nobody near: the farm in it keeps running.
#[test]
fn a_farm_in_a_forced_chunk_runs_without_players_near() {
    let mut f = Farm::new(None, &[("Alpha", 0)]);
    let p = f.clients[0].pos.map(|c| c.floor() as i32);
    f.build(p[0] + 400, p[1] - 1, p[2] + 400, 4);
    f.ticks(30);
    assert!(f.pulses(400) >= 16, "the forced farm works");
}

/// Nobody online: a forced farm still runs (vanilla keeps ticking force-loaded chunks without players), and it is still
/// there when somebody joins again.
#[test]
fn a_forced_farm_runs_while_nobody_is_online() {
    let mut f = Farm::new(None, &[("Alpha", 0), ("Beta", 0)]);
    let p = f.clients[0].pos.map(|c| c.floor() as i32);
    f.build(p[0] + 10, p[1] - 1, p[2] + 6, 4);
    assert!(f.pulses(100) >= 4, "it works at first");
    // Both leave.
    assert!(f.sim.step([ToSim::Leave(1), ToSim::Leave(2)]));
    f.clients.clear();
    assert_eq!(f.sim.player_count(), 0);
    let mut empty = 0;
    for _ in 0..400 {
        assert!(f.sim.step([]));
        empty += f.observers.clone().iter().filter(|&&q| observer_powered(&f.sim, q) == Some(true)).count();
    }
    assert!(empty >= 16, "{empty} observer pulses in 400 ticks with nobody online");
}
