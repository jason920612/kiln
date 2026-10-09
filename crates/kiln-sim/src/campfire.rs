//! Campfires (`CampfireBlock`, `CampfireBlockEntity`): up to four foods lie on the fire and cook
//! while it is lit, each for its campfire cooking recipe's time (`cookingTime`, 600 ticks), then
//! drop as the recipe's result (the food itself when the recipe is gone). A fire that is out
//! lets the cooking go back two ticks at a time. Food is put on by right-clicking with an item
//! that has a campfire cooking recipe; a campfire that is broken drops what is on it.
//!
//! The block entity is a [`ContainerBe`] of four slots (the items) with the two timer arrays
//! (`CookingTimes`, `CookingTotalTimes`); the clients are sent its `Items` (the food they draw).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::{BeKind, ContainerBe, drop_item_stack, pos_random};
use kiln_blocks::{BlockPos, Effect, Level, state};
use kiln_world::Blocks;
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_inventory::recipe::{CookingKind, Recipe};
use kiln_item::ItemStack;

pub(crate) fn is_campfire(s: u16) -> bool {
    logic::block_class(s) == C::CampfireBlock
}

/// The campfire cooking recipe of an item: its index and its `cookingTime`.
fn recipe_for(level: &RegionLevel, stack: &ItemStack) -> Option<(usize, i32)> {
    let rules = &level.env.menus;
    let index = rules.recipes.find_cooking(CookingKind::Campfire, stack, None)?;
    match &rules.recipes.recipes()[index].recipe {
        Recipe::Cooking(c) => Some((index, c.cooking_time)),
        _ => None,
    }
}

/// `BlockEntity.markUpdated`: saved, and the clients that have the chunk see the food
/// (`Level.sendBlockUpdated`).
fn mark_updated(level: &mut RegionLevel, pos: BlockPos) {
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    c.mark_changed();
    crate::container::hopper::changed(level, pos);
    // The chunk's copy is what the update packet is made of.
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let (type_id, saved) = (c.type_id, c.save());
    c.dirty = false;
    if let Some(chunk) = level.cells.chunk_mut(kiln_world::ChunkPos::of_block(pos.x, pos.z))
        && chunk.block_entity(x, pos.y, z).is_some_and(|be| be.kind == type_id)
    {
        let mut be = kiln_world::block_entity::BlockEntity::new(type_id);
        if let (kiln_proto::nbt::Tag::Compound(out), kiln_proto::nbt::Tag::Compound(fields)) = (&mut be.nbt, saved) {
            out.extend(fields);
        }
        chunk.set_block_entity(x, pos.y, z, be);
    }
    level.out.changed.push([pos.x, pos.y, pos.z]);
}

/// `CampfireBlockEntity.cookTick` (a lit campfire) and `cooldownTick` (an unlit one).
pub(crate) fn tick(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    if !is_campfire(s) || level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::Campfire) {
        return;
    }
    if state::get_bool(s, "lit") {
        cook_tick(level, pos, s);
    } else {
        cooldown_tick(level, pos);
    }
}

fn cook_tick(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    let mut changed = false;
    for slot in 0..4 {
        let Some(c) = level.blocks.containers.get_mut(pos) else { return };
        if c.items[slot].is_empty() {
            continue;
        }
        changed = true;
        c.cooking[slot] += 1;
        if c.cooking[slot] < c.cooking_total[slot] {
            continue;
        }
        let input = c.items[slot].clone();
        let result = recipe_for(level, &input).map_or_else(|| input.clone(), |(index, _)| level.env.menus.recipes.assemble_single(index));
        let mut rng = pos_random(level, pos, 7 + slot as u64);
        let mut spawns = std::mem::take(&mut level.out.spawns);
        drop_item_stack([pos.x as f64, pos.y as f64, pos.z as f64], result, &mut rng, &mut spawns);
        level.out.spawns.append(&mut spawns);
        if let Some(c) = level.blocks.containers.get_mut(pos) {
            c.items[slot] = ItemStack::empty();
        }
        mark_updated(level, pos);
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: s });
    }
    if changed && let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
        crate::container::hopper::changed(level, pos);
    }
}

fn cooldown_tick(level: &mut RegionLevel, pos: BlockPos) {
    let mut changed = false;
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        for slot in 0..4 {
            if c.cooking[slot] > 0 {
                changed = true;
                c.cooking[slot] = (c.cooking[slot] - 2).clamp(0, c.cooking_total[slot]);
            }
        }
        if changed {
            c.mark_changed();
        }
    }
    if changed {
        crate::container::hopper::changed(level, pos);
    }
}

/// `CampfireBlock.useItemOn`: an item with a campfire cooking recipe goes onto a free spot (one
/// of it, unless the player is in creative mode); with all four taken the click is consumed
/// without a result. `None`: not food for the fire.
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> Option<bool> {
    if !is_campfire(s) || level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::Campfire) {
        return None;
    }
    let stack = p.in_hand(off_hand).clone();
    let (_, cook_time) = recipe_for(level, &stack)?;
    // `placeFood`.
    let c = level.blocks.containers.get_mut(pos)?;
    let Some(slot) = (0..4).find(|&i| c.items[i].is_empty()) else { return Some(true) };
    c.cooking_total[slot] = cook_time;
    c.cooking[slot] = 0;
    let mut one = stack.clone();
    one.set_count(1);
    c.items[slot] = one;
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    mark_updated(level, pos);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: s });
    p.award_stat(*crate::player_stats::stat::INTERACT_WITH_CAMPFIRE, 1);
    Some(true)
}

/// A campfire block entity read from its saved data: four slots and their timers.
pub(crate) fn load_timers(c: &mut ContainerBe, nbt: &kiln_proto::nbt::Tag) {
    use kiln_proto::nbt::Tag;
    for (key, target) in [("CookingTimes", &mut c.cooking), ("CookingTotalTimes", &mut c.cooking_total)] {
        if let Some(Tag::IntArray(v)) = nbt.get(key) {
            for (i, t) in v.iter().take(4).enumerate() {
                target[i] = *t;
            }
        }
    }
}
