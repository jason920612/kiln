//! Tick cost of game event listeners: `cargo test --release -p kiln-sim --test sculk_bench --
//! --ignored --nocapture`. 300 wandering animals (their steps, landings and other game events)
//! on a superflat world, without and with 1024 sculk sensors on a 3-block grid around them;
//! prints the mean and worst tick time and the cost per sensor.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::{Duration, Instant};

fn run(sensors: bool, ticks: usize) -> (Duration, Duration, usize) {
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
    let setup = ["gamerule minecraft:spawn_mobs false", "gamemode creative Bench"];
    step(&mut sim, &mut client, setup.iter().map(|c| ToSim::Console((*c).into())).collect());
    let p = client.pos;
    let (x0, y, z0) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    let mut cmds = Vec::new();
    let mut placed = 0;
    if sensors {
        for i in 0..32 {
            for k in 0..32 {
                let (x, z) = (x0 - 48 + 3 * i, z0 - 48 + 3 * k);
                cmds.push(ToSim::Console(format!("setblock {x} {} {z} minecraft:sculk_sensor", y - 1)));
                placed += 1;
            }
        }
    }
    let kinds = ["minecraft:pig", "minecraft:cow", "minecraft:sheep", "minecraft:chicken"];
    for i in 0..300 {
        let (a, r) = (i as f64 * 2.399, 4.0 + (i % 40) as f64);
        let (x, z) = (p[0] + r * a.cos(), p[2] + r * a.sin());
        cmds.push(ToSim::Console(format!("summon {} {x} {} {z} {{PersistenceRequired:1b}}", kinds[i % kinds.len()], p[1])));
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
    (total / ticks as u32, worst, placed)
}

#[test]
#[ignore = "benchmark"]
fn a_thousand_sensors_under_a_herd() {
    let (base, base_worst, _) = run(false, 300);
    let (with, with_worst, n) = run(true, 300);
    eprintln!("300 animals, no sensors:     mean {base:?}, worst {base_worst:?}");
    eprintln!("300 animals, {n} sensors: mean {with:?}, worst {with_worst:?}");
    eprintln!("per sensor:                  {:?}", with.saturating_sub(base) / n.max(1) as u32);
}
