//! Regions: index sets of cells that own their cells' payload and per-region parts.

use crate::{CellPos, CellTable, FusePin, RegionPart};
use smallvec::SmallVec;
use std::collections::BTreeMap;
use std::collections::btree_map;

/// Identifies one region lifetime. Allocated monotonically, never reused in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u64);

/// Split-watch label of a cell that joined after the region's last split check.
pub(crate) const UNLABELED: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Slot {
    pub key: u64,
    pub pos: CellPos,
    /// Component this cell belonged to at the region's last split check.
    pub label: u32,
}

/// The cells of a region with their payloads, sorted by [`CellPos`] order.
///
/// Iteration order depends only on the set of cells, never on how the region was built.
#[derive(Debug)]
pub struct CellSet<C> {
    pub(crate) slots: Vec<Slot>,
    cells: Vec<Box<C>>,
}

impl<C> CellSet<C> {
    fn new() -> Self {
        Self { slots: Vec::new(), cells: Vec::new() }
    }

    fn find(&self, pos: CellPos) -> Result<usize, usize> {
        let key = pos.key();
        self.slots.binary_search_by_key(&key, |s| s.key)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn contains(&self, pos: CellPos) -> bool {
        self.find(pos).is_ok()
    }

    pub fn get(&self, pos: CellPos) -> Option<&C> {
        self.find(pos).ok().map(|i| &*self.cells[i])
    }

    pub fn get_mut(&mut self, pos: CellPos) -> Option<&mut C> {
        self.find(pos).ok().map(|i| &mut *self.cells[i])
    }

    pub fn positions(&self) -> impl ExactSizeIterator<Item = CellPos> + '_ {
        self.slots.iter().map(|s| s.pos)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (CellPos, &C)> + '_ {
        self.slots.iter().zip(&self.cells).map(|(s, c)| (s.pos, &**c))
    }

    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (CellPos, &mut C)> + '_ {
        self.slots.iter().zip(&mut self.cells).map(|(s, c)| (s.pos, &mut **c))
    }

    pub(crate) fn index_of(&self, pos: CellPos) -> Option<usize> {
        self.find(pos).ok()
    }

    pub(crate) fn insert(&mut self, pos: CellPos, cell: Box<C>) {
        let i = self.find(pos).expect_err("cell already in region");
        self.slots.insert(i, Slot { key: pos.key(), pos, label: UNLABELED });
        self.cells.insert(i, cell);
    }

    pub(crate) fn remove(&mut self, pos: CellPos) -> Option<Box<C>> {
        let i = self.find(pos).ok()?;
        self.slots.remove(i);
        Some(self.cells.remove(i))
    }

    /// Merges `other` in; its labels are shifted by `label_offset`.
    fn merge(&mut self, other: CellSet<C>, label_offset: u32) {
        let shift = move |mut s: Slot| {
            if s.label != UNLABELED {
                s.label += label_offset;
            }
            s
        };
        let b = other.slots.into_iter().map(shift).zip(other.cells);
        let (a_slots, a_cells) = (std::mem::take(&mut self.slots), std::mem::take(&mut self.cells));
        let n = a_slots.len() + b.len();
        self.slots.reserve_exact(n);
        self.cells.reserve_exact(n);
        let mut a = a_slots.into_iter().zip(a_cells).peekable();
        let mut b = b.peekable();
        loop {
            let take_b = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => y.0.key < x.0.key,
                (Some(_), None) => false,
                (None, Some(_)) => true,
                (None, None) => break,
            };
            let (s, c) = if take_b { b.next() } else { a.next() }.unwrap();
            self.slots.push(s);
            self.cells.push(c);
        }
    }

    /// Stable partition into `n` sets; `piece_of[i]` is the destination of cell `i`.
    fn partition(self, piece_of: &[usize], n: usize) -> SmallVec<[CellSet<C>; 4]> {
        let mut out: SmallVec<[CellSet<C>; 4]> = (0..n).map(|_| CellSet::new()).collect();
        for ((s, c), &p) in self.slots.into_iter().zip(self.cells).zip(piece_of) {
            out[p].slots.push(s);
            out[p].cells.push(c);
        }
        out
    }
}

/// A scheduling unit: an index set of cells plus the state that is not cell-local.
///
/// The scheduler hands out disjoint `&mut Region`s; structure (which cells, which pins)
/// changes only in [`crate::Regionizer::apply`].
#[derive(Debug)]
pub struct Region<C, P> {
    id: RegionId,
    pub(crate) cells: CellSet<C>,
    pub(crate) part: P,
    /// Active fusion pins, both endpoints owned by this region; sorted by pin key.
    pub(crate) pins: SmallVec<[FusePin; 2]>,
    /// Split watch: per label, the tick since which that component has been apart.
    pub(crate) since: Vec<u64>,
    /// A split check is due: a cell was vacated or a pin removed since the last check, or
    /// the last check found more than one component.
    pub(crate) dirty: bool,
}

