//! Food, consumables and their effects, potions, fuel and composting.

use super::ComponentValue;
use super::common::{SoundEvent, read_sound, sound_from_value, sound_to_value, write_sound};
use crate::enums::string_enum;
use crate::holder::{Holder, HolderSet};
use crate::ident::Identifier;
use crate::registry::{self, CONSUME_EFFECT_TYPE, MOB_EFFECT, POTION};
use crate::stack::ItemStackTemplate;
use crate::value::{DataResult, MapBuilder, MapView, Value, err};
use crate::wire::{self, WireResult};
use bytes::{BufMut, BytesMut};
use kiln_proto::{DecodeError, Reader, WriteExt};
use std::sync::OnceLock;

const UNBOUNDED: usize = i32::MAX as usize;

fn list_value<T>(items: &[T], f: impl Fn(&T) -> Value) -> Value {
    Value::List(items.iter().map(f).collect())
}

fn list_from<T>(v: &Value, f: impl Fn(&Value) -> DataResult<T>) -> DataResult<Vec<T>> {
    v.as_list()?.iter().map(f).collect()
}

/// `food` (`FoodProperties.DIRECT_CODEC` / `DIRECT_STREAM_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct Food {
    pub nutrition: i32,
    pub saturation: f32,
    pub can_always_eat: bool,
}

impl ComponentValue for Food {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Food { nutrition: r.varint()?, saturation: r.f32()?, can_always_eat: r.bool()? })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.nutrition);
        out.put_f32(self.saturation);
        out.put_bool(self.can_always_eat);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("nutrition", Value::Int(self.nutrition))
            .put("saturation", Value::Float(self.saturation))
            .opt_default("can_always_eat", self.can_always_eat, false, Value::Bool)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let nutrition = m.req_with("nutrition", Value::as_i32)?;
        if nutrition < 0 {
            return err(format!("nutrition {nutrition} is negative"));
        }
        Ok(Food {
            nutrition,
            saturation: m.req_with("saturation", Value::as_f32)?,
            can_always_eat: m.opt_or("can_always_eat", false, Value::as_bool)?,
        })
    }
}

string_enum! {
    /// `ItemUseAnimation`.
    pub enum ItemUseAnimation {
        None = "none", Eat = "eat", Drink = "drink", Block = "block", Bow = "bow", Trident = "trident",
        Crossbow = "crossbow", Spyglass = "spyglass", TootHorn = "toot_horn", Brush = "brush", Bundle = "bundle",
        Spear = "spear",
    }
}

/// `ConsumeEffect`: dispatched on its `minecraft:consume_effect_type`.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsumeEffect {
    ApplyEffects { effects: Vec<MobEffectInstance>, probability: f32 },
    /// `minecraft:mob_effect` entries.
    RemoveEffects(HolderSet),
    ClearAllEffects,
    TeleportRandomly { diameter: f32, directional_particles: bool },
    PlaySound(SoundEvent),
}

