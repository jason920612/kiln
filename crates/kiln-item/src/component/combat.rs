//! Weapons, tools and armor: attribute modifiers, tool rules, weapons, attack ranges, blocking,
//! equippable, swing animations, use effects, damage types.

use super::ComponentValue;
use super::common::{SoundEvent, read_sound, sound_from_value, sound_to_value, write_sound};
use crate::enums::{impl_component_enum, string_enum};
use crate::holder::{Holder, HolderSet};
use crate::ident::Identifier;
use crate::registry;
use crate::text::Text;
use crate::value::{DataResult, MapBuilder, MapView, Value};
use crate::wire::{self, WireResult};
use bytes::{BufMut, BytesMut};
use kiln_proto::{Reader, WriteExt};

const UNBOUNDED: usize = i32::MAX as usize;

/// `Float.equals`: bitwise, so `-0.0` differs from `0.0` (what `optionalFieldOf` defaults compare with).
fn same_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// An `optionalFieldOf(key, default)` float field.
fn put_f32(m: &mut MapBuilder, key: &str, v: f32, default: f32) {
    if !same_f32(v, default) {
        m.put(key, Value::Float(v));
    }
}

fn opt_sound(m: &mut MapBuilder, key: &str, sound: &Option<SoundEvent>) {
    if let Some(s) = sound {
        m.put(key, sound_to_value(s));
    }
}

fn get_sound(m: &MapView<'_>, key: &str) -> DataResult<Option<SoundEvent>> {
    m.opt(key, sound_from_value)
}

fn read_opt_sound(r: &mut Reader<'_>) -> WireResult<Option<SoundEvent>> {
    wire::read_opt(r, read_sound)
}

fn write_opt_sound(out: &mut BytesMut, sound: &Option<SoundEvent>) {
    wire::write_opt(out, sound, write_sound);
}

/// A `minecraft:sound_event` entry by name (defaults of some fields).
fn sound_named(name: &str) -> SoundEvent {
    Holder::Reference(registry::SOUND_EVENT.id(name).unwrap_or(0))
}

/// `use_effects`.
#[derive(Debug, Clone, PartialEq)]
pub struct UseEffects {
    pub can_sprint: bool,
    pub interact_vibrations: bool,
    /// Movement speed factor while using the item, 0..=1.
    pub speed_multiplier: f32,
}

impl Default for UseEffects {
    fn default() -> Self {
        UseEffects { can_sprint: false, interact_vibrations: true, speed_multiplier: 0.2 }
    }
}

impl ComponentValue for UseEffects {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(UseEffects { can_sprint: r.bool()?, interact_vibrations: r.bool()?, speed_multiplier: r.f32()? })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_bool(self.can_sprint);
        out.put_bool(self.interact_vibrations);
        out.put_f32(self.speed_multiplier);
    }
    fn to_value(&self) -> Value {
        let d = UseEffects::default();
        let mut m = MapBuilder::new();
        m.opt_default("can_sprint", self.can_sprint, d.can_sprint, Value::Bool)
            .opt_default("interact_vibrations", self.interact_vibrations, d.interact_vibrations, Value::Bool);
        put_f32(&mut m, "speed_multiplier", self.speed_multiplier, d.speed_multiplier);
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let d = UseEffects::default();
        Ok(UseEffects {
            can_sprint: m.opt_or("can_sprint", d.can_sprint, Value::as_bool)?,
            interact_vibrations: m.opt_or("interact_vibrations", d.interact_vibrations, Value::as_bool)?,
            speed_multiplier: m.opt_or("speed_multiplier", d.speed_multiplier, Value::as_f32)?,
        })
    }
}

/// `damage_type`: a `minecraft:damage_type` entry (network id); no inline definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageTypeRef(pub i32);

impl ComponentValue for DamageTypeRef {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        registry::DAMAGE_TYPE.read_id(r).map(DamageTypeRef)
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.0);
    }
    fn to_value(&self) -> Value {
        registry::DAMAGE_TYPE.id_to_value(self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        registry::DAMAGE_TYPE.id_from_value(v).map(DamageTypeRef)
    }
}

string_enum! {
    /// `EquipmentSlotGroup`.
    pub enum EquipmentSlotGroup {
        Any = "any", MainHand = "mainhand", OffHand = "offhand", Hand = "hand", Feet = "feet", Legs = "legs",
        Chest = "chest", Head = "head", Armor = "armor", Body = "body", Saddle = "saddle",
    }
}

