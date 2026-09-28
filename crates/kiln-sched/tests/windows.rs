#![cfg(not(loom))]
//! Phase windows: `map_indexed`.

use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use kiln_sched::{PhaseMode, PoolConfig, Strategy, TickPool, Window};

fn pool(workers: usize, phase: PhaseMode, chaos: Option<u64>) -> TickPool {
    let mut cfg = PoolConfig::new(workers);
    cfg.phase = phase;
    cfg.chaos = chaos;
    TickPool::with_config(cfg)
}

const MODES: [PhaseMode; 4] = [PhaseMode::Auto, PhaseMode::Inline, PhaseMode::Parallel, PhaseMode::Mixed];

fn f(x: &u64) -> u64 {
    x.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17) ^ x
}

#[test]
fn output_is_in_input_order() {
    let sizes = [0usize, 1, 2, 3, 63, 64, 65, 1000, 4097, 20_000];
    for workers in [1, 7, 16] {
        for phase in MODES {
            for chaos in [None, Some(11)] {
                let mut pool = pool(workers, phase, chaos);
                for &n in &sizes {
                    let items: Vec<u64> = (0..n as u64).collect();
                    let expect: Vec<u64> = items.iter().map(f).collect();
                    assert_eq!(pool.map_indexed(&items, f), expect, "{workers} {phase:?} {chaos:?} {n}");
                    for chunk in [1, 7, 1000] {
                        let got = pool.serial(|c| c.map_indexed_with(Window::new().chunk(chunk), &items, |_, x| f(x)));
                        assert_eq!(got, expect);
                    }
                }
            }
        }
    }
}

#[test]
fn estimates_pick_the_strategy_without_changing_results() {
    let mut pool = pool(7, PhaseMode::Auto, None);
    let items: Vec<u64> = (0..5000).collect();
    let expect: Vec<u64> = items.iter().map(f).collect();
    for w in [
        Window::new().item_ns(1),
        Window::new().item_ns(1_000),
        Window::new().item_ns(1_000).chunk(3),
        Window::new().strategy(Strategy::Inline),
        Window::new().strategy(Strategy::Parallel),
        Window::new().strategy(Strategy::Parallel).item_ns(50),
    ] {
        assert_eq!(pool.serial(|c| c.map_indexed_with(w, &items, |_, x| f(x))), expect, "{w:?}");
    }
}

#[test]
fn idle_workers_help_the_coordinator() {
    let mut pool = pool(7, PhaseMode::Auto, None);
    let items: Vec<u64> = (0..64).collect();
    let workers = Mutex::new(HashSet::new());
    let t = Instant::now();
    let out = pool.serial(|c| {
        c.map_indexed_with(Window::new().strategy(Strategy::Parallel).chunk(1), &items, |c, &x| {
            workers.lock().unwrap().insert(c.worker());
            std::thread::sleep(Duration::from_millis(2));
            x
        })
    });
    assert_eq!(out, items);
    assert!(workers.lock().unwrap().len() > 1, "only {:?} ran chunks", workers.lock().unwrap());
    assert!(t.elapsed() < Duration::from_millis(100), "{:?}", t.elapsed());
}

#[test]
fn nested_windows() {
    for workers in [1, 7, 16] {
        for phase in MODES {
            for chaos in [None, Some(4)] {
                let mut pool = pool(workers, phase, chaos);
                let outer: Vec<u64> = (0..40).collect();
                let got = pool.serial(|c| {
                    c.map_indexed_with(Window::new(), &outer, |c, &a| {
                        let mid: Vec<u64> = (0..a % 13 + 1).map(|b| a * 100 + b).collect();
                        c.map_indexed_with(Window::new(), &mid, |c, &b| {
                            let inner: Vec<u64> = (0..b % 5 + 1).collect();
                            c.map_indexed(&inner, |x| f(&(x + b))).iter().fold(0u64, |acc, v| acc.wrapping_mul(31) ^ v)
                        })
                    })
                });
                let expect: Vec<Vec<u64>> = outer
                    .iter()
                    .map(|&a| {
                        (0..a % 13 + 1)
                            .map(|b| a * 100 + b)
                            .map(|b| (0..b % 5 + 1).map(|x| f(&(x + b))).fold(0u64, |acc, v| acc.wrapping_mul(31) ^ v))
                            .collect()
                    })
                    .collect();
                assert_eq!(got, expect, "{workers} {phase:?} {chaos:?}");
            }
        }
    }
}

