//! Packet bodies (packet id + data) for the states the server speaks so far.
//! Field layouts follow the 26.3 protocol (minecraft.wiki, checked against the jar).

pub mod entity;

use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets as ids;
use crate::nbt::Tag;
use crate::{DecodeError, Reader, WriteExt};
use uuid::Uuid;

fn packet(id: i32) -> BytesMut {
    let mut b = BytesMut::with_capacity(64);
    b.put_varint(id);
    b
}

// ---- status -------------------------------------------------------------------------------

pub fn status_response(json: &str) -> Bytes {
    let mut b = packet(ids::status::clientbound::STATUS_RESPONSE);
    b.put_string(json);
    b.freeze()
}

pub fn status_pong(ts: i64) -> Bytes {
    let mut b = packet(ids::status::clientbound::PONG_RESPONSE);
    b.put_i64(ts);
    b.freeze()
}

// ---- login --------------------------------------------------------------------------------

pub fn login_disconnect(reason: &str) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_DISCONNECT);
    b.put_string(&serde_json::json!({ "text": reason }).to_string());
    b.freeze()
}

pub fn login_compression(threshold: i32) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_COMPRESSION);
    b.put_varint(threshold);
    b.freeze()
}

/// A game profile property, e.g. `textures`.
pub struct ProfileProperty<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub signature: Option<&'a str>,
}

pub fn put_game_profile(b: &mut BytesMut, uuid: Uuid, name: &str, properties: &[ProfileProperty]) {
    b.put_uuid(uuid);
    b.put_string(name);
    b.put_varint(properties.len() as i32);
    for p in properties {
        b.put_string(p.name);
        b.put_string(p.value);
        b.put_bool(p.signature.is_some());
        if let Some(sig) = p.signature {
            b.put_string(sig);
        }
    }
}

pub fn login_finished(uuid: Uuid, name: &str, properties: &[ProfileProperty], session: Uuid) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_FINISHED);
    put_game_profile(&mut b, uuid, name, properties);
    b.put_uuid(session);
    b.freeze()
}

// ---- configuration ------------------------------------------------------------------------

pub fn config_brand(brand: &str) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::CUSTOM_PAYLOAD);
    b.put_string("minecraft:brand");
    b.put_string(brand);
    b.freeze()
}

pub fn select_known_packs(packs: &[(&str, &str, &str)]) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::SELECT_KNOWN_PACKS);
    b.put_varint(packs.len() as i32);
    for (ns, id, version) in packs {
        b.put_string(ns);
        b.put_string(id);
        b.put_string(version);
    }
    b.freeze()
}

pub fn update_enabled_features(flags: &[&str]) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::UPDATE_ENABLED_FEATURES);
    b.put_varint(flags.len() as i32);
    for f in flags {
        b.put_string(f);
    }
    b.freeze()
}

/// Registry Data with entry names only; the client fills the data from `minecraft:core`.
pub fn registry_data(registry: &str, entries: &[&str]) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::REGISTRY_DATA);
    b.put_string(registry);
    b.put_varint(entries.len() as i32);
    for e in entries {
        b.put_string(e);
        b.put_bool(false);
    }
    b.freeze()
}

pub fn update_tags(id: i32, tags: &[(&str, &[(&str, &[i32])])]) -> Bytes {
    let mut b = packet(id);
    b.put_varint(tags.len() as i32);
    for (registry, list) in tags {
        b.put_string(registry);
        b.put_varint(list.len() as i32);
        for (tag, entries) in *list {
            b.put_string(tag);
            b.put_varint(entries.len() as i32);
            for e in *entries {
                b.put_varint(*e);
            }
        }
    }
    b.freeze()
}

pub fn finish_configuration() -> Bytes {
    packet(ids::configuration::clientbound::FINISH_CONFIGURATION).freeze()
}

pub fn config_disconnect(reason: &str) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::DISCONNECT);
    crate::nbt::text(reason).write_network(&mut b);
    b.freeze()
}

// ---- play ---------------------------------------------------------------------------------

pub struct Login<'a> {
    pub entity_id: i32,
    pub dimensions: &'a [&'a str],
    pub max_players: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub dimension_type: i32,
    pub dimension: &'a str,
    pub game_mode: u8,
    pub is_flat: bool,
    pub sea_level: i32,
}