string_enum! {
    /// `AttributeModifier.Operation`.
    pub enum AttributeOperation {
        AddValue = "add_value", AddMultipliedBase = "add_multiplied_base", AddMultipliedTotal = "add_multiplied_total",
    }
}

string_enum! {
    /// `SwingAnimationType`.
    pub enum SwingAnimationType { None = "none", Whack = "whack", Stab = "stab" }
}

string_enum! {
    enum DisplayType { Default = "default", Hidden = "hidden", Override = "override" }
}

impl_component_enum!(EquipmentSlotGroup);

/// `EquipmentSlot`. Its network id is not its ordinal: mainhand 0, feet 1, legs 2, chest 3,
/// head 4, offhand 5, body 6, saddle 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EquipmentSlot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}

impl EquipmentSlot {
    const BY_ID: [EquipmentSlot; 8] = [
        EquipmentSlot::MainHand,
        EquipmentSlot::Feet,
        EquipmentSlot::Legs,
        EquipmentSlot::Chest,
        EquipmentSlot::Head,
        EquipmentSlot::OffHand,
        EquipmentSlot::Body,
        EquipmentSlot::Saddle,
    ];

    pub fn id(self) -> i32 {
        Self::BY_ID.iter().position(|s| *s == self).unwrap() as i32
    }

    /// Out-of-range ids map to `mainhand` (`ByIdMap` `ZERO`).
    pub fn from_id(id: i32) -> Self {
        usize::try_from(id).ok().and_then(|i| Self::BY_ID.get(i)).copied().unwrap_or(EquipmentSlot::MainHand)
    }

    pub fn name(self) -> &'static str {
        match self {
            EquipmentSlot::MainHand => "mainhand",
            EquipmentSlot::OffHand => "offhand",
            EquipmentSlot::Feet => "feet",
            EquipmentSlot::Legs => "legs",
            EquipmentSlot::Chest => "chest",
            EquipmentSlot::Head => "head",
            EquipmentSlot::Body => "body",
            EquipmentSlot::Saddle => "saddle",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::BY_ID.iter().copied().find(|v| v.name() == s)
    }

    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Self::from_id(r.varint()?))
    }

    pub fn to_value(self) -> Value {
        Value::str(self.name())
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let s = v.as_str()?;
        Self::from_name(s).ok_or_else(|| crate::value::DataError(format!("unknown equipment slot {s:?}")))
    }
}

/// How an attribute modifier appears in the tooltip (`ItemAttributeModifiers.Display`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AttributeDisplay {
    #[default]
    Default,
    Hidden,
    Override(Text),
}

impl AttributeDisplay {
    fn kind(&self) -> DisplayType {
        match self {
            AttributeDisplay::Default => DisplayType::Default,
            AttributeDisplay::Hidden => DisplayType::Hidden,
            AttributeDisplay::Override(_) => DisplayType::Override,
        }
    }

    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(match DisplayType::read(r)? {
            DisplayType::Default => AttributeDisplay::Default,
            DisplayType::Hidden => AttributeDisplay::Hidden,
            DisplayType::Override => AttributeDisplay::Override(Text::read(r)?),
        })
    }

    fn write(&self, out: &mut BytesMut) {
        self.kind().write(out);
        if let AttributeDisplay::Override(t) = self {
            t.write(out);
        }
    }

    /// Dispatched on `type`.
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("type", self.kind().to_value());
        if let AttributeDisplay::Override(t) = self {
            m.put("value", t.to_value());
        }
        m.build()
    }

    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(match m.req_with("type", DisplayType::from_value)? {
            DisplayType::Default => AttributeDisplay::Default,
            DisplayType::Hidden => AttributeDisplay::Hidden,
            DisplayType::Override => AttributeDisplay::Override(m.req_with("value", Text::from_value)?),
        })
    }
}

/// One entry of `attribute_modifiers`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeModifier {
    /// `minecraft:attribute` network id.
    pub attribute: i32,
    pub id: Identifier,
    pub amount: f64,
    pub operation: AttributeOperation,
    pub slot: EquipmentSlotGroup,
    pub display: AttributeDisplay,
}

