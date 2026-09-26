//! Redstone torches (`RedstoneTorchBlock`, `RedstoneWallTorchBlock`), including burnout.

use super::has_signal;
use crate::level::{Effect, Level, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{set_block_and_update, update_neighbors_at};
use kiln_data::block_logic::{self as logic, BlockClass};

/// A torch toggling `when` (`RedstoneTorchBlock.Toggle`); kept per level.
#[derive(Clone, Copy, Debug)]
pub struct Toggle {
    pub pos: BlockPos,
    pub when: i64,
}

fn notify_neighbors<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let block = BlockId::of(s);
    for dir in Direction::ALL {
        update_neighbors_at(level, pos.relative(dir), block);
    }
}

/// Whether the block the torch stands on (or hangs from) is powered.
fn powered<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> bool {
    if logic::block_class(s) == BlockClass::RedstoneWallTorchBlock {
        let back = state::get_dir(s, "facing").unwrap_or(Direction::North).opposite();
        has_signal(level, pos.relative(back), back)
    } else {
        has_signal(level, pos.below(), Direction::Down)
    }
}

pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    notify_neighbors(level, pos, s);
}

pub fn affect_neighbors_after_removal<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if !moved_by_piston {
        notify_neighbors(level, pos, s);
    }
}

pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let block = BlockId::of(s);
    if state::get_bool(s, "lit") == powered(level, pos, s) && !level.block_ticks().will_tick_this_tick(pos, block) {
        schedule_block_tick(level, pos, block, 2, TickPriority::Normal);
    }
}

/// `isToggledTooFrequently`: 8 toggles of this torch within the kept window burn it out.
fn toggled_too_often<L: Level>(level: &mut L, pos: BlockPos, add: bool) -> bool {
    let now = level.game_time();
    let toggles = &mut level.data().torch_toggles;
    if add {
        toggles.push(Toggle { pos, when: now });
    }
    toggles.iter().filter(|t| t.pos == pos).count() >= 8
}

pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let signal = powered(level, pos, s);
    let now = level.game_time();
    let toggles = &mut level.data().torch_toggles;
    while toggles.first().is_some_and(|t| now - t.when > 60) {
        toggles.remove(0);
    }
    if state::get_bool(s, "lit") {
        if signal {
            set_block_and_update(level, pos, state::set_bool(s, "lit", false));
            if toggled_too_often(level, pos, true) {
                level.effect(Effect::LevelEvent { id: 1502, pos, data: 0 });
                let block = BlockId::of(level.block(pos));
                schedule_block_tick(level, pos, block, 160, TickPriority::Normal);
            }
        }
    } else if !signal && !toggled_too_often(level, pos, false) {
        set_block_and_update(level, pos, state::set_bool(s, "lit", true));
    }
}
