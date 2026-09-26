//! `DensityVolume`: a strided box of block positions, the unit of batch evaluation.

/// Positions `min + i * step` for `i` in `0..size` per axis, stored y-fastest, then x,
/// then z (vanilla's buffer layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Volume {
    pub size: [i32; 3],
    pub min: [i32; 3],
    pub step: [i32; 3],
}

impl Volume {
    pub fn new(size: [i32; 3], min: [i32; 3], step: [i32; 3]) -> Self {
        assert!(size.iter().all(|&s| s > 0), "volume size must be positive: {size:?}");
        assert!(step.iter().all(|&s| s > 0), "volume step must be positive: {step:?}");
        Self { size, min, step }
    }

    pub fn blocks(size: [i32; 3], min: [i32; 3]) -> Self {
        Self::new(size, min, [1, 1, 1])
    }

    pub fn len(&self) -> usize {
        (self.size[0] * self.size[1] * self.size[2]) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (y + (x + z * self.size[0]) * self.size[1]) as usize
    }

    #[inline]
    pub fn block_x(&self, i: i32) -> i32 {
        self.min[0].wrapping_add(i.wrapping_mul(self.step[0]))
    }

    #[inline]
    pub fn block_y(&self, i: i32) -> i32 {
        self.min[1].wrapping_add(i.wrapping_mul(self.step[1]))
    }

    #[inline]
    pub fn block_z(&self, i: i32) -> i32 {
        self.min[2].wrapping_add(i.wrapping_mul(self.step[2]))
    }

    /// The last block covered along each axis (`min + size * step - 1`).
    pub fn max_block(&self, axis: usize) -> i32 {
        self.min[axis].wrapping_add(self.size[axis].wrapping_mul(self.step[axis])).wrapping_sub(1)
    }

    /// `DensityVolume.indexOfBlock`: the buffer index of a block position on the volume's
    /// grid, if it is one.
    pub fn index_of_block(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let r = [x.wrapping_sub(self.min[0]), y.wrapping_sub(self.min[1]), z.wrapping_sub(self.min[2])];
        if self.step == [1, 1, 1] {
            return (0..3).all(|a| r[a] >= 0 && r[a] < self.size[a]).then(|| self.index(r[0], r[1], r[2]));
        }
        let on_grid = (0..3).all(|a| {
            r[a] >= 0 && r[a] < self.size[a].wrapping_mul(self.step[a]) && r[a].rem_euclid(self.step[a]) == 0
        });
        on_grid.then(|| self.index(r[0] / self.step[0], r[1] / self.step[1], r[2] / self.step[2]))
    }

    /// Every position in buffer order.
    pub fn positions(&self) -> impl Iterator<Item = [i32; 3]> + '_ {
        (0..self.size[2]).flat_map(move |z| {
            (0..self.size[0])
                .flat_map(move |x| (0..self.size[1]).map(move |y| [self.block_x(x), self.block_y(y), self.block_z(z)]))
        })
    }
}
