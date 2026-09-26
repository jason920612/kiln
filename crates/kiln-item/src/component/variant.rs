//! Entity variants carried by spawn eggs, buckets and paintings.

use super::ComponentValue;
use crate::holder::Holder;
use crate::ident::Identifier;
use crate::registry::{self, Registry};
use crate::text::Text;
use crate::value::{DataError, DataResult, MapBuilder, Value, err};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::{Reader, WriteExt};

/// Declares newtypes over a registry entry referenced by network id: `holderRegistry` on the
/// network, the entry name in NBT (`RegistryFixedCodec`, no inline definitions).
macro_rules! registry_ref {
    ($($(#[$m:meta])* $name:ident => $reg:expr;)*) => {$(
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub i32);

        impl $crate::component::ComponentValue for $name {
            fn read(r: &mut kiln_proto::Reader<'_>) -> $crate::wire::WireResult<Self> {
                $reg.read_id(r).map($name)
            }
            fn write(&self, out: &mut bytes::BytesMut) {
                $reg.write_id(self.0, out)
            }
            fn to_value(&self) -> $crate::value::Value {
                $reg.id_to_value(self.0)
            }
            fn from_value(v: &$crate::value::Value) -> $crate::value::DataResult<Self> {
                $reg.id_from_value(v).map($name)
            }
        }
    )*};
}

pub(crate) use registry_ref;

registry_ref! {
    /// `villager/variant`: a `minecraft:villager_type`.
    VillagerVariant => registry::VILLAGER_TYPE;
    /// `wolf/variant`.
    WolfVariant => Registry("minecraft:wolf_variant");
    /// `wolf/sound_variant`.
    WolfSoundVariant => Registry("minecraft:wolf_sound_variant");
    /// `pig/variant`.
    PigVariant => Registry("minecraft:pig_variant");
    /// `pig/sound_variant`.
    PigSoundVariant => Registry("minecraft:pig_sound_variant");
    /// `cow/variant`.
    CowVariant => Registry("minecraft:cow_variant");
    /// `cow/sound_variant`.
    CowSoundVariant => Registry("minecraft:cow_sound_variant");
    /// `chicken/variant`.
    ChickenVariant => Registry("minecraft:chicken_variant");
    /// `chicken/sound_variant`.
    ChickenSoundVariant => Registry("minecraft:chicken_sound_variant");
    /// `zombie_nautilus/variant`.
    ZombieNautilusVariant => Registry("minecraft:zombie_nautilus_variant");
    /// `frog/variant`.
    FrogVariant => Registry("minecraft:frog_variant");
    /// `cat/variant`.
    CatVariant => Registry("minecraft:cat_variant");
    /// `cat/sound_variant`.
    CatSoundVariant => Registry("minecraft:cat_sound_variant");
}

/// What `ByIdMap` does with a network id no variant has.
#[derive(Clone, Copy)]
enum OutOfBounds {
    /// The first variant (`ZERO`, and `ByIdMap.sparse`'s default).
    First,
    Clamp,
    Wrap,
}

/// Index of the variant with network id `id` among `ids` (declaration order).
fn resolve(id: i32, ids: &[i32], oob: OutOfBounds) -> usize {
    if let Some(i) = ids.iter().position(|&x| x == id) {
        return i;
    }
    match oob {
        OutOfBounds::First => 0,
        OutOfBounds::Clamp => {
            if id < 0 {
                0
            } else {
                ids.len() - 1
            }
        }
        OutOfBounds::Wrap => id.rem_euclid(ids.len() as i32) as usize,
    }
}

/// A `StringRepresentable` enum sent as `ByteBufCodecs.idMapper` over explicit ids.
macro_rules! id_enum {
    ($(#[$m:meta])* pub enum $name:ident ($oob:ident) { $($variant:ident = $id:expr, $str:literal;)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];
            const IDS: &'static [i32] = &[$($id),*];

            /// Network id.
            pub fn id(self) -> i32 {
                Self::IDS[self as usize]
            }

            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $str,)*
                }
            }

            pub fn from_name(s: &str) -> Option<Self> {
                match s {
                    $($str => Some($name::$variant),)*
                    _ => None,
                }
            }
        }

        impl ComponentValue for $name {
            fn read(r: &mut Reader<'_>) -> WireResult<Self> {
                Ok(Self::ALL[resolve(r.varint()?, Self::IDS, OutOfBounds::$oob)])
            }
            fn write(&self, out: &mut BytesMut) {
                out.put_varint(self.id());
            }
            fn to_value(&self) -> Value {
                Value::str(self.name())
            }
            fn from_value(v: &Value) -> DataResult<Self> {
                let s = v.as_str()?;
                Self::from_name(s).ok_or_else(|| DataError(format!("unknown {} {s:?}", stringify!($name))))
            }
        }
    };
}

id_enum! {
    /// `fox/variant`.
    pub enum FoxVariant (First) { Red = 0, "red"; Snow = 1, "snow"; }
}

id_enum! {
    /// `salmon/size`.
    pub enum SalmonSize (Clamp) { Small = 0, "small"; Medium = 1, "medium"; Large = 2, "large"; }
}

id_enum! {
    /// `parrot/variant`.
    pub enum ParrotVariant (Clamp) {
        RedBlue = 0, "red_blue"; Blue = 1, "blue"; Green = 2, "green"; YellowBlue = 3, "yellow_blue"; Gray = 4, "gray";
    }
}