pub fn play_login(l: &Login) -> Bytes {
    let mut b = packet(ids::play::clientbound::LOGIN);
    b.put_i32(l.entity_id);
    b.put_bool(false); // hardcore
    b.put_varint(l.dimensions.len() as i32);
    for d in l.dimensions {
        b.put_string(d);
    }
    b.put_varint(l.max_players);
    b.put_varint(l.view_distance);
    b.put_varint(l.simulation_distance);
    b.put_bool(false); // reduced debug info
    b.put_bool(true); // show respawn screen
    b.put_bool(false); // limited crafting
    // CommonPlayerSpawnInfo
    b.put_varint(l.dimension_type);
    b.put_string(l.dimension);
    b.put_i64(0); // hashed seed
    b.put_varint(l.game_mode as i32);
    b.put_varint(0); // previous game mode: none
    b.put_bool(false); // debug world
    b.put_bool(l.is_flat);
    b.put_bool(false); // no death location
    b.put_varint(0); // portal cooldown
    b.put_varint(l.sea_level);
    // Login
    b.put_bool(false); // online mode
    b.put_bool(false); // enforces secure chat
    b.freeze()
}

pub fn game_event(event: u8, value: f32) -> Bytes {
    let mut b = packet(ids::play::clientbound::GAME_EVENT);
    b.put_u8(event);
    b.put_f32(value);
    b.freeze()
}

pub const GAME_EVENT_START_WAITING_FOR_CHUNKS: u8 = 13;

pub fn player_position(teleport_id: i32, pos: [f64; 3], yaw: f32, pitch: f32) -> Bytes {
    let mut b = packet(ids::play::clientbound::PLAYER_POSITION);
    b.put_varint(teleport_id);
    for v in pos {
        b.put_f64(v);
    }
    for _ in 0..3 {
        b.put_f64(0.0); // velocity
    }
    b.put_f32(yaw);
    b.put_f32(pitch);
    b.put_i32(0); // all absolute
    b.freeze()
}

pub fn set_default_spawn_position(dimension: &str, pos: [i32; 3], yaw: f32, pitch: f32) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_DEFAULT_SPAWN_POSITION);
    b.put_string(dimension);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_f32(yaw);
    b.put_f32(pitch);
    b.freeze()
}

pub fn set_chunk_cache_center(x: i32, z: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_CHUNK_CACHE_CENTER);
    b.put_varint(x);
    b.put_varint(z);
    b.freeze()
}

pub fn chunk_batch_start() -> Bytes {
    packet(ids::play::clientbound::CHUNK_BATCH_START).freeze()
}

pub fn chunk_batch_finished(count: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::CHUNK_BATCH_FINISHED);
    b.put_varint(count);
    b.freeze()
}

/// `body` is everything after the chunk coordinates (heightmaps, sections, block entities, light).
pub fn level_chunk_with_light(x: i32, z: i32, body: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(16 + body.len());
    b.put_varint(ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT);
    b.put_i32(x);
    b.put_i32(z);
    b.put_slice(body);
    b.freeze()
}

/// `body` is Light Data (masks and arrays) for the changed sections.
pub fn light_update(x: i32, z: i32, body: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(8 + body.len());
    b.put_varint(ids::play::clientbound::LIGHT_UPDATE);
    b.put_varint(x);
    b.put_varint(z);
    b.put_slice(body);
    b.freeze()
}

pub fn forget_level_chunk(x: i32, z: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::FORGET_LEVEL_CHUNK);
    // ChunkPos written as a single long: z in the high half, x in the low half.
    b.put_i64(((z as i64) << 32) | (x as u32 as i64));
    b.freeze()
}

pub fn keep_alive(id: i64) -> Bytes {
    let mut b = packet(ids::play::clientbound::KEEP_ALIVE);
    b.put_i64(id);
    b.freeze()
}

pub fn system_chat(text: Tag, overlay: bool) -> Bytes {
    let mut b = packet(ids::play::clientbound::SYSTEM_CHAT);
    text.write_network(&mut b);
    b.put_bool(overlay);
    b.freeze()
}

pub fn play_disconnect(reason: &str) -> Bytes {
    let mut b = packet(ids::play::clientbound::DISCONNECT);
    crate::nbt::text(reason).write_network(&mut b);
    b.freeze()
}

