//! Vanilla 26.3 block behaviour against an abstract [`Level`]: `setBlock` with update
//! flags, neighbour and shape updates in vanilla order, pop-offs, connection shapes,
//! scheduled block and fluid ticks, random-tick selection, block events, water and lava,
//! redstone (wire, torches, repeaters, comparators, levers, buttons, lamps, doors, rails)
//! and pistons.
//!
//! State ids and per-state facts come from `kiln-data`; behaviour dispatches on each block's
//! vanilla class. [`TestLevel`] is an in-memory level for tests and the differential harness
//! (`tools/blocks_diff.py`).

pub mod behaviour;
pub mod block_events;
pub mod commands;
pub mod fluid;
pub mod interact;
mod java_map;
pub mod level;
pub mod placement;
pub mod pos;
pub mod redstone;
pub mod state;
pub mod tags;
pub mod test_level;
pub mod tick;
pub mod ticks;
pub mod update;
pub mod weather;
pub mod fire;

pub use behaviour::piston::{MovingPiston, MovingPistons, tick_moving_pistons};
pub use fluid::FluidType;
pub use level::{Effect, EntityKind, Level, LevelData, Rules, flags, schedule_block_tick, schedule_fluid_tick};
pub use pos::{Axis, BlockPos, Direction};
pub use state::BlockId;
pub use test_level::TestLevel;
pub use ticks::{ChunkTicks, LevelTicks, SavedTick, ScheduledTick, TickPriority};
pub use update::{destroy_block, remove_block, set_block, set_block_and_update, set_block_limit};
