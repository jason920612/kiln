//! Iteration order of a `java.util.HashMap` keyed by `BlockPos`, for behaviour that walks a
//! vanilla hash map (piston moves). Keys hash like `Vec3i.hashCode`, spread by
//! `HashMap.hash`; buckets keep insertion order and split in order on resize.
//!
//! Bins that vanilla would turn into trees (9+ colliding keys in a table of 64+ buckets) keep
//! list order here; small maps never get there.

use crate::pos::BlockPos;

pub struct JavaHashMap<V> {
    table: Vec<Vec<(BlockPos, V)>>,
    size: usize,
}

impl<V> Default for JavaHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

fn spread(p: BlockPos) -> u32 {
    let h = p.java_hash() as u32;
    h ^ (h >> 16)
}

impl<V> JavaHashMap<V> {
    /// `new HashMap<>()`: 16 buckets once the first key goes in, load factor 0.75.
    pub fn new() -> Self {
        Self { table: (0..16).map(|_| Vec::new()).collect(), size: 0 }
    }

    fn bucket(&self, k: BlockPos) -> usize {
        spread(k) as usize & (self.table.len() - 1)
    }

    /// `HashMap.put`.
    pub fn put(&mut self, k: BlockPos, v: V) {
        let i = self.bucket(k);
        if let Some(e) = self.table[i].iter_mut().find(|e| e.0 == k) {
            e.1 = v;
            return;
        }
        let before = self.table[i].len();
        self.table[i].push((k, v));
        // `treeifyBin` resizes instead while the table is under 64 buckets.
        if before >= 8 && self.table.len() < 64 {
            self.resize();
        }
        self.size += 1;
        if self.size > self.table.len() / 4 * 3 {
            self.resize();
        }
    }

    /// `HashMap.remove`.
    pub fn remove(&mut self, k: BlockPos) -> Option<V> {
        let i = self.bucket(k);
        let j = self.table[i].iter().position(|e| e.0 == k)?;
        self.size -= 1;
        Some(self.table[i].remove(j).1)
    }

    fn resize(&mut self) {
        let old = std::mem::take(&mut self.table);
        self.table = (0..old.len() * 2).map(|_| Vec::new()).collect();
        for (k, v) in old.into_iter().flatten() {
            let i = self.bucket(k);
            self.table[i].push((k, v));
        }
    }

    /// Entries in `entrySet()` / `keySet()` order.
    pub fn iter(&self) -> impl Iterator<Item = (BlockPos, &V)> {
        self.table.iter().flatten().map(|(k, v)| (*k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_by_bucket_then_insertion() {
        let mut m = JavaHashMap::new();
        // Hashes 0, 16 and 1 land in buckets 0, 0 and 1.
        m.put(BlockPos::new(16, 0, 0), 'a');
        m.put(BlockPos::new(1, 0, 0), 'b');
        m.put(BlockPos::new(0, 0, 0), 'c');
        let keys: Vec<char> = m.iter().map(|(_, v)| *v).collect();
        assert_eq!(keys, ['a', 'c', 'b']);
        assert_eq!(m.remove(BlockPos::new(16, 0, 0)), Some('a'));
        let keys: Vec<char> = m.iter().map(|(_, v)| *v).collect();
        assert_eq!(keys, ['c', 'b']);
    }

    #[test]
    fn resizes_past_twelve_entries() {
        let mut m = JavaHashMap::new();
        for x in 0..13 {
            m.put(BlockPos::new(x * 16, 0, 0), x);
        }
        // 32 buckets now: hashes 0, 32, 64, ... share bucket 0; 16, 48, ... bucket 16.
        let order: Vec<i32> = m.iter().map(|(_, v)| *v).collect();
        assert_eq!(order, [0, 2, 4, 6, 8, 10, 12, 1, 3, 5, 7, 9, 11]);
    }
}
