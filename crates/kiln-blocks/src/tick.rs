//! The block phases of a level tick (`ServerLevel.tick`): scheduled block ticks, scheduled
//! fluid ticks, random ticks (per chunk, driven by the caller's chunk loop) and block events.

use crate::behaviour;
use crate::fluid::{self, FluidType};
use crate::level::Level;
use crate::pos::BlockPos;
use crate::state::BlockId;
use crate::ticks::ChunkKey;
use kiln_data::block_logic as logic;
use kiln_data::block_props;
use kiln_javamath::random::RandomSource;

/// Vanilla's cap on scheduled ticks of each kind run per game tick.
pub const MAX_TICKS: usize = 65536;

/// `LevelTicks.tick` for block ticks with `ServerLevel.tickBlock`: runs the block ticks due
/// at the current game time in chunks `can_tick` accepts.
/// Starts `ServerLevel.handlingTick`.
pub fn run_block_ticks<L: Level>(level: &mut L, can_tick: impl FnMut(ChunkKey) -> bool) {
    level.data().handling_tick = true;
    let time = level.game_time();
    level.block_ticks().collect(time, MAX_TICKS, can_tick);
    while let Some(t) = level.block_ticks().next_to_run() {
        let s = level.block(t.pos);
        if BlockId::of(s) == t.kind {
            behaviour::tick(level, s, t.pos);
        }
    }
    level.block_ticks().finish_tick();
}

/// `LevelTicks.tick` for fluid ticks with `ServerLevel.tickFluid`.
pub fn run_fluid_ticks<L: Level>(level: &mut L, can_tick: impl FnMut(ChunkKey) -> bool) {
    let time = level.game_time();
    level.fluid_ticks().collect(time, MAX_TICKS, can_tick);
    while let Some(t) = level.fluid_ticks().next_to_run() {
        let s = level.block(t.pos);
        if FluidType::of(logic::fluid(s)) == t.kind {
            fluid::tick(level, t.pos, s);
        }
    }
    level.fluid_ticks().finish_tick();
}

/// `Level.getBlockRandomPos`: advances the level's random-tick LCG.
pub fn block_random_pos<L: Level + ?Sized>(level: &mut L, x: i32, y: i32, z: i32, y_mask: i32) -> BlockPos {
    let data = level.data();
    data.rand_value = data.rand_value.wrapping_mul(3).wrapping_add(1013904223);
    let j = data.rand_value >> 2;
    BlockPos::new(x + (j & 15), y + ((j >> 16) & y_mask), z + (j >> 8 & 15))
}

/// Whether a state is picked up by random ticks (the block or its fluid ticks randomly).
pub fn randomly_ticks(state: u16) -> bool {
    block_props::randomly_ticks(state)
}

/// What `ServerLevel.tickChunk` does for a position it picked: the block's `randomTick` if
/// it ticks randomly, then its fluid's (lava) on the state read before the block's tick.
pub fn random_tick_at<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    if block_props::randomly_ticks(s) {
        behaviour::random_tick(level, s, pos);
    }
    // `FluidState.randomTick` of the state read before the block's tick (lava).
    if kiln_data::block_logic::fluid(s).kind == kiln_data::block_logic::FluidKind::Lava {
        crate::fire::lava_random_tick(level, pos);
    }
}

/// The block part of `ServerLevel.tickChunk` for the chunk at `chunk`: the precipitation
/// rolls (`tickPrecipitation`: ice, snow, cauldrons), then `speed` random ticks in each
/// section that `section_ticks` reports as holding randomly ticking blocks or fluids
/// (sections from the bottom, as `(section_y, ticking)`).
pub fn tick_chunk_blocks<L: Level>(level: &mut L, chunk: ChunkKey, sections: &[(i32, bool)], speed: i32) {
    let (x, z) = (chunk.0 * 16, chunk.1 * 16);
    for _ in 0..speed {
        if level.random().next_int_bounded(48) == 0 {
            let pos = block_random_pos(level, x, 0, z, 15);
            crate::weather::tick_precipitation(level, pos);
        }
    }
    if speed <= 0 {
        return;
    }
    for &(sy, ticking) in sections {
        if !ticking {
            continue;
        }
        for _ in 0..speed {
            let pos = block_random_pos(level, x, sy * 16, z, 15);
            random_tick_at(level, pos);
        }
    }
}
