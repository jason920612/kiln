//! `ItemStack`: an item, a count and a component patch over the item's defaults.

use crate::component::{Component, ComponentId, Key, ids, keys};
use crate::defaults::{ComponentMap, default_components};
use crate::hash;
use crate::patch::DataComponentPatch;
use crate::registry;
use crate::value::{DataError, DataResult, MapBuilder, Value};
use crate::wire::WireResult;
use bytes::BytesMut;
use kiln_proto::nbt::Tag;
use kiln_proto::{DecodeError, Reader, WriteExt};
use std::sync::OnceLock;

/// Network id of `minecraft:air`.
pub fn air() -> i32 {
    static AIR: OnceLock<i32> = OnceLock::new();
    *AIR.get_or_init(|| registry::ITEM.id("minecraft:air").expect("minecraft:air"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    item: i32,
    count: i32,
    patch: DataComponentPatch,
}

impl Default for ItemStack {
    fn default() -> Self {
        Self::empty()
    }
}

impl ItemStack {
    pub fn empty() -> ItemStack {
        ItemStack { item: air(), count: 0, patch: DataComponentPatch::new() }
    }

    /// `count` of `item` (a `minecraft:item` network id) with default components.
    pub fn new(item: i32, count: i32) -> ItemStack {
        ItemStack { item, count, patch: DataComponentPatch::new() }
    }

    /// A stack of the item named `name`, if it exists.
    pub fn of(name: &str, count: i32) -> Option<ItemStack> {
        registry::ITEM.id(name).map(|id| ItemStack::new(id, count))
    }

    /// A stack with `patch` applied the way vanilla constructs one (`PatchedDataComponentMap
    /// .fromPatch`): entries equal to the default value, and removals of components the item
    /// does not have, are dropped.
    pub fn from_parts(item: i32, count: i32, mut patch: DataComponentPatch) -> ItemStack {
        let defaults = default_components(item);
        patch.retain(|id, value| match value {
            Some(v) => defaults.get(id) != Some(v),
            None => defaults.contains(id),
        });
        ItemStack { item, count, patch }
    }

    /// `ItemStack.isEmpty()`: air, or a count below one.
    pub fn is_empty(&self) -> bool {
        self.item == air() || self.count <= 0
    }

    /// The item's network id.
    pub fn item(&self) -> i32 {
        self.item
    }

    /// `minecraft:...` name of the item.
    pub fn item_name(&self) -> &'static str {
        registry::ITEM.name(self.item).unwrap_or("minecraft:air")
    }

    pub fn count(&self) -> i32 {
        if self.is_empty() { 0 } else { self.count }
    }

    pub fn set_count(&mut self, count: i32) {
        self.count = count;
    }

    pub fn with_count(&self, count: i32) -> ItemStack {
        if self.is_empty() {
            return ItemStack::empty();
        }
        ItemStack { count, ..self.clone() }
    }

    pub fn grow(&mut self, n: i32) {
        self.count += n;
    }

    pub fn shrink(&mut self, n: i32) {
        self.count -= n;
    }

    /// Takes up to `n` items off this stack (`ItemStack.split`).
    pub fn split(&mut self, n: i32) -> ItemStack {
        let taken = n.min(self.count());
        let part = self.with_count(taken);
        self.shrink(taken);
        part
    }

    pub fn patch(&self) -> &DataComponentPatch {
        &self.patch
    }

    pub fn defaults(&self) -> &'static ComponentMap {
        default_components(self.item)
    }

    /// The effective value of a component type: the patch's, else the item's default.
    pub fn component(&self, id: ComponentId) -> Option<&Component> {
        if self.is_empty() {
            return None;
        }
        match self.patch.get(id) {
            Some(v) => v,
            None => self.defaults().get(id),
        }
    }

    /// Typed access: `stack.get(keys::DAMAGE)`.
    pub fn get<T>(&self, key: Key<T>) -> Option<&T> {
        self.component(key.id).and_then(|c| key.get(c))
    }

    pub fn has(&self, id: ComponentId) -> bool {
        self.component(id).is_some()
    }

    /// Sets a component; a value equal to the default clears the patch entry instead
    /// (`PatchedDataComponentMap.set`).
    pub fn set(&mut self, value: Component) {
        if self.defaults().get(value.id()) == Some(&value) {
            self.patch.forget(value.id());
        } else {
            self.patch.set(value);
        }
    }

    /// `set` for a typed key: `stack.insert(keys::DAMAGE, 3)`.
    pub fn insert<T>(&mut self, key: Key<T>, value: T) {
        self.set(key.wrap(value));
    }

    /// Removes a component (`PatchedDataComponentMap.remove`).
    pub fn remove(&mut self, id: ComponentId) {
        if self.defaults().contains(id) {
            self.patch.remove(id);
        } else {
            self.patch.forget(id);
        }
    }

    /// `max_stack_size`, 1 if absent.
    pub fn max_stack_size(&self) -> i32 {
        self.get(keys::MAX_STACK_SIZE).copied().unwrap_or(1)
    }

    /// `ItemStack.isStackable`.
    pub fn is_stackable(&self) -> bool {
        self.max_stack_size() > 1 && (!self.is_damageable_item() || !self.is_damaged())
    }

    /// `ItemStack.isDamageableItem`: has `max_damage` and `damage` and is not unbreakable.
    pub fn is_damageable_item(&self) -> bool {
        self.has(ids::MAX_DAMAGE) && !self.has(ids::UNBREAKABLE) && self.has(ids::DAMAGE)
    }

    pub fn is_damaged(&self) -> bool {
        self.is_damageable_item() && self.damage() > 0
    }

    pub fn max_damage(&self) -> i32 {
        self.get(keys::MAX_DAMAGE).copied().unwrap_or(0)
    }

    pub fn damage(&self) -> i32 {
        self.get(keys::DAMAGE).copied().unwrap_or(0).clamp(0, self.max_damage().max(0))
    }

    pub fn is_same_item(&self, other: &ItemStack) -> bool {
        self.item == other.item
    }

    /// `ItemStack.isSameItemSameComponents`.
    pub fn is_same_item_same_components(&self, other: &ItemStack) -> bool {
        self.item == other.item && ((self.is_empty() && other.is_empty()) || self.patch.same_entries(&other.patch))
    }

    // ---- network codecs ----

    /// `ItemStack.OPTIONAL_STREAM_CODEC`: VarInt count (0 = empty), item id, component patch.
    pub fn read_optional(r: &mut Reader<'_>) -> WireResult<ItemStack> {
        Self::read_with(r, false)
    }

    /// `ItemStack.STREAM_CODEC`: as optional, but empty stacks are rejected.
    pub fn read(r: &mut Reader<'_>) -> WireResult<ItemStack> {
        let s = Self::read_optional(r)?;
        if s.is_empty() {
            return Err(DecodeError::Invalid("empty item stack not allowed"));
        }
        Ok(s)
    }

    /// `ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC` (creative slots): component values are
    /// length-prefixed.
    pub fn read_untrusted_optional(r: &mut Reader<'_>) -> WireResult<ItemStack> {
        Self::read_with(r, true)
    }

    fn read_with(r: &mut Reader<'_>, delimited: bool) -> WireResult<ItemStack> {
        let count = r.varint()?;
        if count <= 0 {
            return Ok(ItemStack::empty());
        }
        let item = registry::ITEM.read_id(r)?;
        let patch = if delimited { DataComponentPatch::read_delimited(r)? } else { DataComponentPatch::read(r)? };
        Ok(ItemStack::from_parts(item, count, patch))
    }

    pub fn write_optional(&self, out: &mut BytesMut) {
        self.write_with(out, false);
    }

    /// `ItemStack.STREAM_CODEC`; the stack must not be empty (vanilla throws).
    pub fn write(&self, out: &mut BytesMut) {
        debug_assert!(!self.is_empty(), "empty item stack not allowed");
        self.write_with(out, false);
    }

    pub fn write_untrusted_optional(&self, out: &mut BytesMut) {
        self.write_with(out, true);
    }

    fn write_with(&self, out: &mut BytesMut, delimited: bool) {
        if self.is_empty() {
            out.put_varint(0);
            return;
        }
        out.put_varint(self.count);
        out.put_varint(self.item);
        if delimited { self.patch.write_delimited(out) } else { self.patch.write(out) }
    }

    // ---- persistent codec ----

    /// `ItemStack.CODEC`: `{id, count, components}` (an empty stack has no persistent form;
    /// see [`ItemStack::to_value_optional`]).
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("id", Value::str(self.item_name())).put("count", Value::Int(self.count));
        if !self.patch.is_empty() {
            m.put("components", self.patch.to_value());
        }
        m.build()
    }

    pub fn from_value(v: &Value) -> DataResult<ItemStack> {
        let m = v.as_map()?;
        let item = m.req_with("id", |v| registry::ITEM.id_from_value(v))?;
        let count = m.opt_or("count", 1, Value::as_i32)?;
        if !(1..=99).contains(&count) {
            return Err(DataError(format!("count {count} out of range [1;99]")));
        }
        let patch = m.opt_or("components", DataComponentPatch::new(), DataComponentPatch::from_value)?;
        Ok(ItemStack::from_parts(item, count, patch))
    }

    /// `ItemStack.OPTIONAL_CODEC`: `{}` for an empty stack.
    pub fn to_value_optional(&self) -> Value {
        if self.is_empty() { Value::empty_map() } else { self.to_value() }
    }

    pub fn from_value_optional(v: &Value) -> DataResult<ItemStack> {
        match v {
            Value::Map(m) if m.is_empty() => Ok(ItemStack::empty()),
            _ => ItemStack::from_value(v),
        }
    }

    /// The stack as saved in playerdata and chunks.
    pub fn to_nbt(&self) -> Tag {
        self.to_value().to_nbt()
    }

    pub fn from_nbt(tag: &Tag) -> DataResult<ItemStack> {
        ItemStack::from_value(&Value::from_nbt(tag))
    }

    // ---- hashing ----

    /// The `container_click` hash of every added persistent component, in patch order.
    pub fn component_hashes(&self) -> Vec<(ComponentId, i32)> {
        self.patch.added().filter_map(|c| c.to_value().map(|v| (c.id(), hash::hash(&v)))).collect()
    }
}

