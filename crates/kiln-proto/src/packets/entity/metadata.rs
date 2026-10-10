//! Entity data for `set_entity_data`: typed values for the `EntityDataSerializers` codecs and
//! an encoder for (index, serializer id, value) entries.
//!
//! Field definitions (index and serializer per entity class) are generated in
//! `kiln_data::entities::data`; e.g. `data::living_entity::HEALTH`.

use crate::WriteExt;
use crate::nbt::Tag;
use bytes::{BufMut, BytesMut};
use kiln_data::entities::{DataField, serializer as s};
use uuid::Uuid;

/// Bits of `data::entity::SHARED_FLAGS` (`Entity.FLAG_*`).
pub mod shared_flags {
    pub const ON_FIRE: u8 = 1 << 0;
    pub const CROUCHING: u8 = 1 << 1;
    pub const SPRINTING: u8 = 1 << 3;
    pub const SWIMMING: u8 = 1 << 4;
    pub const INVISIBLE: u8 = 1 << 5;
    pub const GLOWING: u8 = 1 << 6;
    pub const FALL_FLYING: u8 = 1 << 7;
}

/// Bits of `data::living_entity::LIVING_ENTITY_FLAGS`.
pub mod living_flags {
    pub const USING_ITEM: u8 = 1 << 0;
    pub const OFF_HAND: u8 = 1 << 1;
    pub const SPIN_ATTACK: u8 = 1 << 2;
}

/// Bits of `data::avatar::PLAYER_MODE_CUSTOMISATION` (`PlayerModelPart`), the same bits as the
/// client's skin-parts byte in Client Information.
pub mod model_parts {
    pub const CAPE: u8 = 1 << 0;
    pub const JACKET: u8 = 1 << 1;
    pub const LEFT_SLEEVE: u8 = 1 << 2;
    pub const RIGHT_SLEEVE: u8 = 1 << 3;
    pub const LEFT_PANTS_LEG: u8 = 1 << 4;
    pub const RIGHT_PANTS_LEG: u8 = 1 << 5;
    pub const HAT: u8 = 1 << 6;
    pub const ALL: u8 = 0x7f;
}

/// `Direction`, by 3D data value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

/// `HumanoidArm` (the main hand); the same ids as Client Information's main hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanoidArm {
    Left = 0,
    Right = 1,
}

/// An item stack without data components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStack {
    /// Protocol id in `minecraft:item`.
    pub item: i32,
    pub count: i32,
}

/// A particle: type id in `minecraft:particle_type` followed by its type-specific options,
/// already encoded by the caller (empty for simple particles such as `minecraft:flame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Particle {
    pub kind: i32,
    pub options: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalPos {
    /// Dimension id, e.g. `minecraft:overworld`.
    pub dimension: String,
    pub pos: [i32; 3],
}