impl ConsumeEffect {
    fn type_name(&self) -> &'static str {
        match self {
            ConsumeEffect::ApplyEffects { .. } => "minecraft:apply_effects",
            ConsumeEffect::RemoveEffects(_) => "minecraft:remove_effects",
            ConsumeEffect::ClearAllEffects => "minecraft:clear_all_effects",
            ConsumeEffect::TeleportRandomly { .. } => "minecraft:teleport_randomly",
            ConsumeEffect::PlaySound(_) => "minecraft:play_sound",
        }
    }

    /// `ConsumeEffect.STREAM_CODEC`: the type id, then the type's own stream codec.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let ty = CONSUME_EFFECT_TYPE.read_id(r)?;
        Ok(match CONSUME_EFFECT_TYPE.name(ty).unwrap_or("") {
            "minecraft:apply_effects" => ConsumeEffect::ApplyEffects {
                effects: wire::read_list(r, UNBOUNDED, MobEffectInstance::read)?,
                probability: r.f32()?,
            },
            "minecraft:remove_effects" => ConsumeEffect::RemoveEffects(HolderSet::read(MOB_EFFECT, r)?),
            "minecraft:clear_all_effects" => ConsumeEffect::ClearAllEffects,
            "minecraft:teleport_randomly" => {
                ConsumeEffect::TeleportRandomly { diameter: r.f32()?, directional_particles: r.bool()? }
            }
            "minecraft:play_sound" => ConsumeEffect::PlaySound(read_sound(r)?),
            _ => return Err(DecodeError::Invalid("unknown consume effect type")),
        })
    }

    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(CONSUME_EFFECT_TYPE.id(self.type_name()).expect("consume effect type"));
        match self {
            ConsumeEffect::ApplyEffects { effects, probability } => {
                wire::write_list(out, effects, MobEffectInstance::write);
                out.put_f32(*probability);
            }
            ConsumeEffect::RemoveEffects(set) => set.write(out),
            ConsumeEffect::ClearAllEffects => {}
            ConsumeEffect::TeleportRandomly { diameter, directional_particles } => {
                out.put_f32(*diameter);
                out.put_bool(*directional_particles);
            }
            ConsumeEffect::PlaySound(sound) => write_sound(sound, out),
        }
    }

    /// `ConsumeEffect.CODEC`: the type's fields plus `"type"`.
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        match self {
            ConsumeEffect::ApplyEffects { effects, probability } => {
                m.put("effects", list_value(effects, MobEffectInstance::to_value))
                    .opt_default("probability", *probability, 1.0, Value::Float);
            }
            ConsumeEffect::RemoveEffects(set) => {
                m.put("effects", set.to_value(MOB_EFFECT));
            }
            ConsumeEffect::ClearAllEffects => {}
            ConsumeEffect::TeleportRandomly { diameter, directional_particles } => {
                m.opt_default("diameter", *diameter, 16.0, Value::Float)
                    .opt_default("directional_particles", *directional_particles, true, Value::Bool);
            }
            ConsumeEffect::PlaySound(sound) => {
                m.put("sound", sound_to_value(sound));
            }
        }
        m.put("type", Value::str(self.type_name())).build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let ty = m.req_with("type", |t| CONSUME_EFFECT_TYPE.id_from_value(t))?;
        Ok(match CONSUME_EFFECT_TYPE.name(ty).unwrap_or("") {
            "minecraft:apply_effects" => ConsumeEffect::ApplyEffects {
                effects: m.req_with("effects", |l| list_from(l, MobEffectInstance::from_value))?,
                probability: m.opt_or("probability", 1.0, Value::as_f32)?,
            },
            "minecraft:remove_effects" => {
                ConsumeEffect::RemoveEffects(m.req_with("effects", |s| HolderSet::from_value(MOB_EFFECT, s))?)
            }
            "minecraft:clear_all_effects" => ConsumeEffect::ClearAllEffects,
            "minecraft:teleport_randomly" => ConsumeEffect::TeleportRandomly {
                diameter: m.opt_or("diameter", 16.0, Value::as_f32)?,
                directional_particles: m.opt_or("directional_particles", true, Value::as_bool)?,
            },
            "minecraft:play_sound" => ConsumeEffect::PlaySound(m.req_with("sound", sound_from_value)?),
            other => return err(format!("unknown consume effect type {other}")),
        })
    }
}

/// `consumable`.
#[derive(Debug, Clone, PartialEq)]
pub struct Consumable {
    pub consume_seconds: f32,
    pub animation: ItemUseAnimation,
    pub sound: SoundEvent,
    pub has_consume_particles: bool,
    pub on_consume_effects: Vec<ConsumeEffect>,
}

/// `SoundEvents.GENERIC_EAT`, the default consume sound.
fn generic_eat() -> SoundEvent {
    static ID: OnceLock<i32> = OnceLock::new();
    Holder::Reference(*ID.get_or_init(|| registry::SOUND_EVENT.id("minecraft:entity.generic.eat").expect("generic eat")))
}

