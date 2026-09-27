//! Vanilla's `VoxelShape`: per-axis coordinate lists over a grid of full or empty cells.
//!
//! Collision walks this exact structure (`VoxelShape.collideX`, `Shapes.joinIsNotEmpty`), so the
//! shapes are extracted from the game as-is rather than as box lists: results then match in the
//! edge cases too (an entity already overlapping a shape, touching within 1e-7).

use crate::math::{Aabb, Axis, floor, jmax, jmin};
use std::borrow::Cow;
use std::sync::OnceLock;

#[derive(Debug)]
pub struct Shape {
    coords: [Box<[f64]>; 3],
    full: Box<[bool]>,
    size: [usize; 3],
    /// First full cell index per axis (`size` when empty).
    first_full: [usize; 3],
    /// One past the last full cell index per axis (0 when empty).
    last_full: [usize; 3],
    /// `CubeVoxelShape`'s arithmetic `findIndex`, for unmoved `Shapes.create` cubes.
    cube: bool,
    boxes: OnceLock<Vec<Aabb>>,
}

impl Clone for Shape {
    fn clone(&self) -> Self {
        Shape {
            coords: self.coords.clone(),
            full: self.full.clone(),
            size: self.size,
            first_full: self.first_full,
            last_full: self.last_full,
            cube: self.cube,
            boxes: OnceLock::new(),
        }
    }
}

impl Shape {
    /// `coords` per axis; `full` in x-major, then y, then z order.
    pub fn new(coords: [Vec<f64>; 3], full: &[bool]) -> Self {
        let size = [coords[0].len().saturating_sub(1), coords[1].len().saturating_sub(1), coords[2].len().saturating_sub(1)];
        assert_eq!(full.len(), size[0] * size[1] * size[2], "shape cell count");
        let mut first_full = size;
        let mut last_full = [0; 3];
        for x in 0..size[0] {
            for y in 0..size[1] {
                for z in 0..size[2] {
                    if full[(x * size[1] + y) * size[2] + z] {
                        for (a, i) in [x, y, z].into_iter().enumerate() {
                            first_full[a] = first_full[a].min(i);
                            last_full[a] = last_full[a].max(i + 1);
                        }
                    }
                }
            }
        }
        Shape {
            coords: coords.map(Vec::into_boxed_slice),
            full: full.into(),
            size,
            first_full,
            last_full,
            cube: false,
            boxes: OnceLock::new(),
        }
    }

    /// `Shapes.create(AABB)`: empty below 1e-7 extent, a `CubeVoxelShape` when the box lies on a
    /// 1/2^n grid (n ≤ 3) inside the unit cube, otherwise a one-cell `ArrayVoxelShape`.
    pub fn from_box(b: &Aabb) -> Option<Shape> {
        if b.max_x - b.min_x < 1.0e-7 || b.max_y - b.min_y < 1.0e-7 || b.max_z - b.min_z < 1.0e-7 {
            return None;
        }
        let bits = [find_bits(b.min_x, b.max_x), find_bits(b.min_y, b.max_y), find_bits(b.min_z, b.max_z)];
        if bits.iter().any(|&v| v < 0) {
            return Some(Shape::new([vec![b.min_x, b.max_x], vec![b.min_y, b.max_y], vec![b.min_z, b.max_z]], &[true]));
        }
        let n = bits.map(|v| 1usize << v);
        let lo = [b.min_x, b.min_y, b.min_z];
        let hi = [b.max_x, b.max_y, b.max_z];
        let range: [(usize, usize); 3] =
            std::array::from_fn(|a| (java_round(lo[a] * n[a] as f64) as usize, java_round(hi[a] * n[a] as f64) as usize));
        let coords = n.map(|parts| (0..=parts).map(|i| i as f64 / parts as f64).collect::<Vec<_>>());
        let mut full = vec![false; n[0] * n[1] * n[2]];
        for x in range[0].0..range[0].1 {
            for y in range[1].0..range[1].1 {
                for z in range[2].0..range[2].1 {
                    full[(x * n[1] + y) * n[2] + z] = true;
                }
            }
        }
        let mut s = Shape::new(coords, &full);
        s.cube = true;
        Some(s)
    }