/// A value for one data field. 
#[derive(Debug, Clone, PartialEq)]
pub enum DataValue {
    Byte(i8),
    /// `INT`: a VarInt.
    Int(i32),
    /// `LONG`: a VarLong.
    Long(i64),
    Float(f32),
    String(String),
    Component(Tag),
    OptionalComponent(Option<Tag>),
    ItemStack(Option<ItemStack>),
    /// `ITEM_STACK` already encoded with `ItemStack.OPTIONAL_STREAM_CODEC` (stacks with data
    /// components, encoded by kiln-item).
    EncodedItemStack(bytes::Bytes),
    /// `RESOLVABLE_PROFILE` already encoded (`ResolvableProfile.STREAM_CODEC`).
    EncodedProfile(bytes::Bytes),
    Boolean(bool),
    Rotations([f32; 3]),
    BlockPos([i32; 3]),
    OptionalBlockPos(Option<[i32; 3]>),
    Direction(Direction),
    /// `OPTIONAL_LIVING_ENTITY_REFERENCE`: an entity UUID.
    OptionalEntityReference(Option<Uuid>),
    /// Block state id.
    BlockState(i32),
    /// `None` and air (state 0) encode identically, as in vanilla.
    OptionalBlockState(Option<i32>),
    Particle(Particle),
    Particles(Vec<Particle>),
    /// Protocol ids in `minecraft:villager_type` and `minecraft:villager_profession`.
    VillagerData {
        kind: i32,
        profession: i32,
        level: i32,
    },
    /// `OPTIONAL_UNSIGNED_INT` (e.g. a shoulder parrot's variant).
    OptionalUnsignedInt(Option<u32>),
    /// A `kiln_data::entities::pose` id.
    Pose(i32),
    /// A registry entry by network id, for the `*_VARIANT` and `*_SOUND_VARIANT` serializers
    /// other than `PAINTING_VARIANT`.
    Holder(i32),
    /// `PAINTING_VARIANT`: a registered variant by network id (inline variants unsupported).
    PaintingVariant(i32),
    OptionalGlobalPos(Option<GlobalPos>),
    /// An enum id for `SNIFFER_STATE`, `ARMADILLO_STATE`, `COPPER_GOLEM_STATE`,
    /// `WEATHERING_COPPER_STATE` or `DYE_COLOR`.
    Enum(i32),
    HumanoidArm(HumanoidArm),
    Vector3([f32; 3]),
    /// (x, y, z, w).
    Quaternion([f32; 4]),
}

const HOLDER_SERIALIZERS: &[i32] = &[
    s::CAT_VARIANT,
    s::CAT_SOUND_VARIANT,
    s::COW_VARIANT,
    s::COW_SOUND_VARIANT,
    s::WOLF_VARIANT,
    s::WOLF_SOUND_VARIANT,
    s::FROG_VARIANT,
    s::PIG_VARIANT,
    s::PIG_SOUND_VARIANT,
    s::CHICKEN_VARIANT,
    s::CHICKEN_SOUND_VARIANT,
    s::ZOMBIE_NAUTILUS_VARIANT,
];

const ENUM_SERIALIZERS: &[i32] =
    &[s::SNIFFER_STATE, s::ARMADILLO_STATE, s::COPPER_GOLEM_STATE, s::WEATHERING_COPPER_STATE, s::DYE_COLOR];

impl DataValue {
    /// Whether this value is encoded by `serializer`.
    pub fn fits(&self, serializer: i32) -> bool {
        use DataValue as V;
        match self {
            V::Holder(_) => HOLDER_SERIALIZERS.contains(&serializer),
            V::Enum(_) => ENUM_SERIALIZERS.contains(&serializer),
            _ => {
                serializer
                    == match self {
                        V::Byte(_) => s::BYTE,
                        V::Int(_) => s::INT,
                        V::Long(_) => s::LONG,
                        V::Float(_) => s::FLOAT,
                        V::String(_) => s::STRING,
                        V::Component(_) => s::COMPONENT,
                        V::OptionalComponent(_) => s::OPTIONAL_COMPONENT,
                        V::ItemStack(_) | V::EncodedItemStack(_) => s::ITEM_STACK,
                        V::EncodedProfile(_) => s::RESOLVABLE_PROFILE,
                        V::Boolean(_) => s::BOOLEAN,
                        V::Rotations(_) => s::ROTATIONS,
                        V::BlockPos(_) => s::BLOCK_POS,
                        V::OptionalBlockPos(_) => s::OPTIONAL_BLOCK_POS,
                        V::Direction(_) => s::DIRECTION,
                        V::OptionalEntityReference(_) => s::OPTIONAL_LIVING_ENTITY_REFERENCE,
                        V::BlockState(_) => s::BLOCK_STATE,
                        V::OptionalBlockState(_) => s::OPTIONAL_BLOCK_STATE,
                        V::Particle(_) => s::PARTICLE,
                        V::Particles(_) => s::PARTICLES,
                        V::VillagerData { .. } => s::VILLAGER_DATA,
                        V::OptionalUnsignedInt(_) => s::OPTIONAL_UNSIGNED_INT,
                        V::Pose(_) => s::POSE,
                        V::PaintingVariant(_) => s::PAINTING_VARIANT,
                        V::OptionalGlobalPos(_) => s::OPTIONAL_GLOBAL_POS,
                        V::HumanoidArm(_) => s::HUMANOID_ARM,
                        V::Vector3(_) => s::VECTOR3,
                        V::Quaternion(_) => s::QUATERNION,
                        V::Holder(_) | V::Enum(_) => unreachable!(),
                    }
            }
        }
    }

