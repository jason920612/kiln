//! Trims, banners, fireworks, profiles, instruments, music discs, maps, sounds and other registry references.

use super::variant::registry_ref;
use super::{ComponentValue, DyeColor, SoundEvent, read_sound, read_via_nbt, sound_from_value, sound_to_value, write_sound, write_via_nbt};
use crate::enums::string_enum;
use crate::holder::{Holder, HolderSet};
use crate::ident::Identifier;
use crate::registry;
use crate::text::Text;
use crate::value::{DataError, DataResult, MapBuilder, Value, err};
use crate::wire::{self, WireResult};
use bytes::{BufMut, BytesMut};
use kiln_proto::{Reader, WriteExt};
use uuid::Uuid;

registry_ref! {
    /// `block_transformer`: a `minecraft:block_transformer` entry.
    BlockTransformerRef => registry::BLOCK_TRANSFORMER;
    /// `provides_pottery_pattern`: a `minecraft:decorated_pot_pattern` entry.
    PotteryPattern => registry::DECORATED_POT_PATTERN;
}

/// `break_sound`: a sound event (`SoundEvent.STREAM_CODEC` / `CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundEventRef(pub SoundEvent);

impl ComponentValue for SoundEventRef {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_sound(r).map(SoundEventRef)
    }
    fn write(&self, out: &mut BytesMut) {
        write_sound(&self.0, out);
    }
    fn to_value(&self) -> Value {
        sound_to_value(&self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        sound_from_value(v).map(SoundEventRef)
    }
}

/// `mob_visibility`: how visible the wearer is to the given entity types (0..=10).
#[derive(Debug, Clone, PartialEq)]
pub struct MobVisibility {
    pub targeting_entity_types: HolderSet,
    pub visibility: f32,
}

impl ComponentValue for MobVisibility {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(MobVisibility { targeting_entity_types: HolderSet::read(registry::ENTITY_TYPE, r)?, visibility: r.f32()? })
    }
    fn write(&self, out: &mut BytesMut) {
        self.targeting_entity_types.write(out);
        out.put_f32(self.visibility);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("targeting_entity_types", self.targeting_entity_types.to_value(registry::ENTITY_TYPE))
            .put("visibility", Value::Float(self.visibility))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(MobVisibility {
            targeting_entity_types: m.req_with("targeting_entity_types", |v| HolderSet::from_value(registry::ENTITY_TYPE, v))?,
            visibility: m.req_with("visibility", |v| float_in(v, 0.0, 10.0))?,
        })
    }
}

fn float_in(v: &Value, min: f32, max: f32) -> DataResult<f32> {
    let f = v.as_f32()?;
    if (min..=max).contains(&f) { Ok(f) } else { err(format!("{f} out of range [{min};{max}]")) }
}

fn positive_float(v: &Value) -> DataResult<f32> {
    let f = v.as_f32()?;
    if f > 0.0 { Ok(f) } else { err(format!("{f} is not positive")) }
}

fn non_negative_float(v: &Value) -> DataResult<f32> {
    let f = v.as_f32()?;
    if f >= 0.0 { Ok(f) } else { err(format!("{f} is negative")) }
}

// ---- maps -----------------------------------------------------------------------------------

/// `map_decorations`: named markers (persistent only, so NBT on the network).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapDecorations(pub Vec<(String, MapDecoration)>);

/// `MapDecorations.Entry`.
#[derive(Debug, Clone, PartialEq)]
pub struct MapDecoration {
    /// A `minecraft:map_decoration_type` id.
    pub kind: i32,
    pub x: f64,
    pub z: f64,
    pub rotation: f32,
}