impl Default for Consumable {
    fn default() -> Self {
        Consumable {
            consume_seconds: 1.6,
            animation: ItemUseAnimation::Eat,
            sound: generic_eat(),
            has_consume_particles: true,
            on_consume_effects: Vec::new(),
        }
    }
}

impl ComponentValue for Consumable {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Consumable {
            consume_seconds: r.f32()?,
            animation: ItemUseAnimation::read(r)?,
            sound: read_sound(r)?,
            has_consume_particles: r.bool()?,
            on_consume_effects: wire::read_list(r, UNBOUNDED, ConsumeEffect::read)?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_f32(self.consume_seconds);
        self.animation.write(out);
        write_sound(&self.sound, out);
        out.put_bool(self.has_consume_particles);
        wire::write_list(out, &self.on_consume_effects, ConsumeEffect::write);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("consume_seconds", self.consume_seconds, 1.6, Value::Float)
            .opt_default("animation", self.animation, ItemUseAnimation::Eat, ItemUseAnimation::to_value)
            .opt_default("sound", &self.sound, &generic_eat(), sound_to_value)
            .opt_default("has_consume_particles", self.has_consume_particles, true, Value::Bool)
            .opt_default("on_consume_effects", &self.on_consume_effects, &Vec::new(), |e| {
                list_value(e, ConsumeEffect::to_value)
            })
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let consume_seconds = m.opt_or("consume_seconds", 1.6, Value::as_f32)?;
        if consume_seconds < 0.0 {
            return err(format!("consume_seconds {consume_seconds} is negative"));
        }
        Ok(Consumable {
            consume_seconds,
            animation: m.opt_or("animation", ItemUseAnimation::Eat, ItemUseAnimation::from_value)?,
            sound: m.opt_or("sound", generic_eat(), sound_from_value)?,
            has_consume_particles: m.opt_or("has_consume_particles", true, Value::as_bool)?,
            on_consume_effects: m.opt_or("on_consume_effects", Vec::new(), |l| list_from(l, ConsumeEffect::from_value))?,
        })
    }
}

/// `MobEffectInstance.Details`: everything but the effect, with an optional hidden effect
/// underneath (what remains when the outer effect runs out).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectDetails {
    pub amplifier: i32,
    pub duration: i32,
    pub ambient: bool,
    pub show_particles: bool,
    pub show_icon: bool,
    pub hidden_effect: Option<Box<EffectDetails>>,
}

impl EffectDetails {
    /// VarInt amplifier and duration, three booleans, then the optional hidden effect.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(EffectDetails {
            amplifier: r.varint()?,
            duration: r.varint()?,
            ambient: r.bool()?,
            show_particles: r.bool()?,
            show_icon: r.bool()?,
            hidden_effect: wire::read_opt(r, |r| EffectDetails::read(r).map(Box::new))?,
        })
    }

    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.amplifier);
        out.put_varint(self.duration);
        out.put_bool(self.ambient);
        out.put_bool(self.show_particles);
        out.put_bool(self.show_icon);
        wire::write_opt(out, &self.hidden_effect, |h, o| h.write(o));
    }

    /// `MAP_CODEC`: the amplifier is an unsigned byte; `show_icon` is always written.
    fn fields(&self, m: &mut MapBuilder) {
        m.opt_default("amplifier", self.amplifier, 0, |a| Value::Byte(a as u8 as i8))
            .opt_default("duration", self.duration, 0, Value::Int)
            .opt_default("ambient", self.ambient, false, Value::Bool)
            .opt_default("show_particles", self.show_particles, true, Value::Bool)
            .put("show_icon", Value::Bool(self.show_icon))
            .opt("hidden_effect", self.hidden_effect.as_deref(), EffectDetails::to_value);
    }

    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        self.fields(&mut m);
        m.build()
    }

    fn from_map(m: MapView<'_>) -> DataResult<Self> {
        let show_particles = m.opt_or("show_particles", true, Value::as_bool)?;
        Ok(EffectDetails {
            amplifier: m.opt_or("amplifier", 0, |a| Ok(a.as_i8()? as u8 as i32))?,
            duration: m.opt_or("duration", 0, Value::as_i32)?,
            ambient: m.opt_or("ambient", false, Value::as_bool)?,
            show_particles,
            show_icon: m.opt_or("show_icon", show_particles, Value::as_bool)?,
            hidden_effect: m.opt("hidden_effect", |h| EffectDetails::from_value(h).map(Box::new))?,
        })
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        EffectDetails::from_map(v.as_map()?)
    }
}

