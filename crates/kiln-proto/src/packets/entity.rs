//! Entity packets (play, clientbound). Layouts follow the 26.3 bytecode and are checked
//! against vanilla's codecs (`tools/entity_vectors.py`).
//!
//! # Making players visible to each other
//!
//! Per player, keep a [`MovementTracker`] (created at join with
//! `kiln_data::entities::types::PLAYER.update_interval`) and the set of players viewing it.
//!
//! 1. **Join:** send [`player_info_update`] with [`PlayerInfoActions::INITIALIZE`] and the
//!    joining player's entry (name, `textures` property, game mode) to everyone, and one packet
//!    with every online player's entry to the joiner. The client ignores `add_entity` for a
//!    player whose info it has not received.
//! 2. **Start viewing** (viewer within `PLAYER.tracking_range` chunks and its view distance):
//!    send [`bundle_delimiter`], `tracker.spawn(uuid, PLAYER.id, [0.0; 3], 0)`,
//!    [`set_entity_data`] with the non-default fields, [`bundle_delimiter`]. For a player the
//!    usual non-defaults are `data::living_entity::HEALTH` (default 1.0) and
//!    `data::avatar::PLAYER_MODE_CUSTOMISATION` (skin parts from Client Information; default 0
//!    hides the hat and jacket layers). Also `PLAYER_MAIN_HAND` if left-handed.
//! 3. **Every tick:** `tracker.tick(&MoveState { .. })` and send the returned packets to that
//!    player's viewers (not to the player itself). For players, pass the client's yaw as both
//!    `yaw` and `head_yaw`. Changed flags (crouching, sprinting) go out as [`set_entity_data`]
//!    with `data::entity::SHARED_FLAGS` and `data::entity::POSE`. Arm swings use
//!    [`swing_animation`] (26.3 moved them out of [`animate`]).
//! 4. **Stop viewing:** [`remove_entities`] to that viewer.
//! 5. **Leave:** [`remove_entities`] to its viewers and [`player_info_remove`] to everyone.
//!
//! [`teleport_entity`] is for explicit teleports; it does not move the delta base, so keep
//! calling `tick` (the next update sends a full [`entity_position_sync`] if needed).

pub mod metadata;
pub mod movement;
pub mod player_info;
pub mod status;

pub use metadata::{DataValue, EntityData};
pub use movement::{MoveState, MovementTracker, PositionCodec, put_lp_vec3, read_lp_vec3};
pub use player_info::{ChatSession, PlayerInfoActions, PlayerInfoEntry, player_info_remove, player_info_update};
pub use status::{
    AttributeModifier, AttributeSnapshot, MobEffect, ModifierOperation, damage_event, effect_flags, entity_event,
    hurt_animation, remove_mob_effect, take_item_entity, update_attributes, update_mob_effect,
};

use super::packet;
use crate::WriteExt;
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets::play::clientbound as ids;
use uuid::Uuid;

/// An angle in 1/256 turns (`Mth.packDegrees`), as sent in movement packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Angle(pub i8);

impl Angle {
    pub fn from_degrees(degrees: f32) -> Self {
        // Scale in f32, floor in f64 (Mth.floor(float)), saturate to int, wrap to a byte.
        Angle(((degrees * 256.0 / 360.0) as f64).floor() as i32 as i8)
    }

    pub fn degrees(self) -> f32 {
        (self.0 as i32 * 360) as f32 / 256.0
    }
}

/// A relative move in 1/4096 blocks (see [`PositionCodec`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PosDelta {
    Linear([i16; 3]),
    /// Intermediate positions for the client to interpolate through: (delta from the previous
    /// step, ticks until this step).
    Stepped(Vec<([i16; 3], i32)>),
}

impl PosDelta {
    fn write(&self, b: &mut BytesMut) {
        match self {
            PosDelta::Linear(d) => d.iter().for_each(|v| b.put_i16(*v)),
            PosDelta::Stepped(steps) => {
                for (d, ticks) in steps {
                    b.put_varint(*ticks);
                    d.iter().for_each(|v| b.put_i16(*v));
                }
            }
        }
    }

    fn step_count(&self) -> i32 {
        match self {
            PosDelta::Linear(_) => 0,
            PosDelta::Stepped(steps) => steps.len() as i32,
        }
    }
}

/// An absolute position (`PositionPath`): one point, or steps the client interpolates through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionPath<'a> {
    Linear([f64; 3]),
    /// (position, tick offset) per step; the last position is where the entity ends up.
    Stepped(&'a [([f64; 3], i32)]),
}

/// Which parts of a [`teleport_entity`] are relative (`Relative` bit set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Relative(pub i32);