impl AttributeModifier {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(AttributeModifier {
            attribute: registry::ATTRIBUTE.read_id(r)?,
            id: Identifier::read(r)?,
            amount: r.f64()?,
            operation: AttributeOperation::read(r)?,
            slot: EquipmentSlotGroup::read(r)?,
            display: AttributeDisplay::read(r)?,
        })
    }

    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.attribute);
        self.id.write(out);
        out.put_f64(self.amount);
        self.operation.write(out);
        self.slot.write(out);
        self.display.write(out);
    }

    /// `{type, id, amount, operation, slot, display}` (the modifier's map codec is inlined).
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("type", registry::ATTRIBUTE.id_to_value(self.attribute))
            .put("id", self.id.to_value())
            .put("amount", Value::Double(self.amount))
            .put("operation", self.operation.to_value())
            .opt_default("slot", self.slot, EquipmentSlotGroup::Any, EquipmentSlotGroup::to_value);
        if self.display != AttributeDisplay::Default {
            m.put("display", self.display.to_value());
        }
        m.build()
    }

    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(AttributeModifier {
            attribute: m.req_with("type", |v| registry::ATTRIBUTE.id_from_value(v))?,
            id: m.req_with("id", Identifier::from_value)?,
            amount: m.req_with("amount", Value::as_f64)?,
            operation: m.req_with("operation", AttributeOperation::from_value)?,
            slot: m.opt_or("slot", EquipmentSlotGroup::Any, EquipmentSlotGroup::from_value)?,
            display: m.opt_or("display", AttributeDisplay::Default, AttributeDisplay::from_value)?,
        })
    }
}

/// `attribute_modifiers`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributeModifiers(pub Vec<AttributeModifier>);

impl ComponentValue for AttributeModifiers {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, UNBOUNDED, AttributeModifier::read).map(AttributeModifiers)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, AttributeModifier::write);
    }
    fn to_value(&self) -> Value {
        Value::List(self.0.iter().map(AttributeModifier::to_value).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_list()?.iter().map(AttributeModifier::from_value).collect::<DataResult<_>>().map(AttributeModifiers)
    }
}

/// `damage_resistant`: damage types that do not hurt the item (as an entity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageResistant {
    /// `minecraft:damage_type` entries.
    pub types: HolderSet,
}

impl ComponentValue for DamageResistant {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        HolderSet::read(registry::DAMAGE_TYPE, r).map(|types| DamageResistant { types })
    }
    fn write(&self, out: &mut BytesMut) {
        self.types.write(out);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new().put("types", self.types.to_value(registry::DAMAGE_TYPE)).build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Ok(DamageResistant { types: v.as_map()?.req_with("types", |v| HolderSet::from_value(registry::DAMAGE_TYPE, v))? })
    }
}

/// A mining rule of `tool`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRule {
    /// `minecraft:block` entries.
    pub blocks: HolderSet,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}

impl ToolRule {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(ToolRule {
            blocks: HolderSet::read(registry::BLOCK, r)?,
            speed: wire::read_opt(r, |r| r.f32())?,
            correct_for_drops: wire::read_opt(r, |r| r.bool())?,
        })
    }

    fn write(&self, out: &mut BytesMut) {
        self.blocks.write(out);
        wire::write_opt(out, &self.speed, |v, o| o.put_f32(*v));
        wire::write_opt(out, &self.correct_for_drops, |v, o| o.put_bool(*v));
    }

    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("blocks", self.blocks.to_value(registry::BLOCK))
            .opt("speed", self.speed, Value::Float)
            .opt("correct_for_drops", self.correct_for_drops, Value::Bool)
            .build()
    }

    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(ToolRule {
            blocks: m.req_with("blocks", |v| HolderSet::from_value(registry::BLOCK, v))?,
            speed: m.opt("speed", Value::as_f32)?,
            correct_for_drops: m.opt("correct_for_drops", Value::as_bool)?,
        })
    }
}

/// `tool`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub rules: Vec<ToolRule>,
    pub default_mining_speed: f32,
    pub damage_per_block: i32,
    pub can_destroy_blocks_in_creative: bool,
}

