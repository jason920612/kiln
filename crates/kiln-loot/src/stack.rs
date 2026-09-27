//! `ItemStack` operations loot needs beyond kiln-item's own.

use crate::context::LootContext;
use crate::function::{ContainerKind, MergeStrategy};
use kiln_command::nbt_path::{NbtPath, Node};
use kiln_item::component::{BundleContents, ChargedProjectiles, CustomData, ItemContainerContents, ids, keys};
use kiln_item::{DataComponentPatch, ItemStack, ItemStackTemplate};
use kiln_proto::nbt::Tag;

/// `ItemStack.transmuteCopy(item)`: the same count and component patch on another item.
pub fn transmute_copy(stack: &ItemStack, item: i32) -> ItemStack {
    if stack.is_empty() {
        return ItemStack::empty();
    }
    ItemStack::from_parts(item, stack.count(), stack.patch().clone())
}

/// `ItemStack.limitSize`.
pub fn limit_size(stack: &mut ItemStack, max: i32) {
    if !stack.is_empty() && stack.count() > max {
        stack.set_count(max);
    }
}

/// `ItemStack.applyComponents(DataComponentPatch)`.
pub fn apply_patch(stack: &mut ItemStack, patch: &DataComponentPatch) {
    for (id, value) in patch.iter() {
        match value {
            Some(c) => stack.set(c.clone()),
            None => stack.remove(id),
        }
    }
}

/// `CompoundTag.merge`: compounds merge recursively, anything else replaces.
pub fn merge_compound(target: &mut Tag, source: &Tag) {
    let (Tag::Compound(t), Tag::Compound(s)) = (target, source) else { return };
    for (k, v) in s {
        match t.iter_mut().find(|(tk, _)| tk == k) {
            Some((_, existing @ Tag::Compound(_))) if matches!(v, Tag::Compound(_)) => merge_compound(existing, v),
            Some((_, existing)) => *existing = v.clone(),
            None => t.push((k.clone(), v.clone())),
        }
    }
}

/// `CustomData.set`: an empty compound removes the component.
pub fn set_custom_data(stack: &mut ItemStack, tag: Tag) {
    match &tag {
        Tag::Compound(fields) if fields.is_empty() => stack.remove(ids::CUSTOM_DATA),
        _ => stack.insert(keys::CUSTOM_DATA, CustomData(tag)),
    }
}

/// `LootTable.createStackSplitter`: disabled items are dropped, oversized stacks are split
/// into stacks of at most their maximum size.
pub fn split_stack(ctx: &dyn LootContext, stack: ItemStack, sink: &mut dyn FnMut(ItemStack)) {
    if !stack.is_empty() && !ctx.item_enabled(stack.item()) {
        return;
    }
    let max = stack.max_stack_size();
    if stack.count() < max {
        sink(stack);
        return;
    }
    let mut remaining = stack.count();
    while remaining > 0 {
        let part = stack.with_count(max.min(remaining));
        remaining -= part.count();
        sink(part);
    }
}

/// The items of a container component (`ContainerComponentManipulator`), slot by slot;
/// `None` when the stack does not have the component.
pub fn container_contents(stack: &ItemStack, kind: ContainerKind) -> Option<Vec<Option<ItemStack>>> {
    let create = |t: &ItemStackTemplate| Some(t.create());
    Some(match kind {
        ContainerKind::Container => stack.get(keys::CONTAINER)?.0.iter().map(|s| s.as_ref().and_then(create)).collect(),
        ContainerKind::BundleContents => stack.get(keys::BUNDLE_CONTENTS)?.0.iter().map(create).collect(),
        ContainerKind::ChargedProjectiles => stack.get(keys::CHARGED_PROJECTILES)?.0.iter().map(create).collect(),
    })
}

fn template(stack: &ItemStack) -> Option<ItemStackTemplate> {
    (!stack.is_empty()).then(|| ItemStackTemplate::from_stack(stack))
}

/// `ContainerComponentManipulator.setContents`.
pub fn set_container_contents(stack: &mut ItemStack, kind: ContainerKind, items: Vec<ItemStack>) {
    match kind {
        ContainerKind::Container => {
            // `ItemContainerContents.fromItems`: slots up to the last non-empty one.
            let last = items.iter().rposition(|s| !s.is_empty()).map_or(0, |i| i + 1);
            let slots = items[..last.min(256)].iter().map(template).collect();
            stack.insert(keys::CONTAINER, ItemContainerContents(slots));
        }
        ContainerKind::BundleContents => {
            stack.insert(keys::BUNDLE_CONTENTS, BundleContents(items.iter().filter_map(template).collect()));
        }
        ContainerKind::ChargedProjectiles => {
            stack.insert(keys::CHARGED_PROJECTILES, ChargedProjectiles(items.iter().filter_map(template).collect()));
        }
    }
}

/// `ContainerComponentManipulator.modifyItems` after the modifier ran on each item.
pub fn replace_container_contents(stack: &mut ItemStack, kind: ContainerKind, items: Vec<Option<ItemStack>>) {
    match kind {
        ContainerKind::Container => {
            let slots = items.iter().map(|s| s.as_ref().and_then(template)).collect();
            stack.insert(keys::CONTAINER, ItemContainerContents(slots));
        }
        _ => set_container_contents(stack, kind, items.into_iter().flatten().collect()),
    }
}

/// Applies one `copy_custom_data` operation to the target compound.
pub fn apply_copy(target: &mut Tag, path: &NbtPath, op: MergeStrategy, values: &[Tag]) {
    let Some(last) = values.last() else { return };
    match op {
        MergeStrategy::Replace => {
            let _ = path.set(target, last);
        }
        MergeStrategy::Append => {
            if let Some(Tag::List(list)) = get_or_create(target, path, || Tag::List(Vec::new())) {
                list.extend(values.iter().cloned());
            }
        }
        MergeStrategy::Merge => {
            if let Some(slot @ Tag::Compound(_)) = get_or_create(target, path, || Tag::Compound(Vec::new())) {
                merge_compound(slot, last);
            }
        }
    }
}

/// `NbtPath.getOrCreate` for paths of plain child names (other nodes are not supported).
fn get_or_create<'t>(root: &'t mut Tag, path: &NbtPath, make: impl Fn() -> Tag) -> Option<&'t mut Tag> {
    let mut current = root;
    let n = path.nodes.len();
    for (i, node) in path.nodes.iter().enumerate() {
        let Node::Child(name) = node else { return None };
        let Tag::Compound(fields) = current else { return None };
        let idx = match fields.iter().position(|(k, _)| k == name) {
            Some(idx) => idx,
            None => {
                fields.push((name.clone(), if i + 1 == n { make() } else { Tag::Compound(Vec::new()) }));
                fields.len() - 1
            }
        };
        current = &mut fields[idx].1;
    }
    Some(current)
}
