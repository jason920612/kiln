//! Components holding item stacks, entities or block data: containers, bundles, crossbows, bees, block states.

use super::{ComponentValue, read_via_nbt, write_via_nbt};
use crate::ident::Identifier;
use crate::registry::{self, Registry};
use crate::stack::ItemStackTemplate;
use crate::value::{DataError, DataResult, MapBuilder, Value, err};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::nbt::Tag;
use kiln_proto::{DecodeError, Reader, WriteExt};

const UNBOUNDED: usize = i32::MAX as usize;

fn templates_to_value(items: &[ItemStackTemplate]) -> Value {
    Value::List(items.iter().map(ItemStackTemplate::to_value).collect())
}

fn templates_from_value(v: &Value, max: usize) -> DataResult<Vec<ItemStackTemplate>> {
    let items = v.as_list()?;
    if items.len() > max {
        return err(format!("more than {max} item stacks"));
    }
    items.iter().map(ItemStackTemplate::from_value).collect()
}

/// `charged_projectiles`: at most 1024 stacks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChargedProjectiles(pub Vec<ItemStackTemplate>);

impl ComponentValue for ChargedProjectiles {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, 1024, ItemStackTemplate::read).map(ChargedProjectiles)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, ItemStackTemplate::write);
    }
    fn to_value(&self) -> Value {
        templates_to_value(&self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        templates_from_value(v, 1024).map(ChargedProjectiles)
    }
}

/// `bundle_contents` (the selected item is client state and not serialized).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BundleContents(pub Vec<ItemStackTemplate>);

impl ComponentValue for BundleContents {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, UNBOUNDED, ItemStackTemplate::read).map(BundleContents)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, ItemStackTemplate::write);
    }
    fn to_value(&self) -> Value {
        templates_to_value(&self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        templates_from_value(v, UNBOUNDED).map(BundleContents)
    }
}

/// `container`: up to 256 slots, `None` for an empty slot.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemContainerContents(pub Vec<Option<ItemStackTemplate>>);

impl ComponentValue for ItemContainerContents {
    /// A list of optional stacks.
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, 256, |r| wire::read_opt(r, ItemStackTemplate::read)).map(ItemContainerContents)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, |slot, o| wire::write_opt(o, slot, ItemStackTemplate::write));
    }
    /// `[{slot, item}]` for the occupied slots.
    fn to_value(&self) -> Value {
        Value::List(
            self.0
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| {
                    let item = slot.as_ref()?;
                    Some(MapBuilder::new().put("slot", Value::Int(i as i32)).put("item", item.to_value()).build())
                })
                .collect(),
        )
    }
    /// The list is as long as the highest slot index; later duplicates win.
    fn from_value(v: &Value) -> DataResult<Self> {
        let entries = v.as_list()?;
        if entries.len() > 256 {
            return err("more than 256 container slots");
        }
        let mut slots: Vec<(usize, ItemStackTemplate)> = Vec::with_capacity(entries.len());
        for e in entries.iter() {
            let m = e.as_map()?;
            let slot = m.req_with("slot", Value::as_i32)?;
            if !(0..=255).contains(&slot) {
                return err(format!("container slot {slot} out of range [0;255]"));
            }
            slots.push((slot as usize, m.req_with("item", ItemStackTemplate::from_value)?));
        }
        let size = slots.iter().map(|(i, _)| i + 1).max().unwrap_or(0);
        let mut items = vec![None; size];
        for (i, item) in slots {
            items[i] = Some(item);
        }
        Ok(ItemContainerContents(items))
    }
}

/// `debug_stick_state`: the property the debug stick cycles, per block (`minecraft:block` id).
/// Its network form is its codec as NBT.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DebugStickState(pub Vec<(i32, String)>);

impl ComponentValue for DebugStickState {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_via_nbt(r)
    }
    fn write(&self, out: &mut BytesMut) {
        write_via_nbt(self, out)
    }
    /// `{"minecraft:block": "property"}`.
    fn to_value(&self) -> Value {
        Value::Map(self.0.iter().map(|(b, p)| (registry::BLOCK.id_to_value(*b), Value::str(p.clone()))).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let mut out: Vec<(i32, String)> = Vec::new();
        for (k, p) in v.as_map()?.entries() {
            let block = registry::BLOCK.id_from_value(k)?;
            let prop = p.as_str()?;
            let name = registry::BLOCK.name(block).unwrap_or_default();
            let known = kiln_data::blocks::BLOCKS
                .iter()
                .find(|b| b.name == name)
                .is_some_and(|b| b.properties.iter().any(|q| q.name == prop));
            if !known {
                return err(format!("{name} has no property {prop}"));
            }
            match out.iter_mut().find(|(b, _)| *b == block) {
                Some(slot) => slot.1 = prop.to_owned(),
                None => out.push((block, prop.to_owned())),
            }
        }
        Ok(DebugStickState(out))
    }
}