impl ComponentValue for Tool {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Tool {
            rules: wire::read_list(r, UNBOUNDED, ToolRule::read)?,
            default_mining_speed: r.f32()?,
            damage_per_block: r.varint()?,
            can_destroy_blocks_in_creative: r.bool()?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.rules, ToolRule::write);
        out.put_f32(self.default_mining_speed);
        out.put_varint(self.damage_per_block);
        out.put_bool(self.can_destroy_blocks_in_creative);
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("rules", Value::List(self.rules.iter().map(ToolRule::to_value).collect()));
        put_f32(&mut m, "default_mining_speed", self.default_mining_speed, 1.0);
        m.opt_default("damage_per_block", self.damage_per_block, 1, Value::Int)
            .opt_default("can_destroy_blocks_in_creative", self.can_destroy_blocks_in_creative, true, Value::Bool)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(Tool {
            rules: m.req_with("rules", |l| l.as_list()?.iter().map(ToolRule::from_value).collect())?,
            default_mining_speed: m.opt_or("default_mining_speed", 1.0, Value::as_f32)?,
            damage_per_block: m.opt_or("damage_per_block", 1, Value::as_i32)?,
            can_destroy_blocks_in_creative: m.opt_or("can_destroy_blocks_in_creative", true, Value::as_bool)?,
        })
    }
}

/// `weapon`.
#[derive(Debug, Clone, PartialEq)]
pub struct Weapon {
    pub item_damage_per_attack: i32,
    pub disable_blocking_for_seconds: f32,
}

impl Default for Weapon {
    fn default() -> Self {
        Weapon { item_damage_per_attack: 1, disable_blocking_for_seconds: 0.0 }
    }
}

impl ComponentValue for Weapon {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Weapon { item_damage_per_attack: r.varint()?, disable_blocking_for_seconds: r.f32()? })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.item_damage_per_attack);
        out.put_f32(self.disable_blocking_for_seconds);
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.opt_default("item_damage_per_attack", self.item_damage_per_attack, 1, Value::Int);
        put_f32(&mut m, "disable_blocking_for_seconds", self.disable_blocking_for_seconds, 0.0);
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(Weapon {
            item_damage_per_attack: m.opt_or("item_damage_per_attack", 1, Value::as_i32)?,
            disable_blocking_for_seconds: m.opt_or("disable_blocking_for_seconds", 0.0, Value::as_f32)?,
        })
    }
}

/// `attack_range`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackRange {
    pub min_reach: f32,
    pub max_reach: f32,
    pub min_creative_reach: f32,
    pub max_creative_reach: f32,
    pub hitbox_margin: f32,
    pub mob_factor: f32,
}

impl Default for AttackRange {
    fn default() -> Self {
        AttackRange {
            min_reach: 0.0,
            max_reach: 3.0,
            min_creative_reach: 0.0,
            max_creative_reach: 5.0,
            hitbox_margin: 0.3,
            mob_factor: 1.0,
        }
    }
}

