//! Using consumable items (`Consumable.startConsuming`, `LivingEntity.updatingUsingItem`,
//! `completeUsingItem`): food is eaten over its consume time, then feeds the player and
//! leaves its remainder (a bowl, a bottle). Consume effects (potion effects, teleports,
//! clearing effects) are not applied yet.

use crate::{Player, entities};
use kiln_inventory::Container;
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};
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
    /// always edible, or invulnerable) starts being eaten.
    pub(crate) fn use_item(&mut self, off_hand: bool, spawns: &mut Vec<entities::Spawn>) {
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
        let ticks = (consumable.consume_seconds * 20.0) as i32;
        let item = stack.item();
        if ticks <= 0 {
            self.finish_using(off_hand, spawns);
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

    /// `LivingEntity.updatingUsingItem`: switching away from the item stops using it; the last
    /// tick completes it.
    pub(crate) fn tick_using(&mut self, spawns: &mut Vec<entities::Spawn>) {
        let Some(mut u) = self.using else { return };
        let stack = self.hand_stack(u.off_hand);
        if stack.is_empty() || stack.item() != u.item {
            self.stop_using();
            return;
        }
        u.remaining -= 1;
        self.using = Some(u);
        if u.remaining <= 0 {
            self.finish_using(u.off_hand, spawns);
        }
    }

    /// `ItemStack.finishUsingItem` for a consumable: feeds the player (`FoodData.eat`), uses
    /// up one item (not in creative) and leaves the remainder.
    fn finish_using(&mut self, off_hand: bool, spawns: &mut Vec<entities::Spawn>) {
        self.using = None;
        self.meta_dirty = true;
        let slot = self.hand_slot(off_hand);
        let stack = self.inv.item(slot);
        if let Some(food) = stack.get(keys::FOOD) {
            self.food = (self.food + food.nutrition).clamp(0, 20);
            self.saturation = (self.saturation + food.saturation).clamp(0.0, self.food as f32);
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
                        spawns.push(self.throw(rest));
                    }
                }
            }
            self.inv.times_changed += 1;
        }
        self.send(entity::entity_event(self.entity_id, USE_ITEM_COMPLETE));
    }
}
