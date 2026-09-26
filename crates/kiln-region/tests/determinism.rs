//! The same script gives the same deltas and final state, whatever the hash seed of the
//! cell maps and whatever the queue order of cell events within a tick.

mod common;

use common::{Rng, Sim, walkers};
use kiln_region::{CellPos, DefaultCells, RegionPolicy, Regionizer, Regions, TopologyDelta, TopologyEvent};
use std::hash::{BuildHasher, RandomState};

fn scenario(hash_seed: u64) -> (Vec<(u64, TopologyDelta)>, String) {
    let mut sim = Sim::new(RegionPolicy::default(), hash_seed, false);
    walkers(&mut sim, 7, 16, 4000, 14, 1);
    let digest = sim.digest();
    (sim.log, digest)
}

#[test]
fn identical_across_runs_and_hash_seeds() {
    let per_process = RandomState::new().hash_one(0u64);
    let (log, digest) = scenario(0);
    assert!(log.len() > 100, "scenario too small to mean anything: {} deltas", log.len());
    for seed in [0, 0xdead_beef, per_process] {
        let (l, d) = scenario(seed);
        assert_eq!(l, log, "deltas differ with hash seed {seed:#x}");
        assert_eq!(d, digest, "final state differs with hash seed {seed:#x}");
    }
}

/// Batches of cell events (at most one per cell) applied in generated and reversed order.
#[test]
fn cell_event_queue_order_does_not_matter() {
    let run = |reverse: bool| {
        let mut rng = Rng::new(99);
        let mut rz = Regionizer::new(RegionPolicy { split_period: 5, split_hysteresis: 20, ..RegionPolicy::default() });
        let mut regions: Regions<(), ()> = Regions::with_hash_seed(if reverse { 1 } else { 2 });
        let mut log = Vec::new();
        for tick in 1..=3000u64 {
            let mut batch: Vec<TopologyEvent> = Vec::new();
            let mut used = Vec::new();
            for _ in 0..rng.below(6) {
                let p = CellPos::new(rng.range(-10, 10), rng.range(-10, 10));
                if used.contains(&p) {
                    continue;
                }
                used.push(p);
                // Sparse occupancy (about 10%), so regions both merge and split.
                if regions.owner(p).is_some() {
                    batch.push(TopologyEvent::Vacated(p));
                } else if rng.chance(0.1) {
                    batch.push(TopologyEvent::Occupied(p));
                }
            }
            if reverse {
                batch.reverse();
            }
            batch.into_iter().for_each(|e| rz.push(e));
            log.extend(rz.apply(&mut regions, tick, &mut DefaultCells).into_iter().map(|d| (tick, d)));
        }
        let state: Vec<(u64, Vec<CellPos>)> =
            regions.iter().map(|r| (r.id().0, r.cells().positions().collect())).collect();
        (log, state)
    };
    let (a, b) = (run(false), run(true));
    assert!(a.0.iter().any(|(_, d)| matches!(d, TopologyDelta::Split { .. })));
    assert_eq!(a, b);
}
