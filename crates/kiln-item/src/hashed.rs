//! `HashedStack`: how the client describes a slot's new contents in `container_click` (the
//! item, count and a hash per changed component instead of the component values).

use crate::component::ComponentId;
use crate::hash;
use crate::registry;
use crate::stack::ItemStack;
use crate::wire::{self, WireResult};
use bytes::{BufMut, BytesMut};
use kiln_proto::{Reader, WriteExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashedStack {
    Empty,
    Item {
        item: i32,
        count: i32,
        /// Added components with their hashes (`HashedPatchMap.addedComponents`).
        added: Vec<(ComponentId, i32)>,
        removed: Vec<ComponentId>,
    },
}

/// At most 256 added and 256 removed components.
const MAX_ENTRIES: usize = 256;

impl HashedStack {
    /// The hashed form of `stack` (`HashedStack.create`); `None` if its patch has a transient
    /// component, which vanilla cannot hash either.
    pub fn of(stack: &ItemStack) -> Option<HashedStack> {
        if stack.is_empty() {
            return Some(HashedStack::Empty);
        }
        let mut added = Vec::new();
        for c in stack.patch().added() {
            added.push((c.id(), hash::hash(&c.to_value()?)));
        }
        Some(HashedStack::Item { item: stack.item(), count: stack.count(), added, removed: stack.patch().removed().collect() })
    }

    /// `HashedStack.STREAM_CODEC`: an optional (item id, VarInt count, hashed patch).
    pub fn read(r: &mut Reader<'_>) -> WireResult<HashedStack> {
        if !r.bool()? {
            return Ok(HashedStack::Empty);
        }
        let item = registry::ITEM.read_id(r)?;
        let count = r.varint()?;
        let added = wire::read_list(r, MAX_ENTRIES, |r| Ok((read_type(r)?, r.i32()?)))?;
        let removed = wire::read_list(r, MAX_ENTRIES, read_type)?;
        Ok(HashedStack::Item { item, count, added, removed })
    }

    pub fn write(&self, out: &mut BytesMut) {
        match self {
            HashedStack::Empty => out.put_bool(false),
            HashedStack::Item { item, count, added, removed } => {
                out.put_bool(true);
                out.put_varint(*item);
                out.put_varint(*count);
                wire::write_list(out, added, |(id, h), o| {
                    o.put_varint(*id as i32);
                    o.put_i32(*h);
                });
                wire::write_list(out, removed, |id, o| o.put_varint(*id as i32));
            }
        }
    }

    /// Whether `stack` is what the client described (`HashedStack.matches`).
    pub fn matches(&self, stack: &ItemStack) -> bool {
        match self {
            HashedStack::Empty => stack.is_empty(),
            HashedStack::Item { item, count, added, removed } => {
                if stack.is_empty() || *count != stack.count() || *item != stack.item() {
                    return false;
                }
                let patch = stack.patch();
                let mut theirs: Vec<ComponentId> = removed.clone();
                let mut ours: Vec<ComponentId> = patch.removed().collect();
                theirs.sort_unstable();
                theirs.dedup();
                ours.sort_unstable();
                if theirs != ours || patch.added().count() != added.len() {
                    return false;
                }
                patch.added().all(|c| {
                    let Some(&(_, expected)) = added.iter().find(|(id, _)| *id == c.id()) else { return false };
                    c.to_value().is_some_and(|v| hash::hash(&v) == expected)
                })
            }
        }
    }
}

fn read_type(r: &mut Reader<'_>) -> WireResult<ComponentId> {
    registry::DATA_COMPONENT_TYPE.read_id(r).map(|i| i as ComponentId)
}
