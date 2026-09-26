//! Block and item predicates: adventure mode `can_place_on` / `can_break`, container locks.
//!
//! Partial data component predicates (`DataComponentPredicate`) travel as their codec's NBT
//! even on the network (`DataComponentPredicate.TypeBase` uses `fromCodecWithRegistries`), and
//! `NbtPredicate` persists as an SNBT string (`TagParser.LENIENT_CODEC` encodes with its
//! flattened alternative), so this module also prints and parses SNBT like vanilla.

use super::{AttributeOperation, Component, ComponentId, ComponentValue, EquipmentSlotGroup, FireworkShape, read_via_nbt, write_via_nbt};
use crate::holder::HolderSet;
use crate::ident::Identifier;
use crate::registry::{self, Registry};
use crate::text::Text;
use crate::value::{DataError, DataResult, MapBuilder, MapView, Value, err};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::nbt::Tag;
use kiln_proto::{DecodeError, Reader, WriteExt};
use std::cmp::Ordering;

const UNBOUNDED: usize = i32::MAX as usize;

/// `minecraft:data_component_predicate_type`.
pub const DATA_COMPONENT_PREDICATE_TYPE: Registry = Registry("minecraft:data_component_predicate_type");

// ---- components -------------------------------------------------------------------------------

/// `can_place_on` / `can_break`: the block predicates, any of which allows the action.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AdventureModePredicate(pub Vec<BlockPredicate>);

impl ComponentValue for AdventureModePredicate {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, UNBOUNDED, BlockPredicate::read).map(AdventureModePredicate)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, BlockPredicate::write);
    }
    /// `ExtraCodecs.compactListCodec`: a single predicate is written bare.
    fn to_value(&self) -> Value {
        match self.0.as_slice() {
            [one] => one.to_value(),
            all => Value::List(all.iter().map(BlockPredicate::to_value).collect()),
        }
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let preds: Vec<BlockPredicate> = match v {
            Value::Map(_) => vec![BlockPredicate::from_value(v)?],
            _ => v.as_list()?.iter().map(BlockPredicate::from_value).collect::<DataResult<_>>()?,
        };
        if preds.is_empty() {
            return err("adventure mode predicate list is empty");
        }
        Ok(AdventureModePredicate(preds))
    }
}

/// `lock`: the item predicate a key must match (`LockCode`; persistent only, so its network
/// form is its codec as NBT).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LockCode(pub ItemPredicate);

impl ComponentValue for LockCode {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_via_nbt(r)
    }
    fn write(&self, out: &mut BytesMut) {
        write_via_nbt(self, out)
    }
    fn to_value(&self) -> Value {
        self.0.to_value()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        ItemPredicate::from_value(v).map(LockCode)
    }
}

// ---- block and item predicates ----------------------------------------------------------------

/// `BlockPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockPredicate {
    /// `minecraft:block` entries or a block tag.
    pub blocks: Option<HolderSet>,
    pub state: Option<StatePropertiesPredicate>,
    pub nbt: Option<NbtPredicate>,
    pub components: DataComponentMatchers,
}

impl BlockPredicate {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(BlockPredicate {
            blocks: wire::read_opt(r, |r| HolderSet::read(registry::BLOCK, r))?,
            state: wire::read_opt(r, StatePropertiesPredicate::read)?,
            nbt: wire::read_opt(r, NbtPredicate::read)?,
            components: DataComponentMatchers::read(r)?,
        })
    }

    pub fn write(&self, out: &mut BytesMut) {
        wire::write_opt(out, &self.blocks, |b, o| b.write(o));
        wire::write_opt(out, &self.state, StatePropertiesPredicate::write);
        wire::write_opt(out, &self.nbt, NbtPredicate::write);
        self.components.write(out);
    }

    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.opt("blocks", self.blocks.as_ref(), |b| b.to_value(registry::BLOCK))
            .opt("state", self.state.as_ref(), StatePropertiesPredicate::to_value)
            .opt("nbt", self.nbt.as_ref(), NbtPredicate::to_value);
        self.components.put_fields(&mut m);
        m.build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(BlockPredicate {
            blocks: m.opt("blocks", |v| HolderSet::from_value(registry::BLOCK, v))?,
            state: m.opt("state", StatePropertiesPredicate::from_value)?,
            nbt: m.opt("nbt", NbtPredicate::from_value)?,
            components: DataComponentMatchers::from_fields(m)?,
        })
    }
}

/// `ItemPredicate` (persistent only; never sent on its own).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemPredicate {
    /// `minecraft:item` entries or an item tag.
    pub items: Option<HolderSet>,
    pub count: IntBounds,
    pub components: DataComponentMatchers,
}

impl ItemPredicate {
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.opt("items", self.items.as_ref(), |i| i.to_value(registry::ITEM)).opt_default(
            "count",
            &self.count,
            &IntBounds::ANY,
            IntBounds::to_value,
        );
        self.components.put_fields(&mut m);
        m.build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(ItemPredicate {
            items: m.opt("items", |v| HolderSet::from_value(registry::ITEM, v))?,
            count: m.opt_or("count", IntBounds::ANY, IntBounds::from_value)?,
            components: DataComponentMatchers::from_fields(m)?,
        })
    }
}

// ---- min/max bounds ---------------------------------------------------------------------------

/// `MinMaxBounds.Bounds`: optional inclusive limits.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bounds<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

/// `MinMaxBounds.Ints`.
pub type IntBounds = Bounds<i32>;
/// `MinMaxBounds.Doubles`.
pub type DoubleBounds = Bounds<f64>;

impl<T: Copy + PartialEq> Bounds<T> {
    pub const ANY: Bounds<T> = Bounds { min: None, max: None };

    pub fn exactly(v: T) -> Self {
        Bounds { min: Some(v), max: Some(v) }
    }

