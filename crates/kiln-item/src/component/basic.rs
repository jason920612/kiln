//! Simple components: custom data, names and lore, rarity, model data, tooltip display, dye
//! colors, map ids, the ominous bottle amplifier.

use super::{ComponentValue, read_via_nbt, write_via_nbt};
use crate::enums::{impl_component_enum, string_enum};
use crate::registry;
use crate::text::Text;
use crate::value::{DataResult, MapBuilder, Value, err};
use crate::wire::{self, WireResult};
use bytes::{BufMut, BytesMut};
use kiln_proto::nbt::Tag;
use kiln_proto::{DecodeError, Reader, WriteExt};

const UNBOUNDED: usize = i32::MAX as usize;

/// `custom_data`, `bucket_entity_data`: an arbitrary compound (kept as read, so it
/// round-trips byte-exactly).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomData(pub Tag);

impl CustomData {
    pub fn empty() -> Self {
        CustomData(Tag::Compound(Vec::new()))
    }
}

impl ComponentValue for CustomData {
    /// `ByteBufCodecs.COMPOUND_TAG` (`custom_data` itself uses its codec as NBT: the same bytes).
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        match wire::read_nbt(r)? {
            tag @ Tag::Compound(_) => Ok(CustomData(tag)),
            _ => Err(DecodeError::Invalid("custom data must be a compound")),
        }
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_nbt(out, &self.0);
    }
    fn to_value(&self) -> Value {
        Value::from_nbt(&self.0)
    }
    /// A compound, or (`TagParser.FLATTENED_CODEC`) a string of SNBT, which is not supported.
    fn from_value(v: &Value) -> DataResult<Self> {
        match v {
            Value::Map(_) => Ok(CustomData(v.to_nbt())),
            Value::String(_) => err("SNBT custom data strings are not supported"),
            _ => err("custom data must be a compound"),
        }
    }
}

/// `lore`: at most 256 lines.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lore(pub Vec<Text>);

impl ComponentValue for Lore {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, 256, Text::read).map(Lore)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, Text::write);
    }
    fn to_value(&self) -> Value {
        Value::List(self.0.iter().map(Text::to_value).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let lines = v.as_list()?;
        if lines.len() > 256 {
            return err("lore has more than 256 lines");
        }
        lines.iter().map(Text::from_value).collect::<DataResult<_>>().map(Lore)
    }
}

string_enum! {
    pub enum Rarity { Common = "common", Uncommon = "uncommon", Rare = "rare", Epic = "epic" }
}

string_enum! {
    pub enum DyeColor {
        White = "white", Orange = "orange", Magenta = "magenta", LightBlue = "light_blue", Yellow = "yellow",
        Lime = "lime", Pink = "pink", Gray = "gray", LightGray = "light_gray", Cyan = "cyan", Purple = "purple",
        Blue = "blue", Brown = "brown", Green = "green", Red = "red", Black = "black",
    }
}

string_enum! {
    /// `map_post_processing` (transient).
    pub enum MapPostProcessing { Lock = "lock", Scale = "scale" }
}

impl_component_enum!(Rarity, DyeColor, MapPostProcessing);

/// `custom_model_data`: values for item model selectors.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomModelData {
    pub floats: Vec<f32>,
    pub flags: Vec<bool>,
    pub strings: Vec<String>,
    /// RGB colors.
    pub colors: Vec<i32>,
}

impl ComponentValue for CustomModelData {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(CustomModelData {
            floats: wire::read_list(r, UNBOUNDED, |r| r.f32())?,
            flags: wire::read_list(r, UNBOUNDED, |r| r.bool())?,
            strings: wire::read_list(r, UNBOUNDED, |r| wire::read_string(r, 32767))?,
            colors: wire::read_list(r, UNBOUNDED, |r| r.i32())?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.floats, |v, o| o.put_f32(*v));
        wire::write_list(out, &self.flags, |v, o| o.put_bool(*v));
        wire::write_list(out, &self.strings, |v, o| o.put_string(v));
        wire::write_list(out, &self.colors, |v, o| o.put_i32(*v));
    }
    fn to_value(&self) -> Value {
        let list = |items: Vec<Value>| Value::List(items);
        MapBuilder::new()
            .opt_default("floats", &self.floats, &Vec::new(), |v| list(v.iter().map(|&f| Value::Float(f)).collect()))
            .opt_default("flags", &self.flags, &Vec::new(), |v| list(v.iter().map(|&b| Value::Bool(b)).collect()))
            .opt_default("strings", &self.strings, &Vec::new(), |v| list(v.iter().map(Value::str).collect()))
            .opt_default("colors", &self.colors, &Vec::new(), |v| list(v.iter().map(|&c| Value::Int(c)).collect()))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let each = |key, f: fn(&Value) -> DataResult<_>| m.opt_or(key, Vec::new(), |l| l.as_list()?.iter().map(f).collect());
        Ok(CustomModelData {
            floats: m.opt_or("floats", Vec::new(), |l| l.as_list()?.iter().map(Value::as_f32).collect())?,
            flags: m.opt_or("flags", Vec::new(), |l| l.as_list()?.iter().map(Value::as_bool).collect())?,
            strings: m.opt_or("strings", Vec::new(), |l| l.as_list()?.iter().map(|s| s.as_str().map(str::to_owned)).collect())?,
            colors: each("colors", rgb_from_value)?,
        })
    }
}