impl AttackRange {
    fn fields(&self) -> [(&'static str, f32); 6] {
        [
            ("min_reach", self.min_reach),
            ("max_reach", self.max_reach),
            ("min_creative_reach", self.min_creative_reach),
            ("max_creative_reach", self.max_creative_reach),
            ("hitbox_margin", self.hitbox_margin),
            ("mob_factor", self.mob_factor),
        ]
    }
}

impl ComponentValue for AttackRange {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(AttackRange {
            min_reach: r.f32()?,
            max_reach: r.f32()?,
            min_creative_reach: r.f32()?,
            max_creative_reach: r.f32()?,
            hitbox_margin: r.f32()?,
            mob_factor: r.f32()?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        for (_, v) in self.fields() {
            out.put_f32(v);
        }
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        for ((key, v), (_, d)) in self.fields().into_iter().zip(AttackRange::default().fields()) {
            put_f32(&mut m, key, v, d);
        }
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let d = AttackRange::default();
        Ok(AttackRange {
            min_reach: m.opt_or("min_reach", d.min_reach, Value::as_f32)?,
            max_reach: m.opt_or("max_reach", d.max_reach, Value::as_f32)?,
            min_creative_reach: m.opt_or("min_creative_reach", d.min_creative_reach, Value::as_f32)?,
            max_creative_reach: m.opt_or("max_creative_reach", d.max_creative_reach, Value::as_f32)?,
            hitbox_margin: m.opt_or("hitbox_margin", d.hitbox_margin, Value::as_f32)?,
            mob_factor: m.opt_or("mob_factor", d.mob_factor, Value::as_f32)?,
        })
    }
}

/// `equippable`.
#[derive(Debug, Clone, PartialEq)]
pub struct Equippable {
    pub slot: EquipmentSlot,
    pub equip_sound: SoundEvent,
    /// Equipment asset (`minecraft:equipment/...` key) rendered when worn.
    pub asset_id: Option<Identifier>,
    pub camera_overlay: Option<Identifier>,
    /// `minecraft:entity_type` entries that may wear it.
    pub allowed_entities: Option<HolderSet>,
    pub dispensable: bool,
    pub swappable: bool,
    pub damage_on_hurt: bool,
    pub equip_on_interact: bool,
    pub can_be_sheared: bool,
    pub shearing_sound: SoundEvent,
}

const DEFAULT_EQUIP_SOUND: &str = "minecraft:item.armor.equip_generic";
const DEFAULT_SHEARING_SOUND: &str = "minecraft:item.shears.snip";

impl Equippable {
    pub fn new(slot: EquipmentSlot) -> Self {
        Equippable {
            slot,
            equip_sound: sound_named(DEFAULT_EQUIP_SOUND),
            asset_id: None,
            camera_overlay: None,
            allowed_entities: None,
            dispensable: true,
            swappable: true,
            damage_on_hurt: true,
            equip_on_interact: false,
            can_be_sheared: false,
            shearing_sound: sound_named(DEFAULT_SHEARING_SOUND),
        }
    }
}

impl ComponentValue for Equippable {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Equippable {
            slot: EquipmentSlot::read(r)?,
            equip_sound: read_sound(r)?,
            asset_id: wire::read_opt(r, Identifier::read)?,
            camera_overlay: wire::read_opt(r, Identifier::read)?,
            allowed_entities: wire::read_opt(r, |r| HolderSet::read(registry::ENTITY_TYPE, r))?,
            dispensable: r.bool()?,
            swappable: r.bool()?,
            damage_on_hurt: r.bool()?,
            equip_on_interact: r.bool()?,
            can_be_sheared: r.bool()?,
            shearing_sound: read_sound(r)?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.slot.id());
        write_sound(&self.equip_sound, out);
        wire::write_opt(out, &self.asset_id, Identifier::write);
        wire::write_opt(out, &self.camera_overlay, Identifier::write);
        wire::write_opt(out, &self.allowed_entities, HolderSet::write);
        out.put_bool(self.dispensable);
        out.put_bool(self.swappable);
        out.put_bool(self.damage_on_hurt);
        out.put_bool(self.equip_on_interact);
        out.put_bool(self.can_be_sheared);
        write_sound(&self.shearing_sound, out);
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("slot", self.slot.to_value());
        if self.equip_sound != sound_named(DEFAULT_EQUIP_SOUND) {
            m.put("equip_sound", sound_to_value(&self.equip_sound));
        }
        m.opt("asset_id", self.asset_id.as_ref(), Identifier::to_value)
            .opt("camera_overlay", self.camera_overlay.as_ref(), Identifier::to_value)
            .opt("allowed_entities", self.allowed_entities.as_ref(), |s| s.to_value(registry::ENTITY_TYPE))
            .opt_default("dispensable", self.dispensable, true, Value::Bool)
            .opt_default("swappable", self.swappable, true, Value::Bool)
            .opt_default("damage_on_hurt", self.damage_on_hurt, true, Value::Bool)
            .opt_default("equip_on_interact", self.equip_on_interact, false, Value::Bool)
            .opt_default("can_be_sheared", self.can_be_sheared, false, Value::Bool);
        if self.shearing_sound != sound_named(DEFAULT_SHEARING_SOUND) {
            m.put("shearing_sound", sound_to_value(&self.shearing_sound));
        }
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(Equippable {
            slot: m.req_with("slot", EquipmentSlot::from_value)?,
            equip_sound: m.opt_or("equip_sound", sound_named(DEFAULT_EQUIP_SOUND), sound_from_value)?,
            asset_id: m.opt("asset_id", Identifier::from_value)?,
            camera_overlay: m.opt("camera_overlay", Identifier::from_value)?,
            allowed_entities: m.opt("allowed_entities", |v| HolderSet::from_value(registry::ENTITY_TYPE, v))?,
            dispensable: m.opt_or("dispensable", true, Value::as_bool)?,
            swappable: m.opt_or("swappable", true, Value::as_bool)?,
            damage_on_hurt: m.opt_or("damage_on_hurt", true, Value::as_bool)?,
            equip_on_interact: m.opt_or("equip_on_interact", false, Value::as_bool)?,
            can_be_sheared: m.opt_or("can_be_sheared", false, Value::as_bool)?,
            shearing_sound: m.opt_or("shearing_sound", sound_named(DEFAULT_SHEARING_SOUND), sound_from_value)?,
        })
    }
}

/// A damage reduction of `blocks_attacks`.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageReduction {
    pub horizontal_blocking_angle: f32,
    /// `minecraft:damage_type` entries it applies to; all when absent.
    pub types: Option<HolderSet>,
    pub base: f32,
    pub factor: f32,
}

