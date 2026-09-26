//! One test per regionizer rule.

use kiln_region::{
    CellHooks, CellPos, FusePin, FuseReason, Inbox, MaxCounters, MsgKey, RegionId, RegionPolicy, Regionizer, Regions,
    TickList, TopologyDelta, TopologyEvent,
};
use smallvec::smallvec;

#[derive(Debug)]
struct CellData {
    pos: CellPos,
}

type Parts = (TickList<u32>, Inbox<u32>, MaxCounters<1>);

#[derive(Default)]
struct Hooks {
    retired: Vec<CellPos>,
}

impl CellHooks<CellData> for Hooks {
    fn create(&mut self, pos: CellPos) -> Box<CellData> {
        Box::new(CellData { pos })
    }
    fn retire(&mut self, pos: CellPos, cell: Box<CellData>) {
        assert_eq!(cell.pos, pos, "retired payload belongs to another cell");
        self.retired.push(pos);
    }
}

struct World {
    rz: Regionizer,
    regions: Regions<CellData, Parts>,
    hooks: Hooks,
    tick: u64,
}

fn c(x: i32, z: i32) -> CellPos {
    CellPos::new(x, z)
}

fn id(n: u64) -> RegionId {
    RegionId(n)
}

impl World {
    fn new() -> Self {
        Self::with(RegionPolicy::default())
    }

    fn with(policy: RegionPolicy) -> Self {
        Self { rz: Regionizer::new(policy), regions: Regions::new(), hooks: Hooks::default(), tick: 0 }
    }

    fn occupy(&mut self, cells: &[(i32, i32)]) -> &mut Self {
        for &(x, z) in cells {
            self.rz.push(TopologyEvent::Occupied(c(x, z)));
        }
        self
    }

    fn vacate(&mut self, cells: &[(i32, i32)]) -> &mut Self {
        for &(x, z) in cells {
            self.rz.push(TopologyEvent::Vacated(c(x, z)));
        }
        self
    }

    fn push(&mut self, e: TopologyEvent) -> &mut Self {
        self.rz.push(e);
        self
    }

    fn step(&mut self) -> Vec<TopologyDelta> {
        self.tick += 1;
        self.rz.apply(&mut self.regions, self.tick, &mut self.hooks).into_vec()
    }

    /// Steps until `until` (inclusive), returning (tick, delta) pairs.
    fn run_to(&mut self, until: u64) -> Vec<(u64, TopologyDelta)> {
        let mut out = Vec::new();
        while self.tick < until {
            for d in self.step() {
                out.push((self.tick, d));
            }
        }
        out
    }

    fn owner(&self, x: i32, z: i32) -> Option<RegionId> {
        self.regions.owner(c(x, z))
    }

    fn cells_of(&self, r: RegionId) -> Vec<CellPos> {
        self.regions.get(r).unwrap().cells().positions().collect()
    }
}

fn splits(deltas: &[(u64, TopologyDelta)]) -> Vec<(u64, TopologyDelta)> {
    deltas.iter().filter(|(_, d)| matches!(d, TopologyDelta::Split { .. })).cloned().collect()
}

#[test]
fn link_distance_two_merges_three_does_not() {
    let mut w = World::new();
    w.occupy(&[(0, 0), (2, 2)]);
    assert_eq!(w.step(), [TopologyDelta::Created(id(1))]);
    assert_eq!(w.owner(2, 2), Some(id(1)));

    w.occupy(&[(5, 0), (-3, 1), (0, -3)]);
    let d = w.step();
    assert_eq!(d.len(), 3, "each cell 3 away starts its own region: {d:?}");
    assert!(d.iter().all(|d| matches!(d, TopologyDelta::Created(_))));
    assert_eq!(w.regions.len(), 4);
    assert!(w.rz.check_invariants(&w.regions).is_ok());

    let mut w = World::new();
    w.occupy(&[(0, 0), (3, 0)]).step();
    assert_eq!(w.regions.len(), 2);
    w.occupy(&[(0, 10), (2, 10)]).step();
    assert_eq!(w.owner(0, 10), w.owner(2, 10));
}

