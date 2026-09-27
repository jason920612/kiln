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

/// `ContainerComponent.itemCopies()`: the component's items slot by slot (empty stacks for
/// empty container slots); `None` when the stack does not have the component.
pub fn container_contents(stack: &ItemStack, kind: ContainerKind) -> Option<Vec<ItemStack>> {
    Some(match kind {
        ContainerKind::Container => {
            stack.get(keys::CONTAINER)?.0.iter().map(|s| s.as_ref().map_or_else(ItemStack::empty, ItemStackTemplate::create)).collect()
        }
        ContainerKind::BundleContents => stack.get(keys::BUNDLE_CONTENTS)?.0.iter().map(ItemStackTemplate::create).collect(),
        ContainerKind::ChargedProjectiles => stack.get(keys::CHARGED_PROJECTILES)?.0.iter().map(ItemStackTemplate::create).collect(),
    })
}

fn template(stack: &ItemStack) -> Option<ItemStackTemplate> {
    (!stack.is_empty()).then(|| ItemStackTemplate::from_stack(stack))
}

/// `ContainerComponentManipulator.setContents`: the component rebuilt from `items`
/// (`copyWithContents`).
pub fn set_container_contents(stack: &mut ItemStack, kind: ContainerKind, items: Vec<ItemStack>) {
    match kind {
        ContainerKind::Container => {
            // `ItemContainerContents.fromItems`: slots up to the last non-empty one.
            let last = items.iter().rposition(|s| !s.is_empty()).map_or(0, |i| i + 1);
            let slots = items[..last].iter().map(template).collect();
            stack.insert(keys::CONTAINER, ItemContainerContents(slots));
        }
        ContainerKind::BundleContents => {
            let mut bundle = BundleBuilder { items: Vec::new(), weight: Frac::ZERO };
            for mut item in items {
                bundle.try_insert(&mut item);
            }
            stack.insert(keys::BUNDLE_CONTENTS, BundleContents(bundle.items.iter().filter_map(template).collect()));
        }
        ContainerKind::ChargedProjectiles => {
            stack.insert(keys::CHARGED_PROJECTILES, ChargedProjectiles(items.iter().filter_map(template).collect()));
        }
    }
}

/// `org.apache.commons.lang3.math.Fraction` in lowest terms (`None`: int overflow, which
/// vanilla reports as an `ArithmeticException`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frac {
    num: i64,
    den: i64,
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

impl Frac {
    const ZERO: Frac = Frac { num: 0, den: 1 };
    const ONE: Frac = Frac { num: 1, den: 1 };

    fn new(num: i64, den: i64) -> Option<Frac> {
        if den == 0 {
            return None;
        }
        let g = gcd(num, den).max(1);
        let (mut num, mut den) = (num / g, den / g);
        if den < 0 {
            num = -num;
            den = -den;
        }
        (i32::try_from(num).is_ok() && i32::try_from(den).is_ok()).then_some(Frac { num, den })
    }

    fn add(self, o: Frac) -> Option<Frac> {
        Frac::new(self.num * o.den + o.num * self.den, self.den * o.den)
    }

    fn sub(self, o: Frac) -> Option<Frac> {
        Frac::new(self.num * o.den - o.num * self.den, self.den * o.den)
    }

    fn mul(self, n: i64) -> Option<Frac> {
        Frac::new(self.num * n, self.den)
    }

    fn div(self, o: Frac) -> Option<Frac> {
        Frac::new(self.num * o.den, self.den * o.num)
    }
}

/// `BundleContents.getWeight`: a nested bundle weighs its contents plus 1/16, a beehive with bees
/// a whole bundle, anything else one over its maximum stack size.
fn bundle_weight(stack: &ItemStack) -> Option<Frac> {
    if let Some(contents) = stack.get(keys::BUNDLE_CONTENTS) {
        let inner = contents.0.iter().try_fold(Frac::ZERO, |acc, t| acc.add(bundle_weight(&t.create())?.mul(t.count as i64)?))?;
        return inner.add(Frac::new(1, 16)?);
    }
    if stack.get(keys::BEES).is_some_and(|b| !b.0.is_empty()) {
        return Some(Frac::ONE);
    }
    Frac::new(1, stack.max_stack_size() as i64)
}

/// `BundleContents.Mutable` started empty, as `copyWithContents` does.
struct BundleBuilder {
    items: Vec<ItemStack>,
    weight: Frac,
}

impl BundleBuilder {
    /// `Mutable.tryInsert`: what fits goes in front (merged with an equal stack).
    fn try_insert(&mut self, stack: &mut ItemStack) {
        // `canItemBeInBundle` (`Item.canFitInsideContainerItems`: not shulker boxes).
        if stack.is_empty() || stack.item_name().ends_with("shulker_box") {
            return;
        }
        let Some(w) = bundle_weight(stack) else { return };
        let max = Frac::ONE.sub(self.weight).and_then(|free| free.div(w)).map_or(0, |f| (f.num / f.den).max(0) as i32);
        let n = stack.count().min(max);
        if n == 0 {
            return;
        }
        let Some(new_weight) = w.mul(n as i64).and_then(|a| self.weight.add(a)) else { return };
        self.weight = new_weight;
        let found = if stack.is_stackable() { self.items.iter().position(|s| s.is_same_item_same_components(stack)) } else { None };
        match found {
            Some(i) => {
                let old = self.items.remove(i);
                let merged = old.with_count(old.count() + n);
                stack.set_count(stack.count() - n);
                self.items.insert(0, merged);
            }
            None => {
                let part = stack.split(n);
                self.items.insert(0, part);
            }
        }
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
