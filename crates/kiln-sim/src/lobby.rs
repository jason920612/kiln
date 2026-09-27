//! Per-player protocol features of the play state: resource packs pushed after joining,
//! cookies, transfers and dialogs (`ServerCommonPacketListenerImpl`).

use crate::Sim;
use kiln_link::ConnId;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::common::{self, CookieResponse, Dialog, Phase, ResourcePack, ResourcePackAction};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

/// What a player's client reported or answered.
#[derive(Debug, Default)]
pub(crate) struct PlayerLobby {
    /// Cookie keys asked for and not answered yet.
    cookie_requests: Vec<String>,
    /// Answers by key: the payload, or `None` when the client had no such cookie.
    cookies: HashMap<String, Option<Vec<u8>>>,
    /// The last status of each resource pack the client reported.
    packs: HashMap<Uuid, ResourcePackAction>,
}

/// A resource pack to push to players.
pub struct PackOffer<'a> {
    pub id: Uuid,
    pub url: &'a str,
    pub hash: &'a str,
    pub required: bool,
    pub prompt: Option<&'a Tag>,
}

impl Sim {
    /// `ServerCommonPacketListenerImpl.handleResourcePackResponse`: a declined pack disconnects
    /// the player when the server requires its resource pack.
    pub(crate) fn resource_pack_response(&mut self, conn: ConnId, id: Uuid, action: ResourcePackAction) {
        let required = self.config.require_resource_pack;
        let Some(p) = self.players.get_mut(&conn) else { return };
        p.lobby.packs.insert(id, action);
        if action == ResourcePackAction::Declined && required {
            info!("Disconnecting {} due to resource pack {id} rejection", p.name);
            p.disconnect_text(translate("multiplayer.requiredTexturePrompt.disconnect"));
        }
    }

    /// An answer to [`request_cookie`](Self::request_cookie); unrequested answers disconnect
    /// the player, as in vanilla.
    pub(crate) fn cookie_response(&mut self, conn: ConnId, response: CookieResponse) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        match p.lobby.cookie_requests.iter().position(|k| *k == response.key) {
            Some(i) => {
                p.lobby.cookie_requests.remove(i);
                p.lobby.cookies.insert(response.key, response.payload);
            }
            None => p.disconnect_text(translate("multiplayer.disconnect.unexpected_query_response")),
        }
    }

    /// Pushes a resource pack to a player in play (`ClientboundResourcePackPushPacket`).
    pub fn push_resource_pack(&mut self, conn: ConnId, pack: &PackOffer) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        let pack = ResourcePack { id: pack.id, url: pack.url, hash: pack.hash, required: pack.required, prompt: pack.prompt };
        p.send(common::resource_pack_push(Phase::Play, &pack));
    }

    /// Unloads one server pack from a player's client, or all of them.
    pub fn pop_resource_pack(&mut self, conn: ConnId, id: Option<Uuid>) {
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(common::resource_pack_pop(Phase::Play, id));
        }
    }

    /// The last status a player's client reported for a resource pack.
    pub fn resource_pack_status(&self, conn: ConnId, id: Uuid) -> Option<ResourcePackAction> {
        self.players.get(&conn)?.lobby.packs.get(&id).copied()
    }

    /// Stores a cookie on a player's client (kept across transfers).
    pub fn store_cookie(&mut self, conn: ConnId, key: &str, payload: &[u8]) {
        if let Some(p) = self.players.get_mut(&conn)
            && payload.len() <= common::MAX_COOKIE_LEN
        {
            p.send(common::store_cookie(Phase::Play, key, payload));
        }
    }

    /// Asks a player's client for a cookie; the answer shows up in [`cookie`](Self::cookie).
    pub fn request_cookie(&mut self, conn: ConnId, key: &str) {
        if let Some(p) = self.players.get_mut(&conn) {
            p.lobby.cookie_requests.push(key.to_owned());
            p.lobby.cookies.remove(key);
            p.send(common::cookie_request(common::CookiePhase::Play, key));
        }
    }

    /// The answer to the last [`request_cookie`](Self::request_cookie) for `key`: `None` while
    /// unanswered, `Some(None)` when the client has no such cookie.
    pub fn cookie(&self, conn: ConnId, key: &str) -> Option<Option<Vec<u8>>> {
        self.players.get(&conn)?.lobby.cookies.get(key).cloned()
    }

    /// Sends a player to another server (`ClientboundTransferPacket`).
    pub fn transfer(&mut self, conn: ConnId, host: &str, port: i32) {
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(common::transfer(Phase::Play, host, port));
        }
    }

    /// Shows a dialog to a player (`ServerPlayer.openDialog`).
    pub fn show_dialog(&mut self, conn: ConnId, dialog: &Dialog) {
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(common::show_dialog_play(dialog));
        }
    }
}

fn translate(key: &str) -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String(key.into()))])
}