    /// A single number when both limits are equal, else `{min, max}` (either optional).
    fn to_value_with(self, f: fn(T) -> Value) -> Value {
        match (self.min, self.max) {
            (Some(a), Some(b)) if a == b => f(a),
            (min, max) => MapBuilder::new().opt("min", min, f).opt("max", max, f).build(),
        }
    }

    fn from_value_with(v: &Value, f: fn(&Value) -> DataResult<T>) -> DataResult<Self> {
        match v {
            Value::Map(_) => {
                let m = v.as_map()?;
                Ok(Bounds { min: m.opt("min", f)?, max: m.opt("max", f)? })
            }
            _ => f(v).map(Bounds::exactly),
        }
    }
}

impl IntBounds {
    pub fn to_value(&self) -> Value {
        self.to_value_with(Value::Int)
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        Self::from_value_with(v, Value::as_i32)
    }
}

impl DoubleBounds {
    pub fn to_value(&self) -> Value {
        self.to_value_with(Value::Double)
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        Self::from_value_with(v, Value::as_f64)
    }
}

// ---- block state properties -------------------------------------------------------------------

/// `StatePropertiesPredicate`: property name to expected value or range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatePropertiesPredicate(pub Vec<PropertyMatcher>);

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyMatcher {
    pub name: String,
    pub value: ValueMatcher,
}

/// `StatePropertiesPredicate.ValueMatcher`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueMatcher {
    Exact(String),
    Range { min: Option<String>, max: Option<String> },
}

impl StatePropertiesPredicate {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let props = wire::read_list(r, UNBOUNDED, |r| {
            let name = wire::read_string(r, 32767)?;
            // `ByteBufCodecs.either`: true for the exact (left) form.
            let value = if r.bool()? {
                ValueMatcher::Exact(wire::read_string(r, 32767)?)
            } else {
                ValueMatcher::Range {
                    min: wire::read_opt(r, |r| wire::read_string(r, 32767))?,
                    max: wire::read_opt(r, |r| wire::read_string(r, 32767))?,
                }
            };
            Ok(PropertyMatcher { name, value })
        })?;
        Ok(StatePropertiesPredicate(props))
    }

    pub fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, |p, o| {
            o.put_string(&p.name);
            match &p.value {
                ValueMatcher::Exact(v) => {
                    o.put_bool(true);
                    o.put_string(v);
                }
                ValueMatcher::Range { min, max } => {
                    o.put_bool(false);
                    wire::write_opt(o, min, |s, o| o.put_string(s));
                    wire::write_opt(o, max, |s, o| o.put_string(s));
                }
            }
        });
    }

    /// A map from property name to a string or `{min, max}`.
    pub fn to_value(&self) -> Value {
        Value::Map(
            self.0
                .iter()
                .map(|p| {
                    let v = match &p.value {
                        ValueMatcher::Exact(s) => Value::str(s.as_str()),
                        ValueMatcher::Range { min, max } => MapBuilder::new()
                            .opt("min", min.as_deref(), Value::str)
                            .opt("max", max.as_deref(), Value::str)
                            .build(),
                    };
                    (Value::str(p.name.as_str()), v)
                })
                .collect(),
        )
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let mut props: Vec<PropertyMatcher> = Vec::new();
        for (k, v) in v.as_map()?.entries() {
            let value = match v {
                Value::Map(_) => {
                    let m = v.as_map()?;
                    let s = |key| m.opt(key, |v: &Value| v.as_str().map(str::to_owned));
                    ValueMatcher::Range { min: s("min")?, max: s("max")? }
                }
                _ => ValueMatcher::Exact(v.as_str()?.to_owned()),
            };
            let name = k.as_str()?.to_owned();
            match props.iter_mut().find(|p| p.name == name) {
                Some(p) => p.value = value,
                None => props.push(PropertyMatcher { name, value }),
            }
        }
        Ok(StatePropertiesPredicate(props))
    }
}

// ---- NBT predicate ----------------------------------------------------------------------------

/// `NbtPredicate`: a compound the target's NBT must contain. On the network a compound tag;
/// persisted as its SNBT string.
#[derive(Debug, Clone, PartialEq)]
pub struct NbtPredicate(pub Tag);

impl NbtPredicate {
    /// `ByteBufCodecs.COMPOUND_TAG`.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        match wire::read_nbt(r)? {
            tag @ Tag::Compound(_) => Ok(NbtPredicate(tag)),
            _ => Err(DecodeError::Invalid("NBT predicate must be a compound")),
        }
    }

    pub fn write(&self, out: &mut BytesMut) {
        wire::write_nbt(out, &self.0);
    }

    /// The SNBT form (`CompoundTag.toString()`).
    pub fn to_value(&self) -> Value {
        Value::String(snbt(&self.0))
    }

    /// SNBT, or a compound (`CompoundTag.CODEC`, the alternative form).
    pub fn from_value(v: &Value) -> DataResult<Self> {
        let parsed;
        let v = match v {
            Value::String(s) => {
                parsed = parse_snbt(s)?;
                &parsed
            }
            other => other,
        };
        match v {
            Value::Map(_) => Ok(NbtPredicate(v.to_nbt())),
            _ => err("NBT predicate must be a compound"),
        }
    }
}

// ---- data component matchers ------------------------------------------------------------------

/// `DataComponentMatchers`: exact component values (`DataComponentExactPredicate`) and
/// partial predicates (`DataComponentPredicate`), inlined into the enclosing map as
/// `components` and `predicates`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataComponentMatchers {
    pub exact: Vec<Component>,
    pub partial: Vec<PartialPredicate>,
}

