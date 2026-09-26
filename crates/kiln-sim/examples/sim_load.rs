//! Synthetic load without networking: scripted players join the simulation in-process, are
//! teleported to their group's centre and walk around it; reports the simulation's own tick
//! cost. Deterministic for a given set of arguments.
//!
//! usage: cargo run --release -p kiln-sim --example sim_load -- [--players 1000] [--groups 20]
//!        [--spacing 48] [--radius 6] [--ticks 1200] [--view-distance 2] [--behavior crowd|walk]

use kiln_sim::testing::{Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

const SURFACE_Y: f64 = -60.0;

struct Args {
    players: usize,
    groups: usize,
    spacing: f64,
    radius: f64,
    ticks: usize,
    view_distance: u8,
    walk: bool,
    threads: usize,
    unified: bool,
}

fn args() -> Args {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut a = Args {
        players: 1000,
        groups: 20,
        spacing: 48.0,
        radius: 6.0,
        ticks: 1200,
        view_distance: 2,
        walk: false,
        threads: cores.saturating_sub(1).clamp(1, 7),
        unified: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--players" => a.players = value().parse().unwrap(),
            "--groups" => a.groups = value().parse().unwrap(),
            "--spacing" => a.spacing = value().parse().unwrap(),
            "--radius" => a.radius = value().parse().unwrap(),
            "--ticks" => a.ticks = value().parse().unwrap(),
            "--view-distance" => a.view_distance = value().parse().unwrap(),
            "--behavior" => a.walk = value() == "walk",
            "--threads" => a.threads = value().parse().unwrap(),
            "--unified" => a.unified = true,
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1]
}

fn main() {
    let a = args();
    let mut config = SimConfig::new(a.players, 10, None);
    config.pool.workers = a.threads;
    config.unified_regions = a.unified;
    let mut sim = Sim::new(config);
    let mut walkers: Vec<Walker> = Vec::with_capacity(a.players);
    let mut inbox = Vec::new();
    // Joins at 100 per second, like the network bots.
    let joins_per_tick = 5;
    let mut tick = 0usize;
    let mut times = Vec::with_capacity(a.ticks);
    let (mut packets0, mut bytes0) = (0, 0);
    let warmup_done = |w: &[Walker]| w.len() == a.players && w.iter().all(|w| w.client.settled());
    let mut measuring_since: Option<usize> = None;
    loop {
        for _ in 0..joins_per_tick {
            let i = walkers.len();
            if i == a.players {
                break;
            }
            let name = format!("W{i}");
            let (msg, stats) = join(i as u64 + 1, &name, a.view_distance);
            inbox.push(msg);
            let [ox, oz] = group_offset(i % a.groups, a.groups, a.spacing);
            let center = [8.5 + ox, 8.5 + oz];
            inbox.push(kiln_link::ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", center[0], center[1])));
            walkers.push(Walker::new(Client::new(i as u64 + 1, stats), center, i as u64 + 1));
        }
        for w in &mut walkers {
            w.tick(a.radius, a.walk, &mut inbox);
        }
        let start = Instant::now();
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        let elapsed = start.elapsed().as_secs_f64() * 1e3;
        tick += 1;
        match measuring_since {
            None if warmup_done(&walkers) => {
                measuring_since = Some(tick);
                packets0 = walkers.iter().map(|w| w.client.stats.packets.load(Relaxed)).sum();
                bytes0 = walkers.iter().map(|w| w.client.stats.bytes.load(Relaxed)).sum();
            }
            Some(since) => {
                times.push(elapsed);
                if tick - since >= a.ticks {
                    break;
                }
            }
            None => assert!(tick < 20 * 600, "players did not settle"),
        }
    }
    let n = times.len() as f64;
    let packets: u64 = walkers.iter().map(|w| w.client.stats.packets.load(Relaxed)).sum();
    let bytes: u64 = walkers.iter().map(|w| w.client.stats.bytes.load(Relaxed)).sum();
    let disconnected = walkers.iter().filter(|w| w.client.stats.disconnected.load(Relaxed)).count();
    let mean = times.iter().sum::<f64>() / n;
    times.sort_by(f64::total_cmp);
    println!(
        "{} players in {} groups ({} warmup ticks), {} measured ticks: mspt mean {mean:.3} p50 {:.3} p99 {:.3} max {:.3}",
        a.players,
        a.groups,
        measuring_since.unwrap_or(0),
        times.len(),
        percentile(&times, 0.5),
        percentile(&times, 0.99),
        times.last().copied().unwrap_or(0.0),
    );
    println!(
        "sent {:.0} packets/tick, {:.1} kB/tick; disconnected {disconnected}; state hash {:016x}",
        (packets - packets0) as f64 / n,
        (bytes - bytes0) as f64 / n / 1e3,
        sim.state_hash()
    );
    if let Some(r) = sim.last_report() {
        println!("last window: {r}");
    }
}
