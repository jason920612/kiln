//! `DataComponentPatch`: components added to or removed from an item's defaults.

use crate::component::{self, Component, ComponentId};
use crate::registry;
use crate::value::{DataError, DataResult, Value};
use crate::wire::WireResult;
use bytes::BytesMut;
use kiln_proto::{DecodeError, Reader, WriteExt};

/// Entries in insertion order (vanilla's `Reference2ObjectArrayMap`, whose order is the
/// network order): a value, or `None` for a removed component type.
///
/// A patch read from NBT also keeps the entries it could not decode (unknown component types,
/// values that don't match the schema) verbatim, so that saving it again loses nothing. They
/// are not sent to clients.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataComponentPatch {
    entries: Vec<(ComponentId, Option<Component>)>,
    unparsed: Vec<(String, Value)>,
}

impl DataComponentPatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.unparsed.is_empty()
    }

    /// Number of decoded entries (set or removed).
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
        let name = component::name(id);
        self.unparsed.retain(|(k, _)| k.strip_prefix('!').unwrap_or(k) != name);
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

    /// Entries from NBT that could not be decoded, as (key, value) (`"!name"` for removals).
    pub fn unparsed(&self) -> &[(String, Value)] {
        &self.unparsed
    }

    /// Equality as maps (vanilla's `DataComponentPatch.equals`), ignoring entry order.
    pub fn same_entries(&self, other: &DataComponentPatch) -> bool {
        self.len() == other.len()
            && self.unparsed.len() == other.unparsed.len()
            && self.entries.iter().all(|(t, v)| other.get(*t) == Some(v.as_ref()))
            && self.unparsed.iter().all(|e| other.unparsed.contains(e))
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
        let mut patch = DataComponentPatch { entries: Vec::with_capacity(adds + removes), unparsed: Vec::new() };
        for _ in 0..adds {
            let id = read_type(r)?;
            let value = if delimited {
                let len = r.len()?;
                Component::read(id, &mut Reader::new(r.bytes(len)?))?
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
        let mut entries: Vec<(Value, Value)> = self
            .entries
            .iter()
            .filter(|(t, _)| component::is_persistent(*t))
            .map(|(t, v)| match v {
                Some(c) => (Value::str(component::name(*t)), c.to_value().unwrap_or(Value::Empty)),
                None => (Value::String(format!("!{}", component::name(*t))), Value::empty_map()),
            })
            .collect();
        entries.extend(self.unparsed.iter().map(|(k, v)| (Value::str(k.as_str()), v.clone())));
        Value::Map(entries)
    }

    /// Decodes the persistent form, keeping entries that fail to decode (see
    /// [`DataComponentPatch::unparsed`]).
    pub fn from_value(v: &Value) -> DataResult<Self> {
        let mut patch = DataComponentPatch::new();
        for (k, v) in v.as_map()?.entries() {
            let key = k.as_str()?;
            match Self::decode_entry(key, v) {
                Ok((id, value)) => patch.put(id, value),
                Err(_) => {
                    patch.unparsed.retain(|(existing, _)| existing != key);
                    patch.unparsed.push((key.to_owned(), v.clone()));
                }
            }
        }
        Ok(patch)
    }

    /// Decodes the persistent form like vanilla does: any invalid entry is an error.
    pub fn from_value_strict(v: &Value) -> DataResult<Self> {
        let mut patch = DataComponentPatch::new();
        for (k, v) in v.as_map()?.entries() {
            let (id, value) = Self::decode_entry(k.as_str()?, v)?;
            patch.put(id, value);
        }
        Ok(patch)
    }

    fn decode_entry(key: &str, v: &Value) -> DataResult<(ComponentId, Option<Component>)> {
        let (removed, name) = match key.strip_prefix('!') {
            Some(n) => (true, n),
            None => (false, key),
        };
        let id = component::by_name(name).ok_or_else(|| DataError(format!("unknown component type {key:?}")))?;
        if !component::is_persistent(id) {
            return Err(DataError(format!("component type {key:?} is not persistent")));
        }
        if removed {
            return Ok((id, None));
        }
        let c = Component::from_value(id, v).map_err(|e| DataError(format!("{name}: {e}")))?;
        Ok((id, Some(c)))
    }
}

fn read_type(r: &mut Reader<'_>) -> WireResult<ComponentId> {
    registry::DATA_COMPONENT_TYPE.read_id(r).map(|i| i as ComponentId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{ids, keys};

    #[test]
    fn wire_keeps_insertion_order_and_puts_removals_last() {
        let mut p = DataComponentPatch::new();
        p.remove(ids::LORE);
        p.set(keys::DAMAGE.wrap(3));
        p.set(keys::MAX_STACK_SIZE.wrap(16));
        p.set(keys::DAMAGE.wrap(4)); // replaces in place
        let mut out = BytesMut::new();
        p.write(&mut out);
        let expected =
            [2, 1, ids::DAMAGE as u8, 4, ids::MAX_STACK_SIZE as u8, 16, ids::LORE as u8];
        assert_eq!(&out[..], &expected);
        let back = DataComponentPatch::read(&mut Reader::new(&out)).unwrap();
        assert!(back.same_entries(&p));
        assert!(DataComponentPatch::read(&mut Reader::new(&[0, 0])).unwrap().is_empty());
    }

    #[test]
    fn delimited_values_carry_their_length() {
        let mut p = DataComponentPatch::new();
        p.set(keys::DAMAGE.wrap(300));
        let mut out = BytesMut::new();
        p.write_delimited(&mut out);
        assert_eq!(&out[..], &[1, 0, ids::DAMAGE as u8, 2, 0xac, 0x02]);
        assert_eq!(DataComponentPatch::read_delimited(&mut Reader::new(&out)).unwrap(), p);
    }

    #[test]
    fn keeps_undecodable_entries_for_saving() {
        let v = Value::Map(vec![
            (Value::str("minecraft:damage"), Value::Int(2)),
            (Value::str("minecraft:max_damage"), Value::str("not a number")),
            (Value::str("kiln:unknown"), Value::Byte(1)),
            (Value::str("!minecraft:lore"), Value::empty_map()),
        ]);
        let p = DataComponentPatch::from_value(&v).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.unparsed().len(), 2);
        assert!(DataComponentPatch::from_value_strict(&v).is_err());
        let back = p.to_value();
        let mut got: Vec<_> = back.as_map().unwrap().entries().to_vec();
        let mut want: Vec<_> = v.as_map().unwrap().entries().to_vec();
        let key = |e: &(Value, Value)| e.0.as_str().unwrap().to_owned();
        got.sort_by_key(key);
        want.sort_by_key(key);
        assert_eq!(got, want);
    }
}
