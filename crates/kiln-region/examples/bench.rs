//! Merge and split cost with realistic part sizes.
//!
//! `cargo run --release -p kiln-region --example bench [samples]`
//!
//! Two blocks of cells, each half of a region, with entities (tick list), messages (inbox)
//! and counters spread over them in interleaved sequence order, so merge and split do a
//! full linear pass. Times whole `Regionizer::apply` calls, under mimalloc like the server.
//!
//! Reports wall time and, on Windows x86-64, the thread's own CPU time
//! (`QueryThreadCycleTime`), which excludes time spent descheduled: on a loaded machine
//! the wall-time tail measures the other processes.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use kiln_region::{
    CellPos, DefaultCells, Inbox, MaxCounters, MsgKey, RegionPolicy, Regionizer, Regions, TickList, TopologyDelta,
    TopologyEvent,
};
use std::time::Instant;

type Parts = (TickList<u64>, Inbox<u64>, MaxCounters<3>);
type World = Regions<[u64; 8], Parts>;

#[cfg(all(windows, target_arch = "x86_64"))]
mod cpu {
    use std::time::Instant;

    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }

    /// TSC cycles this thread has run.
    pub fn cycles() -> Option<u64> {
        let mut c = 0;
        // SAFETY: the pseudo-handle of the current thread and a valid out pointer.
        (unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) } != 0).then_some(c)
    }

    /// TSC cycles per microsecond.
    pub fn calibrate() -> f64 {
        // SAFETY: rdtsc has no preconditions on x86-64.
        let tsc = || unsafe { core::arch::x86_64::_rdtsc() };
        let (t0, c0) = (Instant::now(), tsc());
        while t0.elapsed().as_millis() < 200 {}
        (tsc() - c0) as f64 / (t0.elapsed().as_secs_f64() * 1e6)
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
mod cpu {
    pub fn cycles() -> Option<u64> {
        None
    }
    pub fn calibrate() -> f64 {
        1.0
    }
}

#[derive(Default)]
struct Samples {
    wall: Vec<f64>,
    cpu: Vec<f64>,
}

struct Timer {
    wall: Instant,
    cycles: Option<u64>,
}

fn start() -> Timer {
    Timer { cycles: cpu::cycles(), wall: Instant::now() }
}

impl Samples {
    fn record(&mut self, t: Timer, cycles_per_us: f64) {
        self.wall.push(t.wall.elapsed().as_secs_f64() * 1e6);
        if let (Some(a), Some(b)) = (t.cycles, cpu::cycles()) {
            self.cpu.push((b - a) as f64 / cycles_per_us);
        }
    }

    fn report(mut self, what: &str) {
        let q = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
            format!("p50 {:>7.1} p99 {:>7.1} max {:>7.1}", at(0.5), at(0.99), at(1.0))
        };
        let n = self.wall.len();
        let wall = q(&mut self.wall);
        let cpu = if self.cpu.is_empty() { "n/a".into() } else { q(&mut self.cpu) };
        println!("{what:<48} n={n:<5} cpu µs: {cpu}   wall µs: {wall}");
    }
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % n
    }
}

/// Cells of a `w`×`h` block with its left edge at `x0`.
fn block(x0: i32, w: i32, h: i32) -> Vec<CellPos> {
    (0..h).flat_map(|z| (0..w).map(move |x| CellPos::new(x0 + x, z))).collect()
}

fn occupy(rz: &mut Regionizer, world: &mut World, cells: &[CellPos], tick: &mut u64) {
    cells.iter().for_each(|&c| rz.push(TopologyEvent::Occupied(c)));
    *tick += 1;
    rz.apply(world, *tick, &mut DefaultCells);
}

/// Spreads entities and messages over `cells` (random cell per element, increasing seq).
fn populate(world: &mut World, cells: &[CellPos], entities: usize, messages: usize, rng: &mut Rng) {
    for seq in 0..entities as u64 {
        let c = cells[rng.below(cells.len() as u64) as usize];
        world.at_mut(c).unwrap().part_mut().0.insert(seq, c, seq);
    }
    for seq in 0..messages as u32 {
        let c = cells[rng.below(cells.len() as u64) as usize];
        let key = MsgKey { src_tick: 1, src_cell: c, seq };
        world.at_mut(c).unwrap().part_mut().1.push(key, c, seq as u64);
    }
}

fn label(w: i32, h: i32, entities: usize) -> String {
    format!("{w}x{h}+{w}x{h} cells ({} chunks), {}k ent", 2 * w * h * 64, entities / 1000)
}

