//! Simple redstone components: levers (`LeverBlock`), buttons (`ButtonBlock`), lamps
//! (`RedstoneLampBlock`) and doors opened by power (`DoorBlock`).

use super::has_neighbor_signal;
use crate::behaviour::support::attached_direction;
use crate::level::{Effect, EntityKind, Level, flags, schedule_block_tick};
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

fn is_weighted(s: u16) -> bool {
    logic::block_class(s) == logic::BlockClass::WeightedPressurePlateBlock
}

/// `getSignalForState`.
fn plate_signal(s: u16) -> i32 {
    if is_weighted(s) { state::get_int(s, "power") } else if state::get_bool(s, "powered") { 15 } else { 0 }
}

/// `getSignalStrength`: from the entities on the plate.
fn plate_strength<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> i32 {
    let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
    let (min, max) = ([x + 0.0625, y, z + 0.0625], [x + 0.9375, y + 0.25, z + 0.9375]);
    let p = logic::params(s);
    if is_weighted(s) {
        let n = (level.count_entities(min, max, EntityKind::Any) as i32).min(p.max_weight);
        if n > 0 { ((n.min(p.max_weight) as f32 / p.max_weight as f32) * 15.0).ceil() as i32 } else { 0 }
    } else if level.count_entities(min, max, if p.plate_mobs_only { EntityKind::Living } else { EntityKind::Any }) > 0 {
        15
    } else {
        0
    }
}

fn plate_update_neighbours<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    update_neighbors_at(level, pos, BlockId::of(s));
    update_neighbors_at(level, pos.below(), BlockId::of(s));
}

/// `BasePressurePlateBlock.checkPressed`.
fn plate_check<L: Level>(level: &mut L, pos: BlockPos, s: u16, old: i32) {
    let new = plate_strength(level, pos, s);
    if old != new {
        let ns = if is_weighted(s) { state::set_int(s, "power", new) } else { state::set_bool(s, "powered", new > 0) };
        set_block(level, pos, ns, flags::CLIENTS);
        plate_update_neighbours(level, pos, s);
    }
    if new == 0 && old > 0 {
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_deactivate" });
    } else if new > 0 && old == 0 {
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_activate" });
    }
    if new > 0 {
        let delay = if is_weighted(s) { 10 } else { 20 };
        schedule_block_tick(level, pos, BlockId::of(s), delay, TickPriority::Normal);
    }
}

pub fn plate_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let old = plate_signal(s);
    if old > 0 {
        plate_check(level, pos, s, old);
    }
}

/// `BasePressurePlateBlock.entityInside`: the level calls this for an entity touching the
/// plate.
pub fn plate_entity_inside<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    if logic::is_instance(s, logic::BlockClass::BasePressurePlateBlock) && plate_signal(s) == 0 {
        plate_check(level, pos, s, 0);
    }
}

pub fn plate_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if !moved_by_piston && plate_signal(s) > 0 {
        plate_update_neighbours(level, pos, s);
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