impl Relative {
    pub const ABSOLUTE: Self = Self(0);
    pub const X: Self = Self(1 << 0);
    pub const Y: Self = Self(1 << 1);
    pub const Z: Self = Self(1 << 2);
    pub const Y_ROT: Self = Self(1 << 3);
    pub const X_ROT: Self = Self(1 << 4);
    pub const DELTA_X: Self = Self(1 << 5);
    pub const DELTA_Y: Self = Self(1 << 6);
    pub const DELTA_Z: Self = Self(1 << 7);
    /// Rotate the velocity by the rotation change.
    pub const ROTATE_DELTA: Self = Self(1 << 8);
}

impl std::ops::BitOr for Relative {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

fn put_vec3(b: &mut BytesMut, v: [f64; 3]) {
    v.iter().for_each(|c| b.put_f64(*c));
}

/// Spawns a non-block, non-experience-orb entity (players included).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddEntity {
    pub entity_id: i32,
    pub uuid: Uuid,
    /// Protocol id in `minecraft:entity_type` (`kiln_data::entities::types::*.id`).
    pub kind: i32,
    pub pos: [f64; 3],
    pub velocity: [f64; 3],
    pub pitch: Angle,
    pub yaw: Angle,
    pub head_yaw: Angle,
    /// Type-specific: e.g. the block state of a falling block, the owner id of a projectile.
    pub data: i32,
}

pub fn add_entity(e: &AddEntity) -> Bytes {
    let mut b = packet(ids::ADD_ENTITY);
    b.put_varint(e.entity_id);
    b.put_uuid(e.uuid);
    b.put_varint(e.kind);
    put_vec3(&mut b, e.pos);
    put_lp_vec3(&mut b, e.velocity);
    b.put_i8(e.pitch.0);
    b.put_i8(e.yaw.0);
    b.put_i8(e.head_yaw.0);
    b.put_varint(e.data);
    b.freeze()
}

pub fn remove_entities(entity_ids: &[i32]) -> Bytes {
    let mut b = packet(ids::REMOVE_ENTITIES);
    b.put_varint(entity_ids.len() as i32);
    for id in entity_ids {
        b.put_varint(*id);
    }
    b.freeze()
}

/// Flags VarInt of `move_entity_pos*`: on-ground bit, then the step count.
fn move_properties(on_ground: bool, delta: &PosDelta) -> i32 {
    on_ground as i32 | delta.step_count() << 1
}

pub fn move_entity_pos(entity_id: i32, delta: &PosDelta, on_ground: bool) -> Bytes {
    let mut b = packet(ids::MOVE_ENTITY_POS);
    b.put_varint(entity_id);
    b.put_varint(move_properties(on_ground, delta));
    delta.write(&mut b);
    b.freeze()
}

pub fn move_entity_pos_rot(entity_id: i32, delta: &PosDelta, yaw: Angle, pitch: Angle, on_ground: bool) -> Bytes {
    let mut b = packet(ids::MOVE_ENTITY_POS_ROT);
    b.put_varint(entity_id);
    b.put_varint(move_properties(on_ground, delta));
    delta.write(&mut b);
    b.put_i8(yaw.0);
    b.put_i8(pitch.0);
    b.freeze()
}

pub fn move_entity_rot(entity_id: i32, yaw: Angle, pitch: Angle, on_ground: bool) -> Bytes {
    let mut b = packet(ids::MOVE_ENTITY_ROT);
    b.put_varint(entity_id);
    b.put_bool(on_ground);
    b.put_i8(yaw.0);
    b.put_i8(pitch.0);
    b.freeze()
}

pub fn rotate_head(entity_id: i32, head_yaw: Angle) -> Bytes {
    let mut b = packet(ids::ROTATE_HEAD);
    b.put_varint(entity_id);
    b.put_i8(head_yaw.0);
    b.freeze()
}

/// Full-precision position and rotation; resets the client's delta base.
pub fn entity_position_sync(entity_id: i32, path: &PositionPath, yaw: f32, pitch: f32, on_ground: bool) -> Bytes {
    let mut b = packet(ids::ENTITY_POSITION_SYNC);
    b.put_varint(entity_id);
    match path {
        PositionPath::Linear(pos) => {
            b.put_varint(0);
            put_vec3(&mut b, *pos);
        }
        PositionPath::Stepped(steps) => {
            b.put_varint(1);
            b.put_varint(steps.len() as i32);
            for (pos, tick_offset) in *steps {
                put_vec3(&mut b, *pos);
                b.put_varint(*tick_offset);
            }
        }
    }
    b.put_f32(yaw);
    b.put_f32(pitch);
    b.put_bool(on_ground);
    b.freeze()
}

/// Teleports an entity (not the receiving player itself: that is `player_position`).
pub fn teleport_entity(
    entity_id: i32,
    pos: [f64; 3],
    velocity: [f64; 3],
    yaw: f32,
    pitch: f32,
    relative: Relative,
    on_ground: bool,
) -> Bytes {
    let mut b = packet(ids::TELEPORT_ENTITY);
    b.put_varint(entity_id);
    put_vec3(&mut b, pos);
    put_vec3(&mut b, velocity);
    b.put_f32(yaw);
    b.put_f32(pitch);
    b.put_i32(relative.0);
    b.put_bool(on_ground);
    b.freeze()
}

