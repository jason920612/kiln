//! Block positions and directions (`BlockPos`, `Direction`, `Direction.Axis`).

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl fmt::Debug for BlockPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    pub fn relative(self, dir: Direction) -> Self {
        let [dx, dy, dz] = dir.step();
        self.offset(dx, dy, dz)
    }

    pub fn relative_by(self, dir: Direction, n: i32) -> Self {
        let [dx, dy, dz] = dir.step();
        self.offset(dx * n, dy * n, dz * n)
    }

    pub fn above(self) -> Self {
        self.offset(0, 1, 0)
    }

    pub fn below(self) -> Self {
        self.offset(0, -1, 0)
    }

    pub fn chunk(self) -> (i32, i32) {
        (self.x >> 4, self.z >> 4)
    }

    /// `Vec3i.hashCode`, which fixes the iteration order of Java hash sets of positions.
    pub fn java_hash(self) -> i32 {
        (self.y.wrapping_add(self.z.wrapping_mul(31))).wrapping_mul(31).wrapping_add(self.x)
    }
}

/// Directions in `Direction.values()` order (the 3D data value).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum Direction {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Direction {
    /// `Direction.values()`.
    pub const ALL: [Direction; 6] = [Self::Down, Self::Up, Self::North, Self::South, Self::West, Self::East];
    /// `Direction.Plane.HORIZONTAL` iteration order.
    pub const HORIZONTAL: [Direction; 4] = [Self::North, Self::East, Self::South, Self::West];
    /// `Direction.Plane.VERTICAL` iteration order.
    pub const VERTICAL: [Direction; 2] = [Self::Up, Self::Down];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i]
    }

    pub fn step(self) -> [i32; 3] {
        match self {
            Self::Down => [0, -1, 0],
            Self::Up => [0, 1, 0],
            Self::North => [0, 0, -1],
            Self::South => [0, 0, 1],
            Self::West => [-1, 0, 0],
            Self::East => [1, 0, 0],
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    pub fn axis(self) -> Axis {
        match self {
            Self::Down | Self::Up => Axis::Y,
            Self::North | Self::South => Axis::Z,
            Self::West | Self::East => Axis::X,
        }
    }

    pub fn is_horizontal(self) -> bool {
        self.axis() != Axis::Y
    }

    pub fn is_positive(self) -> bool {
        matches!(self, Self::Up | Self::South | Self::East)
    }

    /// Rotation around Y (horizontal directions only).
    pub fn clockwise(self) -> Self {
        match self {
            Self::North => Self::East,
            Self::East => Self::South,
            Self::South => Self::West,
            Self::West => Self::North,
            d => d,
        }
    }

    pub fn counter_clockwise(self) -> Self {
        match self {
            Self::North => Self::West,
            Self::West => Self::South,
            Self::South => Self::East,
            Self::East => Self::North,
            d => d,
        }
    }

    /// The property value name ("north", ...).
    pub fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Up => "up",
            Self::North => "north",
            Self::South => "south",
            Self::West => "west",
            Self::East => "east",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "down" => Self::Down,
            "up" => Self::Up,
            "north" => Self::North,
            "south" => Self::South,
            "west" => Self::West,
            "east" => Self::East,
            _ => return None,
        })
    }

    /// `Direction.from2DDataValue`: south, west, north, east.
    pub fn from_2d(i: i32) -> Self {
        [Self::South, Self::West, Self::North, Self::East][i.rem_euclid(4) as usize]
    }

    pub fn to_2d(self) -> i32 {
        match self {
            Self::South => 0,
            Self::West => 1,
            Self::North => 2,
            Self::East => 3,
            _ => -1,
        }
    }

    /// `Direction.fromYRot`: the horizontal direction a yaw faces.
    pub fn from_yaw(yaw: f64) -> Self {
        Self::from_2d(((yaw / 90.0) + 0.5).floor() as i64 as i32 & 3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hash_matches_vec3i() {
        assert_eq!(BlockPos::new(1, 2, 3).java_hash(), (2 + 3 * 31) * 31 + 1);
        assert_eq!(BlockPos::new(-5, -60, 7).java_hash(), (-60 + 7 * 31) * 31 - 5);
    }

    #[test]
    fn yaw_directions() {
        assert_eq!(Direction::from_yaw(0.0), Direction::South);
        assert_eq!(Direction::from_yaw(90.0), Direction::West);
        assert_eq!(Direction::from_yaw(180.0), Direction::North);
        assert_eq!(Direction::from_yaw(-90.0), Direction::East);
        assert_eq!(Direction::from_yaw(44.0), Direction::South);
        assert_eq!(Direction::from_yaw(46.0), Direction::West);
    }
}
