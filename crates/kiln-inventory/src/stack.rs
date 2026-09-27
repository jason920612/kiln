//! `ItemStack` operations with vanilla's exact semantics.
//!
//! Vanilla keeps a stack's item when its count drops to zero (the stack is then empty, but
//! `grow` brings it back), and every comparison treats an empty stack as air. These helpers
//! mirror that instead of kiln-item's raw count arithmetic, and must be used for all menu
//! logic (never compare stacks with `==`).

use kiln_item::ItemStack;

pub trait StackExt {
    /// `ItemStack.grow`: `setCount(getCount() + n)` (an empty stack counts as 0).
    fn grow_count(&mut self, n: i32);
    /// `ItemStack.shrink`.
    fn shrink_count(&mut self, n: i32);
    /// `ItemStack.split`: takes up to `n` items into a new stack.
    fn split_count(&mut self, n: i32) -> ItemStack;
    /// `ItemStack.copy`: an empty stack copies to air.
    fn copy(&self) -> ItemStack;
    /// `ItemStack.copyWithCount`.
    fn copy_with_count(&self, n: i32) -> ItemStack;
    /// `ItemStack.copyAndClear`.
    fn copy_and_clear(&mut self) -> ItemStack;
    /// `ItemStack.limitSize`.
    fn limit_size(&mut self, max: i32);
    /// `ItemStack.getItem()`: air for an empty stack.
    fn effective_item(&self) -> i32;
}

impl StackExt for ItemStack {
    fn grow_count(&mut self, n: i32) {
        self.set_count(self.count().wrapping_add(n));
    }

    fn shrink_count(&mut self, n: i32) {
        self.set_count(self.count().wrapping_sub(n));
    }

    fn split_count(&mut self, n: i32) -> ItemStack {
        let taken = n.min(self.count());
        let part = self.copy_with_count(taken);
        self.shrink_count(taken);
        part
    }

    fn copy(&self) -> ItemStack {
        if self.is_empty() { ItemStack::empty() } else { self.clone() }
    }

    fn copy_with_count(&self, n: i32) -> ItemStack {
        if self.is_empty() {
            return ItemStack::empty();
        }
        let mut c = self.clone();
        c.set_count(n);
        c
    }

    fn copy_and_clear(&mut self) -> ItemStack {
        if self.is_empty() {
            return ItemStack::empty();
        }
        let c = self.clone();
        self.set_count(0);
        c
    }

    fn limit_size(&mut self, max: i32) {
        if !self.is_empty() && self.count() > max {
            self.set_count(max);
        }
    }

    fn effective_item(&self) -> i32 {
        if self.is_empty() { kiln_item::stack::air() } else { self.item() }
    }
}

/// `ItemStack.isSameItem`.
pub fn same_item(a: &ItemStack, b: &ItemStack) -> bool {
    a.effective_item() == b.effective_item()
}

/// `ItemStack.isSameItemSameComponents`.
pub fn same_item_same_components(a: &ItemStack, b: &ItemStack) -> bool {
    if !same_item(a, b) {
        return false;
    }
    (a.is_empty() && b.is_empty()) || a.patch().same_entries(b.patch())
}

/// `ItemStack.validateStrict`: no damageable stack of more than one, nested stacks and the stack
/// itself within their maximum sizes.
pub fn is_valid_strict(s: &ItemStack) -> bool {
    use kiln_item::component::ids;
    use kiln_item::keys;
    let within = |t: &kiln_item::ItemStackTemplate| t.count <= t.create_unchecked_max();
    if s.has(ids::MAX_DAMAGE) && s.max_stack_size() > 1 {
        return false;
    }
    if let Some(c) = s.get(keys::CONTAINER)
        && !c.0.iter().flatten().all(within)
    {
        return false;
    }
    if let Some(b) = s.get(keys::BUNDLE_CONTENTS)
        && !b.0.iter().all(within)
    {
        return false;
    }
    if let Some(p) = s.get(keys::CHARGED_PROJECTILES)
        && !p.0.iter().all(within)
    {
        return false;
    }
    s.count() <= s.max_stack_size()
}

trait TemplateMax {
    fn create_unchecked_max(&self) -> i32;
}

impl TemplateMax for kiln_item::ItemStackTemplate {
    fn create_unchecked_max(&self) -> i32 {
        self.create().max_stack_size()
    }
}

/// `ItemStackTemplate.create`: an invalid stack comes out empty (vanilla logs a warning).
pub fn create_checked(t: &kiln_item::ItemStackTemplate) -> ItemStack {
    let s = t.create();
    if is_valid_strict(&s) { s } else { ItemStack::empty() }
}

/// `ItemStack.matches`: same count, item and components.
pub fn matches(a: &ItemStack, b: &ItemStack) -> bool {
    if a.count() != b.count() {
        return false;
    }
    // The common case: the same item without component changes.
    if a.item() == b.item() && a.patch().is_empty() && b.patch().is_empty() {
        return true;
    }
    same_item_same_components(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone(n: i32) -> ItemStack {
        ItemStack::of("stone", n).unwrap()
    }

    #[test]
    fn empty_stacks_keep_their_item_and_grow_back() {
        let mut s = stone(2);
        s.shrink_count(5);
        assert!(s.is_empty());
        assert!(matches(&s, &ItemStack::empty()));
        s.grow_count(3);
        assert_eq!((s.count(), s.item_name()), (3, "minecraft:stone"));
    }

    #[test]
    fn split_and_copy() {
        let mut s = stone(10);
        let part = s.split_count(4);
        assert_eq!((s.count(), part.count()), (6, 4));
        let mut e = stone(0);
        assert!(e.split_count(3).is_empty());
        assert!(e.copy().item() == kiln_item::stack::air());
        assert!(!same_item(&stone(1), &ItemStack::of("dirt", 1).unwrap()));
    }
}
