//! Nether portals as blocks: vanilla's `PortalShape` (finding a frame, filling it), the portal
//! block's `updateShape` (a broken frame takes the portal down) and fire lighting a frame
//! (`BaseFireBlock.onPlace`, `BaseFireBlock.canBePlacedAt`).

use crate::level::Level;
use crate::pos::{Axis, BlockPos, Direction};
use crate::state;
use kiln_data::blocks::default_state as d;

/// `PortalShape.MAX_WIDTH` / `MAX_HEIGHT`.
pub const MAX_SIZE: i32 = 21;

/// `PortalShape`: a frame's interior, or an invalid shape (zero width or height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalShape {
    pub axis: Axis,
    /// Portal blocks already inside.
    pub portal_blocks: i32,
    /// West for an X portal, south for a Z portal (`rightDir`).
    pub right: Direction,
    pub bottom_left: BlockPos,
    pub height: i32,
    pub width: i32,
}

/// `PortalShape.FRAME`: `#minecraft:nether_portal_frame` (obsidian).
fn frame(s: u16) -> bool {
    crate::tags::is(s, "minecraft:nether_portal_frame")
}

/// `PortalShape.isEmpty`: air, fire or portal.
fn empty(s: u16) -> bool {
    kiln_data::blocks_types::is_air(s) || crate::tags::is(s, "minecraft:fire") || state::is(s, d::NETHER_PORTAL)
}

impl PortalShape {
    /// `PortalShape.findAnyShape` around `pos`, reading blocks through `get`; `min_y` is the
    /// level's bottom.
    pub fn find_any(get: &dyn Fn(BlockPos) -> u16, min_y: i32, pos: BlockPos, axis: Axis) -> PortalShape {
        let right = if axis == Axis::X { Direction::West } else { Direction::South };
        let invalid = |at| PortalShape { axis, portal_blocks: 0, right, bottom_left: at, height: 0, width: 0 };
        let Some(bottom_left) = bottom_left(get, min_y, right, pos) else { return invalid(pos) };
        let width = edge_distance(get, bottom_left, right);
        let width = if (2..=MAX_SIZE).contains(&width) { width } else { 0 };
        if width == 0 {
            return invalid(bottom_left);
        }
        let mut portal_blocks = 0;
        let height = distance_to_top(get, bottom_left, right, width, &mut portal_blocks);
        let height = if (3..=MAX_SIZE).contains(&height) && has_top_frame(get, bottom_left, right, width, height) { height } else { 0 };
        PortalShape { axis, portal_blocks, right, bottom_left, height, width }
    }

    /// `findPortalShape` with `findEmptyPortalShape`'s filter (valid, no portal blocks yet): the
    /// preferred axis first, then the other.
    pub fn find_empty(get: &dyn Fn(BlockPos) -> u16, min_y: i32, pos: BlockPos, axis: Axis) -> Option<PortalShape> {
        let ok = |s: &PortalShape| s.is_valid() && s.portal_blocks == 0;
        let first = Self::find_any(get, min_y, pos, axis);
        if ok(&first) {
            return Some(first);
        }
        let other = if axis == Axis::X { Axis::Z } else { Axis::X };
        Some(Self::find_any(get, min_y, pos, other)).filter(ok)
    }

    pub fn is_valid(&self) -> bool {
        (2..=MAX_SIZE).contains(&self.width) && (3..=MAX_SIZE).contains(&self.height)
    }

    pub fn is_complete(&self) -> bool {
        self.is_valid() && self.portal_blocks == self.width * self.height
    }

    /// The interior positions in `BlockPos.betweenClosed` order (x fastest, then y, then z).
    pub fn interior(&self) -> Vec<BlockPos> {
        let a = self.bottom_left;
        let b = self.bottom_left.relative_by(Direction::Up, self.height - 1).relative_by(self.right, self.width - 1);
        let mut out = Vec::new();
        for z in a.z.min(b.z)..=a.z.max(b.z) {
            for y in a.y.min(b.y)..=a.y.max(b.y) {
                for x in a.x.min(b.x)..=a.x.max(b.x) {
                    out.push(BlockPos::new(x, y, z));
                }
            }
        }
        out
    }

