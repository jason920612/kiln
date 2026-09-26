#![cfg(not(loom))]
//! Scheduling priorities: a worker waiting in a window never starts another unit nor helps
//! another unit's window; housekeeping waits for forks.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use kiln_sched::{PoolConfig, Strategy, TickPool, Window};

thread_local! {
    /// Units whose `map_indexed` call is on this thread's stack, innermost last.
    static IN_WINDOW: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn spin_for(d: Duration) {
    let t = Instant::now();
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

struct Region {
    id: usize,
    items: Vec<u32>,
}

#[derive(Default)]
struct Log {
    /// A unit started on a thread that was inside another unit's window.
    unit_started_while_waiting: AtomicUsize,
    /// A chunk ran on a thread that was waiting in a window of a different unit.
    foreign_chunk_while_waiting: AtomicUsize,
    /// Chunks that ran on a thread other than their window's owner (the test is vacuous
    /// without helping).
    helped: AtomicUsize,
    chunks: AtomicUsize,
}

fn tick(r: &mut Region, c: &kiln_sched::Ctx<'_>, log: &Log) {
    IN_WINDOW.with(|w| {
        if !w.borrow().is_empty() {
            log.unit_started_while_waiting.fetch_add(1, Relaxed);
        }
    });
    let id = r.id;
    let owner = std::thread::current().id();
    IN_WINDOW.with(|w| w.borrow_mut().push(id));
    let out = c.map_indexed_with(Window::new().strategy(Strategy::Parallel).chunk(1), &r.items, |c, &x| {
        log.chunks.fetch_add(1, Relaxed);
        IN_WINDOW.with(|w| {
            if w.borrow().last().is_some_and(|&top| top != id) {
                log.foreign_chunk_while_waiting.fetch_add(1, Relaxed);
            }
        });
        if std::thread::current().id() != owner {
            log.helped.fetch_add(1, Relaxed);
        }
        spin_for(Duration::from_micros(20 + (x as u64 % 7) * 10));
        // A nested window: its owner is whichever thread runs this chunk.
        if x.is_multiple_of(5) {
            IN_WINDOW.with(|w| w.borrow_mut().push(id));
            let inner: Vec<u32> = (0..6).collect();
            let v = c.map_indexed_with(Window::new().strategy(Strategy::Parallel).chunk(1), &inner, |_, &y| {
                IN_WINDOW.with(|w| {
                    if w.borrow().last().is_some_and(|&top| top != id) {
                        log.foreign_chunk_while_waiting.fetch_add(1, Relaxed);
                    }
                });
                spin_for(Duration::from_micros(15));
                y
            });
            IN_WINDOW.with(|w| w.borrow_mut().pop());
            assert_eq!(v, inner);
        }
        x
    });
    IN_WINDOW.with(|w| w.borrow_mut().pop());
    assert_eq!(out, r.items);
}

#[test]
fn waiting_worker_never_starts_another_unit() {
    for (workers, chaos) in [(2, None), (7, None), (7, Some(21)), (16, None), (16, Some(22))] {
        let mut cfg = PoolConfig::new(workers);
        cfg.chaos = chaos;
        let mut pool = TickPool::with_config(cfg);
        let log = Log::default();
        // Many more units than workers, so a waiting worker always has units it could take.
        let mut regions: Vec<Region> =
            (0..48).map(|id| Region { id, items: (0..(8 + id as u32 % 24)).collect() }).collect();
        for _ in 0..5 {
            pool.run_units(&mut regions, |r| r.items.len() as u64 * 50_000, |r, c| tick(r, c, &log));
        }
        assert_eq!(log.unit_started_while_waiting.load(Relaxed), 0, "workers {workers}");
        assert_eq!(log.foreign_chunk_while_waiting.load(Relaxed), 0, "workers {workers}");
        assert!(log.helped.load(Relaxed) > 0, "no chunk was ever helped with (workers {workers})");
        assert!(log.chunks.load(Relaxed) > 0);
    }
}

#[test]
fn housekeeping_waits_for_the_fork() {
    let mut pool = TickPool::new(4);
    let hk = pool.housekeeper();
    let ran = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let flag = ran.clone();
    let mut one = vec![0u8];
    pool.run_units(
        &mut one,
        |_| 0,
        |_, _| {
            let (set, tx) = (flag.clone(), tx.clone());
            hk.spawn(move || {
                set.store(true, Relaxed);
                let _ = tx.send(());
            });
            // Three workers are idle for 30 ms, but the fork is still in progress.
            std::thread::sleep(Duration::from_millis(30));
            assert!(!flag.load(Relaxed), "housekeeping ran during the fork");
        },
    );
    rx.recv_timeout(Duration::from_secs(10)).expect("housekeeping ran after the fork");
    assert!(ran.load(Relaxed));
}

#[test]
fn housekeeping_runs_and_survives_panics() {
    let pool = TickPool::new(3);
    let (tx, rx) = mpsc::channel();
    pool.spawn_housekeeping(|| panic!("housekeeping failure"));
    for i in 0..100 {
        let tx = tx.clone();
        pool.spawn_housekeeping(move || tx.send(i).unwrap());
    }
    let mut got: Vec<i32> = (0..100).map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
    got.sort_unstable();
    assert_eq!(got, (0..100).collect::<Vec<_>>());
    // A job's counters are bumped after it returns, so the last send can beat them.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stats = pool.stats();
    let settled = |st: &[kiln_sched::WorkerStats]| {
        st.iter().map(|s| s.housekeeping).sum::<u64>() >= 101 && st.iter().map(|s| s.housekeeping_panics).sum::<u64>() >= 1
    };
    while !settled(&stats) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
        stats = pool.stats();
    }
    assert_eq!(stats.iter().map(|s| s.housekeeping).sum::<u64>(), 101);
    assert_eq!(stats.iter().map(|s| s.housekeeping_panics).sum::<u64>(), 1);
}

#[test]
fn single_worker_housekeeping_runs_on_request_or_drop() {
    let mut pool = TickPool::new(1);
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..3 {
        let c = count.clone();
        pool.spawn_housekeeping(move || {
            c.fetch_add(1, Relaxed);
        });
    }
    assert_eq!(pool.run_housekeeping(Instant::now() + Duration::from_secs(5)), 3);
    let c = count.clone();
    pool.housekeeper().spawn(move || {
        c.fetch_add(1, Relaxed);
    });
    drop(pool);
    assert_eq!(count.load(Relaxed), 4);
}