impl ComponentValue for MapDecorations {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_via_nbt(r)
    }
    fn write(&self, out: &mut BytesMut) {
        write_via_nbt(self, out)
    }
    fn to_value(&self) -> Value {
        Value::Map(
            self.0
                .iter()
                .map(|(k, d)| {
                    let entry = MapBuilder::new()
                        .put("type", registry::MAP_DECORATION_TYPE.id_to_value(d.kind))
                        .put("x", Value::Double(d.x))
                        .put("z", Value::Double(d.z))
                        .put("rotation", Value::Float(d.rotation))
                        .build();
                    (Value::str(k.as_str()), entry)
                })
                .collect(),
        )
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let mut out: Vec<(String, MapDecoration)> = Vec::new();
        for (k, e) in v.as_map()?.entries() {
            let m = e.as_map()?;
            let d = MapDecoration {
                kind: m.req_with("type", |v| registry::MAP_DECORATION_TYPE.id_from_value(v))?,
                x: m.req_with("x", Value::as_f64)?,
                z: m.req_with("z", Value::as_f64)?,
                rotation: m.req_with("rotation", Value::as_f32)?,
            };
            let key = k.as_str()?.to_owned();
            match out.iter_mut().find(|(n, _)| *n == key) {
                Some(slot) => slot.1 = d,
                None => out.push((key, d)),
            }
        }
        Ok(MapDecorations(out))
    }
}

/// `lodestone_tracker`.
#[derive(Debug, Clone, PartialEq)]
pub struct LodestoneTracker {
    pub target: Option<GlobalPos>,
    /// Whether the compass forgets the target once the lodestone is gone (default true).
    pub tracked: bool,
}

/// `GlobalPos`: a dimension and a block position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalPos {
    pub dimension: Identifier,
    pub pos: [i32; 3],
}

impl GlobalPos {
    /// `GlobalPos.STREAM_CODEC`: the dimension id, then the packed position.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(GlobalPos { dimension: Identifier::read(r)?, pos: kiln_proto::packets::read_position(r)? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.dimension.write(out);
        out.put_position(self.pos[0], self.pos[1], self.pos[2]);
    }

    /// `{dimension, pos: [I; x, y, z]}`.
    pub fn to_value(&self) -> Value {
        MapBuilder::new().put("dimension", self.dimension.to_value()).put("pos", Value::IntList(self.pos.to_vec())).build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let pos = m.req_with("pos", Value::as_int_stream)?;
        let pos: [i32; 3] = pos.try_into().map_err(|p: Vec<i32>| DataError(format!("block position needs 3 ints, got {}", p.len())))?;
        Ok(GlobalPos { dimension: m.req_with("dimension", Identifier::from_value)?, pos })
    }
}

impl ComponentValue for LodestoneTracker {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(LodestoneTracker { target: wire::read_opt(r, GlobalPos::read)?, tracked: r.bool()? })
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_opt(out, &self.target, GlobalPos::write);
        out.put_bool(self.tracked);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("target", self.target.as_ref(), GlobalPos::to_value)
            .opt_default("tracked", self.tracked, true, Value::Bool)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(LodestoneTracker { target: m.opt("target", GlobalPos::from_value)?, tracked: m.opt_or("tracked", true, Value::as_bool)? })
    }
}

/// `recipes` (knowledge books): recipe ids (persistent only, so NBT on the network).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recipes(pub Vec<Identifier>);

impl ComponentValue for Recipes {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_via_nbt(r)
    }
    fn write(&self, out: &mut BytesMut) {
        write_via_nbt(self, out)
    }
    fn to_value(&self) -> Value {
        Value::List(self.0.iter().map(Identifier::to_value).collect())
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_list()?.iter().map(Identifier::from_value).collect::<DataResult<_>>().map(Recipes)
    }
}

// ---- trims ----------------------------------------------------------------------------------

/// An inline trim material (`TrimMaterial.DIRECT_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct TrimMaterialDef {
    pub palette_id: Identifier,
    pub description: Text,
}

impl TrimMaterialDef {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(TrimMaterialDef { palette_id: Identifier::read(r)?, description: Text::read(r)? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.palette_id.write(out);
        self.description.write(out);
    }

    pub fn to_value(&self) -> Value {
        MapBuilder::new().put("palette_id", self.palette_id.to_value()).put("description", self.description.to_value()).build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(TrimMaterialDef {
            palette_id: m.req_with("palette_id", Identifier::from_value)?,
            description: m.req_with("description", Text::from_value)?,
        })
    }
}

/// An inline trim pattern (`TrimPattern.DIRECT_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct TrimPatternDef {
    pub asset_id: Identifier,
    pub description: Text,
    pub decal: bool,
}