// ---- serverbound play ---------------------------------------------------------------------

/// Serverbound play packets the simulation cares about.
#[derive(Debug)]
pub enum PlayIn {
    AcceptTeleport { id: i32 },
    KeepAlive { id: i64 },
    Move { pos: Option<[f64; 3]>, rot: Option<[f32; 2]>, on_ground: bool },
    ChunkBatchReceived { chunks_per_tick: f32 },
    ClientInformation { view_distance: u8 },
    Chat { message: String },
    PlayerLoaded,
    PlayerAction { action: i32, pos: [i32; 3], face: u8, sequence: i32 },
    UseItemOn { hand: i32, pos: [i32; 3], face: i32, cursor: [f32; 3], inside: bool, sequence: i32 },
    SetCarriedItem { slot: i16 },
    SetCreativeSlot { slot: i16, item: Option<ItemStack> },
    /// Arm swing (26.3 renamed `swing` to `punch`; it has no fields).
    Punch,
}

/// An item stack as sent by the client. Component patches are kept as raw, length-delimited
/// entries (the "untrusted" slot codec prefixes each component with its length).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    pub item: i32,
    pub count: i32,
    pub added: Vec<(i32, Bytes)>,
    pub removed: Vec<i32>,
}

/// Block position packed as x:26, z:26, y:12.
pub fn read_position(r: &mut Reader) -> Result<[i32; 3], DecodeError> {
    let v = r.i64()?;
    Ok([(v >> 38) as i32, (v << 52 >> 52) as i32, (v << 26 >> 38) as i32])
}

/// `ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC`: count, then item id and a delimited component patch.
pub fn read_untrusted_slot(r: &mut Reader) -> Result<Option<ItemStack>, DecodeError> {
    let count = r.varint()?;
    if count <= 0 {
        return Ok(None);
    }
    let item = r.varint()?;
    let adds = r.len()?;
    let removes = r.len()?;
    if adds + removes > 256 {
        return Err(DecodeError::Invalid("too many item components"));
    }
    let mut added = Vec::with_capacity(adds);
    for _ in 0..adds {
        let ty = r.varint()?;
        let len = r.len()?;
        if len > 2 * 1024 * 1024 {
            return Err(DecodeError::Invalid("item component too large"));
        }
        added.push((ty, Bytes::copy_from_slice(r.bytes(len)?)));
    }
    let mut removed = Vec::with_capacity(removes);
    for _ in 0..removes {
        removed.push(r.varint()?);
    }
    Ok(Some(ItemStack { item, count, added, removed }))
}

/// World clock state for Set Time: registry id in `minecraft:world_clock`, time, rate.
pub struct ClockState {
    pub clock: i32,
    pub time: i64,
    pub fraction: f32,
    pub rate: f32,
}

/// 26.x Set Time: world age plus the state of each world clock.
pub fn set_time(game_time: i64, clocks: &[ClockState]) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_TIME);
    b.put_i64(game_time);
    b.put_varint(clocks.len() as i32);
    for c in clocks {
        b.put_varint(c.clock);
        b.put_varlong(c.time);
        b.put_f32(c.fraction);
        b.put_f32(c.rate);
    }
    b.freeze()
}

pub fn block_update(pos: [i32; 3], state: u16) -> Bytes {
    let mut b = packet(ids::play::clientbound::BLOCK_UPDATE);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_varint(state as i32);
    b.freeze()
}

pub fn block_changed_ack(sequence: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::BLOCK_CHANGED_ACK);
    b.put_varint(sequence);
    b.freeze()
}

