//! Per-region state that follows merges and splits, and reference implementations.

use crate::CellPos;
use smallvec::SmallVec;

/// State owned by a region (not by a cell) that must follow topology changes.
///
/// Both operations must be deterministic and must neither lose nor duplicate elements:
/// `merge` is a linear merge in a key order that does not depend on the partition, and
/// `split` is a stable partition by the new owner of each element's cell.
pub trait RegionPart: Default {
    /// Moves everything in `from` into `into`.
    fn merge(into: &mut Self, from: Self);

    /// Partitions `self` into `n` parts; `owner_of(cell)` is the index (`< n`) of the part
    /// that owns `cell`. Part 0 stays with the region that keeps its id.
    fn split(self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]>;

    /// Number of cell-bound elements (entities, messages, ...), for the conservation check.
    fn count(&self) -> usize {
        0
    }

    /// Calls `f` with the cell of every cell-bound element, for the routing check: each
    /// element must sit in the region that owns its cell.
    fn for_each_cell(&self, _f: &mut dyn FnMut(CellPos)) {}
}

impl RegionPart for () {
    fn merge(_: &mut Self, _: Self) {}
    fn split(self, _: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        (0..n).map(|_| ()).collect()
    }
}

macro_rules! tuple_part {
    ($($t:ident $i:tt),+) => {
        impl<$($t: RegionPart),+> RegionPart for ($($t,)+) {
            fn merge(into: &mut Self, from: Self) {
                $($t::merge(&mut into.$i, from.$i);)+
            }

            #[allow(non_snake_case)]
            fn split(self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
                $(let mut $t = self.$i.split(owner_of, n).into_iter();)+
                (0..n).map(|_| ($($t.next().expect("split returned too few parts"),)+)).collect()
            }

            fn count(&self) -> usize {
                0 $(+ self.$i.count())+
            }

            fn for_each_cell(&self, f: &mut dyn FnMut(CellPos)) {
                $(self.$i.for_each_cell(f);)+
            }
        }
    };
}

tuple_part!(A 0);
tuple_part!(A 0, B 1);
tuple_part!(A 0, B 1, C 2);
tuple_part!(A 0, B 1, C 2, D 3);

fn empty_parts<T: Default>(n: usize) -> SmallVec<[T; 4]> {
    (0..n).map(|_| T::default()).collect()
}

/// Merges two lists sorted by `key` into one; on equal keys `a` comes first.
fn merge_sorted<T, K: Ord>(a: Vec<T>, b: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    match (a.last(), b.first()) {
        (None, _) => return b,
        (_, None) => return a,
        (Some(x), Some(y)) if key(x) <= key(y) => {
            let mut a = a;
            a.extend(b);
            return a;
        }
        _ => {}
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut a = a.into_iter().peekable();
    let mut b = b.into_iter().peekable();
    while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
        if key(y) < key(x) {
            out.push(b.next().unwrap());
        } else {
            out.push(a.next().unwrap());
        }
    }
    out.extend(a);
    out.extend(b);
    out
}

/// Stable partition of `items` by the owner of each item's cell.
fn partition<T>(
    items: Vec<T>,
    cell: impl Fn(&T) -> CellPos,
    owner_of: &dyn Fn(CellPos) -> usize,
    n: usize,
) -> SmallVec<[Vec<T>; 4]> {
    let mut out: SmallVec<[Vec<T>; 4]> = empty_parts(n);
    for item in items {
        out[owner_of(cell(&item))].push(item);
    }
    out
}

/// One element of a [`TickList`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickEntry<T> {
    /// Global, never reused sequence number: the tick order.
    pub seq: u64,
    /// Cell the element currently lives in.
    pub cell: CellPos,
    pub value: T,
}

/// Elements ticked in global sequence order (the shape of `TickOrder` and `BeOrder`).
///
/// Merge is a linear merge by `seq`; split is a stable partition by each element's cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickList<T> {
    items: Vec<TickEntry<T>>,
}