#[test]
fn occupying_a_bridge_merges_all_linked_regions() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).step();
    w.occupy(&[(4, 0)]).step();
    w.occupy(&[(2, 4)]).step();
    assert_eq!(w.regions.len(), 3);
    assert!(c(4, 0) < c(2, 4));
    w.occupy(&[(2, 2)]);
    let d = w.step();
    assert_eq!(d, [TopologyDelta::Merged { into: id(1), from: smallvec![id(2), id(3)] }]);
    assert_eq!(w.regions.len(), 1);
    assert_eq!(w.cells_of(id(1)).len(), 4);
}

#[test]
fn merge_target_is_smallest_anchor_not_oldest() {
    let mut w = World::new();
    w.occupy(&[(4, 0)]).step(); // id 1
    w.occupy(&[(0, 0)]).step(); // id 2, smaller anchor
    assert!(c(0, 0) < c(4, 0));
    w.occupy(&[(2, 0)]);
    assert_eq!(w.step(), [TopologyDelta::Merged { into: id(2), from: smallvec![id(1)] }]);
    let r = w.regions.get(id(2)).unwrap();
    assert_eq!(r.anchor(), c(0, 0));
    assert_eq!(w.cells_of(id(2)), [c(0, 0), c(2, 0), c(4, 0)]);
}

#[test]
fn anchor_is_smallest_cell_in_morton_order() {
    let mut w = World::new();
    w.occupy(&[(1, 1), (0, 1), (1, 0), (-1, 0)]).step();
    let r = w.regions.get(id(1)).unwrap();
    let cells: Vec<CellPos> = r.cells().positions().collect();
    let mut sorted = cells.clone();
    sorted.sort();
    assert_eq!(cells, sorted, "cells are kept in CellPos order");
    assert_eq!(r.anchor(), c(-1, 0));
    assert_eq!(r.anchor(), *cells.iter().min().unwrap());
    w.vacate(&[(-1, 0)]).step();
    assert_eq!(w.regions.get(id(1)).unwrap().anchor(), c(1, 0).min(c(0, 1)));
}

#[test]
fn ids_are_monotonic_and_never_reused() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).step();
    w.vacate(&[(0, 0)]);
    assert_eq!(w.step(), [TopologyDelta::Dead(id(1))]);
    w.occupy(&[(0, 0)]);
    assert_eq!(w.step(), [TopologyDelta::Created(id(2))]);
    w.occupy(&[(1, 0), (2, 0), (3, 0), (4, 0)]).step();
    w.vacate(&[(1, 0), (2, 0), (3, 0)]).step();
    let d = w.run_to(400);
    let split = splits(&d);
    assert_eq!(split.len(), 1);
    assert_eq!(split[0].1, TopologyDelta::Split { from: id(2), into: smallvec![id(3)] });
    assert_eq!(w.rz.next_id(), id(4));
}

#[test]
fn vacating_the_last_cell_kills_the_region_and_returns_the_payload() {
    let mut w = World::new();
    w.occupy(&[(5, 5), (6, 5)]).step();
    w.vacate(&[(5, 5)]);
    assert_eq!(w.step(), []);
    w.vacate(&[(6, 5)]);
    assert_eq!(w.step(), [TopologyDelta::Dead(id(1))]);
    assert_eq!(w.hooks.retired, [c(5, 5), c(6, 5)]);
    assert!(w.regions.is_empty() && w.regions.table().is_empty());
}

#[test]
fn cell_events_reduce_to_the_last_one_per_cell() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).vacate(&[(0, 0)]);
    assert_eq!(w.step(), []);
    w.occupy(&[(0, 0)]).step();
    w.vacate(&[(0, 0)]).occupy(&[(0, 0)]);
    assert_eq!(w.step(), []);
    assert_eq!(w.owner(0, 0), Some(id(1)), "the no-op first batch allocated no id");
    assert!(w.hooks.retired.is_empty());
}

