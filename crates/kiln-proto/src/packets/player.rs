//! State of the receiving player (play, clientbound): abilities, health, experience, respawn
//! and dimension changes, view and simulation distance, item cooldowns, the tick rate and the
//! camera. Layouts follow the 26.3 bytecode.

use super::packet;
use crate::WriteExt;
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets::play::clientbound as ids;

/// Player abilities; vanilla speeds are 0.05 (flying) and 0.1 (walking).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Abilities {
    pub invulnerable: bool,
    pub flying: bool,
    pub may_fly: bool,
    /// Creative mode: instant breaking, infinite items.
    pub instabuild: bool,
    pub flying_speed: f32,
    pub walking_speed: f32,
}

pub fn player_abilities(a: &Abilities) -> Bytes {
    let mut b = packet(ids::PLAYER_ABILITIES);
    let flags = a.invulnerable as u8 | (a.flying as u8) << 1 | (a.may_fly as u8) << 2 | (a.instabuild as u8) << 3;
    b.put_u8(flags);
    b.put_f32(a.flying_speed);
    b.put_f32(a.walking_speed);
    b.freeze()
}

/// Health (0 or less shows the death screen), food 0..=20, saturation.
pub fn set_health(health: f32, food: i32, saturation: f32) -> Bytes {
    let mut b = packet(ids::SET_HEALTH);
    b.put_f32(health);
    b.put_varint(food);
    b.put_f32(saturation);
    b.freeze()
}

/// `progress` is the fill of the experience bar, 0.0..=1.0.
pub fn set_experience(progress: f32, level: i32, total: i32) -> Bytes {
    let mut b = packet(ids::SET_EXPERIENCE);
    b.put_f32(progress);
    b.put_varint(level);
    b.put_varint(total);
    b.freeze()
}

/// `CommonPlayerSpawnInfo`: the dimension and game mode the player spawns into, shared by
/// `login` and [`respawn`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnInfo<'a> {
    /// Id in `minecraft:dimension_type`, in the order sent during configuration.
    pub dimension_type: i32,
    /// Dimension key, e.g. `minecraft:overworld`.
    pub dimension: &'a str,
    /// First 8 bytes of the SHA-256 of the world seed (biome noise on the client).
    pub hashed_seed: i64,
    /// Game mode id: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: u8,
    /// For the F3+F4 switcher; `None` when there is none.
    pub previous_game_mode: Option<u8>,
    pub is_debug: bool,
    pub is_flat: bool,
    /// Dimension and block position of the last death (recovery compasses).
    pub death_location: Option<(&'a str, [i32; 3])>,
    pub portal_cooldown: i32,
    pub sea_level: i32,
}

pub(crate) fn put_spawn_info(b: &mut BytesMut, s: &SpawnInfo) {
    b.put_varint(s.dimension_type);
    b.put_string(s.dimension);
    b.put_i64(s.hashed_seed);
    b.put_varint(s.game_mode as i32);
    b.put_varint(s.previous_game_mode.map_or(0, |m| m as i32 + 1));
    b.put_bool(s.is_debug);
    b.put_bool(s.is_flat);
    b.put_bool(s.death_location.is_some());
    if let Some((dimension, [x, y, z])) = s.death_location {
        b.put_string(dimension);
        b.put_position(x, y, z);
    }
    b.put_varint(s.portal_cooldown);
    b.put_varint(s.sea_level);
}

/// What the client keeps of the player entity across a [`respawn`] (bit set).
pub mod respawn_keep {
    pub const NOTHING: u8 = 0;
    pub const ATTRIBUTE_MODIFIERS: u8 = 1;
    pub const ENTITY_DATA: u8 = 2;
    pub const ALL: u8 = 3;
}

/// Respawn after death, or a dimension change (the client rebuilds its world).
pub fn respawn(spawn: &SpawnInfo, keep: u8) -> Bytes {
    let mut b = packet(ids::RESPAWN);
    put_spawn_info(&mut b, spawn);
    b.put_u8(keep);
    b.freeze()
}

pub fn set_simulation_distance(chunks: i32) -> Bytes {
    let mut b = packet(ids::SET_SIMULATION_DISTANCE);
    b.put_varint(chunks);
    b.freeze()
}

/// The server's view distance; the client unloads chunks beyond it.
pub fn set_chunk_cache_radius(chunks: i32) -> Bytes {
    let mut b = packet(ids::SET_CHUNK_CACHE_RADIUS);
    b.put_varint(chunks);
    b.freeze()
}

/// Puts a cooldown group on the client (an item's `use_cooldown` group, by default the item
/// id, e.g. `minecraft:ender_pearl`); 0 ticks clears it.
pub fn cooldown(group: &str, ticks: i32) -> Bytes {
    let mut b = packet(ids::COOLDOWN);
    b.put_string(group);
    b.put_varint(ticks);
    b.freeze()
}

/// The server tick rate (vanilla 20.0) and whether ticking is frozen (`/tick`).
pub fn ticking_state(tick_rate: f32, frozen: bool) -> Bytes {
    let mut b = packet(ids::TICKING_STATE);
    b.put_f32(tick_rate);
    b.put_bool(frozen);
    b.freeze()
}

/// Ticks left to step while frozen (`/tick step`).
pub fn ticking_step(steps: i32) -> Bytes {
    let mut b = packet(ids::TICKING_STEP);
    b.put_varint(steps);
    b.freeze()
}

/// Views the world from another entity (spectator mode); the player's own id resets it.
pub fn set_camera(entity_id: i32) -> Bytes {
    let mut b = packet(ids::SET_CAMERA);
    b.put_varint(entity_id);
    b.freeze()
}
