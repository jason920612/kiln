//! Fork-join and window overhead of the tick pool.
//!
//! `cargo run -p kiln-sched --release --example forkjoin -- [--workers 7] [--iters 10000]
//! [--cold 1000] [--idle-ms 50] [--spin-us 50]`
//!
//! Fork: `run_units` over N empty units. `total` is the whole call, `wake` the time from the
//! call to the start of the last unit to begin, `join` from the end of the last unit to finish
//! until the call returned. Hot iterations run back to back (workers still spinning); cold ones
//! follow `--idle-ms` of idle time, so the workers are parked as between ticks.
//! Window: `map_indexed` over trivial items against a plain inline map.

use std::hint::black_box;
use std::time::{Duration, Instant};

use kiln_sched::{PoolConfig, Strategy, TickPool, Window};

// Same allocator as kiln-server; the system heap adds page-fault noise to the window runs.
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct Args {
    workers: usize,
    iters: usize,
    cold: usize,
    idle: Duration,
    spin: Duration,
}

fn args() -> Args {
    let mut a = Args {
        workers: 7,
        iters: 10_000,
        cold: 1_000,
        idle: Duration::from_millis(50),
        spin: PoolConfig::new(1).spin,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    for pair in argv.chunks(2) {
        let v: u64 = pair.get(1).and_then(|v| v.parse().ok()).expect("flags take a number");
        match pair[0].as_str() {
            "--workers" => a.workers = v as usize,
            "--iters" => a.iters = v as usize,
            "--cold" => a.cold = v as usize,
            "--idle-ms" => a.idle = Duration::from_millis(v),
            "--spin-us" => a.spin = Duration::from_micros(v),
            f => panic!("unknown flag {f}"),
        }
    }
    a
}

#[derive(Clone, Copy)]
struct Unit {
    start: Instant,
    end: Instant,
}

#[derive(Default)]
struct Samples {
    total: Vec<u64>,
    wake: Vec<u64>,
    join: Vec<u64>,
}

fn fork(pool: &mut TickPool, units: &mut [Unit], s: &mut Samples) {
    let t0 = Instant::now();
    pool.run_units(
        units,
        |_| 0,
        |u, _| {
            u.start = Instant::now();
            u.end = Instant::now();
        },
    );
    let t1 = Instant::now();
    let last_start = units.iter().map(|u| u.start).max().unwrap();
    let last_end = units.iter().map(|u| u.end).max().unwrap();
    s.total.push((t1 - t0).as_nanos() as u64);
    s.wake.push((last_start - t0).as_nanos() as u64);
    s.join.push((t1 - last_end).as_nanos() as u64);
}

fn pct(v: &mut [u64]) -> String {
    v.sort_unstable();
    let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize] as f64 / 1000.0;
    format!("p50 {:7.2}  p99 {:7.2}  max {:8.2}", at(0.5), at(0.99), at(1.0))
}

fn report(label: &str, s: &mut Samples) {
    println!("  {label:<5} total µs: {}", pct(&mut s.total));
    println!("  {:<5} wake  µs: {}", "", pct(&mut s.wake));
    println!("  {:<5} join  µs: {}", "", pct(&mut s.join));
}

fn main() {
    let a = args();
    let mut cfg = PoolConfig::new(a.workers);
    cfg.spin = a.spin;
    let mut pool = TickPool::with_config(cfg);
    println!(
        "workers {} (incl. caller), spin {:?}, hot iters {}, cold iters {} after {:?} idle",
        a.workers, a.spin, a.iters, a.cold, a.idle
    );

    for n in [1usize, 7, 50] {
        let now = Instant::now();
        let mut units = vec![Unit { start: now, end: now }; n];
        let mut warm = Samples::default();
        for _ in 0..1_000 {
            fork(&mut pool, &mut units, &mut warm);
        }
        let mut hot = Samples::default();
        for _ in 0..a.iters {
            fork(&mut pool, &mut units, &mut hot);
        }
        let mut cold = Samples::default();
        for _ in 0..a.cold {
            std::thread::sleep(a.idle);
            fork(&mut pool, &mut units, &mut cold);
        }
        println!("run_units, {n} empty units");
        report("hot", &mut hot);
        if a.cold > 0 {
            report("cold", &mut cold);
        }
    }

    let per = |n: usize| (a.iters / (n / 1000).max(1)).clamp(100, 10_000);
    println!("map_indexed over n trivial items, per call (µs)");
    for n in [1_000usize, 10_000, 100_000] {
        let items: Vec<u64> = (0..n as u64).collect();
        let f = |x: &u64| x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (x >> 7);
        let iters = per(n);
        let time = |run: &mut dyn FnMut() -> Vec<u64>| {
            let mut v: Vec<u64> = (0..iters)
                .map(|_| {
                    let t = Instant::now();
                    black_box(run());
                    t.elapsed().as_nanos() as u64
                })
                .collect();
            pct(&mut v)
        };
        let inline = time(&mut || items.iter().map(f).collect());
        let auto = time(&mut || pool.map_indexed(&items, f));
        let chunk = n.div_ceil(4 * a.workers);
        let w = Window::new().strategy(Strategy::Parallel).chunk(chunk);
        let parallel = time(&mut || pool.serial(|c| c.map_indexed_with(w, &items, |_, x| f(x))));
        println!("  n {n:>6}  inline   {inline}");
        println!("  {:>8}  auto     {auto}", "");
        println!("  {:>8}  parallel {parallel}  ({} chunks of {chunk})", "", n.div_ceil(chunk));
    }

    // A parallel window right after idling: its helpers must be woken from park.
    let items: Vec<u64> = (0..28).collect();
    let w = Window::new().strategy(Strategy::Parallel).chunk(1);
    let mut cold_window: Vec<u64> = (0..a.cold.min(200))
        .map(|_| {
            std::thread::sleep(a.idle);
            let t = Instant::now();
            black_box(pool.serial(|c| c.map_indexed_with(w, &items, |_, &x| x)));
            t.elapsed().as_nanos() as u64
        })
        .collect();
    if !cold_window.is_empty() {
        println!("cold parallel window, 28 one-item chunks (µs): {}", pct(&mut cold_window));
    }

    let stats = pool.stats();
    let parked: Duration = stats.iter().map(|s| s.parked).sum();
    let working: Duration = stats.iter().map(|s| s.working).sum();
    println!("worker totals: working {working:?}, parked {parked:?}");
}