impl DataComponentMatchers {
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.partial.is_empty()
    }

    /// Exact components as `TypedDataComponent` (type id and network value), then up to 64
    /// partial predicates.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let exact = wire::read_list(r, UNBOUNDED, |r| {
            let id = registry::DATA_COMPONENT_TYPE.read_id(r)? as ComponentId;
            Component::read(id, r)
        })?;
        let partial = wire::read_list(r, 64, PartialPredicate::read)?;
        Ok(DataComponentMatchers { exact, partial })
    }

    pub fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.exact, |c, o| {
            o.put_varint(c.id() as i32);
            c.write(o);
        });
        wire::write_list(out, &self.partial, PartialPredicate::write);
    }

    /// Adds `components` (transient types left out) and `predicates`, each omitted when empty.
    pub fn put_fields(&self, m: &mut MapBuilder) {
        if !self.exact.is_empty() {
            let entries =
                self.exact.iter().filter_map(|c| c.to_value().map(|v| (Value::str(super::name(c.id())), v))).collect();
            m.put("components", Value::Map(entries));
        }
        if !self.partial.is_empty() {
            m.put("predicates", Value::Map(self.partial.iter().map(|p| (Value::str(p.key()), p.to_value())).collect()));
        }
    }

    pub fn from_fields(m: MapView<'_>) -> DataResult<Self> {
        let mut out = DataComponentMatchers::default();
        if let Some(v) = m.get("components") {
            for (k, v) in v.as_map()?.entries() {
                let name = k.as_str()?;
                let id = super::by_name(name).ok_or_else(|| DataError(format!("unknown component type {name:?}")))?;
                let c = Component::from_value(id, v).map_err(|e| DataError(format!("{name}: {e}")))?;
                match out.exact.iter_mut().find(|e| e.id() == id) {
                    Some(slot) => *slot = c,
                    None => out.exact.push(c),
                }
            }
        }
        if let Some(v) = m.get("predicates") {
            for (k, v) in v.as_map()?.entries() {
                let p = PartialPredicate::from_value(k.as_str()?, v)?;
                match out.partial.iter_mut().find(|e| e.key() == p.key()) {
                    Some(slot) => *slot = p,
                    None => out.partial.push(p),
                }
            }
        }
        Ok(out)
    }
}

/// A partial data component predicate, keyed by its `minecraft:data_component_predicate_type`
/// (or, for [`PartialPredicate::Exists`], by the component type it requires).
#[derive(Debug, Clone, PartialEq)]
pub enum PartialPredicate {
    Damage(DamagePredicate),
    Enchantments(Vec<EnchantmentPredicate>),
    StoredEnchantments(Vec<EnchantmentPredicate>),
    PotionContents(PotionsPredicate),
    CustomData(NbtPredicate),
    Container(Option<CollectionPredicate<ItemPredicate>>),
    BundleContents(Option<CollectionPredicate<ItemPredicate>>),
    FireworkExplosion(FireworkPredicate),
    Fireworks(FireworksPredicate),
    WritableBookContent(Option<CollectionPredicate<String>>),
    WrittenBookContent(WrittenBookPredicate),
    AttributeModifiers(Option<CollectionPredicate<AttributeModifierPredicate>>),
    Trim(TrimPredicate),
    /// `minecraft:jukebox_song` entries or tag.
    JukeboxPlayable(Option<HolderSet>),
    /// `minecraft:villager_type` entries or tag.
    VillagerVariant(HolderSet),
    /// `AnyValue`: the component type is present.
    Exists(ComponentId),
}

const PREDICATE_TYPES: &[&str] = &[
    "minecraft:damage",
    "minecraft:enchantments",
    "minecraft:stored_enchantments",
    "minecraft:potion_contents",
    "minecraft:custom_data",
    "minecraft:container",
    "minecraft:bundle_contents",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
    "minecraft:attribute_modifiers",
    "minecraft:trim",
    "minecraft:jukebox_playable",
    "minecraft:villager/variant",
];

