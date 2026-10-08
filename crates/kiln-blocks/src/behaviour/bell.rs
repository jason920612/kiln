//! Bells (`BellBlock`): hung from a ceiling, standing on the floor or held by one or two walls.
//! A click on the body, an arrow, a blast or a redstone pulse rings it; the block entity (the
//! simulation's) shakes, tells the villagers around and, with raiders near, resonates.

use super::support::can_support_center;
use super::sturdy;
use crate::block_events::block_event;
use crate::level::{Effect, Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state::{self, BlockId};
use crate::update::set_block_and_update;
use kiln_data::block_logic::Support;
use kiln_data::blocks::default_state as d;

/// `BellBlock.getConnectedDirection`: the way the bell points away from what holds it.
fn connected_direction(s: u16) -> Direction {
    match state::get(s, "attachment") {
        Some("floor") => Direction::Up,
        Some("ceiling") => Direction::Down,
        _ => state::get_dir(s, "facing").unwrap_or(Direction::North).opposite(),
    }
}

/// `BellBlock.canSurvive`.
pub fn can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let dir = connected_direction(s).opposite();
    if dir == Direction::Up {
        can_support_center(level.block(pos.above()), Direction::Down)
    } else {
        // `FaceAttachedHorizontalDirectionalBlock.canAttach`.
        sturdy(level.block(pos.relative(dir)), dir.opposite(), Support::Full)
    }
}

/// `BellBlock.getStateForPlacement`: `None` where it cannot be put.
pub fn placement<L: Level + ?Sized>(level: &L, default: u16, pos: BlockPos, face: Direction, horizontal: Direction) -> Option<u16> {
    let sturdy_at = |p: BlockPos, toward: Direction| sturdy(level.block(p), toward, Support::Full);
    if face.axis() == crate::pos::Axis::Y {
        let s = state::set_dir(state::set(default, "attachment", if face == Direction::Down { "ceiling" } else { "floor" }), "facing", horizontal);
        return can_survive(level, s, pos).then_some(s);
    }
    let double = if face.axis() == crate::pos::Axis::X {
        sturdy_at(pos.west(), Direction::East) && sturdy_at(pos.east(), Direction::West)
    } else {
        sturdy_at(pos.north(), Direction::South) && sturdy_at(pos.south(), Direction::North)
    };
    let s = state::set(state::set_dir(default, "facing", face.opposite()), "attachment", if double { "double_wall" } else { "single_wall" });
    if can_survive(level, s, pos) {
        return Some(s);
    }
    let floor = sturdy_at(pos.below(), Direction::Up);
    let s = state::set(s, "attachment", if floor { "floor" } else { "ceiling" });
    can_survive(level, s, pos).then_some(s)
}

/// `BellBlock.updateShape`: a bell loses its hold (a double wall bell keeps one), or gains or
/// loses its second wall.
pub fn update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor_pos: BlockPos, neighbor: u16) -> u16 {
    let attach = state::get(s, "attachment").unwrap_or("floor");
    let connected = connected_direction(s).opposite();
    if connected == dir && !can_survive(level, s, pos) && attach != "double_wall" {
        return d::AIR;
    }
    let _ = neighbor_pos;
    let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
    if dir.axis() == facing.axis() {
        if attach == "double_wall" && !sturdy(neighbor, dir.opposite(), Support::Full) {
            return state::set_dir(state::set(s, "attachment", "single_wall"), "facing", dir.opposite());
        }
        if attach == "single_wall" && connected.opposite() == dir && sturdy(neighbor, facing, Support::Full) {
            return state::set(s, "attachment", "double_wall");
        }
    }
    s
}

/// `BellBlock.isProperHit`: the body of the bell (not the top beam), by how it hangs.
pub fn is_proper_hit(s: u16, face: Direction, y: f64) -> bool {
    if face.axis() == crate::pos::Axis::Y || y > 0.8123999834060669 {
        return false;
    }
    let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
    match state::get(s, "attachment") {
        Some("floor") => facing.axis() == face.axis(),
        Some("single_wall") | Some("double_wall") => facing.axis() != face.axis(),
        Some("ceiling") => true,
        _ => false,
    }
}

/// `BellBlock.attemptToRing(entity, level, pos, direction)`: false without a block entity.
pub fn attempt_to_ring<L: Level + ?Sized>(level: &mut L, pos: BlockPos, direction: Option<Direction>) -> bool {
    let s = level.block(pos);
    let dir = direction.unwrap_or_else(|| state::get_dir(s, "facing").unwrap_or(Direction::North));
    if !level.bell_hit(pos, dir) {
        return false;
    }
    // `BellBlockEntity.onHit`'s block event, then the sound and the game event.
    block_event(level, pos, BlockId::of(s), 1, dir.index() as i32);
    level.effect(Effect::Sound { pos, sound: "minecraft:block.bell.use", volume: 2.0, pitch: 1.0 });
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    true
}

/// `BellBlock.onHit` for a player's click (`check_hit`) or a projectile (`!check_hit`): whether
/// the bell was hit properly, and whether it rang.
pub fn on_hit<L: Level + ?Sized>(level: &mut L, pos: BlockPos, face: Direction, y: f64, check_hit: bool) -> (bool, bool) {
    let s = level.block(pos);
    let proper = !check_hit || is_proper_hit(s, face, y);
    if !proper {
        return (false, false);
    }
    (true, attempt_to_ring(level, pos, Some(face)))
}

/// `BellBlock.neighborChanged`: a bell rings when power reaches it.
pub fn neighbor_changed<L: Level + ?Sized>(level: &mut L, s: u16, pos: BlockPos) {
    let signal = has_neighbor_signal(level, pos);
    if signal != state::get_bool(s, "powered") {
        if signal {
            attempt_to_ring(level, pos, None);
        }
        set_block_and_update(level, pos, state::set_bool(s, "powered", signal));
    }
}

/// `BellBlock.triggerEvent` (`BellBlockEntity.triggerEvent`): event 1 starts the shaking.
pub fn trigger_event<L: Level + ?Sized>(level: &mut L, pos: BlockPos, a: i32, b: i32) -> bool {
    if a != 1 {
        return false;
    }
    let dir = Direction::from_index(b.clamp(0, 5) as usize);
    level.bell_event(pos, dir)
}

#[allow(dead_code)]
const _: u32 = flags::CLIENTS;
