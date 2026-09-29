//! Leaves (`LeavesBlock` distance and decay), falling blocks (`FallingBlock`), fence gates
//! (`FenceGateBlock`) and trapdoors (`TrapDoorBlock`).

use crate::fluid;
use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{drop_and_remove, set_block, set_block_and_update};
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_data::block_props;
use kiln_data::blocks_types::is_air;

/// Whether `s` has the leaves' `DISTANCE` property (1..=7), not scaffolding's 0..=7.
fn has_leaf_distance(s: u16) -> bool {
    state::has(s, "distance") && logic::is_instance(s, BlockClass::LeavesBlock)
}

/// `LeavesBlock.getDistanceAt`.
fn distance_at(s: u16) -> i32 {
    if tags::is(s, "minecraft:prevents_nearby_leaf_decay") {
        0
    } else if has_leaf_distance(s) {
        state::get_int(s, "distance")
    } else {
        7
    }
}

/// `LeavesBlock.updateDistance`.
pub fn leaves_distance<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> u16 {
    let mut d = 7;
    for dir in Direction::ALL {
        d = d.min(distance_at(level.block(pos.relative(dir))) + 1);
        if d == 1 {
            break;
        }
    }
    state::set_int(s, "distance", d)
}

/// `LeavesBlock.updateShape`: re-check distance next tick when a neighbour could change it.
pub fn leaves_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, neighbor: u16) -> u16 {
    let d = distance_at(neighbor) + 1;
    if d != 1 || state::get_int(s, "distance") != d {
        schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
    }
    s
}

pub fn leaves_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let new = leaves_distance(level, s, pos);
    set_block_and_update(level, pos, new);
}

/// `LeavesBlock.randomTick`: far from logs and not player-placed, leaves decay.
pub fn leaves_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !state::get_bool(s, "persistent") && state::get_int(s, "distance") == 7 {
        drop_and_remove(level, pos, s);
    }
}

fn fall_delay(s: u16) -> i32 {
    if logic::block_class(s) == BlockClass::DragonEggBlock { 5 } else { 2 }
}

/// `FallingBlock.onPlace` / `updateShape`: check for falling shortly.
pub fn falling_schedule<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    schedule_block_tick(level, pos, BlockId::of(s), fall_delay(s), TickPriority::Normal);
}

/// `FallingBlock.isFree`.
pub fn is_free(s: u16) -> bool {
    is_air(s) || tags::is(s, "minecraft:fire") || block_props::liquid(s) || block_props::replaceable(s)
}

/// `FallingBlock.tick`: with nothing below, the block becomes a falling entity (the level
/// spawns it from [`Effect::FallingBlock`]).
pub fn falling_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !is_free(level.block(pos.below())) || !level.in_bounds(pos.below()) {
        return;
    }
    level.effect(Effect::FallingBlock { pos, state: s });
    set_block(level, pos, fluid::legacy_block(logic::fluid(s)), flags::ALL);
}

/// `FenceGateBlock.updateShape`: lowered between walls.
pub fn gate_update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
    if facing.clockwise().axis() != dir.axis() {
        return s;
    }
    let wall = |x: u16| tags::is(x, "minecraft:walls");
    state::set_bool(s, "in_wall", wall(neighbor) || wall(level.block(pos.relative(dir.opposite()))))
}

/// `FenceGateBlock` / `TrapDoorBlock` `neighborChanged`: power opens them.
pub fn powered_open_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let powered = has_neighbor_signal(level, pos);
    if powered == state::get_bool(s, "powered") {
        return;
    }
    let trapdoor = logic::is_instance(s, BlockClass::TrapDoorBlock);
    let new = state::set_bool(state::set_bool(s, "powered", powered), "open", powered);
    set_block(level, pos, new, flags::CLIENTS);
    if state::get_bool(s, "open") != powered {
        level.effect(Effect::GameEvent { pos, event: if powered { "minecraft:block_open" } else { "minecraft:block_close" } });
    }
    if trapdoor && state::get_bool(s, "waterlogged") {
        fluid::tick_water_if_waterlogged(level, new, pos);
    }
}

/// `SnifferEggBlock.onPlace`: the first crack in 8000 ticks (4000 on moss) plus up to 300.
pub fn sniffer_egg_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let boosted = tags::is(level.block(pos.below()), "minecraft:sniffer_egg_hatch_boost");
    if boosted {
        level.effect(Effect::LevelEvent { id: 3009, pos, data: 0 });
    }
    let delay = if boosted { 12000 } else { 24000 } / 3;
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_place" });
    let extra = kiln_javamath::random::RandomSource::next_int_bounded(level.random(), 300);
    schedule_block_tick(level, pos, BlockId::of(s), delay + extra, TickPriority::Normal);
}

/// `SnifferEggBlock.tick`: a crack (hatch 0 to 2), then the hatching: the egg breaks and the
/// level spawns a baby sniffer from [`Effect::HatchSniffer`].
pub fn sniffer_egg_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let hatch = state::get_int(s, "hatch");
    let pitch = 0.9 + kiln_javamath::random::RandomSource::next_float(level.random()) * 0.2;
    if hatch < 2 {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.sniffer_egg.crack", volume: 0.7, pitch });
        set_block(level, pos, state::set_int(s, "hatch", hatch + 1), flags::CLIENTS);
    } else {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.sniffer_egg.hatch", volume: 0.7, pitch });
        crate::update::destroy_block(level, pos, false, 512);
        level.effect(Effect::HatchSniffer { pos });
    }
}
