//! The client side of the 26.3 protocol that bots speak: serverbound packet builders
//! (packet id + data) and decoders for the few clientbound packets they read.
//! Layouts are taken from the 26.3 server jar's stream codecs.

use crate::text;
use bytes::{BufMut, BytesMut};
use kiln_data::packets as ids;
use kiln_data::version;
use kiln_proto::{DecodeError, Reader, WriteExt};
use uuid::Uuid;

const INTENT_LOGIN: i32 = 2;

// ---- handshake and login ------------------------------------------------------------------

pub fn intention(b: &mut BytesMut, host: &str, port: u16) {
    b.put_varint(ids::handshake::serverbound::INTENTION);
    b.put_varint(version::PROTOCOL);
    b.put_string(host);
    b.put_u16(port);
    b.put_varint(INTENT_LOGIN);
}

pub fn hello(b: &mut BytesMut, name: &str, uuid: Uuid) {
    b.put_varint(ids::login::serverbound::HELLO);
    b.put_string(name);
    b.put_uuid(uuid);
}

pub fn login_acknowledged(b: &mut BytesMut) {
    b.put_varint(ids::login::serverbound::LOGIN_ACKNOWLEDGED);
}

/// Answers a login plugin request with "not understood" (no payload).
pub fn custom_query_answer(b: &mut BytesMut, transaction: i32) {
    b.put_varint(ids::login::serverbound::CUSTOM_QUERY_ANSWER);
    b.put_varint(transaction);
    b.put_bool(false);
}

/// Answers a cookie request with "no cookie"; `packet_id` differs per state.
pub fn cookie_response(b: &mut BytesMut, packet_id: i32, key: &str) {
    b.put_varint(packet_id);
    b.put_string(key);
    b.put_bool(false);
}

// ---- configuration ------------------------------------------------------------------------

/// Client Information as the vanilla client sends it by default, apart from the view distance.
pub fn client_information(b: &mut BytesMut, packet_id: i32, view_distance: u8) {
    b.put_varint(packet_id);
    b.put_string("en_us");
    b.put_u8(view_distance);
    b.put_varint(0); // chat visibility: full
    b.put_bool(true); // chat colors
    b.put_u8(0x7f); // all skin parts
    b.put_varint(1); // main hand: right
    b.put_bool(false); // text filtering
    b.put_bool(true); // allow server listing
    b.put_varint(0); // particles: all
}

pub fn brand(b: &mut BytesMut, packet_id: i32, brand: &str) {
    b.put_varint(packet_id);
    b.put_string("minecraft:brand");
    b.put_string(brand);
}

/// Select Known Packs reply. The serverbound list has the clientbound layout, so a client
/// that knows everything the server offered echoes the offer back unchanged.
pub fn select_known_packs(b: &mut BytesMut, offered: &[u8]) {
    b.put_varint(ids::configuration::serverbound::SELECT_KNOWN_PACKS);
    b.put_slice(offered);
}

pub fn finish_configuration(b: &mut BytesMut) {
    b.put_varint(ids::configuration::serverbound::FINISH_CONFIGURATION);
}

pub fn accept_code_of_conduct(b: &mut BytesMut) {
    b.put_varint(ids::configuration::serverbound::ACCEPT_CODE_OF_CONDUCT);
}

/// Keep Alive reply; `packet_id` differs between configuration and play.
pub fn keep_alive(b: &mut BytesMut, packet_id: i32, id: i64) {
    b.put_varint(packet_id);
    b.put_i64(id);
}

/// Pong reply to a Ping; `packet_id` differs between configuration and play.
pub fn pong(b: &mut BytesMut, packet_id: i32, id: i32) {
    b.put_varint(packet_id);
    b.put_i32(id);
}

// ---- play ---------------------------------------------------------------------------------

/// Accept Teleportation echoes the resulting absolute position and rotation since 26.3;
/// the server treats them as the client's first move after the teleport.
pub fn accept_teleportation(b: &mut BytesMut, id: i32, pos: [f64; 3], yaw: f32, pitch: f32) {
    b.put_varint(ids::play::serverbound::ACCEPT_TELEPORTATION);
    b.put_varint(id);
    for v in pos {
        b.put_f64(v);
    }
    b.put_f32(yaw);
    b.put_f32(pitch);
}

/// A Move Player packet; which variant is sent follows the vanilla client's rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Move {
    Pos([f64; 3]),
    PosRot([f64; 3], f32, f32),
    Rot(f32, f32),
    StatusOnly,
}

const FLAG_ON_GROUND: u8 = 1;

