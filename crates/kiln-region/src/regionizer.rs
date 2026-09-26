//! Topology changes: occupancy, merge, lazy split with hysteresis, fusion.

use crate::region::{Region, UNLABELED};
use crate::{CELL_BLOCKS, CellPos, RegionId, RegionPart, Regions};
use smallvec::{SmallVec, smallvec};

/// Two occupied cells are linked when their Chebyshev distance is at most this.
pub const LINK_CHEB: i32 = 2;
/// Asserted minimum distance between loaded chunks of different regions. Cells of
/// 8×8 chunks with link distance 2 actually guarantee 256 blocks.
pub const MIN_GAP_BLOCKS: i32 = 192;
/// Ticks between split checks of a region.
pub const SPLIT_PERIOD: u64 = 20;
/// Ticks a component must stay disconnected before it may split off.
pub const SPLIT_HYSTERESIS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionPolicy {
    pub link_cheb: i32,
    pub split_period: u64,
    pub split_hysteresis: u64,
    pub min_gap_blocks: i32,
    /// One region per dimension (the vanilla profile, or `max_regions = 1`): every new cell
    /// joins the existing region and nothing ever splits.
    pub unified: bool,
}

impl Default for RegionPolicy {
    fn default() -> Self {
        Self {
            link_cheb: LINK_CHEB,
            split_period: SPLIT_PERIOD,
            split_hysteresis: SPLIT_HYSTERESIS,
            min_gap_blocks: MIN_GAP_BLOCKS,
            unified: false,
        }
    }
}

impl RegionPolicy {
    pub fn unified() -> Self {
        Self { unified: true, ..Self::default() }
    }

    /// Cells of different regions are at least `link_cheb + 1` apart, so `link_cheb` whole
    /// empty cells separate their loaded chunks.
    pub fn guaranteed_gap_blocks(&self) -> i32 {
        self.link_cheb * CELL_BLOCKS
    }

    fn validate(&self) {
        assert!(self.link_cheb >= 1, "link_cheb must be at least 1");
        assert!(self.split_period >= 1, "split_period must be at least 1");
        assert!(
            self.guaranteed_gap_blocks() >= self.min_gap_blocks,
            "link distance {} guarantees {} blocks, less than min_gap_blocks {}",
            self.link_cheb,
            self.guaranteed_gap_blocks(),
            self.min_gap_blocks
        );
    }
}

/// Why two regions are fused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FuseReason {
    /// An unbounded command (command block, function) runs next tick.
    Command,
    /// An explosion's reach touches the other region.
    Explosion,
    /// A fast mover's swept box touches the other region.
    FastMover,
    /// Global state written by one region and read by the other in the same tick.
    GlobalConflict,
    /// An uncatalogued cross-region access.
    Uncatalogued,
    /// Forced by an operator.
    Operator,
}

/// Binds the regions owning cells `a` and `b` into one scheduling unit until `until_tick`.
///
/// Pins are cell-addressed, like messages, so they stay meaningful across merges and
/// splits: while active, a pin counts as a link between `a` and `b` for split checks. A
/// pin is dropped when it expires, when either cell is vacated, or on
/// [`TopologyEvent::Expire`] for its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FusePin {
    pub a: CellPos,
    pub b: CellPos,
    pub reason: FuseReason,
    /// First tick at which the pin no longer holds.
    pub until_tick: u64,
}

impl FusePin {
    pub fn new(a: CellPos, b: CellPos, reason: FuseReason, until_tick: u64) -> Self {
        let (a, b) = if b < a { (b, a) } else { (a, b) };
        Self { a, b, reason, until_tick }
    }

