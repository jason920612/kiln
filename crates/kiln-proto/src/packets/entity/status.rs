//! Entity status packets: entity events, damage and hurt animations, item pickup, attributes
//! and mob effects. Layouts follow the 26.3 bytecode.

use super::super::packet;
use crate::WriteExt;
use bytes::{BufMut, Bytes};
use kiln_data::packets::play::clientbound as ids;

/// An `EntityEvent` id (e.g. 3 death, 9 use item complete, 24..=28 op permission level 0..=4
/// for the receiving player, 35 totem of undying).
pub fn entity_event(entity_id: i32, event: u8) -> Bytes {
    let mut b = packet(ids::ENTITY_EVENT);
    b.put_i32(entity_id);
    b.put_u8(event);
    b.freeze()
}

/// Damage taken, for the hurt effect and the damage tilt direction. `damage_type` is the id in
/// `minecraft:damage_type` (order sent during configuration); `cause` is the responsible entity
/// (e.g. the shooter), `direct` the one that hit (e.g. the arrow).
pub fn damage_event(
    entity_id: i32,
    damage_type: i32,
    cause: Option<i32>,
    direct: Option<i32>,
    source_pos: Option<[f64; 3]>,
) -> Bytes {
    let mut b = packet(ids::DAMAGE_EVENT);
    b.put_varint(entity_id);
    b.put_varint(damage_type);
    b.put_varint(cause.map_or(0, |id| id + 1));
    b.put_varint(direct.map_or(0, |id| id + 1));
    b.put_bool(source_pos.is_some());
    if let Some(pos) = source_pos {
        pos.iter().for_each(|c| b.put_f64(*c));
    }
    b.freeze()
}

/// Hurt tilt without a damage event; `yaw` is the direction the damage came from.
pub fn hurt_animation(entity_id: i32, yaw: f32) -> Bytes {
    let mut b = packet(ids::HURT_ANIMATION);
    b.put_varint(entity_id);
    b.put_f32(yaw);
    b.freeze()
}

/// The pickup animation of an item (or arrow, experience orb) flying to the collector. Send
/// before removing the item entity.
pub fn take_item_entity(item_id: i32, collector_id: i32, amount: i32) -> Bytes {
    let mut b = packet(ids::TAKE_ITEM_ENTITY);
    b.put_varint(item_id);
    b.put_varint(collector_id);
    b.put_varint(amount);
    b.freeze()
}

/// `AttributeModifier.Operation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierOperation {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeModifier<'a> {
    /// Identifier, e.g. `minecraft:sprinting`.
    pub id: &'a str,
    pub amount: f64,
    pub operation: ModifierOperation,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeSnapshot<'a> {
    /// Protocol id in `minecraft:attribute` (`kiln_data::builtin_id`).
    pub attribute: i32,
    pub base: f64,
    pub modifiers: &'a [AttributeModifier<'a>],
}

/// Vanilla rejects more attributes per packet.
pub const MAX_ATTRIBUTES: usize = 128;

pub fn update_attributes(entity_id: i32, attributes: &[AttributeSnapshot]) -> Bytes {
    debug_assert!(attributes.len() <= MAX_ATTRIBUTES);
    let mut b = packet(ids::UPDATE_ATTRIBUTES);
    b.put_varint(entity_id);
    b.put_varint(attributes.len() as i32);
    for a in attributes {
        b.put_varint(a.attribute);
        b.put_f64(a.base);
        b.put_varint(a.modifiers.len() as i32);
        for m in a.modifiers {
            b.put_string(m.id);
            b.put_f64(m.amount);
            b.put_varint(m.operation as i32);
        }
    }
    b.freeze()
}

/// Mob effect display bits.
pub mod effect_flags {
    pub const AMBIENT: u8 = 0x01;
    pub const VISIBLE: u8 = 0x02;
    pub const SHOW_ICON: u8 = 0x04;
    /// Fade the effect in (darkness, nausea).
    pub const BLEND: u8 = 0x08;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobEffect {
    /// Protocol id in `minecraft:mob_effect`.
    pub effect: i32,
    /// Level - 1.
    pub amplifier: i32,
    /// Ticks; -1 is infinite.
    pub duration: i32,
    /// [`effect_flags`] bits.
    pub flags: u8,
}

pub fn update_mob_effect(entity_id: i32, e: &MobEffect) -> Bytes {
    let mut b = packet(ids::UPDATE_MOB_EFFECT);
    b.put_varint(entity_id);
    b.put_varint(e.effect);
    b.put_varint(e.amplifier);
    b.put_varint(e.duration);
    b.put_u8(e.flags);
    b.freeze()
}

pub fn remove_mob_effect(entity_id: i32, effect: i32) -> Bytes {
    let mut b = packet(ids::REMOVE_MOB_EFFECT);
    b.put_varint(entity_id);
    b.put_varint(effect);
    b.freeze()
}