pub fn move_player(b: &mut BytesMut, m: Move, on_ground: bool) {
    use ids::play::serverbound as sb;
    let flags = if on_ground { FLAG_ON_GROUND } else { 0 };
    match m {
        Move::Pos(pos) => {
            b.put_varint(sb::MOVE_PLAYER_POS);
            pos.iter().for_each(|v| b.put_f64(*v));
        }
        Move::PosRot(pos, yaw, pitch) => {
            b.put_varint(sb::MOVE_PLAYER_POS_ROT);
            pos.iter().for_each(|v| b.put_f64(*v));
            b.put_f32(yaw);
            b.put_f32(pitch);
        }
        Move::Rot(yaw, pitch) => {
            b.put_varint(sb::MOVE_PLAYER_ROT);
            b.put_f32(yaw);
            b.put_f32(pitch);
        }
        Move::StatusOnly => b.put_varint(sb::MOVE_PLAYER_STATUS_ONLY),
    }
    b.put_u8(flags);
}

pub fn client_tick_end(b: &mut BytesMut) {
    b.put_varint(ids::play::serverbound::CLIENT_TICK_END);
}

pub fn chunk_batch_received(b: &mut BytesMut, chunks_per_tick: f32) {
    b.put_varint(ids::play::serverbound::CHUNK_BATCH_RECEIVED);
    b.put_f32(chunks_per_tick);
}

pub fn player_loaded(b: &mut BytesMut) {
    b.put_varint(ids::play::serverbound::PLAYER_LOADED);
}

pub fn configuration_acknowledged(b: &mut BytesMut) {
    b.put_varint(ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED);
}

/// A command typed in chat, without the leading slash.
pub fn chat_command(b: &mut BytesMut, command: &str) {
    b.put_varint(ids::play::serverbound::CHAT_COMMAND);
    b.put_string(command);
}

/// An unsigned chat message with nothing acknowledged (offline mode, no chat session).
pub fn chat(b: &mut BytesMut, message: &str, timestamp_ms: i64, salt: i64) {
    b.put_varint(ids::play::serverbound::CHAT);
    b.put_string(message);
    b.put_i64(timestamp_ms);
    b.put_i64(salt);
    b.put_bool(false); // no signature
    b.put_varint(0); // last seen: offset
    b.put_slice(&[0; 3]); // last seen: acknowledged, a fixed 20-bit set
    b.put_u8(0); // last seen: checksum (0 = not checked)
}

// ---- clientbound --------------------------------------------------------------------------

/// Relative-flag bits of Player Position (`Relative` ordinals).
pub mod relative {
    pub const X: u32 = 1 << 0;
    pub const Y: u32 = 1 << 1;
    pub const Z: u32 = 1 << 2;
    pub const YAW: u32 = 1 << 3;
    pub const PITCH: u32 = 1 << 4;
}

/// Clientbound Player Position (a teleport).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Teleport {
    pub id: i32,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub relative: u32,
}

impl Teleport {
    pub fn read(r: &mut Reader) -> Result<Self, DecodeError> {
        let id = r.varint()?;
        let pos = [r.f64()?, r.f64()?, r.f64()?];
        let _velocity = [r.f64()?, r.f64()?, r.f64()?];
        let yaw = r.f32()?;
        let pitch = r.f32()?;
        let relative = r.i32()? as u32;
        r.finish()?;
        Ok(Self { id, pos, yaw, pitch, relative })
    }

    /// The absolute position and rotation after applying this teleport to the current ones.
    pub fn apply(&self, pos: [f64; 3], yaw: f32, pitch: f32) -> ([f64; 3], f32, f32) {
        let rel = |bit: u32| self.relative & bit != 0;
        let mut out = self.pos;
        for (i, bit) in [relative::X, relative::Y, relative::Z].into_iter().enumerate() {
            if rel(bit) {
                out[i] += pos[i];
            }
        }
        let yaw = if rel(relative::YAW) { yaw + self.yaw } else { self.yaw };
        let pitch = if rel(relative::PITCH) { pitch + self.pitch } else { self.pitch };
        (out, yaw, pitch.clamp(-90.0, 90.0))
    }
}

/// Login Disconnect carries the reason as a JSON text component.
pub fn read_login_disconnect(r: &mut Reader) -> Result<String, DecodeError> {
    Ok(text::from_json(r.string(262_144)?))
}

