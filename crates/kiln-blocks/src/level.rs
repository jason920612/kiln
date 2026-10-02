//! The world as block behaviour sees it.
//!
//! A [`Level`] stores block states and the per-level machinery vanilla keeps in `Level` /
//! `ServerLevel` (scheduled ticks, the neighbour-update queue, the level random, block
//! events). Everything that decides what happens (update order, shapes, fluids, redstone)
//! lives in this crate and runs against the trait, so the same code drives an in-memory test
//! level and a simulation region.

use crate::behaviour::piston::{MovingPiston, MovingPistons};
use crate::block_events::BlockEvents;
use crate::redstone::torch::Toggle;
use crate::fluid::FluidType;
use crate::pos::{BlockPos, Direction};
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
    /// The dimension's `infiniburn` block tag (fire never burns out on these).
    pub infiniburn: &'static str,
}

impl Default for Rules {
    /// Overworld with default game rules.
    fn default() -> Self {
        Self {
            water_source_conversion: true,
            lava_source_conversion: false,
            fast_lava: false,
            water_evaporates: false,
            tnt_explodes: true,
            infiniburn: "minecraft:infiniburn_overworld",
        }
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
    /// A sound the acting player's client plays itself (`level.playSound(player, ...)`):
    /// everyone else hears it.
    ActorSound { pos: BlockPos, sound: &'static str, volume: f32, pitch: f32 },
    /// A level event the acting player's client shows itself (`levelEvent(player, ...)`, such
    /// as 2001 for a block the player broke): everyone else sees it.
    ActorLevelEvent { id: i32, pos: BlockPos, data: i32 },
    /// A vibration game event (`minecraft:block_activate`, ...).
    GameEvent { pos: BlockPos, event: &'static str },
    /// A game event about the block `state` (`GameEvent.Context.of(entity, state)`): a
    /// `#dampens_vibrations` block (wool, carpets) makes no vibration.
    BlockGameEvent { pos: BlockPos, event: &'static str, state: u16 },
    /// `FallingBlockEntity.fall`: the block left `pos` as a falling entity (already removed).
    FallingBlock { pos: BlockPos, state: u16 },
    /// `TntBlock.prime`: spawn a primed TNT at `pos` (the block is removed).
    PrimedTnt { pos: BlockPos },
    /// `NoteBlock.triggerEvent`: play `instrument` at `note` (0..=24) with its particle.
    NoteBlock { pos: BlockPos, instrument: &'static str, note: i32 },
    /// A block event that ran and must reach clients (`ClientboundBlockEventPacket`).
    BlockEvent { pos: BlockPos, block: BlockId, a: i32, b: i32 },
    /// `PistonMovingBlockEntity.tick` before advancing: entities in the moving block's way
    /// (`moveCollidedEntities`) and stuck to honey or slime (`moveStuckEntities`) are
    /// carried to `progress`.
    PistonMove { pos: BlockPos, piston: MovingPiston, progress: f32 },
    /// `SnifferEggBlock.tick`: a baby sniffer hatches at the egg's center (the egg is gone).
    HatchSniffer { pos: BlockPos },
    /// `FrogspawnBlock.tick`: the tadpoles that hatch at `pos` (the spawn is gone): for each,
    /// the x and z offsets in the block and the yaw.
    HatchFrogspawn { pos: BlockPos, tadpoles: Vec<(f64, f64, i32)> },
}

pub trait Level {
    type Random: RandomSource;

    /// Block state at `pos`: void air outside the build height (`Level.getBlockState`).
    fn block(&self, pos: BlockPos) -> u16;

    /// The storage half of `LevelChunk.setBlockState`: writes `state`, maintains heightmaps,
    /// light and block entities, and returns the previous state, or `None` when nothing
    /// changed (same state, or air into an empty section) or `pos` cannot be written.
    /// Behaviour callbacks (`onPlace`, removal side effects) are the caller's. Moving-piston
    /// block entities are kept in [`LevelData::pistons`], not by the level.
    fn set_raw(&mut self, pos: BlockPos, state: u16, flags: u32) -> Option<u16>;

    /// `Level.isInValidBounds`: inside the build height and the horizontal world limit.
    fn in_bounds(&self, pos: BlockPos) -> bool;

    /// `Level.hasChunkAt`.
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }

    /// `Level.getMinY`: the bottom of the level.
    fn min_y(&self) -> i32 {
        -64
    }

    /// `BaseFireBlock.inPortalDimension`: fire lights nether portals here (the overworld and
    /// the nether).
    fn portals_light(&self) -> bool {
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

    /// The output signal stored in the comparator's block entity at `pos` (0 if none).
    fn comparator_output(&self, pos: BlockPos) -> i32;

    /// Stores a comparator's output in its block entity.
    fn set_comparator_output(&mut self, pos: BlockPos, value: i32);

    /// `getAnalogOutputSignal` of blocks whose output lives in a block entity or depends on
    /// entities (containers, lecterns, jukeboxes, sculk sensors, command blocks, detector
    /// rails with minecarts, ...), read from side `dir`.
    fn block_entity_analog(&self, _pos: BlockPos, _state: u16, _dir: Direction) -> i32 {
        0
    }

    /// Players looking into the container block entity at `pos` (`ContainerOpenersCounter`):
    /// a trapped chest's signal.
    fn container_openers(&self, _pos: BlockPos) -> i32 {
        0
    }

    /// A scheduled tick of a block whose behaviour lives in its block entity: chests, barrels
    /// and ender chests recheck their openers, dispensers and droppers dispense.
    fn block_entity_tick(&mut self, _pos: BlockPos, _state: u16) {}

    /// The analog output of the single item frame at `pos` facing `facing`, if exactly one.
    fn item_frame_analog(&self, _pos: BlockPos, _facing: Direction) -> Option<i32> {
        None
    }

    /// Entities of the kind intersecting the box (pressure plates: not spectators, not
    /// ignoring block triggers; detector rails: minecarts).
    fn count_entities(&self, _min: [f64; 3], _max: [f64; 3], _kind: EntityKind) -> usize {
        0
    }

    /// Called before each queued update runs (vanilla's neighbour-update debug listener).
    fn trace_update(&mut self, _update: UpdateTrace) {}

    /// `Level.getHeight`: the build height.
    fn height(&self) -> i32 {
        384
    }

    /// The weather and rules precipitation reads (clear by default).
    fn weather(&self) -> crate::weather::Weather {
        crate::weather::Weather::default()
    }

    /// The `MOTION_BLOCKING` heightmap: the y above the column's topmost motion-blocking block.
    fn motion_blocking_height(&self, _x: i32, _z: i32) -> i32 {
        self.min_y()
    }

    /// The climate of the biome at `biome_pos` (`Level.getBiome`), its temperature read at
    /// `pos`; `None` without biome data (no precipitation effects).
    fn climate(&self, _biome_pos: BlockPos, _pos: BlockPos) -> Option<crate::weather::Climate> {
        None
    }

    /// `getBrightness(LightLayer.BLOCK, pos)`.
    fn block_light(&self, _pos: BlockPos) -> i32 {
        0
    }

    /// `Level.isRainingAt`.
    fn is_raining_at(&self, _pos: BlockPos) -> bool {
        false
    }

    /// The `minecraft:gameplay/creaking_active` environment attribute at `pos` (the
    /// overworld's night): what wakes a creaking heart.
    fn creaking_active(&self, _pos: BlockPos) -> bool {
        false
    }

    /// Makes the level random's next draws for work at `pos` independent of what else the
    /// level did (a simulation split into regions reseeds it from the position and time;
    /// the vanilla level keeps its one random, so the default does nothing).
    fn reseed_random(&mut self, _pos: BlockPos) {}

    /// `ServerLevel.canSpreadFireAround`: a non-spectator player closer than
    /// `fire_spread_radius_around_player` (always with -1).
    fn can_spread_fire_around(&self, _pos: BlockPos) -> bool {
        true
    }

    /// `Difficulty.getId` (0 peaceful .. 3 hard).
    fn difficulty(&self) -> i32 {
        2
    }

    /// The `minecraft:gameplay/increased_fire_burnout` environment attribute at `pos` (wet
    /// biomes).
    fn increased_fire_burnout(&self, _pos: BlockPos) -> bool {
        false
    }
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
    /// Moving-piston block entities and their ticker order.
    pub pistons: MovingPistons,
    /// `ServerLevel.handlingTick`: true from the block ticks through the block events (set
    /// by [`crate::tick::run_block_ticks`], cleared by
    /// [`crate::block_events::run_block_events`]).
    pub handling_tick: bool,
}

impl LevelData {
    pub fn new(max_chained_neighbor_updates: i32, rand_value: i32) -> Self {
        Self {
            updater: NeighborUpdater::new(max_chained_neighbor_updates),
            block_events: BlockEvents::default(),
            torch_toggles: Vec::new(),
            rand_value,
            pistons: MovingPistons::default(),
            handling_tick: false,
        }
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