id_enum! {
    /// `tropical_fish/pattern`: the id packs the body size (bit 0) and the pattern index (bits 8..).
    pub enum TropicalFishPattern (First) {
        Kob = 0, "kob"; Sunstreak = 1 << 8, "sunstreak"; Snooper = 2 << 8, "snooper"; Dasher = 3 << 8, "dasher";
        Brinely = 4 << 8, "brinely"; Spotty = 5 << 8, "spotty"; Flopper = 1, "flopper"; Stripey = 1 | 1 << 8, "stripey";
        Glitter = 1 | 2 << 8, "glitter"; Blockfish = 1 | 3 << 8, "blockfish"; Betty = 1 | 4 << 8, "betty";
        Clayfish = 1 | 5 << 8, "clayfish";
    }
}

id_enum! {
    /// `mooshroom/variant`.
    pub enum MooshroomVariant (Clamp) { Red = 0, "red"; Brown = 1, "brown"; }
}

id_enum! {
    /// `rabbit/variant` (the killer bunny is id 99).
    pub enum RabbitVariant (First) {
        Brown = 0, "brown"; White = 1, "white"; Black = 2, "black"; WhiteSplotched = 3, "white_splotched";
        Gold = 4, "gold"; Salt = 5, "salt"; Evil = 99, "evil";
    }
}

id_enum! {
    /// `horse/variant`.
    pub enum HorseVariant (Wrap) {
        White = 0, "white"; Creamy = 1, "creamy"; Chestnut = 2, "chestnut"; Brown = 3, "brown"; Black = 4, "black";
        Gray = 5, "gray"; DarkBrown = 6, "dark_brown";
    }
}

id_enum! {
    /// `llama/variant`.
    pub enum LlamaVariant (Clamp) { Creamy = 0, "creamy"; White = 1, "white"; Brown = 2, "brown"; Gray = 3, "gray"; }
}

id_enum! {
    /// `axolotl/variant`.
    pub enum AxolotlVariant (First) { Lucy = 0, "lucy"; Wild = 1, "wild"; Gold = 2, "gold"; Cyan = 3, "cyan"; Blue = 4, "blue"; }
}

/// An inline painting variant (`PaintingVariant.DIRECT_CODEC`).
#[derive(Debug, Clone, PartialEq)]
pub struct PaintingVariantDef {
    /// Blocks, 1..=16.
    pub width: i32,
    pub height: i32,
    pub asset_id: Identifier,
    pub title: Option<Text>,
    pub author: Option<Text>,
}

impl PaintingVariantDef {
    /// `PaintingVariant.DIRECT_STREAM_CODEC`.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(PaintingVariantDef {
            width: r.varint()?,
            height: r.varint()?,
            asset_id: Identifier::read(r)?,
            title: wire::read_opt(r, Text::read)?,
            author: wire::read_opt(r, Text::read)?,
        })
    }

    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.width);
        out.put_varint(self.height);
        self.asset_id.write(out);
        wire::write_opt(out, &self.title, Text::write);
        wire::write_opt(out, &self.author, Text::write);
    }

    pub fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("width", Value::Int(self.width))
            .put("height", Value::Int(self.height))
            .put("asset_id", self.asset_id.to_value())
            .opt("title", self.title.as_ref(), Text::to_value)
            .opt("author", self.author.as_ref(), Text::to_value)
            .build()
    }

    pub fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let size = |key: &str| {
            m.req_with(key, |v| match v.as_i32()? {
                n @ 1..=16 => Ok(n),
                n => err(format!("{n} out of range [1;16]")),
            })
        };
        Ok(PaintingVariantDef {
            width: size("width")?,
            height: size("height")?,
            asset_id: m.req_with("asset_id", Identifier::from_value)?,
            title: m.opt("title", Text::from_value)?,
            author: m.opt("author", Text::from_value)?,
        })
    }
}

/// `painting/variant`: a `minecraft:painting_variant` entry, or (on the network only) an inline
/// definition. Vanilla's persistent codec cannot save inline variants; they are written in
/// their direct form here.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintingVariant(pub Holder<PaintingVariantDef>);

impl ComponentValue for PaintingVariant {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Holder::read(registry::PAINTING_VARIANT, r, PaintingVariantDef::read).map(PaintingVariant)
    }
    fn write(&self, out: &mut BytesMut) {
        self.0.write(out, PaintingVariantDef::write);
    }
    fn to_value(&self) -> Value {
        self.0.to_value(registry::PAINTING_VARIANT, PaintingVariantDef::to_value)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Holder::from_value(registry::PAINTING_VARIANT, v, PaintingVariantDef::from_value).map(PaintingVariant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_range_ids_follow_by_id_map() {
        let ids = [0, 1, 2];
        assert_eq!(resolve(7, &ids, OutOfBounds::First), 0);
        assert_eq!(resolve(7, &ids, OutOfBounds::Clamp), 2);
        assert_eq!(resolve(-3, &ids, OutOfBounds::Clamp), 0);
        assert_eq!(resolve(-1, &ids, OutOfBounds::Wrap), 2);
        assert_eq!(TropicalFishPattern::Betty.id(), 1025);
        assert_eq!(RabbitVariant::Evil.id(), 99);
    }
}