/// Counts live instances, so leaks and double drops both show up.
#[derive(Debug)]
struct Tracked<'a>(&'a AtomicIsize);

impl<'a> Tracked<'a> {
    fn new(live: &'a AtomicIsize) -> Self {
        live.fetch_add(1, Relaxed);
        Tracked(live)
    }
}

impl Drop for Tracked<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

#[test]
fn panics_propagate_and_drop_every_output_once() {
    for workers in [1, 7, 16] {
        for phase in MODES {
            for chaos in [None, Some(8)] {
                let mut pool = pool(workers, phase, chaos);
                for bad in [0usize, 1, 500, 999] {
                    let live = AtomicIsize::new(0);
                    let items: Vec<usize> = (0..1000).collect();
                    let r = panic::catch_unwind(AssertUnwindSafe(|| {
                        pool.serial(|c| {
                            c.map_indexed_with(Window::new().chunk(16), &items, |_, &i| {
                                if i == bad || i == bad / 2 + 300 {
                                    panic!("item {i}");
                                }
                                Tracked::new(&live)
                            })
                        })
                    }));
                    let msg = *r.expect_err("panic must propagate").downcast::<String>().unwrap();
                    assert!(msg.starts_with("item "), "{msg}");
                    assert_eq!(live.load(Relaxed), 0, "{workers} {phase:?} {chaos:?} bad {bad}");
                }
                // Still usable afterwards.
                let items: Vec<u64> = (0..300).collect();
                assert_eq!(pool.map_indexed(&items, |x| x + 1), (1..301).collect::<Vec<_>>());
            }
        }
    }
}

#[test]
fn zero_sized_outputs() {
    let mut pool = pool(7, PhaseMode::Parallel, None);
    let items = vec![0u8; 10_000];
    let count = AtomicUsize::new(0);
    let out: Vec<()> = pool.map_indexed(&items, |_| {
        count.fetch_add(1, Relaxed);
    });
    assert_eq!(out.len(), 10_000);
    assert_eq!(count.load(Relaxed), 10_000);
}

#[test]
fn windows_inside_units() {
    for workers in [1, 7, 16] {
        for phase in MODES {
            let mut pool = pool(workers, phase, Some(workers as u64));
            let mut regions: Vec<(u64, Vec<u64>, u64)> =
                (0..30).map(|r| (r, (0..(r * 97 % 700)).collect(), 0)).collect();
            pool.run_units(
                &mut regions,
                |r| r.1.len() as u64 * 1000,
                |r, c| {
                    let out = c.map_indexed(&r.1, |x| f(&(x ^ r.0)));
                    r.2 = out.iter().fold(r.0, |acc, v| acc.rotate_left(5) ^ v);
                },
            );
            for (id, items, got) in &regions {
                let expect = items.iter().map(|x| f(&(x ^ id))).fold(*id, |acc, v| acc.rotate_left(5) ^ v);
                assert_eq!(*got, expect);
            }
        }
    }
}

#[test]
fn map_mut_visits_each_item_once_in_place() {
    for workers in [1, 7] {
        for phase in MODES {
            for chaos in [None, Some(5)] {
                let mut pool = pool(workers, phase, chaos);
                for n in [0usize, 1, 2, 65, 3000] {
                    let mut items: Vec<(u64, u32)> = (0..n as u64).map(|x| (x, 0)).collect();
                    let got = pool.serial(|c| {
                        c.map_mut(&mut items, |it| {
                            it.1 += 1;
                            f(&it.0)
                        })
                    });
                    let expect: Vec<u64> = (0..n as u64).map(|x| f(&x)).collect();
                    assert_eq!(got, expect, "{workers} {phase:?} {chaos:?} {n}");
                    assert!(items.iter().all(|it| it.1 == 1), "every item mutated exactly once");
                }
            }
        }
    }
}
