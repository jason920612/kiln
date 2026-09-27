//! `java.util.HashSet<BlockPos>` with Java's iteration order, which tree placement depends
//! on: decorators walk the log and leaf sets (stably sorted by y) and draw randoms per
//! position, and `TreeFeature.updateLeaves` pops positions in iteration order.
//!
//! Mirrors `HashMap`: power-of-two table (16 initially, load factor 0.75), the hash spread
//! `h ^ (h >>> 16)`, bins appended at the tail and split in order on resize. A bin reaching
//! the treeify threshold in a table smaller than 64 resizes instead; treeified bins (larger
//! tables) keep insertion order here, which vanilla's red-black bins do not always do.

use crate::pos::BlockPos;

/// `Vec3i.hashCode`.
fn hash(p: BlockPos) -> u32 {
    let h = p.y.wrapping_add(p.z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(p.x) as u32;
    h ^ (h >> 16)
}

const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct JHashSet {
    bins: Vec<Vec<BlockPos>>,
    len: usize,
}

impl JHashSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn index(&self, h: u32) -> usize {
        h as usize & (self.bins.len() - 1)
    }

    pub fn contains(&self, p: BlockPos) -> bool {
        !self.bins.is_empty() && self.bins[self.index(hash(p))].contains(&p)
    }

    /// `add`: false if already present.
    pub fn insert(&mut self, p: BlockPos) -> bool {
        if self.bins.is_empty() {
            self.bins = vec![Vec::new(); 16];
        }
        let h = hash(p);
        let i = self.index(h);
        if self.bins[i].contains(&p) {
            return false;
        }
        self.bins[i].push(p);
        if self.bins[i].len() > TREEIFY_THRESHOLD && self.bins.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
        } else if self.bins[i].len() > TREEIFY_THRESHOLD {
            eprintln!("JHASHSET TREEIFY {p:?}");
        }
        self.len += 1;
        if self.len > self.bins.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    fn resize(&mut self) {
        let n = self.bins.len();
        let mut bins = vec![Vec::new(); n * 2];
        for (i, bin) in std::mem::take(&mut self.bins).into_iter().enumerate() {
            for p in bin {
                let j = if hash(p) as usize & n == 0 { i } else { i + n };
                bins[j].push(p);
            }
        }
        self.bins = bins;
    }

    /// `iterator().next()` followed by `remove()`.
    pub fn pop_first(&mut self) -> Option<BlockPos> {
        let bin = self.bins.iter_mut().find(|b| !b.is_empty())?;
        self.len -= 1;
        Some(bin.remove(0))
    }

    /// Iteration order.
    pub fn iter(&self) -> impl Iterator<Item = BlockPos> + '_ {
        self.bins.iter().flatten().copied()
    }

    /// `new ObjectArrayList<>(set)` sorted stably by y (`TreeDecorator.Context`).
    pub fn sorted_by_y(&self) -> Vec<BlockPos> {
        let mut v: Vec<BlockPos> = self.iter().collect();
        v.sort_by_key(|p| p.y);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_order() {
        use kiln_javamath::random::{LegacyRandom, RandomSource};
        let mut r = LegacyRandom::new(42);
        let mut s = JHashSet::new();
        for _ in 0..300 {
            let x = r.next_int_bounded(20) - 10;
            let y = 60 + r.next_int_bounded(12);
            let z = r.next_int_bounded(20) - 10;
            s.insert(BlockPos::new(x, y, z));
        }
        for i in 0..12 {
            s.insert(BlockPos::new(-500 + 31 * i, 100 - i, 7));
        }
        let mut out = String::new();
        for p in s.iter() {
            out += &format!("{},{},{};", p.x, p.y, p.z);
        }
        println!("ORDER {out}");
        for _ in 0..5 {
            s.pop_first();
        }
        let mut out = String::new();
        for p in s.iter() {
            out += &format!("{},{},{};", p.x, p.y, p.z);
            if out.len() > 200 {
                break;
            }
        }
        println!("ORDER {out}");
    }

    #[test]
    fn iterates_like_java() {
        // java: new HashSet<>() of these BlockPos iterates in this order.
        let mut s = JHashSet::new();
        for p in [BlockPos::new(1, 64, 1), BlockPos::new(0, 64, 0), BlockPos::new(-1, 64, 2), BlockPos::new(3, 70, -5)] {
            s.insert(p);
        }
        let order: Vec<u32> = s.iter().map(|p| hash(p) & 15).collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(s.len(), 4);
        assert!(!s.insert(BlockPos::new(0, 64, 0)));
    }
}
