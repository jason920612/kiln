//! Chunk loading from a saved world in the running simulation: scripted players are
//! teleported across the world a few blocks per tick, so chunks load (and unload, saving)
//! continuously; the storage logs its per-load latency and file opens every 2000 loads, and
//! the tick time is reported at the end. Run it on an Anvil world and on its native
//! conversion (`kiln world convert`) to compare.
//!
//! usage: cargo run --release -p kiln-sim --example sim_storage -- <world> [--players 4]
//!        [--ticks 1200] [--speed 8] [--view-distance 10] [--extent 2048]
//! Players start on lines across the square (0,0)..(extent, extent) and wrap around it.

use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::Instant;

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new("info")).init();
    let mut args = std::env::args().skip(1);
    let world = std::path::PathBuf::from(args.next().expect("world directory"));
    let (mut players, mut ticks, mut speed, mut vd, mut extent) = (4usize, 1200usize, 8.0f64, 10u8, 2048.0f64);
    while let Some(flag) = args.next() {
        let v = args.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--players" => players = v.parse().unwrap(),
            "--ticks" => ticks = v.parse().unwrap(),
            "--speed" => speed = v.parse().unwrap(),
            "--view-distance" => vd = v.parse().unwrap(),
            "--extent" => extent = v.parse().unwrap(),
            other => panic!("unknown argument {other}"),
        }
    }
    let mut config = SimConfig::new(players, vd, Some(world));
    config.simulation_distance = 4;
    let mut sim = Sim::new(config);
    let mut clients = Vec::new();
    let mut inbox = Vec::new();
    for i in 0..players {
        let (msg, stats) = join(i as u64 + 1, &format!("S{i}"), vd);
        inbox.push(msg);
        clients.push(Client::new(i as u64 + 1, stats));
    }
    let lane = |i: usize| extent * (i as f64 + 0.5) / players as f64;
    let mut times = Vec::with_capacity(ticks);
    let start = Instant::now();
    for tick in 0..ticks {
        for (i, c) in clients.iter_mut().enumerate() {
            let x = (64.0 + tick as f64 * speed) % extent;
            inbox.push(kiln_link::ToSim::Console(format!("tp S{i} {x:.1} 180 {:.1}", lane(i))));
            c.tick(None, &mut inbox);
        }
        let t = Instant::now();
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let wall = start.elapsed().as_secs_f64();
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    times.sort_by(f64::total_cmp);
    let p = |q: f64| times[((times.len() as f64 * q) as usize).min(times.len() - 1)];
    println!(
        "{players} players at {speed} blocks/tick, view distance {vd}, {ticks} ticks in {wall:.1} s: mspt mean {mean:.2} p50 {:.2} p99 {:.2} max {:.2}",
        p(0.5),
        p(0.99),
        times.last().unwrap()
    );
    let t = Instant::now();
    let (done, _wait) = std::sync::mpsc::channel();
    sim.step([kiln_link::ToSim::Shutdown { done }]);
    println!("shutdown save {:.1} s", t.elapsed().as_secs_f64());
}
