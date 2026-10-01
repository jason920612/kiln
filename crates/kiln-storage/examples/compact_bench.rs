//! What compaction of native cell files costs the thread that saves: a save loop that rewrites
//! chunk records and flushes, with compaction inline (the old behaviour) or in the background.
//!
//! usage: cargo run --release -p kiln-storage --example compact_bench -- [flushes] [cells] [kib-per-chunk] [sync|nosync]
//!
//! Each flush rewrites a quarter of the 64 chunks of every cell, so cells fill with stale
//! records and compact every few flushes (far more often than a real server, which saves a
//! chunk when it unloads or on the autosave). The flush time is what a save adds to a tick.

use kiln_storage::native::FORM_NBT;
use kiln_storage::native::cellfile::CHUNK;
use kiln_storage::{CompactionMode, NativeStore};
use kiln_world::ChunkPos;
use std::time::{Duration, Instant};

fn payload(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        // Half repeated bytes: zstd shrinks it like chunk data.
        out.extend_from_slice(&x.to_le_bytes()[..4]);
        out.extend_from_slice(&[0, 0, 0, 0]);
    }
    out.truncate(len);
    out
}

fn run(mode: CompactionMode, flushes: usize, cells: i32, kib: usize, sync: bool) {
    let dir = std::env::temp_dir().join(format!("kiln-compact-bench-{mode:?}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = NativeStore::open(&dir);
    store.sync = sync;
    store.mode = mode;
    // The world exists before the measurement starts: every chunk saved once.
    for c in 0..cells {
        for slot in 0..64 {
            store.write(CHUNK, ChunkPos::new(c * 8 + slot % 8, slot / 8), FORM_NBT, Some(&payload(slot as u64, kib * 1024)));
        }
    }
    store.flush().unwrap();
    store.finish_compactions();
    let started = store.compaction.started;
    let mut times = Vec::with_capacity(flushes);
    let mut seed = 99u64;
    let wall = Instant::now();
    for i in 0..flushes {
        for c in 0..cells {
            for _ in 0..16 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let slot = ((seed >> 33) % 64) as i32;
                store.write(CHUNK, ChunkPos::new(c * 8 + slot % 8, slot / 8), FORM_NBT, Some(&payload(seed, kib * 1024)));
            }
        }
        let t = Instant::now();
        store.flush().unwrap();
        times.push(t.elapsed());
        // The rest of a 50 ms tick: the save is not the only thing the thread does.
        std::thread::sleep(Duration::from_millis(if i % 2 == 0 { 8 } else { 2 }));
    }
    let wall = wall.elapsed();
    store.finish_compactions();
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let p = |q: f64| ms(times[((times.len() as f64 * q) as usize).min(times.len() - 1)]);
    let over: usize = times.iter().filter(|d| d.as_millis() > 10).count();
    let c = &store.compaction;
    println!(
        "{:<10} {} flushes in {:.1} s: flush mean {:.2} ms, p50 {:.2}, p90 {:.2}, p99 {:.2}, max {:.2} ms; {} flushes over 10 ms; \
         {} compactions ({} KiB -> {} KiB), owner thread {:.1} ms total (worst piece {:.2} ms), background {:.1} ms",
        format!("{mode:?}"),
        flushes,
        wall.as_secs_f64(),
        times.iter().map(|d| ms(*d)).sum::<f64>() / times.len() as f64,
        p(0.5),
        p(0.9),
        p(0.99),
        ms(*times.last().unwrap()),
        over,
        c.finished - started.min(c.finished),
        c.bytes_before / 1024,
        c.bytes_after / 1024,
        c.owner_us as f64 / 1e3,
        c.owner_max_us as f64 / 1e3,
        c.background_us as f64 / 1e3,
    );
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize, d: usize| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(d);
    let (flushes, cells, kib) = (n(0, 300), n(1, 6) as i32, n(2, 24));
    let sync = args.get(3).is_none_or(|a| a != "nosync");
    println!("{flushes} flushes, {cells} cells of 64 chunks of {kib} KiB, sync {sync}");
    for mode in [CompactionMode::Inline, CompactionMode::Background, CompactionMode::Inline, CompactionMode::Background] {
        run(mode, flushes, cells, kib, sync);
    }
}
