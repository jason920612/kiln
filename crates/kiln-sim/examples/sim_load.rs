//! Synthetic load without networking: scripted players join the simulation in-process, are
//! teleported to their group's centre and walk around it; reports the simulation's own tick
//! cost. Deterministic for a given set of arguments.
//!
//! usage: cargo run --release -p kiln-sim --example sim_load -- [--players 1000] [--groups 20]
//!        [--spacing 48] [--radius 6] [--ticks 1200] [--view-distance 2] [--behavior crowd|walk]
//!        [--threads n] [--unified] [--inline] [--independent] [--slow-ms n]
//!        [--spin-us n] [--inline-below-us n] [--chunk-us n] [--helper-share-us n]
//!
//! Prints the process CPU time per measured tick next to the wall time: idle workers spinning
//! cost CPU without showing in mspt.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use kiln_sim::testing::{Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

const SURFACE_Y: f64 = -60.0;

/// Process CPU time, so the measured ticks' CPU cost can be compared (idle workers spinning
/// count here but not in the tick's wall time).
#[cfg(windows)]
mod cpu {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        lo: u32,
        hi: u32,
    }

    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(p: isize, c: *mut FileTime, e: *mut FileTime, k: *mut FileTime, u: *mut FileTime) -> i32;
        fn QueryProcessCycleTime(p: isize, cycles: *mut u64) -> i32;
    }

    /// Kernel plus user time in seconds, and TSC cycles, of the whole process.
    pub fn now() -> Option<(f64, u64)> {
        let (mut c, mut e, mut k, mut u) = Default::default();
        let mut cycles = 0;
        // SAFETY: the current-process pseudo-handle and valid out pointers.
        let ok = unsafe {
            GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) != 0
                && QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) != 0
        };
        let secs = |t: &FileTime| ((t.hi as u64) << 32 | t.lo as u64) as f64 * 1e-7;
        ok.then(|| (secs(&k) + secs(&u), cycles))
    }
}