impl PartialPredicate {
    fn type_name(&self) -> &'static str {
        PREDICATE_TYPES[match self {
            PartialPredicate::Damage(_) => 0,
            PartialPredicate::Enchantments(_) => 1,
            PartialPredicate::StoredEnchantments(_) => 2,
            PartialPredicate::PotionContents(_) => 3,
            PartialPredicate::CustomData(_) => 4,
            PartialPredicate::Container(_) => 5,
            PartialPredicate::BundleContents(_) => 6,
            PartialPredicate::FireworkExplosion(_) => 7,
            PartialPredicate::Fireworks(_) => 8,
            PartialPredicate::WritableBookContent(_) => 9,
            PartialPredicate::WrittenBookContent(_) => 10,
            PartialPredicate::AttributeModifiers(_) => 11,
            PartialPredicate::Trim(_) => 12,
            PartialPredicate::JukeboxPlayable(_) => 13,
            PartialPredicate::VillagerVariant(_) => 14,
            PartialPredicate::Exists(_) => unreachable!(),
        }]
    }

    /// The key in `predicates` (`DataComponentPredicate.Type.CODEC`).
    pub fn key(&self) -> &'static str {
        match self {
            PartialPredicate::Exists(id) => super::name(*id),
            _ => self.type_name(),
        }
    }

    /// `DataComponentPredicate.SINGLE_STREAM_CODEC`: the type (`either` of a predicate type id
    /// or, for "exists", a component type id), then the predicate's codec as NBT.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let invalid = |_| DecodeError::Invalid("data component predicate");
        if r.bool()? {
            let id = DATA_COMPONENT_PREDICATE_TYPE.read_id(r)?;
            let name = DATA_COMPONENT_PREDICATE_TYPE.name(id).ok_or(DecodeError::Invalid("predicate type"))?;
            let value = wire::read_value(r)?;
            Self::from_typed_value(name, &value).map_err(invalid)?.ok_or(DecodeError::Invalid("predicate type"))
        } else {
            let id = registry::DATA_COMPONENT_TYPE.read_id(r)? as ComponentId;
            wire::read_nbt(r)?;
            Ok(PartialPredicate::Exists(id))
        }
    }

    pub fn write(&self, out: &mut BytesMut) {
        match self {
            PartialPredicate::Exists(id) => {
                out.put_bool(false);
                out.put_varint(*id as i32);
            }
            _ => {
                out.put_bool(true);
                out.put_varint(DATA_COMPONENT_PREDICATE_TYPE.id(self.type_name()).unwrap_or(0));
            }
        }
        wire::write_value(out, &self.to_value());
    }

    pub fn to_value(&self) -> Value {
        let items = |c: &Option<CollectionPredicate<ItemPredicate>>| {
            MapBuilder::new().opt("items", c.as_ref(), |c| c.to_value(ItemPredicate::to_value)).build()
        };
        match self {
            PartialPredicate::Damage(p) => p.to_value(),
            PartialPredicate::Enchantments(l) | PartialPredicate::StoredEnchantments(l) => {
                Value::List(l.iter().map(EnchantmentPredicate::to_value).collect())
            }
            PartialPredicate::PotionContents(p) => p.to_value(),
            PartialPredicate::CustomData(p) => p.to_value(),
            PartialPredicate::Container(c) | PartialPredicate::BundleContents(c) => items(c),
            PartialPredicate::FireworkExplosion(p) => p.to_value(),
            PartialPredicate::Fireworks(p) => p.to_value(),
            PartialPredicate::WritableBookContent(c) => {
                MapBuilder::new().opt("pages", c.as_ref(), |c| c.to_value(|s| Value::str(s.as_str()))).build()
            }
            PartialPredicate::WrittenBookContent(p) => p.to_value(),
            PartialPredicate::AttributeModifiers(c) => MapBuilder::new()
                .opt("modifiers", c.as_ref(), |c| c.to_value(AttributeModifierPredicate::to_value))
                .build(),
            PartialPredicate::Trim(p) => p.to_value(),
            PartialPredicate::JukeboxPlayable(s) => {
                MapBuilder::new().opt("song", s.as_ref(), |s| s.to_value(registry::JUKEBOX_SONG)).build()
            }
            PartialPredicate::VillagerVariant(s) => s.to_value(registry::VILLAGER_TYPE),
            // `MapCodec.unitCodec`.
            PartialPredicate::Exists(_) => Value::empty_map(),
        }
    }

    /// Decodes the entry `key: v` of `predicates`: a predicate type, else a component type.
    pub fn from_value(key: &str, v: &Value) -> DataResult<Self> {
        let name = Identifier::parse(key).ok_or_else(|| DataError(format!("invalid predicate type {key:?}")))?;
        if let Some(p) = Self::from_typed_value(name.as_str(), v).map_err(|e| DataError(format!("{key}: {e}")))? {
            return Ok(p);
        }
        match super::by_name(name.as_str()) {
            Some(id) => Ok(PartialPredicate::Exists(id)),
            None => err(format!("unknown predicate type {key:?}")),
        }
    }

    /// `None` if `name` is not a predicate type.
    fn from_typed_value(name: &str, v: &Value) -> DataResult<Option<Self>> {
        let items = |v: &Value| -> DataResult<_> {
            v.as_map()?.opt("items", |v| CollectionPredicate::from_value(v, ItemPredicate::from_value))
        };
        let enchantments = |v: &Value| -> DataResult<Vec<EnchantmentPredicate>> {
            v.as_list()?.iter().map(EnchantmentPredicate::from_value).collect()
        };
        Ok(Some(match name {
            "minecraft:damage" => PartialPredicate::Damage(DamagePredicate::from_value(v)?),
            "minecraft:enchantments" => PartialPredicate::Enchantments(enchantments(v)?),
            "minecraft:stored_enchantments" => PartialPredicate::StoredEnchantments(enchantments(v)?),
            "minecraft:potion_contents" => PartialPredicate::PotionContents(PotionsPredicate::from_value(v)?),
            "minecraft:custom_data" => PartialPredicate::CustomData(NbtPredicate::from_value(v)?),
            "minecraft:container" => PartialPredicate::Container(items(v)?),
            "minecraft:bundle_contents" => PartialPredicate::BundleContents(items(v)?),
            "minecraft:firework_explosion" => PartialPredicate::FireworkExplosion(FireworkPredicate::from_value(v)?),
            "minecraft:fireworks" => PartialPredicate::Fireworks(FireworksPredicate::from_value(v)?),
            "minecraft:writable_book_content" => PartialPredicate::WritableBookContent(v.as_map()?.opt("pages", |v| {
                CollectionPredicate::from_value(v, |p| p.as_str().map(str::to_owned))
            })?),
            "minecraft:written_book_content" => PartialPredicate::WrittenBookContent(WrittenBookPredicate::from_value(v)?),
            "minecraft:attribute_modifiers" => PartialPredicate::AttributeModifiers(v.as_map()?.opt("modifiers", |v| {
                CollectionPredicate::from_value(v, AttributeModifierPredicate::from_value)
            })?),
            "minecraft:trim" => PartialPredicate::Trim(TrimPredicate::from_value(v)?),
            "minecraft:jukebox_playable" => PartialPredicate::JukeboxPlayable(
                v.as_map()?.opt("song", |v| HolderSet::from_value(registry::JUKEBOX_SONG, v))?,
            ),
            "minecraft:villager/variant" => PartialPredicate::VillagerVariant(HolderSet::from_value(registry::VILLAGER_TYPE, v)?),
            _ => return Ok(None),
        }))
    }
}