/// `MobEffectInstance`: a `minecraft:mob_effect` id and its details.
#[derive(Debug, Clone, PartialEq)]
pub struct MobEffectInstance {
    pub effect: i32,
    pub details: EffectDetails,
}

impl MobEffectInstance {
    /// `MobEffectInstance.STREAM_CODEC`.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(MobEffectInstance { effect: MOB_EFFECT.read_id(r)?, details: EffectDetails::read(r)? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.effect);
        self.details.write(out);
    }

    /// `MobEffectInstance.CODEC`: `{id, ...details}`.
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("id", MOB_EFFECT.id_to_value(self.effect));
        self.details.fields(&mut m);
        m.build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(MobEffectInstance {
            effect: m.req_with("id", |id| MOB_EFFECT.id_from_value(id))?,
            details: EffectDetails::from_map(m)?,
        })
    }
}

/// `use_remainder`: what replaces the stack once used up.
#[derive(Debug, Clone, PartialEq)]
pub struct UseRemainder(pub ItemStackTemplate);

impl ComponentValue for UseRemainder {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        ItemStackTemplate::read(r).map(UseRemainder)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out);
    }
    fn to_value(&self) -> Value {
        self.0.to_value()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        ItemStackTemplate::from_value(v).map(UseRemainder)
    }
}

/// `use_cooldown`.
#[derive(Debug, Clone, PartialEq)]
pub struct UseCooldown {
    pub seconds: f32,
    pub cooldown_group: Option<Identifier>,
}

impl ComponentValue for UseCooldown {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(UseCooldown { seconds: r.f32()?, cooldown_group: wire::read_opt(r, Identifier::read)? })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_f32(self.seconds);
        wire::write_opt(out, &self.cooldown_group, Identifier::write);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("seconds", Value::Float(self.seconds))
            .opt("cooldown_group", self.cooldown_group.as_ref(), Identifier::to_value)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let seconds = m.req_with("seconds", Value::as_f32)?;
        if seconds <= 0.0 {
            return err(format!("cooldown seconds {seconds} is not positive"));
        }
        Ok(UseCooldown { seconds, cooldown_group: m.opt("cooldown_group", Identifier::from_value)? })
    }
}

/// `death_protection`: effects applied when it saves its holder from dying.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeathProtection {
    pub death_effects: Vec<ConsumeEffect>,
}

impl ComponentValue for DeathProtection {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(DeathProtection { death_effects: wire::read_list(r, UNBOUNDED, ConsumeEffect::read)? })
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.death_effects, ConsumeEffect::write);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("death_effects", &self.death_effects, &Vec::new(), |e| list_value(e, ConsumeEffect::to_value))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(DeathProtection {
            death_effects: m.opt_or("death_effects", Vec::new(), |l| list_from(l, ConsumeEffect::from_value))?,
        })
    }
}

/// `villager_food`: what a villager's food is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VillagerFood {
    pub nutrition: i32,
}