    /// Writes the value with its serializer's codec (no index or serializer id).
    pub fn write(&self, b: &mut BytesMut) {
        use DataValue as V;
        match self {
            V::Byte(v) => b.put_i8(*v),
            V::Int(v) | V::BlockState(v) | V::Pose(v) | V::Holder(v) | V::Enum(v) => b.put_varint(*v),
            V::Long(v) => b.put_varlong(*v),
            V::Float(v) => b.put_f32(*v),
            V::String(v) => b.put_string(v),
            V::Component(t) => t.write_network(b),
            V::OptionalComponent(t) => {
                b.put_bool(t.is_some());
                if let Some(t) = t {
                    t.write_network(b);
                }
            }
            V::ItemStack(None) => b.put_varint(0),
            V::ItemStack(Some(stack)) => {
                b.put_varint(stack.count);
                b.put_varint(stack.item);
                b.put_varint(0); // component patch: nothing added
                b.put_varint(0); // nothing removed
            }
            V::EncodedItemStack(bytes) | V::EncodedProfile(bytes) => b.put_slice(bytes),
            V::Boolean(v) => b.put_bool(*v),
            V::Rotations(v) | V::Vector3(v) => v.iter().for_each(|f| b.put_f32(*f)),
            V::Quaternion(v) => v.iter().for_each(|f| b.put_f32(*f)),
            V::BlockPos([x, y, z]) => b.put_position(*x, *y, *z),
            V::OptionalBlockPos(p) => {
                b.put_bool(p.is_some());
                if let Some([x, y, z]) = p {
                    b.put_position(*x, *y, *z);
                }
            }
            V::Direction(d) => b.put_varint(*d as i32),
            V::OptionalEntityReference(u) => {
                b.put_bool(u.is_some());
                if let Some(u) = u {
                    b.put_uuid(*u);
                }
            }
            V::OptionalBlockState(state) => b.put_varint(state.unwrap_or(0)),
            V::Particle(p) => put_particle(b, p),
            V::Particles(list) => {
                b.put_varint(list.len() as i32);
                list.iter().for_each(|p| put_particle(b, p));
            }
            V::VillagerData { kind, profession, level } => {
                b.put_varint(*kind);
                b.put_varint(*profession);
                b.put_varint(*level);
            }
            V::OptionalUnsignedInt(v) => b.put_varint(v.map_or(0, |v| v as i32 + 1)),
            // ByteBufCodecs.holder: 0 means an inline value, otherwise registry id + 1.
            V::PaintingVariant(id) => b.put_varint(id + 1),
            V::OptionalGlobalPos(g) => {
                b.put_bool(g.is_some());
                if let Some(GlobalPos { dimension, pos: [x, y, z] }) = g {
                    b.put_string(dimension);
                    b.put_position(*x, *y, *z);
                }
            }
            V::HumanoidArm(arm) => b.put_varint(*arm as i32),
        }
    }
}

fn put_particle(b: &mut BytesMut, p: &Particle) {
    b.put_varint(p.kind);
    b.put_slice(&p.options);
}

/// Encoded entries for one `set_entity_data` packet.
#[derive(Debug, Clone, Default)]
pub struct EntityData {
    buf: BytesMut,
}