    /// `VoxelShape.move(x, y, z)` as a new shape (coordinates shifted like `OffsetDoubleList`).
    pub fn moved(&self, x: f64, y: f64, z: f64) -> Shape {
        let mut s = self.clone();
        for (a, off) in [x, y, z].into_iter().enumerate() {
            s.coords[a] = s.coords[a].iter().map(|c| c + off).collect();
        }
        s.cube = false;
        s
    }

    pub fn is_empty(&self) -> bool {
        self.first_full[0] >= self.size[0]
    }

    pub fn size(&self, axis: Axis) -> usize {
        self.size[axis as usize]
    }

    pub fn coords(&self, axis: Axis) -> &[f64] {
        &self.coords[axis as usize]
    }

    fn is_full(&self, x: usize, y: usize, z: usize) -> bool {
        self.full[(x * self.size[1] + y) * self.size[2] + z]
    }

    /// `DiscreteVoxelShape.isFullWide`: out-of-range cells are empty.
    pub fn is_full_wide(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0
            && y >= 0
            && z >= 0
            && (x as usize) < self.size[0]
            && (y as usize) < self.size[1]
            && (z as usize) < self.size[2]
            && self.is_full(x as usize, y as usize, z as usize)
    }

    /// `VoxelShape.min(axis)` of the shape moved by `off`.
    pub fn min(&self, axis: Axis, off: f64) -> f64 {
        let a = axis as usize;
        let i = self.first_full[a];
        if i >= self.size[a] { f64::INFINITY } else { self.coords[a][i] + off }
    }

    /// `VoxelShape.max(axis)` of the shape moved by `off`.
    pub fn max(&self, axis: Axis, off: f64) -> f64 {
        let a = axis as usize;
        let i = self.last_full[a];
        if i == 0 { f64::NEG_INFINITY } else { self.coords[a][i] + off }
    }

    /// `VoxelShape.findIndex`: the cell containing `v` along `axis`, -1 ..= size.
    fn find_index(&self, axis: Axis, v: f64, off: f64) -> i32 {
        let a = axis as usize;
        if self.cube && off == 0.0 {
            let n = self.size[a] as f64;
            return floor(jmin(jmax(v * n, -1.0), n));
        }
        let coords = &self.coords[a];
        // Mth.binarySearch(0, size + 1, i -> v < coord(i)) - 1
        let (mut from, mut len) = (0usize, coords.len());
        while len > 0 {
            let half = len / 2;
            let mid = from + half;
            if v < coords[mid] + off {
                len = half;
            } else {
                from = mid + 1;
                len -= half + 1;
            }
        }
        from as i32 - 1
    }

    /// Whether the (unmoved) shape's cell containing the local point is full (`VoxelShape.clip`).
    pub fn contains_point(&self, x: f64, y: f64, z: f64) -> bool {
        self.is_full_wide(self.find_index(Axis::X, x, 0.0), self.find_index(Axis::Y, y, 0.0), self.find_index(Axis::Z, z, 0.0))
    }