impl TrimPatternDef {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(TrimPatternDef { asset_id: Identifier::read(r)?, description: Text::read(r)?, decal: r.bool()? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.asset_id.write(out);
        self.description.write(out);
        out.put_bool(self.decal);
    }

    /// `decal` is always written (`optionalAlwaysPresentFieldOf`).
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("asset_id", self.asset_id.to_value())
            .put("description", self.description.to_value())
            .put("decal", Value::Bool(self.decal))
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(TrimPatternDef {
            asset_id: m.req_with("asset_id", Identifier::from_value)?,
            description: m.req_with("description", Text::from_value)?,
            decal: m.opt_or("decal", false, Value::as_bool)?,
        })
    }
}

pub type TrimMaterial = Holder<TrimMaterialDef>;
pub type TrimPattern = Holder<TrimPatternDef>;

fn read_material(r: &mut Reader<'_>) -> WireResult<TrimMaterial> {
    Holder::read(registry::TRIM_MATERIAL, r, TrimMaterialDef::read)
}

fn material_to_value(m: &TrimMaterial) -> Value {
    m.to_value(registry::TRIM_MATERIAL, TrimMaterialDef::to_value)
}

fn material_from_value(v: &Value) -> DataResult<TrimMaterial> {
    Holder::from_value(registry::TRIM_MATERIAL, v, TrimMaterialDef::from_value)
}

/// `trim`: an armor trim.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorTrim {
    pub material: TrimMaterial,
    pub pattern: TrimPattern,
}

impl ComponentValue for ArmorTrim {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(ArmorTrim { material: read_material(r)?, pattern: Holder::read(registry::TRIM_PATTERN, r, TrimPatternDef::read)? })
    }
    fn write(&self, out: &mut BytesMut) {
        self.material.write(out, TrimMaterialDef::write);
        self.pattern.write(out, TrimPatternDef::write);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("material", material_to_value(&self.material))
            .put("pattern", self.pattern.to_value(registry::TRIM_PATTERN, TrimPatternDef::to_value))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(ArmorTrim {
            material: m.req_with("material", material_from_value)?,
            pattern: m.req_with("pattern", |v| Holder::from_value(registry::TRIM_PATTERN, v, TrimPatternDef::from_value))?,
        })
    }
}

/// `provides_trim_material`: the trim material a smithing ingredient applies.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvidesTrimMaterial(pub TrimMaterial);

impl ComponentValue for ProvidesTrimMaterial {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        read_material(r).map(ProvidesTrimMaterial)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out, TrimMaterialDef::write);
    }
    fn to_value(&self) -> Value {
        material_to_value(&self.0)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        material_from_value(v).map(ProvidesTrimMaterial)
    }
}

// ---- instruments and music discs ------------------------------------------------------------

/// An inline instrument (`Instrument.DIRECT_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentDef {
    pub sound_event: SoundEvent,
    pub use_duration: f32,
    pub range: f32,
    pub durability_damage: i32,
    pub description: Text,
}

impl InstrumentDef {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(InstrumentDef {
            sound_event: read_sound(r)?,
            use_duration: r.f32()?,
            range: r.f32()?,
            durability_damage: r.varint()?,
            description: Text::read(r)?,
        })
    }

    pub fn write(&self, out: &mut BytesMut) {
        write_sound(&self.sound_event, out);
        out.put_f32(self.use_duration);
        out.put_f32(self.range);
        out.put_varint(self.durability_damage);
        self.description.write(out);
    }

    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("sound_event", sound_to_value(&self.sound_event))
            .put("use_duration", Value::Float(self.use_duration))
            .put("range", Value::Float(self.range))
            .opt_default("durability_damage", self.durability_damage, 0, Value::Int)
            .put("description", self.description.to_value())
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(InstrumentDef {
            sound_event: m.req_with("sound_event", sound_from_value)?,
            use_duration: m.req_with("use_duration", non_negative_float)?,
            range: m.req_with("range", positive_float)?,
            durability_damage: m.opt_or("durability_damage", 0, |v| match v.as_i32()? {
                n if n >= 0 => Ok(n),
                n => err(format!("{n} is negative")),
            })?,
            description: m.req_with("description", Text::from_value)?,
        })
    }
}

/// `instrument` (goat horns): a `minecraft:instrument` entry or an inline definition.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentComponent(pub Holder<InstrumentDef>);