impl<T> Default for TickList<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T> TickList<T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, TickEntry<T>> {
        self.items.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, TickEntry<T>> {
        self.items.iter_mut()
    }

    fn find(&self, seq: u64) -> Result<usize, usize> {
        self.items.binary_search_by_key(&seq, |e| e.seq)
    }

    /// Inserts at the position of `seq` (appending is O(1)). Returns false and leaves the
    /// list unchanged if `seq` is already present.
    pub fn insert(&mut self, seq: u64, cell: CellPos, value: T) -> bool {
        if self.items.last().is_none_or(|e| e.seq < seq) {
            self.items.push(TickEntry { seq, cell, value });
            return true;
        }
        match self.find(seq) {
            Ok(_) => false,
            Err(i) => {
                self.items.insert(i, TickEntry { seq, cell, value });
                true
            }
        }
    }

    pub fn get(&self, seq: u64) -> Option<&TickEntry<T>> {
        self.find(seq).ok().map(|i| &self.items[i])
    }

    pub fn get_mut(&mut self, seq: u64) -> Option<&mut TickEntry<T>> {
        self.find(seq).ok().map(|i| &mut self.items[i])
    }

    pub fn remove(&mut self, seq: u64) -> Option<TickEntry<T>> {
        self.find(seq).ok().map(|i| self.items.remove(i))
    }

    pub fn retain(&mut self, f: impl FnMut(&TickEntry<T>) -> bool) {
        self.items.retain(f);
    }
}

impl<T> RegionPart for TickList<T> {
    fn merge(into: &mut Self, from: Self) {
        let a = std::mem::take(&mut into.items);
        into.items = merge_sorted(a, from.items, |e| e.seq);
    }

    fn split(self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        partition(self.items, |e| e.cell, owner_of, n).into_iter().map(|items| Self { items }).collect()
    }

    fn count(&self) -> usize {
        self.items.len()
    }

    fn for_each_cell(&self, f: &mut dyn FnMut(CellPos)) {
        self.items.iter().for_each(|e| f(e.cell));
    }
}

/// Application order of cross-region messages; independent of the region partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MsgKey {
    pub src_tick: u64,
    pub src_cell: CellPos,
    pub seq: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Letter<M> {
    pub key: MsgKey,
    /// Cell the message is addressed to; the region owning it holds the message.
    pub target: CellPos,
    pub body: M,
}

/// Position-addressed messages awaiting delivery, kept in `MsgKey` order.
///
/// Merge is a linear merge by key; split routes each message to its target cell's owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbox<M> {
    items: Vec<Letter<M>>,
}

impl<M> Default for Inbox<M> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<M> Inbox<M> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Letter<M>> {
        self.items.iter()
    }

    /// Adds a message; returns false if one with the same key is already queued.
    pub fn push(&mut self, key: MsgKey, target: CellPos, body: M) -> bool {
        if self.items.last().is_none_or(|l| l.key < key) {
            self.items.push(Letter { key, target, body });
            return true;
        }
        match self.items.binary_search_by_key(&key, |l| l.key) {
            Ok(_) => false,
            Err(i) => {
                self.items.insert(i, Letter { key, target, body });
                true
            }
        }
    }

    /// Takes every queued message, in key order.
    pub fn drain(&mut self) -> std::vec::Drain<'_, Letter<M>> {
        self.items.drain(..)
    }

    pub fn retain(&mut self, f: impl FnMut(&Letter<M>) -> bool) {
        self.items.retain(f);
    }
}

impl<M> RegionPart for Inbox<M> {
    fn merge(into: &mut Self, from: Self) {
        let a = std::mem::take(&mut into.items);
        into.items = merge_sorted(a, from.items, |l| l.key);
    }

    fn split(self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        partition(self.items, |l| l.target, owner_of, n).into_iter().map(|items| Self { items }).collect()
    }