/// `ItemStackTemplate`: a non-empty item, count and patch that is not normalized against the
/// item's defaults (used by containers, bundles, crossbows, use remainders, hover events).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStackTemplate {
    pub item: i32,
    pub count: i32,
    pub patch: DataComponentPatch,
}

impl ItemStackTemplate {
    pub fn new(item: i32, count: i32) -> Self {
        ItemStackTemplate { item, count, patch: DataComponentPatch::new() }
    }

    /// The template of a non-empty stack (`ItemStackTemplate.fromNonEmptyStack`).
    pub fn from_stack(stack: &ItemStack) -> Self {
        ItemStackTemplate { item: stack.item, count: stack.count, patch: stack.patch.clone() }
    }

    /// The stack this template creates (vanilla also validates it).
    pub fn create(&self) -> ItemStack {
        ItemStack::from_parts(self.item, self.count, self.patch.clone())
    }

    /// `ItemStackTemplate.STREAM_CODEC`: item id, VarInt count, component patch.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let item = registry::ITEM.read_id(r)?;
        if item == air() {
            return Err(DecodeError::Invalid("item stack template of air"));
        }
        Ok(ItemStackTemplate { item, count: r.varint()?, patch: DataComponentPatch::read(r)? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.item);
        out.put_varint(self.count);
        self.patch.write(out);
    }