impl ComponentValue for InstrumentComponent {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Holder::read(registry::INSTRUMENT, r, InstrumentDef::read).map(InstrumentComponent)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out, InstrumentDef::write);
    }
    fn to_value(&self) -> Value {
        self.0.to_value(registry::INSTRUMENT, InstrumentDef::to_value)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Holder::from_value(registry::INSTRUMENT, v, InstrumentDef::from_value).map(InstrumentComponent)
    }
}

/// An inline jukebox song (`JukeboxSong.DIRECT_STREAM_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct JukeboxSongDef {
    pub sound_event: SoundEvent,
    pub description: Text,
    pub length_in_seconds: f32,
    /// 0..=15.
    pub comparator_output: i32,
}

impl JukeboxSongDef {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(JukeboxSongDef {
            sound_event: read_sound(r)?,
            description: Text::read(r)?,
            length_in_seconds: r.f32()?,
            comparator_output: r.varint()?,
        })
    }

    pub fn write(&self, out: &mut BytesMut) {
        write_sound(&self.sound_event, out);
        self.description.write(out);
        out.put_f32(self.length_in_seconds);
        out.put_varint(self.comparator_output);
    }

    /// `JukeboxSong.DIRECT_CODEC` (vanilla's component codec cannot save inline songs; they
    /// are written in this form here).
    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("sound_event", sound_to_value(&self.sound_event))
            .put("description", self.description.to_value())
            .put("length_in_seconds", Value::Float(self.length_in_seconds))
            .put("comparator_output", Value::Int(self.comparator_output))
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(JukeboxSongDef {
            sound_event: m.req_with("sound_event", sound_from_value)?,
            description: m.req_with("description", Text::from_value)?,
            length_in_seconds: m.req_with("length_in_seconds", positive_float)?,
            comparator_output: m.req_with("comparator_output", |v| match v.as_i32()? {
                n @ 0..=15 => Ok(n),
                n => err(format!("{n} out of range [0;15]")),
            })?,
        })
    }
}

/// `jukebox_playable`: a `minecraft:jukebox_song` entry, or (on the network) an inline song.
#[derive(Debug, Clone, PartialEq)]
pub struct JukeboxPlayable(pub Holder<JukeboxSongDef>);

impl ComponentValue for JukeboxPlayable {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Holder::read(registry::JUKEBOX_SONG, r, JukeboxSongDef::read).map(JukeboxPlayable)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out, JukeboxSongDef::write);
    }
    fn to_value(&self) -> Value {
        self.0.to_value(registry::JUKEBOX_SONG, JukeboxSongDef::to_value)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Holder::from_value(registry::JUKEBOX_SONG, v, JukeboxSongDef::from_value).map(JukeboxPlayable)
    }
}

// ---- banners --------------------------------------------------------------------------------

/// An inline banner pattern (`BannerPattern.DIRECT_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct BannerPatternDef {
    pub asset_id: Identifier,
    pub translation_key: String,
}

impl BannerPatternDef {
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(BannerPatternDef { asset_id: Identifier::read(r)?, translation_key: wire::read_string(r, 32767)? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.asset_id.write(out);
        out.put_string(&self.translation_key);
    }

    pub fn to_value(&self) -> Value {
        MapBuilder::new().put("asset_id", self.asset_id.to_value()).put("translation_key", Value::str(self.translation_key.as_str())).build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(BannerPatternDef {
            asset_id: m.req_with("asset_id", Identifier::from_value)?,
            translation_key: m.req_with("translation_key", |v| v.as_str().map(str::to_owned))?,
        })
    }
}

/// One banner layer: a pattern and its color.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerLayer {
    pub pattern: Holder<BannerPatternDef>,
    pub color: DyeColor,
}

/// `banner_patterns`: layers from bottom to top.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BannerPatternLayers(pub Vec<BannerLayer>);

impl ComponentValue for BannerPatternLayers {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, i32::MAX as usize, |r| {
            Ok(BannerLayer { pattern: Holder::read(registry::BANNER_PATTERN, r, BannerPatternDef::read)?, color: DyeColor::read(r)? })
        })
        .map(BannerPatternLayers)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, |l, o| {
            l.pattern.write(o, BannerPatternDef::write);
            l.color.write(o);
        });
    }
    fn to_value(&self) -> Value {
        Value::List(
            self.0
                .iter()
                .map(|l| {
                    MapBuilder::new()
                        .put("pattern", l.pattern.to_value(registry::BANNER_PATTERN, BannerPatternDef::to_value))
                        .put("color", l.color.to_value())
                        .build()
                })
                .collect(),
        )
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_list()?
            .iter()
            .map(|l| {
                let m = l.as_map()?;
                Ok(BannerLayer {
                    pattern: m.req_with("pattern", |v| Holder::from_value(registry::BANNER_PATTERN, v, BannerPatternDef::from_value))?,
                    color: m.req_with("color", DyeColor::from_value)?,
                })
            })
            .collect::<DataResult<_>>()
            .map(BannerPatternLayers)
    }
}