    /// `VoxelShape.collide(axis, box, distance)` for this shape moved by `off`: how far `bx` can
    /// move along `axis` (up to `distance`) before hitting it.
    pub fn collide(&self, axis: Axis, bx: &Aabb, mut distance: f64, off: [f64; 3]) -> f64 {
        if self.is_empty() {
            return distance;
        }
        if distance.abs() < 1.0e-7 {
            return 0.0;
        }
        // The cycled axes: `ax` is the movement axis, then the two others in vanilla's order.
        let (ax, ay, az) = match axis {
            Axis::X => (Axis::X, Axis::Y, Axis::Z),
            Axis::Y => (Axis::Y, Axis::Z, Axis::X),
            Axis::Z => (Axis::Z, Axis::X, Axis::Y),
        };
        let (ox, oy, oz) = (off[ax as usize], off[ay as usize], off[az as usize]);
        let max_x = bx.max(ax);
        let min_x = bx.min(ax);
        let x_first = self.find_index(ax, min_x + 1.0e-7, ox);
        let x_last = self.find_index(ax, max_x - 1.0e-7, ox);
        let y_first = self.find_index(ay, bx.min(ay) + 1.0e-7, oy).max(0);
        let y_last = (self.size(ay) as i32).min(self.find_index(ay, bx.max(ay) - 1.0e-7, oy) + 1);
        let z_first = self.find_index(az, bx.min(az) + 1.0e-7, oz).max(0);
        let z_last = (self.size(az) as i32).min(self.find_index(az, bx.max(az) - 1.0e-7, oz) + 1);
        let x_size = self.size(ax) as i32;
        let cell = |x: i32, y: i32, z: i32| -> bool {
            let mut p = [0i32; 3];
            p[ax as usize] = x;
            p[ay as usize] = y;
            p[az as usize] = z;
            self.is_full_wide(p[0], p[1], p[2])
        };
        if distance > 0.0 {
            for x in x_last + 1..x_size {
                for y in y_first..y_last {
                    for z in z_first..z_last {
                        if cell(x, y, z) {
                            let d = self.coords[ax as usize][x as usize] + ox - max_x;
                            if d >= -1.0e-7 {
                                distance = jmin(distance, d);
                            }
                            return distance;
                        }
                    }
                }
            }
        } else if distance < 0.0 {
            let mut x = x_first - 1;
            while x >= 0 {
                for y in y_first..y_last {
                    for z in z_first..z_last {
                        if cell(x, y, z) {
                            let d = self.coords[ax as usize][x as usize + 1] + ox - min_x;
                            if d <= 1.0e-7 {
                                distance = jmax(distance, d);
                            }
                            return distance;
                        }
                    }
                }
                x -= 1;
            }
        }
        distance
    }

    /// `VoxelShape.toAabbs()` in shape-local coordinates (`forAllBoxes` with merging).
    pub fn boxes(&self) -> &[Aabb] {
        self.boxes.get_or_init(|| {
            let mut out = Vec::new();
            self.for_all_boxes(|x0, y0, z0, x1, y1, z1| {
                let c = &self.coords;
                out.push(Aabb::new(c[0][x0], c[1][y0], c[2][z0], c[0][x1], c[1][y1], c[2][z1]));
            });
            out
        })
    }

    /// `BitSetDiscreteVoxelShape.forAllBoxes(consumer, true)`: greedy z-strip, then x, then y
    /// merging over a scratch copy of the cells.
    fn for_all_boxes(&self, mut consume: impl FnMut(usize, usize, usize, usize, usize, usize)) {
        let [xs, ys, zs] = self.size;
        let mut bits = self.full.to_vec();
        let idx = |x: usize, y: usize, z: usize| (x * ys + y) * zs + z;
        let strip_full = |bits: &[bool], z0: usize, z1: usize, x: usize, y: usize| -> bool {
            x < xs && y < ys && (z0..z1).all(|z| bits[idx(x, y, z)])
        };
        let clear_strip = |bits: &mut [bool], z0: usize, z1: usize, x: usize, y: usize| {
            for z in z0..z1 {
                bits[idx(x, y, z)] = false;
            }
        };
        for y in 0..ys {
            for x in 0..xs {
                let mut start: Option<usize> = None;
                for z in 0..=zs {
                    if z < zs && bits[idx(x, y, z)] {
                        start.get_or_insert(z);
                    } else if let Some(z0) = start.take() {
                        let mut x2 = x;
                        let mut y2 = y;
                        clear_strip(&mut bits, z0, z, x, y);
                        while strip_full(&bits, z0, z, x2 + 1, y) {
                            clear_strip(&mut bits, z0, z, x2 + 1, y);
                            x2 += 1;
                        }
                        while (x..=x2).all(|xi| strip_full(&bits, z0, z, xi, y2 + 1)) {
                            for xi in x..=x2 {
                                clear_strip(&mut bits, z0, z, xi, y2 + 1);
                            }
                            y2 += 1;
                        }
                        consume(x, y, z0, x2 + 1, y2 + 1, z);
                    }
                }
            }
        }
    }
}

