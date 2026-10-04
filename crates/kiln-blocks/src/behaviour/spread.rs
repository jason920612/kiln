//! What spreads, melts and dies on its own: grass and mycelium (`SpreadingSnowyBlock`), snow
//! layers, ice and frosted ice. Each function follows the vanilla method call by call: the
//! order and number of random draws is part of the behaviour.

use super::connect;
use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use kiln_data::block_logic::{self as logic, FluidKind};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

/// `Mth.nextInt(random, min, max)`: `min` to `max` inclusive.
pub(crate) fn mth_next_int<R: RandomSource + ?Sized>(random: &mut R, min: i32, max: i32) -> i32 {
    if min >= max { min } else { random.next_int_bounded(max - min + 1) + min }
}

/// `LightEngine.isEmptyShape`.
fn empty_shape(s: u16) -> bool {
    !block_props::can_occlude(s) || !block_props::uses_shape_for_light_occlusion(s)
}

/// `LightEngine.getLightDampeningInto(from, to, dir, to's dampening)`: 16 when the two
/// occlusion shapes close the face between them, else the dampening of `to`.
pub fn light_dampening_into(from: u16, to: u16, dir: Direction) -> i32 {
    let (from_empty, to_empty) = (empty_shape(from), empty_shape(to));
    let dampening = i32::from(block_props::light_dampening(to));
    if from_empty && to_empty {
        return dampening;
    }
    let closes = (!from_empty && block_props::face_full(from, dir as u8)) || (!to_empty && block_props::face_full(to, dir.opposite() as u8));
    if closes { 16 } else { dampening }
}

/// `SpreadingSnowyBlock.canStayAlive`.
fn can_stay_alive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let above = level.block(pos.above());
    if state::is(above, d::SNOW) && state::get_int(above, "layers") == 1 {
        return true;
    }
    let f = logic::fluid(above);
    if !f.is_empty() && f.amount == 8 {
        return false;
    }
    light_dampening_into(s, above, Direction::Up) < 15
}

/// `SpreadingSnowyBlock.canPropagate`.
fn can_propagate<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    can_stay_alive(level, s, pos) && logic::fluid(level.block(pos.above())).kind != FluidKind::Water
}

/// `SpreadingSnowyBlock.randomTick` (grass block, mycelium): dies to dirt when covered, and
/// spreads to four random dirt blocks around it in light.
pub fn spreading_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !can_stay_alive(level, s, pos) {
        crate::set_block_and_update(level, pos, d::DIRT);
        return;
    }
    if level.max_local_raw_brightness(pos.above()) >= 9 {
        let spread = BlockId::of(s).default_state();
        for _ in 0..4 {
            let dx = level.random().next_int_bounded(3) - 1;
            let dy = level.random().next_int_bounded(5) - 3;
            let dz = level.random().next_int_bounded(3) - 1;
            let target = pos.offset(dx, dy, dz);
            if state::is(level.block(target), d::DIRT) && can_propagate(level, spread, target) {
                let snowy = connect::snowy_setting(level.block(target.above()));
                crate::set_block_and_update(level, target, state::set_bool(spread, "snowy", snowy));
            }
        }
    }
}

/// `SnowLayerBlock.randomTick`: block light above 11 melts the layers away.
pub fn snow_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.block_light(pos) > 11 {
        level.effect(Effect::Drop { pos, state: s });
        crate::remove_block(level, pos, false);
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_destroy", state: s });
    }
}

/// `IceBlock.randomTick` (also frosted ice's).
pub fn ice_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.block_light(pos) > 11 - i32::from(block_props::light_dampening(s)) {
        melt(level, s, pos);
    }
}

/// `IceBlock.melt`: water, or nothing where water evaporates.
pub fn melt<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.rules().water_evaporates {
        crate::remove_block(level, pos, false);
        return;
    }
    crate::set_block_and_update(level, pos, d::WATER);
    crate::update::neighbor_changed_at(level, pos, BlockId::of(d::WATER));
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_destroy", state: s });
}

/// `FrostedIceBlock.onPlace`: the first fade check in 60 to 120 ticks.
pub fn frosted_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let delay = mth_next_int(level.random(), 60, 120);
    schedule_block_tick(level, pos, BlockId::of(s), delay, TickPriority::Normal);
}

/// `FrostedIceBlock.fewerNeigboursThan`.
fn fewer_neighbours_than<L: Level + ?Sized>(level: &L, pos: BlockPos, n: i32) -> bool {
    let mut count = 0;
    for dir in Direction::ALL {
        if state::is(level.block(pos.relative(dir)), d::FROSTED_ICE) {
            count += 1;
            if count >= n {
                return false;
            }
        }
    }
    true
}

/// `FrostedIceBlock.slightlyMelt`: one more age, or the melt (true) at the last one.
fn slightly_melt<L: Level>(level: &mut L, s: u16, pos: BlockPos) -> bool {
    let age = state::get_int(s, "age");
    if age < 3 {
        crate::set_block(level, pos, state::set_int(s, "age", age + 1), flags::CLIENTS);
        return false;
    }
    melt(level, s, pos);
    true
}

/// `FrostedIceBlock.tick`.
pub fn frosted_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let id = BlockId::of(s);
    if level.random().next_int_bounded(3) == 0 || fewer_neighbours_than(level, pos, 4) {
        let brightness = if level.is_end() { level.block_light(pos) } else { level.max_local_raw_brightness(pos) };
        if brightness > 11 - state::get_int(s, "age") - i32::from(block_props::light_dampening(s)) && slightly_melt(level, s, pos) {
            for dir in Direction::ALL {
                let n = pos.relative(dir);
                let ns = level.block(n);
                if state::is(ns, d::FROSTED_ICE) && !slightly_melt(level, ns, n) {
                    let delay = mth_next_int(level.random(), 20, 40);
                    schedule_block_tick(level, n, id, delay, TickPriority::Normal);
                }
            }
            return;
        }
    }
    let delay = mth_next_int(level.random(), 20, 40);
    schedule_block_tick(level, pos, id, delay, TickPriority::Normal);
}

/// `FrostedIceBlock.neighborChanged`: a frosted ice block that lost its neighbours melts.
pub fn frosted_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId) {
    if source == BlockId::of(d::FROSTED_ICE) && fewer_neighbours_than(level, pos, 2) {
        melt(level, s, pos);
    }
}
