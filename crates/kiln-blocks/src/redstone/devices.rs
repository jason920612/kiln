//! Observers (`ObserverBlock`), note blocks (`NoteBlock`) and TNT (`TntBlock`).

use super::has_neighbor_signal;
use crate::block_events::block_event;
use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{neighbor_changed_at, remove_block, set_block, set_block_and_update, update_neighbors_at_except};
use kiln_data::block_logic as logic;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use std::sync::OnceLock;

fn facing(s: u16) -> Direction {
    state::get_dir(s, "facing").unwrap_or(Direction::North)
}

fn observer_front<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let f = facing(s);
    let front = pos.relative(f.opposite());
    neighbor_changed_at(level, front, BlockId::of(s));
    update_neighbors_at_except(level, front, BlockId::of(s), Some(f));
}

/// `ObserverBlock.updateShape`: a change in front starts a pulse.
pub fn observer_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if facing(s) == dir && !state::get_bool(s, "powered") && !level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)) {
        schedule_block_tick(level, pos, BlockId::of(s), 2, TickPriority::Normal);
    }
    s
}

pub fn observer_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "powered") {
        set_block(level, pos, state::set_bool(s, "powered", false), flags::CLIENTS);
    } else {
        set_block(level, pos, state::set_bool(s, "powered", true), flags::CLIENTS);
        schedule_block_tick(level, pos, BlockId::of(s), 2, TickPriority::Normal);
    }
    observer_front(level, pos, s);
}

pub fn observer_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if state::same_block(old, s) || !state::get_bool(s, "powered") || level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)) {
        return;
    }
    let off = state::set_bool(s, "powered", false);
    set_block(level, pos, off, flags::CLIENTS | flags::KNOWN_SHAPE);
    observer_front(level, pos, off);
}

pub fn observer_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "powered") && level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)) {
        observer_front(level, pos, state::set_bool(s, "powered", false));
    }
}

/// `TntBlock.prime`: the level spawns the primed TNT from [`Effect::PrimedTnt`].
pub(crate) fn prime<L: Level>(level: &mut L, pos: BlockPos) -> bool {
    if !level.rules().tnt_explodes {
        return false;
    }
    level.effect(Effect::PrimedTnt { pos });
    level.effect(Effect::Sound { pos, sound: "minecraft:entity.tnt.primed", volume: 1.0, pitch: 1.0 });
    level.effect(Effect::GameEvent { pos, event: "minecraft:prime_fuse" });
    true
}

pub fn tnt_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if !state::same_block(old, s) && has_neighbor_signal(level, pos) && prime(level, pos) {
        remove_block(level, pos, false);
    }
}

pub fn tnt_neighbor_changed<L: Level>(level: &mut L, pos: BlockPos) {
    if has_neighbor_signal(level, pos) && prime(level, pos) {
        remove_block(level, pos, false);
    }
}

fn instrument_names() -> &'static [&'static str] {
    BlockId::of(d::NOTE_BLOCK).info().properties.iter().find(|p| p.name == "instrument").map_or(&[], |p| p.values)
}

/// `NoteBlockInstrument.worksAboveNoteBlock` by instrument index.
fn works_above(index: u8) -> bool {
    static TABLE: OnceLock<[bool; 32]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [false; 32];
        for s in 0..kiln_data::blocks::STATE_COUNT as u16 {
            let (i, above, _) = logic::instrument(s);
            t[i as usize] |= above;
        }
        t
    })[index as usize & 31]
}

/// `NoteBlock.setInstrument`: a head above picks the sound, else the block below.
pub fn note_instrument<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> u16 {
    let names = instrument_names();
    let (above, above_works, _) = logic::instrument(level.block(pos.above()));
    let index = if above_works {
        above
    } else {
        let (below, below_works, _) = logic::instrument(level.block(pos.below()));
        if below_works { 0 } else { below }
    };
    names.get(index as usize).map_or(s, |n| state::set(s, "instrument", n))
}

pub fn note_update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if dir.axis() == crate::pos::Axis::Y { note_instrument(level, pos, s) } else { s }
}

pub fn note_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let powered = has_neighbor_signal(level, pos);
    if powered == state::get_bool(s, "powered") {
        return;
    }
    if powered {
        play_note(level, s, pos);
    }
    set_block_and_update(level, pos, state::set_bool(s, "powered", powered));
}

/// `NoteBlock.playNote`: the note sounds (through a block event) unless a block on top
/// muffles it.
pub fn play_note<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let index = state::value_index(s, "instrument").unwrap_or(0) as u8;
    if works_above(index) || is_air(level.block(pos.above())) {
        block_event(level, pos, BlockId::of(s), 0, 0);
        level.effect(Effect::GameEvent { pos, event: "minecraft:note_block_play" });
    }
}

/// `NoteBlock.triggerEvent`: the note sounds for everyone nearby.
pub fn note_trigger<L: Level>(level: &mut L, s: u16, pos: BlockPos) -> bool {
    let instrument = state::get(s, "instrument").unwrap_or("harp");
    level.effect(Effect::NoteBlock { pos, instrument, note: state::get_int(s, "note") });
    true
}
