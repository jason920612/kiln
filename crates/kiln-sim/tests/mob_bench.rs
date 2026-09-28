//! Tick cost of many mobs: `cargo test --release -p kiln-sim --test mob_bench -- --ignored
//! --nocapture`. 500 mobs (the eight types, half monsters at night around a survival player,
//! half animals) on a superflat world; prints the mean and worst tick time with and without
//! them.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::{Duration, Instant};

fn run(mobs: usize, ticks: usize, kinds: &[&str]) -> (Duration, Duration, usize) {
    run_with(mobs, ticks, kinds, &[])
}

/// `run`, with console commands after the mobs are summoned (effects for everyone).
fn run_with(mobs: usize, ticks: usize, kinds: &[&str], after: &[&str]) -> (Duration, Duration, usize) {
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
    let p = client.pos;
    let mut cmds = Vec::new();
    for i in 0..mobs {
        let (a, r) = (i as f64 * 2.399, 6.0 + (i % 40) as f64);
        let (x, z) = (p[0] + r * a.cos(), p[2] + r * a.sin());
        cmds.push(ToSim::Console(format!("summon {} {x} {} {z} {{PersistenceRequired:1b}}", kinds[i % kinds.len()], p[1])));
    }
    step(&mut sim, &mut client, cmds);
    step(&mut sim, &mut client, after.iter().map(|c| ToSim::Console((*c).into())).collect());
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
    let (base, base_worst, _) = run(0, 200, &FIRST);
    let (with, with_worst, n) = run(500, 200, &FIRST);
    eprintln!("no mobs:  mean {base:?}, worst {base_worst:?}");
    eprintln!("{n} mobs: mean {with:?}, worst {with_worst:?}");
    eprintln!("per mob:  {:?}", (with.saturating_sub(base)) / n.max(1) as u32);
}

const FIRST: [&str; 8] = [
    "minecraft:zombie",
    "minecraft:skeleton",
    "minecraft:creeper",
    "minecraft:spider",
    "minecraft:pig",
    "minecraft:cow",
    "minecraft:sheep",
    "minecraft:chicken",
];

/// 500 mobs of every type Kiln simulates, round robin.
#[test]
#[ignore = "benchmark"]
fn five_hundred_mixed_mobs() {
    let all: Vec<&str> = kiln_entity::mob::ALL_KINDS.iter().map(|k| k.type_name()).collect();
    let (base, base_worst, _) = run(0, 200, &all);
    let (with, with_worst, n) = run(500, 200, &all);
    eprintln!("no mobs:  mean {base:?}, worst {base_worst:?}");
    eprintln!("{n} mobs ({} types): mean {with:?}, worst {with_worst:?}", all.len());
    eprintln!("per mob:  {:?}", (with.saturating_sub(base)) / n.max(1) as u32);
}

/// The same mix, every mob with three effects (speed, regeneration, poison: attribute
/// modifiers, heal and hurt ticks, entity data).
#[test]
#[ignore = "benchmark"]
fn five_hundred_mixed_mobs_with_effects() {
    let all: Vec<&str> = kiln_entity::mob::ALL_KINDS.iter().map(|k| k.type_name()).collect();
    let effects = [
        "effect give @e[type=!minecraft:player] minecraft:speed 1000 1",
        "effect give @e[type=!minecraft:player] minecraft:regeneration 1000 0",
        "effect give @e[type=!minecraft:player] minecraft:poison 1000 0",
    ];
    let (base, _, _) = run(0, 200, &all);
    let (plain, plain_worst, n0) = run(500, 200, &all);
    let (with, with_worst, n) = run_with(500, 200, &all, &effects);
    eprintln!("no mobs:  mean {base:?}");
    eprintln!("{n0} mobs: mean {plain:?}, worst {plain_worst:?}");
    eprintln!("{n} mobs with 3 effects each: mean {with:?}, worst {with_worst:?}");
    eprintln!("per mob:  {:?} without effects, {:?} with", plain.saturating_sub(base) / n0.max(1) as u32, with.saturating_sub(base) / n.max(1) as u32);
}
