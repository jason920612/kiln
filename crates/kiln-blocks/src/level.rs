//! The world as block behaviour sees it.
//!
//! A [`Level`] stores block states and the per-level machinery vanilla keeps in `Level` /
//! `ServerLevel` (scheduled ticks, the neighbour-update queue, the level random, block
//! events). Everything that decides what happens (update order, shapes, fluids, redstone)
//! lives in this crate and runs against the trait, so the same code drives an in-memory test
//! level and a simulation region.

use crate::block_events::BlockEvents;
use crate::redstone::torch::Toggle;
use crate::fluid::FluidType;
use crate::pos::BlockPos;
use crate::state::BlockId;
use crate::ticks::{LevelTicks, ScheduledTick, TickPriority};
use crate::update::NeighborUpdater;
use kiln_javamath::random::RandomSource;

/// `Block.UPDATE_*` flags for [`crate::set_block`].
pub mod flags {
    pub const NEIGHBORS: u32 = 1;
    pub const CLIENTS: u32 = 2;
    pub const INVISIBLE: u32 = 4;
    pub const IMMEDIATE: u32 = 8;
    pub const KNOWN_SHAPE: u32 = 16;
    pub const SUPPRESS_DROPS: u32 = 32;
    pub const MOVE_BY_PISTON: u32 = 64;
    pub const SKIP_SHAPE_UPDATE_ON_WIRE: u32 = 128;
    pub const SKIP_BLOCK_ENTITY_SIDEEFFECTS: u32 = 256;
    pub const SKIP_ON_PLACE: u32 = 512;
    pub const NONE: u32 = 260;
    pub const ALL: u32 = 3;
    pub const ALL_IMMEDIATE: u32 = 11;
    pub const SKIP_ALL_SIDEEFFECTS: u32 = 816;
    /// Default recursion limit for shape updates (`Block.UPDATE_LIMIT`).
    pub const LIMIT: i32 = 512;
}

/// Game rules and dimension attributes block behaviour reads.
#[derive(Clone, Debug)]
pub struct Rules {
    /// `minecraft:water_source_conversion`.
    pub water_source_conversion: bool,
    /// `minecraft:lava_source_conversion`.
    pub lava_source_conversion: bool,
    /// The dimension's `fast_lava` environment attribute (the nether).
    pub fast_lava: bool,
    /// The dimension's `water_evaporates` environment attribute (the nether).
    pub water_evaporates: bool,
    /// `minecraft:tnt_explodes`.
    pub tnt_explodes: bool,
}

impl Default for Rules {
    /// Overworld with default game rules.
    fn default() -> Self {
        Self { water_source_conversion: true, lava_source_conversion: false, fast_lava: false, water_evaporates: false, tnt_explodes: true }
    }
}