impl ComponentValue for VillagerFood {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(VillagerFood { nutrition: r.varint()? })
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.nutrition);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new().put("nutrition", Value::Int(self.nutrition)).build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let nutrition = v.as_map()?.req_with("nutrition", Value::as_i32)?;
        if nutrition <= 0 {
            return err(format!("villager food nutrition {nutrition} is not positive"));
        }
        Ok(VillagerFood { nutrition })
    }
}

/// `potion_contents`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PotionContents {
    /// A `minecraft:potion` id.
    pub potion: Option<i32>,
    pub custom_color: Option<i32>,
    pub custom_effects: Vec<MobEffectInstance>,
    pub custom_name: Option<String>,
}

impl ComponentValue for PotionContents {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(PotionContents {
            potion: wire::read_opt(r, |r| POTION.read_id(r))?,
            custom_color: wire::read_opt(r, |r| r.i32())?,
            custom_effects: wire::read_list(r, UNBOUNDED, MobEffectInstance::read)?,
            custom_name: wire::read_opt(r, |r| wire::read_string(r, 32767))?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_opt(out, &self.potion, |id, o| o.put_varint(*id));
        wire::write_opt(out, &self.custom_color, |c, o| o.put_i32(*c));
        wire::write_list(out, &self.custom_effects, MobEffectInstance::write);
        wire::write_opt(out, &self.custom_name, |s, o| o.put_string(s));
    }
    /// `FULL_CODEC` (the bare-potion alternative is only read).
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt("potion", self.potion, |id| POTION.id_to_value(id))
            .opt("custom_color", self.custom_color, Value::Int)
            .opt_default("custom_effects", &self.custom_effects, &Vec::new(), |e| list_value(e, MobEffectInstance::to_value))
            .opt("custom_name", self.custom_name.as_ref(), |s| Value::str(s.as_str()))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        if let Value::String(_) = v {
            return Ok(PotionContents { potion: Some(POTION.id_from_value(v)?), ..Default::default() });
        }
        let m = v.as_map()?;
        Ok(PotionContents {
            potion: m.opt("potion", |p| POTION.id_from_value(p))?,
            custom_color: m.opt("custom_color", Value::as_i32)?,
            custom_effects: m.opt_or("custom_effects", Vec::new(), |l| list_from(l, MobEffectInstance::from_value))?,
            custom_name: m.opt("custom_name", |s| s.as_str().map(str::to_owned))?,
        })
    }
}

/// One `suspicious_stew_effects` entry: a `minecraft:mob_effect` id and a duration in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StewEffect {
    pub effect: i32,
    pub duration: i32,
}

/// `suspicious_stew_effects`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SuspiciousStewEffects(pub Vec<StewEffect>);

impl ComponentValue for SuspiciousStewEffects {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        wire::read_list(r, UNBOUNDED, |r| Ok(StewEffect { effect: MOB_EFFECT.read_id(r)?, duration: r.varint()? }))
            .map(SuspiciousStewEffects)
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.0, |e, o| {
            o.put_varint(e.effect);
            o.put_varint(e.duration);
        });
    }
    fn to_value(&self) -> Value {
        list_value(&self.0, |e| {
            MapBuilder::new()
                .put("id", MOB_EFFECT.id_to_value(e.effect))
                .opt_default("duration", e.duration, 160, Value::Int)
                .build()
        })
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        list_from(v, |e| {
            let m = e.as_map()?;
            Ok(StewEffect {
                effect: m.req_with("id", |id| MOB_EFFECT.id_from_value(id))?,
                duration: m.lenient_or("duration", 160, Value::as_i32),
            })
        })
        .map(SuspiciousStewEffects)
    }
}

/// `ResolvableInt`: a constant, or a `minecraft:context_int_provider` key resolved in context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvableInt {
    Constant(i32),
    Reference(Identifier),
}

