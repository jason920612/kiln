//! Lecterns (`LecternBlock`): a book on a stand. Turning a page makes the lectern a power source for
//! two ticks (`powered`, 15 to the block it stands on and to every neighbour); taking the book resets
//! the state. The book, its page and the menu are the block entity's (the level's).

use crate::BlockId;
use crate::level::{Effect, Level, schedule_block_tick};
use crate::pos::BlockPos;
use crate::state;
use crate::ticks::TickPriority;
use crate::update::{set_block_and_update, update_neighbors_at};

/// `LecternBlock.updateBelow`: the block under the lectern follows its power.
fn update_below<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    update_neighbors_at(level, pos.below(), BlockId::of(s));
}

/// `LecternBlock.changePowered`.
fn change_powered<L: Level>(level: &mut L, pos: BlockPos, s: u16, powered: bool) {
    set_block_and_update(level, pos, state::set_bool(s, "powered", powered));
    update_below(level, pos, s);
}

/// `LecternBlock.tick`: the pulse ends.
pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    change_powered(level, pos, s, false);
}

/// `LecternBlock.signalPageChange`: power for two ticks and the page turning sound.
pub fn signal_page_change<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    change_powered(level, pos, s, true);
    schedule_block_tick(level, pos, BlockId::of(s), 2, TickPriority::Normal);
    level.effect(Effect::LevelEvent { id: 1043, pos, data: 0 });
}

/// `LecternBlock.resetBookState`: unpowered, with or without a book.
pub fn reset_book_state<L: Level>(level: &mut L, pos: BlockPos, s: u16, has_book: bool) {
    let new = state::set_bool(state::set_bool(s, "powered", false), "has_book", has_book);
    set_block_and_update(level, pos, new);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: new });
    update_below(level, pos, s);
}

/// `LecternBlock.affectNeighborsAfterRemoval`: a powered lectern tells the block under it.
pub fn removed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "powered") {
        update_below(level, pos, s);
    }
}