/// `provides_banner_patterns`: the `minecraft:banner_pattern` entries a banner pattern item unlocks.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvidesBannerPatterns(pub HolderSet);

impl ComponentValue for ProvidesBannerPatterns {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        HolderSet::read(registry::BANNER_PATTERN, r).map(ProvidesBannerPatterns)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out);
    }
    fn to_value(&self) -> Value {
        self.0.to_value(registry::BANNER_PATTERN)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        HolderSet::from_value(registry::BANNER_PATTERN, v).map(ProvidesBannerPatterns)
    }
}

// ---- fireworks ------------------------------------------------------------------------------

string_enum! {
    /// `FireworkExplosion.Shape`.
    pub enum FireworkShape {
        SmallBall = "small_ball", LargeBall = "large_ball", Star = "star", Creeper = "creeper", Burst = "burst",
    }
}

/// `firework_explosion` (firework stars), also an element of `fireworks`.
#[derive(Debug, Clone, PartialEq)]
pub struct FireworkExplosion {
    pub shape: FireworkShape,
    /// RGB colors.
    pub colors: Vec<i32>,
    pub fade_colors: Vec<i32>,
    pub has_trail: bool,
    pub has_twinkle: bool,
}

fn colors_value(c: &[i32]) -> Value {
    Value::List(c.iter().map(|&x| Value::Int(x)).collect())
}

fn colors_from(v: &Value) -> DataResult<Vec<i32>> {
    v.as_list()?.iter().map(Value::as_i32).collect()
}

impl ComponentValue for FireworkExplosion {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(FireworkExplosion {
            shape: FireworkShape::read(r)?,
            colors: wire::read_list(r, i32::MAX as usize, |r| r.i32())?,
            fade_colors: wire::read_list(r, i32::MAX as usize, |r| r.i32())?,
            has_trail: r.bool()?,
            has_twinkle: r.bool()?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        self.shape.write(out);
        wire::write_list(out, &self.colors, |c, o| o.put_i32(*c));
        wire::write_list(out, &self.fade_colors, |c, o| o.put_i32(*c));
        out.put_bool(self.has_trail);
        out.put_bool(self.has_twinkle);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("shape", self.shape.to_value())
            .opt_default("colors", &self.colors[..], &[][..], colors_value)
            .opt_default("fade_colors", &self.fade_colors[..], &[][..], colors_value)
            .opt_default("has_trail", self.has_trail, false, Value::Bool)
            .opt_default("has_twinkle", self.has_twinkle, false, Value::Bool)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(FireworkExplosion {
            shape: m.req_with("shape", FireworkShape::from_value)?,
            colors: m.opt_or("colors", Vec::new(), colors_from)?,
            fade_colors: m.opt_or("fade_colors", Vec::new(), colors_from)?,
            has_trail: m.opt_or("has_trail", false, Value::as_bool)?,
            has_twinkle: m.opt_or("has_twinkle", false, Value::as_bool)?,
        })
    }
}

/// `fireworks` (rockets).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fireworks {
    /// 0..=255; persisted as an unsigned byte.
    pub flight_duration: i32,
    /// At most 256.
    pub explosions: Vec<FireworkExplosion>,
}

impl ComponentValue for Fireworks {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Fireworks { flight_duration: r.varint()?, explosions: wire::read_list(r, 256, FireworkExplosion::read)? })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.flight_duration);
        wire::write_list(out, &self.explosions, FireworkExplosion::write);
    }
    /// `ExtraCodecs.UNSIGNED_BYTE` writes the duration as a (signed) byte.
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("flight_duration", self.flight_duration, 0, |d| Value::Byte(d as u8 as i8))
            .opt_default("explosions", &self.explosions[..], &[][..], |e| Value::List(e.iter().map(FireworkExplosion::to_value).collect()))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let explosions: Vec<FireworkExplosion> =
            m.opt_or("explosions", Vec::new(), |l| l.as_list()?.iter().map(FireworkExplosion::from_value).collect())?;
        if explosions.len() > 256 {
            return err("more than 256 firework explosions");
        }
        Ok(Fireworks { flight_duration: m.opt_or("flight_duration", 0, |v| Ok(v.as_i8()? as u8 as i32))?, explosions })
    }
}