impl Default for DamageReduction {
    fn default() -> Self {
        DamageReduction { horizontal_blocking_angle: 90.0, types: None, base: 0.0, factor: 1.0 }
    }
}

impl DamageReduction {
    fn same(&self, o: &DamageReduction) -> bool {
        same_f32(self.horizontal_blocking_angle, o.horizontal_blocking_angle)
            && self.types == o.types
            && same_f32(self.base, o.base)
            && same_f32(self.factor, o.factor)
    }

    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(DamageReduction {
            horizontal_blocking_angle: r.f32()?,
            types: wire::read_opt(r, |r| HolderSet::read(registry::DAMAGE_TYPE, r))?,
            base: r.f32()?,
            factor: r.f32()?,
        })
    }

    fn write(&self, out: &mut BytesMut) {
        out.put_f32(self.horizontal_blocking_angle);
        wire::write_opt(out, &self.types, HolderSet::write);
        out.put_f32(self.base);
        out.put_f32(self.factor);
    }

    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        put_f32(&mut m, "horizontal_blocking_angle", self.horizontal_blocking_angle, 90.0);
        m.opt("type", self.types.as_ref(), |s| s.to_value(registry::DAMAGE_TYPE))
            .put("base", Value::Float(self.base))
            .put("factor", Value::Float(self.factor))
            .build()
    }

    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(DamageReduction {
            horizontal_blocking_angle: m.opt_or("horizontal_blocking_angle", 90.0, Value::as_f32)?,
            types: m.opt("type", |v| HolderSet::from_value(registry::DAMAGE_TYPE, v))?,
            base: m.req_with("base", Value::as_f32)?,
            factor: m.req_with("factor", Value::as_f32)?,
        })
    }
}

/// Durability lost when blocking (`BlocksAttacks.ItemDamageFunction`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDamageFunction {
    pub threshold: f32,
    pub base: f32,
    pub factor: f32,
}

impl Default for ItemDamageFunction {
    fn default() -> Self {
        ItemDamageFunction { threshold: 1.0, base: 0.0, factor: 1.0 }
    }
}

impl ItemDamageFunction {
    fn is_default(&self) -> bool {
        let d = ItemDamageFunction::default();
        same_f32(self.threshold, d.threshold) && same_f32(self.base, d.base) && same_f32(self.factor, d.factor)
    }

    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("threshold", Value::Float(self.threshold))
            .put("base", Value::Float(self.base))
            .put("factor", Value::Float(self.factor))
            .build()
    }

    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(ItemDamageFunction {
            threshold: m.req_with("threshold", Value::as_f32)?,
            base: m.req_with("base", Value::as_f32)?,
            factor: m.req_with("factor", Value::as_f32)?,
        })
    }
}

/// `blocks_attacks`.
#[derive(Debug, Clone, PartialEq)]
pub struct BlocksAttacks {
    pub block_delay_seconds: f32,
    pub disable_cooldown_scale: f32,
    pub damage_reductions: Vec<DamageReduction>,
    pub item_damage: ItemDamageFunction,
    /// `minecraft:damage_type` entries that bypass blocking.
    pub bypassed_by: Option<HolderSet>,
    pub block_sound: Option<SoundEvent>,
    pub disabled_sound: Option<SoundEvent>,
}

