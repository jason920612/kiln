//! `BoundingBox`: an inclusive integer box.

use crate::block_facts::Dir;
use crate::pos::BlockPos;
use kiln_proto::nbt::Tag;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoundingBox {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

impl BoundingBox {
    /// `new BoundingBox(...)`: inverted bounds are swapped (vanilla logs and fixes them).
    pub fn new(x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> Self {
        Self { min_x: x0.min(x1), min_y: y0.min(y1), min_z: z0.min(z1), max_x: x0.max(x1), max_y: y0.max(y1), max_z: z0.max(z1) }
    }

    pub fn from_corners(a: BlockPos, b: BlockPos) -> Self {
        Self::new(a.x, a.y, a.z, b.x, b.y, b.z)
    }

    pub fn infinite() -> Self {
        Self { min_x: i32::MIN, min_y: i32::MIN, min_z: i32::MIN, max_x: i32::MAX, max_y: i32::MAX, max_z: i32::MAX }
    }

    /// `BoundingBox.orientBox`: a box of `size` at an offset from `(x, y, z)`, turned to face
    /// `dir`.
    #[allow(clippy::too_many_arguments)]
    pub fn orient(x: i32, y: i32, z: i32, ox: i32, oy: i32, oz: i32, sx: i32, sy: i32, sz: i32, dir: Dir) -> Self {
        match dir {
            Dir::North => Self::new(x + ox, y + oy, z - sz + 1 + oz, x + sx - 1 + ox, y + sy - 1 + oy, z + oz),
            Dir::West => Self::new(x - sz + 1 + oz, y + oy, z + ox, x + oz, y + sy - 1 + oy, z + sx - 1 + ox),
            Dir::East => Self::new(x + oz, y + oy, z + ox, x + sz - 1 + oz, y + sy - 1 + oy, z + sx - 1 + ox),
            _ => Self::new(x + ox, y + oy, z + oz, x + sx - 1 + ox, y + sy - 1 + oy, z + sz - 1 + oz),
        }
    }

    pub fn intersects(&self, o: &BoundingBox) -> bool {
        self.max_x >= o.min_x
            && self.min_x <= o.max_x
            && self.max_z >= o.min_z
            && self.min_z <= o.max_z
            && self.max_y >= o.min_y
            && self.min_y <= o.max_y
    }

    /// `intersects(minX, minZ, maxX, maxZ)`: overlap in x and z.
    pub fn intersects_xz(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        self.max_x >= x0 && self.min_x <= x1 && self.max_z >= z0 && self.min_z <= z1
    }

    pub fn is_inside(&self, p: BlockPos) -> bool {
        p.x >= self.min_x && p.x <= self.max_x && p.z >= self.min_z && p.z <= self.max_z && p.y >= self.min_y && p.y <= self.max_y
    }

    pub fn encapsulate(&mut self, o: &BoundingBox) {
        self.min_x = self.min_x.min(o.min_x);
        self.min_y = self.min_y.min(o.min_y);
        self.min_z = self.min_z.min(o.min_z);
        self.max_x = self.max_x.max(o.max_x);
        self.max_y = self.max_y.max(o.max_y);
        self.max_z = self.max_z.max(o.max_z);
    }

    pub fn encapsulate_pos(&mut self, p: BlockPos) {
        self.encapsulate(&BoundingBox::new(p.x, p.y, p.z, p.x, p.y, p.z));
    }

    /// `move` (in place).
    pub fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.min_x += dx;
        self.min_y += dy;
        self.min_z += dz;
        self.max_x += dx;
        self.max_y += dy;
        self.max_z += dz;
    }

    pub fn moved(&self, dx: i32, dy: i32, dz: i32) -> BoundingBox {
        let mut b = *self;
        b.shift(dx, dy, dz);
        b
    }

    pub fn inflated(&self, x: i32, y: i32, z: i32) -> BoundingBox {
        BoundingBox::new(self.min_x - x, self.min_y - y, self.min_z - z, self.max_x + x, self.max_y + y, self.max_z + z)
    }

    pub fn x_span(&self) -> i32 {
        self.max_x - self.min_x + 1
    }

    pub fn y_span(&self) -> i32 {
        self.max_y - self.min_y + 1
    }

    pub fn z_span(&self) -> i32 {
        self.max_z - self.min_z + 1
    }

    /// `getCenter`.
    pub fn center(&self) -> BlockPos {
        BlockPos::new(
            self.min_x + (self.max_x - self.min_x + 1) / 2,
            self.min_y + (self.max_y - self.min_y + 1) / 2,
            self.min_z + (self.max_z - self.min_z + 1) / 2,
        )
    }

    /// The box of a chunk column between `min_y` and `max_y` (`getWritableArea`-style).
    pub fn chunk(cx: i32, cz: i32, min_y: i32, max_y: i32) -> BoundingBox {
        BoundingBox::new(cx << 4, min_y, cz << 4, (cx << 4) + 15, max_y, (cz << 4) + 15)
    }

    /// `BoundingBox.CODEC`: an int array `[minX, minY, minZ, maxX, maxY, maxZ]`.
    pub fn to_tag(&self) -> Tag {
        Tag::IntArray(vec![self.min_x, self.min_y, self.min_z, self.max_x, self.max_y, self.max_z])
    }

    pub fn from_tag(tag: &Tag) -> Option<BoundingBox> {
        match tag {
            Tag::IntArray(v) if v.len() == 6 => Some(BoundingBox::new(v[0], v[1], v[2], v[3], v[4], v[5])),
            _ => None,
        }
    }
}