/// Configuration and play Disconnect carry the reason as an NBT text component.
pub fn read_disconnect(r: &mut Reader) -> Result<String, DecodeError> {
    text::from_nbt(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_proto::packets::{self as server, PlayIn};

    fn built(f: impl FnOnce(&mut BytesMut)) -> BytesMut {
        let mut b = BytesMut::new();
        f(&mut b);
        b
    }

    /// Decodes with the server's own play decoder.
    fn server_decode(b: &[u8]) -> PlayIn {
        let mut r = Reader::new(b);
        let id = r.varint().unwrap();
        server::decode_play(id, &mut r).unwrap().expect("server ignored the packet")
    }

    #[test]
    fn server_reads_play_packets() {
        let b = built(|b| accept_teleportation(b, 7, [1.0, 2.0, 3.0], 90.0, 10.0));
        assert!(matches!(server_decode(&b), PlayIn::AcceptTeleport { id: 7 }));

        let b = built(|b| move_player(b, Move::PosRot([1.5, -60.0, 2.5], 45.0, 0.0), true));
        match server_decode(&b) {
            PlayIn::Move { pos, rot, on_ground } => {
                assert_eq!(pos, Some([1.5, -60.0, 2.5]));
                assert_eq!(rot, Some([45.0, 0.0]));
                assert!(on_ground);
            }
            other => panic!("{other:?}"),
        }
        let b = built(|b| move_player(b, Move::Pos([1.0, 2.0, 3.0]), false));
        assert!(matches!(server_decode(&b), PlayIn::Move { pos: Some(_), rot: None, on_ground: false }));
        let b = built(|b| move_player(b, Move::Rot(1.0, 2.0), true));
        assert!(matches!(server_decode(&b), PlayIn::Move { pos: None, rot: Some(_), on_ground: true }));
        let b = built(|b| move_player(b, Move::StatusOnly, true));
        assert!(matches!(server_decode(&b), PlayIn::Move { pos: None, rot: None, on_ground: true }));

        let b = built(|b| keep_alive(b, ids::play::serverbound::KEEP_ALIVE, -5));
        assert!(matches!(server_decode(&b), PlayIn::KeepAlive { id: -5 }));
        let b = built(|b| chunk_batch_received(b, 64.0));
        assert!(matches!(server_decode(&b), PlayIn::ChunkBatchReceived { chunks_per_tick: 64.0 }));
        assert!(matches!(server_decode(&built(player_loaded)), PlayIn::PlayerLoaded));
        let b = built(|b| chat(b, "hello", 1_700_000_000_000, 42));
        assert!(matches!(server_decode(&b), PlayIn::Chat { message } if message == "hello"));
        let b = built(|b| client_information(b, ids::play::serverbound::CLIENT_INFORMATION, 5));
        assert!(matches!(server_decode(&b), PlayIn::ClientInformation(i) if i.view_distance == 5));
    }

    #[test]
    fn server_reads_configuration_client_information() {
        let b = built(|b| client_information(b, ids::configuration::serverbound::CLIENT_INFORMATION, 2));
        let mut r = Reader::new(&b);
        assert_eq!(r.varint().unwrap(), ids::configuration::serverbound::CLIENT_INFORMATION);
        assert_eq!(server::read_client_information(&mut r).unwrap().view_distance, 2);
        r.finish().unwrap();
    }

    #[test]
    fn reads_server_player_position() {
        let p = server::player_position(3, [8.5, -60.0, 8.5], 12.0, -5.0);
        let mut r = Reader::new(&p);
        assert_eq!(r.varint().unwrap(), ids::play::clientbound::PLAYER_POSITION);
        let t = Teleport::read(&mut r).unwrap();
        assert_eq!(t, Teleport { id: 3, pos: [8.5, -60.0, 8.5], yaw: 12.0, pitch: -5.0, relative: 0 });
        assert_eq!(t.apply([100.0, 0.0, 100.0], 1.0, 2.0), ([8.5, -60.0, 8.5], 12.0, -5.0));
    }

    #[test]
    fn relative_teleport_adds_to_current_values() {
        let t = Teleport {
            id: 1,
            pos: [1.0, 0.0, -1.0],
            yaw: 10.0,
            pitch: 80.0,
            relative: relative::X | relative::Z | relative::YAW | relative::PITCH,
        };
        assert_eq!(t.apply([5.0, 64.0, 5.0], 20.0, 30.0), ([6.0, 0.0, 4.0], 30.0, 90.0));
    }

    #[test]
    fn reads_server_disconnect_reasons() {
        let p = server::login_disconnect("The server is full.");
        let mut r = Reader::new(&p[1..]);
        assert_eq!(read_login_disconnect(&mut r).unwrap(), "The server is full.");
        let p = server::play_disconnect("Timed out");
        let mut r = Reader::new(&p[1..]);
        assert_eq!(read_disconnect(&mut r).unwrap(), "Timed out");
        r.finish().unwrap();
    }
}