impl<C, P> Region<C, P> {
    pub fn id(&self) -> RegionId {
        self.id
    }

    /// The smallest cell: a stable key for scheduling and reports.
    pub fn anchor(&self) -> CellPos {
        self.cells.slots[0].pos
    }

    /// Number of cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Always false for a region inside [`Regions`].
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn cells(&self) -> &CellSet<C> {
        &self.cells
    }

    /// Payload access; the set of cells itself cannot change here.
    pub fn cells_mut(&mut self) -> &mut CellSet<C> {
        &mut self.cells
    }

    pub fn part(&self) -> &P {
        &self.part
    }

    pub fn part_mut(&mut self) -> &mut P {
        &mut self.part
    }

    /// Lends the region's cells and part out (to tick away from the scheduler), leaving the
    /// region empty in place: the cell table keeps pointing at it, lookups through it find
    /// nothing, and [`Regionizer::apply`](crate::Regionizer::apply) must not run on this
    /// dimension until [`restore`](Self::restore) puts them back.
    pub fn lend(&mut self) -> (CellSet<C>, P)
    where
        P: Default,
    {
        (std::mem::replace(&mut self.cells, CellSet::new()), std::mem::take(&mut self.part))
    }

    /// Puts back what [`lend`](Self::lend) took.
    pub fn restore(&mut self, cells: CellSet<C>, part: P) {
        debug_assert!(self.cells.is_empty(), "restoring into a region that is not lent");
        self.cells = cells;
        self.part = part;
    }

    pub fn cells_and_part_mut(&mut self) -> (&mut CellSet<C>, &mut P) {
        (&mut self.cells, &mut self.part)
    }

    pub fn pins(&self) -> &[FusePin] {
        &self.pins
    }

    /// Adds a normalized pin; an existing pin with the same key keeps the later expiry.
    pub(crate) fn add_pin(&mut self, pin: FusePin) {
        match self.pins.binary_search_by_key(&pin.key(), FusePin::key) {
            Ok(i) => self.pins[i].until_tick = self.pins[i].until_tick.max(pin.until_tick),
            Err(i) => self.pins.insert(i, pin),
        }
    }

    /// Renumbers labels densely in cell order so `since` stays bounded by the cell count.
    fn compact_labels(&mut self) {
        let mut map = vec![UNLABELED; self.since.len()];
        let mut since = Vec::new();
        for s in &mut self.cells.slots {
            if s.label != UNLABELED {
                let l = s.label as usize;
                if map[l] == UNLABELED {
                    map[l] = since.len() as u32;
                    since.push(self.since[l]);
                }
                s.label = map[l];
            }
        }
        self.since = since;
    }
}

impl<C, P: RegionPart> Region<C, P> {
    pub(crate) fn new(id: RegionId, pos: CellPos, cell: Box<C>) -> Self {
        let mut cells = CellSet::new();
        cells.insert(pos, cell);
        Self { id, cells, part: P::default(), pins: SmallVec::new(), since: Vec::new(), dirty: false }
    }

    pub(crate) fn from_parts(id: RegionId, cells: CellSet<C>, part: P, tick: u64) -> Self {
        Self { id, cells, part, pins: SmallVec::new(), since: vec![tick], dirty: false }
    }

    /// Absorbs `other`: cells, part (linear merge), pins and split watch.
    pub(crate) fn absorb(&mut self, other: Region<C, P>) {
        let offset = self.since.len() as u32;
        self.cells.merge(other.cells, offset);
        self.since.extend(other.since);
        self.compact_labels();
        P::merge(&mut self.part, other.part);
        for pin in other.pins {
            self.add_pin(pin);
        }
        self.dirty |= other.dirty;
    }