/// `CollectionPredicate`: element predicates the collection must contain, counted matches,
/// and a size range.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectionPredicate<P> {
    pub contains: Option<Vec<P>>,
    pub count: Option<Vec<(P, IntBounds)>>,
    pub size: Option<IntBounds>,
}

impl<P> CollectionPredicate<P> {
    pub fn to_value(&self, f: impl Fn(&P) -> Value) -> Value {
        MapBuilder::new()
            .opt("contains", self.contains.as_ref(), |l| Value::List(l.iter().map(&f).collect()))
            .opt("count", self.count.as_ref(), |l| {
                Value::List(
                    l.iter().map(|(p, c)| MapBuilder::new().put("test", f(p)).put("count", c.to_value()).build()).collect(),
                )
            })
            .opt("size", self.size.as_ref(), IntBounds::to_value)
            .build()
    }

    pub fn from_value(v: &Value, f: impl Fn(&Value) -> DataResult<P>) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(CollectionPredicate {
            contains: m.opt("contains", |l| l.as_list()?.iter().map(&f).collect())?,
            count: m.opt("count", |l| {
                l.as_list()?
                    .iter()
                    .map(|e| {
                        let e = e.as_map()?;
                        Ok((e.req_with("test", &f)?, e.req_with("count", IntBounds::from_value)?))
                    })
                    .collect()
            })?,
            size: m.opt("size", IntBounds::from_value)?,
        })
    }
}

/// `DamagePredicate`: remaining durability and damage ranges.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DamagePredicate {
    pub durability: IntBounds,
    pub damage: IntBounds,
}

impl DamagePredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("durability", &self.durability, &IntBounds::ANY, IntBounds::to_value)
            .opt_default("damage", &self.damage, &IntBounds::ANY, IntBounds::to_value)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(DamagePredicate {
            durability: m.opt_or("durability", IntBounds::ANY, IntBounds::from_value)?,
            damage: m.opt_or("damage", IntBounds::ANY, IntBounds::from_value)?,
        })
    }
}

/// `EnchantmentPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnchantmentPredicate {
    /// `minecraft:enchantment` entries or tag.
    pub enchantments: Option<HolderSet>,
    pub levels: IntBounds,
}

impl EnchantmentPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("enchantments", self.enchantments.as_ref(), |s| s.to_value(registry::ENCHANTMENT))
            .opt_default("levels", &self.levels, &IntBounds::ANY, IntBounds::to_value)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(EnchantmentPredicate {
            enchantments: m.opt("enchantments", |v| HolderSet::from_value(registry::ENCHANTMENT, v))?,
            levels: m.opt_or("levels", IntBounds::ANY, IntBounds::from_value)?,
        })
    }
}

/// `PotionsPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PotionsPredicate {
    /// `minecraft:potion` entries or tag.
    pub potions: Option<HolderSet>,
    pub effects: Option<CollectionPredicate<MobEffectsPredicate>>,
}

impl PotionsPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("potions", self.potions.as_ref(), |s| s.to_value(registry::POTION))
            .opt("effects", self.effects.as_ref(), |c| c.to_value(MobEffectsPredicate::to_value))
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(PotionsPredicate {
            potions: m.opt("potions", |v| HolderSet::from_value(registry::POTION, v))?,
            effects: m.opt("effects", |v| CollectionPredicate::from_value(v, MobEffectsPredicate::from_value))?,
        })
    }
}

/// `MobEffectsPredicate`: `minecraft:mob_effect` id to instance predicate.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MobEffectsPredicate(pub Vec<(i32, MobEffectInstancePredicate)>);

/// `MobEffectsPredicate.MobEffectInstancePredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MobEffectInstancePredicate {
    pub amplifier: IntBounds,
    pub duration: IntBounds,
    pub ambient: Option<bool>,
    pub visible: Option<bool>,
}

impl MobEffectsPredicate {
    pub fn to_value(&self) -> Value {
        Value::Map(
            self.0
                .iter()
                .map(|(effect, p)| {
                    let v = MapBuilder::new()
                        .opt_default("amplifier", &p.amplifier, &IntBounds::ANY, IntBounds::to_value)
                        .opt_default("duration", &p.duration, &IntBounds::ANY, IntBounds::to_value)
                        .opt("ambient", p.ambient, Value::Bool)
                        .opt("visible", p.visible, Value::Bool)
                        .build();
                    (registry::MOB_EFFECT.id_to_value(*effect), v)
                })
                .collect(),
        )
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let mut out: Vec<(i32, MobEffectInstancePredicate)> = Vec::new();
        for (k, v) in v.as_map()?.entries() {
            let effect = registry::MOB_EFFECT.id_from_value(k)?;
            let m = v.as_map()?;
            let p = MobEffectInstancePredicate {
                amplifier: m.opt_or("amplifier", IntBounds::ANY, IntBounds::from_value)?,
                duration: m.opt_or("duration", IntBounds::ANY, IntBounds::from_value)?,
                ambient: m.opt("ambient", Value::as_bool)?,
                visible: m.opt("visible", Value::as_bool)?,
            };
            match out.iter_mut().find(|(e, _)| *e == effect) {
                Some(slot) => slot.1 = p,
                None => out.push((effect, p)),
            }
        }
        Ok(MobEffectsPredicate(out))
    }
}

/// `FireworkExplosionPredicate.FireworkPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FireworkPredicate {
    pub shape: Option<FireworkShape>,
    pub has_twinkle: Option<bool>,
    pub has_trail: Option<bool>,
}

impl FireworkPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("shape", self.shape, FireworkShape::to_value)
            .opt("has_twinkle", self.has_twinkle, Value::Bool)
            .opt("has_trail", self.has_trail, Value::Bool)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(FireworkPredicate {
            shape: m.opt("shape", FireworkShape::from_value)?,
            has_twinkle: m.opt("has_twinkle", Value::as_bool)?,
            has_trail: m.opt("has_trail", Value::as_bool)?,
        })
    }
}

