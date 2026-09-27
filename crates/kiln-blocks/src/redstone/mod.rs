//! Redstone: signal queries (`SignalGetter`), wire, torches, diodes, levers, buttons, lamps
//! and the doors they open.

pub mod components;
pub mod devices;
pub mod diode;
pub mod torch;
pub mod wire;

use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::state;
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_data::blocks::default_state as d;

/// `BlockState.getSignal`: power `s` at `pos` sends toward `dir` (seen from the block at
/// `pos - dir`). `wires` is false while a wire measures its surroundings, which silences
/// all wires (`RedstoneWireBlock.shouldSignal`).
pub fn weak<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, wires: bool) -> i32 {
    match logic::block_class(s) {
        BlockClass::RedstoneWireBlock => {
            if wires {
                wire::signal(level, s, pos, dir)
            } else {
                0
            }
        }
        _ => logic::weak_signal(s, dir as u8) as i32,
    }
}

/// `BlockState.getDirectSignal`: strong power.
pub fn strong<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, wires: bool) -> i32 {
    match logic::block_class(s) {
        BlockClass::RedstoneWireBlock => {
            if wires {
                wire::signal(level, s, pos, dir)
            } else {
                0
            }
        }
        _ => logic::strong_signal(s, dir as u8) as i32,
    }
}

/// `BlockState.isSignalSource`.
pub fn is_source(s: u16) -> bool {
    logic::is_signal_source(s)
}

/// `SignalGetter.getSignal(pos, dir)`: the block's own output plus, for a conductor, the
/// strongest strong power it receives.
pub fn signal_with<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction, wires: bool) -> i32 {
    let s = level.block(pos);
    let own = weak(level, s, pos, dir, wires);
    if logic::is_redstone_conductor(s) { own.max(direct_signal_to(level, pos, wires)) } else { own }
}

pub fn signal<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> i32 {
    signal_with(level, pos, dir, true)
}

/// `SignalGetter.getDirectSignalTo`.
pub fn direct_signal_to<L: Level + ?Sized>(level: &L, pos: BlockPos, wires: bool) -> i32 {
    let mut best = 0;
    for dir in Direction::ALL {
        let n = pos.relative(dir);
        best = best.max(strong(level, level.block(n), n, dir, wires));
        if best >= 15 {
            return best;
        }
    }
    best
}

/// `SignalGetter.hasSignal`.
pub fn has_signal<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> bool {
    signal(level, pos, dir) > 0
}

/// `SignalGetter.hasNeighborSignal`.
pub fn has_neighbor_signal<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    Direction::ALL.iter().any(|&dir| has_signal(level, pos.relative(dir), dir))
}

/// `SignalGetter.getBestNeighborSignal`.
pub fn best_neighbor_signal<L: Level + ?Sized>(level: &L, pos: BlockPos, wires: bool) -> i32 {
    let mut best = 0;
    for dir in Direction::ALL {
        let s = signal_with(level, pos.relative(dir), dir, wires);
        if s >= 15 {
            return 15;
        }
        best = best.max(s);
    }
    best
}

/// `SignalGetter.getControlInputSignal`: what a diode's side input reads.
pub fn control_input<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction, diodes_only: bool) -> i32 {
    let s = level.block(pos);
    if diodes_only {
        return if logic::is_instance(s, BlockClass::DiodeBlock) { strong(level, s, pos, dir, true) } else { 0 };
    }
    if state::is(s, d::REDSTONE_BLOCK) {
        15
    } else if state::is(s, d::REDSTONE_WIRE) {
        state::get_int(s, "power")
    } else if is_source(s) {
        strong(level, s, pos, dir, true)
    } else {
        0
    }
}