impl ComponentValue for BlocksAttacks {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(BlocksAttacks {
            block_delay_seconds: r.f32()?,
            disable_cooldown_scale: r.f32()?,
            damage_reductions: wire::read_list(r, UNBOUNDED, DamageReduction::read)?,
            item_damage: ItemDamageFunction { threshold: r.f32()?, base: r.f32()?, factor: r.f32()? },
            bypassed_by: wire::read_opt(r, |r| HolderSet::read(registry::DAMAGE_TYPE, r))?,
            block_sound: read_opt_sound(r)?,
            disabled_sound: read_opt_sound(r)?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_f32(self.block_delay_seconds);
        out.put_f32(self.disable_cooldown_scale);
        wire::write_list(out, &self.damage_reductions, DamageReduction::write);
        out.put_f32(self.item_damage.threshold);
        out.put_f32(self.item_damage.base);
        out.put_f32(self.item_damage.factor);
        wire::write_opt(out, &self.bypassed_by, HolderSet::write);
        write_opt_sound(out, &self.block_sound);
        write_opt_sound(out, &self.disabled_sound);
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        put_f32(&mut m, "block_delay_seconds", self.block_delay_seconds, 0.0);
        put_f32(&mut m, "disable_cooldown_scale", self.disable_cooldown_scale, 1.0);
        let default_reductions = self.damage_reductions.len() == 1 && self.damage_reductions[0].same(&DamageReduction::default());
        if !default_reductions {
            m.put("damage_reductions", Value::List(self.damage_reductions.iter().map(DamageReduction::to_value).collect()));
        }
        if !self.item_damage.is_default() {
            m.put("item_damage", self.item_damage.to_value());
        }
        m.opt("bypassed_by", self.bypassed_by.as_ref(), |s| s.to_value(registry::DAMAGE_TYPE));
        opt_sound(&mut m, "block_sound", &self.block_sound);
        opt_sound(&mut m, "disabled_sound", &self.disabled_sound);
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(BlocksAttacks {
            block_delay_seconds: m.opt_or("block_delay_seconds", 0.0, Value::as_f32)?,
            disable_cooldown_scale: m.opt_or("disable_cooldown_scale", 1.0, Value::as_f32)?,
            damage_reductions: m.opt_or("damage_reductions", vec![DamageReduction::default()], |l| {
                l.as_list()?.iter().map(DamageReduction::from_value).collect()
            })?,
            item_damage: m.opt_or("item_damage", ItemDamageFunction::default(), ItemDamageFunction::from_value)?,
            bypassed_by: m.opt("bypassed_by", |v| HolderSet::from_value(registry::DAMAGE_TYPE, v))?,
            block_sound: get_sound(&m, "block_sound")?,
            disabled_sound: get_sound(&m, "disabled_sound")?,
        })
    }
}

/// `piercing_weapon`.
#[derive(Debug, Clone, PartialEq)]
pub struct PiercingWeapon {
    pub deals_knockback: bool,
    pub dismounts: bool,
    pub sound: Option<SoundEvent>,
    pub hit_sound: Option<SoundEvent>,
}

impl ComponentValue for PiercingWeapon {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(PiercingWeapon {
            deals_knockback: r.bool()?,
            dismounts: r.bool()?,
            sound: read_opt_sound(r)?,
            hit_sound: read_opt_sound(r)?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_bool(self.deals_knockback);
        out.put_bool(self.dismounts);
        write_opt_sound(out, &self.sound);
        write_opt_sound(out, &self.hit_sound);
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.opt_default("deals_knockback", self.deals_knockback, true, Value::Bool)
            .opt_default("dismounts", self.dismounts, false, Value::Bool);
        opt_sound(&mut m, "sound", &self.sound);
        opt_sound(&mut m, "hit_sound", &self.hit_sound);
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(PiercingWeapon {
            deals_knockback: m.opt_or("deals_knockback", true, Value::as_bool)?,
            dismounts: m.opt_or("dismounts", false, Value::as_bool)?,
            sound: get_sound(&m, "sound")?,
            hit_sound: get_sound(&m, "hit_sound")?,
        })
    }
}

/// A speed condition of `kinetic_weapon` (`KineticWeapon.Condition`).
#[derive(Debug, Clone, PartialEq)]
pub struct KineticCondition {
    pub max_duration_ticks: i32,
    pub min_speed: f32,
    pub min_relative_speed: f32,
}

impl KineticCondition {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(KineticCondition { max_duration_ticks: r.varint()?, min_speed: r.f32()?, min_relative_speed: r.f32()? })
    }

    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.max_duration_ticks);
        out.put_f32(self.min_speed);
        out.put_f32(self.min_relative_speed);
    }

    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("max_duration_ticks", Value::Int(self.max_duration_ticks));
        put_f32(&mut m, "min_speed", self.min_speed, 0.0);
        put_f32(&mut m, "min_relative_speed", self.min_relative_speed, 0.0);
        m.build()
    }

    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(KineticCondition {
            max_duration_ticks: m.req_with("max_duration_ticks", Value::as_i32)?,
            min_speed: m.opt_or("min_speed", 0.0, Value::as_f32)?,
            min_relative_speed: m.opt_or("min_relative_speed", 0.0, Value::as_f32)?,
        })
    }
}