/// Two w×h blocks two empty columns apart; occupying the gap merges them.
fn bench_merge(w: i32, h: i32, entities: usize, samples: usize, cpu: f64) {
    let mut s = Samples::default();
    let mut rng = Rng(0x1234_5678);
    for _ in 0..samples {
        let mut rz = Regionizer::default();
        let mut world = World::new();
        let (a, b) = (block(0, w, h), block(w + 2, w, h));
        let mut tick = 0;
        occupy(&mut rz, &mut world, &a, &mut tick);
        occupy(&mut rz, &mut world, &b, &mut tick);
        assert_eq!(world.len(), 2);
        let all: Vec<CellPos> = a.iter().chain(&b).copied().collect();
        populate(&mut world, &all, entities, entities / 10, &mut rng);
        rz.push(TopologyEvent::Occupied(CellPos::new(w, 0)));
        tick += 1;
        let t = start();
        let deltas = rz.apply(&mut world, tick, &mut DefaultCells);
        s.record(t, cpu);
        assert!(matches!(deltas[0], TopologyDelta::Merged { .. }));
        assert_eq!(world.len(), 1);
    }
    s.report(&format!("merge {}", label(w, h, entities)));
}

/// One region of two w×h blocks joined by a bridge; the bridge is vacated and the apply
/// that performs the split is timed, as are the periodic checks before it.
fn bench_split(w: i32, h: i32, entities: usize, samples: usize, cpu: f64) {
    let mut split = Samples::default();
    let mut check = Samples::default();
    let mut rng = Rng(0x9abc_def0);
    for _ in 0..samples {
        let mut rz = Regionizer::default();
        let mut world = World::new();
        let (a, b) = (block(0, w, h), block(w + 3, w, h));
        let bridge = [CellPos::new(w, 0), CellPos::new(w + 1, 0), CellPos::new(w + 2, 0)];
        let mut tick = 0;
        let all: Vec<CellPos> = a.iter().chain(&b).chain(&bridge).copied().collect();
        occupy(&mut rz, &mut world, &all, &mut tick);
        assert_eq!(world.len(), 1);
        let halves: Vec<CellPos> = a.iter().chain(&b).copied().collect();
        populate(&mut world, &halves, entities, entities / 10, &mut rng);
        bridge.iter().for_each(|&c| rz.push(TopologyEvent::Vacated(c)));
        loop {
            tick += 1;
            let t = start();
            let deltas = rz.apply(&mut world, tick, &mut DefaultCells);
            if deltas.iter().any(|d| matches!(d, TopologyDelta::Split { .. })) {
                split.record(t, cpu);
                break;
            }
            if tick % rz.policy().split_period == 1 {
                check.record(t, cpu);
            }
        }
        assert_eq!(world.len(), 2);
    }
    split.report(&format!("split {}", label(w, h, entities)));
    check.report(&format!("  check without split, {}", label(w, h, entities)));
}

/// A player-like step: one new cell next to a large region, vacated again next tick.
fn bench_occupy(w: i32, h: i32, entities: usize, samples: usize, cpu: f64) {
    let mut rz = Regionizer::new(RegionPolicy::default());
    let mut world = World::new();
    let a = block(0, w, h);
    let mut tick = 0;
    occupy(&mut rz, &mut world, &a, &mut tick);
    populate(&mut world, &a, entities, 0, &mut Rng(5));
    let mut s = Samples::default();
    for i in 0..samples {
        let c = CellPos::new(-1, i as i32 % h);
        rz.push(TopologyEvent::Occupied(c));
        tick += 1;
        let t = start();
        rz.apply(&mut world, tick, &mut DefaultCells);
        s.record(t, cpu);
        rz.push(TopologyEvent::Vacated(c));
        tick += 1;
        rz.apply(&mut world, tick, &mut DefaultCells);
    }
    s.report(&format!("occupy next to {} cells, {}k ent", w * h, entities / 1000));
}

/// B0 cost when nothing changes.
fn bench_idle(regions: i32, samples: usize, cpu: f64) {
    let mut rz = Regionizer::default();
    let mut world = World::new();
    let cells: Vec<CellPos> = (0..regions).map(|i| CellPos::new(i * 3, 0)).collect();
    let mut tick = 0;
    occupy(&mut rz, &mut world, &cells, &mut tick);
    assert_eq!(world.len(), regions as usize);
    let mut s = Samples::default();
    for _ in 0..samples {
        tick += 1;
        let t = start();
        rz.apply(&mut world, tick, &mut DefaultCells);
        s.record(t, cpu);
    }
    s.report(&format!("idle apply, {regions} regions"));
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("warning: debug build (invariant checks after every apply); use --release");
    }
    let samples: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(500);
    let cpu = cpu::calibrate();
    println!("TSC {:.0} MHz; messages = entities / 10 in every case", cpu);
    // 8+8 cells = 1,024 chunks: the design's "≤ 1,000-chunk region" target.
    let cases = [(8, 1, 10_000), (4, 2, 10_000), (8, 8, 10_000), (8, 8, 100_000), (12, 12, 100_000)];
    for &(w, h, e) in &cases {
        bench_merge(w, h, e, samples, cpu);
    }
    for &(w, h, e) in &cases {
        bench_split(w, h, e, samples, cpu);
    }
    bench_occupy(16, 16, 10_000, samples, cpu);
    bench_idle(100, samples, cpu);
    bench_idle(1000, samples, cpu);
}