/// `TypedEntityData`: an entity or block entity type and its NBT (stored without `id`, which
/// the persistent form adds back).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedEntityData<const BLOCK: bool> {
    /// Network id in `minecraft:entity_type` or `minecraft:block_entity_type`.
    pub kind: i32,
    /// A compound without an `id` entry.
    pub tag: Tag,
}

/// `entity_data`: an entity type and its data.
pub type EntityData = TypedEntityData<false>;
/// `block_entity_data`: a block entity type and its data.
pub type BlockEntityData = TypedEntityData<true>;

impl<const BLOCK: bool> TypedEntityData<BLOCK> {
    const REGISTRY: Registry = if BLOCK { registry::BLOCK_ENTITY_TYPE } else { registry::ENTITY_TYPE };

    /// Data of `kind`; an `id` entry in `tag` is dropped (vanilla's `TypedEntityData.of`).
    pub fn new(kind: i32, tag: Tag) -> Self {
        TypedEntityData { kind, tag: strip_id(tag) }
    }

    /// `streamCodec`: the type's VarInt id, then the compound.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let kind = Self::REGISTRY.read_id(r)?;
        match wire::read_nbt(r)? {
            tag @ Tag::Compound(_) => Ok(Self::new(kind, tag)),
            _ => Err(DecodeError::Invalid("entity data must be a compound")),
        }
    }

    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.kind);
        wire::write_nbt(out, &self.tag);
    }

    /// `codec`: the compound with `id` set to the type's name.
    pub fn to_value(&self) -> Value {
        let mut v = Value::from_nbt(&self.tag);
        if let Value::Map(entries) = &mut v {
            entries.retain(|(k, _)| k.as_str() != Ok("id"));
            entries.push((Value::str("id"), Self::REGISTRY.id_to_value(self.kind)));
        }
        v
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let Value::Map(_) = v else { return err("entity data must be a compound") };
        let Tag::Compound(mut fields) = v.to_nbt() else { unreachable!() };
        let at = fields.iter().position(|(k, _)| k == "id").ok_or_else(|| DataError("missing id".into()))?;
        let (_, id) = fields.remove(at);
        let name = id.as_str().ok_or_else(|| DataError("id must be a string".into()))?;
        let kind = Self::REGISTRY.id(name).ok_or_else(|| DataError(format!("unknown {} {name:?}", Self::REGISTRY.0)))?;
        Ok(TypedEntityData { kind, tag: Tag::Compound(fields) })
    }
}

fn strip_id(tag: Tag) -> Tag {
    match tag {
        Tag::Compound(mut fields) => {
            fields.retain(|(k, _)| k != "id");
            Tag::Compound(fields)
        }
        other => other,
    }
}

impl<const BLOCK: bool> ComponentValue for TypedEntityData<BLOCK> {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Self::read(r)
    }
    fn write(&self, out: &mut BytesMut) {
        Self::write(self, out)
    }
    fn to_value(&self) -> Value {
        Self::to_value(self)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Self::from_value(v)
    }
}

/// `pot_decorations`: the sherd (or brick) on each side.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PotDecorations {
    pub back: Option<ItemStackTemplate>,
    pub left: Option<ItemStackTemplate>,
    pub right: Option<ItemStackTemplate>,
    pub front: Option<ItemStackTemplate>,
}