#[cfg(not(windows))]
mod cpu {
    pub fn now() -> Option<(f64, u64)> {
        None
    }
}

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
    /// Every phase window inline (the serial baseline for crowd windows).
    inline: bool,
    /// Independent scheduling instead of lockstep.
    independent: bool,
    /// Milliseconds injected into each tick of group 0's region.
    slow_ms: u64,
    /// Pool tuning overrides in microseconds (idle spin, inline threshold, chunk target).
    spin_us: Option<u64>,
    inline_below_us: Option<u64>,
    chunk_us: Option<u64>,
    helper_share_us: Option<u64>,
    /// Mobs summoned around the groups' centres once everyone is in (rabbits, foxes, cats, ocelots,
    /// zombies, piglins, hoglins, wolves: the ones that look for players).
    mobs: usize,
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
        inline: false,
        independent: false,
        slow_ms: 0,
        spin_us: None,
        inline_below_us: None,
        chunk_us: None,
        helper_share_us: None,
        mobs: 0,
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
            "--inline" => a.inline = true,
            "--independent" => a.independent = true,
            "--slow-ms" => a.slow_ms = value().parse().unwrap(),
            "--spin-us" => a.spin_us = Some(value().parse().unwrap()),
            "--inline-below-us" => a.inline_below_us = Some(value().parse().unwrap()),
            "--chunk-us" => a.chunk_us = Some(value().parse().unwrap()),
            "--mobs" => a.mobs = value().parse().unwrap(),
            "--helper-share-us" => a.helper_share_us = Some(value().parse().unwrap()),
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
    let us = std::time::Duration::from_micros;
    if let Some(v) = a.spin_us {
        config.pool.spin = us(v);
    }
    if let Some(v) = a.inline_below_us {
        config.pool.inline_below = us(v);
    }
    if let Some(v) = a.chunk_us {
        config.pool.chunk_target = us(v);
    }
    if let Some(v) = a.helper_share_us {
        config.pool.helper_share = us(v);
    }
    if a.inline {
        config.pool.phase = kiln_sched::PhaseMode::Inline;
    }
    if a.independent {
        config.schedule = kiln_sim::ScheduleMode::Independent;
    }
    if a.slow_ms > 0 {
        let [ox, oz] = group_offset(0, a.groups, a.spacing);
        config.inject_delay = Some(kiln_sim::InjectedDelay {
            dimension: "minecraft:overworld".into(),
            x: (8.5 + ox) as i32,
            z: (8.5 + oz) as i32,
            delay: std::time::Duration::from_millis(a.slow_ms),
        });
    }
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
    let mut cpu0 = None;
    let mut wall0 = Instant::now();
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
                const KINDS: [&str; 8] = ["rabbit", "fox", "cat", "ocelot", "zombie", "piglin", "hoglin", "wolf"];
                for i in 0..a.mobs {
                    let [ox, oz] = group_offset(i % a.groups, a.groups, a.spacing);
                    let ang = i as f64 * 2.399;
                    let r = 3.0 + (i % 9) as f64;
                    inbox.push(kiln_link::ToSim::Console(format!(
                        "summon minecraft:{} {} {SURFACE_Y} {} {{PersistenceRequired:1b}}",
                        KINDS[i % KINDS.len()],
                        8.5 + ox + r * ang.cos(),
                        8.5 + oz + r * ang.sin()
                    )));
                }
                cpu0 = cpu::now();
                sim.reset_pool_stats();
                wall0 = Instant::now();
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
    let cpu1 = cpu::now();
    let wall = wall0.elapsed().as_secs_f64();
    // Regions ticking away (independent mode) come back before anything is counted.
    sim.rendezvous();
    if a.independent || a.slow_ms > 0 {
        let local = |g: usize| {
            let [ox, oz] = group_offset(g, a.groups, a.spacing);
            sim.local_tick_at("minecraft:overworld", (8.5 + ox) as i32, (8.5 + oz) as i32).unwrap_or(0)
        };
        let (rendezvous, waited, lends) = sim.independent_stats();
        println!(
            "own ticks: group 0 {}, last group {}; {rendezvous} rendezvous ({:.1} ms waited), {lends} ticks away",
            local(0),
            local(a.groups - 1),
            waited.as_secs_f64() * 1e3
        );
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
        "{} regions; sent {:.0} packets/tick, {:.1} kB/tick; disconnected {disconnected}; state hash {:016x}",
        sim.region_count(),
        (packets - packets0) as f64 / n,
        (bytes - bytes0) as f64 / n / 1e3,
        sim.state_hash()
    );
    if std::env::var_os("KILN_SINK_IDS").is_some() {
        let mut total: std::collections::BTreeMap<i32, (u64, u64)> = Default::default();
        for w in &walkers {
            for (id, (n, b)) in w.client.stats.by_id.lock().unwrap().iter() {
                let e = total.entry(*id).or_default();
                e.0 += n;
                e.1 += b;
            }
        }
        let mut rows: Vec<_> = total.into_iter().collect();
        rows.sort_by_key(|(_, (_, b))| std::cmp::Reverse(*b));
        for (id, (n, b)) in rows.iter().take(8) {
            println!("  packet {id:#04x}: {n} packets, {:.1} kB", *b as f64 / 1e3);
        }
    }
    if let (Some((s0, c0)), Some((s1, c1))) = (cpu0, cpu1) {
        let t = times.len() as f64;
        println!(
            "cpu {:.3} ms/tick ({:.2} Mcycles/tick), {:.2} cores busy over {:.1} s",
            (s1 - s0) * 1e3 / t,
            (c1 - c0) as f64 / 1e6 / t,
            (s1 - s0) / wall,
            wall
        );
    }
    let st = sim.pool_stats();
    let helpers = &st[1.min(st.len())..];
    let sum = |f: fn(&kiln_sched::WorkerStats) -> std::time::Duration| helpers.iter().map(f).sum::<std::time::Duration>();
    let t = times.len() as f64;
    println!(
        "helpers ({}): working {:.3} ms/tick, parked {:.3} ms/tick, {:.1} chunks/tick",
        helpers.len(),
        sum(|w| w.working).as_secs_f64() * 1e3 / t,
        sum(|w| w.parked).as_secs_f64() * 1e3 / t,
        helpers.iter().map(|w| w.chunks).sum::<u64>() as f64 / t,
    );
    if let Some(r) = sim.last_report() {
        println!("last window: {r}");
    }
}
