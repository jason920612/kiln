//! Types shared by several component groups.

use crate::holder::Holder;
use crate::ident::Identifier;
use crate::registry;
use crate::value::{DataResult, MapBuilder, Value};
use crate::wire::{self, WireResult};
use bytes::{BufMut, BytesMut};
use kiln_proto::Reader;

/// A sound event definition (`SoundEvent`): a sound id and an optional fixed range.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundEventDef {
    pub sound_id: Identifier,
    pub range: Option<f32>,
}

/// `Holder<SoundEvent>` (`SoundEvent.STREAM_CODEC` / `SoundEvent.CODEC`): a
/// `minecraft:sound_event` entry or an inline definition.
pub type SoundEvent = Holder<SoundEventDef>;

impl SoundEventDef {
    /// `SoundEvent.DIRECT_STREAM_CODEC`.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(SoundEventDef { sound_id: Identifier::read(r)?, range: wire::read_opt(r, |r| r.f32())? })
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.sound_id.write(out);
        wire::write_opt(out, &self.range, |v, o| o.put_f32(*v));
    }

    /// `SoundEvent.DIRECT_CODEC`.
    pub fn to_value(&self) -> Value {
        MapBuilder::new().put("sound_id", self.sound_id.to_value()).opt("range", self.range, Value::Float).build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        Ok(SoundEventDef {
            sound_id: m.req_with("sound_id", Identifier::from_value)?,
            range: m.lenient_or("range", None, |v| v.as_f32().map(Some)),
        })
    }
}

pub fn read_sound(r: &mut Reader<'_>) -> WireResult<SoundEvent> {
    Holder::read(registry::SOUND_EVENT, r, SoundEventDef::read)
}

pub fn write_sound(sound: &SoundEvent, out: &mut BytesMut) {
    sound.write(out, SoundEventDef::write);
}

pub fn sound_to_value(sound: &SoundEvent) -> Value {
    sound.to_value(registry::SOUND_EVENT, SoundEventDef::to_value)
}

pub fn sound_from_value(v: &Value) -> DataResult<SoundEvent> {
    Holder::from_value(registry::SOUND_EVENT, v, SoundEventDef::from_value)
}
