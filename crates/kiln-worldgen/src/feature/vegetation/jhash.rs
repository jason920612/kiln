//! Iteration order of `java.util.HashSet`, which several features iterate after filling one.

use crate::pos::BlockPos;

/// `Vec3i.hashCode`.
pub fn pos_hash(p: BlockPos) -> i32 {
    p.y.wrapping_add(p.z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(p.x)
}

/// A `HashSet` created with the default capacity and only added to: the distinct items in
/// the order the set iterates them.
///
/// Within a bucket entries keep insertion order (resizes split buckets stably), so the order
/// is a stable sort by bucket in the final table. The table doubles when the size passes 3/4
/// of the capacity, and also when a bucket reaches 9 entries while the table is smaller than
/// 64 (`treeifyBin`). Treeified buckets (9+ colliding entries in a larger table) would put the
/// tree root first; that does not happen for the position sets features build.
#[derive(Clone, Debug)]
pub struct JavaHashSet<T> {
    items: Vec<(i32, T)>,
    index: std::collections::HashSet<T>,
    cap: usize,
    counts: Vec<u32>,
}

impl<T: Eq + std::hash::Hash + Copy> JavaHashSet<T> {
    pub fn new() -> Self {
        Self { items: Vec::new(), index: Default::default(), cap: 16, counts: vec![0; 16] }
    }

    fn spread(h: i32) -> i32 {
        h ^ ((h as u32) >> 16) as i32
    }

    fn bucket(&self, h: i32) -> usize {
        Self::spread(h) as usize & (self.cap - 1)
    }

    fn resize(&mut self) {
        self.cap *= 2;
        self.counts = vec![0; self.cap];
        for i in 0..self.items.len() {
            let b = self.bucket(self.items[i].0);
            self.counts[b] += 1;
        }
    }

    pub fn contains(&self, item: &T) -> bool {
        self.index.contains(item)
    }

    /// `add`: false if the item was already present.
    pub fn insert(&mut self, hash: i32, item: T) -> bool {
        if !self.index.insert(item) {
            return false;
        }
        let b = self.bucket(hash);
        self.items.push((hash, item));
        self.counts[b] += 1;
        if self.counts[b] >= 9 && self.cap < 64 {
            self.resize();
        }
        if self.items.len() > self.cap * 3 / 4 {
            self.resize();
        }
        true
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The items in iteration order.
    pub fn iter_order(&self) -> Vec<T> {
        let mut v: Vec<(usize, T)> = self.items.iter().map(|&(h, t)| (self.bucket(h), t)).collect();
        v.sort_by_key(|(b, _)| *b);
        v.into_iter().map(|(_, t)| t).collect()
    }
}

impl<T: Eq + std::hash::Hash + Copy> Default for JavaHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A `HashSet<BlockPos>`.
#[derive(Clone, Debug, Default)]
pub struct PosSet(JavaHashSet<BlockPos>);

impl PosSet {
    pub fn new() -> Self {
        Self(JavaHashSet::new())
    }

    pub fn insert(&mut self, p: BlockPos) -> bool {
        self.0.insert(pos_hash(p), p)
    }

    pub fn contains(&self, p: BlockPos) -> bool {
        self.0.contains(&p)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter_order(&self) -> Vec<BlockPos> {
        self.0.iter_order()
    }
}
