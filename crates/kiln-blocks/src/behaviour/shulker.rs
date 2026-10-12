//! The lids of shulker boxes (`ShulkerBoxBlockEntity`'s animation): a box that a player opens
//! (block event 1 with the number of openers) lifts its lid over ten ticks, pushing what is in
//! its way ([`Effect::ShulkerLid`]), and lowers it again when the last opener has gone.
//!
//! Only boxes whose lid is not at rest are kept (in [`MovingPistons::lids`]): a closed lid ticks
//! to nothing.

use crate::level::{Effect, Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::update::{update_neighbors_at, update_neighbour_shapes};
use kiln_data::block_logic::BlockClass;

/// `ShulkerBoxBlockEntity.AnimationStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LidStatus {
    Closed,
    Opening,
    Opened,
    Closing,
}

/// The animation fields of a shulker box's block entity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lid {
    pub status: LidStatus,
    pub progress: f32,
    pub progress_old: f32,
}

impl Lid {
    pub const REST: Lid = Lid { status: LidStatus::Closed, progress: 0.0, progress_old: 0.0 };

    /// `getProgress(1.0f)`: `Mth.lerp(1, progressOld, progress)`.
    pub fn shown(&self) -> f32 {
        self.progress_old + 1.0f32 * (self.progress - self.progress_old)
    }
}

/// `ShulkerBoxBlockEntity.triggerEvent`: event 1 sets the number of openers; none starts the
/// lid closing, the first starts it opening. True for event 1 (it reaches the clients).
pub fn trigger_event<L: Level>(level: &mut L, pos: BlockPos, a: i32, b: i32) -> bool {
    if a != 1 {
        return false;
    }
    let lid = level.data().pistons.lids.entry(pos).or_insert(Lid::REST);
    if b == 0 {
        lid.status = LidStatus::Closing;
    }
    if b == 1 {
        lid.status = LidStatus::Opening;
    }
    true
}

/// `Level.tickBlockEntities` for the shulker boxes with a lid on the move: each in position
/// order, if `can_tick` accepts its position (`ShulkerBoxBlockEntity.tick` -> `updateAnimation`).
pub fn tick_lids<L: Level>(level: &mut L, mut can_tick: impl FnMut(BlockPos) -> bool) {
    let positions: Vec<BlockPos> = level.data().pistons.lids.keys().copied().collect();
    for pos in positions {
        if !can_tick(pos) {
            continue;
        }
        let s = level.block(pos);
        if !is_shulker_box(s) {
            level.data().pistons.lids.remove(&pos);
            continue;
        }
        update_animation(level, pos, s);
        // A lid at rest is not kept.
        if level.data().pistons.lids.get(&pos).is_some_and(|l| l.status == LidStatus::Closed && l.progress == 0.0 && l.progress_old == 0.0) {
            level.data().pistons.lids.remove(&pos);
        }
    }
}

/// `ShulkerBoxBlockEntity.updateAnimation`.
fn update_animation<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let Some(lid) = level.data().pistons.lids.get_mut(&pos) else { return };
    lid.progress_old = lid.progress;
    let old = lid.progress_old;
    match lid.status {
        LidStatus::Closed => lid.progress = 0.0,
        LidStatus::Opening => {
            lid.progress += 0.1;
            if old == 0.0 {
                do_neighbor_updates(level, pos, s);
            }
            if let Some(lid) = level.data().pistons.lids.get_mut(&pos)
                && lid.progress >= 1.0
            {
                lid.status = LidStatus::Opened;
                lid.progress = 1.0;
                do_neighbor_updates(level, pos, s);
            }
            // `moveCollidedEntities` (the facing is the block's).
            if let Some(lid) = level.data().pistons.lids.get(&pos).copied() {
                let facing = state::get_dir(s, "facing").unwrap_or(Direction::Up);
                level.effect(Effect::ShulkerLid { pos, facing, progress_old: lid.progress_old, progress: lid.progress });
            }
        }
        LidStatus::Closing => {
            lid.progress -= 0.1;
            if old == 1.0 {
                do_neighbor_updates(level, pos, s);
            }
            if let Some(lid) = level.data().pistons.lids.get_mut(&pos)
                && lid.progress <= 0.0
            {
                lid.status = LidStatus::Closed;
                lid.progress = 0.0;
                do_neighbor_updates(level, pos, s);
            }
        }
        LidStatus::Opened => lid.progress = 1.0,
    }
}

/// `ShulkerBoxBlockEntity.doNeighborUpdates`.
fn do_neighbor_updates<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    update_neighbour_shapes(level, s, pos, flags::ALL, flags::LIMIT);
    update_neighbors_at(level, pos, BlockId::of(s));
}

/// Whether `s` is a shulker box.
pub fn is_shulker_box(s: u16) -> bool {
    kiln_data::block_logic::block_class(s) == BlockClass::ShulkerBoxBlock
}

/// `ShulkerBoxBlock.getBlockSupportShape`: a box whose lid is not closed (`!isClosed()`, in any phase of the animation) only
/// has `SHAPES_OPEN_SUPPORT[facing.opposite]`, a one pixel wall on its base. The side of that wall, when `s` at `pos` is such a box.
pub fn open_base<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> Option<Direction> {
    if !is_shulker_box(s) || level.shulker_lid(pos).is_none_or(|l| l.status == LidStatus::Closed) {
        return None;
    }
    Some(state::get_dir(s, "facing").unwrap_or(Direction::Up).opposite())
}
