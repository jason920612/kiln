//! Tick cost of busy container block entities: `cargo test --release -p kiln-sim --test
//! container_bench -- --ignored --nocapture`. 400 hopper columns (a chest of 64 items feeding two
//! hoppers into a chest) and 100 burning furnaces around a player on a superflat world; prints
//! the mean and worst tick time with and without them.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::{Duration, Instant};

fn run(columns: i32, furnaces: i32, ticks: usize) -> (Duration, Duration) {
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
    let p = client.pos.map(|c| c.floor() as i32);
    let mut cmds = vec![ToSim::Console("gamemode creative Bench".into())];
    let side = (columns as f64).sqrt().ceil() as i32;
    for i in 0..columns {
        let (x, z) = (p[0] - side + 2 * (i % side), p[2] - side + 2 * (i / side));
        let y = p[1];
        cmds.push(ToSim::Console(format!("setblock {x} {y} {z} minecraft:chest")));
        cmds.push(ToSim::Console(format!("setblock {x} {} {z} minecraft:hopper[facing=down]", y + 1)));
        cmds.push(ToSim::Console(format!("setblock {x} {} {z} minecraft:hopper[facing=down]", y + 2)));
        cmds.push(ToSim::Console(format!("setblock {x} {} {z} minecraft:chest{{Items:[{{Slot:0b,id:\"minecraft:cobblestone\",count:64}}]}}", y + 3)));
    }
    for i in 0..furnaces {
        let (x, z) = (p[0] - 10 + i % 20, p[2] + side + 3 + i / 20);
        cmds.push(ToSim::Console(format!(
            "setblock {x} {} {z} minecraft:furnace{{Items:[{{Slot:0b,id:\"minecraft:raw_iron\",count:64}},{{Slot:1b,id:\"minecraft:coal\",count:64}}]}}",
            p[1]
        )));
    }
    step(&mut sim, &mut client, cmds);
    for _ in 0..20 {
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
    (total / ticks as u32, worst)
}

#[test]
#[ignore]
fn container_tick_cost() {
    let (base_mean, base_worst) = run(0, 0, 200);
    let (mean, worst) = run(400, 100, 200);
    println!("empty: mean {:.3} ms, worst {:.3} ms", base_mean.as_secs_f64() * 1e3, base_worst.as_secs_f64() * 1e3);
    println!("400 hopper columns + 100 furnaces: mean {:.3} ms, worst {:.3} ms", mean.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
}
