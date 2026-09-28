//! Tick cost of many mobs: `cargo test --release -p kiln-sim --test mob_bench -- --ignored
//! --nocapture`. 500 mobs (the eight types, half monsters at night around a survival player,
//! half animals) on a superflat world; prints the mean and worst tick time with and without
//! them.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::{Duration, Instant};

fn run(mobs: usize, ticks: usize) -> (Duration, Duration, usize) {
    let mut sim = Sim::new(SimConfig::new(8, 6, None));
    let (msg, stats) = join(1, "Bench", 6);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    let step = |sim: &mut Sim, client: &mut Client, extra: Vec<ToSim>| {
        let mut inbox = extra;
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    };
    for _ in 0..20 {
        step(&mut sim, &mut client, Vec::new());
    }
    let setup = ["gamerule minecraft:spawn_mobs false", "time set 18000", "gamemode creative Bench"];
    step(&mut sim, &mut client, setup.iter().map(|c| ToSim::Console((*c).into())).collect());
    let kinds = ["zombie", "skeleton", "creeper", "spider", "pig", "cow", "sheep", "chicken"];
    let p = client.pos;
    let mut cmds = Vec::new();
    for i in 0..mobs {
        let (a, r) = (i as f64 * 2.399, 6.0 + (i % 40) as f64);
        let (x, z) = (p[0] + r * a.cos(), p[2] + r * a.sin());
        cmds.push(ToSim::Console(format!("summon minecraft:{} {x} {} {z} {{PersistenceRequired:1b}}", kinds[i % kinds.len()], p[1])));
    }
    step(&mut sim, &mut client, cmds);
    for _ in 0..40 {
        step(&mut sim, &mut client, Vec::new());
    }
    let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
    for _ in 0..ticks {
        let t = Instant::now();
        step(&mut sim, &mut client, Vec::new());
        let d = t.elapsed();
        total += d;
        worst = worst.max(d);
    }
    (total / ticks as u32, worst, sim.mobs().len())
}

#[test]
#[ignore = "benchmark"]
fn five_hundred_mobs() {
    let (base, base_worst, _) = run(0, 200);
    let (with, with_worst, n) = run(500, 200);
    eprintln!("no mobs:  mean {base:?}, worst {base_worst:?}");
    eprintln!("{n} mobs: mean {with:?}, worst {with_worst:?}");
    eprintln!("per mob:  {:?}", (with.saturating_sub(base)) / n.max(1) as u32);
}
