//! Player list packets: `player_info_update` (tab list entries and the profiles the client
//! needs before it can spawn a player entity) and `player_info_remove`.

use super::super::{ProfileProperty, packet};
use crate::WriteExt;
use crate::nbt::Tag;
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets::play::clientbound as ids;
use std::ops::BitOr;
use uuid::Uuid;

/// `ClientboundPlayerInfoUpdatePacket.Action` set, written as a fixed 8-bit set (bit = ordinal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerInfoActions(u8);

impl PlayerInfoActions {
    /// Profile name and properties (skin); creates the entry.
    pub const ADD_PLAYER: Self = Self(1 << 0);
    pub const INITIALIZE_CHAT: Self = Self(1 << 1);
    pub const UPDATE_GAME_MODE: Self = Self(1 << 2);
    /// Whether the entry is shown in the tab list.
    pub const UPDATE_LISTED: Self = Self(1 << 3);
    pub const UPDATE_LATENCY: Self = Self(1 << 4);
    pub const UPDATE_DISPLAY_NAME: Self = Self(1 << 5);
    pub const UPDATE_LIST_ORDER: Self = Self(1 << 6);
    pub const UPDATE_HAT: Self = Self(1 << 7);
    /// What vanilla sends for a joining player (`createPlayerInitializing`).
    pub const INITIALIZE: Self = Self(0xff);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for PlayerInfoActions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// A signed chat session (`RemoteChatSession.Data`).
#[derive(Debug, Clone, Copy)]
pub struct ChatSession<'a> {
    pub session_id: Uuid,
    /// Key expiry, epoch milliseconds.
    pub expires_at: i64,
    /// X.509-encoded RSA public key (at most 512 bytes).
    pub public_key: &'a [u8],
    /// Mojang's signature of the key (at most 4096 bytes).
    pub key_signature: &'a [u8],
}

/// One player's entry. Only the fields selected by the packet's actions are written.
#[derive(Clone, Copy)]
pub struct PlayerInfoEntry<'a> {
    pub uuid: Uuid,
    /// At most 16 characters.
    pub name: &'a str,
    /// At most 16 properties, e.g. `textures`.
    pub properties: &'a [ProfileProperty<'a>],
    pub chat_session: Option<ChatSession<'a>>,
    /// `GameType` id: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    pub listed: bool,
    /// Milliseconds.
    pub latency: i32,
    pub display_name: Option<&'a Tag>,
    pub list_order: i32,
    pub show_hat: bool,
}

impl<'a> PlayerInfoEntry<'a> {
    /// An entry with vanilla's defaults for a fresh player: listed, no chat session.
    pub fn new(uuid: Uuid, name: &'a str, properties: &'a [ProfileProperty<'a>], game_mode: i32) -> Self {
        Self {
            uuid,
            name,
            properties,
            chat_session: None,
            game_mode,
            listed: true,
            latency: 0,
            display_name: None,
            list_order: 0,
            show_hat: true,
        }
    }
}

pub fn player_info_update(actions: PlayerInfoActions, entries: &[PlayerInfoEntry]) -> Bytes {
    let mut b = packet(ids::PLAYER_INFO_UPDATE);
    b.put_u8(actions.0);
    b.put_varint(entries.len() as i32);
    for e in entries {
        b.put_uuid(e.uuid);
        if actions.contains(PlayerInfoActions::ADD_PLAYER) {
            b.put_string(e.name);
            put_properties(&mut b, e.properties);
        }
        if actions.contains(PlayerInfoActions::INITIALIZE_CHAT) {
            b.put_bool(e.chat_session.is_some());
            if let Some(c) = &e.chat_session {
                b.put_uuid(c.session_id);
                b.put_i64(c.expires_at);
                b.put_varint(c.public_key.len() as i32);
                b.put_slice(c.public_key);
                b.put_varint(c.key_signature.len() as i32);
                b.put_slice(c.key_signature);
            }
        }
        if actions.contains(PlayerInfoActions::UPDATE_GAME_MODE) {
            b.put_varint(e.game_mode);
        }
        if actions.contains(PlayerInfoActions::UPDATE_LISTED) {
            b.put_bool(e.listed);
        }
        if actions.contains(PlayerInfoActions::UPDATE_LATENCY) {
            b.put_varint(e.latency);
        }
        if actions.contains(PlayerInfoActions::UPDATE_DISPLAY_NAME) {
            b.put_bool(e.display_name.is_some());
            if let Some(name) = e.display_name {
                name.write_network(&mut b);
            }
        }
        if actions.contains(PlayerInfoActions::UPDATE_LIST_ORDER) {
            b.put_varint(e.list_order);
        }
        if actions.contains(PlayerInfoActions::UPDATE_HAT) {
            b.put_bool(e.show_hat);
        }
    }
    b.freeze()
}

/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`.
fn put_properties(b: &mut BytesMut, properties: &[ProfileProperty]) {
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

pub fn player_info_remove(uuids: &[Uuid]) -> Bytes {
    let mut b = packet(ids::PLAYER_INFO_REMOVE);
    b.put_varint(uuids.len() as i32);
    for u in uuids {
        b.put_uuid(*u);
    }
    b.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Reader;

    #[test]
    fn actions_are_written_in_ordinal_order() {
        let uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let props = [ProfileProperty { name: "textures", value: "e30=", signature: None }];
        let entry = PlayerInfoEntry { latency: 42, ..PlayerInfoEntry::new(uuid, "Steve", &props, 1) };
        let actions = PlayerInfoActions::UPDATE_LATENCY | PlayerInfoActions::ADD_PLAYER;
        let p = player_info_update(actions, &[entry]);
        let mut r = Reader::new(&p);
        assert_eq!(r.varint().unwrap(), ids::PLAYER_INFO_UPDATE);
        assert_eq!(r.u8().unwrap(), 0b1_0001);
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.uuid().unwrap(), uuid);
        assert_eq!(r.string(16).unwrap(), "Steve");
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!((r.string(64).unwrap(), r.string(32767).unwrap(), r.bool().unwrap()), ("textures", "e30=", false));
        assert_eq!(r.varint().unwrap(), 42);
        r.finish().unwrap();
    }

    #[test]
    fn remove_lists_uuids() {
        let p = player_info_remove(&[Uuid::nil(), Uuid::from_u128(u128::MAX)]);
        assert_eq!(p.len(), 1 + 1 + 32);
        assert_eq!(p[1], 2);
    }
}