/// Decodes a serverbound play packet; `Ok(None)` for packets we ignore for now.
pub fn decode_play(id: i32, r: &mut Reader) -> Result<Option<PlayIn>, DecodeError> {
    use ids::play::serverbound as sb;
    let pkt = match id {
        sb::ACCEPT_TELEPORTATION => {
            let id = r.varint()?;
            r.rest(); // echoed position and rotation
            PlayIn::AcceptTeleport { id }
        }
        sb::KEEP_ALIVE => PlayIn::KeepAlive { id: r.i64()? },
        sb::MOVE_PLAYER_POS => {
            let pos = [r.f64()?, r.f64()?, r.f64()?];
            PlayIn::Move { pos: Some(pos), rot: None, on_ground: r.u8()? & 1 != 0 }
        }
        sb::MOVE_PLAYER_POS_ROT => {
            let pos = [r.f64()?, r.f64()?, r.f64()?];
            let rot = [r.f32()?, r.f32()?];
            PlayIn::Move { pos: Some(pos), rot: Some(rot), on_ground: r.u8()? & 1 != 0 }
        }
        sb::MOVE_PLAYER_ROT => {
            let rot = [r.f32()?, r.f32()?];
            PlayIn::Move { pos: None, rot: Some(rot), on_ground: r.u8()? & 1 != 0 }
        }
        sb::MOVE_PLAYER_STATUS_ONLY => PlayIn::Move { pos: None, rot: None, on_ground: r.u8()? & 1 != 0 },
        sb::CHUNK_BATCH_RECEIVED => PlayIn::ChunkBatchReceived { chunks_per_tick: r.f32()? },
        sb::CLIENT_INFORMATION => {
            let info = read_client_information(r)?;
            PlayIn::ClientInformation { view_distance: info }
        }
        sb::CHAT => {
            let message = r.string(256)?.to_owned();
            r.rest(); // timestamp, salt, signature, acknowledgements: unsigned chat for now
            PlayIn::Chat { message }
        }
        sb::PLAYER_LOADED => PlayIn::PlayerLoaded,
        sb::PLAYER_ACTION => PlayIn::PlayerAction {
            action: r.varint()?,
            pos: read_position(r)?,
            face: r.u8()?,
            sequence: r.varint()?,
        },
        sb::USE_ITEM_ON => PlayIn::UseItemOn {
            hand: r.varint()?,
            pos: read_position(r)?,
            face: r.varint()?,
            cursor: [r.f32()?, r.f32()?, r.f32()?],
            inside: r.bool()?,
            sequence: {
                let _world_border_hit = r.bool()?;
                r.varint()?
            },
        },
        sb::SET_CARRIED_ITEM => PlayIn::SetCarriedItem { slot: r.i16()? },
        sb::SET_CREATIVE_MODE_SLOT => PlayIn::SetCreativeSlot { slot: r.i16()?, item: read_untrusted_slot(r)? },
        sb::PUNCH => PlayIn::Punch,
        _ => {
            r.rest();
            return Ok(None);
        }
    };
    r.finish()?;
    Ok(Some(pkt))
}

/// Client Information (configuration and play); returns the view distance.
pub fn read_client_information(r: &mut Reader) -> Result<u8, DecodeError> {
    let _locale = r.string(16)?;
    let view_distance = r.i8()?.max(2) as u8;
    let _chat_mode = r.varint()?;
    let _chat_colors = r.bool()?;
    let _skin_parts = r.u8()?;
    let _main_hand = r.varint()?;
    let _text_filtering = r.bool()?;
    let _allow_listing = r.bool()?;
    let _particles = r.varint()?;
    Ok(view_distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_roundtrip_including_negatives() {
        for pos in [[18357644, 831, -20882616], [-1, -64, -1], [0, 0, 0], [-33554432, -2048, 33554431]] {
            let mut b = BytesMut::new();
            b.put_position(pos[0], pos[1], pos[2]);
            assert_eq!(read_position(&mut Reader::new(&b)).unwrap(), pos);
        }
    }

    #[test]
    fn untrusted_slot_skips_delimited_components() {
        let mut b = BytesMut::new();
        b.put_varint(3); // count
        b.put_varint(42); // item
        b.put_varint(1); // one added component
        b.put_varint(1); // one removed component
        b.put_varint(7);
        b.put_varint(2);
        b.put_slice(&[0xAA, 0xBB]);
        b.put_varint(9);
        let mut r = Reader::new(&b);
        let s = read_untrusted_slot(&mut r).unwrap().unwrap();
        r.finish().unwrap();
        assert_eq!((s.item, s.count, s.removed.as_slice()), (42, 3, &[9][..]));
        assert_eq!(&s.added[0].1[..], &[0xAA, 0xBB]);
        assert_eq!(read_untrusted_slot(&mut Reader::new(&[0])).unwrap(), None);
    }
}