    fn count(&self) -> usize {
        self.items.len()
    }

    fn for_each_cell(&self, f: &mut dyn FnMut(CellPos)) {
        self.items.iter().for_each(|l| f(l.target));
    }
}

/// Monotonic counters (sub-tick, entity-local, message seq): merge takes the maximum,
/// split copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxCounters<const N: usize>(pub [u64; N]);

impl<const N: usize> Default for MaxCounters<N> {
    fn default() -> Self {
        Self([0; N])
    }
}

impl<const N: usize> RegionPart for MaxCounters<N> {
    fn merge(into: &mut Self, from: Self) {
        for (a, b) in into.0.iter_mut().zip(from.0) {
            *a = (*a).max(b);
        }
    }

    fn split(self, _: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        (0..n).map(|_| self).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(x: i32) -> CellPos {
        CellPos::new(x, 0)
    }

    #[test]
    fn tick_list_merge_is_linear_by_seq() {
        let mut a = TickList::default();
        let mut b = TickList::default();
        for s in [1, 4, 5, 9] {
            a.insert(s, c(0), s);
        }
        for s in [2, 3, 6, 10] {
            b.insert(s, c(1), s);
        }
        RegionPart::merge(&mut a, b);
        let seqs: Vec<u64> = a.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5, 6, 9, 10]);
        assert!(!a.insert(4, c(0), 0));
    }

    #[test]
    fn tick_list_split_is_stable() {
        let mut a = TickList::default();
        for s in 0..10u64 {
            a.insert(s, c((s % 3) as i32), s);
        }
        let parts = a.split(&|cell| if cell.x == 1 { 1 } else { 0 }, 2);
        let p0: Vec<u64> = parts[0].iter().map(|e| e.seq).collect();
        let p1: Vec<u64> = parts[1].iter().map(|e| e.seq).collect();
        assert_eq!(p0, [0, 2, 3, 5, 6, 8, 9]);
        assert_eq!(p1, [1, 4, 7]);
    }

    #[test]
    fn inbox_orders_by_key_and_routes_by_target() {
        let key = |t, x, s| MsgKey { src_tick: t, src_cell: c(x), seq: s };
        let mut a = Inbox::default();
        a.push(key(5, 0, 0), c(0), 'a');
        a.push(key(3, 9, 1), c(1), 'b');
        let mut b = Inbox::default();
        b.push(key(4, 0, 0), c(1), 'c');
        RegionPart::merge(&mut a, b);
        let order: String = a.iter().map(|l| l.body).collect();
        assert_eq!(order, "bca");
        let parts = a.split(&|cell| cell.x as usize, 2);
        assert_eq!(parts[0].iter().map(|l| l.body).collect::<String>(), "a");
        assert_eq!(parts[1].iter().map(|l| l.body).collect::<String>(), "bc");
    }

    #[test]
    fn counters_merge_max_split_copy() {
        let mut a = MaxCounters([3, 9]);
        RegionPart::merge(&mut a, MaxCounters([5, 1]));
        assert_eq!(a.0, [5, 9]);
        let parts = a.split(&|_| 0, 3);
        assert!(parts.iter().all(|p| p.0 == [5, 9]));
    }

    #[test]
    fn tuples_split_componentwise() {
        let mut t = TickList::default();
        t.insert(1, c(1), ());
        let mut i = Inbox::default();
        i.push(MsgKey { src_tick: 0, src_cell: c(0), seq: 0 }, c(0), ());
        let parts = (t, i, MaxCounters([7])).split(&|cell| cell.x as usize, 2);
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].0.len(), parts[0].1.len()), (0, 1));
        assert_eq!((parts[1].0.len(), parts[1].1.len()), (1, 0));
        assert_eq!(parts[1].2.0, [7]);
        assert_eq!(parts[0].count() + parts[1].count(), 2);
    }
}