    pub(crate) fn key(&self) -> (CellPos, CellPos, FuseReason) {
        (self.a, self.b, self.reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopologyEvent {
    /// A cell got its first region-accessible chunk.
    Occupied(CellPos),
    /// A cell lost its last region-accessible chunk. Every part element bound to the cell
    /// must have been removed first.
    Vacated(CellPos),
    Fuse(FusePin),
    /// Drops every pin with this reason now.
    Expire(FuseReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TopologyDelta {
    Created(RegionId),
    /// `from` (in merge order) were absorbed into `into` and no longer exist.
    Merged { into: RegionId, from: SmallVec<[RegionId; 4]> },
    /// `from` keeps its id and the component holding its anchor; `into` are new regions.
    Split { from: RegionId, into: SmallVec<[RegionId; 4]> },
    Dead(RegionId),
}

pub type Deltas = SmallVec<[TopologyDelta; 8]>;

/// Creates payloads for newly occupied cells and receives those of vacated cells (boxed,
/// so the sim can recycle the allocation).
pub trait CellHooks<C> {
    fn create(&mut self, pos: CellPos) -> Box<C>;
    #[allow(clippy::boxed_local)]
    fn retire(&mut self, _pos: CellPos, _cell: Box<C>) {}
}

/// Default payloads; vacated payloads are dropped.
pub struct DefaultCells;

impl<C: Default> CellHooks<C> for DefaultCells {
    fn create(&mut self, _: CellPos) -> Box<C> {
        Box::default()
    }
}

/// Owns the topology rules and the event queue of one dimension.
#[derive(Debug, Clone)]
pub struct Regionizer {
    policy: RegionPolicy,
    pending: Vec<TopologyEvent>,
    next_id: u64,
    /// Neighbourhood offsets within the link distance, excluding the origin.
    around: Vec<(i32, i32)>,
    /// The half of `around` that is lexicographically after the origin.
    forward: Vec<(i32, i32)>,
}

impl Default for Regionizer {
    fn default() -> Self {
        Self::new(RegionPolicy::default())
    }
}

impl Regionizer {
    pub fn new(policy: RegionPolicy) -> Self {
        policy.validate();
        let r = policy.link_cheb;
        let around: Vec<(i32, i32)> =
            (-r..=r).flat_map(|dz| (-r..=r).map(move |dx| (dx, dz))).filter(|&d| d != (0, 0)).collect();
        let forward = around.iter().copied().filter(|&(dx, dz)| dz > 0 || (dz == 0 && dx > 0)).collect();
        Self { policy, pending: Vec::new(), next_id: 1, around, forward }
    }

    pub fn policy(&self) -> &RegionPolicy {
        &self.policy
    }

    /// Queues an event for the next [`apply`](Self::apply).
    pub fn push(&mut self, event: TopologyEvent) {
        self.pending.push(event);
    }

    pub fn pending(&self) -> &[TopologyEvent] {
        &self.pending
    }

    /// The id the next new region will get.
    pub fn next_id(&self) -> RegionId {
        RegionId(self.next_id)
    }

    fn alloc_id(&mut self) -> RegionId {
        let id = RegionId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Applies the queued events and the periodic split checks for `tick`. Call exactly
    /// once per tick, in the serial phase (B0): `&mut Regions` proves no region is ticking.
    ///
    /// Order: (1) cell events, reduced to the last event per cell, vacated cells first and
    /// then occupied cells, each in cell order; (2) `Fuse`/`Expire` in queue order; (3) pin
    /// expiry at `until_tick <= tick`; (4) split checks of due regions in id order.
    pub fn apply<C, P: RegionPart>(
        &mut self,
        regions: &mut Regions<C, P>,
        tick: u64,
        hooks: &mut impl CellHooks<C>,
    ) -> Deltas {
        let events = std::mem::take(&mut self.pending);
        let mut deltas = Deltas::new();
        let count_before = cfg!(debug_assertions).then(|| part_count(regions));

        let mut cell_events: Vec<(CellPos, usize, bool)> = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match *e {
                TopologyEvent::Occupied(p) => Some((p, i, true)),
                TopologyEvent::Vacated(p) => Some((p, i, false)),
                _ => None,
            })
            .collect();
        cell_events.sort_unstable_by_key(|&(p, i, _)| (p, i));
        let mut net: Vec<(CellPos, bool)> = Vec::with_capacity(cell_events.len());
        for (p, _, occupied) in cell_events {
            match net.last_mut() {
                Some(last) if last.0 == p => last.1 = occupied,
                _ => net.push((p, occupied)),
            }
        }
        for &(pos, _) in net.iter().filter(|e| !e.1) {
            self.vacate(regions, pos, hooks, &mut deltas);
        }
        for &(pos, _) in net.iter().filter(|e| e.1) {
            self.occupy(regions, pos, hooks, &mut deltas);
        }

        for event in &events {
            match *event {
                TopologyEvent::Fuse(pin) => self.fuse(regions, pin, tick, &mut deltas),
                TopologyEvent::Expire(reason) => drop_pins(regions, |p| p.reason == reason),
                _ => {}
            }
        }
        drop_pins(regions, |p| p.until_tick <= tick);

        if !self.policy.unified {
            let period = self.policy.split_period;
            let due: Vec<RegionId> = regions
                .map
                .iter()
                .filter(|(id, r)| r.dirty && tick % period == id.0 % period)
                .map(|(&id, _)| id)
                .collect();
            for id in due {
                self.check_split(regions, id, tick, &mut deltas);
            }
        }

        if let Some(before) = count_before {
            let after = part_count(regions);
            assert_eq!(before, after, "part elements not conserved across apply at tick {tick}");
            if let Err(e) = self.check_invariants(regions) {
                panic!("regionizer invariant violated at tick {tick}: {e}");
            }
        }
        deltas
    }

    fn vacate<C, P: RegionPart>(
        &mut self,
        regions: &mut Regions<C, P>,
        pos: CellPos,
        hooks: &mut impl CellHooks<C>,
        deltas: &mut Deltas,
    ) {
        let Some(id) = regions.table.get(pos) else { return };
        regions.table.remove(pos);
        let region = regions.map.get_mut(&id).expect("table points to a live region");
        let cell = region.cells.remove(pos).expect("table and region agree");
        region.pins.retain(|p| p.a != pos && p.b != pos);
        region.dirty = true;
        if region.cells.is_empty() {
            regions.map.remove(&id);
            deltas.push(TopologyDelta::Dead(id));
        }
        hooks.retire(pos, cell);
    }

    fn occupy<C, P: RegionPart>(
        &mut self,
        regions: &mut Regions<C, P>,
        pos: CellPos,
        hooks: &mut impl CellHooks<C>,
        deltas: &mut Deltas,
    ) {
        if regions.table.contains(pos) {
            return;
        }
        let mut owners: SmallVec<[RegionId; 8]> = SmallVec::new();
        if self.policy.unified {
            owners.extend(regions.map.keys().next().copied());
        } else {
            for &(dx, dz) in &self.around {
                if let Some(id) = regions.table.get(pos.offset(dx, dz))
                    && !owners.contains(&id)
                {
                    owners.push(id);
                }
            }
        }
        let cell = hooks.create(pos);
        let Some(target) = self.merge_all(regions, owners, deltas) else {
            let id = self.alloc_id();
            regions.map.insert(id, Region::new(id, pos, cell));
            regions.table.set(pos, id);
            deltas.push(TopologyDelta::Created(id));
            return;
        };
        regions.map.get_mut(&target).unwrap().cells.insert(pos, cell);
        regions.table.set(pos, target);
    }

    /// Merges `ids` into the one with the smallest anchor, the others in anchor order.
    fn merge_all<C, P: RegionPart>(
        &mut self,
        regions: &mut Regions<C, P>,
        mut ids: SmallVec<[RegionId; 8]>,
        deltas: &mut Deltas,
    ) -> Option<RegionId> {
        ids.sort_by_cached_key(|id| regions.map[id].anchor());
        let (&into, from) = ids.split_first()?;
        for &f in from {
            let src = regions.map.remove(&f).expect("merging a live region");
            for pos in src.cells.positions() {
                regions.table.set(pos, into);
            }
            regions.map.get_mut(&into).unwrap().absorb(src);
        }
        if !from.is_empty() {
            deltas.push(TopologyDelta::Merged { into, from: from.iter().copied().collect() });
        }
        Some(into)
    }

    fn fuse<C, P: RegionPart>(&mut self, regions: &mut Regions<C, P>, pin: FusePin, tick: u64, deltas: &mut Deltas) {
        let pin = FusePin::new(pin.a, pin.b, pin.reason, pin.until_tick);
        if pin.until_tick <= tick {
            return;
        }
        let (Some(ra), Some(rb)) = (regions.table.get(pin.a), regions.table.get(pin.b)) else { return };
        let ids = if ra == rb { smallvec![ra] } else { smallvec![ra, rb] };
        let into = self.merge_all(regions, ids, deltas).unwrap();
        regions.map.get_mut(&into).unwrap().add_pin(pin);
    }

    /// Finds the region's components (links plus active pins). Components other than the
    /// one holding the anchor split off once they have been apart for the hysteresis time.
    ///
    /// Apartness is tracked with labels: each check labels every cell with its component
    /// and records per label since when that component has been apart. At the next check a
    /// component is apart since the latest `since` of the labels it contains, or since now
    /// if one of its labels also occurs in another component (it was connected to it).
    fn check_split<C, P: RegionPart>(
        &mut self,
        regions: &mut Regions<C, P>,
        id: RegionId,
        tick: u64,
        deltas: &mut Deltas,
    ) {
        let region = regions.map.get_mut(&id).unwrap();
        let n = region.cells.len();
        let mut uf = UnionFind::new(n);
        for i in 0..n {
            let p = region.cells.slots[i].pos;
            for &(dx, dz) in &self.forward {
                if let Some(j) = region.cells.index_of(p.offset(dx, dz)) {
                    uf.union(i, j);
                }
            }
        }
        for pin in &region.pins {
            if let (Some(i), Some(j)) = (region.cells.index_of(pin.a), region.cells.index_of(pin.b)) {
                uf.union(i, j);
            }
        }
        // Components numbered in order of first cell, i.e. by anchor.
        let mut comp_of = vec![0u32; n];
        let mut root_comp = vec![u32::MAX; n];
        let mut m = 0u32;
        for (i, c) in comp_of.iter_mut().enumerate() {
            let root = uf.find(i);
            if root_comp[root] == u32::MAX {
                root_comp[root] = m;
                m += 1;
            }
            *c = root_comp[root];
        }
        let m = m as usize;
        if m == 1 {
            region.cells.slots.iter_mut().for_each(|s| s.label = 0);
            region.since = vec![tick];
            region.dirty = false;
            return;
        }

        let labels = region.since.len();
        let mut label_comp = vec![u32::MAX; labels];
        let mut spanning = vec![false; labels];
        for (s, &c) in region.cells.slots.iter().zip(&comp_of) {
            if s.label != UNLABELED {
                let l = s.label as usize;
                if label_comp[l] == u32::MAX {
                    label_comp[l] = c;
                } else if label_comp[l] != c {
                    spanning[l] = true;
                }
            }
        }
        let mut since: Vec<Option<u64>> = vec![None; m];
        let mut reset = vec![false; m];
        for (s, &c) in region.cells.slots.iter().zip(&comp_of) {
            if s.label != UNLABELED {
                let l = s.label as usize;
                let c = c as usize;
                if spanning[l] {
                    reset[c] = true;
                } else {
                    since[c] = since[c].max(Some(region.since[l]));
                }
            }
        }
        let since: Vec<u64> = (0..m).map(|c| if reset[c] { tick } else { since[c].unwrap_or(tick) }).collect();

        // Piece 0 keeps the anchor's component and the young ones.
        let mut piece_of_comp = vec![0usize; m];
        let mut stay_label = vec![0u32; m];
        let mut stay_since = Vec::new();
        let mut leaving = 0usize;
        for c in 0..m {
            if c > 0 && tick - since[c] >= self.policy.split_hysteresis {
                leaving += 1;
                piece_of_comp[c] = leaving;
            } else {
                stay_label[c] = stay_since.len() as u32;
                stay_since.push(since[c]);
            }
        }
        for (s, &c) in region.cells.slots.iter_mut().zip(&comp_of) {
            s.label = stay_label[c as usize];
        }
        region.dirty = stay_since.len() > 1;
        region.since = stay_since;
        if leaving == 0 {
            return;
        }

        let piece_of: Vec<usize> = comp_of.iter().map(|&c| piece_of_comp[c as usize]).collect();
        let new_ids: SmallVec<[RegionId; 4]> = (0..leaving).map(|_| self.alloc_id()).collect();
        let region = regions.map.get_mut(&id).unwrap();
        let pieces = region.split_off(&piece_of, &new_ids, tick);
        for piece in pieces {
            for pos in piece.cells.positions() {
                regions.table.set(pos, piece.id());
            }
            regions.map.insert(piece.id(), piece);
        }
        deltas.push(TopologyDelta::Split { from: id, into: new_ids });
    }

    /// Checks §4.5.5 invariants 1, 2 and 5 plus internal consistency. Invariant 3
    /// (conservation) is checked by `apply` itself in debug builds, which also calls this.
    pub fn check_invariants<C, P: RegionPart>(&self, regions: &Regions<C, P>) -> Result<(), String> {
        let table = &regions.table;
        let mut cells = 0usize;
        for (&id, r) in &regions.map {
            if r.id() != id {
                return Err(format!("region {id:?} stored under the wrong id {:?}", r.id()));
            }
            if id.0 == 0 || id.0 >= self.next_id {
                return Err(format!("region id {id:?} was never allocated"));
            }
            if r.cells.is_empty() {
                return Err(format!("region {id:?} has no cells"));
            }
            for w in r.cells.slots.windows(2) {
                if w[0].key >= w[1].key {
                    return Err(format!("region {id:?} cells out of order at {:?}", w[1].pos));
                }
            }
            for s in &r.cells.slots {
                if s.key != s.pos.key() {
                    return Err(format!("region {id:?} has a stale key for {:?}", s.pos));
                }
                if table.get(s.pos) != Some(id) {
                    return Err(format!("cell {:?} is in region {id:?} but the table says {:?}", s.pos, table.get(s.pos)));
                }
                if s.label != UNLABELED && s.label as usize >= r.since.len() {
                    return Err(format!("region {id:?} cell {:?} has label {} out of range", s.pos, s.label));
                }
            }
            cells += r.cells.len();
            for pin in &r.pins {
                if pin.b < pin.a || table.get(pin.a) != Some(id) || table.get(pin.b) != Some(id) {
                    return Err(format!("pin {pin:?} held by {id:?} does not bind two of its cells"));
                }
            }
            if r.pins.windows(2).any(|w| w[0].key() >= w[1].key()) {
                return Err(format!("region {id:?} pins not sorted or duplicated"));
            }
            let mut stray = None;
            r.part.for_each_cell(&mut |c| {
                if stray.is_none() && table.get(c) != Some(id) {
                    stray = Some(c);
                }
            });
            if let Some(c) = stray {
                return Err(format!("region {id:?} holds an element of cell {c:?}, owned by {:?}", table.get(c)));
            }
        }
        if cells != table.len() {
            return Err(format!("regions hold {cells} cells but the table has {}", table.len()));
        }
        if self.policy.unified {
            if regions.map.len() > 1 {
                return Err(format!("unified policy but {} regions", regions.map.len()));
            }
            return Ok(());
        }
        for (pos, id) in table.iter_unordered() {
            for &(dx, dz) in &self.around {
                let other = pos.offset(dx, dz);
                if let Some(o) = table.get(other)
                    && o != id
                {
                    return Err(format!(
                        "cells {pos:?} ({id:?}) and {other:?} ({o:?}) are {} cells apart, loaded chunks only {} blocks apart",
                        pos.cheb(other),
                        (pos.cheb(other) as i32 - 1) * CELL_BLOCKS
                    ));
                }
            }
        }
        Ok(())
    }
}

fn part_count<C, P: RegionPart>(regions: &Regions<C, P>) -> usize {
    regions.map.values().map(|r| r.part.count()).sum()
}

fn drop_pins<C, P>(regions: &mut Regions<C, P>, f: impl Fn(&FusePin) -> bool) {
    for r in regions.map.values_mut() {
        let n = r.pins.len();
        r.pins.retain(|p| !f(p));
        if r.pins.len() != n {
            r.dirty = true;
        }
    }
}

struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self { parent: (0..n as u32).collect() }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] as usize != i {
            let p = self.parent[i] as usize;
            self.parent[i] = self.parent[p];
            i = p;
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            self.parent[hi] = lo as u32;
        }
    }
}
