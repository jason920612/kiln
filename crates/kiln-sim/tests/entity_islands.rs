//! A region's entities tick in parallel islands or tiles (`entities/islands.rs`): the result
//! must not depend on the workers, the scheduling or how the world splits into regions. Each
//! scenario summons a crowd of mobs around a few players and compares every tick's state hash
//! and every player's packet stream (with `KILN_DATAPACK`, the packet and byte counts: the
//! streams then carry advancement dates from the clock).

use kiln_sim::testing::{Client, Walker, group_offset, join, stream_digest};
use kiln_sim::{Sim, SimConfig};
use std::sync::atomic::Ordering::Relaxed;

const SURFACE_Y: f64 = -60.0;
const PLAYERS: usize = 12;
const GROUPS: usize = 4;
const MOBS: usize = 96;
const KINDS: [&str; 8] = ["pig", "cow", "zombie", "rabbit", "fox", "goat", "frog", "wolf"];

/// Per-tick state hashes, the players' stream digest, and their packet and byte counts.
fn run(spacing: f64, workers: usize, chaos: Option<u64>, unified: bool) -> (Vec<u64>, u64, Vec<(u64, u64)>) {
    kiln_sim::testing::hash_packets();
    let mut config = SimConfig::new(PLAYERS, 4, None);
    config.keep_alive = false;
    config.pool.workers = workers;
    config.pool.chaos = chaos;
    config.unified_regions = unified;
    let mut sim = Sim::new(config);
    let mut walkers: Vec<Walker> = Vec::new();
    let mut inbox = Vec::new();
    for i in 0..PLAYERS {
        let name = format!("P{i}");
        let (msg, stats) = join(i as u64 + 1, &name, 4);
        inbox.push(msg);
        let [ox, oz] = group_offset(i % GROUPS, GROUPS, spacing);
        let center = [8.5 + ox, 8.5 + oz];
        inbox.push(kiln_link::ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", center[0], center[1])));
        walkers.push(Walker::new(Client::new(i as u64 + 1, stats), center, i as u64 + 1));
    }
    let mut hashes = Vec::new();
    let mut summoned = false;
    for tick in 0..400 {
        for w in &mut walkers {
            w.tick(5.0, true, &mut inbox);
        }
        if !summoned && walkers.iter().all(|w| w.client.settled()) {
            summoned = true;
            inbox.push(kiln_link::ToSim::Console("time set 14000".into()));
            for i in 0..MOBS {
                let [ox, oz] = group_offset(i % GROUPS, GROUPS, spacing);
                let (a, r) = (i as f64 * 2.399, 2.0 + (i % 7) as f64);
                inbox.push(kiln_link::ToSim::Console(format!(
                    "summon minecraft:{} {} {SURFACE_Y} {} {{PersistenceRequired:1b}}",
                    KINDS[i % KINDS.len()],
                    8.5 + ox + r * a.cos(),
                    8.5 + oz + r * a.sin()
                )));
            }
        }
        assert!(sim.step(inbox.drain(..)));
        if summoned {
            hashes.push(sim.state_hash());
        }
        assert!(tick < 399 || summoned, "players did not settle");
    }
    let counts = walkers.iter().map(|w| (w.client.stats.packets.load(Relaxed), w.client.stats.bytes.load(Relaxed))).collect();
    (hashes, stream_digest(&walkers, &[]), counts)
}

fn check(spacing: f64) {
    let reference = run(spacing, 1, None, true);
    assert!(reference.0.len() > 200, "the mobs ticked");
    for (workers, chaos, unified) in [(4, Some(3), true), (7, Some(11), true), (7, Some(5), false)] {
        let r = run(spacing, workers, chaos, unified);
        assert_eq!(r.0, reference.0, "state hashes, {workers} workers, chaos {chaos:?}, unified {unified}");
        assert_eq!(r.2, reference.2, "packet counts, {workers} workers, chaos {chaos:?}, unified {unified}");
        if std::env::var_os("KILN_DATAPACK").is_none() {
            assert_eq!(r.1, reference.1, "packet streams, {workers} workers, chaos {chaos:?}, unified {unified}");
        }
    }
}

/// Groups far enough apart to tick as islands.
#[test]
fn islands_do_not_depend_on_the_workers() {
    check(80.0);
}

/// One crowd: the entities tick in tiles.
#[test]
fn tiles_do_not_depend_on_the_workers() {
    check(16.0);
}
