//! Packets shared by the configuration and play states (and login, for cookies): resource
//! packs, transfers, cookies, server links, crash report details, dialogs, pings, plus the
//! configuration-only code of conduct. Layouts follow the 26.3 bytecode
//! (`net.minecraft.network.protocol.{common,cookie,configuration}`); the same codec is used in
//! every state except `show_dialog` (see [`show_dialog_play`], [`show_dialog_configuration`]).

use super::packet;
use crate::nbt::{self, Tag};
use crate::{DecodeError, Reader, WriteExt};
use bytes::{BufMut, Bytes};
use kiln_data::packets as ids;
use uuid::Uuid;

/// The states the common packets exist in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Configuration,
    Play,
}

impl Phase {
    fn id(self, configuration: i32, play: i32) -> i32 {
        match self {
            Phase::Configuration => configuration,
            Phase::Play => play,
        }
    }
}

/// Cookies also exist in login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookiePhase {
    Login,
    Configuration,
    Play,
}

impl From<Phase> for CookiePhase {
    fn from(p: Phase) -> Self {
        match p {
            Phase::Configuration => CookiePhase::Configuration,
            Phase::Play => CookiePhase::Play,
        }
    }
}

// ---- resource packs -----------------------------------------------------------------------

pub struct ResourcePack<'a> {
    pub id: Uuid,
    pub url: &'a str,
    /// Lowercase hex SHA-1 of the pack (at most 40 characters; may be empty).
    pub hash: &'a str,
    /// The client disconnects if the player declines a required pack.
    pub required: bool,
    pub prompt: Option<&'a Tag>,
}

pub fn resource_pack_push(phase: Phase, pack: &ResourcePack) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    debug_assert!(pack.hash.len() <= 40, "resource pack hash longer than 40 characters");
    let mut b = packet(phase.id(c::RESOURCE_PACK_PUSH, p::RESOURCE_PACK_PUSH));
    b.put_uuid(pack.id);
    b.put_string(pack.url);
    b.put_string(pack.hash);
    b.put_bool(pack.required);
    b.put_bool(pack.prompt.is_some());
    if let Some(prompt) = pack.prompt {
        prompt.write_network(&mut b);
    }
    b.freeze()
}

/// Unloads one pack, or every server pack with `None`.
pub fn resource_pack_pop(phase: Phase, id: Option<Uuid>) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    let mut b = packet(phase.id(c::RESOURCE_PACK_POP, p::RESOURCE_PACK_POP));
    b.put_bool(id.is_some());
    if let Some(id) = id {
        b.put_uuid(id);
    }
    b.freeze()
}

/// `ServerboundResourcePackPacket.Action`: the client reports each pack's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePackAction {
    SuccessfullyLoaded,
    Declined,
    FailedDownload,
    Accepted,
    Downloaded,
    InvalidUrl,
    FailedReload,
    Discarded,
}

impl ResourcePackAction {
    const ALL: [Self; 8] = [
        Self::SuccessfullyLoaded,
        Self::Declined,
        Self::FailedDownload,
        Self::Accepted,
        Self::Downloaded,
        Self::InvalidUrl,
        Self::FailedReload,
        Self::Discarded,
    ];

    /// Whether this is the pack's last report (anything but accepted/downloaded).
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Accepted | Self::Downloaded)
    }
}

/// Serverbound `resource_pack` (configuration and play).
pub fn read_resource_pack_response(r: &mut Reader) -> Result<(Uuid, ResourcePackAction), DecodeError> {
    let id = r.uuid()?;
    let action = read_enum(r, &ResourcePackAction::ALL, "resource pack action")?;
    Ok((id, action))
}

/// A VarInt ordinal that must be in range (`FriendlyByteBuf.readEnum`).
pub(crate) fn read_enum<T: Copy>(r: &mut Reader, all: &[T], what: &'static str) -> Result<T, DecodeError> {
    let i = r.varint()?;
    usize::try_from(i).ok().and_then(|i| all.get(i).copied()).ok_or(DecodeError::Invalid(what))
}

// ---- transfer and cookies -----------------------------------------------------------------

