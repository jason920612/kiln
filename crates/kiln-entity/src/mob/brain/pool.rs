//! The jump candidates of the long jumpers (`LongJumpToRandomPos`'s `jumpCandidates`): a list of
//! (position, weight) from which `WeightedRandom.getRandomItem` takes items one at a time, a
//! pick being the first item whose running weight passes a random number below the total.
//! A pick walks the list in vanilla (and removes the item, shifting the rest); here a Fenwick
//! tree over the weights finds the same item in logarithmic time and a removed item keeps its
//! place with weight 0, so the later picks fall on the same items in the same order.

use crate::math::BlockPos;

#[derive(Clone, Debug, Default)]
pub struct WeightedPool {
    items: Vec<BlockPos>,
    /// The weights of the items still in the pool (0 once taken).
    weights: Vec<i32>,
    /// Fenwick tree over `weights` (1-based).
    tree: Vec<i32>,
    alive: usize,
    total: i64,
}

impl WeightedPool {
    pub fn clear(&mut self) {
        self.items.clear();
        self.weights.clear();
        self.tree.clear();
        self.alive = 0;
        self.total = 0;
    }

    /// Adds an item (weights are at least 1); call [`WeightedPool::build`] after the last.
    pub fn push(&mut self, pos: BlockPos, weight: i32) {
        debug_assert!(weight > 0);
        self.items.push(pos);
        self.weights.push(weight);
        self.alive += 1;
        self.total += weight as i64;
    }

    /// Builds the tree over what was pushed.
    pub fn build(&mut self) {
        let n = self.items.len();
        self.tree.clear();
        self.tree.resize(n + 1, 0);
        for i in 1..=n {
            self.tree[i] += self.weights[i - 1];
            let parent = i + (i & i.wrapping_neg());
            if parent <= n {
                let v = self.tree[i];
                self.tree[parent] += v;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.alive == 0
    }

    /// The sum of the weights left (what `getRandomItem` draws below).
    pub fn total(&self) -> i64 {
        self.total
    }

    /// `WeightedRandom.getWeightedItem(items, r)` with the removal: the item the running weight
    /// passes `r` at (`0 <= r < total`), with its weight.
    pub fn take(&mut self, mut r: i32) -> Option<(BlockPos, i32)> {
        let n = self.items.len();
        let mut pos = 0usize;
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next <= n && self.tree[next] <= r {
                pos = next;
                r -= self.tree[next];
            }
            step >>= 1;
        }
        // `pos` items have a running weight of at most `r`: the next one is the pick.
        if pos >= n {
            return None;
        }
        let w = self.weights[pos];
        let mut i = pos + 1;
        while i <= n {
            self.tree[i] -= w;
            i += i & i.wrapping_neg();
        }
        self.weights[pos] = 0;
        self.alive -= 1;
        self.total -= w as i64;
        Some((self.items[pos], w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The linear walk with removal the pool replaces.
    fn linear(list: &mut Vec<(BlockPos, i32)>, mut r: i32) -> Option<(BlockPos, i32)> {
        for i in 0..list.len() {
            r -= list[i].1;
            if r < 0 {
                return Some(list.remove(i));
            }
        }
        None
    }

    #[test]
    fn picks_what_the_linear_walk_picks() {
        for n in [1usize, 2, 3, 7, 64, 100, 405] {
            let mut list: Vec<(BlockPos, i32)> = (0..n).map(|i| (BlockPos::new(i as i32, 0, 0), 1 + (i as i32 * 7) % 13)).collect();
            let mut pool = WeightedPool::default();
            for &(p, w) in &list {
                pool.push(p, w);
            }
            pool.build();
            let mut seed = 12345u64;
            while !list.is_empty() {
                let total: i64 = list.iter().map(|c| c.1 as i64).sum();
                assert_eq!(pool.total(), total);
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let r = ((seed >> 33) % total as u64) as i32;
                assert_eq!(pool.take(r), linear(&mut list, r), "n {n}");
            }
            assert!(pool.is_empty());
        }
    }
}