    /// The portal block of this shape's axis.
    pub fn portal_state(&self) -> u16 {
        portal_state(self.axis)
    }

    /// `createPortalBlocks`: fills the interior (flags 18: clients, known shape).
    pub fn create_portal_blocks<L: Level>(&self, level: &mut L) {
        let portal = self.portal_state();
        for p in self.interior() {
            crate::update::set_block(level, p, portal, crate::flags::CLIENTS | crate::flags::KNOWN_SHAPE);
        }
    }
}

/// `minecraft:nether_portal[axis=...]`.
pub fn portal_state(axis: Axis) -> u16 {
    state::set(d::NETHER_PORTAL, "axis", if axis == Axis::Z { "z" } else { "x" })
}

/// A portal block's axis.
pub fn portal_axis(s: u16) -> Axis {
    if state::get(s, "axis") == Some("z") { Axis::Z } else { Axis::X }
}

fn bottom_left(get: &dyn Fn(BlockPos) -> u16, min_y: i32, right: Direction, pos: BlockPos) -> Option<BlockPos> {
    let floor = min_y.max(pos.y - MAX_SIZE);
    let mut pos = pos;
    while pos.y > floor && empty(get(pos.below())) {
        pos = pos.below();
    }
    let left = right.opposite();
    let edge = edge_distance(get, pos, left) - 1;
    (edge >= 0).then(|| pos.relative_by(left, edge))
}

/// `getDistanceUntilEdgeAboveFrame`.
fn edge_distance(get: &dyn Fn(BlockPos) -> u16, pos: BlockPos, dir: Direction) -> i32 {
    for width in 0..=MAX_SIZE {
        let at = pos.relative_by(dir, width);
        let s = get(at);
        if !empty(s) {
            if frame(s) {
                return width;
            }
            break;
        }
        if !frame(get(at.below())) {
            break;
        }
    }
    0
}

fn has_top_frame(get: &dyn Fn(BlockPos) -> u16, bottom_left: BlockPos, right: Direction, width: i32, height: i32) -> bool {
    (0..width).all(|i| frame(get(bottom_left.relative_by(Direction::Up, height).relative_by(right, i))))
}

/// `getDistanceUntilTop`, counting the portal blocks inside.
fn distance_to_top(get: &dyn Fn(BlockPos) -> u16, bottom_left: BlockPos, right: Direction, width: i32, portals: &mut i32) -> i32 {
    for height in 0..MAX_SIZE {
        let row = bottom_left.relative_by(Direction::Up, height);
        if !frame(get(row.relative_by(right, -1))) || !frame(get(row.relative_by(right, width))) {
            return height;
        }
        for i in 0..width {
            let s = get(row.relative_by(right, i));
            if !empty(s) {
                return height;
            }
            if state::is(s, d::NETHER_PORTAL) {
                *portals += 1;
            }
        }
    }
    MAX_SIZE
}

/// `NetherPortalBlock.updateShape`: the portal goes when the neighbour along its plane is no
/// portal and its frame is no longer complete.
pub fn portal_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    let axis = portal_axis(s);
    let update_axis = dir.axis();
    let wrong_axis = axis != update_axis && update_axis != Axis::Y;
    if wrong_axis || state::is(neighbor, d::NETHER_PORTAL) {
        return s;
    }
    let min_y = level.min_y();
    let complete = PortalShape::find_any(&|p| level.block(p), min_y, pos, axis).is_complete();
    if complete { s } else { d::AIR }
}