/// `FireworksPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FireworksPredicate {
    pub explosions: Option<CollectionPredicate<FireworkPredicate>>,
    pub flight_duration: IntBounds,
}

impl FireworksPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("explosions", self.explosions.as_ref(), |c| c.to_value(FireworkPredicate::to_value))
            .opt_default("flight_duration", &self.flight_duration, &IntBounds::ANY, IntBounds::to_value)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(FireworksPredicate {
            explosions: m.opt("explosions", |v| CollectionPredicate::from_value(v, FireworkPredicate::from_value))?,
            flight_duration: m.opt_or("flight_duration", IntBounds::ANY, IntBounds::from_value)?,
        })
    }
}

/// `WrittenBookPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WrittenBookPredicate {
    pub pages: Option<CollectionPredicate<Text>>,
    pub author: Option<String>,
    pub title: Option<String>,
    pub generation: IntBounds,
    pub resolved: Option<bool>,
}

impl WrittenBookPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("pages", self.pages.as_ref(), |c| c.to_value(Text::to_value))
            .opt("author", self.author.as_deref(), Value::str)
            .opt("title", self.title.as_deref(), Value::str)
            .opt_default("generation", &self.generation, &IntBounds::ANY, IntBounds::to_value)
            .opt("resolved", self.resolved, Value::Bool)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let s = |key| m.opt(key, |v: &Value| v.as_str().map(str::to_owned));
        Ok(WrittenBookPredicate {
            pages: m.opt("pages", |v| CollectionPredicate::from_value(v, Text::from_value))?,
            author: s("author")?,
            title: s("title")?,
            generation: m.opt_or("generation", IntBounds::ANY, IntBounds::from_value)?,
            resolved: m.opt("resolved", Value::as_bool)?,
        })
    }
}

/// `AttributeModifiersPredicate.EntryPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributeModifierPredicate {
    /// `minecraft:attribute` entries or tag.
    pub attribute: Option<HolderSet>,
    pub id: Option<Identifier>,
    pub amount: DoubleBounds,
    pub operation: Option<AttributeOperation>,
    pub slot: Option<EquipmentSlotGroup>,
}

impl AttributeModifierPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("attribute", self.attribute.as_ref(), |s| s.to_value(registry::ATTRIBUTE))
            .opt("id", self.id.as_ref(), Identifier::to_value)
            .opt_default("amount", &self.amount, &DoubleBounds::ANY, DoubleBounds::to_value)
            .opt("operation", self.operation, AttributeOperation::to_value)
            .opt("slot", self.slot, EquipmentSlotGroup::to_value)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(AttributeModifierPredicate {
            attribute: m.opt("attribute", |v| HolderSet::from_value(registry::ATTRIBUTE, v))?,
            id: m.opt("id", Identifier::from_value)?,
            amount: m.opt_or("amount", DoubleBounds::ANY, DoubleBounds::from_value)?,
            operation: m.opt("operation", AttributeOperation::from_value)?,
            slot: m.opt("slot", EquipmentSlotGroup::from_value)?,
        })
    }
}

/// `TrimPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrimPredicate {
    /// `minecraft:trim_material` entries or tag.
    pub material: Option<HolderSet>,
    /// `minecraft:trim_pattern` entries or tag.
    pub pattern: Option<HolderSet>,
}

impl TrimPredicate {
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("material", self.material.as_ref(), |s| s.to_value(registry::TRIM_MATERIAL))
            .opt("pattern", self.pattern.as_ref(), |s| s.to_value(registry::TRIM_PATTERN))
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(TrimPredicate {
            material: m.opt("material", |v| HolderSet::from_value(registry::TRIM_MATERIAL, v))?,
            pattern: m.opt("pattern", |v| HolderSet::from_value(registry::TRIM_PATTERN, v))?,
        })
    }
}

// ---- SNBT -------------------------------------------------------------------------------------

/// `Tag.toString()` (`StringTagVisitor`): compact SNBT with compound keys sorted.
pub fn snbt(tag: &Tag) -> String {
    let mut out = String::new();
    write_snbt(tag, &mut out);
    out
}

fn write_snbt(tag: &Tag, out: &mut String) {
    use std::fmt::Write as _;
    match tag {
        Tag::Byte(v) => write!(out, "{v}b").unwrap(),
        Tag::Short(v) => write!(out, "{v}s").unwrap(),
        Tag::Int(v) => write!(out, "{v}").unwrap(),
        Tag::Long(v) => write!(out, "{v}L").unwrap(),
        Tag::Float(v) => {
            out.push_str(&java_float(*v));
            out.push('f');
        }
        Tag::Double(v) => {
            out.push_str(&java_double(*v));
            out.push('d');
        }
        Tag::ByteArray(v) => {
            out.push_str("[B;");
            for (i, b) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write!(out, "{b}B").unwrap();
            }
            out.push(']');
        }
        Tag::IntArray(v) => {
            out.push_str("[I;");
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write!(out, "{x}").unwrap();
            }
            out.push(']');
        }
        Tag::LongArray(v) => {
            out.push_str("[L;");
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write!(out, "{x}L").unwrap();
            }
            out.push(']');
        }
        Tag::String(s) => quote(s, out),
        Tag::List(items) => {
            // `ListTag` holds wrapped elements of mixed lists unwrapped.
            let compounds = items.first().is_some_and(|t| matches!(t, Tag::Compound(_)));
            out.push('[');
            for (i, t) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_snbt(if compounds { t.unwrap_list_element() } else { t }, out);
            }
            out.push(']');
        }
        Tag::Compound(fields) => {
            let mut sorted: Vec<&(String, Tag)> = fields.iter().collect();
            sorted.sort_by(|a, b| java_cmp(&a.0, &b.0));
            out.push('{');
            for (i, (k, v)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if is_bare_key(k) {
                    out.push_str(k);
                } else {
                    quote(k, out);
                }
                out.push(':');
                write_snbt(v, out);
            }
            out.push('}');
        }
    }
}

