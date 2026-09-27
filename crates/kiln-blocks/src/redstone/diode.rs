//! Diodes (`DiodeBlock`): repeaters with delay and locking, comparators (`ComparatorBlock`)
//! with compare/subtract modes reading analog outputs. A comparator's output lives in its
//! block entity, kept by the level ([`Level::comparator_output`]).

use super::{analog, control_input, signal};
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

fn is_comparator(s: u16) -> bool {
    logic::block_class(s) == BlockClass::ComparatorBlock
}

fn delay(s: u16) -> i32 {
    if is_comparator(s) { 2 } else { state::get_int(s, "delay") * 2 }
}

/// `DiodeBlock.getInputSignal`, with `ComparatorBlock`'s analog reading.
fn input_signal<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> i32 {
    let f = facing(s);
    let n = pos.relative(f);
    let mut i = signal(level, n, f);
    if i < 15 {
        let ns = level.block(n);
        i = i.max(if state::is(ns, d::REDSTONE_WIRE) { state::get_int(ns, "power") } else { 0 });
    }
    if !is_comparator(s) {
        return i;
    }
    let ns = level.block(n);
    if logic::has_analog_output(ns) {
        return analog::output(level, ns, n, f.opposite());
    }
    if i < 15 && logic::is_redstone_conductor(ns) {
        let far = n.relative(f);
        let fs = level.block(far);
        let frame = level.item_frame_analog(far, f).unwrap_or(i32::MIN);
        let block = if logic::has_analog_output(fs) { analog::output(level, fs, far, f.opposite()) } else { i32::MIN };
        let best = frame.max(block);
        if best != i32::MIN {
            i = best;
        }
    }
    i
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

fn subtract(s: u16) -> bool {
    state::get(s, "mode") == Some("subtract")
}

fn should_turn_on<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> bool {
    let input = input_signal(level, pos, s);
    if !is_comparator(s) {
        return input > 0;
    }
    if input == 0 {
        return false;
    }
    let side = side_signal(level, pos, s);
    input > side || input == side && !subtract(s)
}

/// `ComparatorBlock.calculateOutputSignal`.
fn comparator_output<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> i32 {
    let input = input_signal(level, pos, s);
    if input == 0 {
        return 0;
    }
    let side = side_signal(level, pos, s);
    if side > input {
        0
    } else if subtract(s) {
        input - side
    } else {
        input
    }
}

/// The diode's output toward its front (`DiodeBlock.getSignal` for `dir == facing`).
pub fn output_signal<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> i32 {
    if facing(s) != dir || !state::get_bool(s, "powered") {
        return 0;
    }
    if is_comparator(s) { level.comparator_output(pos) } else { 15 }
}

/// `shouldPrioritize`: the block in front is a diode not facing back into this one.
fn should_prioritize<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> bool {
    let front_dir = facing(s).opposite();
    let front = level.block(pos.relative(front_dir));
    is_diode(front) && facing(front) != front_dir
}

/// `RepeaterBlock` / `ComparatorBlock` `updateShape`.
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
    if is_comparator(s) {
        refresh_comparator(level, pos, s);
        return;
    }
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

/// `ComparatorBlock.refreshOutputState`.
fn refresh_comparator<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let out = comparator_output(level, pos, s);
    let old = level.comparator_output(pos);
    level.set_comparator_output(pos, out);
    if old == out && subtract(s) {
        return;
    }
    let on = should_turn_on(level, pos, s);
    let powered = state::get_bool(s, "powered");
    if powered && !on {
        set_block(level, pos, state::set_bool(s, "powered", false), flags::CLIENTS);
    } else if !powered && on {
        set_block(level, pos, state::set_bool(s, "powered", true), flags::CLIENTS);
    }
    update_front(level, pos, s);
}

fn check_tick_on_neighbor<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let block = BlockId::of(s);
    if is_comparator(s) {
        if level.block_ticks().will_tick_this_tick(pos, block) {
            return;
        }
        let out = comparator_output(level, pos, s);
        if out != level.comparator_output(pos) || state::get_bool(s, "powered") != should_turn_on(level, pos, s) {
            let priority = if should_prioritize(level, pos, s) { TickPriority::High } else { TickPriority::Normal };
            schedule_block_tick(level, pos, block, 2, priority);
        }
        return;
    }
    if is_locked(level, pos, s) {
        return;
    }
    let powered = state::get_bool(s, "powered");
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