impl ComponentValue for PotDecorations {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(PotDecorations {
            back: wire::read_opt(r, ItemStackTemplate::read)?,
            left: wire::read_opt(r, ItemStackTemplate::read)?,
            right: wire::read_opt(r, ItemStackTemplate::read)?,
            front: wire::read_opt(r, ItemStackTemplate::read)?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        for side in [&self.back, &self.left, &self.right, &self.front] {
            wire::write_opt(out, side, ItemStackTemplate::write);
        }
    }
    fn to_value(&self) -> Value {
        let side = |s: &ItemStackTemplate| s.to_value();
        MapBuilder::new()
            .opt("back", self.back.as_ref(), side)
            .opt("left", self.left.as_ref(), side)
            .opt("right", self.right.as_ref(), side)
            .opt("front", self.front.as_ref(), side)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(PotDecorations {
            back: m.opt("back", ItemStackTemplate::from_value)?,
            left: m.opt("left", ItemStackTemplate::from_value)?,
            right: m.opt("right", ItemStackTemplate::from_value)?,
            front: m.opt("front", ItemStackTemplate::from_value)?,
        })
    }
}

/// `block_state`: block state properties by name, in the order they were given.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockItemStateProperties(pub Vec<(String, String)>);

impl ComponentValue for BlockItemStateProperties {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let n = wire::read_count(r, UNBOUNDED)?;
        let mut props: Vec<(String, String)> = Vec::with_capacity(n.min(r.remaining()));
        for _ in 0..n {
            let k = wire::read_string(r, 32767)?;
            let v = wire::read_string(r, 32767)?;
            put(&mut props, k, v);
        }
        Ok(BlockItemStateProperties(props))
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.0.len() as i32);
        for (k, v) in &self.0 {
            out.put_string(k);
            out.put_string(v);
        }
    }
    fn to_value(&self) -> Value {
        Value::Map(self.0.iter().map(|(k, v)| (Value::str(k.clone()), Value::str(v.clone()))).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let mut props = Vec::new();
        for (k, v) in v.as_map()?.entries() {
            put(&mut props, k.as_str()?.to_owned(), v.as_str()?.to_owned());
        }
        Ok(BlockItemStateProperties(props))
    }
}

/// Map insertion: a repeated key keeps its position and takes the new value.
fn put(props: &mut Vec<(String, String)>, k: String, v: String) {
    match props.iter_mut().find(|(n, _)| *n == k) {
        Some(slot) => slot.1 = v,
        None => props.push((k, v)),
    }
}

/// A bee in a hive (`BeehiveBlockEntity.Occupant`).
#[derive(Debug, Clone, PartialEq)]
pub struct BeeOccupant {
    pub entity_data: EntityData,
    pub ticks_in_hive: i32,
    pub min_ticks_in_hive: i32,
}

impl BeeOccupant {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(BeeOccupant { entity_data: EntityData::read(r)?, ticks_in_hive: r.varint()?, min_ticks_in_hive: r.varint()? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.entity_data.write(out);
        out.put_varint(self.ticks_in_hive);
        out.put_varint(self.min_ticks_in_hive);
    }

    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("entity_data", self.entity_data.to_value())
            .put("ticks_in_hive", Value::Int(self.ticks_in_hive))
            .put("min_ticks_in_hive", Value::Int(self.min_ticks_in_hive))
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(BeeOccupant {
            entity_data: m.req_with("entity_data", EntityData::from_value)?,
            ticks_in_hive: m.req_with("ticks_in_hive", Value::as_i32)?,
            min_ticks_in_hive: m.req_with("min_ticks_in_hive", Value::as_i32)?,
        })
    }
}

/// `bees`: the occupants of a beehive or bee nest.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bees(pub Vec<BeeOccupant>);

impl ComponentValue for Bees {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, UNBOUNDED, BeeOccupant::read).map(Bees)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, BeeOccupant::write);
    }
    fn to_value(&self) -> Value {
        Value::List(self.0.iter().map(BeeOccupant::to_value).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_list()?.iter().map(BeeOccupant::from_value).collect::<DataResult<_>>().map(Bees)
    }
}

/// `sulfur_cube_content`: the block item a sulfur cube absorbed.
#[derive(Debug, Clone, PartialEq)]
pub struct SulfurCubeContent(pub ItemStackTemplate);

impl ComponentValue for SulfurCubeContent {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        ItemStackTemplate::read(r).map(SulfurCubeContent)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out);
    }
    fn to_value(&self) -> Value {
        self.0.to_value()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        ItemStackTemplate::from_value(v).map(SulfurCubeContent)
    }
}

/// `container_loot`: an unopened loot table (network form: its codec as NBT).
#[derive(Debug, Clone, PartialEq)]
pub struct SeededContainerLoot {
    pub loot_table: Identifier,
    pub seed: i64,
}

impl ComponentValue for SeededContainerLoot {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_via_nbt(r)
    }
    fn write(&self, out: &mut BytesMut) {
        write_via_nbt(self, out)
    }
    fn to_value(&self) -> Value {
        MapBuilder::new().put("loot_table", self.loot_table.to_value()).opt_default("seed", self.seed, 0, Value::Long).build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(SeededContainerLoot {
            loot_table: m.req_with("loot_table", Identifier::from_value)?,
            seed: m.opt_or("seed", 0, Value::as_i64)?,
        })
    }
}