/// `kinetic_weapon`.
#[derive(Debug, Clone, PartialEq)]
pub struct KineticWeapon {
    pub contact_cooldown_ticks: i32,
    pub delay_ticks: i32,
    pub dismount_conditions: Option<KineticCondition>,
    pub knockback_conditions: Option<KineticCondition>,
    pub damage_conditions: Option<KineticCondition>,
    pub forward_movement: f32,
    pub damage_multiplier: f32,
    pub sound: Option<SoundEvent>,
    pub hit_sound: Option<SoundEvent>,
}

impl ComponentValue for KineticWeapon {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(KineticWeapon {
            contact_cooldown_ticks: r.varint()?,
            delay_ticks: r.varint()?,
            dismount_conditions: wire::read_opt(r, KineticCondition::read)?,
            knockback_conditions: wire::read_opt(r, KineticCondition::read)?,
            damage_conditions: wire::read_opt(r, KineticCondition::read)?,
            forward_movement: r.f32()?,
            damage_multiplier: r.f32()?,
            sound: read_opt_sound(r)?,
            hit_sound: read_opt_sound(r)?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.contact_cooldown_ticks);
        out.put_varint(self.delay_ticks);
        wire::write_opt(out, &self.dismount_conditions, KineticCondition::write);
        wire::write_opt(out, &self.knockback_conditions, KineticCondition::write);
        wire::write_opt(out, &self.damage_conditions, KineticCondition::write);
        out.put_f32(self.forward_movement);
        out.put_f32(self.damage_multiplier);
        write_opt_sound(out, &self.sound);
        write_opt_sound(out, &self.hit_sound);
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.opt_default("contact_cooldown_ticks", self.contact_cooldown_ticks, 10, Value::Int)
            .opt_default("delay_ticks", self.delay_ticks, 0, Value::Int)
            .opt("dismount_conditions", self.dismount_conditions.as_ref(), KineticCondition::to_value)
            .opt("knockback_conditions", self.knockback_conditions.as_ref(), KineticCondition::to_value)
            .opt("damage_conditions", self.damage_conditions.as_ref(), KineticCondition::to_value);
        put_f32(&mut m, "forward_movement", self.forward_movement, 0.0);
        put_f32(&mut m, "damage_multiplier", self.damage_multiplier, 1.0);
        opt_sound(&mut m, "sound", &self.sound);
        opt_sound(&mut m, "hit_sound", &self.hit_sound);
        m.build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(KineticWeapon {
            contact_cooldown_ticks: m.opt_or("contact_cooldown_ticks", 10, Value::as_i32)?,
            delay_ticks: m.opt_or("delay_ticks", 0, Value::as_i32)?,
            dismount_conditions: m.opt("dismount_conditions", KineticCondition::from_value)?,
            knockback_conditions: m.opt("knockback_conditions", KineticCondition::from_value)?,
            damage_conditions: m.opt("damage_conditions", KineticCondition::from_value)?,
            forward_movement: m.opt_or("forward_movement", 0.0, Value::as_f32)?,
            damage_multiplier: m.opt_or("damage_multiplier", 1.0, Value::as_f32)?,
            sound: get_sound(&m, "sound")?,
            hit_sound: get_sound(&m, "hit_sound")?,
        })
    }
}

/// `attack_animation`, `interact_animation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwingAnimation {
    pub kind: SwingAnimationType,
    /// Ticks.
    pub duration: i32,
}

impl Default for SwingAnimation {
    fn default() -> Self {
        SwingAnimation { kind: SwingAnimationType::Whack, duration: 6 }
    }
}

impl ComponentValue for SwingAnimation {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(SwingAnimation { kind: SwingAnimationType::read(r)?, duration: r.varint()? })
    }
    fn write(&self, out: &mut BytesMut) {
        self.kind.write(out);
        out.put_varint(self.duration);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("type", self.kind, SwingAnimationType::Whack, SwingAnimationType::to_value)
            .opt_default("duration", self.duration, 6, Value::Int)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(SwingAnimation {
            kind: m.opt_or("type", SwingAnimationType::Whack, SwingAnimationType::from_value)?,
            duration: m.opt_or("duration", 6, Value::as_i32)?,
        })
    }
}
