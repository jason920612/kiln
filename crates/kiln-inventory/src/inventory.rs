//! The player inventory (`net.minecraft.world.entity.player.Inventory`): 36 main slots
//! (0-8 the hotbar) and the equipment slots 36-42.

use crate::container::{self, Container};
use crate::stack::{StackExt, same_item_same_components};
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;

/// Main slots (`Inventory.INVENTORY_SIZE`).
pub const MAIN_SIZE: usize = 36;
/// Hotbar slots (`Inventory.SELECTION_SIZE`).
pub const HOTBAR_SIZE: usize = 9;
pub const SLOT_OFFHAND: usize = 40;
pub const SLOT_BODY_ARMOR: usize = 41;
pub const SLOT_SADDLE: usize = 42;
/// Main slots plus equipment (`getContainerSize`).
pub const SIZE: usize = 43;

/// Equipment slots by inventory index 36-42 (`Inventory.EQUIPMENT_SLOT_MAPPING`).
pub const EQUIPMENT: [EquipmentSlot; 7] = [
    EquipmentSlot::Feet,
    EquipmentSlot::Legs,
    EquipmentSlot::Chest,
    EquipmentSlot::Head,
    EquipmentSlot::OffHand,
    EquipmentSlot::Body,
    EquipmentSlot::Saddle,
];

/// Inventory index of an equipment slot (`MAINHAND` is the selected hotbar slot).
pub fn equipment_index(slot: EquipmentSlot, selected: usize) -> usize {
    match slot {
        EquipmentSlot::MainHand => selected,
        _ => MAIN_SIZE + EQUIPMENT.iter().position(|s| *s == slot).unwrap(),
    }
}

/// A slot of `ClientboundSetPlayerInventoryPacket` sent by [`PlayerInventory::place_item_back`].
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryUpdate {
    pub slot: usize,
    pub stack: ItemStack,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInventory {
    /// Main slots; 0-8 are the hotbar.
    pub items: Vec<ItemStack>,
    /// `EntityEquipment`, in [`EQUIPMENT`] order.
    pub equipment: Vec<ItemStack>,
    /// Selected hotbar slot.
    pub selected: usize,
    /// `Inventory.timesChanged`.
    pub times_changed: u64,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        PlayerInventory {
            items: vec![ItemStack::empty(); MAIN_SIZE],
            equipment: vec![ItemStack::empty(); EQUIPMENT.len()],
            selected: 0,
            times_changed: 0,
        }
    }
}

impl PlayerInventory {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stack in an equipment slot (`MAINHAND` is the selected hotbar slot).
    pub fn equipped(&self, slot: EquipmentSlot) -> &ItemStack {
        self.item(equipment_index(slot, self.selected))
    }

    pub fn selected_item(&self) -> &ItemStack {
        &self.items[self.selected]
    }

    /// `hasRemainingSpaceForItem`.
    fn has_remaining_space_for(&self, dest: &ItemStack, stack: &ItemStack) -> bool {
        !dest.is_empty()
            && same_item_same_components(dest, stack)
            && dest.is_stackable()
            && dest.count() < self.max_stack_size_for(dest)
    }

    /// `getFreeSlot`: the first empty main slot.
    pub fn free_slot(&self) -> Option<usize> {
        self.items.iter().position(ItemStack::is_empty)
    }

    /// `getSlotWithRemainingSpace`: the selected slot, the off hand, then the main slots.
    pub fn slot_with_remaining_space(&self, stack: &ItemStack) -> Option<usize> {
        if self.has_remaining_space_for(self.item(self.selected), stack) {
            return Some(self.selected);
        }
        if self.has_remaining_space_for(self.item(SLOT_OFFHAND), stack) {
            return Some(SLOT_OFFHAND);
        }
        (0..MAIN_SIZE).find(|&i| self.has_remaining_space_for(&self.items[i], stack))
    }

    /// `addResource(ItemStack)`: returns the count left over.
    fn add_resource(&mut self, stack: &ItemStack) -> i32 {
        match self.slot_with_remaining_space(stack).or_else(|| self.free_slot()) {
            Some(slot) => self.add_resource_at(slot, stack),
            None => stack.count(),
        }
    }

    /// `addResource(int, ItemStack)`.
    fn add_resource_at(&mut self, slot: usize, stack: &ItemStack) -> i32 {
        let mut count = stack.count();
        if self.item(slot).is_empty() {
            self.set_item(slot, stack.copy_with_count(0));
        }
        let current = self.item(slot);
        let space = self.max_stack_size_for(current) - current.count();
        let add = count.min(space);
        if add == 0 {
            return count;
        }
        count -= add;
        self.item_mut(slot).grow_count(add);
        count
    }

