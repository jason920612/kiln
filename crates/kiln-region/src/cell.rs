//! Cell positions, their canonical order and the cell → region table.

use crate::RegionId;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

/// A cell is 2^`CELL_SHIFT` × 2^`CELL_SHIFT` chunks, full height.
pub const CELL_SHIFT: i32 = 3;
/// Side length of a cell in blocks.
pub const CELL_BLOCKS: i32 = 16 << CELL_SHIFT;

/// Position of a cell in cell coordinates (chunk coordinate >> `CELL_SHIFT`).
///
/// Cells are ordered by their Morton (Z-order) key over sign-biased coordinates. A region
/// keeps its cells in this order, and its anchor is the smallest cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPos {
    pub x: i32,
    pub z: i32,
}

impl CellPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub const fn of_chunk(chunk_x: i32, chunk_z: i32) -> Self {
        Self { x: chunk_x >> CELL_SHIFT, z: chunk_z >> CELL_SHIFT }
    }

    pub const fn of_block(block_x: i32, block_z: i32) -> Self {
        Self::of_chunk(block_x >> 4, block_z >> 4)
    }

    /// Chebyshev distance in cells.
    pub fn cheb(self, other: CellPos) -> u32 {
        self.x.abs_diff(other.x).max(self.z.abs_diff(other.z))
    }

    pub fn offset(self, dx: i32, dz: i32) -> CellPos {
        CellPos { x: self.x.wrapping_add(dx), z: self.z.wrapping_add(dz) }
    }

    /// The Morton key that defines the order of cells.
    pub fn key(self) -> u64 {
        spread(self.x as u32 ^ 0x8000_0000) | spread(self.z as u32 ^ 0x8000_0000) << 1
    }
}

/// Spreads the bits of `v` into the even bit positions of a u64.
fn spread(v: u32) -> u64 {
    let mut v = v as u64;
    v = (v | v << 16) & 0x0000_FFFF_0000_FFFF;
    v = (v | v << 8) & 0x00FF_00FF_00FF_00FF;
    v = (v | v << 4) & 0x0F0F_0F0F_0F0F_0F0F;
    v = (v | v << 2) & 0x3333_3333_3333_3333;
    (v | v << 1) & 0x5555_5555_5555_5555
}

impl Ord for CellPos {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for CellPos {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for CellPos {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64((self.x as u32 as u64) << 32 | self.z as u32 as u64);
    }
}

impl From<(i32, i32)> for CellPos {
    fn from((x, z): (i32, i32)) -> Self {
        Self { x, z }
    }
}

/// Seedable hasher for cell-keyed maps. Nothing in the regionizer iterates a hash map to
/// make a decision; the seed exists so tests can prove it.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellHashBuilder {
    seed: u64,
}

impl CellHashBuilder {
    pub fn with_seed(seed: u64) -> Self {
        Self { seed }
    }
}

impl BuildHasher for CellHashBuilder {
    type Hasher = CellHasher;
    fn build_hasher(&self) -> CellHasher {
        CellHasher(self.seed)
    }
}

pub struct CellHasher(u64);

impl Hasher for CellHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.0 ^= v;
    }

    fn finish(&self) -> u64 {
        // splitmix64 finalizer
        let mut z = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

pub type CellMap<V> = HashMap<CellPos, V, CellHashBuilder>;

/// Owner of every occupied cell. Rewritten only by [`crate::Regionizer::apply`]; shared
/// read-only while regions tick.
#[derive(Debug, Clone, Default)]
pub struct CellTable {
    owners: CellMap<RegionId>,
}

impl CellTable {
    pub fn with_hash_seed(seed: u64) -> Self {
        Self { owners: CellMap::with_hasher(CellHashBuilder::with_seed(seed)) }
    }

    pub fn get(&self, pos: CellPos) -> Option<RegionId> {
        self.owners.get(&pos).copied()
    }

    pub fn contains(&self, pos: CellPos) -> bool {
        self.owners.contains_key(&pos)
    }

    /// Number of occupied cells.
    pub fn len(&self) -> usize {
        self.owners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    pub(crate) fn set(&mut self, pos: CellPos, id: RegionId) {
        self.owners.insert(pos, id);
    }

    pub(crate) fn remove(&mut self, pos: CellPos) {
        self.owners.remove(&pos);
    }

    /// Unordered iteration, for checks only.
    pub(crate) fn iter_unordered(&self) -> impl Iterator<Item = (CellPos, RegionId)> + '_ {
        self.owners.iter().map(|(&p, &id)| (p, id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morton_order_is_total_and_signed() {
        let a = CellPos::new(-1, -1);
        let b = CellPos::new(0, 0);
        let c = CellPos::new(1, 0);
        let d = CellPos::new(0, 1);
        assert!(a < b && b < c && c < d);
        assert!(CellPos::new(i32::MIN, i32::MIN) < a);
        assert!(CellPos::new(i32::MAX, i32::MAX) > d);
        assert_eq!(CellPos::new(3, -7).key(), CellPos::new(3, -7).key());
        assert_ne!(CellPos::new(3, -7).key(), CellPos::new(-7, 3).key());
    }

    #[test]
    fn chunk_to_cell() {
        assert_eq!(CellPos::of_chunk(7, 8), CellPos::new(0, 1));
        assert_eq!(CellPos::of_chunk(-1, -8), CellPos::new(-1, -1));
        assert_eq!(CellPos::of_chunk(-9, 0), CellPos::new(-2, 0));
        assert_eq!(CellPos::of_block(127, 128), CellPos::new(0, 1));
    }
}
