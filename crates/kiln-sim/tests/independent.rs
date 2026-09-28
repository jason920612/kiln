//! Independent scheduling (design REG-02): a region slowed down on purpose leaves the
//! lockstep, the other regions keep 20 TPS, and when the slow region comes back its
//! scheduled ticks are still due after the same number of its own ticks. Not deterministic
//! (wall time decides how many server ticks a slow region misses), so these tests check
//! rates and invariants, never state hashes.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, Walker, group_offset, join};
use kiln_sim::{InjectedDelay, ScheduleMode, Sim, SimConfig};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

const PER_GROUP: usize = 6;
const GROUPS: usize = 2;
/// Far enough apart that each group gets its own region.
const SPACING: f64 = 1024.0;
const SURFACE_Y: f64 = -60.0;
const SLOW: Duration = Duration::from_millis(100);
const TICK: Duration = Duration::from_millis(50);

fn center(g: usize) -> [f64; 2] {
    let [ox, oz] = group_offset(g, GROUPS, SPACING);
    [8.5 + ox, 8.5 + oz]
}

struct World {
    sim: Sim,
    walkers: Vec<Walker>,
    inbox: Vec<ToSim>,
}

impl World {
    /// Players in both groups; group 0's region sleeps [`SLOW`] in each of its ticks.
    fn new(schedule: ScheduleMode) -> World {
        let mut config = SimConfig::new(PER_GROUP * GROUPS, 4, None);
        config.pool.workers = 4;
        config.schedule = schedule;
        let [x, z] = center(0);
        config.inject_delay = Some(InjectedDelay { dimension: "minecraft:overworld".into(), x: x as i32, z: z as i32, delay: SLOW });
        let mut w = World { sim: Sim::new(config), walkers: Vec::new(), inbox: Vec::new() };
        for i in 0..PER_GROUP * GROUPS {
            let conn = i as u64 + 1;
            let name = format!("P{i}");
            let (msg, stats) = join(conn, &name, 2);
            let c = center(i % GROUPS);
            w.inbox.push(msg);
            w.inbox.push(ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", c[0], c[1])));
            w.walkers.push(Walker::new(Client::new(conn, stats), c, conn));
        }
        // Everyone joined, was placed and confirmed the teleport.
        for _ in 0..200 {
            w.step();
            if w.walkers.iter().all(|w| w.client.settled()) {
                break;
            }
        }
        assert!(w.walkers.iter().all(|w| w.client.settled()), "players did not settle");
        w
    }

    fn step(&mut self) -> Duration {
        for w in &mut self.walkers {
            w.tick(4.0, true, &mut self.inbox);
        }
        let start = Instant::now();
        assert!(self.sim.step(self.inbox.drain(..)), "simulation stopped");
        start.elapsed()
    }

    fn own_ticks(&self, g: usize) -> u64 {
        let [x, z] = center(g);
        self.sim.local_tick_at("minecraft:overworld", x as i32, z as i32).expect("region")
    }

    fn packets(&self, g: usize) -> u64 {
        self.walkers.iter().skip(g).step_by(GROUPS).map(|w| w.client.stats.packets.load(Relaxed)).sum()
    }
}

/// The slow region costs every lockstep tick 100 ms; in independent mode the other region
/// ticks every 50 ms (20 TPS) while the slow one ticks about every 100 ms on its own.
#[test]
fn a_slow_region_does_not_hold_back_the_others() {
    let mut lockstep = World::new(ScheduleMode::Lockstep);
    let slow: Vec<Duration> = (0..5).map(|_| lockstep.step()).collect();
    assert!(slow.iter().all(|d| *d >= SLOW), "the injected delay slows every lockstep tick: {slow:?}");

    let mut w = World::new(ScheduleMode::Independent);
    // Let the slow region's first ticks mark it as too slow, and the chunks around the
    // players load (a cell getting its first chunk changes the topology: a rendezvous).
    for _ in 0..30 {
        w.step();
    }
    let (fast0, slow0, packets0) = (w.own_ticks(1), w.own_ticks(0), w.packets(1));
    let rendezvous0 = w.sim.independent_stats().0;
    // Twenty ticks at the server's cadence, like `kiln_sim::run`.
    let ticks = 40;
    let start = Instant::now();
    let mut next = start;
    let mut longest = Duration::ZERO;
    for _ in 0..ticks {
        longest = longest.max(w.step());
        next += TICK;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let elapsed = start.elapsed();
    assert!(w.sim.regions_away() <= 1);
    let fast = w.own_ticks(1) - fast0;
    assert_eq!(fast, ticks, "the fast region ticks every server tick");
    let met = w.sim.independent_stats().0 - rendezvous0;
    assert_eq!(met, 0, "nothing needed the whole server");
    // A tick that waited for the slow region would take its 100 ms (a loaded machine can
    // stretch a normal tick past 50 ms now and then).
    assert!(longest < SLOW, "no server tick waited for the slow region (longest {longest:?})");
    assert!(elapsed < TICK * (ticks as u32 + 10), "20 TPS held: {ticks} ticks took {elapsed:?}");
    assert!(w.packets(1) > packets0 + fast * PER_GROUP as u64, "the fast region's players are served every tick");
    w.sim.rendezvous();
    assert_eq!(w.sim.regions_away(), 0);
    let slow = w.own_ticks(0) - slow0;
    // About one tick per 100 ms of wall time (plus the one in flight).
    let expected = elapsed.as_secs_f64() / SLOW.as_secs_f64();
    assert!((slow as f64) <= expected + 2.0 && slow as f64 >= expected * 0.6, "slow region ticked {slow} times in {elapsed:?}");
    // Everyone is still there and connected.
    assert_eq!(w.sim.player_count(), PER_GROUP * GROUPS);
    assert!(w.walkers.iter().all(|w| !w.client.stats.disconnected.load(Relaxed)));
    let (rendezvous, _, lends) = w.sim.independent_stats();
    assert!(lends as f64 >= slow as f64 - 1.0, "the slow region ticked away ({lends} times, {rendezvous} rendezvous)");
}

/// Water placed in the slow region flows on the fifth of the region's own ticks, as in
/// lockstep, although the server ran many more ticks meanwhile: its scheduled ticks shift
/// by the ticks it missed each time it comes back.
#[test]
fn scheduled_ticks_keep_their_delay_in_own_ticks() {
    for schedule in [ScheduleMode::Lockstep, ScheduleMode::Independent] {
        let mut w = World::new(schedule);
        for _ in 0..3 {
            w.step();
        }
        let [cx, cz] = center(0);
        let (x, y, z) = (cx as i32 + 3, SURFACE_Y as i32, cz as i32 + 3);
        let water = |w: &World| w.sim.block_at(x + 1, y, z).is_some_and(kiln_data::blocks_types::has_fluid);
        w.sim.rendezvous();
        let before = w.own_ticks(0);
        w.inbox.push(ToSim::Console(format!("setblock {x} {y} {z} minecraft:water")));
        w.step();
        let mut server_ticks = 1;
        // Free running up to the second own tick (the server runs far ahead meanwhile), then
        // one own tick per server tick with a look after each.
        while w.own_ticks(0) < before + 2 {
            w.step();
            server_ticks += 1;
        }
        loop {
            w.sim.rendezvous();
            let n = w.own_ticks(0) - before;
            assert_eq!(water(&w), n >= 5, "{schedule:?}: after {n} own ticks ({server_ticks} server ticks)");
            if n >= 5 {
                break;
            }
            w.step();
            server_ticks += 1;
        }
        if schedule == ScheduleMode::Independent {
            assert!(server_ticks > 5, "the server ran ahead of the slow region ({server_ticks} server ticks)");
        }
    }
}

/// Console commands, chat and leaving need the whole server: they wait for the slow region,
/// then run as in lockstep; broadcasts reach its players.
#[test]
fn server_wide_actions_meet_the_slow_region() {
    let mut w = World::new(ScheduleMode::Independent);
    for _ in 0..5 {
        w.step();
    }
    let packets0 = w.packets(0);
    // A player of the slow region moves with a command.
    let [cx, cz] = center(0);
    w.inbox.push(ToSim::Console(format!("tp P0 {} {SURFACE_Y} {}", cx + 2.0, cz)));
    w.inbox.push(ToSim::Console("say hello everyone".into()));
    w.step();
    for _ in 0..10 {
        w.step();
    }
    w.sim.rendezvous();
    assert!(w.packets(0) > packets0, "the slow region's players got packets");
    let (rendezvous, _, _) = w.sim.independent_stats();
    assert!(rendezvous >= 2, "{rendezvous} rendezvous");
    assert_eq!(w.sim.player_count(), PER_GROUP * GROUPS);
    // A player of the slow region leaves.
    w.inbox.push(ToSim::Leave(1));
    w.step();
    w.sim.rendezvous();
    assert_eq!(w.sim.player_count(), PER_GROUP * GROUPS - 1);
}
