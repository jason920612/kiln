//! Lecterns (`LecternBlock`, `LecternBlockEntity`, `LecternMenu`): a written or writable book is put on
//! the stand with a click, read in a menu (the client shows the page, the buttons turn it, jump to one or
//! take the book back) and the lectern pulses a redstone signal for two ticks whenever the page turns; a
//! comparator reads how far into the book it is. A lectern that is broken drops its book.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::{BeKind, ContainerBe, page_count};
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Effect, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_item::ItemStack;

/// `LecternBlockEntity.hasBook`: the item has the contents of a written or writable book.
fn has_book(book: &ItemStack) -> bool {
    book.has(kiln_item::keys::WRITABLE_BOOK_CONTENT) || book.has(kiln_item::keys::WRITTEN_BOOK_CONTENT)
}

/// `LecternBlockEntity.getRedstoneSignal`: how far into the book the page is, 1 to 15.
pub(crate) fn analog(c: &ContainerBe) -> i32 {
    let pages = page_count(&c.items[0]);
    let fraction = if pages > 1 { c.page as f32 / (pages as f32 - 1.0) } else { 1.0 };
    (fraction * 14.0).floor() as i32 + i32::from(has_book(&c.items[0]))
}

/// `LecternBlock.useItemOn`: a book of the `lectern_books` tag goes on an empty stand. `None`: not for the lectern.
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> Option<bool> {
    if logic::block_class(s) != C::LecternBlock || state::get_bool(s, "has_book") {
        return None;
    }
    let held = p.in_hand(off_hand).clone();
    if held.is_empty() || !kiln_entity::mob::item_tag(held.item(), "minecraft:lectern_books") {
        return None;
    }
    // `placeBook`: one of the stack goes in (`consumeAndReturn`), at page 0.
    let mut one = held;
    one.set_count(1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    let c = level.blocks.containers.get_mut(pos).filter(|c| c.kind == BeKind::Lectern)?;
    c.items[0] = one;
    c.page = 0;
    c.mark_changed();
    kiln_blocks::behaviour::lectern::reset_book_state(level, pos, s, true);
    level.effect(Effect::Sound { pos, sound: "minecraft:item.book.put", volume: 1.0, pitch: 1.0 });
    Some(true)
}

/// What a menu operation did to the lectern at `pos`: a turned page pulses (`signalPageChange`), a book that
/// was taken resets the stand (`onBookItemRemove`: page 0, no book, unpowered).
pub(crate) fn after_menu(level: &mut RegionLevel, pos: BlockPos) {
    let Some(c) = level.blocks.containers.get_mut(pos).filter(|c| c.kind == BeKind::Lectern) else { return };
    let turned = std::mem::take(&mut c.page_turned);
    let gone = c.items[0].is_empty();
    if gone {
        c.page = 0;
    }
    let s = level.block(pos);
    if logic::block_class(s) != C::LecternBlock {
        return;
    }
    if gone && state::get_bool(s, "has_book") {
        kiln_blocks::behaviour::lectern::reset_book_state(level, pos, s, false);
    } else if turned {
        kiln_blocks::behaviour::lectern::signal_page_change(level, pos, s);
    }
}

/// `LecternBlockEntity.preRemoveSideEffects`: the book pops out above the stand, a little toward the way it faces.
pub(crate) fn removed(level: &mut RegionLevel, pos: BlockPos, old: u16, c: &ContainerBe) {
    if logic::block_class(old) != C::LecternBlock || !state::get_bool(old, "has_book") || c.items[0].is_empty() {
        return;
    }
    let step = state::get_dir(old, "facing").map_or([0, 0, 0], |d| d.step());
    let at = [pos.x as f64 + 0.5 + 0.25 * step[0] as f64, pos.y as f64 + 1.0, pos.z as f64 + 0.5 + 0.25 * step[2] as f64];
    let seed = crate::mobs::loot_seed(level.env.seed, level.env.game_time, 0, (pos.x as u64) << 32 ^ pos.z as u64 ^ (pos.y as u64) << 16);
    let mut spawn: Spawn = crate::mobs::drop_item(c.items[0].clone(), at, seed as u64);
    spawn.pos = at;
    if let Body::Item { pickup_delay, .. } = &mut spawn.body {
        *pickup_delay = 10;
    }
    level.out.spawns.push(spawn);
}