    /// `ItemStackTemplate.CODEC` / `MAP_CODEC`: `{id, count (omitted when 1), components
    /// (omitted when empty)}`.
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("id", registry::ITEM.id_to_value(self.item)).opt_default("count", self.count, 1, Value::Int);
        if !self.patch.is_empty() {
            m.put("components", self.patch.to_value());
        }
        m.build()
    }

    /// Also accepts a bare item id (`CODEC`'s alternative form).
    pub fn from_value(v: &Value) -> DataResult<Self> {
        if let Value::String(_) = v {
            return Self::checked(registry::ITEM.id_from_value(v)?, 1, DataComponentPatch::new());
        }
        let m = v.as_map()?;
        let item = m.req_with("id", |v| registry::ITEM.id_from_value(v))?;
        let count = m.opt_or("count", 1, Value::as_i32)?;
        if !(1..=99).contains(&count) {
            return Err(DataError(format!("count {count} out of range [1;99]")));
        }
        Self::checked(item, count, m.opt_or("components", DataComponentPatch::new(), DataComponentPatch::from_value)?)
    }

    fn checked(item: i32, count: i32, patch: DataComponentPatch) -> DataResult<Self> {
        if item == air() {
            return Err(DataError("item must be non-empty".into()));
        }
        Ok(ItemStackTemplate { item, count, patch })
    }
}