/// `String.compareTo`: UTF-16 code unit order.
fn java_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `[A-Za-z._]+[A-Za-z0-9._+-]*`, and not `true`/`false` in any case.
fn is_bare_key(k: &str) -> bool {
    let mut chars = k.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '.' || c == '_');
    first_ok
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        && !k.eq_ignore_ascii_case("true")
        && !k.eq_ignore_ascii_case("false")
}

/// `StringTag.quoteAndEscape`: double quotes unless the first quote character in the string
/// is a double quote.
fn quote(s: &str, out: &mut String) {
    let start = out.len();
    out.push(' ');
    let mut q: Option<char> = None;
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' | '\'' => {
                let quote = *q.get_or_insert(if c == '"' { '\'' } else { '"' });
                if quote == c {
                    out.push('\\');
                }
                out.push(c);
            }
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\x{:02X}", c as u32));
            }
            c => out.push(c),
        }
    }
    let q = q.unwrap_or('"');
    out.replace_range(start..start + 1, q.encode_utf8(&mut [0; 4]));
    out.push(q);
}

/// `Float.toString`.
pub fn java_float(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    java_decimal(v == 0.0, v.is_sign_negative(), &format!("{:e}", v.abs()), v.abs() as f64)
}

/// `Double.toString`.
pub fn java_double(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    java_decimal(v == 0.0, v.is_sign_negative(), &format!("{:e}", v.abs()), v.abs())
}

/// Java's layout of the shortest round-trip digits (`sci` is Rust's `{:e}` of the magnitude):
/// plain notation for magnitudes in [1e-3, 1e7), else `d.dddE±n`; always a fractional digit.
fn java_decimal(zero: bool, negative: bool, sci: &str, magnitude: f64) -> String {
    let sign = if negative { "-" } else { "" };
    if zero {
        return format!("{sign}0.0");
    }
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if (1e-3..1e7).contains(&magnitude) {
        let point = exp + 1;
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{exp}", &digits[..1], frac)
    }
}

/// Parses SNBT (`TagParser`) into the value `NbtOps` would build: compounds, lists, typed
/// arrays, quoted and bare strings, suffixed numbers (hex/binary, `_` separators, signedness
/// prefixes) and `true`/`false`.
pub fn parse_snbt(s: &str) -> DataResult<Value> {
    let mut p = Snbt { s: s.as_bytes(), i: 0, text: s };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return err(format!("trailing SNBT at {}", p.i));
    }
    Ok(v)
}

struct Snbt<'a> {
    s: &'a [u8],
    i: usize,
    text: &'a str,
}