/// Region 1 = cells (0,0)..(4,0); its checks run at ticks ≡ 1 (mod 20).
fn line_then_cut(w: &mut World) {
    w.occupy(&[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
    w.step();
    w.run_to(4);
    w.vacate(&[(1, 0), (2, 0), (3, 0)]);
    w.step(); // tick 5
}

#[test]
fn split_only_after_hysteresis() {
    let mut w = World::new();
    line_then_cut(&mut w);
    // First check seeing two components: tick 21. Split 100 ticks later, at 121.
    let d = w.run_to(120);
    assert!(splits(&d).is_empty(), "split before the hysteresis ran out: {d:?}");
    assert_eq!(w.owner(4, 0), Some(id(1)));
    assert_eq!(w.step(), [TopologyDelta::Split { from: id(1), into: smallvec![id(2)] }]);
    assert_eq!(w.owner(0, 0), Some(id(1)), "the anchor's component keeps the id");
    assert_eq!(w.owner(4, 0), Some(id(2)));
}

#[test]
fn hysteresis_policy_is_configurable() {
    let mut w = World::with(RegionPolicy { split_period: 5, split_hysteresis: 10, ..RegionPolicy::default() });
    line_then_cut(&mut w);
    // Checks at ticks ≡ 1 (mod 5): first sees the cut at 6, split at 16.
    let d = w.run_to(16);
    assert_eq!(splits(&d), [(16, TopologyDelta::Split { from: id(1), into: smallvec![id(2)] })]);
}

#[test]
fn reconnecting_resets_the_hysteresis() {
    let mut w = World::new();
    line_then_cut(&mut w);
    w.run_to(69);
    w.occupy(&[(2, 0)]); // linked to both halves again
    assert_eq!(w.step(), [], "bridging cells of the same region is not a merge");
    w.run_to(89);
    w.vacate(&[(2, 0)]);
    w.step(); // 90; check at 101 sees the cut anew
    let d = w.run_to(200);
    assert!(splits(&d).is_empty(), "{d:?}");
    assert_eq!(w.step(), [TopologyDelta::Split { from: id(1), into: smallvec![id(2)] }]);
    assert_eq!(w.tick, 201);
}

#[test]
fn walking_back_and_forth_does_not_thrash() {
    let mut w = World::new();
    w.occupy(&[(0, 0), (2, 0), (4, 0)]).step();
    let mut deltas = Vec::new();
    for t in 2..=3000u64 {
        if t % 50 == 0 {
            if (t / 50) % 2 == 1 {
                w.vacate(&[(2, 0)]);
            } else {
                w.occupy(&[(2, 0)]);
            }
        }
        deltas.extend(w.step());
    }
    assert!(deltas.is_empty(), "50-tick disconnections must not split: {deltas:?}");
    assert_eq!(w.regions.len(), 1);
}

#[test]
fn components_split_independently_by_age() {
    let mut w = World::new();
    w.occupy(&[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0), (8, 0)]).step();
    w.run_to(4);
    w.vacate(&[(5, 0), (6, 0), (7, 0)]).step(); // (8,0) apart from tick 21
    w.run_to(44);
    w.vacate(&[(1, 0), (2, 0), (3, 0)]).step(); // (4,0) apart from tick 61
    let d = w.run_to(400);
    assert_eq!(
        splits(&d),
        [
            (121, TopologyDelta::Split { from: id(1), into: smallvec![id(2)] }),
            (161, TopologyDelta::Split { from: id(1), into: smallvec![id(3)] }),
        ]
    );
    assert_eq!(w.owner(8, 0), Some(id(2)));
    assert_eq!(w.owner(4, 0), Some(id(3)));
}

#[test]
fn fusion_merges_unlinked_regions_until_expiry() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).step(); // id 1
    w.occupy(&[(10, 0)]).step(); // id 2
    let pin = FusePin::new(c(10, 0), c(0, 0), FuseReason::Explosion, 300);
    w.push(TopologyEvent::Fuse(pin));
    assert_eq!(w.step(), [TopologyDelta::Merged { into: id(1), from: smallvec![id(2)] }]);
    assert_eq!(w.regions.get(id(1)).unwrap().pins(), [FusePin::new(c(0, 0), c(10, 0), FuseReason::Explosion, 300)]);
    // Force a check: vacating another cell marks the region dirty.
    w.occupy(&[(1, 0)]).step();
    w.vacate(&[(1, 0)]).step();
    let d = w.run_to(299);
    assert!(d.is_empty(), "pinned regions must stay fused: {d:?}");
    // Pin gone at 300; the check at 301 sees the halves apart; split at 401.
    let d = w.run_to(401);
    assert_eq!(d, [(401, TopologyDelta::Split { from: id(1), into: smallvec![id(3)] })]);
    assert!(w.regions.get(id(1)).unwrap().pins().is_empty());
}