/// Sends the client to another server; it reconnects with the transfer intent.
pub fn transfer(phase: Phase, host: &str, port: i32) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    let mut b = packet(phase.id(c::TRANSFER, p::TRANSFER));
    b.put_string(host);
    b.put_varint(port);
    b.freeze()
}

/// Largest cookie payload vanilla accepts in either direction.
pub const MAX_COOKIE_LEN: usize = 5120;

/// Asks the client for the cookie stored under `key` (an identifier).
pub fn cookie_request(phase: CookiePhase, key: &str) -> Bytes {
    let id = match phase {
        CookiePhase::Login => ids::login::clientbound::COOKIE_REQUEST,
        CookiePhase::Configuration => ids::configuration::clientbound::COOKIE_REQUEST,
        CookiePhase::Play => ids::play::clientbound::COOKIE_REQUEST,
    };
    let mut b = packet(id);
    b.put_string(key);
    b.freeze()
}

/// Stores a cookie on the client (kept across transfers, not across restarts).
pub fn store_cookie(phase: Phase, key: &str, payload: &[u8]) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    debug_assert!(payload.len() <= MAX_COOKIE_LEN, "cookie payload over {MAX_COOKIE_LEN} bytes");
    let mut b = packet(phase.id(c::STORE_COOKIE, p::STORE_COOKIE));
    b.put_string(key);
    b.put_varint(payload.len() as i32);
    b.put_slice(payload);
    b.freeze()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieResponse {
    pub key: String,
    /// `None` when the client has no cookie under this key.
    pub payload: Option<Vec<u8>>,
}

/// Serverbound `cookie_response` (login, configuration and play).
pub fn read_cookie_response(r: &mut Reader) -> Result<CookieResponse, DecodeError> {
    let key = read_identifier(r)?;
    let payload = if r.bool()? {
        let len = r.len()?;
        if len > MAX_COOKIE_LEN {
            return Err(DecodeError::Invalid("cookie payload too large"));
        }
        Some(r.bytes(len)?.to_vec())
    } else {
        None
    };
    Ok(CookieResponse { key, payload })
}

/// An `Identifier` (`namespace:path`, namespace defaulting to `minecraft`), validated as vanilla does.
pub fn read_identifier(r: &mut Reader) -> Result<String, DecodeError> {
    let s = r.string(32767)?;
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    let ns_ok = ns.bytes().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'));
    let path_ok = path.bytes().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'));
    if !(ns_ok && path_ok) {
        return Err(DecodeError::Invalid("identifier"));
    }
    Ok(s.to_owned())
}

// ---- server links, report details, code of conduct ----------------------------------------

/// `ServerLinks.KnownLinkType`: links the client labels itself (localized).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownLink {
    BugReport,
    CommunityGuidelines,
    Support,
    Status,
    Feedback,
    Community,
    Website,
    Forums,
    News,
    Announcements,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkLabel<'a> {
    Known(KnownLink),
    Custom(&'a Tag),
}

/// Links shown in the pause menu (and on the disconnect screen for [`KnownLink::BugReport`]).
pub fn server_links(phase: Phase, links: &[(LinkLabel, &str)]) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    let mut b = packet(phase.id(c::SERVER_LINKS, p::SERVER_LINKS));
    b.put_varint(links.len() as i32);
    for (label, url) in links {
        match label {
            LinkLabel::Known(k) => {
                b.put_bool(true);
                b.put_varint(*k as i32);
            }
            LinkLabel::Custom(text) => {
                b.put_bool(false);
                text.write_network(&mut b);
            }
        }
        b.put_string(url);
    }
    b.freeze()
}

/// Vanilla's limits for [`custom_report_details`].
pub const MAX_REPORT_DETAILS: usize = 32;

/// Extra sections for the client's crash report: at most 32 entries, keys up to 128
/// characters, values up to 4096.
pub fn custom_report_details(phase: Phase, details: &[(&str, &str)]) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    debug_assert!(details.len() <= MAX_REPORT_DETAILS);
    let mut b = packet(phase.id(c::CUSTOM_REPORT_DETAILS, p::CUSTOM_REPORT_DETAILS));
    b.put_varint(details.len() as i32);
    for (key, value) in details {
        b.put_string(key);
        b.put_string(value);
    }
    b.freeze()
}

