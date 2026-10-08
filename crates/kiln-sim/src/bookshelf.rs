//! Chiseled bookshelves (`ChiseledBookShelfBlock`, `ChiseledBookShelfBlockEntity`): six slots in
//! two rows of three on the front, each holding one book (`#bookshelf_books`). The slot a click
//! hits (`SelectableSlotContainer.getHitSlot`: thirds of the front across, halves up and down) is
//! the one a book goes into or comes out of; `slot_N_occupied` follows the books, and a
//! comparator reads the slot touched last (`last_interacted_slot + 1`).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::{BeKind, ContainerBe};
use crate::entities::Spawn;
use kiln_blocks::{BlockPos, Direction, Effect, Level, state};
use kiln_item::ItemStack;

/// `SelectableSlotContainer.getSection(x, count)`.
fn section(x: f32, count: i32) -> i32 {
    let a = x * 16.0;
    let b = 16.0 / count as f32;
    ((a / b).floor() as i32).clamp(0, count - 1)
}

/// `SelectableSlotContainer.getHitSlot` for a click on `face` at `cursor` (relative to the block).
fn hit_slot(s: u16, face: Direction, cursor: [f32; 3]) -> Option<usize> {
    if state::get_dir(s, "facing")? != face {
        return None;
    }
    let (x, y, z) = (cursor[0] as f64, cursor[1] as f64, cursor[2] as f64);
    let (u, v) = match face {
        Direction::North => ((1.0 - x) as f32, y as f32),
        Direction::South => (x as f32, y as f32),
        Direction::West => (z as f32, y as f32),
        Direction::East => ((1.0 - z) as f32, y as f32),
        _ => return None,
    };
    let row = section(1.0 - v, 2);
    let column = section(u, 3);
    Some((column + row * 3) as usize)
}

fn occupied(s: u16, slot: usize) -> bool {
    state::get_bool(s, &format!("slot_{slot}_occupied"))
}

/// Whether the item is a book that can stand on the shelf (`#minecraft:bookshelf_books`).
fn is_book(stack: &ItemStack) -> bool {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:item")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == "minecraft:bookshelf_books"))
        .is_some_and(|(_, ids)| ids.contains(&stack.item()))
}

/// `ChiseledBookShelfBlockEntity.updateState(slot)`: the shelf remembers the slot, its block
/// state shows the books, and the change is a game event.
fn update_state(level: &mut RegionLevel, pos: BlockPos, slot: usize) {
    let s = level.block(pos);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    c.last_slot = slot as i32;
    let mut now = s;
    for i in 0..6 {
        now = state::set_bool(now, &format!("slot_{i}_occupied"), !c.items[i].is_empty());
    }
    c.mark_changed();
    kiln_blocks::set_block_and_update(level, pos, now);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: now });
    crate::container::hopper::changed(level, pos);
}

fn is_enchanted(stack: &ItemStack) -> bool {
    stack.item_name() == "minecraft:enchanted_book"
}

/// `ChiseledBookShelfBlock.useItemOn`: a book goes into the free slot that was clicked.
/// `Some(false)`: for the empty hand (not a book, or the slot is taken).
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, face: Direction, cursor: [f32; 3], off_hand: bool, stack: &ItemStack) -> Option<bool> {
    if level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::ChiseledBookshelf) {
        return None;
    }
    if !is_book(stack) {
        return Some(false);
    }
    let Some(slot) = hit_slot(s, face, cursor) else { return None };
    if occupied(s, slot) {
        return Some(false);
    }
    // `addBook`.
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, stack.item()), 1);
    let mut one = stack.clone();
    one.set_count(1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.items[slot] = one;
    }
    update_state(level, pos, slot);
    let sound = if is_enchanted(stack) { "minecraft:block.chiseled_bookshelf.insert.enchanted" } else { "minecraft:block.chiseled_bookshelf.insert" };
    level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
    Some(true)
}

/// `ChiseledBookShelfBlock.useWithoutItem`: the clicked book comes out into the inventory (or
/// onto the ground).
pub(crate) fn use_without_item(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, face: Direction, cursor: [f32; 3], spawns: &mut Vec<Spawn>) -> bool {
    if level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::ChiseledBookshelf) {
        return false;
    }
    let Some(slot) = hit_slot(s, face, cursor) else { return false };
    if !occupied(s, slot) {
        return true;
    }
    // `removeBook`: `removeItem(slot, 1)`.
    let Some(c) = level.blocks.containers.get_mut(pos) else { return false };
    let mut book = std::mem::replace(&mut c.items[slot], ItemStack::empty());
    if !book.is_empty() {
        update_state(level, pos, slot);
    }
    let sound = if is_enchanted(&book) { "minecraft:block.chiseled_bookshelf.pickup.enchanted" } else { "minecraft:block.chiseled_bookshelf.pickup" };
    level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
    p.add_to_inventory(&mut book);
    if !book.is_empty() {
        spawns.push(p.throw(book));
    }
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    true
}

/// The comparator reading of a shelf (`getAnalogOutputSignal`: the slot touched last, plus one).
pub(crate) fn analog(c: &ContainerBe) -> i32 {
    c.last_slot + 1
}
