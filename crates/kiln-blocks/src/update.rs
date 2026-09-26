//! Setting blocks and propagating updates: `Level.setBlock`, `LevelChunk.setBlockState`'s
//! behaviour callbacks, `CollectingNeighborUpdater`, shape updates (`updateShape`,
//! `updateNeighbourShapes`, `Block.updateOrDestroy`), `destroyBlock` and `removeBlock`.
//!
//! All neighbour and shape updates go through one queue per level: the first update runs
//! immediately; updates it triggers are collected per layer and run depth-first in the order
//! they were added, and at most `max_chained_neighbor_updates` run per chain.

use crate::behaviour;
use crate::fluid;
use crate::level::{Effect, Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::state::{BlockId, same_block};
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;

/// `NeighborUpdater.UPDATE_ORDER`.
pub const UPDATE_ORDER: [Direction; 6] =
    [Direction::West, Direction::East, Direction::Down, Direction::Up, Direction::North, Direction::South];

/// `BlockBehaviour.UPDATE_SHAPE_ORDER`.
pub const UPDATE_SHAPE_ORDER: [Direction; 6] =
    [Direction::West, Direction::East, Direction::North, Direction::South, Direction::Down, Direction::Up];

#[derive(Clone, Copy, Debug)]
enum Update {
    Shape { dir: Direction, neighbor_state: u16, pos: BlockPos, neighbor_pos: BlockPos, flags: u32, limit: i32 },
    Simple { pos: BlockPos, source: BlockId },
    Full { state: u16, pos: BlockPos, source: BlockId, moved_by_piston: bool },
    Multi { source_pos: BlockPos, source: BlockId, skip: Option<Direction>, idx: u8 },
}

enum Step {
    Shape { dir: Direction, neighbor_state: u16, pos: BlockPos, neighbor_pos: BlockPos, flags: u32, limit: i32 },
    Neighbor { state: Option<u16>, pos: BlockPos, source: BlockId, moved_by_piston: bool },
}

impl Update {
    /// `NeighborUpdates.runNext`: the next update to execute and whether more remain.
    fn next(&mut self) -> (Step, bool) {
        match *self {
            Update::Shape { dir, neighbor_state, pos, neighbor_pos, flags, limit } => {
                (Step::Shape { dir, neighbor_state, pos, neighbor_pos, flags, limit }, false)
            }
            Update::Simple { pos, source } => (Step::Neighbor { state: None, pos, source, moved_by_piston: false }, false),
            Update::Full { state, pos, source, moved_by_piston } => {
                (Step::Neighbor { state: Some(state), pos, source, moved_by_piston }, false)
            }
            Update::Multi { source_pos, source, skip, ref mut idx } => {
                let dir = UPDATE_ORDER[*idx as usize];
                *idx += 1;
                if (*idx as usize) < 6 && Some(UPDATE_ORDER[*idx as usize]) == skip {
                    *idx += 1;
                }
                let step = Step::Neighbor { state: None, pos: source_pos.relative(dir), source, moved_by_piston: false };
                (step, (*idx as usize) < 6)
            }
        }
    }
}

/// The per-level update queue (`CollectingNeighborUpdater`).
pub struct NeighborUpdater {
    max_chained: i32,
    stack: Vec<Update>,
    added: Vec<Update>,
    count: i32,
    skipped: u64,
}

impl NeighborUpdater {
    /// `max_chained` is the server's `max-chained-neighbor-updates` (vanilla default
    /// 1000000; negative for no limit).
    pub fn new(max_chained: i32) -> Self {
        Self { max_chained, stack: Vec::new(), added: Vec::new(), count: 0, skipped: 0 }
    }

    /// Updates dropped because a chain hit the limit, since the level was created.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }
}

impl Default for NeighborUpdater {
    fn default() -> Self {
        Self::new(1_000_000)
    }
}

fn add_and_run<L: Level>(level: &mut L, update: Update) {
    let u = &mut level.data().updater;
    let chained = u.count > 0;
    let over = u.max_chained >= 0 && u.count >= u.max_chained;
    u.count += 1;
    if over {
        u.skipped += 1;
    } else if chained {
        u.added.push(update);
    } else {
        u.stack.push(update);
    }
    if !chained {
        run_updates(level);
    }
}

fn run_updates<L: Level>(level: &mut L) {
    loop {
        let u = &mut level.data().updater;
        if u.stack.is_empty() && u.added.is_empty() {
            break;
        }
        // The first update added in this layer ends on top.
        while let Some(x) = u.added.pop() {
            u.stack.push(x);
        }
        loop {
            let (step, more) = level.data().updater.stack.last_mut().expect("update").next();
            execute(level, step);
            let u = &mut level.data().updater;
            if !more {
                u.stack.pop();
                break;
            }
            if !u.added.is_empty() {
                break;
            }
        }
    }
    let u = &mut level.data().updater;
    u.stack.clear();
    u.added.clear();
    u.count = 0;
}

fn execute<L: Level>(level: &mut L, step: Step) {
    match step {
        Step::Shape { dir, neighbor_state, pos, neighbor_pos, flags, limit } => {
            execute_shape_update(level, dir, pos, neighbor_pos, neighbor_state, flags, limit)
        }
        Step::Neighbor { state, pos, source, moved_by_piston } => {
            let state = state.unwrap_or_else(|| level.block(pos));
            behaviour::neighbor_changed(level, state, pos, source, moved_by_piston);
        }
    }
}

/// `NeighborUpdater.executeShapeUpdate`.
fn execute_shape_update<L: Level>(
    level: &mut L,
    dir: Direction,
    pos: BlockPos,
    neighbor_pos: BlockPos,
    neighbor_state: u16,
    flags: u32,
    limit: i32,
) {
    let state = level.block(pos);
    if flags & flags::SKIP_SHAPE_UPDATE_ON_WIRE != 0 && crate::state::is(state, d::REDSTONE_WIRE) {
        return;
    }
    let new = behaviour::update_shape(level, state, pos, dir, neighbor_pos, neighbor_state);
    update_or_destroy(level, state, new, pos, flags, limit);
}

/// `ServerLevel.updateNeighborsAt(pos, block)`: `neighborChanged` on all six neighbours in
/// `UPDATE_ORDER`.
pub fn update_neighbors_at<L: Level>(level: &mut L, pos: BlockPos, source: BlockId) {
    update_neighbors_at_except(level, pos, source, None);
}

/// `updateNeighborsAtExceptFromFacing`.
pub fn update_neighbors_at_except<L: Level>(level: &mut L, pos: BlockPos, source: BlockId, skip: Option<Direction>) {
    let idx = if Some(UPDATE_ORDER[0]) == skip { 1 } else { 0 };
    add_and_run(level, Update::Multi { source_pos: pos, source, skip, idx });
}

/// `Level.neighborChanged(pos, block, orientation)`: one `neighborChanged` at `pos`.
pub fn neighbor_changed_at<L: Level>(level: &mut L, pos: BlockPos, source: BlockId) {
    add_and_run(level, Update::Simple { pos, source });
}

/// `Level.neighborChanged(state, pos, block, orientation, movedByPiston)`: with a known state.
pub fn neighbor_changed_with<L: Level>(level: &mut L, state: u16, pos: BlockPos, source: BlockId, moved_by_piston: bool) {
    add_and_run(level, Update::Full { state, pos, source, moved_by_piston });
}

/// `Level.neighborShapeChanged`: queues `updateShape` of the block at `pos` because the block
/// at `neighbor_pos` (toward `dir`) became `neighbor_state`.
pub fn neighbor_shape_changed<L: Level>(
    level: &mut L,
    dir: Direction,
    pos: BlockPos,
    neighbor_pos: BlockPos,
    neighbor_state: u16,
    flags: u32,
    limit: i32,
) {
    add_and_run(level, Update::Shape { dir, neighbor_state, pos, neighbor_pos, flags, limit });
}

/// `BlockState.updateNeighbourShapes`.
pub fn update_neighbour_shapes<L: Level>(level: &mut L, state: u16, pos: BlockPos, flags: u32, limit: i32) {
    for dir in UPDATE_SHAPE_ORDER {
        neighbor_shape_changed(level, dir.opposite(), pos.relative(dir), pos, state, flags, limit);
    }
}

/// `Block.updateOrDestroy`.
pub fn update_or_destroy<L: Level>(level: &mut L, old: u16, new: u16, pos: BlockPos, flags: u32, limit: i32) {
    if new == old {
        return;
    }
    if is_air(new) {
        destroy_block(level, pos, flags & flags::SUPPRESS_DROPS == 0, limit);
    } else {
        set_block_limit(level, pos, new, flags & !flags::SUPPRESS_DROPS, limit);
    }
}

/// `Block.updateFromNeighbourShapes`: the state after asking all six neighbours.
pub fn update_from_neighbour_shapes<L: Level>(level: &mut L, state: u16, pos: BlockPos) -> u16 {
    let mut s = state;
    for dir in UPDATE_SHAPE_ORDER {
        let n = pos.relative(dir);
        let ns = level.block(n);
        s = behaviour::update_shape(level, s, pos, dir, n, ns);
    }
    s
}

/// `Level.setBlock(pos, state, flags)`.
pub fn set_block<L: Level>(level: &mut L, pos: BlockPos, state: u16, flags: u32) -> bool {
    set_block_limit(level, pos, state, flags, flags::LIMIT)
}

/// `Level.setBlock(pos, state, flags, recursionLeft)`.
pub fn set_block_limit<L: Level>(level: &mut L, pos: BlockPos, state: u16, flags: u32, limit: i32) -> bool {
    if !level.in_bounds(pos) {
        return false;
    }
    let Some(old) = set_block_state(level, pos, state, flags) else { return false };
    if level.block(pos) == state {
        if flags & flags::NEIGHBORS != 0 {
            update_neighbors_at(level, pos, BlockId::of(old));
            if logic::has_analog_output(state) {
                update_neighbour_for_output_signal(level, pos, BlockId::of(state));
            }
        }
        if flags & flags::KNOWN_SHAPE == 0 && limit > 0 {
            let f = flags & !(flags::NEIGHBORS | flags::SUPPRESS_DROPS);
            behaviour::update_indirect_neighbour_shapes(level, old, pos, f, limit - 1);
            update_neighbour_shapes(level, state, pos, f, limit - 1);
            behaviour::update_indirect_neighbour_shapes(level, state, pos, f, limit - 1);
        }
    }
    true
}

/// `LevelChunk.setBlockState`: the storage write plus `affectNeighborsAfterRemoval` and
/// `onPlace`. Returns the old state, or `None` if nothing was set or the removal side effects
/// replaced the new block.
fn set_block_state<L: Level>(level: &mut L, pos: BlockPos, state: u16, flags: u32) -> Option<u16> {
    let old = level.set_raw(pos, state, flags)?;
    let changed = !same_block(old, state);
    let moved = flags & flags::MOVE_BY_PISTON != 0;
    if (changed || logic::is_instance(state, BlockClass::BaseRailBlock)) && (flags & flags::NEIGHBORS != 0 || moved) {
        behaviour::affect_neighbors_after_removal(level, old, pos, moved);
    }
    if !same_block(level.block(pos), state) {
        return None;
    }
    if flags & flags::SKIP_ON_PLACE == 0 {
        behaviour::on_place(level, state, pos, old, moved);
    }
    Some(old)
}

/// `Level.setBlockAndUpdate`.
pub fn set_block_and_update<L: Level>(level: &mut L, pos: BlockPos, state: u16) -> bool {
    set_block(level, pos, state, flags::ALL)
}

/// `Level.removeBlock`: replaces the block with its fluid (or air).
pub fn remove_block<L: Level>(level: &mut L, pos: BlockPos, moved_by_piston: bool) -> bool {
    let legacy = fluid::legacy_block(logic::fluid(level.block(pos)));
    set_block(level, pos, legacy, flags::ALL | if moved_by_piston { flags::MOVE_BY_PISTON } else { 0 })
}

/// `Level.destroyBlock`: break effects, optional drops, then the block's fluid (or air).
pub fn destroy_block<L: Level>(level: &mut L, pos: BlockPos, drop: bool, limit: i32) -> bool {
    let state = level.block(pos);
    if is_air(state) {
        return false;
    }
    level.effect(Effect::LevelEvent { id: 2001, pos, data: state as i32 });
    if drop {
        level.effect(Effect::Drop { pos, state });
    }
    let legacy = fluid::legacy_block(logic::fluid(state));
    let done = set_block_limit(level, pos, legacy, flags::ALL, limit);
    if done {
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_destroy" });
    }
    done
}

/// `Block.dropResources` followed by `Level.removeBlock` (blocks that pop off on a
/// neighbour change).
pub fn drop_and_remove<L: Level>(level: &mut L, pos: BlockPos, state: u16) {
    level.effect(Effect::Drop { pos, state });
    remove_block(level, pos, false);
}

/// `Level.updateNeighbourForOutputSignal`: comparators beside `pos`, or behind a conductor
/// beside it, re-read their input.
pub fn update_neighbour_for_output_signal<L: Level>(level: &mut L, pos: BlockPos, source: BlockId) {
    for dir in Direction::HORIZONTAL {
        let mut n = pos.relative(dir);
        if !level.is_loaded(n) {
            continue;
        }
        let mut s = level.block(n);
        if crate::state::is(s, d::COMPARATOR) {
            neighbor_changed_with(level, s, n, source, false);
        } else if logic::is_redstone_conductor(s) {
            n = n.relative(dir);
            s = level.block(n);
            if crate::state::is(s, d::COMPARATOR) {
                neighbor_changed_with(level, s, n, source, false);
            }
        }
    }
}
