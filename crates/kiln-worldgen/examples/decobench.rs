//! Single-threaded FEATURES cost per placed feature and structure, over a square of chunks
//! decorated in canonical order (terrain generated first, untimed).
//!
//! usage: cargo run --release -p kiln-worldgen --example decobench [-- <side> [seed]]

use kiln_worldgen::decorate::{Invocation, Observer};
use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::proto::ProtoChunk;
use kiln_worldgen::region::Region;
use kiln_worldgen::structure::{ChunkStarts, StartCache};
use kiln_worldgen::{Datapack, Worldgen, order};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Timer<'a> {
    world: &'a Worldgen,
    started: Option<Instant>,
    spent: HashMap<String, (Duration, u64)>,
}

impl Observer for Timer<'_> {
    fn before(&mut self, _inv: Invocation, _r: &mut Region) -> bool {
        self.started = Some(Instant::now());
        true
    }

    fn after(&mut self, inv: Invocation, _r: &mut Region) {
        let t = self.started.take().unwrap().elapsed();
        let name = match inv {
            Invocation::Feature { placed, .. } => self.world.decorator.features.placed[placed].name.clone(),
            Invocation::Structure { structure, .. } => format!("{} (structure)", self.world.structures.structures[structure].name),
        };
        let e = self.spent.entry(name).or_default();
        e.0 += t;
        e.1 += 1;
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let side: i32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(12);
    let seed: i64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(12345);
    let dir = PathBuf::from(std::env::var_os("KILN_WORK").expect("set KILN_WORK")).join("generated");
    let pack = Datapack::load(&dir).expect("load datapack");
    let world = Worldgen::overworld(&pack, seed, true).expect("worldgen");
    let targets: Vec<(i32, i32)> = (0..side * side).map(|i| (300 + i % side, -200 + i / side)).collect();
    let decorated = order::decoration_order(&targets);
    let mut gs = GenScratch::default();
    let mut chunks: HashMap<(i32, i32), Box<ProtoChunk>> = HashMap::new();
    for &(x, z) in &decorated {
        for dz in -1..=1 {
            for dx in -1..=1 {
                chunks.entry((x + dx, z + dz)).or_insert_with(|| Box::new(world.generator.generate(&mut gs, x + dx, z + dz)));
            }
        }
    }
    let cache = StartCache::default();
    let mut timer = Timer { world: &world, started: None, spent: HashMap::new() };
    let mut starts_time = Duration::ZERO;
    let start = Instant::now();
    for &(x, z) in &decorated {
        let t = Instant::now();
        let starts = ChunkStarts::new(&world.structures, &world.generator, &cache, &mut gs.structures, x, z);
        starts_time += t.elapsed();
        let window: Vec<Box<ProtoChunk>> = (0..9).map(|i| chunks.remove(&(x + i % 3 - 1, z + i / 3 - 1)).unwrap()).collect();
        let mut r = Region::new(window, x, z, &world.generator, &mut gs);
        world.decorator.decorate(&mut r, Some((&world.structures, &starts)), &mut timer);
        for c in r.into_chunks() {
            chunks.insert((c.x, c.z), c);
        }
    }
    let total = start.elapsed();
    let t = Instant::now();
    for &(x, z) in &decorated {
        std::hint::black_box(ChunkStarts::new(&world.structures, &world.generator, &cache, &mut gs.structures, x, z));
    }
    let warm = t.elapsed();
    let t = Instant::now();
    let mut starts_n = 0usize;
    for &(x, z) in &decorated {
        starts_n += world.structures.create_starts(&world.generator, &mut gs.structures, x, z).len();
    }
    println!(
        "warm ChunkStarts::new {:.3} ms/chunk; create_starts {:.3} ms/chunk ({starts_n} starts)",
        warm.as_secs_f64() * 1e3 / decorated.len() as f64,
        t.elapsed().as_secs_f64() * 1e3 / decorated.len() as f64
    );
    let n = decorated.len() as f64;
    println!(
        "{} chunks decorated: {:.2} ms/chunk (structure starts and references {:.2} ms/chunk)",
        decorated.len(),
        total.as_secs_f64() * 1e3 / n,
        starts_time.as_secs_f64() * 1e3 / n
    );
    let mut rows: Vec<_> = timer.spent.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.0));
    for (name, (t, count)) in rows.iter().take(25) {
        println!("  {name:55} {:>8.3} ms/chunk  {count:>7} calls", t.as_secs_f64() * 1e3 / n);
    }
}
