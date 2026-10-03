//! A crowd's packets are a function of its inputs: what the optimisations of the serial phases
//! (the locator bar, tracking) leave out must not change a byte a player receives. Each
//! scenario plays scripted walkers that crouch, go spectator, leave and rejoin, and compares
//! the state hash and one hash of every player's packet stream (keep-alives left out) with
//! the values the straightforward implementation produced (constants below, recorded before
//! the locator bar was reorganised; `cargo run --release -p kiln-sim --example sim_load`
//! with `--churn` and `KILN_SINK_DIGEST=1` prints the same numbers for the same arguments).

use kiln_sim::testing::{Churn, Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};

const SURFACE_Y: f64 = -60.0;

struct Scenario {
    players: usize,
    groups: usize,
    spacing: f64,
    walk: bool,
    ticks: usize,
}

/// Same script as `sim_load --churn` (5 joins per tick, then `ticks` measured ticks).
fn run(s: &Scenario) -> (u64, u64, usize) {
    kiln_sim::testing::hash_packets();
    let mut config = SimConfig::new(s.players, 10, None);
    config.keep_alive = false;
    config.pool.workers = 3;
    let mut sim = Sim::new(config);
    let mut walkers: Vec<Walker> = Vec::new();
    let mut inbox = Vec::new();
    let mut churn = Churn::new(s.players);
    let mut tick = 0usize;
    let mut since = None;
    loop {
        for _ in 0..5 {
            let i = walkers.len();
            if i == s.players {
                break;
            }
            let name = format!("W{i}");
            let (msg, stats) = join(i as u64 + 1, &name, 2);
            inbox.push(msg);
            let [ox, oz] = group_offset(i % s.groups, s.groups, s.spacing);
            let center = [8.5 + ox, 8.5 + oz];
            inbox.push(kiln_link::ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", center[0], center[1])));
            walkers.push(Walker::new(Client::new(i as u64 + 1, stats), center, i as u64 + 1));
        }
        for w in &mut walkers {
            w.tick(6.0, s.walk, &mut inbox);
        }
        if let Some(since) = since {
            churn.tick(tick - since, &mut walkers, &mut inbox, s.groups, s.spacing, 2, SURFACE_Y);
        }
        assert!(sim.step(inbox.drain(..)));
        tick += 1;
        match since {
            None if walkers.len() == s.players && walkers.iter().all(|w| w.client.settled()) => since = Some(tick),
            Some(t) if tick - t >= s.ticks => break,
            None => assert!(tick < 20 * 600, "players did not settle"),
            _ => {}
        }
    }
    (sim.state_hash(), churn.stream_digest(&walkers), sim.region_count())
}

fn check(s: Scenario, hash: u64, digest: u64, regions: usize) {
    let got = run(&s);
    assert_eq!(got.2, regions, "regions");
    assert_eq!(
        (format!("{:016x}", got.0), format!("{:016x}", got.1)),
        (format!("{hash:016x}"), format!("{digest:016x}")),
        "state hash and packet stream digest"
    );
}

/// One region, everyone close: block links.
#[test]
fn crowd_in_one_region() {
    check(Scenario { players: 150, groups: 4, spacing: 48.0, walk: false, ticks: 150 }, 0xe94274c461b259ab, 0x92401cec36661511, 1);
}

/// Groups far enough apart for chunk links, walking.
#[test]
fn groups_with_chunk_links() {
    check(Scenario { players: 80, groups: 4, spacing: 200.0, walk: true, ticks: 150 }, 0x9d96bd81102791fd, 0x2e2f7e87a454c5de, 1);
}

/// Groups past the locator bar's 332 blocks: azimuth links, one region each.
#[test]
fn groups_with_azimuth_links() {
    check(Scenario { players: 60, groups: 3, spacing: 1500.0, walk: true, ticks: 150 }, 0x16e1ef1b0c272be9, 0x537a8b0853c53dac, 3);
}