/// `BaseFireBlock.onPlace`'s portal part: fire placed in an empty frame (in the overworld or
/// the nether) becomes a portal. Returns whether it did.
pub fn fire_on_place<L: Level>(level: &mut L, pos: BlockPos) -> bool {
    if !level.portals_light() {
        return false;
    }
    let min_y = level.min_y();
    match PortalShape::find_empty(&|p| level.block(p), min_y, pos, Axis::X) {
        Some(shape) => {
            shape.create_portal_blocks(level);
            true
        }
        None => false,
    }
}

pub use crate::fire::fire_state;

/// `BaseFireBlock.canBePlacedAt` for a player facing `forward` (horizontal): the position is
/// air and fire survives there, or it would light a portal.
pub fn fire_can_be_placed_at<L: Level + ?Sized>(level: &L, pos: BlockPos, forward: Direction) -> bool {
    if !kiln_data::blocks_types::is_air(level.block(pos)) {
        return false;
    }
    let fire = fire_state(level, pos);
    crate::fire::can_survive(level, fire, pos) || is_portal(level, pos, forward)
}

/// `BaseFireBlock.isPortal`.
fn is_portal<L: Level + ?Sized>(level: &L, pos: BlockPos, forward: Direction) -> bool {
    if !level.portals_light() {
        return false;
    }
    if !Direction::ALL.iter().any(|&dir| state::is(level.block(pos.relative(dir)), d::OBSIDIAN)) {
        return false;
    }
    let axis = forward.counter_clockwise().axis();
    let min_y = level.min_y();
    PortalShape::find_empty(&|p| level.block(p), min_y, pos, axis).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn frame_at(blocks: &mut HashMap<BlockPos, u16>, origin: BlockPos, axis: Axis, w: i32, h: i32) {
        let along = if axis == Axis::X { Direction::East } else { Direction::South };
        for i in -1..=w {
            for j in -1..=h {
                if i == -1 || i == w || j == -1 || j == h {
                    blocks.insert(origin.relative_by(along, i).relative_by(Direction::Up, j), d::OBSIDIAN);
                }
            }
        }
    }

    #[test]
    fn finds_an_empty_frame_and_its_interior() {
        let mut blocks = HashMap::new();
        let origin = BlockPos::new(10, 64, -5);
        frame_at(&mut blocks, origin, Axis::X, 2, 3);
        let get = |p: BlockPos| blocks.get(&p).copied().unwrap_or(d::AIR);
        let shape = PortalShape::find_empty(&get, -64, BlockPos::new(11, 65, -5), Axis::Z).expect("frame");
        assert_eq!((shape.axis, shape.width, shape.height, shape.bottom_left), (Axis::X, 2, 3, BlockPos::new(11, 64, -5)));
        assert_eq!(shape.interior().len(), 6);
        // A frame missing a corner block still counts (corners are not checked); one missing
        // a side block does not.
        let mut broken = blocks.clone();
        broken.remove(&BlockPos::new(9, 65, -5));
        let get = |p: BlockPos| broken.get(&p).copied().unwrap_or(d::AIR);
        assert!(PortalShape::find_empty(&get, -64, BlockPos::new(10, 65, -5), Axis::X).is_none());
    }

    #[test]
    fn large_frames_up_to_21() {
        let mut blocks = HashMap::new();
        let origin = BlockPos::new(0, 0, 0);
        frame_at(&mut blocks, origin, Axis::Z, 21, 21);
        let get = |p: BlockPos| blocks.get(&p).copied().unwrap_or(d::AIR);
        let shape = PortalShape::find_any(&get, -64, BlockPos::new(0, 10, 10), Axis::Z);
        assert_eq!((shape.width, shape.height), (21, 21));
        let mut blocks = HashMap::new();
        frame_at(&mut blocks, origin, Axis::Z, 22, 3);
        let get = |p: BlockPos| blocks.get(&p).copied().unwrap_or(d::AIR);
        assert!(!PortalShape::find_any(&get, -64, BlockPos::new(0, 1, 1), Axis::Z).is_valid());
    }
}