/// The server's code of conduct, shown before joining (configuration only); the client answers
/// with `accept_code_of_conduct` (no fields).
pub fn code_of_conduct(text: &str) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::CODE_OF_CONDUCT);
    b.put_string(text);
    b.freeze()
}

// ---- dialogs ------------------------------------------------------------------------------

/// A dialog: an entry of the `minecraft:dialog` registry or a definition sent inline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dialog<'a> {
    /// Registry id, in the order the registry was sent during configuration.
    Registered(i32),
    /// A dialog definition as NBT: a compound with `type` (e.g. `"minecraft:notice"`) and the
    /// type's fields, as in a data pack's `dialog/*.json`.
    Inline(&'a Tag),
}

/// Play `show_dialog`: a registry holder (id + 1, or 0 and the inline definition).
pub fn show_dialog_play(dialog: &Dialog) -> Bytes {
    let mut b = packet(ids::play::clientbound::SHOW_DIALOG);
    match dialog {
        Dialog::Registered(id) => b.put_varint(id + 1),
        Dialog::Inline(def) => {
            b.put_varint(0);
            def.write_network(&mut b);
        }
    }
    b.freeze()
}

/// Configuration `show_dialog`: registries are not usable yet, so the definition is always
/// inline (and cannot refer to other registry entries).
pub fn show_dialog_configuration(definition: &Tag) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::SHOW_DIALOG);
    definition.write_network(&mut b);
    b.freeze()
}

pub fn clear_dialog(phase: Phase) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    packet(phase.id(c::CLEAR_DIALOG, p::CLEAR_DIALOG)).freeze()
}

/// Largest `custom_click_action` payload vanilla accepts.
pub const MAX_CLICK_PAYLOAD: usize = 65536;

/// Serverbound `custom_click_action` (configuration and play): a dialog or chat click with a
/// `minecraft:custom` action. The payload is a length-prefixed NBT tag; an End tag means none.
pub fn read_custom_click_action(r: &mut Reader) -> Result<(String, Option<Tag>), DecodeError> {
    let id = read_identifier(r)?;
    let len = r.len()?;
    if len > MAX_CLICK_PAYLOAD {
        return Err(DecodeError::Invalid("click action payload too large"));
    }
    let data = r.bytes(len)?;
    let payload = match data.first() {
        None => return Err(DecodeError::Eof),
        Some(0) => None,
        Some(_) => Some(nbt::read_network(data).map_err(|_| DecodeError::Invalid("click action payload"))?.0),
    };
    Ok((id, payload))
}

// ---- pings --------------------------------------------------------------------------------

/// The client answers with `pong` carrying the same id.
pub fn ping(phase: Phase, id: i32) -> Bytes {
    use ids::{configuration::clientbound as c, play::clientbound as p};
    let mut b = packet(phase.id(c::PING, p::PING));
    b.put_i32(id);
    b.freeze()
}

/// Configuration keep-alive (play has [`super::keep_alive`]).
pub fn keep_alive_configuration(id: i64) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::KEEP_ALIVE);
    b.put_i64(id);
    b.freeze()
}

/// Answer to the client's `ping_request` (the network debug screen); echoes its time.
pub fn pong_response(time: i64) -> Bytes {
    let mut b = packet(ids::play::clientbound::PONG_RESPONSE);
    b.put_i64(time);
    b.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_validated() {
        let mut b = bytes::BytesMut::new();
        for s in ["minecraft:a/b", "kiln:x.y-z_0", "plain"] {
            b.clear();
            b.put_string(s);
            assert_eq!(read_identifier(&mut Reader::new(&b)).unwrap(), s);
        }
        for s in ["Minecraft:a", "a:b:c", "a:B", "sp ace"] {
            b.clear();
            b.put_string(s);
            assert!(read_identifier(&mut Reader::new(&b)).is_err(), "{s}");
        }
    }

    #[test]
    fn resource_pack_actions_are_bounded() {
        let mut b = bytes::BytesMut::new();
        b.put_uuid(Uuid::nil());
        b.put_varint(8);
        assert!(read_resource_pack_response(&mut Reader::new(&b)).is_err());
    }
}
