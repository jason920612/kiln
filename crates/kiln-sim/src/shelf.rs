//! Wooden shelves (`ShelfBlock`, `ShelfBlockEntity`): three slots on the front, each holding a whole stack. A click on a slot swaps the
//! stack in the player's hand with the one there (an empty hand takes it, an empty slot takes what is held). On a powered shelf a
//! click swaps the player's hotbar with the whole row of powered shelves it is chained to (`SideChainPartBlock`): the shelf's own
//! three slots with hotbar slots 6 to 8, the next one's with 3 to 5, and so on. The chain itself is the block's
//! (`kiln_blocks::behaviour::shelf`).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::BeKind;
use kiln_blocks::behaviour::shelf as chain;
use kiln_blocks::{BlockPos, Direction, Effect, Level, state};
use kiln_inventory::Container;
use kiln_item::ItemStack;

fn sound(level: &mut RegionLevel, pos: BlockPos, sound: &'static str) {
    level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
}

/// `ShelfBlockEntity.setChanged(event)`: the chunk keeps the items, clients are sent them (`sendBlockUpdated`), the change is a game event
/// (none for `""`).
fn changed(level: &mut RegionLevel, pos: BlockPos, event: &'static str) {
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
    }
    crate::container::open::sync_chunk_copy(level, pos);
    level.out.changed.push([pos.x, pos.y, pos.z]);
    if !event.is_empty() {
        let s = level.block(pos);
        level.effect(Effect::BlockGameEvent { pos, event, state: s });
    }
}

/// `ShelfBlockEntity.swapItemNoUpdate`: the stack in `slot` comes out, `stack` goes in (limited to the slot's maximum).
fn swap_item_no_update(level: &mut RegionLevel, pos: BlockPos, slot: usize, mut stack: ItemStack) -> Option<ItemStack> {
    let c = level.blocks.containers.get_mut(pos).filter(|c| c.kind == BeKind::Shelf)?;
    let old = c.remove_item_no_update(slot);
    let max = c.max_stack_size_for(&stack);
    stack.limit_size(max);
    c.items[slot] = stack;
    Some(old)
}

/// `ShelfBlock.swapSingleItem`: whether something came out of the shelf.
fn swap_single_item(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, slot: usize, held: &ItemStack) -> Option<bool> {
    let old = swap_item_no_update(level, pos, slot, held.clone())?;
    // (A creative player keeps what he puts in.)
    let replacement = if p.infinite_materials() && old.is_empty() { held.clone() } else { old.clone() };
    let selected = p.inv.selected;
    *p.inv.item_mut(selected) = replacement.clone();
    p.inv.times_changed += 1;
    let silent = replacement.get(kiln_item::keys::USE_EFFECTS).is_some_and(|u| !u.interact_vibrations);
    changed(level, pos, if silent { "" } else { "minecraft:item_interact_finish" });
    Some(!old.is_empty())
}

/// `ShelfBlock.swapHotbar`: whether any slot swapped.
fn swap_hotbar(p: &mut Player, level: &mut RegionLevel, pos: BlockPos) -> bool {
    let row = chain::all_connected_to(level, pos);
    if row.is_empty() {
        return false;
    }
    let mut swapped = false;
    for (i, &at) in row.iter().enumerate() {
        if level.blocks.containers.get(at).is_none_or(|c| c.kind != BeKind::Shelf) {
            continue;
        }
        for j in 0..3usize {
            let index = 9 - (row.len() - i) as i32 * 3 + j as i32;
            if index < 0 || index as usize > p.inv.size() {
                continue;
            }
            let index = index as usize;
            let removed = p.inv.remove_item_no_update(index);
            let Some(out) = swap_item_no_update(level, at, j, removed.clone()) else { continue };
            if !removed.is_empty() || !out.is_empty() {
                *p.inv.item_mut(index) = out;
                swapped = true;
            }
        }
        p.inv.times_changed += 1;
        changed(level, at, "minecraft:entity_interact");
    }
    swapped
}

/// `ShelfBlock.useItemOn` for the main hand (an empty hand included). `Some(true)`: the click did something; `None`: not a shelf, or
/// not this click's.
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, face: Direction, cursor: [f32; 3], off_hand: bool) -> Option<bool> {
    if off_hand || level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::Shelf) {
        return None;
    }
    let slot = chain::hit_slot(s, face, cursor)?;
    if !state::get_bool(s, "powered") {
        let held = p.inv.selected_item().clone();
        if swap_single_item(p, level, pos, slot, &held)? {
            sound(level, pos, if held.is_empty() { "minecraft:block.shelf.take_item" } else { "minecraft:block.shelf.single_swap" });
        } else if !held.is_empty() {
            sound(level, pos, "minecraft:block.shelf.place_item");
        } else {
            return None;
        }
        return Some(true);
    }
    // (`CONSUME` when nothing swapped: the click is taken, nothing else happens.)
    if swap_hotbar(p, level, pos) {
        sound(level, pos, "minecraft:block.shelf.multi_swap");
    }
    Some(true)
}

/// `ShelfBlock.getAnalogOutputSignal`.
pub(crate) fn analog(level: &RegionLevel, pos: BlockPos, s: u16, from: Direction) -> i32 {
    let Some(c) = level.blocks.containers.get(pos).filter(|c| c.kind == BeKind::Shelf) else { return 0 };
    chain::analog(s, from, [!c.items[0].is_empty(), !c.items[1].is_empty(), !c.items[2].is_empty()])
}
