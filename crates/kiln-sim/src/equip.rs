//! Equipping by right click and the equip sound: `Item.use` of an item with the `equippable`
//! component (`Equippable.swapWithEquipmentSlot`) and `LivingEntity.onEquipItem` (called by
//! `setItemSlot` and by a player moving an armor piece into its slot in the inventory screen).
//!
//! Not simulated: the `EQUIP` / `UNEQUIP` game event that wakes sculk sensors.

use crate::Player;
use crate::entities::Spawn;
use kiln_inventory::Container;
use kiln_inventory::inventory::equipment_index;
use kiln_inventory::stack::StackExt;
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};
use kiln_proto::packets::world_fx::SoundSource;

impl Player {
    /// `Item.use` of an item with a swappable `equippable` component, which swaps it with
    /// what is worn in its slot. Returns whether the item handled the use (vanilla's
    /// `InteractionResult` was not `PASS`).
    pub(crate) fn use_equippable(&mut self, off_hand: bool, rules: &kiln_inventory::Rules, spawns: &mut Vec<Spawn>) -> bool {
        let hand = self.hand_index(off_hand);
        let stack = self.inv.item(hand).clone();
        let Some(equippable) = stack.get(keys::EQUIPPABLE).cloned() else { return false };
        if !equippable.swappable {
            return false;
        }
        let slot = equippable.slot;
        // `Player.canUseSlot` is true for every slot; only the humanoid slots exist for a player.
        if matches!(slot, EquipmentSlot::Body | EquipmentSlot::Saddle | EquipmentSlot::MainHand) || !rules.is_equippable_in_slot(&stack, slot) {
            return false;
        }
        let at = equipment_index(slot, self.inv.selected);
        let worn = self.inv.item(at).clone();
        let creative = self.game_mode == 1;
        // The curse of binding holds the worn piece (creative players break it); an identical
        // piece changes nothing.
        if (rules.prevents_armor_change(&worn) && !creative) || stack.is_same_item_same_components(&worn) {
            return true;
        }
        self.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, stack.item()), 1);
        // `setItemSlot` reports the stack that was in the slot, which the swap has already
        // emptied (`copyAndClear`): the equip sound is judged against an empty stack.
        let before = ItemStack::empty();
        if stack.count() <= 1 {
            // The held item goes to the slot and the worn one to the hand (or the hand empties).
            let in_hand = if worn.is_empty() {
                if creative { stack.clone() } else { ItemStack::empty() }
            } else {
                worn
            };
            let to_equip = stack;
            *self.inv.item_mut(at) = to_equip.clone();
            *self.inv.item_mut(hand) = in_hand;
            self.inv.times_changed += 1;
            self.on_equip_item(slot, &before, &to_equip);
        } else {
            // One of the stack is worn; what was worn goes into the inventory (or is dropped).
            let mut swapped = worn;
            *self.inv.item_mut(at) = ItemStack::empty();
            let to_equip = if creative {
                stack.copy_with_count(1)
            } else {
                self.inv.item_mut(hand).split_count(1)
            };
            *self.inv.item_mut(at) = to_equip.clone();
            self.inv.times_changed += 1;
            self.on_equip_item(slot, &before, &to_equip);
            if !swapped.is_empty() {
                self.add_to_inventory(&mut swapped);
                if !swapped.is_empty() {
                    spawns.push(self.throw(swapped));
                }
            }
        }
        true
    }

    /// `LivingEntity.onEquipItem`: the equip sound of the new item when it is not the same as
    /// the old one and names this slot, for everyone near (the seed comes off the player's
    /// random).
    pub(crate) fn on_equip_item(&mut self, slot: EquipmentSlot, old: &ItemStack, new: &ItemStack) {
        if self.game_mode == 3 || old.is_same_item_same_components(new) {
            return;
        }
        let Some(equippable) = new.get(keys::EQUIPPABLE) else { return };
        if equippable.slot != slot {
            return;
        }
        let sound = match &equippable.equip_sound {
            kiln_item::Holder::Reference(id) => kiln_item::registry::SOUND_EVENT.name(*id),
            kiln_item::Holder::Direct(_) => None,
        }
        .unwrap_or("minecraft:item.armor.equip_generic");
        self.sound_for_all(sound, SoundSource::Players, 1.0, 1.0);
    }
}