impl Snbt<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> DataResult<()> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            err(format!("expected {:?} at {} in SNBT", c as char, self.i))
        }
    }

    fn value(&mut self) -> DataResult<Value> {
        match self.peek() {
            Some(b'{') => self.compound(),
            Some(b'[') => self.list(),
            Some(b'"' | b'\'') => self.quoted().map(Value::String),
            Some(_) => {
                let word = self.bare();
                if word.is_empty() {
                    return err(format!("unexpected SNBT at {}", self.i));
                }
                Ok(literal(word))
            }
            None => err("unexpected end of SNBT"),
        }
    }

    fn bare(&mut self) -> &str {
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || b"._+-".contains(&self.s[self.i])) {
            self.i += 1;
        }
        &self.text[start..self.i]
    }

    fn compound(&mut self) -> DataResult<Value> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Map(entries));
        }
        loop {
            let key = match self.peek() {
                Some(b'"' | b'\'') => self.quoted()?,
                _ => self.bare().to_owned(),
            };
            if key.is_empty() && self.s.get(self.i) != Some(&b':') {
                return err(format!("expected key at {} in SNBT", self.i));
            }
            self.expect(b':')?;
            let v = self.value()?;
            entries.push((Value::String(key), v));
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        break;
                    }
                }
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return err(format!("expected ',' or '}}' at {} in SNBT", self.i)),
            }
        }
        Ok(Value::Map(entries))
    }

    fn list(&mut self) -> DataResult<Value> {
        self.expect(b'[')?;
        self.ws();
        let array = match (self.s.get(self.i), self.s.get(self.i + 1)) {
            (Some(&t @ (b'B' | b'I' | b'L')), Some(b';')) => {
                self.i += 2;
                Some(t)
            }
            _ => None,
        };
        let mut items = Vec::new();
        if self.peek() != Some(b']') {
            loop {
                items.push(self.value()?);
                match self.peek() {
                    Some(b',') => {
                        self.i += 1;
                        if self.peek() == Some(b']') {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
        self.expect(b']')?;
        Ok(match array {
            Some(b'B') => Value::ByteList(items.iter().map(Value::as_i8).collect::<DataResult<_>>()?),
            Some(b'I') => Value::IntList(items.iter().map(Value::as_i32).collect::<DataResult<_>>()?),
            Some(_) => Value::LongList(items.iter().map(Value::as_i64).collect::<DataResult<_>>()?),
            None => Value::List(items),
        })
    }

    fn quoted(&mut self) -> DataResult<String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = String::new();
        let rest = &self.text[self.i..];
        let mut chars = rest.char_indices();
        while let Some((off, c)) = chars.next() {
            match c {
                c if c as u32 == q as u32 => {
                    self.i += off + 1;
                    return Ok(out);
                }
                '\\' => {
                    let Some((_, e)) = chars.next() else { break };
                    let mut hex = |n: usize| -> DataResult<char> {
                        let digits: String = (0..n).filter_map(|_| chars.next().map(|(_, c)| c)).collect();
                        u32::from_str_radix(&digits, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or_else(|| DataError(format!("bad SNBT escape {digits:?}")))
                    };
                    out.push(match e {
                        'b' => '\u{8}',
                        't' => '\t',
                        'n' => '\n',
                        'f' => '\u{c}',
                        'r' => '\r',
                        's' => ' ',
                        'x' => hex(2)?,
                        'u' => hex(4)?,
                        'U' => hex(8)?,
                        other => other,
                    });
                }
                c => out.push(c),
            }
        }
        err("unterminated SNBT string")
    }
}

/// A bare SNBT word: a boolean, a number, or else a string.
fn literal(word: &str) -> Value {
    match word {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    number(word).unwrap_or_else(|| Value::str(word))
}

fn number(word: &str) -> Option<Value> {
    let (neg, body) = match word.as_bytes().first()? {
        b'-' => (true, &word[1..]),
        b'+' => (false, &word[1..]),
        _ => (false, word),
    };
    if !body.as_bytes().first()?.is_ascii_digit() && !body.starts_with('.') {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    // Integers: optional base prefix, digits with `_`, optional [us] then b/s/i/l suffix.
    let (radix, digits) = if let Some(d) = lower.strip_prefix("0x") {
        (16, d)
    } else if let Some(d) = lower.strip_prefix("0b").filter(|d| !d.is_empty() && d.bytes().all(|c| matches!(c, b'0' | b'1' | b'_' | b'u' | b's' | b'b' | b'i' | b'l'))) {
        (2, d)
    } else {
        (10, lower.as_str())
    };
    let int_suffix = |d: &str| -> Option<(String, Option<char>, Option<char>)> {
        let mut d = d.to_owned();
        let mut ty = None;
        if let Some(c) = d.chars().last().filter(|c| matches!(c, 'b' | 's' | 'i' | 'l'))
            && (radix != 16 || matches!(c, 'l' | 'i'))
        {
            ty = Some(c);
            d.pop();
        }
        let mut sign = None;
        if ty.is_some()
            && let Some(c) = d.chars().last().filter(|c| matches!(c, 'u' | 's'))
        {
            sign = Some(c);
            d.pop();
        }
        Some((d, ty, sign))
    };
    if let Some((d, ty, sign)) = int_suffix(digits) {
        let clean: String = d.chars().filter(|c| *c != '_').collect();
        if !clean.is_empty() && clean.chars().all(|c| c.is_digit(radix)) {
            let magnitude = i128::from_str_radix(&clean, radix).ok()?;
            let v = if neg { -magnitude } else { magnitude };
            let unsigned = sign == Some('u') || (sign.is_none() && radix != 10);
            return Some(match ty {
                Some('b') => Value::Byte(if unsigned { v as u8 as i8 } else { i8::try_from(v).ok()? }),
                Some('s') => Value::Short(if unsigned { v as u16 as i16 } else { i16::try_from(v).ok()? }),
                Some('l') => Value::Long(if unsigned { v as u64 as i64 } else { i64::try_from(v).ok()? }),
                _ => Value::Int(if unsigned { v as u32 as i32 } else { i32::try_from(v).ok()? }),
            });
        }
    }
    if radix != 10 {
        return None;
    }
    // Floats: digits, '.', exponent, optional f/d suffix.
    let (d, ty) = match lower.chars().last()? {
        c @ ('f' | 'd') => (&lower[..lower.len() - 1], Some(c)),
        _ => (lower.as_str(), None),
    };
    let clean: String = d.chars().filter(|c| *c != '_').collect();
    if !clean.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'+' | b'-')) {
        return None;
    }
    let x: f64 = clean.parse().ok()?;
    let x = if neg { -x } else { x };
    Some(match ty {
        Some('f') => Value::Float(x as f32),
        _ => Value::Double(x),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_number_strings() {
        assert_eq!(java_double(1.0), "1.0");
        assert_eq!(java_double(100.0), "100.0");
        assert_eq!(java_double(0.001), "0.001");
        assert_eq!(java_double(0.0001), "1.0E-4");
        assert_eq!(java_double(1e7), "1.0E7");
        assert_eq!(java_double(1234567.5), "1234567.5");
        assert_eq!(java_double(-2.5e-10), "-2.5E-10");
        assert_eq!(java_float(0.1), "0.1");
        assert_eq!(java_float(3.4028235e38), "3.4028235E38");
        assert_eq!(java_float(-0.0), "-0.0");
    }

    #[test]
    fn snbt_round_trip() {
        let v = Value::Map(vec![
            (Value::str("b"), Value::Byte(1)),
            (Value::str("a"), Value::List(vec![Value::Int(1), Value::Int(2)])),
            (Value::str("x y"), Value::str("it's")),
            (Value::str("true"), Value::Double(1.5)),
            (Value::str("arr"), Value::IntList(vec![1, -2])),
        ]);
        let tag = v.to_nbt();
        let s = snbt(&tag);
        assert_eq!(s, r#"{a:[1,2],arr:[I;1,-2],b:1b,"true":1.5d,"x y":"it's"}"#);
        // SNBT sorts keys; compound order after parsing depends on insertion order.
        assert_eq!(snbt(&parse_snbt(&s).unwrap().to_nbt()), s);
        assert_eq!(snbt(&Tag::String("say \"hi\"".into())), r#"'say "hi"'"#);
        assert_eq!(parse_snbt("[B;1B,2b]").unwrap(), Value::ByteList(vec![1, 2]));
        assert_eq!(parse_snbt("{n:0x1F,m:-3sb,f:2.5e1f,t:true}").unwrap().as_map().unwrap().get("n"), Some(&Value::Int(31)));
    }
}