/// Side effects the level applies or forwards to clients; behaviour never needs their result.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `Block.dropResources`: the block at `pos` (in `state`) drops its loot.
    Drop { pos: BlockPos, state: u16 },
    /// `levelEvent(id, pos, data)`: particles and sounds (1501 lava fizz, 1502 redstone torch
    /// burnout, 2001 block destroyed with data = state id, ...).
    LevelEvent { id: i32, pos: BlockPos, data: i32 },
    /// A sound event (`minecraft:block.lever.click`, ...) at the block's center.
    Sound { pos: BlockPos, sound: &'static str, volume: f32, pitch: f32 },
    /// A vibration game event (`minecraft:block_activate`, ...).
    GameEvent { pos: BlockPos, event: &'static str },
    /// `FallingBlockEntity.fall`: the block left `pos` as a falling entity (already removed).
    FallingBlock { pos: BlockPos, state: u16 },
    /// `TntBlock.prime`: spawn a primed TNT at `pos` (the block is removed).
    PrimedTnt { pos: BlockPos },
    /// `NoteBlock.triggerEvent`: play `instrument` at `note` (0..=24) with its particle.
    NoteBlock { pos: BlockPos, instrument: &'static str, note: i32 },
    /// A block event that ran and must reach clients (`ClientboundBlockEventPacket`).
    BlockEvent { pos: BlockPos, block: BlockId, a: i32, b: i32 },
}

pub trait Level {
    type Random: RandomSource;

    /// Block state at `pos`: void air outside the build height (`Level.getBlockState`).
    fn block(&self, pos: BlockPos) -> u16;

    /// The storage half of `LevelChunk.setBlockState`: writes `state`, maintains heightmaps,
    /// light and block entities, and returns the previous state, or `None` when nothing
    /// changed (same state, or air into an empty section) or `pos` cannot be written.
    /// Behaviour callbacks (`onPlace`, removal side effects) are the caller's.
    fn set_raw(&mut self, pos: BlockPos, state: u16, flags: u32) -> Option<u16>;

    /// `Level.isInValidBounds`: inside the build height and the horizontal world limit.
    fn in_bounds(&self, pos: BlockPos) -> bool;

    /// `Level.hasChunkAt`.
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }

    fn game_time(&self) -> i64;

    /// `Level.nextSubTickCount`: one counter for block and fluid ticks.
    fn next_sub_tick(&mut self) -> i64;

    fn block_ticks(&mut self) -> &mut LevelTicks<BlockId>;

    fn fluid_ticks(&mut self) -> &mut LevelTicks<FluidType>;

    /// The level random (`Level.random`).
    fn random(&mut self) -> &mut Self::Random;

    /// Per-level behaviour state: the update queue, block events, torch burnout history.
    fn data(&mut self) -> &mut LevelData;

    fn rules(&self) -> &Rules;

    /// `LevelReader.getRawBrightness`: max of block light and sky light minus `sky_darken`.
    fn raw_brightness(&self, _pos: BlockPos, _sky_darken: i32) -> i32 {
        15
    }

    fn effect(&mut self, effect: Effect);

    /// Entities of the kind intersecting the box (pressure plates: not spectators, not
    /// ignoring block triggers; detector rails: minecarts).
    fn count_entities(&self, _min: [f64; 3], _max: [f64; 3], _kind: EntityKind) -> usize {
        0
    }

    /// Called before each queued update runs (vanilla's neighbour-update debug listener).
    fn trace_update(&mut self, _update: UpdateTrace) {}
}

/// Which entities a block counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Any,
    Living,
    Minecart,
}

/// An update about to run: `neighborChanged` or `updateShape` of the block at `pos`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateTrace {
    Neighbor(BlockPos),
    Shape(BlockPos),
}

/// State vanilla keeps on the level object for block behaviour.
pub struct LevelData {
    pub updater: NeighborUpdater,
    pub block_events: BlockEvents,
    /// `RedstoneTorchBlock.RECENT_TOGGLES` for this level.
    pub torch_toggles: Vec<Toggle>,
    /// `Level.randValue`: the LCG choosing random-tick positions.
    pub rand_value: i32,
}

impl LevelData {
    pub fn new(max_chained_neighbor_updates: i32, rand_value: i32) -> Self {
        Self { updater: NeighborUpdater::new(max_chained_neighbor_updates), block_events: BlockEvents::default(), torch_toggles: Vec::new(), rand_value }
    }
}

/// `ScheduledTickAccess.scheduleTick(pos, block, delay, priority)`.
pub fn schedule_block_tick<L: Level + ?Sized>(level: &mut L, pos: BlockPos, block: BlockId, delay: i32, priority: TickPriority) {
    let tick = ScheduledTick { kind: block, pos, trigger: level.game_time() + delay as i64, priority, sub: level.next_sub_tick() };
    level.block_ticks().schedule(tick);
}

/// `ScheduledTickAccess.scheduleTick(pos, fluid, delay)`.
pub fn schedule_fluid_tick<L: Level + ?Sized>(level: &mut L, pos: BlockPos, fluid: FluidType, delay: i32) {
    let tick = ScheduledTick { kind: fluid, pos, trigger: level.game_time() + delay as i64, priority: TickPriority::Normal, sub: level.next_sub_tick() };
    level.fluid_ticks().schedule(tick);
}
