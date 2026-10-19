//! wp87: world clocks and timelines of data packs. The test pack `tools/wp87/packs/kiln87_clock` has a clock of its own (`kiln87:short`), a
//! timeline that follows it (period 1000 ticks, a marker, tracks for `monsters_burn`, `creaking_active` and `sky_light_level`) and a
//! superflat dimension whose type has that clock as its default clock and that timeline: the clock runs by itself, `/time` finds it, the
//! level's attributes follow it and not the overworld's, and the clients are told its state.
//!
//! The levels and registries are process-wide: every test takes `live::exclusive()`.

mod support;

use bytes::Bytes;
use kiln_link::ToSim;
use kiln_proto::Reader;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;
use support::wp87_world;

const SHORT: &str = "kiln87:short";
const LEVEL: &str = "kiln87:short";

struct World {
    sim: Sim,
    client: Client,
    dir: PathBuf,
}

impl World {
    fn new(tag: &str) -> World {
        World::in_dir(wp87_world(tag, &["kiln87_clock"]))
    }

    fn in_dir(dir: PathBuf) -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, Some(dir.clone())));
        let (msg, stats) = join(1, "Keeper", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
        let mut w = World { sim, client: Client::new(1, stats), dir };
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        self.sim.console_now(command);
    }

    fn clock(&self) -> i64 {
        self.sim.clock_ticks(SHORT).expect("the pack's clock")
    }

    fn log(&mut self) -> Vec<Bytes> {
        std::mem::take(self.client.stats.log.lock().unwrap().as_mut().unwrap())
    }

    fn shutdown(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(!self.sim.step([ToSim::Shutdown { done: tx }]));
        rx.recv().unwrap();
        self.dir = PathBuf::new();
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The clocks of the last Set Time packet: `(clock id, time, rate)`.
fn set_time(log: &[Bytes]) -> Option<Vec<(i32, i64, f32)>> {
    let p = log.iter().rev().find(|p| Reader::new(p).varint().ok() == Some(kiln_data::packets::play::clientbound::SET_TIME))?;
    let mut r = Reader::new(p);
    r.varint().ok()?;
    r.i64().ok()?;
    let n = r.varint().ok()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let (clock, time) = (r.varint().ok()?, r.varlong().ok()?);
        let _fraction = r.f32().ok()?;
        out.push((clock, time, r.f32().ok()?));
    }
    Some(out)
}

#[test]
fn a_clock_of_a_pack_runs_by_itself_and_answers_time() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("runs");
    let overworld = w.sim.day_time();
    w.run(&format!("time of {SHORT} set 250"));
    assert_eq!(w.clock(), 250);
    w.ticks(10);
    assert_eq!(w.clock(), 260, "it runs one tick a tick");
    assert_eq!(w.sim.day_time(), overworld + 10, "and the overworld's has its own count");
    w.run(&format!("time of {SHORT} pause"));
    w.ticks(10);
    assert_eq!(w.clock(), 260, "paused");
    w.run(&format!("time of {SHORT} resume"));
    w.run(&format!("time of {SHORT} rate 2"));
    w.ticks(10);
    assert_eq!(w.clock(), 280, "twice as fast");
    // The default clock of a command run in the level is its type's.
    w.run(&format!("execute in {LEVEL} run time set 100"));
    assert_eq!(w.clock(), 100);
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn the_markers_and_timelines_of_a_clock_are_those_of_its_timelines() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("markers");
    w.run(&format!("time of {SHORT} pause"));
    w.run(&format!("time of {SHORT} set 260"));
    w.run(&format!("time of {SHORT} set kiln87:peak"));
    assert_eq!(w.clock(), 500, "the next moment the marker names");
    w.run(&format!("time of {SHORT} set 650"));
    w.run(&format!("time of {SHORT} set kiln87:peak"));
    assert_eq!(w.clock(), 1500, "the marker repeats every 1000 ticks, as its timeline does");
    // A marker of another clock's timeline is not found.
    let before = w.clock();
    w.run(&format!("time of {SHORT} set minecraft:noon"));
    assert_eq!(w.clock(), before);
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn the_level_of_the_pack_follows_the_clock_of_the_pack() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("follows");
    w.run("time pause");
    w.run("time set noon");
    w.run(&format!("time of {SHORT} pause"));
    // Monsters burn in the level while its clock is in the stretch the timeline sets.
    w.run(&format!("time of {SHORT} set 300"));
    assert_eq!(w.sim.timeline_bool_in(LEVEL, "minecraft:gameplay/monsters_burn"), Some(false));
    w.run(&format!("time of {SHORT} set 700"));
    assert_eq!(w.sim.timeline_bool_in(LEVEL, "minecraft:gameplay/monsters_burn"), Some(true));
    assert_eq!(w.sim.timeline_bool_in(LEVEL, "minecraft:gameplay/creaking_active"), Some(false));
    w.run(&format!("time of {SHORT} set 300"));
    assert_eq!(w.sim.timeline_bool_in(LEVEL, "minecraft:gameplay/creaking_active"), Some(true));
    // The overworld, at noon, follows its own clock.
    assert_eq!(w.sim.timeline_bool_in("minecraft:overworld", "minecraft:gameplay/monsters_burn"), Some(true));
    // The sky light level is 15 times the track: 15 at 0, 3 at 500, 9 at 250 and at 750.
    for (clock, darken) in [(0, 0), (500, 12), (250, 6), (750, 6)] {
        w.run(&format!("time of {SHORT} set {clock}"));
        assert_eq!(w.sim.sky_darken_in(LEVEL), Some(darken), "sky darkness at {clock}");
    }
    // A level without a day timeline has no sky darkness at all, as before.
    assert_eq!(w.sim.sky_darken_in("minecraft:the_end"), Some(0));
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn clients_are_told_the_clocks_of_the_pack_and_the_world_keeps_them() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("told");
    w.run(&format!("time of {SHORT} set 333"));
    w.ticks(2);
    let log = w.log();
    let clocks = set_time(&log).expect("a Set Time packet");
    let id = kiln_data::synced_id("minecraft:world_clock", SHORT).unwrap();
    assert_eq!(clocks.len(), 3, "the overworld's, the End's and the pack's: {clocks:?}");
    let ours = clocks.iter().find(|c| c.0 == id).expect("the pack's clock");
    assert_eq!((ours.1, ours.2), (333, 1.0));
    // Paused, it is told with rate 0.
    w.run(&format!("time of {SHORT} pause"));
    let log = w.log();
    assert_eq!(set_time(&log).unwrap().iter().find(|c| c.0 == id).map(|c| c.2), Some(0.0));
    // The world keeps the clock (and its pause) across a restart.
    w.run(&format!("time of {SHORT} set 777"));
    let dir = w.dir.clone();
    w.shutdown();
    drop(w);
    let again = World::in_dir(dir);
    assert_eq!(again.clock(), 777, "after a restart");
    drop(again);
    kiln_data::levels::reset();
}
