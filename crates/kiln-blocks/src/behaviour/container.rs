//! The block side of container blocks: chest connection shapes (`ChestBlock`), hopper and
//! dispenser redstone (`HopperBlock.checkPoweredState`, `DispenserBlock.neighborChanged`),
//! trapped chest signals, and their placement states. Contents, menus and block entity ticks
//! are the level's (see [`Level::block_entity_tick`] and [`Level::container_openers`]).

use crate::level::{Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{set_block, update_neighbors_at};
use kiln_data::block_logic::{self as logic, BlockClass as C};

/// `ChestBlock`, `TrappedChestBlock` and the copper chests.
pub fn is_chest(s: u16) -> bool {
    logic::is_instance(s, C::ChestBlock)
}

fn is_copper_chest(s: u16) -> bool {
    logic::is_instance(s, C::CopperChestBlock)
}

/// `ChestBlock.chestCanConnectTo`: the same block (copper chests: any copper chest).
pub fn chest_can_connect_to(chest: u16, other: u16) -> bool {
    if is_copper_chest(chest) { is_copper_chest(other) } else { state::same_block(chest, other) }
}

/// `ChestBlock.getConnectedDirection`: toward the other half of a double chest.
pub fn connected_direction(s: u16) -> Direction {
    let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
    if state::get(s, "type") == Some("left") { facing.clockwise() } else { facing.counter_clockwise() }
}

/// The other half of a double chest (`None` for a single chest).
pub fn chest_partner(s: u16, pos: BlockPos) -> Option<BlockPos> {
    (is_chest(s) && state::get(s, "type") != Some("single")).then(|| pos.relative(connected_direction(s)))
}

/// `ChestBlock.candidatePartnerFacing`: the facing of a single chest of this kind beside `pos`.
fn candidate_partner_facing<L: Level + ?Sized>(level: &L, chest: u16, pos: BlockPos, dir: Direction) -> Option<Direction> {
    let s = level.block(pos.relative(dir));
    (chest_can_connect_to(chest, s) && state::get(s, "type") == Some("single")).then(|| state::get_dir(s, "facing")).flatten()
}

/// `ChestBlock.getStateForPlacement`: facing the player, joined to a single chest beside it
/// that faces the same way (or, sneaking against a chest's side, to that chest).
pub fn chest_placement<L: Level + ?Sized>(level: &L, d: u16, pos: BlockPos, horizontal: Direction, clicked: Direction, sneaking: bool) -> u16 {
    let mut ty = "single";
    let mut facing = horizontal.opposite();
    if clicked.is_horizontal()
        && sneaking
        && let Some(partner) = candidate_partner_facing(level, d, pos, clicked.opposite())
        && partner.axis() != clicked.axis()
    {
        facing = partner;
        ty = if facing.counter_clockwise() == clicked.opposite() { "right" } else { "left" };
    }
    if ty == "single" && !sneaking {
        ty = if candidate_partner_facing(level, d, pos, facing.clockwise()) == Some(facing) {
            "left"
        } else if candidate_partner_facing(level, d, pos, facing.counter_clockwise()) == Some(facing) {
            "right"
        } else {
            "single"
        };
    }
    state::set(state::set_dir(d, "facing", facing), "type", ty)
}

/// `ChestBlock.updateShape` (after the waterlogged check): a single chest joins a neighbour
/// that became its other half; a double chest whose other half left becomes single.
pub fn chest_update_shape(s: u16, dir: Direction, neighbor: u16) -> u16 {
    let r = chest_update_shape_base(s, dir, neighbor);
    // `CopperChestBlock.updateShape`: a half of a double chest takes the weathering stage of the
    // half it is connected to (the neighbour's block with its own properties).
    if is_copper_chest(s) && is_copper_chest(neighbor) && state::get(r, "type") != Some("single") && connected_direction(r) == dir {
        return state::with_properties_of(BlockId::of(neighbor).default_state(), r);
    }
    r
}

fn chest_update_shape_base(s: u16, dir: Direction, neighbor: u16) -> u16 {
    if chest_can_connect_to(s, neighbor) && dir.is_horizontal() {
        let other = state::get(neighbor, "type").unwrap_or("single");
        if state::get(s, "type") == Some("single")
            && other != "single"
            && state::get_dir(s, "facing") == state::get_dir(neighbor, "facing")
            && connected_direction(neighbor) == dir.opposite()
        {
            return state::set(s, "type", if other == "left" { "right" } else { "left" });
        }
    } else if connected_direction(s) == dir {
        return state::set(s, "type", "single");
    }
    s
}

/// `HopperBlock.getStateForPlacement`: toward the clicked block, down when clicking a top or
/// bottom face.
pub fn hopper_placement(d: u16, clicked: Direction) -> u16 {
    let dir = clicked.opposite();
    let facing = if dir == Direction::Up || dir == Direction::Down { Direction::Down } else { dir };
    state::set_bool(state::set_dir(d, "facing", facing), "enabled", true)
}

/// `HopperBlock.checkPoweredState`: a powered hopper is locked.
pub fn hopper_check_powered<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let enabled = !has_neighbor_signal(level, pos);
    if enabled != state::get_bool(s, "enabled") {
        set_block(level, pos, state::set_bool(s, "enabled", enabled), flags::CLIENTS);
    }
}

/// `HopperBlock.onPlace`.
pub fn hopper_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if !state::same_block(old, s) {
        hopper_check_powered(level, s, pos);
    }
}

/// `DispenserBlock.neighborChanged`: a rising edge (quasi-connectivity: the block above
/// counts) dispenses 4 ticks later.
pub fn dispenser_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let powered = has_neighbor_signal(level, pos) || has_neighbor_signal(level, pos.above());
    let triggered = state::get_bool(s, "triggered");
    if powered && !triggered {
        schedule_block_tick(level, pos, BlockId::of(s), 4, TickPriority::Normal);
        set_block(level, pos, state::set_bool(s, "triggered", true), flags::CLIENTS);
    } else if !powered && triggered {
        set_block(level, pos, state::set_bool(s, "triggered", false), flags::CLIENTS);
    }
}

/// `TrappedChestBlock.getSignal`: the number of players looking inside, at most 15.
pub fn trapped_chest_signal<L: Level + ?Sized>(level: &L, pos: BlockPos) -> i32 {
    level.container_openers(pos).clamp(0, 15)
}

/// `TrappedChestBlockEntity.signalOpenCount`: the chest and the block below re-read the signal
/// when the count changes.
pub fn trapped_chest_count_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, before: i32, after: i32) {
    if before != after {
        let block = BlockId::of(s);
        update_neighbors_at(level, pos, block);
        update_neighbors_at(level, pos.below(), block);
    }
}

/// `ShulkerBoxBlock.getStateForPlacement`: facing the clicked face.
pub fn shulker_placement(d: u16, clicked: Direction) -> u16 {
    state::set_dir(d, "facing", clicked)
}
