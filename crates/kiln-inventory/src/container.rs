//! Containers: the storage behind menu slots (`net.minecraft.world.Container`).

use crate::stack::StackExt;
use kiln_item::ItemStack;

/// Storage a menu slot reads and writes (`Container`). Block entities (chests, furnaces,
/// hoppers...) implement it; [`SimpleContainer`] has the behavior of vanilla's
/// `SimpleContainer` and `BaseContainerBlockEntity`.
///
/// Vanilla code mutates the stack returned by `getItem` in place without notifying the
/// container; [`Container::item_mut`] is that path, [`Container::set_item`] the notifying one.
pub trait Container {
    fn size(&self) -> usize;

    /// `getItem`.
    fn item(&self, slot: usize) -> &ItemStack;

    /// `getItem` for in-place mutation (no `setChanged`).
    fn item_mut(&mut self, slot: usize) -> &mut ItemStack;

    /// `setItem`.
    fn set_item(&mut self, slot: usize, stack: ItemStack);

    /// `removeItem`: splits up to `count` items off a slot.
    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack;

    /// `removeItemNoUpdate`: takes the whole stack.
    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack;

    /// `getMaxStackSize()`.
    fn max_stack_size(&self) -> i32 {
        99
    }

    /// `getMaxStackSize(ItemStack)`.
    fn max_stack_size_for(&self, stack: &ItemStack) -> i32 {
        self.max_stack_size().min(stack.max_stack_size())
    }

    /// `setChanged`.
    fn set_changed(&mut self) {}

    /// `ContainerData` values the menu synchronizes (furnace fuel and progress...).
    fn data(&self, _index: usize) -> i32 {
        0
    }

    /// `ContainerData.set` (a lectern's page).
    fn set_data(&mut self, _index: usize, _value: i32) {}
}

/// `ContainerHelper.removeItem`.
pub fn remove_item(items: &mut [ItemStack], slot: usize, count: i32) -> ItemStack {
    match items.get_mut(slot) {
        Some(s) if !s.is_empty() && count > 0 => s.split_count(count),
        _ => ItemStack::empty(),
    }
}

/// `ContainerHelper.takeItem`.
pub fn take_item(items: &mut [ItemStack], slot: usize) -> ItemStack {
    items.get_mut(slot).map(std::mem::take).unwrap_or_default()
}

/// `SimpleContainer` / `BaseContainerBlockEntity`: `setItem` limits the stack to the slot's
/// maximum, and changes are counted so the owner can mark itself dirty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimpleContainer {
    pub items: Vec<ItemStack>,
    /// Number of `setChanged` calls.
    pub changes: u64,
    /// `ContainerData` values (4 for furnaces, 2 for brewing stands...).
    pub data: Vec<i32>,
}

impl SimpleContainer {
    pub fn new(size: usize) -> Self {
        SimpleContainer { items: vec![ItemStack::empty(); size], changes: 0, data: Vec::new() }
    }

    pub fn from_items(items: Vec<ItemStack>) -> Self {
        SimpleContainer { items, changes: 0, data: Vec::new() }
    }
}

impl Container for SimpleContainer {
    fn size(&self) -> usize {
        self.items.len()
    }

    fn item(&self, slot: usize) -> &ItemStack {
        &self.items[slot]
    }

    fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        &mut self.items[slot]
    }

    fn set_item(&mut self, slot: usize, mut stack: ItemStack) {
        let max = self.max_stack_size_for(&stack);
        stack.limit_size(max);
        self.items[slot] = stack;
        self.set_changed();
    }

    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        let removed = remove_item(&mut self.items, slot, count);
        if !removed.is_empty() {
            self.set_changed();
        }
        removed
    }

    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack {
        take_item(&mut self.items, slot)
    }

    fn set_changed(&mut self) {
        self.changes += 1;
    }

    fn data(&self, index: usize) -> i32 {
        self.data.get(index).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_item_limits_to_the_item_maximum() {
        let mut c = SimpleContainer::new(3);
        c.set_item(0, ItemStack::of("ender_pearl", 40).unwrap());
        assert_eq!(c.item(0).count(), 16);
        assert_eq!(c.remove_item(0, 5).count(), 5);
        assert_eq!(c.item(0).count(), 11);
        assert!(c.remove_item(1, 5).is_empty());
        assert_eq!(c.changes, 2);
        assert_eq!(c.remove_item_no_update(0).count(), 11);
        assert!(c.item(0).is_empty());
    }
}
