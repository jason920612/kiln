//! Using consumable items (`Consumable.startConsuming`, `LivingEntity.updatingUsingItem`,
//! `completeUsingItem`, `Consumable.onConsume`): food and potions are eaten or drunk over their
//! consume time, then feed the player, apply their effects and leave their remainder (a bowl,
//! a bottle, a bucket).
//!
//! On completion, in vanilla's order: the consume particles and sound, the consumable listeners
//! (`FoodProperties` feeds, `PotionContents` applies the potion's effects with the item's
//! duration scale, `SuspiciousStewEffects` adds its effects), then `on_consume_effects`:
//! `apply_effects` (with its probability), `remove_effects`, `clear_all_effects` (milk),
//! `teleport_randomly` (chorus fruit) and `play_sound`.
//!
//! The particles and sounds draw from the player's random (`Entity.random`) exactly as vanilla
//! does, so that probabilities that follow (rotten flesh's hunger) come out the same.

use crate::hazards::BlockAt;
use crate::health::DamageCtx;
use crate::Player;
use crate::effects::Effect;
use kiln_entity::math::{Aabb, BlockPos};
use kiln_inventory::Container;
use kiln_item::component::{ConsumeEffect, EquipmentSlot, ItemUseAnimation};
use kiln_item::{HolderSet, ItemStack, keys};
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity;

/// An item being used: which hand, the item it started with, ticks left.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Using {
    pub off_hand: bool,
    pub item: i32,
    pub remaining: i32,
}

/// `EntityEvent.USE_ITEM_COMPLETE`.
const USE_ITEM_COMPLETE: u8 = 9;
/// `EntityEvent.TELEPORT` (chorus fruit particles).
const TELEPORT: u8 = 46;

impl Player {
    fn hand_slot(&self, off_hand: bool) -> usize {
        let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
        kiln_inventory::inventory::equipment_index(slot, self.inv.selected)
    }

    fn hand_stack(&self, off_hand: bool) -> &ItemStack {
        self.inv.item(self.hand_slot(off_hand))
    }

    /// `LivingEntity.DATA_LIVING_ENTITY_FLAGS`: using an item, and with which hand.
    pub(crate) fn living_flags(&self) -> i8 {
        match self.using {
            Some(u) => 1 | if u.off_hand { 2 } else { 0 },
            None => 0,
        }
    }

    /// `ServerPlayerGameMode.useItem` for consumables: food the player can eat (hungry,
    /// always edible, or invulnerable) and other consumables start being used.
    pub(crate) fn use_item(&mut self, off_hand: bool, block: BlockAt, ctx: &mut DamageCtx) {
        if self.game_mode == 3 || self.using.is_some() {
            return;
        }
        let stack = self.hand_stack(off_hand);
        let Some(consumable) = stack.get(keys::CONSUMABLE) else { return };
        if let Some(food) = stack.get(keys::FOOD)
            && !(matches!(self.game_mode, 1 | 3) || food.can_always_eat || self.food < 20)
        {
            return;
        }
        let ticks = consume_ticks(consumable);
        let item = stack.item();
        if ticks <= 0 {
            self.finish_using(off_hand, block, ctx);
            return;
        }
        self.using = Some(Using { off_hand, item, remaining: ticks });
        self.meta_dirty = true;
    }

    /// `LivingEntity.releaseUsingItem` / `stopUsingItem`.
    pub(crate) fn stop_using(&mut self) {
        if self.using.take().is_some() {
            self.meta_dirty = true;
        }
    }

    /// `LivingEntity.updatingUsingItem`: switching away from the item stops using it; every
    /// fourth tick past the first 21.875% emits particles and sounds (`onUseTick`); the last
    /// tick completes it.
    pub(crate) fn tick_using(&mut self, block: BlockAt, ctx: &mut DamageCtx) {
        let Some(mut u) = self.using else { return };
        let stack = self.hand_stack(u.off_hand);
        if stack.is_empty() || stack.item() != u.item {
            self.stop_using();
            return;
        }
        if let Some(consumable) = stack.get(keys::CONSUMABLE).cloned() {
            // `Consumable.shouldEmitParticlesAndSounds(remaining)`.
            let total = consume_ticks(&consumable);
            if total - u.remaining > (total as f32 * 0.21875) as i32 && u.remaining % 4 == 0 {
                self.consume_particles_and_sounds(&consumable, 5);
            }
        }
        u.remaining -= 1;
        self.using = Some(u);
        if u.remaining == 0 {
            // `ServerPlayer.completeUsingItem` tells the client first.
            self.send(entity::entity_event(self.entity_id, USE_ITEM_COMPLETE));
            self.finish_using(u.off_hand, block, ctx);
        }
    }

