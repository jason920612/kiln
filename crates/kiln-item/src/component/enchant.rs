//! Enchantments, enchantability and repair materials.

use super::ComponentValue;
use crate::holder::HolderSet;
use crate::registry;
use crate::value::{DataError, DataResult, MapBuilder, Value, err};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::{Reader, WriteExt};

/// `enchantments`, `stored_enchantments`: `minecraft:enchantment` network id to level (1..=255).
///
/// Vanilla keeps these in a hash map keyed by identity, so the order it writes them in is not
/// stable; entries keep the order they were decoded or inserted in, and equality ignores order.
#[derive(Debug, Clone, Default)]
pub struct Enchantments(pub Vec<(i32, i32)>);

impl Enchantments {
    pub fn level(&self, enchantment: i32) -> i32 {
        self.0.iter().find(|(e, _)| *e == enchantment).map_or(0, |(_, l)| *l)
    }

    /// Sets a level, replacing an existing entry in place; 0 or less removes it.
    pub fn set(&mut self, enchantment: i32, level: i32) {
        if level <= 0 {
            self.0.retain(|(e, _)| *e != enchantment);
        } else if let Some(slot) = self.0.iter_mut().find(|(e, _)| *e == enchantment) {
            slot.1 = level;
        } else {
            self.0.push((enchantment, level));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl PartialEq for Enchantments {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().all(|(e, l)| other.0.iter().any(|(e2, l2)| e == e2 && l == l2))
    }
}

impl ComponentValue for Enchantments {
    /// `ByteBufCodecs.map(Enchantment.STREAM_CODEC, VAR_INT)`.
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let n = wire::read_count(r, i32::MAX as usize)?;
        let mut out = Enchantments(Vec::with_capacity(n.min(r.remaining())));
        for _ in 0..n {
            let e = registry::ENCHANTMENT.read_id(r)?;
            let level = r.varint()?;
            match out.0.iter_mut().find(|(x, _)| *x == e) {
                Some(slot) => slot.1 = level,
                None => out.0.push((e, level)),
            }
        }
        Ok(out)
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.0.len() as i32);
        for (e, level) in &self.0 {
            out.put_varint(*e);
            out.put_varint(*level);
        }
    }
    /// `{"minecraft:sharpness": 5, ...}`.
    fn to_value(&self) -> Value {
        Value::Map(self.0.iter().map(|(e, l)| (registry::ENCHANTMENT.id_to_value(*e), Value::Int(*l))).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let mut out = Enchantments::default();
        for (k, l) in v.as_map()?.entries() {
            let e = registry::ENCHANTMENT.id_from_value(k)?;
            let level = l.as_i32()?;
            if !(1..=255).contains(&level) {
                return Err(DataError(format!("enchantment level {level} out of range [1;255]")));
            }
            match out.0.iter_mut().find(|(x, _)| *x == e) {
                Some(slot) => slot.1 = level,
                None => out.0.push((e, level)),
            }
        }
        Ok(out)
    }
}

/// `enchantable`: the enchanting table's enchantability (positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enchantable(pub i32);

impl ComponentValue for Enchantable {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.varint().map(Enchantable)
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.0);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new().put("value", Value::Int(self.0)).build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        match v.as_map()?.req_with("value", Value::as_i32)? {
            n if n > 0 => Ok(Enchantable(n)),
            n => err(format!("enchantability must be positive: {n}")),
        }
    }
}

/// `repairable`: items that repair this one in an anvil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repairable {
    /// `minecraft:item` entries.
    pub items: HolderSet,
}

impl ComponentValue for Repairable {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        HolderSet::read(registry::ITEM, r).map(|items| Repairable { items })
    }
    fn write(&self, out: &mut BytesMut) {
        self.items.write(out);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new().put("items", self.items.to_value(registry::ITEM)).build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Ok(Repairable { items: v.as_map()?.req_with("items", |v| HolderSet::from_value(registry::ITEM, v))? })
    }
}
