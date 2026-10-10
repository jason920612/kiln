//! Vanilla's `Vec3`, `AABB`, `BlockPos` and `Direction` with Java's floating-point semantics.
//!
//! Every operation evaluates in the same order as the Java original so results match bit for
//! bit (Rust never fuses `a * b + c`).

use std::ops::{Add, Sub};

/// `Math.min(double, double)`: NaN wins and `-0.0 < +0.0`.
#[inline]
pub fn jmin(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b { a } else { b }
}

/// `Math.max(double, double)`: NaN wins and `-0.0 < +0.0`.
#[inline]
pub fn jmax(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Mth.floor(double)`: `(int) Math.floor(x)` (saturating, NaN to 0, as Rust's `as`).
#[inline]
pub fn floor(x: f64) -> i32 {
    x.floor() as i32
}

/// `Mth.ceil(double)`.
#[inline]
pub fn ceil(x: f64) -> i32 {
    x.ceil() as i32
}

/// `Mth.lerp(double, double, double)`.
#[inline]
pub fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// `Mth.equal(double, double)`: within 1e-5.
#[inline]
pub fn mth_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < 9.999999747378752e-6
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    /// `Direction.axisStepOrder`: Y first, then the larger horizontal component.
    pub fn step_order(v: Vec3) -> [Axis; 3] {
        if v.x.abs() < v.z.abs() { [Axis::Y, Axis::Z, Axis::X] } else { [Axis::Y, Axis::X, Axis::Z] }
    }

    pub fn positive(self) -> Direction {
        match self {
            Axis::X => Direction::East,
            Axis::Y => Direction::Up,
            Axis::Z => Direction::South,
        }
    }
}

/// `Direction`, in vanilla's ordinal (3D data value) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub const ALL: [Direction; 6] =
        [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East];
    /// `Direction.Plane.HORIZONTAL` iteration order.
    pub const HORIZONTAL: [Direction; 4] = [Direction::North, Direction::East, Direction::South, Direction::West];

    pub fn step(self) -> (i32, i32, i32) {
        match self {
            Direction::Down => (0, -1, 0),
            Direction::Up => (0, 1, 0),
            Direction::North => (0, 0, -1),
            Direction::South => (0, 0, 1),
            Direction::West => (-1, 0, 0),
            Direction::East => (1, 0, 0),
        }
    }

    pub fn axis(self) -> Axis {
        match self {
            Direction::Down | Direction::Up => Axis::Y,
            Direction::North | Direction::South => Axis::Z,
            Direction::West | Direction::East => Axis::X,
        }
    }

    pub fn opposite(self) -> Direction {
        match self {
            Direction::Down => Direction::Up,
            Direction::Up => Direction::Down,
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }

    pub fn is_positive(self) -> bool {
        matches!(self, Direction::Up | Direction::South | Direction::East)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// `BlockPos.containing(x, y, z)`.
    pub fn containing(x: f64, y: f64, z: f64) -> Self {
        Self::new(floor(x), floor(y), floor(z))
    }

    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    pub fn relative(self, d: Direction) -> Self {
        let (dx, dy, dz) = d.step();
        self.offset(dx, dy, dz)
    }

    pub fn above(self) -> Self {
        self.offset(0, 1, 0)
    }

    pub fn below(self) -> Self {
        self.offset(0, -1, 0)
    }

    pub fn at_y(self, y: i32) -> Self {
        Self::new(self.x, y, self.z)
    }

    /// `BlockPos.asLong`.
    pub fn as_long(self) -> i64 {
        ((self.x as i64 & 0x3FF_FFFF) << 38) | ((self.z as i64 & 0x3FF_FFFF) << 12) | (self.y as i64 & 0xFFF)
    }

    pub fn center(self) -> Vec3 {
        Vec3::new(self.x as f64 + 0.5, self.y as f64 + 0.5, self.z as f64 + 0.5)
    }

    pub fn to_vec3(self) -> Vec3 {
        Vec3::new(self.x as f64, self.y as f64, self.z as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn add(self, x: f64, y: f64, z: f64) -> Self {
        Self::new(self.x + x, self.y + y, self.z + z)
    }

    /// `Vec3.subtract(x, y, z)`, which vanilla computes as `add(-x, -y, -z)`.
    pub fn subtract(self, x: f64, y: f64, z: f64) -> Self {
        self.add(-x, -y, -z)
    }

    pub fn scale(self, d: f64) -> Self {
        self.multiply(d, d, d)
    }

    pub fn multiply(self, x: f64, y: f64, z: f64) -> Self {
        Self::new(self.x * x, self.y * y, self.z * z)
    }

    pub fn multiply_vec(self, o: Vec3) -> Self {
        self.multiply(o.x, o.y, o.z)
    }

    pub fn length_sqr(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn horizontal_distance_sqr(self) -> f64 {
        self.x * self.x + self.z * self.z
    }

    pub fn horizontal_distance(self) -> f64 {
        (self.x * self.x + self.z * self.z).sqrt()
    }

    pub fn distance_to_sqr(self, o: Vec3) -> f64 {
        let dx = o.x - self.x;
        let dy = o.y - self.y;
        let dz = o.z - self.z;
        dx * dx + dy * dy + dz * dz
    }

    pub fn normalize(self) -> Self {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if len < 9.999999747378752e-6 { Vec3::ZERO } else { Self::new(self.x / len, self.y / len, self.z / len) }
    }

    pub fn get(self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }

    pub fn with(self, axis: Axis, v: f64) -> Self {
        match axis {
            Axis::X => Self::new(v, self.y, self.z),
            Axis::Y => Self::new(self.x, v, self.z),
            Axis::Z => Self::new(self.x, self.y, v),
        }
    }

    /// `Vec3.relative(Direction, double)`.
    pub fn relative(self, d: Direction, distance: f64) -> Self {
        let (sx, sy, sz) = d.step();
        Self::new(self.x + distance * sx as f64, self.y + distance * sy as f64, self.z + distance * sz as f64)
    }

    pub fn is_zero(self) -> bool {
        self == Vec3::ZERO
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// `Vec3.horizontal`: the same without the height.
    pub fn horizontal(self) -> Self {
        Self::new(self.x, 0.0, self.z)
    }

    /// `Vec3.projectedOn`: this vector's shadow along `o` (`o` itself when it has no length).
    pub fn projected_on(self, o: Vec3) -> Self {
        if o.length_sqr() == 0.0 {
            return o;
        }
        o.scale(self.dot(o)).scale(1.0 / o.length_sqr())
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::add(self, o.x, o.y, o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        self.subtract(o.x, o.y, o.z)
    }
}

/// `AABB`. The constructor orders each axis with `Math.min`/`Math.max` like vanilla's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

impl Aabb {
    pub fn new(x1: f64, y1: f64, z1: f64, x2: f64, y2: f64, z2: f64) -> Self {
        Self {
            min_x: jmin(x1, x2),
            min_y: jmin(y1, y2),
            min_z: jmin(z1, z2),
            max_x: jmax(x1, x2),
            max_y: jmax(y1, y2),
            max_z: jmax(z1, z2),
        }
    }

    pub fn of_block(p: BlockPos) -> Self {
        let (x, y, z) = (p.x as f64, p.y as f64, p.z as f64);
        Self::new(x, y, z, (p.x + 1) as f64, (p.y + 1) as f64, (p.z + 1) as f64)
    }

    pub fn min(&self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.min_x,
            Axis::Y => self.min_y,
            Axis::Z => self.min_z,
        }
    }

    pub fn max(&self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.max_x,
            Axis::Y => self.max_y,
            Axis::Z => self.max_z,
        }
    }

    pub fn set_min_y(&self, v: f64) -> Self {
        Self::new(self.min_x, v, self.min_z, self.max_x, self.max_y, self.max_z)
    }

    pub fn set_max_y(&self, v: f64) -> Self {
        Self::new(self.min_x, self.min_y, self.min_z, self.max_x, v, self.max_z)
    }

    pub fn expand_towards(&self, x: f64, y: f64, z: f64) -> Self {
        let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1) =
            (self.min_x, self.min_y, self.min_z, self.max_x, self.max_y, self.max_z);
        if x < 0.0 {
            x0 += x;
        } else if x > 0.0 {
            x1 += x;
        }
        if y < 0.0 {
            y0 += y;
        } else if y > 0.0 {
            y1 += y;
        }
        if z < 0.0 {
            z0 += z;
        } else if z > 0.0 {
            z1 += z;
        }
        Self::new(x0, y0, z0, x1, y1, z1)
    }

    pub fn expand_towards_vec(&self, v: Vec3) -> Self {
        self.expand_towards(v.x, v.y, v.z)
    }

    pub fn inflate(&self, x: f64, y: f64, z: f64) -> Self {
        Self::new(self.min_x - x, self.min_y - y, self.min_z - z, self.max_x + x, self.max_y + y, self.max_z + z)
    }

    pub fn inflate_all(&self, d: f64) -> Self {
        self.inflate(d, d, d)
    }

    pub fn deflate(&self, x: f64, y: f64, z: f64) -> Self {
        self.inflate(-x, -y, -z)
    }

    /// `AABB.nextDeflated`: every face one step (an ulp) inwards.
    pub fn next_deflated(&self) -> Self {
        Self::new(self.min_x.next_up(), self.min_y.next_up(), self.min_z.next_up(), self.max_x.next_down(), self.max_y.next_down(), self.max_z.next_down())
    }

    pub fn deflate_all(&self, d: f64) -> Self {
        self.deflate(d, d, d)
    }

    /// `AABB.move(x, y, z)` (named `offset` to avoid confusion with entity movement).
    pub fn offset(&self, x: f64, y: f64, z: f64) -> Self {
        Self::new(self.min_x + x, self.min_y + y, self.min_z + z, self.max_x + x, self.max_y + y, self.max_z + z)
    }

    pub fn offset_vec(&self, v: Vec3) -> Self {
        self.offset(v.x, v.y, v.z)
    }

    /// `AABB.intersect(other)`: the overlap (the constructor orders the corners, so boxes that do
    /// not meet give the gap between them).
    pub fn intersect(&self, o: &Aabb) -> Self {
        Aabb::new(
            jmax(self.min_x, o.min_x),
            jmax(self.min_y, o.min_y),
            jmax(self.min_z, o.min_z),
            jmin(self.max_x, o.max_x),
            jmin(self.max_y, o.max_y),
            jmin(self.max_z, o.max_z),
        )
    }

    /// `AABB.minmax(other)`: the box around both.
    pub fn minmax(&self, o: &Aabb) -> Self {
        Aabb::new(
            jmin(self.min_x, o.min_x),
            jmin(self.min_y, o.min_y),
            jmin(self.min_z, o.min_z),
            jmax(self.max_x, o.max_x),
            jmax(self.max_y, o.max_y),
            jmax(self.max_z, o.max_z),
        )
    }

    pub fn intersects(&self, o: &Aabb) -> bool {
        self.intersects_raw(o.min_x, o.min_y, o.min_z, o.max_x, o.max_y, o.max_z)
    }

    pub fn intersects_raw(&self, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> bool {
        self.min_x < x1 && self.max_x > x0 && self.min_y < y1 && self.max_y > y0 && self.min_z < z1 && self.max_z > z0
    }

    pub fn intersects_block(&self, p: BlockPos) -> bool {
        self.intersects_raw(p.x as f64, p.y as f64, p.z as f64, (p.x + 1) as f64, (p.y + 1) as f64, (p.z + 1) as f64)
    }

    pub fn contains(&self, v: Vec3) -> bool {
        v.x >= self.min_x
            && v.x < self.max_x
            && v.y >= self.min_y
            && v.y < self.max_y
            && v.z >= self.min_z
            && v.z < self.max_z
    }

    pub fn x_size(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn y_size(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub fn z_size(&self) -> f64 {
        self.max_z - self.min_z
    }

    /// `AABB.getSize`: the mean edge length.
    pub fn size(&self) -> f64 {
        (self.x_size() + self.y_size() + self.z_size()) / 3.0
    }

    pub fn center(&self) -> Vec3 {
        Vec3::new(lerp(0.5, self.min_x, self.max_x), lerp(0.5, self.min_y, self.max_y), lerp(0.5, self.min_z, self.max_z))
    }

    /// `AABB.clip(from, to)`: the entry point of the segment, if it hits.
    pub fn clip(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let mut scale = 1.0;
        let d = to - from;
        self.clip_direction(from, &mut scale, None, d)?;
        Some(from.add(scale * d.x, scale * d.y, scale * d.z))
    }

    /// `AABB.getDirection(...)`: narrows `scale` to the nearest face crossed by `from + t·d`.
    pub fn clip_direction(&self, from: Vec3, scale: &mut f64, mut dir: Option<Direction>, d: Vec3) -> Option<Direction> {
        let b = self;
        if d.x > 1.0e-7 {
            dir = clip_point(scale, dir, d.x, d.y, d.z, b.min_x, b.min_y, b.max_y, b.min_z, b.max_z, Direction::West, from.x, from.y, from.z);
        } else if d.x < -1.0e-7 {
            dir = clip_point(scale, dir, d.x, d.y, d.z, b.max_x, b.min_y, b.max_y, b.min_z, b.max_z, Direction::East, from.x, from.y, from.z);
        }
        if d.y > 1.0e-7 {
            dir = clip_point(scale, dir, d.y, d.z, d.x, b.min_y, b.min_z, b.max_z, b.min_x, b.max_x, Direction::Down, from.y, from.z, from.x);
        } else if d.y < -1.0e-7 {
            dir = clip_point(scale, dir, d.y, d.z, d.x, b.max_y, b.min_z, b.max_z, b.min_x, b.max_x, Direction::Up, from.y, from.z, from.x);
        }
        if d.z > 1.0e-7 {
            dir = clip_point(scale, dir, d.z, d.x, d.y, b.min_z, b.min_x, b.max_x, b.min_y, b.max_y, Direction::North, from.z, from.x, from.y);
        } else if d.z < -1.0e-7 {
            dir = clip_point(scale, dir, d.z, d.x, d.y, b.max_z, b.min_x, b.max_x, b.min_y, b.max_y, Direction::South, from.z, from.x, from.y);
        }
        dir
    }

    /// `AABB.collidedAlongVector`: whether this box moving by `v` touches any of `boxes`.
    pub fn collided_along_vector(&self, v: Vec3, boxes: &[Aabb]) -> bool {
        let center = self.center();
        let end = center + v;
        for b in boxes {
            let grown = b.inflate(self.x_size() * 0.5 - 1.0e-7, self.y_size() * 0.5 - 1.0e-7, self.z_size() * 0.5 - 1.0e-7);
            if grown.contains(end) || grown.contains(center) {
                return true;
            }
            if grown.clip(center, end).is_some() {
                return true;
            }
        }
        false
    }

    pub fn has_nan(&self) -> bool {
        [self.min_x, self.min_y, self.min_z, self.max_x, self.max_y, self.max_z].iter().any(|v| v.is_nan())
    }
}

#[allow(clippy::too_many_arguments)]
fn clip_point(
    scale: &mut f64,
    dir: Option<Direction>,
    da: f64,
    db: f64,
    dc: f64,
    begin: f64,
    min_b: f64,
    max_b: f64,
    min_c: f64,
    max_c: f64,
    face: Direction,
    start_a: f64,
    start_b: f64,
    start_c: f64,
) -> Option<Direction> {
    let t = (begin - start_a) / da;
    let pb = start_b + t * db;
    let pc = start_c + t * dc;
    if 0.0 < t && t < *scale && min_b - 1.0e-7 < pb && pb < max_b + 1.0e-7 && min_c - 1.0e-7 < pc && pc < max_c + 1.0e-7 {
        *scale = t;
        Some(face)
    } else {
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_min_max_signed_zero() {
        assert!(jmin(0.0, -0.0).is_sign_negative());
        assert!(jmin(-0.0, 0.0).is_sign_negative());
        assert!(jmax(-0.0, 0.0).is_sign_positive());
        assert!(jmax(0.0, -0.0).is_sign_positive());
        assert!(jmin(f64::NAN, 1.0).is_nan());
    }

    #[test]
    fn block_pos_long() {
        assert_eq!(BlockPos::new(0, 0, 0).as_long(), 0);
        assert_eq!(BlockPos::new(-1, -1, -1).as_long(), -1);
        assert_eq!(BlockPos::new(1, 2, 3).as_long(), (1 << 38) | (3 << 12) | 2);
    }

    #[test]
    fn clip_hits_front_face() {
        let b = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        let hit = b.clip(Vec3::new(-1.0, 0.5, 0.5), Vec3::new(2.0, 0.5, 0.5)).unwrap();
        assert_eq!(hit, Vec3::new(0.0, 0.5, 0.5));
        assert!(b.clip(Vec3::new(-1.0, 2.0, 0.5), Vec3::new(2.0, 2.0, 0.5)).is_none());
    }
}
