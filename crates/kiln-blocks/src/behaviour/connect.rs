//! Connection shapes: fences (`FenceBlock`), panes and bars (`IronBarsBlock`), walls
//! (`WallBlock`), stairs (`StairBlock`) and snowy grass (`SnowyBlock`).

use super::sturdy;
use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::state;
use crate::tags;
use kiln_data::block_logic::{self as logic, BlockClass, Support};
use kiln_data::blocks::default_state as d;

/// `Block.isExceptionForConnection`.
pub fn is_exception_for_connection(s: u16) -> bool {
    logic::is_instance(s, BlockClass::LeavesBlock)
        || state::is(s, d::BARRIER)
        || state::is(s, d::CARVED_PUMPKIN)
        || state::is(s, d::JACK_O_LANTERN)
        || state::is(s, d::MELON)
        || state::is(s, d::PUMPKIN)
        || tags::is(s, "minecraft:shulker_boxes")
}

/// `FenceGateBlock.connectsToDirection`: a gate in line with `dir`.
fn gate_in_line(s: u16, dir: Direction) -> bool {
    logic::is_instance(s, BlockClass::FenceGateBlock)
        && state::get_dir(s, "facing").is_some_and(|f| f.clockwise().axis() == dir.axis())
}

/// `FenceBlock.connectsTo`.
fn fence_connects(fence: u16, s: u16, face_sturdy: bool, dir: Direction) -> bool {
    let same_fence = tags::is(s, "minecraft:fences")
        && tags::is(s, "minecraft:wooden_fences") == tags::is(fence, "minecraft:wooden_fences");
    !is_exception_for_connection(s) && face_sturdy || same_fence || gate_in_line(s, dir)
}

/// `IronBarsBlock.attachsTo`.
fn pane_attaches(s: u16, face_sturdy: bool) -> bool {
    !is_exception_for_connection(s) && face_sturdy || logic::is_instance(s, BlockClass::IronBarsBlock) || tags::is(s, "minecraft:walls")
}

/// `WallBlock.connectsTo`.
fn wall_connects(s: u16, face_sturdy: bool, dir: Direction) -> bool {
    tags::is(s, "minecraft:walls")
        || !is_exception_for_connection(s) && face_sturdy
        || logic::is_instance(s, BlockClass::IronBarsBlock)
        || gate_in_line(s, dir)
}

/// Whether the neighbour toward `dir` offers a full face back toward us.
fn faces_back(neighbor: u16, dir: Direction) -> bool {
    sturdy(neighbor, dir.opposite(), Support::Full)
}

/// `FenceBlock` / `IronBarsBlock` `updateShape` for a horizontal neighbour.
pub fn cross_update(state: u16, dir: Direction, neighbor: u16) -> u16 {
    let connects = if logic::is_instance(state, BlockClass::FenceBlock) {
        fence_connects(state, neighbor, faces_back(neighbor, dir), dir)
    } else {
        pane_attaches(neighbor, faces_back(neighbor, dir))
    };
    state::set_bool(state, dir.name(), connects)
}

/// `FenceBlock` / `IronBarsBlock` `getStateForPlacement` connections.
pub fn cross_placement<L: Level + ?Sized>(level: &L, state: u16, pos: BlockPos) -> u16 {
    let mut s = state;
    for dir in [Direction::North, Direction::East, Direction::South, Direction::West] {
        s = cross_update(s, dir, level.block(pos.relative(dir)));
    }
    s
}

fn wall_side(s: u16, dir: Direction) -> bool {
    state::get(s, dir.name()).is_some_and(|v| v != "none")
}

/// `WallBlock.updateShape(level, state, topPos, topNeighbour, n, e, s, w)`: sides tall where
/// the block above covers them, low otherwise, then the post.
fn wall_with_sides(state: u16, top: u16, sides: [bool; 4]) -> u16 {
    let mut st = state;
    for (i, dir) in [Direction::North, Direction::East, Direction::South, Direction::West].into_iter().enumerate() {
        let v = if !sides[i] {
            "none"
        } else if logic::wall_side_covered(top, i as u8) {
            "tall"
        } else {
            "low"
        };
        st = state::set(st, dir.name(), v);
    }
    state::set_bool(st, "up", wall_raise_post(st, top))
}

