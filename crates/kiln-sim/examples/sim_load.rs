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
//!
//! `KILN_SLOW_PRINT=<ms>` prints the measured ticks slower than that with their phases;
//! `KILN_SAMPLE=1` (Windows) samples every thread's stack while measuring and prints the
//! functions by self and total time (`KILN_SAMPLE_US` interval, `KILN_SAMPLE_TOP` rows,
//! `KILN_SAMPLE_FILTER` keeps the stacks through a function whose name contains it).

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(windows)]
#[path = "support/sampler.rs"]
mod sampler;

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
    /// The kinds the mobs cycle through (`--kinds fox,piglin`; default: eight common ones).
    kinds: Vec<String>,
    /// Beds, workstations and a bell around each group's centre, so villagers have points of
    /// interest to claim and walk to.
    village: bool,
    /// `time set` this many ticks when the mobs come (villager schedules: 2000 work, 9000 meet, 13000 rest).
    day_time: Option<i64>,
    /// Players that crouch, go spectator and leave and rejoin while measuring (waypoint and
    /// tracking churn); for comparing packet streams between builds.
    churn: bool,
    /// `--entity-ticking serial|islands|tiles` and `--locator-interval n` (see `SimConfig`).
    entity_ticking: Option<kiln_sim::EntityTicking>,
    locator_interval: u32,
    /// `--prewake-us n`: workers spin this long from each tick's start.
    prewake_us: u64,
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
        kinds: Vec::new(),
        village: false,
        day_time: None,
        churn: false,
        entity_ticking: None,
        locator_interval: 1,
        prewake_us: 0,
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
            "--village" => a.village = true,
            "--churn" => a.churn = true,
            "--entity-ticking" => a.entity_ticking = Some(kiln_sim::EntityTicking::parse(&value()).expect("serial, islands or tiles")),
            "--locator-interval" => a.locator_interval = value().parse().unwrap(),
            "--prewake-us" => a.prewake_us = value().parse().unwrap(),
            "--day-time" => a.day_time = Some(value().parse().unwrap()),
            "--kinds" => a.kinds = value().split(',').map(str::to_owned).collect(),
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
    if let Some(t) = a.entity_ticking {
        config.entity_ticking = t;
    }
    config.locator_interval = a.locator_interval;
    config.prewake = std::time::Duration::from_micros(a.prewake_us);
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
    let mut churn = kiln_sim::testing::Churn::new(a.players);
    let mut cpu0 = None;
    #[cfg(windows)]
    let mut sampling: Option<sampler::Sampler> = None;
    let mut wall0 = Instant::now();
    let mut last_totals: Vec<(&str, std::time::Duration)> = Vec::new();
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
        if a.churn && let Some(since) = measuring_since {
            churn.tick(tick - since, &mut walkers, &mut inbox, a.groups, a.spacing, a.view_distance, SURFACE_Y);
        }
        let start = Instant::now();
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        let elapsed = start.elapsed().as_secs_f64() * 1e3;
        tick += 1;
        match measuring_since {
            None if warmup_done(&walkers) => {
                measuring_since = Some(tick);
                const DEFAULT_KINDS: [&str; 8] = ["rabbit", "fox", "cat", "ocelot", "zombie", "piglin", "hoglin", "wolf"];
                let kinds: Vec<String> = if a.kinds.is_empty() { DEFAULT_KINDS.iter().map(|k| k.to_string()).collect() } else { a.kinds.clone() };
                if let Some(t) = a.day_time {
                    inbox.push(kiln_link::ToSim::Console(format!("time set {t}")));
                }
                if a.village {
                    for g in 0..a.groups {
                        let [ox, oz] = group_offset(g, a.groups, a.spacing);
                        let (cx, cz) = ((8.5 + ox) as i32, (8.5 + oz) as i32);
                        let y = SURFACE_Y as i32;
                        let mut set = |dx: i32, dz: i32, block: &str| {
                            inbox.push(kiln_link::ToSim::Console(format!("setblock {} {y} {} {block}", cx + dx, cz + dz)));
                        };
                        // Eight beds (head to the north of the foot) in a row, job sites in another.
                        for b in 0..8 {
                            set(-8 + 2 * b, 5, "minecraft:red_bed[facing=north,part=foot]");
                            set(-8 + 2 * b, 4, "minecraft:red_bed[facing=north,part=head]");
                        }
                        const JOBS: [&str; 10] =
                            ["composter", "lectern", "barrel", "blast_furnace", "smoker", "cartography_table", "brewing_stand", "grindstone", "loom", "fletching_table"];
                        for (j, job) in JOBS.iter().enumerate() {
                            set(-9 + 2 * j as i32, -6, &format!("minecraft:{job}"));
                        }
                        set(0, 0, "minecraft:bell[facing=north,attachment=floor]");
                    }
                }
                for i in 0..a.mobs {
                    let [ox, oz] = group_offset(i % a.groups, a.groups, a.spacing);
                    let ang = i as f64 * 2.399;
                    let r = 3.0 + (i % 9) as f64;
                    inbox.push(kiln_link::ToSim::Console(format!(
                        "summon minecraft:{} {} {SURFACE_Y} {} {{PersistenceRequired:1b}}",
                        kinds[i % kinds.len()],
                        8.5 + ox + r * ang.cos(),
                        8.5 + oz + r * ang.sin()
                    )));
                }
                cpu0 = cpu::now();
                sim.reset_pool_stats();
                sim.reset_phase_totals();
                kiln_entity::prof::start();
                #[cfg(windows)]
                {
                    sampling = sampler::Sampler::start();
                }
                wall0 = Instant::now();
                packets0 = walkers.iter().map(|w| w.client.stats.packets.load(Relaxed)).sum();
                bytes0 = walkers.iter().map(|w| w.client.stats.bytes.load(Relaxed)).sum();
            }
            Some(since) => {
                times.push(elapsed);
                // The phases' time this tick (the totals' change), for the slow ones.
                let totals = sim.phase_totals();
                if std::env::var_os("KILN_SLOW_PRINT").is_some_and(|v| v.to_str().and_then(|v| v.parse::<f64>().ok()).is_some_and(|ms| elapsed > ms)) {
                    let phases: Vec<String> = totals
                        .iter()
                        .map(|(n, d)| (n, d.saturating_sub(last_totals.iter().find(|(m, _)| m == n).map_or(std::time::Duration::ZERO, |(_, d)| *d))))
                        .filter(|(_, d)| d.as_secs_f64() >= 2e-4)
                        .map(|(n, d)| format!("{n} {:.2}", d.as_secs_f64() * 1e3))
                        .collect();
                    eprintln!("slow tick {} (measured tick {}): {elapsed:.1} ms: {}", tick, tick - since, phases.join(" | "));
                }
                last_totals = totals;
                if tick - since >= a.ticks {
                    break;
                }
            }
            None => assert!(tick < 20 * 600, "players did not settle"),
        }
    }
    let cpu1 = cpu::now();
    #[cfg(windows)]
    let sampling = sampling.take();
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
    // What lives in the world at the end (natural spawns included), most common first.
    let mut census: std::collections::BTreeMap<&str, usize> = Default::default();
    for (kind, _) in sim.entities() {
        *census.entry(kind.trim_start_matches("minecraft:")).or_default() += 1;
    }
    let mut census: Vec<_> = census.into_iter().collect();
    census.sort_by_key(|&(k, n)| (std::cmp::Reverse(n), k));
    let total: usize = census.iter().map(|(_, n)| n).sum();
    let top: Vec<String> = census.iter().take(10).map(|(k, n)| format!("{k} {n}")).collect();
    println!("entities at the end: {total} ({})", top.join(", "));
    if std::env::var_os("KILN_SINK_DIGEST").is_some() {
        println!("packet stream digest {:016x} ({} players)", churn.stream_digest(&walkers), walkers.len());
    }
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
    let phases: Vec<String> = sim.phase_totals().iter().map(|(n, d)| format!("{n} {:.3}", d.as_secs_f64() * 1e3 / t)).collect();
    println!("phases ms/tick: {}", phases.join(" | "));
    kiln_entity::prof::report(times.len() as u64);
    #[cfg(windows)]
    if let Some(s) = sampling {
        s.report();
    }
    if let Some(r) = sim.last_report() {
        println!("last window: {r}");
    }
}