/// `Shapes.findBits`: the grid resolution (log2, 0..=3) a range snaps to, or -1.
fn find_bits(min: f64, max: f64) -> i32 {
    if min < -1.0e-7 || max > 1.0000001 {
        return -1;
    }
    for bits in 0..=3 {
        let n = (1 << bits) as f64;
        let a = min * n;
        let b = max * n;
        let snap_a = (a - java_round(a) as f64).abs() < 1.0e-7 * n;
        let snap_b = (b - java_round(b) as f64).abs() < 1.0e-7 * n;
        if snap_a && snap_b {
            return bits;
        }
    }
    -1
}

/// `Math.round(double)`: round half up, computed on the bits like the JDK.
fn java_round(a: f64) -> i64 {
    let bits = a.to_bits() as i64;
    let biased_exp = (bits & 0x7FF0_0000_0000_0000) >> 52;
    let shift = (53 - 2 + 1023) - biased_exp;
    if shift & -64 == 0 {
        let mut r = (bits & 0x000F_FFFF_FFFF_FFFF) | 0x0010_0000_0000_0000;
        if bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        a as i64
    }
}

/// A shape placed in the world: vanilla's `shape.move(pos)` (coordinates plus an offset).
#[derive(Clone, Debug)]
pub struct Collider {
    pub shape: Cow<'static, Shape>,
    pub offset: [f64; 3],
}

impl Collider {
    pub fn at(shape: &'static Shape, x: f64, y: f64, z: f64) -> Self {
        Collider { shape: Cow::Borrowed(shape), offset: [x, y, z] }
    }

    /// `Shapes.create(box)`, unmoved.
    pub fn from_box(b: &Aabb) -> Option<Self> {
        Shape::from_box(b).map(|s| Collider { shape: Cow::Owned(s), offset: [0.0; 3] })
    }

    pub fn collide(&self, axis: Axis, bx: &Aabb, distance: f64) -> f64 {
        self.shape.collide(axis, bx, distance, self.offset)
    }

    /// `getCoords(axis)` of the placed shape.
    pub fn coords(&self, axis: Axis) -> impl Iterator<Item = f64> + '_ {
        let off = self.offset[axis as usize];
        self.shape.coords(axis).iter().map(move |c| c + off)
    }

    pub fn boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        let [x, y, z] = self.offset;
        self.shape.boxes().iter().map(move |b| b.offset(x, y, z))
    }
}

/// `Shapes.collide(axis, box, shapes, distance)`.
pub fn collide_all(axis: Axis, bx: &Aabb, shapes: &[Collider], mut distance: f64) -> f64 {
    for s in shapes {
        if distance.abs() < 1.0e-7 {
            return 0.0;
        }
        distance = s.collide(axis, bx, distance);
    }
    distance
}

/// `Shapes.create(box)` without allocating: the swept box collision tests join against.
#[derive(Clone, Copy, Debug)]
pub struct BoxShape {
    coords: [[f64; 9]; 3],
    len: [usize; 3],
    /// Full cells per axis: `lo..hi`.
    lo: [usize; 3],
    hi: [usize; 3],
}

impl BoxShape {
    /// `None` for vanilla's `Shapes.empty()` (an extent below 1e-7).
    pub fn new(b: &Aabb) -> Option<BoxShape> {
        if b.max_x - b.min_x < 1.0e-7 || b.max_y - b.min_y < 1.0e-7 || b.max_z - b.min_z < 1.0e-7 {
            return None;
        }
        let lo_v = [b.min_x, b.min_y, b.min_z];
        let hi_v = [b.max_x, b.max_y, b.max_z];
        let bits = [find_bits(b.min_x, b.max_x), find_bits(b.min_y, b.max_y), find_bits(b.min_z, b.max_z)];
        let mut s = BoxShape { coords: [[0.0; 9]; 3], len: [2; 3], lo: [0; 3], hi: [1; 3] };
        if bits.iter().any(|&v| v < 0) {
            for a in 0..3 {
                s.coords[a][0] = lo_v[a];
                s.coords[a][1] = hi_v[a];
            }
        } else {
            for a in 0..3 {
                let n = 1usize << bits[a];
                for i in 0..=n {
                    s.coords[a][i] = i as f64 / n as f64;
                }
                s.len[a] = n + 1;
                s.lo[a] = java_round(lo_v[a] * n as f64) as usize;
                s.hi[a] = java_round(hi_v[a] * n as f64) as usize;
            }
        }
        Some(s)
    }
}

