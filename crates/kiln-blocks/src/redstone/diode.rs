//! Diodes (`DiodeBlock`): repeaters with delay and locking. Comparators need their block
//! entity's output and are not simulated yet.

use super::{control_input, signal};
use crate::behaviour::support::can_support_rigid;
use crate::level::{Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{drop_and_remove, neighbor_changed_at, set_block, update_neighbors_at, update_neighbors_at_except};
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_data::blocks::default_state as d;

fn facing(s: u16) -> Direction {
    state::get_dir(s, "facing").unwrap_or(Direction::North)
}

pub fn is_diode(s: u16) -> bool {
    logic::is_instance(s, BlockClass::DiodeBlock)
}

fn delay(s: u16) -> i32 {
    state::get_int(s, "delay") * 2
}

/// `getInputSignal`: from the block behind (toward `facing`).
fn input_signal<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> i32 {
    let f = facing(s);
    let n = pos.relative(f);
    let i = signal(level, n, f);
    if i >= 15 {
        return i;
    }
    let ns = level.block(n);
    i.max(if state::is(ns, d::REDSTONE_WIRE) { state::get_int(ns, "power") } else { 0 })
}

/// `getAlternateSignal`: side inputs (repeaters only listen to diodes there).
fn side_signal<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> i32 {
    let f = facing(s);
    let (cw, ccw) = (f.clockwise(), f.counter_clockwise());
    let diodes_only = logic::block_class(s) == BlockClass::RepeaterBlock;
    control_input(level, pos.relative(cw), cw, diodes_only).max(control_input(level, pos.relative(ccw), ccw, diodes_only))
}

/// `RepeaterBlock.isLocked`.
pub fn is_locked<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> bool {
    logic::block_class(s) == BlockClass::RepeaterBlock && side_signal(level, pos, s) > 0
}

fn should_turn_on<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> bool {
    input_signal(level, pos, s) > 0
}

/// `shouldPrioritize`: the block in front is a diode not facing back into this one.
fn should_prioritize<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> bool {
    let front_dir = facing(s).opposite();
    let front = level.block(pos.relative(front_dir));
    is_diode(front) && facing(front) != front_dir
}

/// `RepeaterBlock.updateShape`.
pub fn update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    if dir == Direction::Down && !can_support_rigid(neighbor) {
        return d::AIR;
    }
    if logic::block_class(s) == BlockClass::RepeaterBlock && dir.axis() != facing(s).axis() {
        return state::set_bool(s, "locked", is_locked(level, pos, s));
    }
    s
}

pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if is_locked(level, pos, s) {
        return;
    }
    let powered = state::get_bool(s, "powered");
    let on = should_turn_on(level, pos, s);
    if powered && !on {
        set_block(level, pos, state::set_bool(s, "powered", false), flags::CLIENTS);
    } else if !powered {
        set_block(level, pos, state::set_bool(s, "powered", true), flags::CLIENTS);
        if !on {
            schedule_block_tick(level, pos, BlockId::of(s), delay(s), TickPriority::VeryHigh);
        }
    }
}

fn check_tick_on_neighbor<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    if is_locked(level, pos, s) {
        return;
    }
    let powered = state::get_bool(s, "powered");
    let block = BlockId::of(s);
    if powered != should_turn_on(level, pos, s) && !level.block_ticks().will_tick_this_tick(pos, block) {
        let priority = if should_prioritize(level, pos, s) {
            TickPriority::ExtremelyHigh
        } else if powered {
            TickPriority::VeryHigh
        } else {
            TickPriority::High
        };
        schedule_block_tick(level, pos, block, delay(s), priority);
    }
}

pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !state::same_block(level.block(pos), s) {
        return;
    }
    if can_support_rigid(level.block(pos.below())) {
        check_tick_on_neighbor(level, pos, s);
        return;
    }
    drop_and_remove(level, pos, s);
    for dir in Direction::ALL {
        update_neighbors_at(level, pos.relative(dir), BlockId::of(s));
    }
}

/// `updateNeighborsInFront`.
fn update_front<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let f = facing(s);
    let front = pos.relative(f.opposite());
    neighbor_changed_at(level, front, BlockId::of(s));
    update_neighbors_at_except(level, front, BlockId::of(s), Some(f));
}

pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    update_front(level, pos, s);
}

pub fn affect_neighbors_after_removal<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if !moved_by_piston {
        update_front(level, pos, s);
    }
}

/// `DiodeBlock.setPlacedBy`: a diode placed onto a powered input turns on next tick.
pub fn placed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if should_turn_on(level, pos, s) {
        schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
    }
}

/// `RepeaterBlock.getStateForPlacement`.
pub fn placement<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> u16 {
    if logic::block_class(s) == BlockClass::RepeaterBlock { state::set_bool(s, "locked", is_locked(level, pos, s)) } else { s }
}
