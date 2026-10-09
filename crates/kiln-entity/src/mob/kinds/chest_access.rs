//! Chests as mobs reach them (the copper golem's `TransportItemsBetweenContainers`):
//! `ChestBlock.getContainer(.., false)` (a double chest is one container of both halves, a
//! blocked chest none), `ChestBlock.isChestBlockedAt`, and the container operations.

use crate::level::{EntityFilter, EntityLevel};
use crate::math::{Aabb, BlockPos, Direction};
use kiln_data::block_logic::{self, BlockClass as C};
use kiln_item::ItemStack;

/// `state.getBlock() instanceof ChestBlock` (chests, trapped chests, copper chests).
pub fn is_chest_block(state: u16) -> bool {
    block_logic::is_instance(state, C::ChestBlock)
}

fn prop(state: u16, name: &str) -> Option<&'static str> {
    kiln_data::blocks_types::block_of(state).property(state, name)
}

fn horizontal(name: &str) -> Direction {
    match name {
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        _ => Direction::East,
    }
}

fn clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        _ => Direction::North,
    }
}

fn counter_clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::West,
        Direction::West => Direction::South,
        Direction::South => Direction::East,
        _ => Direction::North,
    }
}

/// `ChestBlock.TYPE`: `Some(true)` for a left half, `Some(false)` right, `None` single.
pub fn double_half(state: u16) -> Option<bool> {
    match prop(state, "type") {
        Some("left") => Some(true),
        Some("right") => Some(false),
        _ => None,
    }
}

/// `ChestBlock.getConnectedBlockPos`.
pub fn connected_pos(pos: BlockPos, state: u16) -> BlockPos {
    let facing = horizontal(prop(state, "facing").unwrap_or("north"));
    pos.relative(if double_half(state) == Some(true) { clockwise(facing) } else { counter_clockwise(facing) })
}

/// `ChestBlock.isChestBlockedAt`: a redstone conductor above, or a sitting cat on top.
pub fn blocked_at(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    if block_logic::is_redstone_conductor(level.block(pos.above())) {
        return true;
    }
    let area = Aabb::new(pos.x as f64, (pos.y + 1) as f64, pos.z as f64, (pos.x + 1) as f64, (pos.y + 2) as f64, (pos.z + 1) as f64);
    level.entities_in(&area, EntityFilter::Living, -1).into_iter().any(|id| {
        level.entity(id).and_then(crate::mob::data).is_some_and(|m| m.kind == crate::mob::MobKind::Cat && super::tame::get(m).is_some_and(|t| t.sitting))
    })
}

/// `ChestBlock.getContainer(chest, state, level, pos, false)`: the block entity positions of
/// the container, first half then second (one for a single chest), `None` when the chest
/// (or the half beside it) is blocked or has no block entity.
pub fn combine(level: &dyn EntityLevel, pos: BlockPos, state: u16) -> Option<Vec<BlockPos>> {
    level.block_entity_serial(pos)?;
    if blocked_at(level, pos) {
        return None;
    }
    let Some(this_is_left) = double_half(state) else { return Some(vec![pos]) };
    let other = connected_pos(pos, state);
    let os = level.block(other);
    let same_block = kiln_data::blocks_types::block_of(os).name == kiln_data::blocks_types::block_of(state).name;
    // `getBlockType`: right is FIRST, left SECOND; the neighbour must be the other half, facing alike.
    if same_block && double_half(os) == Some(!this_is_left) && prop(os, "facing") == prop(state, "facing") {
        if blocked_at(level, other) {
            return None;
        }
        if level.block_entity_serial(other).is_some() {
            // `this_is_left`: this block is SECOND.
            return Some(if this_is_left { vec![other, pos] } else { vec![pos, other] });
        }
    }
    Some(vec![pos])
}

/// The slots of a combined container (`CompoundContainer`: first half's, then second's).
pub fn items(level: &dyn EntityLevel, halves: &[BlockPos]) -> Vec<ItemStack> {
    halves.iter().flat_map(|&p| level.container_items(p).unwrap_or_default()).collect()
}

/// Writes the slots of a combined container back (`setChanged` on each half).
pub fn set_items(level: &mut dyn EntityLevel, halves: &[BlockPos], mut all: Vec<ItemStack>) {
    for &p in halves {
        let n = level.container_items(p).map_or(0, |v| v.len());
        let rest = all.split_off(n.min(all.len()));
        level.set_container_items(p, std::mem::replace(&mut all, rest));
    }
}