impl ResolvableInt {
    /// `ByteBufCodecs.either`: `true` and an `INT`, or `false` and the key.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(if r.bool()? { ResolvableInt::Constant(r.i32()?) } else { ResolvableInt::Reference(Identifier::read(r)?) })
    }

    pub fn write(&self, out: &mut BytesMut) {
        match self {
            ResolvableInt::Constant(v) => {
                out.put_bool(true);
                out.put_i32(*v);
            }
            ResolvableInt::Reference(key) => {
                out.put_bool(false);
                key.write(out);
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            ResolvableInt::Constant(v) => Value::Int(*v),
            ResolvableInt::Reference(key) => key.to_value(),
        }
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        match v.as_i32() {
            Ok(n) => Ok(ResolvableInt::Constant(n)),
            Err(_) => Identifier::from_value(v).map(ResolvableInt::Reference),
        }
    }
}

/// `ResolvableFloat`: a constant, or a `minecraft:context_float_provider` key.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvableFloat {
    Constant(f32),
    Reference(Identifier),
}

impl ResolvableFloat {
    /// `ByteBufCodecs.either`: `true` and a `FLOAT`, or `false` and the key.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(if r.bool()? { ResolvableFloat::Constant(r.f32()?) } else { ResolvableFloat::Reference(Identifier::read(r)?) })
    }

    pub fn write(&self, out: &mut BytesMut) {
        match self {
            ResolvableFloat::Constant(v) => {
                out.put_bool(true);
                out.put_f32(*v);
            }
            ResolvableFloat::Reference(key) => {
                out.put_bool(false);
                key.write(out);
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            ResolvableFloat::Constant(v) => Value::Float(*v),
            ResolvableFloat::Reference(key) => key.to_value(),
        }
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        match v.as_f32() {
            Ok(n) => Ok(ResolvableFloat::Constant(n)),
            Err(_) => Identifier::from_value(v).map(ResolvableFloat::Reference),
        }
    }
}

/// `compostable`: composter layers added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compostable {
    pub layers: ResolvableInt,
}

impl ComponentValue for Compostable {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(Compostable { layers: ResolvableInt::read(r)? })
    }
    fn write(&self, out: &mut BytesMut) {
        self.layers.write(out);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new().put("layers", self.layers.to_value()).build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Ok(Compostable { layers: v.as_map()?.req_with("layers", ResolvableInt::from_value)? })
    }
}

/// `cooking_fuel`: furnace burn time and speed.
#[derive(Debug, Clone, PartialEq)]
pub struct CookingFuel {
    pub burn_time: ResolvableInt,
    pub speed_multiplier: ResolvableFloat,
}

impl ComponentValue for CookingFuel {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(CookingFuel { burn_time: ResolvableInt::read(r)?, speed_multiplier: ResolvableFloat::read(r)? })
    }
    fn write(&self, out: &mut BytesMut) {
        self.burn_time.write(out);
        self.speed_multiplier.write(out);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("burn_time", self.burn_time.to_value())
            .put("speed_multiplier", self.speed_multiplier.to_value())
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(CookingFuel {
            burn_time: m.req_with("burn_time", ResolvableInt::from_value)?,
            speed_multiplier: m.req_with("speed_multiplier", ResolvableFloat::from_value)?,
        })
    }
}

/// `brewing_fuel`: brewing stand uses and speed.
#[derive(Debug, Clone, PartialEq)]
pub struct BrewingFuel {
    pub uses: ResolvableInt,
    pub speed_multiplier: ResolvableFloat,
}

impl ComponentValue for BrewingFuel {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(BrewingFuel { uses: ResolvableInt::read(r)?, speed_multiplier: ResolvableFloat::read(r)? })
    }
    fn write(&self, out: &mut BytesMut) {
        self.uses.write(out);
        self.speed_multiplier.write(out);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("uses", self.uses.to_value())
            .put("speed_multiplier", self.speed_multiplier.to_value())
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(BrewingFuel {
            uses: m.req_with("uses", ResolvableInt::from_value)?,
            speed_multiplier: m.req_with("speed_multiplier", ResolvableFloat::from_value)?,
        })
    }
}