/// `ExtraCodecs.RGB_COLOR_CODEC`: an int, or three floats in 0..1.
pub fn rgb_from_value(v: &Value) -> DataResult<i32> {
    match v {
        Value::List(_) => {
            let c = v.as_list()?;
            if c.len() != 3 {
                return err("RGB color needs 3 components");
            }
            let ch = |i: usize| -> DataResult<i32> { Ok((c[i].as_f32()? * 255.0) as i32 & 0xff) };
            Ok((ch(0)? << 16) | (ch(1)? << 8) | ch(2)?)
        }
        _ => v.as_i32(),
    }
}

/// `tooltip_display`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TooltipDisplay {
    pub hide_tooltip: bool,
    /// Component types whose tooltip lines are hidden, in insertion order.
    pub hidden_components: Vec<super::ComponentId>,
}

impl ComponentValue for TooltipDisplay {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let hide_tooltip = r.bool()?;
        let mut hidden_components: Vec<super::ComponentId> =
            wire::read_list(r, UNBOUNDED, |r| registry::DATA_COMPONENT_TYPE.read_id(r).map(|i| i as super::ComponentId))?;
        dedup_in_order(&mut hidden_components);
        Ok(TooltipDisplay { hide_tooltip, hidden_components })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_bool(self.hide_tooltip);
        wire::write_list(out, &self.hidden_components, |id, o| o.put_varint(*id as i32));
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("hide_tooltip", self.hide_tooltip, false, Value::Bool)
            .opt_default("hidden_components", &self.hidden_components, &Vec::new(), |v| {
                Value::List(v.iter().map(|&id| Value::str(super::name(id))).collect())
            })
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let mut hidden_components = m.opt_or("hidden_components", Vec::new(), |l| {
            l.as_list()?
                .iter()
                .map(|t| registry::DATA_COMPONENT_TYPE.id_from_value(t).map(|i| i as super::ComponentId))
                .collect()
        })?;
        dedup_in_order(&mut hidden_components);
        Ok(TooltipDisplay { hide_tooltip: m.opt_or("hide_tooltip", false, Value::as_bool)?, hidden_components })
    }
}

/// A linked set keeps the first occurrence of each element.
fn dedup_in_order<T: PartialEq + Copy>(v: &mut Vec<T>) {
    let mut seen = Vec::with_capacity(v.len());
    v.retain(|x| {
        let fresh = !seen.contains(x);
        if fresh {
            seen.push(*x);
        }
        fresh
    });
}

/// `intangible_projectile`: `Unit` whose network form is its codec as NBT (an empty compound).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IntangibleProjectile;

impl ComponentValue for IntangibleProjectile {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_via_nbt(r)
    }
    fn write(&self, out: &mut BytesMut) {
        write_via_nbt(self, out)
    }
    fn to_value(&self) -> Value {
        Value::empty_map()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_map().map(|_| IntangibleProjectile)
    }
}

/// `dyed_color`: an RGB color (`INT` on the network).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DyedColor(pub i32);

impl ComponentValue for DyedColor {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.i32().map(DyedColor)
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_i32(self.0);
    }
    fn to_value(&self) -> Value {
        Value::Int(self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        rgb_from_value(v).map(DyedColor)
    }
}

/// `map_id` (`VAR_INT` on the network).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapId(pub i32);

impl ComponentValue for MapId {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.varint().map(MapId)
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.0);
    }
    fn to_value(&self) -> Value {
        Value::Int(self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_i32().map(MapId)
    }
}

/// `ominous_bottle_amplifier`: 0..=4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OminousBottleAmplifier(pub i32);

impl ComponentValue for OminousBottleAmplifier {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.varint().map(OminousBottleAmplifier)
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.0);
    }
    fn to_value(&self) -> Value {
        Value::Int(self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        match v.as_i32()? {
            n @ 0..=4 => Ok(OminousBottleAmplifier(n)),
            n => err(format!("ominous bottle amplifier {n} out of range [0;4]")),
        }
    }
}