    /// `Consumable.emitParticlesAndSounds(random, entity, stack, count)`: the random draws of
    /// the sound and the item particles, and the sound for viewers.
    fn consume_particles_and_sounds(&mut self, consumable: &kiln_item::component::Consumable, particles: i32) {
        let r = &mut self.entity_rng;
        let eat_volume = if r.next_bool() { 0.5 } else { 1.0 };
        let eat_pitch = 1.0 + 0.2 * (r.next_float() - r.next_float());
        let drink_pitch = r.next_float() * (1.0 - 0.9) + 0.9;
        let drink = consumable.animation == ItemUseAnimation::Drink;
        let (volume, pitch) = if drink { (0.5, drink_pitch) } else { (eat_volume, eat_pitch) };
        if consumable.has_consume_particles {
            // `spawnItemParticles`: four draws per particle (the particles are client-side).
            for _ in 0..particles * 4 {
                r.next_float();
            }
        }
        if let kiln_item::Holder::Reference(id) = &consumable.sound
            && let Some(name) = kiln_item::registry::SOUND_EVENT.name(*id)
        {
            self.queue_sound(name, volume, pitch);
        }
    }

    /// `ItemStack.finishUsingItem` for a consumable (`Consumable.onConsume`): particles and
    /// sounds, the listeners, the consume effects, one item used up (not in creative) and the
    /// remainder.
    pub(crate) fn finish_using(&mut self, off_hand: bool, block: BlockAt, ctx: &mut DamageCtx) {
        self.using = None;
        self.meta_dirty = true;
        let slot = self.hand_slot(off_hand);
        let stack = self.inv.item(slot).clone();
        let Some(consumable) = stack.get(keys::CONSUMABLE).cloned() else { return };
        self.consume_particles_and_sounds(&consumable, 16);
        // Listeners: food, then potion contents, then suspicious stew.
        if let Some(food) = stack.get(keys::FOOD) {
            let r = &mut self.entity_rng;
            r.next_float();
            r.next_float();
            self.eat(food.nutrition, food.saturation);
            // The burp's pitch.
            self.entity_rng.next_float();
        }
        if let Some(contents) = stack.get(keys::POTION_CONTENTS) {
            let scale = stack.get(keys::POTION_DURATION_SCALE).copied().unwrap_or(1.0);
            for e in crate::effects::potion_effects(contents, scale) {
                self.apply_potion_effect(e, ctx);
            }
        }
        if let Some(stew) = stack.get(keys::SUSPICIOUS_STEW_EFFECTS) {
            for e in &stew.0 {
                self.add_effect(Effect::simple(e.effect, e.duration, 0));
            }
        }
        for effect in &consumable.on_consume_effects {
            self.apply_consume_effect(effect, block, ctx.game_time);
        }
        let remainder = stack.get(keys::USE_REMAINDER).map(|r| r.0.create());
        if self.game_mode != 1 {
            let stack = self.inv.item_mut(slot);
            stack.shrink(1);
            // `UseRemainder.convertIntoRemainder`: into the emptied slot, else the inventory.
            if let Some(mut rest) = remainder {
                if stack.is_empty() {
                    *stack = rest;
                } else {
                    self.add_to_inventory(&mut rest);
                    if !rest.is_empty() {
                        ctx.spawns.push(self.throw(rest));
                    }
                }
            }
            self.inv.times_changed += 1;
        }
    }

    /// `ConsumeEffect.apply`.
    fn apply_consume_effect(&mut self, effect: &ConsumeEffect, block: BlockAt, now: i64) -> bool {
        match effect {
            ConsumeEffect::ApplyEffects { effects, probability } => {
                if self.entity_rng.next_float() >= *probability {
                    return false;
                }
                let mut any = false;
                for e in effects {
                    any |= self.add_effect(Effect::from_item(e));
                }
                any
            }
            ConsumeEffect::RemoveEffects(set) => {
                let ids: Vec<i32> = match set {
                    HolderSet::Direct(ids) => ids.clone(),
                    HolderSet::Tag(tag) => tag_entries("minecraft:mob_effect", tag.as_str()),
                };
                let mut any = false;
                for id in ids {
                    any |= self.remove_effect(id);
                }
                any
            }
            ConsumeEffect::ClearAllEffects => self.remove_all_effects(),
            ConsumeEffect::TeleportRandomly { diameter, directional_particles: _ } => self.teleport_randomly(*diameter, block, now),
            ConsumeEffect::PlaySound(sound) => {
                if let kiln_item::Holder::Reference(id) = sound
                    && let Some(name) = kiln_item::registry::SOUND_EVENT.name(*id)
                {
                    self.queue_sound(name, 1.0, 1.0);
                }
                true
            }
        }
    }