    /// `Inventory.add(int, ItemStack)`: moves as much of `stack` as fits (into `slot`, or
    /// anywhere for `None`); `infinite` is `Player.hasInfiniteMaterials`, which makes the
    /// whole stack vanish when nothing fits.
    pub fn add(&mut self, slot: Option<usize>, stack: &mut ItemStack, infinite: bool) -> bool {
        if stack.is_empty() {
            return false;
        }
        if !stack.is_damaged() {
            let mut before;
            loop {
                before = stack.count();
                let left = match slot {
                    None => self.add_resource(stack),
                    Some(s) => self.add_resource_at(s, stack),
                };
                stack.set_count(left);
                if stack.is_empty() || stack.count() >= before {
                    break;
                }
            }
            if stack.count() == before && infinite {
                stack.set_count(0);
                return true;
            }
            return stack.count() < before;
        }
        match slot.or_else(|| self.free_slot()) {
            Some(s) => {
                self.items[s] = stack.copy_and_clear();
                true
            }
            None if infinite => {
                stack.set_count(0);
                true
            }
            None => false,
        }
    }

    /// `placeItemBackInInventory(stack, true, ...)`: returns the slot updates vanilla sends
    /// (`ClientboundSetPlayerInventoryPacket`) and what did not fit (dropped by vanilla).
    pub fn place_item_back(&mut self, mut stack: ItemStack, infinite: bool) -> (Vec<InventoryUpdate>, Option<ItemStack>) {
        let mut updates = Vec::new();
        while !stack.is_empty() {
            let Some(slot) = self.slot_with_remaining_space(&stack).or_else(|| self.free_slot()) else {
                return (updates, Some(stack));
            };
            let space = stack.max_stack_size() - self.item(slot).count();
            let mut part = stack.split_count(space);
            if self.add(Some(slot), &mut part, infinite) {
                updates.push(InventoryUpdate { slot, stack: self.item(slot).copy() });
            }
        }
        (updates, None)
    }
}

impl Container for PlayerInventory {
    fn size(&self) -> usize {
        SIZE
    }

    fn item(&self, slot: usize) -> &ItemStack {
        static EMPTY: std::sync::OnceLock<ItemStack> = std::sync::OnceLock::new();
        match slot {
            0..MAIN_SIZE => &self.items[slot],
            MAIN_SIZE..SIZE => &self.equipment[slot - MAIN_SIZE],
            _ => EMPTY.get_or_init(ItemStack::empty),
        }
    }

    fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        match slot {
            0..MAIN_SIZE => &mut self.items[slot],
            _ => &mut self.equipment[slot - MAIN_SIZE],
        }
    }

    fn set_item(&mut self, slot: usize, stack: ItemStack) {
        match slot {
            0..MAIN_SIZE => self.items[slot] = stack,
            MAIN_SIZE..SIZE => self.equipment[slot - MAIN_SIZE] = stack,
            _ => {}
        }
    }

    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        match slot {
            0..MAIN_SIZE => container::remove_item(&mut self.items, slot, count),
            MAIN_SIZE..SIZE if !self.equipment[slot - MAIN_SIZE].is_empty() => {
                self.equipment[slot - MAIN_SIZE].split_count(count)
            }
            _ => ItemStack::empty(),
        }
    }

    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack {
        match slot {
            0..MAIN_SIZE => container::take_item(&mut self.items, slot),
            MAIN_SIZE..SIZE => std::mem::take(&mut self.equipment[slot - MAIN_SIZE]),
            _ => ItemStack::empty(),
        }
    }

    fn set_changed(&mut self) {
        self.times_changed += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(name: &str, n: i32) -> ItemStack {
        ItemStack::of(name, n).unwrap()
    }

    #[test]
    fn add_fills_existing_stacks_then_free_slots() {
        let mut inv = PlayerInventory::new();
        inv.items[3] = stack("stone", 60);
        inv.items[0] = stack("dirt", 1);
        let mut s = stack("stone", 10);
        assert!(inv.add(None, &mut s, false));
        assert!(s.is_empty());
        assert_eq!(inv.items[3].count(), 64);
        assert_eq!(inv.items[1].count(), 6);
    }

    #[test]
    fn damaged_items_go_to_a_free_slot_whole() {
        let mut inv = PlayerInventory::new();
        let mut sword = stack("diamond_sword", 1);
        sword.insert(kiln_item::keys::DAMAGE, 3);
        assert!(inv.add(None, &mut sword, false));
        assert!(sword.is_empty());
        assert_eq!(inv.items[0].get(kiln_item::keys::DAMAGE), Some(&3));
    }

    #[test]
    fn place_back_reports_updates_and_overflow() {
        let mut inv = PlayerInventory::new();
        for i in 0..MAIN_SIZE {
            inv.items[i] = stack("dirt", 64);
        }
        inv.items[5] = stack("stone", 50);
        let (updates, left) = inv.place_item_back(stack("stone", 20), false);
        assert_eq!(updates, vec![InventoryUpdate { slot: 5, stack: stack("stone", 64) }]);
        assert_eq!(left.map(|s| s.count()), Some(6));
    }
}
