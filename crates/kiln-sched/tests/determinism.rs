#![cfg(not(loom))]
//! Strict-mode contract: a tick's results do not depend on the worker count, the window
//! strategy or the schedule (chaos), and equal a plain sequential run.

use kiln_sched::{Ctx, PhaseMode, PoolConfig, TickPool, Window};

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A window through the pool, or a plain loop for the reference run.
fn map<In: Sync, Out: Send>(
    c: Option<&Ctx<'_>>,
    w: Window,
    items: &[In],
    f: impl Fn(Option<&Ctx<'_>>, &In) -> Out + Sync,
) -> Vec<Out> {
    match c {
        Some(c) => c.map_indexed_with(w, items, |c, x| f(Some(c), x)),
        None => items.iter().map(|x| f(None, x)).collect(),
    }
}

struct Region {
    id: u64,
    ents: Vec<u64>,
    state: u64,
    /// `(src tick, src region, seq, payload)`: merged by key, independent of the partition.
    outbox: Vec<(u64, u64, u64, u64)>,
}

fn tick_region(r: &mut Region, c: Option<&Ctx<'_>>, tick: u64) {
    let state = r.state;
    let w = match r.id % 3 {
        0 => Window::new(),
        1 => Window::new().item_ns(2_000),
        _ => Window::new().chunk(3),
    };
    // Windows read a snapshot and are pure; results are applied serially in index order.
    let next = map(c, w, &r.ents, |_, &e| mix(e ^ state.rotate_left((tick % 64) as u32) ^ tick));
    for (e, n) in r.ents.iter_mut().zip(&next) {
        *e = *n;
        r.state = mix(r.state ^ n).rotate_left(1);
    }
    // A window whose items open nested windows.
    let groups: Vec<&[u64]> = r.ents.chunks(17).collect();
    let sums = map(c, Window::new(), &groups, |c, g| {
        let v = map(c, Window::new(), g, |_, &e| mix(e ^ tick));
        v.iter().fold(0u64, |a, x| a.wrapping_mul(0x0100_0000_01B3) ^ x)
    });
    for s in &sums {
        r.state = mix(r.state ^ s);
    }
    if r.state.is_multiple_of(4) {
        r.ents.push(r.state);
    }
    if r.state.is_multiple_of(5) && !r.ents.is_empty() {
        let i = (r.state % r.ents.len() as u64) as usize;
        r.ents.remove(i);
    }
    r.outbox.clear();
    for k in 0..r.state % 3 {
        r.outbox.push((tick, r.id, k, mix(r.state.wrapping_add(k))));
    }
}

fn hash_region(r: &Region) -> u64 {
    r.ents.iter().fold(mix(r.id ^ r.state), |a, e| mix(a ^ e))
}

fn run(mut pool: Option<&mut TickPool>, ticks: u64) -> Vec<u64> {
    let mut regions: Vec<Region> = (0..24u64)
        .map(|id| Region { id, ents: (0..(id * 523) % 3000).map(|e| mix(e + id)).collect(), state: id, outbox: vec![] })
        .collect();
    let n = regions.len() as u64;
    let mut hashes = Vec::new();
    for t in 0..ticks {
        match pool.as_deref_mut() {
            Some(p) => {
                // Estimates vary per tick, so the LPT order and the batches change.
                p.run_units(&mut regions, |r| mix(r.state ^ t) % 400_000, |r, c| tick_region(r, Some(c), t));
            }
            None => regions.iter_mut().for_each(|r| tick_region(r, None, t)),
        }
        // Serial segment: merge messages by key, deliver them, hash the world through a window.
        let mut msgs: Vec<_> = regions.iter().flat_map(|r| r.outbox.iter().copied()).collect();
        msgs.sort_unstable();
        for (_, _, _, payload) in msgs {
            regions[(payload % n) as usize].ents.push(payload);
        }
        let per_region = match pool.as_deref_mut() {
            Some(p) => p.serial(|c| map(Some(c), Window::new(), &regions, |_, r| hash_region(r))),
            None => regions.iter().map(hash_region).collect(),
        };
        hashes.push(per_region.iter().fold(t, |a, h| mix(a ^ h)));
    }
    hashes
}

#[test]
fn identical_across_workers_strategies_and_schedules() {
    const TICKS: u64 = 40;
    let reference = run(None, TICKS);
    for workers in [1, 7, 16] {
        for phase in [PhaseMode::Auto, PhaseMode::Inline, PhaseMode::Parallel, PhaseMode::Mixed] {
            for chaos in [None, Some(1), Some(0xC0FFEE)] {
                let mut cfg = PoolConfig::new(workers);
                cfg.phase = phase;
                cfg.chaos = chaos;
                let mut pool = TickPool::with_config(cfg);
                let got = run(Some(&mut pool), TICKS);
                let first_diff = got.iter().zip(&reference).position(|(a, b)| a != b);
                assert_eq!(first_diff, None, "workers {workers} {phase:?} chaos {chaos:?}: tick hashes diverge");
            }
        }
    }
}
