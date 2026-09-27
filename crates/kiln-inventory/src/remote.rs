//! What the client is believed to hold (`RemoteSlot.Synchronized`), compared against the
//! server's stacks to decide which slot updates to send.

use crate::stack::{StackExt, matches};
use kiln_item::{HashedStack, ItemStack, hash};

/// `RemoteSlot.Synchronized`: either a stack the server sent (or confirmed), or the hash the
/// client reported in `container_click`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteSlot {
    stack: Option<ItemStack>,
    hash: Option<HashedStack>,
}

impl RemoteSlot {
    /// `force`: the client now holds `stack` (the server sent it).
    pub fn force(&mut self, stack: &ItemStack) {
        self.stack = Some(stack.copy());
        self.hash = None;
    }

    /// `receive`: the client reported this hash.
    pub fn receive(&mut self, hash: HashedStack) {
        self.stack = None;
        self.hash = Some(hash);
    }

    /// `matches`: whether the client already has `stack`; a matching hash is remembered as the
    /// stack itself.
    pub fn matches(&mut self, stack: &ItemStack) -> bool {
        if let Some(remote) = &self.stack {
            return matches(remote, stack);
        }
        if self.hash.as_ref().is_some_and(|h| hashed_matches(h, stack)) {
            self.stack = Some(stack.copy());
            return true;
        }
        false
    }
}

/// `HashedStack.matches` with vanilla's hash generator. A transient component cannot be hashed
/// (vanilla's generator throws); it never matches here.
pub fn hashed_matches(hashed: &HashedStack, stack: &ItemStack) -> bool {
    match hashed {
        HashedStack::Empty => stack.is_empty(),
        HashedStack::Item { item, count, added, removed } => {
            if *count != stack.count() || *item != stack.effective_item() {
                return false;
            }
            let empty = kiln_item::DataComponentPatch::new();
            let patch = if stack.is_empty() { &empty } else { stack.patch() };
            let mut theirs = removed.clone();
            theirs.sort_unstable();
            theirs.dedup();
            let mut ours: Vec<_> = patch.removed().collect();
            ours.sort_unstable();
            if theirs != ours {
                return false;
            }
            // A map on the wire: a repeated type keeps its last hash.
            let expected: std::collections::BTreeMap<_, _> = added.iter().copied().collect();
            if expected.len() != patch.added().count() {
                return false;
            }
            patch.added().all(|c| {
                let Some(&h) = expected.get(&c.id()) else { return false };
                c.to_value().is_some_and(|v| hash(&v) == h)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_item::keys;

    #[test]
    fn hash_then_confirm() {
        let mut sword = ItemStack::of("diamond_sword", 1).unwrap();
        sword.insert(keys::DAMAGE, 4);
        let mut r = RemoteSlot::default();
        assert!(!r.matches(&sword), "unknown remote state never matches");
        r.receive(HashedStack::of(&sword).unwrap());
        assert!(r.matches(&sword));
        let mut other = sword.clone();
        other.insert(keys::DAMAGE, 5);
        assert!(!r.matches(&other));
        r.force(&ItemStack::empty());
        assert!(r.matches(&ItemStack::of("stone", 0).unwrap()));
    }
}