impl EntityData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `field = value`.
    ///
    /// # Panics
    /// If `value` is not encoded by the field's serializer (the client would fail to decode it).
    pub fn set(&mut self, field: DataField, value: &DataValue) -> &mut Self {
        assert!(value.fits(field.serializer), "{value:?} does not fit serializer {}", field.serializer);
        self.buf.put_u8(field.index);
        self.buf.put_varint(field.serializer);
        value.write(&mut self.buf);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The entries without the terminator.
    pub fn entries(&self) -> &[u8] {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::entities::{data, pose, types};

    fn encode(field: DataField, value: DataValue) -> Vec<u8> {
        let mut d = EntityData::new();
        d.set(field, &value);
        d.entries().to_vec()
    }

    #[test]
    fn entry_is_index_serializer_value() {
        assert_eq!(encode(data::living_entity::HEALTH, DataValue::Float(20.0)), [9, 3, 0x41, 0xa0, 0, 0]);
        assert_eq!(encode(data::entity::POSE, DataValue::Pose(pose::CROUCHING)), [6, 20, 5]);
        assert_eq!(encode(data::avatar::PLAYER_MODE_CUSTOMISATION, DataValue::Byte(0x7f)), [16, 0, 0x7f]);
        assert_eq!(encode(data::avatar::PLAYER_MAIN_HAND, DataValue::HumanoidArm(HumanoidArm::Left)), [15, 42, 0]);
        assert_eq!(encode(data::entity::AIR_SUPPLY, DataValue::Int(300)), [1, 1, 0xac, 0x02]);
    }

    #[test]
    fn optional_encodings() {
        let parrot = data::player::SHOULDER_PARROT_LEFT;
        assert_eq!(encode(parrot, DataValue::OptionalUnsignedInt(None)), [19, 19, 0]);
        assert_eq!(encode(parrot, DataValue::OptionalUnsignedInt(Some(3))), [19, 19, 4]);
        let name = data::entity::CUSTOM_NAME;
        assert_eq!(encode(name, DataValue::OptionalComponent(None)), [2, 6, 0]);
        assert_eq!(
            encode(name, DataValue::OptionalComponent(Some(crate::nbt::text("Hi")))),
            [2, 6, 1, 8, 0, 2, b'H', b'i']
        );
        let sleeping = data::living_entity::SLEEPING_POS;
        assert_eq!(encode(sleeping, DataValue::OptionalBlockPos(None)), [14, 11, 0]);
        let mut want = vec![14, 11, 1];
        want.extend_from_slice(&((1i64 << 38) | (3 << 12) | 2).to_be_bytes());
        assert_eq!(encode(sleeping, DataValue::OptionalBlockPos(Some([1, 2, 3]))), want);
    }

    #[test]
    fn player_fields_cover_the_hierarchy() {
        let fields = types::PLAYER.fields();
        let indices: Vec<u8> = fields.iter().map(|(_, f)| f.index).collect();
        assert_eq!(indices, (0..=20).collect::<Vec<u8>>());
        assert_eq!(fields[0], ("DATA_SHARED_FLAGS_ID", data::entity::SHARED_FLAGS));
        assert_eq!(fields[16], ("DATA_PLAYER_MODE_CUSTOMISATION", data::avatar::PLAYER_MODE_CUSTOMISATION));
        assert_eq!(fields[20].1, data::player::SHOULDER_PARROT_RIGHT);
    }

    #[test]
    fn holder_and_enum_values_fit_their_serializers() {
        assert!(DataValue::Holder(0).fits(s::CAT_VARIANT));
        assert!(!DataValue::Holder(0).fits(s::PAINTING_VARIANT));
        assert!(DataValue::Enum(0).fits(s::DYE_COLOR));
        assert!(!DataValue::Enum(0).fits(s::INT));
        assert!(DataValue::Int(0).fits(s::INT));
        assert!(!DataValue::Int(0).fits(s::BYTE));
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn mismatched_serializer_panics() {
        EntityData::new().set(data::living_entity::HEALTH, &DataValue::Int(20));
    }
}