// ---- profiles -------------------------------------------------------------------------------

/// A signed game profile property (e.g. `textures`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

/// `profile` (player heads): a complete game profile, or a partial one that is resolved later,
/// plus skin overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvableProfile {
    pub profile: ProfileData,
    pub skin: SkinPatch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileData {
    /// A resolved profile (`GameProfile`).
    Full { id: Uuid, name: String, properties: Vec<ProfileProperty> },
    /// `ResolvableProfile.Partial`: a name and/or id to resolve.
    Partial { name: Option<String>, id: Option<Uuid>, properties: Vec<ProfileProperty> },
}

string_enum! {
    /// `PlayerModelType` (a boolean on the network: slim).
    pub enum PlayerModel { Slim = "slim", Wide = "wide" }
}

/// `PlayerSkin.Patch`: texture asset overrides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkinPatch {
    pub texture: Option<Identifier>,
    pub cape: Option<Identifier>,
    pub elytra: Option<Identifier>,
    pub model: Option<PlayerModel>,
}

/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`: at most 16. Vanilla stores them in a multimap,
/// which groups values by name (in first-seen order).
fn read_properties(r: &mut Reader<'_>) -> WireResult<Vec<ProfileProperty>> {
    let props = wire::read_list(r, 16, |r| {
        Ok(ProfileProperty {
            name: wire::read_string(r, 64)?,
            value: wire::read_string(r, 32767)?,
            signature: wire::read_opt(r, |r| wire::read_string(r, 1024))?,
        })
    })?;
    Ok(group_by_name(props))
}

fn group_by_name(props: Vec<ProfileProperty>) -> Vec<ProfileProperty> {
    let mut names: Vec<&str> = Vec::new();
    for p in &props {
        if !names.contains(&p.name.as_str()) {
            names.push(&p.name);
        }
    }
    names.iter().flat_map(|n| props.iter().filter(move |p| p.name == *n)).cloned().collect()
}

fn write_properties(out: &mut BytesMut, props: &[ProfileProperty]) {
    wire::write_list(out, props, |p, o| {
        o.put_string(&p.name);
        o.put_string(&p.value);
        wire::write_opt(o, &p.signature, |s, o| o.put_string(s));
    });
}

/// `ExtraCodecs.PROPERTY_MAP`: a list of `{name, value, signature}`.
fn properties_value(props: &[ProfileProperty]) -> Value {
    Value::List(
        props
            .iter()
            .map(|p| {
                MapBuilder::new()
                    .put("name", Value::str(p.name.as_str()))
                    .put("value", Value::str(p.value.as_str()))
                    .opt("signature", p.signature.as_deref(), Value::str)
                    .build()
            })
            .collect(),
    )
}

/// Also accepts the legacy form `{name: [values]}`.
fn properties_from(v: &Value) -> DataResult<Vec<ProfileProperty>> {
    let props = match v {
        Value::Map(entries) => {
            let mut out = Vec::new();
            for (k, values) in entries {
                for value in values.as_list()?.iter() {
                    out.push(ProfileProperty { name: k.as_str()?.to_owned(), value: value.as_str()?.to_owned(), signature: None });
                }
            }
            out
        }
        _ => {
            let list = v.as_list()?;
            if list.len() > 16 {
                return err("more than 16 profile properties");
            }
            list.iter()
                .map(|p| {
                    let m = p.as_map()?;
                    Ok(ProfileProperty {
                        name: m.req_with("name", |v| v.as_str().map(str::to_owned))?,
                        value: m.req_with("value", |v| v.as_str().map(str::to_owned))?,
                        signature: m.opt("signature", |v| v.as_str().map(str::to_owned))?,
                    })
                })
                .collect::<DataResult<_>>()?
        }
    };
    Ok(group_by_name(props))
}

/// `ExtraCodecs.PLAYER_NAME`: up to 16 printable ASCII characters.
fn player_name(v: &Value) -> DataResult<String> {
    let s = v.as_str()?;
    if s.chars().count() > 16 || !s.chars().all(|c| c > ' ' && c < '\u{7f}') {
        return err(format!("invalid player name {s:?}"));
    }
    Ok(s.to_owned())
}

impl ComponentValue for ResolvableProfile {
    /// `ByteBufCodecs.either(GAME_PROFILE, Partial.STREAM_CODEC)` (true for a full profile),
    /// then the skin patch.
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let profile = if r.bool()? {
            ProfileData::Full { id: r.uuid()?, name: wire::read_string(r, 16)?, properties: read_properties(r)? }
        } else {
            ProfileData::Partial {
                name: wire::read_opt(r, |r| wire::read_string(r, 16))?,
                id: wire::read_opt(r, |r| r.uuid())?,
                properties: read_properties(r)?,
            }
        };
        let skin = SkinPatch {
            texture: wire::read_opt(r, Identifier::read)?,
            cape: wire::read_opt(r, Identifier::read)?,
            elytra: wire::read_opt(r, Identifier::read)?,
            model: wire::read_opt(r, |r| Ok(if r.bool()? { PlayerModel::Slim } else { PlayerModel::Wide }))?,
        };
        Ok(ResolvableProfile { profile, skin })
    }
    fn write(&self, out: &mut BytesMut) {
        match &self.profile {
            ProfileData::Full { id, name, properties } => {
                out.put_bool(true);
                out.put_uuid(*id);
                out.put_string(name);
                write_properties(out, properties);
            }
            ProfileData::Partial { name, id, properties } => {
                out.put_bool(false);
                wire::write_opt(out, name, |n, o| o.put_string(n));
                wire::write_opt(out, id, |u, o| o.put_uuid(*u));
                write_properties(out, properties);
            }
        }
        let s = &self.skin;
        wire::write_opt(out, &s.texture, Identifier::write);
        wire::write_opt(out, &s.cape, Identifier::write);
        wire::write_opt(out, &s.elytra, Identifier::write);
        wire::write_opt(out, &s.model, |m, o| o.put_bool(*m == PlayerModel::Slim));
    }
    fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        match &self.profile {
            ProfileData::Full { id, name, properties } => {
                m.put("id", wire::uuid_to_value(*id)).put("name", Value::str(name.as_str()));
                m.opt_default("properties", &properties[..], &[][..], properties_value);
            }
            ProfileData::Partial { name, id, properties } => {
                m.opt("name", name.as_deref(), Value::str).opt("id", *id, wire::uuid_to_value);
                m.opt_default("properties", &properties[..], &[][..], properties_value);
            }
        }
        let s = &self.skin;
        m.opt("texture", s.texture.as_ref(), Identifier::to_value)
            .opt("cape", s.cape.as_ref(), Identifier::to_value)
            .opt("elytra", s.elytra.as_ref(), Identifier::to_value)
            .opt("model", s.model, PlayerModel::to_value)
            .build()
    }
    /// Also accepts a bare player name.
    fn from_value(v: &Value) -> DataResult<Self> {
        if let Value::String(_) = v {
            let profile = ProfileData::Partial { name: Some(player_name(v)?), id: None, properties: Vec::new() };
            return Ok(ResolvableProfile { profile, skin: SkinPatch::default() });
        }
        let m = v.as_map()?;
        let properties = m.opt_or("properties", Vec::new(), properties_from)?;
        let full = match (m.get("id").map(wire::uuid_from_value), m.get("name").map(player_name)) {
            (Some(Ok(id)), Some(Ok(name))) => Some(ProfileData::Full { id, name, properties: properties.clone() }),
            _ => None,
        };
        let profile = match full {
            Some(p) => p,
            None => ProfileData::Partial { name: m.opt("name", player_name)?, id: m.opt("id", wire::uuid_from_value)?, properties },
        };
        let skin = SkinPatch {
            texture: m.opt("texture", Identifier::from_value)?,
            cape: m.opt("cape", Identifier::from_value)?,
            elytra: m.opt("elytra", Identifier::from_value)?,
            model: m.opt("model", PlayerModel::from_value)?,
        };
        Ok(ResolvableProfile { profile, skin })
    }
}