#[test]
fn renewed_pin_keeps_latest_expiry_and_expire_drops_early() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).occupy(&[(10, 0)]).step();
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(10, 0), FuseReason::Command, 50)));
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(10, 0), FuseReason::Command, 80)));
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(10, 0), FuseReason::Command, 60)));
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(10, 0), FuseReason::Operator, 1000)));
    w.step();
    let pins = w.regions.get(id(1)).unwrap().pins().to_vec();
    assert_eq!(pins.len(), 2);
    assert_eq!(pins[0].until_tick, 80);
    w.push(TopologyEvent::Expire(FuseReason::Operator));
    w.step();
    assert_eq!(w.regions.get(id(1)).unwrap().pins().len(), 1);
    w.run_to(80);
    assert!(w.regions.get(id(1)).unwrap().pins().is_empty());
    // Expired pins arriving late are ignored.
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(10, 0), FuseReason::Command, 81)));
    w.step();
    assert!(w.regions.get(id(1)).unwrap().pins().is_empty());
}

#[test]
fn pin_to_unowned_cell_is_ignored_and_vacating_drops_pins() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).occupy(&[(10, 0)]).step();
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(20, 0), FuseReason::FastMover, 1000)));
    assert_eq!(w.step(), []);
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(10, 0), FuseReason::FastMover, 1000)));
    w.occupy(&[(11, 0)]);
    w.step();
    assert_eq!(w.regions.len(), 1);
    w.vacate(&[(10, 0)]).step();
    assert!(w.regions.get(id(1)).unwrap().pins().is_empty());
    // (11,0) is no longer pinned: it splits off after the hysteresis.
    let d = w.run_to(300);
    assert_eq!(splits(&d).len(), 1);
    assert_eq!(w.regions.len(), 2);
}

#[test]
fn fused_region_still_splits_off_unrelated_components() {
    let mut w = World::new();
    w.occupy(&[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]).occupy(&[(20, 0)]).step();
    w.push(TopologyEvent::Fuse(FusePin::new(c(0, 0), c(20, 0), FuseReason::Operator, 10_000)));
    w.step();
    w.run_to(4);
    w.vacate(&[(1, 0), (2, 0), (3, 0)]).step();
    let d = w.run_to(500);
    assert_eq!(splits(&d), [(121, TopologyDelta::Split { from: id(1), into: smallvec![id(3)] })]);
    assert_eq!(w.owner(4, 0), Some(id(3)));
    assert_eq!(w.owner(20, 0), Some(id(1)), "the pinned pair stays together");
}