/// `WallBlock.shouldRaisePost`.
fn wall_raise_post(state: u16, top: u16) -> bool {
    if logic::is_instance(top, BlockClass::WallBlock) && state::get_bool(top, "up") {
        return true;
    }
    let side = |d: Direction| state::get(state, d.name()).unwrap_or("none");
    let (n, s, e, w) = (side(Direction::North), side(Direction::South), side(Direction::East), side(Direction::West));
    let (sn, ss, se, sw) = (n == "none", s == "none", e == "none", w == "none");
    let all_none_or_uneven = (sn && ss && sw && se) || sn != ss || sw != se;
    if all_none_or_uneven {
        return true;
    }
    let tall_line = (n == "tall" && s == "tall") || (e == "tall" && w == "tall");
    if tall_line {
        return false;
    }
    tags::is(top, "minecraft:wall_post_override") || logic::wall_post_covered(top)
}

/// `WallBlock.updateShape` for a neighbour change (not below).
pub fn wall_update<L: Level + ?Sized>(level: &L, state: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    if dir == Direction::Up {
        let sides = [Direction::North, Direction::East, Direction::South, Direction::West].map(|d| wall_side(state, d));
        return wall_with_sides(state, neighbor, sides);
    }
    let sides = [Direction::North, Direction::East, Direction::South, Direction::West].map(|d| {
        if d == dir { wall_connects(neighbor, faces_back(neighbor, d), d) } else { wall_side(state, d) }
    });
    wall_with_sides(state, level.block(pos.above()), sides)
}

/// `WallBlock.getStateForPlacement` connections.
pub fn wall_placement<L: Level + ?Sized>(level: &L, state: u16, pos: BlockPos) -> u16 {
    let sides = [Direction::North, Direction::East, Direction::South, Direction::West].map(|d| {
        let n = level.block(pos.relative(d));
        wall_connects(n, faces_back(n, d), d)
    });
    wall_with_sides(state, level.block(pos.above()), sides)
}

fn is_stairs(s: u16) -> bool {
    logic::is_instance(s, BlockClass::StairBlock)
}

fn stairs_facing(s: u16) -> Direction {
    state::get_dir(s, "facing").unwrap_or(Direction::North)
}

/// `StairBlock.canTakeShape`.
fn can_take_shape<L: Level + ?Sized>(level: &L, state: u16, pos: BlockPos, dir: Direction) -> bool {
    let n = level.block(pos.relative(dir));
    !is_stairs(n) || stairs_facing(n) != stairs_facing(state) || state::get(n, "half") != state::get(state, "half")
}

/// `StairBlock.getStairsShape`.
pub fn stairs_shape<L: Level + ?Sized>(level: &L, state: u16, pos: BlockPos) -> &'static str {
    let facing = stairs_facing(state);
    let front = level.block(pos.relative(facing));
    if is_stairs(front) && state::get(state, "half") == state::get(front, "half") {
        let f = stairs_facing(front);
        if f.axis() != facing.axis() && can_take_shape(level, state, pos, f.opposite()) {
            return if f == facing.counter_clockwise() { "outer_left" } else { "outer_right" };
        }
    }
    let back = level.block(pos.relative(facing.opposite()));
    if is_stairs(back) && state::get(state, "half") == state::get(back, "half") {
        let f = stairs_facing(back);
        if f.axis() != facing.axis() && can_take_shape(level, state, pos, f) {
            return if f == facing.counter_clockwise() { "inner_left" } else { "inner_right" };
        }
    }
    "straight"
}

/// `SnowyBlock.isSnowySetting`.
pub fn snowy_setting(above: u16) -> bool {
    tags::is(above, "minecraft:snow")
}
