//! Simple redstone components: levers (`LeverBlock`), buttons (`ButtonBlock`), lamps
//! (`RedstoneLampBlock`) and doors opened by power (`DoorBlock`).

use super::has_neighbor_signal;
use crate::behaviour::support::attached_direction;
use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{set_block, set_block_and_update, update_neighbors_at};
use kiln_data::block_logic as logic;

/// `LeverBlock` / `ButtonBlock` `updateNeighbours`: the block itself and the one it is on.
fn update_attached<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let block = BlockId::of(s);
    update_neighbors_at(level, pos, block);
    update_neighbors_at(level, pos.relative(attached_direction(s).opposite()), block);
}

/// Lever and button removal: a powered one takes its power away.
pub fn attached_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if !moved_by_piston && state::get_bool(s, "powered") {
        update_attached(level, pos, s);
    }
}

/// `LeverBlock.pull`.
pub fn pull_lever<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let s = state::set_bool(s, "powered", !state::get_bool(s, "powered"));
    set_block_and_update(level, pos, s);
    update_attached(level, pos, s);
    let on = state::get_bool(s, "powered");
    level.effect(Effect::Sound { pos, sound: "minecraft:block.lever.click", volume: 0.3, pitch: if on { 0.6 } else { 0.5 } });
    level.effect(Effect::GameEvent { pos, event: if on { "minecraft:block_activate" } else { "minecraft:block_deactivate" } });
}

/// `ButtonBlock.press`.
pub fn press_button<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let s = state::set_bool(s, "powered", true);
    set_block_and_update(level, pos, s);
    update_attached(level, pos, s);
    let ticks = logic::params(s).ticks_to_stay_pressed;
    schedule_block_tick(level, pos, BlockId::of(s), ticks, TickPriority::Normal);
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_activate" });
}

/// `ButtonBlock.tick` → `checkPressed`: no arrows are simulated, so a pressed button pops
/// back up.
pub fn button_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !state::get_bool(s, "powered") {
        return;
    }
    let s = state::set_bool(s, "powered", false);
    set_block_and_update(level, pos, s);
    update_attached(level, pos, s);
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_deactivate" });
}

pub fn lamp_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let lit = state::get_bool(s, "lit");
    if lit != has_neighbor_signal(level, pos) {
        if lit {
            schedule_block_tick(level, pos, BlockId::of(s), 4, TickPriority::Normal);
        } else {
            set_block(level, pos, state::set_bool(s, "lit", true), flags::CLIENTS);
        }
    }
}

pub fn lamp_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "lit") && !has_neighbor_signal(level, pos) {
        set_block(level, pos, state::set_bool(s, "lit", false), flags::CLIENTS);
    }
}

/// `DoorBlock.neighborChanged`: either half powered opens the door.
pub fn door_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId) {
    let other = if state::get(s, "half") == Some("lower") { Direction::Up } else { Direction::Down };
    let powered = has_neighbor_signal(level, pos) || has_neighbor_signal(level, pos.relative(other));
    if BlockId::of(s) != source && powered != state::get_bool(s, "powered") {
        if powered != state::get_bool(s, "open") {
            level.effect(Effect::GameEvent { pos, event: if powered { "minecraft:block_open" } else { "minecraft:block_close" } });
        }
        let s = state::set_bool(state::set_bool(s, "powered", powered), "open", powered);
        set_block(level, pos, s, flags::CLIENTS);
    }
}
