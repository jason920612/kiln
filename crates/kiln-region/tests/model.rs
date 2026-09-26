//! Property tests against the reference model (DT-R3 shape): random scripts of occupancy,
//! fusion and part traffic; after every apply the partition, deltas and pins must equal
//! the model's, and every entity and message must be conserved and correctly routed.

mod common;

use common::{REASONS, Sim, walkers};
use kiln_region::{CellPos, RegionPolicy};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::RngSeed;

#[derive(Debug, Clone)]
enum Op {
    Occupy(i32, i32),
    OccupyNear(Index, i32, i32),
    Vacate(Index),
    VacateAt(i32, i32),
    Spawn(Index, u8),
    Move(Index, Index),
    Despawn(Index),
    Send(Index, Index),
    Deliver(Index),
    Bump(Index),
    Fuse(Index, Index, u8, u16),
    Expire(u8),
    Tick(u8),
}

fn op() -> impl Strategy<Value = Op> {
    let coord = -8..8i32;
    prop_oneof![
        4 => (coord.clone(), coord.clone()).prop_map(|(x, z)| Op::Occupy(x, z)),
        4 => (any::<Index>(), -3..=3i32, -3..=3i32).prop_map(|(i, dx, dz)| Op::OccupyNear(i, dx, dz)),
        5 => any::<Index>().prop_map(Op::Vacate),
        1 => (coord.clone(), coord).prop_map(|(x, z)| Op::VacateAt(x, z)),
        3 => (any::<Index>(), 1..6u8).prop_map(|(i, n)| Op::Spawn(i, n)),
        3 => (any::<Index>(), any::<Index>()).prop_map(|(e, c)| Op::Move(e, c)),
        1 => any::<Index>().prop_map(Op::Despawn),
        2 => (any::<Index>(), any::<Index>()).prop_map(|(t, s)| Op::Send(t, s)),
        1 => any::<Index>().prop_map(Op::Deliver),
        1 => any::<Index>().prop_map(Op::Bump),
        1 => (any::<Index>(), any::<Index>(), 0..3u8, 1..400u16).prop_map(|(a, b, r, d)| Op::Fuse(a, b, r, d)),
        1 => (0..3u8).prop_map(Op::Expire),
        4 => (1..40u8).prop_map(Op::Tick),
    ]
}

fn policies() -> [RegionPolicy; 4] {
    [
        RegionPolicy::default(),
        RegionPolicy { split_period: 3, split_hysteresis: 7, ..RegionPolicy::default() },
        RegionPolicy { link_cheb: 3, split_period: 7, split_hysteresis: 30, ..RegionPolicy::default() },
        RegionPolicy::unified(),
    ]
}

fn run(policy: RegionPolicy, ops: &[Op]) {
    let mut sim = Sim::new(policy, 0, true);
    for op in ops {
        let live = sim.live_cells();
        let pick = |i: &Index| live[i.index(live.len())];
        match op {
            Op::Occupy(x, z) => sim.occupy(CellPos::new(*x, *z)),
            Op::OccupyNear(i, dx, dz) if !live.is_empty() => sim.occupy(pick(i).offset(*dx, *dz)),
            Op::Vacate(i) if !live.is_empty() => sim.vacate(pick(i)),
            Op::VacateAt(x, z) => sim.vacate(CellPos::new(*x, *z)),
            Op::Spawn(i, n) if !live.is_empty() => {
                let p = pick(i);
                (0..*n).for_each(|_| sim.spawn(p));
            }
            Op::Move(e, c) if !live.is_empty() && sim.entity_count() > 0 => {
                let seqs = sim.entity_seqs();
                sim.move_entity(seqs[e.index(seqs.len())], pick(c));
            }
            Op::Despawn(e) if sim.entity_count() > 0 => {
                let seqs = sim.entity_seqs();
                sim.despawn(seqs[e.index(seqs.len())]);
            }
            Op::Send(t, s) if !live.is_empty() => sim.send(pick(s), pick(t)),
            Op::Deliver(i) if !sim.regions.is_empty() => {
                let ids = sim.region_ids();
                sim.deliver(ids[i.index(ids.len())]);
            }
            Op::Bump(i) if !sim.regions.is_empty() => {
                let ids = sim.region_ids();
                sim.bump(ids[i.index(ids.len())]);
            }
            Op::Fuse(a, b, r, d) if !live.is_empty() => {
                sim.fuse(pick(a), pick(b), REASONS[*r as usize], sim.tick + *d as u64)
            }
            Op::Expire(r) => sim.expire(REASONS[*r as usize]),
            Op::Tick(n) => (0..*n).for_each(|_| sim.step()),
            _ => {}
        }
    }
    // Let pending splits and pin expiries play out.
    for _ in 0..600 {
        sim.step();
    }
}

fn config() -> ProptestConfig {
    let mut c = ProptestConfig::default();
    if c.rng_seed == RngSeed::Random {
        c.rng_seed = RngSeed::Fixed(0x6b69_6c6e);
    }
    c.failure_persistence = None;
    c
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn regionizer_matches_model(policy in 0..4usize, ops in prop::collection::vec(op(), 1..300)) {
        run(policies()[policy], &ops);
    }
}

#[test]
fn walkers_match_model() {
    for (seed, policy) in [(1, 0), (2, 0), (3, 1), (4, 2)] {
        let mut sim = Sim::new(policies()[policy], seed, true);
        walkers(&mut sim, seed, 12, 3000, 12, 1);
        let splits = sim.log.iter().filter(|(_, d)| matches!(d, kiln_region::TopologyDelta::Split { .. })).count();
        let merges = sim.log.iter().filter(|(_, d)| matches!(d, kiln_region::TopologyDelta::Merged { .. })).count();
        assert!(splits > 5 && merges > 5, "seed {seed}: scenario too tame ({splits} splits, {merges} merges)");
    }
}

/// DT-R3 scale: `cargo test --release -p kiln-region --test model -- --ignored`.
#[test]
#[ignore]
fn walkers_soak() {
    for seed in 0..200 {
        let mut sim = Sim::new(policies()[(seed % 3) as usize], seed, true);
        walkers(&mut sim, seed, 20, 5000, 16, 1);
    }
}
