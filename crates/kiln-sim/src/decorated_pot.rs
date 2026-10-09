//! Decorated pots (`DecoratedPotBlock`, `DecoratedPotBlockEntity`): a one-slot container with
//! four sherds on its sides. A click with an item puts one of it in (a stack of the same item
//! grows by one) and the pot wobbles; any other click makes the pot shake its head (a failure
//! sound and the opposite wobble). A pot hit by a projectile cracks and breaks; broken with a
//! pickaxe or the like (and no silk touch) a pot cracks first and drops its sherds instead of
//! itself. The wobble is a block event (`1`, the style), which clients animate.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::{BeKind, unpack_loot};
use kiln_blocks::{BlockId, BlockPos, Effect, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_item::ItemStack;
use kiln_proto::packets::world_fx;

/// `DecoratedPotBlockEntity.WobbleStyle` by ordinal.
const POSITIVE: i32 = 0;
const NEGATIVE: i32 = 1;

pub(crate) fn is_pot(s: u16) -> bool {
    logic::block_class(s) == C::DecoratedPotBlock
}

/// `DecoratedPotBlockEntity.wobble`: the block event.
fn wobble(level: &mut RegionLevel, pos: BlockPos, s: u16, style: i32) {
    kiln_blocks::block_events::block_event(level, pos, BlockId::of(s), 1, style);
}

/// The pot's item after its loot (if any) was rolled (`getTheItem`).
fn the_item(level: &mut RegionLevel, pos: BlockPos) -> Option<ItemStack> {
    let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    let c = level.blocks.containers.get_mut(pos).filter(|c| c.kind == BeKind::DecoratedPot)?;
    unpack_loot(c, pos, loot.as_deref(), false, game_time, seed);
    Some(c.items[0].clone())
}

/// `DecoratedPotBlock.useItemOn`: `Some(true)` the item went in; `Some(false)` it did not (the empty-hand
/// use follows); `None` not a pot.
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> Option<bool> {
    if !is_pot(s) {
        return None;
    }
    let held = p.in_hand(off_hand).clone();
    let inside = the_item(level, pos)?;
    if held.is_empty() || !(inside.is_empty() || (inside.is_same_item_same_components(&held) && inside.count() < inside.max_stack_size())) {
        return Some(false);
    }
    wobble(level, pos, s, POSITIVE);
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, held.item()), 1);
    // `consumeAndReturn(1, player)`.
    let mut one = held.clone();
    one.set_count(1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    let fill;
    {
        let c = level.blocks.containers.get_mut(pos)?;
        if c.items[0].is_empty() {
            fill = one.count() as f32 / one.max_stack_size() as f32;
            c.items[0] = one;
        } else {
            c.items[0].grow(1);
            fill = c.items[0].count() as f32 / c.items[0].max_stack_size() as f32;
        }
        c.mark_changed();
    }
    level.effect(Effect::Sound { pos, sound: "minecraft:block.decorated_pot.insert", volume: 1.0, pitch: 0.7 + 0.5 * fill });
    if let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:dust_plume") {
        let at = [pos.x as f64 + 0.5, pos.y as f64 + 1.2, pos.z as f64 + 0.5];
        let pkt = world_fx::level_particles(&world_fx::LevelParticles {
            particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::None },
            override_limiter: false,
            always_show: false,
            pos: at,
            offset: [0.0; 3],
            max_speed: [0.0; 3],
            count: 7,
            randomization: world_fx::ParticleRandomization::Default,
        });
        level.out.packets.push((at, 32.0, pkt));
    }
    let now = level.block(pos);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: now });
    Some(true)
}

/// `DecoratedPotBlock.useWithoutItem`: the pot shakes its head.
pub(crate) fn use_without_item(level: &mut RegionLevel, pos: BlockPos, s: u16) -> bool {
    if !is_pot(s) || level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::DecoratedPot) {
        return false;
    }
    level.effect(Effect::Sound { pos, sound: "minecraft:block.decorated_pot.insert_fail", volume: 1.0, pitch: 1.0 });
    wobble(level, pos, s, NEGATIVE);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: s });
    true
}

/// `DecoratedPotBlock.playerWillDestroy`: a pot broken with a tool that breaks pots (and no silk touch)
/// cracks first, and so drops its sherds.
pub(crate) fn will_destroy(level: &mut RegionLevel, pos: BlockPos, s: u16, tool: &ItemStack) {
    if !is_pot(s) {
        return;
    }
    let breaks = !tool.is_empty() && kiln_entity::mob::item_tag(tool.item(), "minecraft:breaks_decorated_pots");
    let silk = tool.get(kiln_item::keys::ENCHANTMENTS).is_some_and(|e| kiln_item::registry::ENCHANTMENT.id("minecraft:silk_touch").is_some_and(|id| e.level(id) > 0));
    if breaks && !silk {
        // `setBlock(pos, cracked, 260)`: clients only, no neighbour updates.
        kiln_blocks::set_block(level, pos, state::set_bool(s, "cracked", true), kiln_blocks::flags::NONE);
    }
}

/// `DecoratedPotBlock.onProjectileHit`: the pot cracks and breaks (with its drops, as the projectile's doing).
pub(crate) fn projectile_hit(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    if !is_pot(s) {
        return;
    }
    kiln_blocks::set_block(level, pos, state::set_bool(s, "cracked", true), kiln_blocks::flags::NONE);
    let cracked = level.block(pos);
    kiln_blocks::destroy_block(level, pos, false, 512);
    level.effect(Effect::EntityDrop { pos, state: cracked });
}