/// What `joinIsNotEmpty` reads from a shape.
trait Voxels {
    fn coords(&self, axis: usize) -> &[f64];
    fn offset(&self, axis: usize) -> f64;
    fn full_wide(&self, x: i32, y: i32, z: i32) -> bool;
    fn min(&self, axis: usize) -> f64;
    fn max(&self, axis: usize) -> f64;
}

struct Placed<'a>(&'a Shape, [f64; 3]);

impl Voxels for Placed<'_> {
    fn coords(&self, axis: usize) -> &[f64] {
        &self.0.coords[axis]
    }
    fn offset(&self, axis: usize) -> f64 {
        self.1[axis]
    }
    fn full_wide(&self, x: i32, y: i32, z: i32) -> bool {
        self.0.is_full_wide(x, y, z)
    }
    fn min(&self, axis: usize) -> f64 {
        self.0.min(Axis::ALL[axis], self.1[axis])
    }
    fn max(&self, axis: usize) -> f64 {
        self.0.max(Axis::ALL[axis], self.1[axis])
    }
}

impl Voxels for BoxShape {
    fn coords(&self, axis: usize) -> &[f64] {
        &self.coords[axis][..self.len[axis]]
    }
    fn offset(&self, _axis: usize) -> f64 {
        0.0
    }
    fn full_wide(&self, x: i32, y: i32, z: i32) -> bool {
        [x, y, z].iter().enumerate().all(|(a, &i)| i >= self.lo[a] as i32 && i < self.hi[a] as i32)
    }
    fn min(&self, axis: usize) -> f64 {
        self.coords[axis][self.lo[axis]]
    }
    fn max(&self, axis: usize) -> f64 {
        self.coords[axis][self.hi[axis]]
    }
}

/// `Shapes.joinIsNotEmpty(a, b, BooleanOp.AND)` for two placed shapes.
pub fn intersects(a: &Shape, a_off: [f64; 3], b: &Shape, b_off: [f64; 3]) -> bool {
    !a.is_empty() && !b.is_empty() && join_and(&Placed(a, a_off), &Placed(b, b_off))
}

/// `Shapes.joinIsNotEmpty(shape, Shapes.create(box), AND)`.
pub fn intersects_box(a: &Shape, a_off: [f64; 3], b: &BoxShape) -> bool {
    !a.is_empty() && join_and(&Placed(a, a_off), b)
}

fn join_and(a: &impl Voxels, b: &impl Voxels) -> bool {
    for i in 0..3 {
        if a.max(i) < b.min(i) - 1.0e-7 || b.max(i) < a.min(i) - 1.0e-7 {
            return false;
        }
    }
    let (Some(mx), Some(my), Some(mz)) = (merge(a, b, 0), merge(a, b, 1), merge(a, b, 2)) else { return false };
    for &(ax, bx) in mx.cells() {
        for &(ay, by) in my.cells() {
            for &(az, bz) in mz.cells() {
                if a.full_wide(ax, ay, az) && b.full_wide(bx, by, bz) {
                    return true;
                }
            }
        }
    }
    false
}

/// Merged cells of one axis: (index in a, index in b).
struct Merged {
    cells: [(i32, i32); 48],
    len: usize,
}

impl Merged {
    fn cells(&self) -> &[(i32, i32)] {
        &self.cells[..self.len]
    }
}