#[test]
fn split_moves_payload_pointers_and_partitions_parts() {
    let mut w = World::new();
    line_then_cut(&mut w);
    let key = |seq| MsgKey { src_tick: 1, src_cell: c(0, 0), seq };
    let before: Vec<(CellPos, *const CellData)> =
        w.regions.get(id(1)).unwrap().cells().iter().map(|(p, d)| (p, d as *const CellData)).collect();
    {
        let (tl, inbox, counters) = w.regions.get_mut(id(1)).unwrap().part_mut();
        for s in 0..10u64 {
            tl.insert(s, if s % 3 == 0 { c(4, 0) } else { c(0, 0) }, s as u32);
        }
        inbox.push(key(2), c(4, 0), 20);
        inbox.push(key(1), c(0, 0), 10);
        inbox.push(key(3), c(4, 0), 30);
        counters.0[0] = 77;
    }
    w.run_to(121);
    let (a, b) = (w.regions.get(id(1)).unwrap(), w.regions.get(id(2)).unwrap());
    let seqs = |r: &kiln_region::Region<CellData, Parts>| r.part().0.iter().map(|e| e.seq).collect::<Vec<_>>();
    assert_eq!(seqs(a), [1, 2, 4, 5, 7, 8]);
    assert_eq!(seqs(b), [0, 3, 6, 9]);
    assert_eq!(a.part().1.iter().map(|l| l.body).collect::<Vec<_>>(), [10]);
    assert_eq!(b.part().1.iter().map(|l| l.body).collect::<Vec<_>>(), [20, 30]);
    assert_eq!((a.part().2.0, b.part().2.0), ([77], [77]));
    for (pos, ptr) in before {
        let data = w.regions.at(pos).unwrap().cells().get(pos).unwrap();
        assert_eq!(data as *const CellData, ptr, "payload of {pos:?} was moved, not its pointer");
        assert_eq!(data.pos, pos);
    }
}

#[test]
fn merge_interleaves_parts_by_key() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).occupy(&[(4, 0)]).step();
    for (r, cell, seqs) in [(1, c(0, 0), [1u64, 4, 5]), (2, c(4, 0), [2, 3, 6])] {
        let tl = &mut w.regions.get_mut(id(r)).unwrap().part_mut().0;
        for s in seqs {
            tl.insert(s, cell, 0);
        }
    }
    w.regions.get_mut(id(1)).unwrap().part_mut().2.0[0] = 5;
    w.regions.get_mut(id(2)).unwrap().part_mut().2.0[0] = 9;
    w.occupy(&[(2, 0)]).step();
    let r = w.regions.get(id(1)).unwrap();
    assert_eq!(r.part().0.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
    assert_eq!(r.part().2.0, [9]);
}

#[test]
fn unified_policy_keeps_one_region() {
    let mut w = World::with(RegionPolicy::unified());
    w.occupy(&[(0, 0), (100, -100), (-50, 7)]);
    assert_eq!(w.step(), [TopologyDelta::Created(id(1))]);
    w.vacate(&[(-50, 7)]);
    let d = w.run_to(1000);
    assert!(d.is_empty());
    assert_eq!(w.regions.len(), 1);
    assert_eq!(w.cells_of(id(1)).len(), 2);
}

#[test]
fn invariant_check_reports_misrouted_elements() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).occupy(&[(10, 0)]).step();
    w.regions.get_mut(id(1)).unwrap().part_mut().0.insert(1, c(10, 0), 0);
    let err = w.rz.check_invariants(&w.regions).unwrap_err();
    assert!(err.contains("holds an element"), "{err}");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "not conserved")]
fn vacating_a_cell_with_elements_is_caught() {
    let mut w = World::new();
    w.occupy(&[(0, 0)]).step();
    w.regions.get_mut(id(1)).unwrap().part_mut().0.insert(1, c(0, 0), 0);
    w.vacate(&[(0, 0)]).step();
}

#[test]
#[should_panic(expected = "guarantees")]
fn policy_must_guarantee_the_minimum_gap() {
    Regionizer::new(RegionPolicy { link_cheb: 1, ..RegionPolicy::default() });
}