    /// Splits off `n - 1` new regions. `piece_of[i]` is the piece of cell `i` (0 stays).
    pub(crate) fn split_off(
        &mut self,
        piece_of: &[usize],
        new_ids: &[RegionId],
        tick: u64,
    ) -> SmallVec<[Region<C, P>; 4]> {
        let n = new_ids.len() + 1;
        let part = std::mem::take(&mut self.part);
        let mut parts = {
            let lookup = PieceLookup::new(&self.cells, piece_of);
            part.split(&|c| lookup.get(c), n).into_iter()
        };
        assert_eq!(parts.len(), n, "RegionPart::split returned the wrong number of parts");
        let mut pin_piece: SmallVec<[usize; 2]> = SmallVec::new();
        for pin in &self.pins {
            pin_piece.push(self.cells.index_of(pin.a).map_or(0, |i| piece_of[i]));
        }
        let mut sets = std::mem::replace(&mut self.cells, CellSet::new()).partition(piece_of, n).into_iter();
        self.cells = sets.next().unwrap();
        self.part = parts.next().unwrap();
        let mut out: SmallVec<[Region<C, P>; 4]> = new_ids
            .iter()
            .zip(sets.zip(parts))
            .map(|(&id, (mut cells, part))| {
                cells.slots.iter_mut().for_each(|s| s.label = 0);
                Region::from_parts(id, cells, part, tick)
            })
            .collect();
        let pins = std::mem::take(&mut self.pins);
        for (pin, p) in pins.into_iter().zip(pin_piece) {
            if p == 0 { self.pins.push(pin) } else { out[p - 1].pins.push(pin) }
        }
        out
    }
}

/// Cell → split piece for `RegionPart::split`, called once per element: a dense table over
/// the bounding box when that is compact, binary search otherwise. Unknown cells map to 0.
enum PieceLookup<'a, C> {
    Dense { x0: i32, z0: i32, w: usize, h: usize, piece: Vec<u32> },
    Search { cells: &'a CellSet<C>, piece_of: &'a [usize] },
}

impl<'a, C> PieceLookup<'a, C> {
    fn new(cells: &'a CellSet<C>, piece_of: &'a [usize]) -> Self {
        let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for p in cells.positions() {
            (x0, z0, x1, z1) = (x0.min(p.x), z0.min(p.z), x1.max(p.x), z1.max(p.z));
        }
        let (w, h) = (x1.abs_diff(x0) as usize + 1, z1.abs_diff(z0) as usize + 1);
        if w.saturating_mul(h) > (16 * cells.len()).max(4096) {
            return Self::Search { cells, piece_of };
        }
        let mut piece = vec![0; w * h];
        for (p, &i) in cells.positions().zip(piece_of) {
            piece[(p.z - z0) as usize * w + (p.x - x0) as usize] = i as u32;
        }
        Self::Dense { x0, z0, w, h, piece }
    }

    fn get(&self, c: CellPos) -> usize {
        match self {
            Self::Dense { x0, z0, w, h, piece } => {
                let (dx, dz) = (c.x.wrapping_sub(*x0) as u32 as usize, c.z.wrapping_sub(*z0) as u32 as usize);
                if dx < *w && dz < *h { piece[dz * w + dx] as usize } else { 0 }
            }
            Self::Search { cells, piece_of } => cells.index_of(c).map_or(0, |i| piece_of[i]),
        }
    }
}

/// The regionized part of a dimension: the cell table and the live regions.
#[derive(Debug)]
pub struct Regions<C, P> {
    pub(crate) table: CellTable,
    pub(crate) map: BTreeMap<RegionId, Region<C, P>>,
}

impl<C, P> Default for Regions<C, P> {
    fn default() -> Self {
        Self { table: CellTable::default(), map: BTreeMap::new() }
    }
}

impl<C, P> Regions<C, P> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses `seed` for the cell table's hasher (tests vary it to prove order independence).
    pub fn with_hash_seed(seed: u64) -> Self {
        Self { table: CellTable::with_hash_seed(seed), map: BTreeMap::new() }
    }

    pub fn table(&self) -> &CellTable {
        &self.table
    }

    pub fn owner(&self, pos: CellPos) -> Option<RegionId> {
        self.table.get(pos)
    }

    pub fn get(&self, id: RegionId) -> Option<&Region<C, P>> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: RegionId) -> Option<&mut Region<C, P>> {
        self.map.get_mut(&id)
    }

    /// The region owning `pos`.
    pub fn at(&self, pos: CellPos) -> Option<&Region<C, P>> {
        self.map.get(&self.table.get(pos)?)
    }

    pub fn at_mut(&mut self, pos: CellPos) -> Option<&mut Region<C, P>> {
        self.map.get_mut(&self.table.get(pos)?)
    }

    /// Number of regions.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Regions in id order.
    pub fn iter(&self) -> btree_map::Values<'_, RegionId, Region<C, P>> {
        self.map.values()
    }

    pub fn iter_mut(&mut self) -> btree_map::ValuesMut<'_, RegionId, Region<C, P>> {
        self.map.values_mut()
    }

    /// The shared table plus disjoint `&mut` to every region, for the parallel tick.
    pub fn split_mut(&mut self) -> (&CellTable, btree_map::ValuesMut<'_, RegionId, Region<C, P>>) {
        (&self.table, self.map.values_mut())
    }
}
