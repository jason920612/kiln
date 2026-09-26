#![cfg(not(loom))]
//! Region fork-join: `run_units`.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use kiln_sched::{PoolConfig, TickPool};

fn pool(workers: usize, chaos: Option<u64>) -> TickPool {
    let mut cfg = PoolConfig::new(workers);
    cfg.chaos = chaos;
    TickPool::with_config(cfg)
}

#[derive(Default)]
struct Unit {
    id: usize,
    runs: u32,
    cost: u64,
}

fn units(n: usize) -> Vec<Unit> {
    (0..n).map(|id| Unit { id, runs: 0, cost: (id as u64 * 7919) % 1_000_000 }).collect()
}

#[test]
fn every_unit_runs_exactly_once() {
    for workers in [1, 2, 7, 16] {
        for chaos in [None, Some(3)] {
            let mut pool = pool(workers, chaos);
            for n in [0, 1, 2, 7, 50, 500] {
                let mut us = units(n);
                let calls = AtomicUsize::new(0);
                for _ in 0..20 {
                    let report = pool.run_units(
                        &mut us,
                        |u| u.cost,
                        |u, _| {
                            u.runs += 1;
                            calls.fetch_add(1, Relaxed);
                        },
                    );
                    assert_eq!(report.unit_ns.len(), n);
                }
                assert!(us.iter().all(|u| u.runs == 20), "workers {workers} chaos {chaos:?} n {n}");
                assert_eq!(calls.load(Relaxed), 20 * n);
            }
        }
    }
}

#[test]
fn units_borrow_local_data() {
    let mut pool = pool(4, None);
    let table: Vec<u64> = (0..1000).collect();
    let mut sums = vec![0u64; 64];
    pool.run_units(&mut sums, |_| 0, |s, _| *s = table.iter().sum());
    assert!(sums.iter().all(|&s| s == 499_500));
}

#[test]
fn single_worker_starts_units_largest_estimate_first() {
    let mut pool = pool(1, None);
    let mut us = units(40);
    let order = Mutex::new(Vec::new());
    pool.run_units(&mut us, |u| u.cost, |u, _| order.lock().unwrap().push(u.id));
    let mut expected: Vec<usize> = (0..40).collect();
    expected.sort_by_key(|&i| (std::cmp::Reverse(us[i].cost), i));
    assert_eq!(order.into_inner().unwrap(), expected);
}

#[test]
fn panics_propagate_after_all_units_finished() {
    for (workers, chaos) in [(1, None), (7, None), (7, Some(5)), (16, None)] {
        let mut pool = pool(workers, chaos);
        let mut us = units(60);
        let r = panic::catch_unwind(AssertUnwindSafe(|| {
            pool.run_units(
                &mut us,
                |u| u.cost,
                |u, _| {
                    u.runs += 1;
                    if u.id % 20 == 3 {
                        panic!("unit {} failed", u.id);
                    }
                },
            )
        }));
        let payload = r.expect_err("the unit panic must reach the caller");
        let msg = payload.downcast_ref::<String>().expect("panic message");
        assert!(msg.starts_with("unit ") && msg.ends_with(" failed"), "{msg}");
        assert!(us.iter().all(|u| u.runs == 1), "every unit still ran exactly once");
        // The pool is still usable.
        pool.run_units(&mut us, |u| u.cost, |u, _| u.runs += 1);
        assert!(us.iter().all(|u| u.runs == 2));
    }
}

#[test]
fn reports_per_unit_elapsed_time() {
    let mut pool = pool(4, None);
    let mut us = units(8);
    let report = pool.run_units(
        &mut us,
        |u| u.cost,
        |u, _| {
            if u.id == 5 {
                std::thread::sleep(Duration::from_millis(20));
            }
        },
    );
    assert!(report.unit_ns[5] >= 15_000_000, "{:?}", report.unit_ns);
    assert!(report.wall >= Duration::from_nanos(report.unit_ns[5]));
}

#[test]
fn units_run_in_parallel() {
    // Seven sleeping units on seven workers finish in about one sleep, not seven.
    let mut pool = pool(7, None);
    let mut us = units(7);
    let t = Instant::now();
    pool.run_units(&mut us, |_| 10_000_000, |_, _| std::thread::sleep(Duration::from_millis(50)));
    assert!(t.elapsed() < Duration::from_millis(300), "{:?}", t.elapsed());
}

#[test]
fn stats_count_units_and_parking() {
    let mut pool = pool(4, None);
    let mut us = units(100);
    pool.run_units(&mut us, |u| u.cost, |_, _| {});
    // Idle workers park between ticks instead of spinning. A park is only accounted once it
    // ends, so wake everyone with a fork of slow units before reading the counters.
    std::thread::sleep(Duration::from_millis(300));
    let mut slow = units(4);
    pool.run_units(&mut slow, |_| 10_000_000, |_, _| std::thread::sleep(Duration::from_millis(20)));
    let stats = pool.stats();
    assert_eq!(stats.iter().map(|s| s.units).sum::<u64>(), 104);
    for (i, s) in stats.iter().enumerate().skip(1) {
        assert!(s.parked >= Duration::from_millis(150), "worker {i}: {s:?}");
    }
    assert!(stats.iter().map(|s| s.working).sum::<Duration>() >= Duration::from_millis(60), "{stats:?}");
    pool.reset_stats();
    assert!(pool.stats().iter().all(|s| s.units == 0));
}
