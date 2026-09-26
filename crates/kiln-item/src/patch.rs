//! `DataComponentPatch`: components added to or removed from an item's defaults.

use crate::component::{self, Component, ComponentId};
use crate::registry;
use crate::value::{DataError, DataResult, Value};
use crate::wire::WireResult;
use bytes::BytesMut;
use kiln_proto::{DecodeError, Reader, WriteExt};

/// Entries in insertion order (vanilla's `Reference2ObjectArrayMap`, whose order is the
/// network order): a value, or `None` for a removed component type.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataComponentPatch {
    entries: Vec<(ComponentId, Option<Component>)>,
}

impl DataComponentPatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `Some(Some(value))` if set, `Some(None)` if removed, `None` if not in the patch.
    pub fn get(&self, id: ComponentId) -> Option<Option<&Component>> {
        self.entries.iter().find(|(t, _)| *t == id).map(|(_, v)| v.as_ref())
    }

    /// Sets a value, replacing an existing entry for its type in place.
    pub fn set(&mut self, value: Component) {
        self.put(value.id(), Some(value));
    }

    /// Marks a component type as removed.
    pub fn remove(&mut self, id: ComponentId) {
        self.put(id, None);
    }

    /// Drops the entry for `id` (neither set nor removed afterwards).
    pub fn forget(&mut self, id: ComponentId) {
        self.entries.retain(|(t, _)| *t != id);
    }

    fn put(&mut self, id: ComponentId, value: Option<Component>) {
        match self.entries.iter_mut().find(|(t, _)| *t == id) {
            Some(slot) => slot.1 = value,
            None => self.entries.push((id, value)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ComponentId, Option<&Component>)> {
        self.entries.iter().map(|(t, v)| (*t, v.as_ref()))
    }

    pub fn added(&self) -> impl Iterator<Item = &Component> {
        self.entries.iter().filter_map(|(_, v)| v.as_ref())
    }

    pub fn removed(&self) -> impl Iterator<Item = ComponentId> + '_ {
        self.entries.iter().filter(|(_, v)| v.is_none()).map(|(t, _)| *t)
    }

    /// Equality as maps (vanilla's `DataComponentPatch.equals`), ignoring entry order.
    pub fn same_entries(&self, other: &DataComponentPatch) -> bool {
        self.len() == other.len() && self.entries.iter().all(|(t, v)| other.get(*t) == Some(v.as_ref()))
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(ComponentId, Option<&Component>) -> bool) {
        self.entries.retain(|(t, v)| keep(*t, v.as_ref()));
    }

    /// `DataComponentPatch.STREAM_CODEC`: counts of added and removed entries, the added
    /// (type, value) pairs, then the removed types.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Self::read_with(r, false)
    }

    /// `DELIMITED_STREAM_CODEC` (untrusted stacks): each value is prefixed with its length.
    pub fn read_delimited(r: &mut Reader<'_>) -> WireResult<Self> {
        Self::read_with(r, true)
    }

    fn read_with(r: &mut Reader<'_>, delimited: bool) -> WireResult<Self> {
        let adds = r.varint()?;
        let removes = r.varint()?;
        let (Ok(adds), Ok(removes)) = (usize::try_from(adds), usize::try_from(removes)) else {
            return Err(DecodeError::Invalid("negative component count"));
        };
        if adds.saturating_add(removes) > r.remaining() {
            return Err(DecodeError::Eof);
        }
        let mut patch = DataComponentPatch { entries: Vec::with_capacity(adds + removes) };
        for _ in 0..adds {
            let id = read_type(r)?;
            let value = if delimited {
                let len = r.len()?;
                let mut sub = Reader::new(r.bytes(len)?);
                Component::read(id, &mut sub)?
            } else {
                Component::read(id, r)?
            };
            patch.put(id, Some(value));
        }
        for _ in 0..removes {
            let id = read_type(r)?;
            patch.put(id, None);
        }
        Ok(patch)
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.write_with(out, false);
    }

    pub fn write_delimited(&self, out: &mut BytesMut) {
        self.write_with(out, true);
    }

    fn write_with(&self, out: &mut BytesMut, delimited: bool) {
        out.put_varint(self.added().count() as i32);
        out.put_varint(self.removed().count() as i32);
        for c in self.added() {
            out.put_varint(c.id() as i32);
            if delimited {
                let mut buf = BytesMut::new();
                c.write(&mut buf);
                out.put_varint(buf.len() as i32);
                out.extend_from_slice(&buf);
            } else {
                c.write(out);
            }
        }
        for id in self.removed() {
            out.put_varint(id as i32);
        }
    }

    /// `DataComponentPatch.CODEC`: `{"minecraft:type": value, "!minecraft:removed": {}}`;
    /// transient components are left out.
    pub fn to_value(&self) -> Value {
        Value::Map(
            self.entries
                .iter()
                .filter(|(t, _)| component::is_persistent(*t))
                .map(|(t, v)| match v {
                    Some(c) => (Value::str(component::name(*t)), c.to_value().unwrap_or(Value::Empty)),
                    None => (Value::String(format!("!{}", component::name(*t))), Value::empty_map()),
                })
                .collect(),
        )
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let mut patch = DataComponentPatch::new();
        for (k, v) in v.as_map()?.entries() {
            let key = k.as_str()?;
            let (removed, name) = match key.strip_prefix('!') {
                Some(n) => (true, n),
                None => (false, key),
            };
            let id = component::by_name(name).ok_or_else(|| DataError(format!("unknown component type {key:?}")))?;
            if !component::is_persistent(id) {
                return Err(DataError(format!("component type {key:?} is not persistent")));
            }
            if removed {
                patch.put(id, None);
            } else {
                let c = Component::from_value(id, v).map_err(|e| DataError(format!("{name}: {e}")))?;
                patch.put(id, Some(c));
            }
        }
        Ok(patch)
    }
}

fn read_type(r: &mut Reader<'_>) -> WireResult<ComponentId> {
    registry::DATA_COMPONENT_TYPE.read_id(r).map(|i| i as ComponentId)
}
