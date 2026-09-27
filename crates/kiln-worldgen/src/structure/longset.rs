//! fastutil's `LongOpenHashSet` insertion and iteration order. Vanilla keeps structure
//! references in one and places starts in its iteration order, which depends on the hash
//! layout.

/// `HashCommon.mix(long)`.
fn mix(x: i64) -> i64 {
    let h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64);
    let h = h ^ ((h as u64) >> 32) as i64;
    h ^ ((h as u64) >> 16) as i64
}

/// `HashCommon.arraySize(expected, 0.75)`.
fn array_size(expected: usize) -> usize {
    ((expected as f64 / 0.75).ceil() as usize).next_power_of_two().max(2)
}

/// `HashCommon.maxFill(n, 0.75)`.
fn max_fill(n: usize) -> usize {
    ((n as f64 * 0.75).ceil() as usize).min(n - 1)
}

/// A `LongOpenHashSet` (default capacity, load factor 0.75) without removal.
#[derive(Clone, Debug)]
pub struct LongSet {
    key: Vec<i64>,
    contains_zero: bool,
    size: usize,
    max_fill: usize,
}

impl Default for LongSet {
    fn default() -> Self {
        let n = array_size(16);
        Self { key: vec![0; n], contains_zero: false, size: 0, max_fill: max_fill(n) }
    }
}

impl LongSet {
    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn add(&mut self, k: i64) -> bool {
        if k == 0 {
            if self.contains_zero {
                return false;
            }
            self.contains_zero = true;
        } else {
            let mask = self.key.len() - 1;
            let mut pos = (mix(k) as usize) & mask;
            while self.key[pos] != 0 {
                if self.key[pos] == k {
                    return false;
                }
                pos = (pos + 1) & mask;
            }
            self.key[pos] = k;
        }
        self.size += 1;
        if self.size >= self.max_fill {
            self.rehash(array_size(self.size + 1));
        }
        true
    }

    fn rehash(&mut self, n: usize) {
        let mask = n - 1;
        let mut new = vec![0i64; n];
        let mut i = self.key.len();
        let mut left = self.size - self.contains_zero as usize;
        while left > 0 {
            i -= 1;
            if self.key[i] == 0 {
                continue;
            }
            let mut pos = (mix(self.key[i]) as usize) & mask;
            while new[pos] != 0 {
                pos = (pos + 1) & mask;
            }
            new[pos] = self.key[i];
            left -= 1;
        }
        self.key = new;
        self.max_fill = max_fill(n);
    }

    /// Iteration order: zero first, then the table from the end.
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        let zero = self.contains_zero.then_some(0);
        zero.into_iter().chain(self.key.iter().rev().copied().filter(|&k| k != 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_every_key_once() {
        let mut s = LongSet::default();
        for i in 0..100i64 {
            assert!(s.add(i * 7919 - 50));
        }
        assert!(!s.add(7919 * 3 - 50));
        let mut all: Vec<i64> = s.iter().collect();
        assert_eq!(all.len(), 100);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 100);
    }
}
