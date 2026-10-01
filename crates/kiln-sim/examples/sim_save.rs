//! Tick time while a native world is saved over and over: a player stands in a native-format
//! world, every tick a command rewrites blocks around them (so their chunks are dirty) and
//! `save-all` saves them on the tick thread. Cell files fill with stale records and compact;
//! `KILN_COMPACTION=inline` compacts on the tick thread (the old behaviour), the default on a
//! background thread. Run both and compare the tick times.
//!
//! usage: cargo run --release -p kiln-sim --example sim_save -- [ticks] [view-distance]

use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let ticks: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(400);
    let vd: u8 = args.next().and_then(|a| a.parse().ok()).unwrap_or(6);
    let world = std::env::temp_dir().join(format!("kiln-sim-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    std::fs::create_dir_all(&world).unwrap();
    let mut config = SimConfig::new(4, vd, Some(world.clone()));
    config.world_format = kiln_storage::WorldFormat::Native;
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Saver", vd);
    let mut client = Client::new(1, stats);
    assert!(sim.step([msg, kiln_link::ToSim::Console("gamemode creative Saver".into())]));
    let mut inbox = Vec::new();
    for _ in 0..40 {
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox.drain(..)));
    }
    let blocks = ["stone", "dirt", "cobblestone", "andesite", "granite", "diorite", "oak_planks", "gravel"];
    let mut times = Vec::with_capacity(ticks);
    for tick in 0..ticks {
        let b = blocks[tick % blocks.len()];
        // A layer of blocks over the loaded area, different from the last tick's.
        inbox.push(kiln_link::ToSim::Console(format!("fill ~-{r} 5 ~-{r} ~{r} {h} ~{r} {b}", r = (vd as i32 - 1) * 16, h = 6 + (tick % 20))));
        inbox.push(kiln_link::ToSim::Console("save-all".into()));
        client.tick(None, &mut inbox);
        let t = Instant::now();
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let over: usize = times.iter().filter(|&&t| t > 50.0).count();
    times.sort_by(f64::total_cmp);
    let p = |q: f64| times[((times.len() as f64 * q) as usize).min(times.len() - 1)];
    println!(
        "compaction {}: {ticks} ticks with a save each: mspt mean {mean:.2} p50 {:.2} p90 {:.2} p99 {:.2} max {:.2}; {over} ticks over 50 ms",
        std::env::var("KILN_COMPACTION").unwrap_or_else(|_| "background".into()),
        p(0.5),
        p(0.9),
        p(0.99),
        times.last().unwrap()
    );
    let (done, _wait) = std::sync::mpsc::channel();
    sim.step([kiln_link::ToSim::Shutdown { done }]);
    let _ = std::fs::remove_dir_all(&world);
}
