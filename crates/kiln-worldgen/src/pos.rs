//! Block positions (`BlockPos`).

use crate::block_facts::Dir;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    #[inline]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self { x: self.x + dx, y: self.y + dy, z: self.z + dz }
    }

    #[inline]
    pub fn above(self) -> Self {
        self.offset(0, 1, 0)
    }

    #[inline]
    pub fn below(self) -> Self {
        self.offset(0, -1, 0)
    }

    #[inline]
    pub fn above_n(self, n: i32) -> Self {
        self.offset(0, n, 0)
    }

    #[inline]
    pub fn below_n(self, n: i32) -> Self {
        self.offset(0, -n, 0)
    }

    #[inline]
    pub fn relative(self, dir: Dir) -> Self {
        let (dx, dy, dz) = dir.offset();
        self.offset(dx, dy, dz)
    }

    #[inline]
    pub fn relative_n(self, dir: Dir, n: i32) -> Self {
        let (dx, dy, dz) = dir.offset();
        self.offset(dx * n, dy * n, dz * n)
    }

    #[inline]
    pub fn at_y(self, y: i32) -> Self {
        Self { y, ..self }
    }

    #[inline]
    pub fn chunk_x(self) -> i32 {
        self.x >> 4
    }

    #[inline]
    pub fn chunk_z(self) -> i32 {
        self.z >> 4
    }

    /// `Vec3i.distSqr`.
    pub fn dist_sqr(self, o: BlockPos) -> f64 {
        let (dx, dy, dz) = ((self.x - o.x) as f64, (self.y - o.y) as f64, (self.z - o.z) as f64);
        dx * dx + dy * dy + dz * dz
    }

    /// `Vec3i.distManhattan`.
    pub fn dist_manhattan(self, o: BlockPos) -> i32 {
        (self.x - o.x).abs() + (self.y - o.y).abs() + (self.z - o.z).abs()
    }

    /// `BlockPos.asLong`.
    pub fn as_long(self) -> i64 {
        ((self.x as i64 & 0x3FF_FFFF) << 38) | ((self.z as i64 & 0x3FF_FFFF) << 12) | (self.y as i64 & 0xFFF)
    }
}