pub fn set_entity_data(entity_id: i32, data: &EntityData) -> Bytes {
    let entries = data.entries();
    let mut b = BytesMut::with_capacity(8 + entries.len());
    b.put_varint(ids::SET_ENTITY_DATA);
    b.put_varint(entity_id);
    b.put_slice(entries);
    b.put_u8(0xff);
    b.freeze()
}

/// Velocity in blocks per tick.
pub fn set_entity_motion(entity_id: i32, velocity: [f64; 3]) -> Bytes {
    let mut b = packet(ids::SET_ENTITY_MOTION);
    b.put_varint(entity_id);
    put_lp_vec3(&mut b, velocity);
    b.freeze()
}

/// `animate` actions (`ClientboundAnimatePacket`); arm swings are [`swing_animation`].
pub mod animation {
    pub const WAKE_UP: u8 = 0;
    pub const CRITICAL_HIT: u8 = 1;
    pub const MAGIC_CRITICAL_HIT: u8 = 2;
}

pub fn animate(entity_id: i32, action: u8) -> Bytes {
    let mut b = packet(ids::ANIMATE);
    b.put_varint(entity_id);
    b.put_u8(action);
    b.freeze()
}

/// `SwingAnimationType` ids.
pub mod swing {
    pub const NONE: i32 = 0;
    pub const WHACK: i32 = 1;
    pub const STAB: i32 = 2;
    /// `SwingAnimation.DEFAULT` duration (with `WHACK`).
    pub const DEFAULT_DURATION: i32 = 6;
}

/// Arm swing; `off_hand` selects the hand. Vanilla's default is `swing::WHACK` for
/// `swing::DEFAULT_DURATION` ticks.
pub fn swing_animation(entity_id: i32, off_hand: bool, kind: i32, duration: i32) -> Bytes {
    let mut b = packet(ids::SWING_ANIMATION);
    b.put_varint(entity_id);
    b.put_varint(off_hand as i32);
    b.put_varint(kind);
    b.put_varint(duration);
    b.freeze()
}

/// Starts or ends a bundle: the client applies the packets in between in one frame.
pub fn bundle_delimiter() -> Bytes {
    packet(ids::BUNDLE_DELIMITER).freeze()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Reader;

    #[test]
    fn angle_packing_matches_mth() {
        let cases =
            [(0.0, 0), (90.0, 64), (-90.0, -64), (180.0, -128), (359.0, -1), (-0.1, -1), (45.7, 32), (720.0, 0)];
        for (deg, want) in cases {
            assert_eq!(Angle::from_degrees(deg).0, want, "{deg}");
        }
        assert_eq!(Angle(64).degrees(), 90.0);
        assert_eq!(Angle(-128).degrees(), -180.0);
    }

    #[test]
    fn move_entity_pos_layout() {
        let p = move_entity_pos(300, &PosDelta::Linear([1, -1, 4096]), true);
        assert_eq!(&p[..], &[ids::MOVE_ENTITY_POS as u8, 0xac, 0x02, 1, 0, 1, 0xff, 0xff, 0x10, 0][..]);
        let stepped = PosDelta::Stepped(vec![([1, 2, 3], 1), ([0, 0, -1], 2)]);
        let p = move_entity_pos(1, &stepped, false);
        let mut r = Reader::new(&p[1..]);
        assert_eq!((r.varint().unwrap(), r.varint().unwrap()), (1, 4));
        assert_eq!((r.varint().unwrap(), r.i16().unwrap(), r.i16().unwrap(), r.i16().unwrap()), (1, 1, 2, 3));
        assert_eq!((r.varint().unwrap(), r.i16().unwrap(), r.i16().unwrap(), r.i16().unwrap()), (2, 0, 0, -1));
        r.finish().unwrap();
    }

    #[test]
    fn rotation_orders_differ_between_packets() {
        // add_entity: pitch, yaw, head yaw; move_entity_*: yaw, pitch.
        let e = AddEntity {
            entity_id: 1,
            uuid: Uuid::nil(),
            kind: 0,
            pos: [0.0; 3],
            velocity: [0.0; 3],
            pitch: Angle(1),
            yaw: Angle(2),
            head_yaw: Angle(3),
            data: 0,
        };
        let p = add_entity(&e);
        assert_eq!(&p[p.len() - 4..], &[1, 2, 3, 0]);
        let p = move_entity_rot(1, Angle(2), Angle(1), false);
        assert_eq!(&p[1..], &[1, 0, 2, 1]);
    }

    #[test]
    fn set_entity_data_is_terminated() {
        let empty = set_entity_data(5, &EntityData::new());
        assert_eq!(&empty[1..], &[5, 0xff]);
    }
}