    /// `TeleportRandomlyConsumeEffect.apply`: up to 16 tries at a random spot within `diameter`
    /// (`LivingEntity.randomTeleport`: down to the first block that blocks motion, then a free,
    /// dry box without dangerous blocks).
    fn teleport_randomly(&mut self, diameter: f32, block: BlockAt, now: i64) -> bool {
        // `level.getMinY()` to `getMinY() + getLogicalHeight() - 1` of the player's level.
        let dim = kiln_data::dimension_type(crate::DIMENSIONS[self.dim].0).expect("dimension type");
        let (min_y, max_y) = (dim.min_y, dim.min_y + dim.logical_height - 1);
        for _ in 0..16 {
            let x = self.pos[0] + (self.entity_rng.next_double() - 0.5) * diameter as f64;
            let y = (self.pos[1] + (self.entity_rng.next_double() - 0.5) * diameter as f64).clamp(min_y as f64, max_y as f64);
            let z = self.pos[2] + (self.entity_rng.next_double() - 0.5) * diameter as f64;
            if let Some(to) = self.random_teleport_target(x, y, z, block) {
                self.stop_using();
                self.teleport(to, self.rot, now);
                self.block_effects_from = to;
                self.entity_events.push(TELEPORT);
                self.send(entity::entity_event(self.entity_id, TELEPORT));
                self.queue_sound("minecraft:item.chorus_fruit.teleport", 1.0, 1.0);
                self.fall_distance = 0.0;
                return true;
            }
        }
        false
    }

    fn random_teleport_target(&self, x: f64, y: f64, z: f64, block: BlockAt) -> Option<[f64; 3]> {
        let mut y = y;
        let mut pos = BlockPos::containing(x, y, z);
        let min_y = kiln_data::dimension_type(crate::DIMENSIONS[self.dim].0).map_or(-64, |d| d.min_y);
        while pos.y > min_y {
            pos = pos.below();
            let state = block(pos);
            if kiln_entity::physics::collision_shape(state).is_empty() {
                y -= 1.0;
                continue;
            }
            if in_block_tag(state, "minecraft:dangerous_for_teleportation") {
                return None;
            }
            let bb = self.bounding_box_at([x, y, z]);
            if !self.box_is_clear(&bb, block) {
                return None;
            }
            return Some([x, y, z]);
        }
        None
    }

    /// `noCollision`, `containsAnyLiquid` and no dangerous blocks in `bb`.
    fn box_is_clear(&self, bb: &Aabb, block: BlockAt) -> bool {
        let floor = |v: f64| v.floor() as i32;
        for bx in floor(bb.min_x)..=floor(bb.max_x) {
            for by in floor(bb.min_y)..=floor(bb.max_y) {
                for bz in floor(bb.min_z)..=floor(bb.max_z) {
                    let state = block(BlockPos::new(bx, by, bz));
                    if !kiln_entity::physics::fluid_state(state).is_empty()
                        || in_block_tag(state, "minecraft:dangerous_for_teleportation")
                    {
                        return false;
                    }
                    let shape = kiln_entity::physics::collision_shape(state);
                    if shape.boxes().iter().any(|b| b.offset(bx as f64, by as f64, bz as f64).intersects(bb)) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// `Consumable.consumeTicks`.
fn consume_ticks(c: &kiln_item::component::Consumable) -> i32 {
    (c.consume_seconds * 20.0) as i32
}

/// Entries of a registry tag (network ids).
fn tag_entries(registry: &str, tag: &str) -> Vec<i32> {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == registry)
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .map(|(_, ids)| ids.to_vec())
        .unwrap_or_default()
}

/// Whether a block state's block is in a `minecraft:block` tag.
fn in_block_tag(state: u16, tag: &str) -> bool {
    let name = kiln_entity::blocks::block_name(state);
    let Some(id) = kiln_item::registry::BLOCK.id(name) else { return false };
    tag_entries("minecraft:block", tag).contains(&id)
}