/// `Shapes.createIndexMerger` for `AND`: `None` when the lists do not overlap
/// (`NonOverlappingMerger`, which never pairs two cells); otherwise `IndirectMerger` with
/// firstOnly = secondOnly = false (`IdenticalMerger` yields the same cells).
fn merge(sa: &impl Voxels, sb: &impl Voxels, axis: usize) -> Option<Merged> {
    let (a, a_off, b, b_off) = (sa.coords(axis), sa.offset(axis), sb.coords(axis), sb.offset(axis));
    let (n, m) = (a.len(), b.len());
    if a[n - 1] + a_off < b[0] + b_off - 1.0e-7 || b[m - 1] + b_off < a[0] + a_off - 1.0e-7 {
        return None;
    }
    let mut out = Merged { cells: [(0, 0); 48], len: 0 };
    let mut last = f64::NAN;
    let (mut i, mut j) = (0usize, 0usize);
    loop {
        let lower_done = i >= n;
        let upper_done = j >= m;
        if lower_done && upper_done {
            break;
        }
        let take_lower = !lower_done && (upper_done || a[i] + a_off < b[j] + b_off + 1.0e-7);
        if take_lower {
            i += 1;
            if j == 0 || upper_done {
                continue;
            }
        } else {
            j += 1;
            if i == 0 || lower_done {
                continue;
            }
        }
        let cell = (i as i32 - 1, j as i32 - 1);
        let v = if take_lower { a[i - 1] + a_off } else { b[j - 1] + b_off };
        // Java: !(last >= v - 1e-7), NaN included.
        if last >= v - 1.0e-7 {
            out.cells[out.len - 1] = cell;
        } else {
            out.cells[out.len] = cell;
            out.len += 1;
            last = v;
        }
    }
    // forMergedIndexes visits resultLength - 1 cells.
    out.len = out.len.max(1) - 1;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Shape {
        Shape::new([vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]], &[true])
    }

    #[test]
    fn collide_against_floor() {
        let s = cube();
        let b = Aabb::new(0.375, 64.0, 0.375, 0.625, 64.25, 0.625);
        assert_eq!(s.collide(Axis::Y, &b, -0.04, [0.0, 63.0, 0.0]), 0.0);
        let b = Aabb::new(0.375, 64.5, 0.375, 0.625, 64.75, 0.625);
        assert_eq!(s.collide(Axis::Y, &b, -1.0, [0.0, 63.0, 0.0]), -0.5);
        // Beside the block: no collision.
        let b = Aabb::new(1.375, 64.5, 0.375, 1.625, 64.75, 0.625);
        assert_eq!(s.collide(Axis::Y, &b, -1.0, [0.0, 63.0, 0.0]), -1.0);
    }

    #[test]
    fn box_shapes_and_merged_boxes() {
        let b = Shape::from_box(&Aabb::new(0.0, 0.0, 0.0, 0.5, 1.0, 1.0)).unwrap();
        assert_eq!(b.size(Axis::X), 2);
        assert_eq!(b.boxes(), &[Aabb::new(0.0, 0.0, 0.0, 0.5, 1.0, 1.0)]);
        let odd = Shape::from_box(&Aabb::new(10.1, 0.0, 0.0, 10.3, 1.0, 1.0)).unwrap();
        assert_eq!(odd.coords(Axis::X), &[10.1, 10.3]);
        // A 2x1x1 grid with both cells full merges into one box.
        let two = Shape::new([vec![0.0, 0.5, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]], &[true, true]);
        assert_eq!(two.boxes(), &[Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0)]);
    }

    #[test]
    fn join_touching_is_empty() {
        let s = cube();
        let entity = Shape::from_box(&Aabb::new(10.2, 65.0, 10.2, 10.4, 65.5, 10.4)).unwrap();
        assert!(!intersects(&s, [10.0, 64.0, 10.0], &entity, [0.0; 3]));
        let entity = Shape::from_box(&Aabb::new(10.2, 64.9, 10.2, 10.4, 65.5, 10.4)).unwrap();
        assert!(intersects(&s, [10.0, 64.0, 10.0], &entity, [0.0; 3]));
    }
}
