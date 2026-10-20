//! The simulation's side of chat: the packet handlers of `ServerGamePacketListenerImpl`
//! (`handleChat`, `handleChatCommand`, `handleSignedChatCommand`, `handleChatAck`,
//! `handleChatSessionUpdate`) and the delivery of chat (`PlayerList.broadcastChatMessage`,
//! `OutgoingChatMessage`). The message chain itself is in [`crate::chat`].

use crate::chat::{self, Body, DecodeError, Message, Signature};
use crate::commands::{CommandSource, console_text, has_illegal_chars};
use crate::{ConnId, Sim};
use kiln_command::host::{ChatKind, ChatMessage};
use kiln_command::text::{HoverEntity, Text};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::commands::{LastSeenUpdate, SignedChatCommand};
use kiln_proto::packets::{self, PlayerChat};
use std::collections::HashMap;
use tracing::{error, info, warn};

/// `ChatVisiblity`: the packets a player with this chat setting does not get.
pub(crate) const CHAT_FULL: u8 = 0;
pub(crate) const CHAT_HIDDEN: u8 = 2;

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A player's display name as chat shows it (`Player.getDisplayName`): clicking it suggests
/// `/tell <name> `, shift-clicking inserts the name, hovering shows the entity.
pub(crate) fn decorate_player_name(base: Text, name: &str, uuid: uuid::Uuid) -> Text {
    base.click(kiln_command::text::ClickEvent::SuggestCommand(format!("/tell {name} ")))
        .insertion(name)
        .hover_entity(HoverEntity { name: Some(Text::literal(name)), id: "minecraft:player".to_owned(), uuid })
}

impl Sim {
    /// The name of the sender or target of a chat message: players' names get the click and hover
    /// events of their display name, the console's stays plain.
    fn chat_name(&self, name: &Text) -> Text {
        let plain = name.to_plain();
        match self.players.values().find(|p| p.name == plain) {
            Some(p) if name.style.hover_entity.is_none() => decorate_player_name(name.clone(), &p.name, p.uuid),
            _ => name.clone(),
        }
    }

    /// `unpackAndApplyLastSeen`: `None` (and the player is kicked) when the acknowledgements are wrong.
    fn apply_last_seen(&mut self, conn: ConnId, update: &LastSeenUpdate) -> Option<Vec<Signature>> {
        let p = self.players.get_mut(&conn)?;
        match p.chat.last_seen.apply_update(update.offset, update.acknowledged, update.checksum) {
            Ok(seen) => Some(seen),
            Err(e) => {
                error!("Failed to validate message acknowledgements from {}: {}", p.name, e.0);
                p.disconnect_text(Text::translate("multiplayer.disconnect.chat_validation_failed", vec![]).to_nbt());
                None
            }
        }
    }

    /// `tryHandleChat`: false when the player was kicked for illegal characters or the message is
    /// not for them to send (chat hidden in their options).
    fn try_handle_chat(&mut self, conn: ConnId, text: &str, is_command: bool) -> bool {
        let Some(p) = self.players.get_mut(&conn) else { return false };
        if has_illegal_chars(text) {
            p.disconnect_text(Text::translate("multiplayer.disconnect.illegal_characters", vec![]).to_nbt());
            return false;
        }
        if !is_command && p.client.chat_visibility == CHAT_HIDDEN {
            p.send_unfiltered(packets::system_chat(Text::translate("chat.disabled.options", vec![]).color("red").to_nbt(), false));
            return false;
        }
        p.last_action = std::time::Instant::now();
        true
    }

    /// `detectRateSpam`: too many messages or commands in a short time and a player who is not an operator is kicked.
    fn detect_rate_spam(&mut self, conn: ConnId, chat: bool) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        let throttle = if chat { &mut p.chat.chat_throttle } else { &mut p.chat.command_throttle };
        throttle.increment();
        if !throttle.under_threshold() && !self.commands.is_op(&p.name) {
            p.disconnect_text(Text::translate("disconnect.spam", vec![]).to_nbt());
        }
    }

    /// `handleMessageDecodeFailure`: the player is told in red, nothing else happens.
    fn decode_failure(&mut self, conn: ConnId, e: DecodeError) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        warn!("Failed to update secure chat state for {}: '{}'", p.name, console_text(&Text::translate(e.key(), vec![])));
        p.send(packets::system_chat(Text::translate(e.key(), vec![]).color("red").to_nbt(), false));
    }

    /// `ServerboundChatPacket`.
    pub(crate) fn handle_chat(
        &mut self,
        conn: ConnId,
        message: String,
        timestamp_ms: i64,
        salt: i64,
        signature: Option<Box<[u8; 256]>>,
        last_seen: LastSeenUpdate,
    ) {
        let Some(seen) = self.apply_last_seen(conn, &last_seen) else { return };
        if !self.try_handle_chat(conn, &message, false) {
            return;
        }
        let now = now_ms();
        let Some(p) = self.players.get_mut(&conn) else { return };
        let body = Body { content: message.clone(), timestamp_ms, salt, last_seen: seen };
        let signature = signature.map(Signature);
        let decoded = p.chat.decoder.unpack(p.uuid, signature, body, now);
        let msg = match decoded {
            Ok(m) => m,
            Err(e) => return self.decode_failure(conn, e),
        };
        if self.plugin_chat(conn, &message) {
            return;
        }
        self.broadcast_chat_message(Some(conn), &msg, ChatKind::Chat, None);
        self.detect_rate_spam(conn, true);
    }

    /// `ServerboundChatCommandPacket`: a command with no signed arguments.
    pub(crate) fn handle_chat_command(&mut self, conn: ConnId, command: String) {
        if !self.try_handle_chat(conn, &command, true) {
            return;
        }
        // `performUnsignedChatCommand`: with secure chat enforced, a command with message arguments needs signatures.
        let enforce = self.config.server.enforce_secure_profile;
        if enforce && !self.signable_arguments(conn, &command).is_empty() {
            if let Some(p) = self.players.get_mut(&conn) {
                error!("Received unsigned command packet from {}, but the command requires signable arguments: {command}", p.name);
                p.send(packets::system_chat(Text::translate("chat.disabled.invalid_command_signature", vec![]).color("red").to_nbt(), false));
            }
            return;
        }
        if !self.plugin_command_denied(conn, &command) {
            self.run_command(conn, &command);
        }
        self.detect_rate_spam(conn, false);
    }

    fn signable_arguments(&mut self, conn: ConnId, command: &str) -> Vec<(String, String)> {
        let dispatcher = self.commands.dispatcher.clone();
        let previous = std::mem::replace(&mut self.commands.source, CommandSource::Player(conn));
        let stack = self.source_stack(CommandSource::Player(conn));
        let stack = std::mem::replace(&mut self.commands.stack, stack);
        let args = dispatcher.signable_arguments(command, self);
        self.commands.source = previous;
        self.commands.stack = stack;
        args
    }

    /// `ServerboundChatCommandSignedPacket`: the signed arguments go to the command as chat messages.
    pub(crate) fn handle_chat_command_signed(&mut self, conn: ConnId, c: SignedChatCommand) {
        let Some(seen) = self.apply_last_seen(conn, &c.last_seen) else { return };
        if !self.try_handle_chat(conn, &c.command, true) {
            return;
        }
        let args = self.signable_arguments(conn, &c.command);
        let now = now_ms();
        let mut messages: HashMap<String, Message> = HashMap::new();
        let Some(p) = self.players.get_mut(&conn) else { return };
        if c.signatures.is_empty() {
            // `collectUnsignedArguments`: each argument is an unsigned message of the player's.
            for (name, value) in &args {
                let body = Body::unsigned(value, now);
                match p.chat.decoder.unpack(p.uuid, None, body, now) {
                    Ok(m) => {
                        messages.insert(name.clone(), m);
                    }
                    Err(e) => return self.decode_failure(conn, e),
                }
            }
        } else {
            for s in &c.signatures {
                let Some((name, value)) = args.iter().find(|(n, _)| *n == s.argument) else {
                    p.chat.break_chain();
                    error!("Signed command mismatch between server and client ('{}'): got [{}] from client, but expected [{}]", c.command, c.signatures.iter().map(|s| s.argument.as_str()).collect::<Vec<_>>().join(","), args.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(","));
                    p.send(packets::system_chat(Text::translate("chat.disabled.invalid_command_signature", vec![]).color("red").to_nbt(), false));
                    return;
                };
                let body = Body { content: value.clone(), timestamp_ms: c.timestamp, salt: c.salt, last_seen: seen.clone() };
                match p.chat.decoder.unpack(p.uuid, Some(Signature(s.signature.clone())), body, now) {
                    Ok(m) => {
                        messages.insert(name.clone(), m);
                    }
                    Err(e) => return self.decode_failure(conn, e),
                }
            }
            if let Some((missing, _)) = args.iter().find(|(n, _)| !messages.contains_key(n)) {
                let _ = missing;
                error!("Signed command mismatch between server and client ('{}')", c.command);
                if let Some(p) = self.players.get_mut(&conn) {
                    p.send(packets::system_chat(Text::translate("chat.disabled.invalid_command_signature", vec![]).color("red").to_nbt(), false));
                }
                return;
            }
        }
        if !self.plugin_command_denied(conn, &c.command) {
            self.commands.signing = messages;
            self.run_command(conn, &c.command);
            self.commands.signing = HashMap::new();
        }
        self.detect_rate_spam(conn, false);
    }

    /// `ServerboundChatAckPacket`.
    pub(crate) fn handle_chat_ack(&mut self, conn: ConnId, offset: i32) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        if let Err(e) = p.chat.last_seen.apply_offset(offset) {
            error!("Failed to validate message acknowledgement offset from {}: {}", p.name, e.0);
            p.disconnect_text(Text::translate("multiplayer.disconnect.chat_validation_failed", vec![]).to_nbt());
        }
    }

    /// `ServerboundChatSessionUpdatePacket`.
    pub(crate) fn handle_chat_session_update(&mut self, conn: ConnId, session_id: uuid::Uuid, expires_at_ms: i64, key: Vec<u8>, key_signature: Vec<u8>) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        // The key is decoded with the packet (`Crypt.byteToPublicKey`): one that is not an RSA key fails to decode.
        if <rsa::RsaPublicKey as rsa::pkcs8::DecodePublicKey>::from_public_key_der(&key).is_err() {
            let reason = "Internal Exception: io.netty.handler.codec.DecoderException: Failed to decode packet 'serverbound/minecraft:chat_session_update'";
            p.disconnect_text(Text::translate("disconnect.genericReason", vec![kiln_command::text::Arg::Str(reason.to_owned())]).to_nbt());
            return;
        }
        let old = p.chat.decoder.session();
        if old.is_some_and(|s| s.same_key(expires_at_ms, &key, &key_signature)) {
            return;
        }
        if old.is_some_and(|s| expires_at_ms < s.expires_at_ms) {
            p.disconnect_text(Text::translate("multiplayer.disconnect.expired_public_key", vec![]).to_nbt());
            return;
        }
        let keys = self.config.info.services_keys();
        if keys.is_empty() {
            warn!("Ignoring chat session from {} due to missing Services public key", p.name);
            return;
        }
        let keys: Vec<rsa::RsaPublicKey> = keys.iter().filter_map(|k| rsa::pkcs8::DecodePublicKey::from_public_key_der(k).ok()).collect();
        match chat::Session::validate(p.uuid, session_id, expires_at_ms, &key, &key_signature, &keys) {
            Ok(session) => {
                let uuid = p.uuid;
                p.chat.decoder = chat::Decoder::keyed(session, uuid);
                self.announce_chat_session(conn);
            }
            Err(chat::SessionError::InvalidSignature) => {
                error!("Failed to validate profile key");
                p.disconnect_text(Text::translate("multiplayer.disconnect.invalid_public_key_signature", vec![]).to_nbt());
            }
        }
    }

    /// `INITIALIZE_CHAT`: everyone learns the player's chat session.
    fn announce_chat_session(&mut self, conn: ConnId) {
        let Some(p) = self.players.get(&conn) else { return };
        let props = p.profile_properties();
        let pkt = kiln_proto::packets::entity::player_info_update(kiln_proto::packets::entity::PlayerInfoActions::INITIALIZE_CHAT, &[p.info_entry(&props)]);
        drop(props);
        self.broadcast(pkt);
    }

    /// `PlayerList.broadcastChatMessage`: `message` to every player (as `PLAYER_CHAT`, or disguised if it is the server's),
    /// and the line in the server log.
    pub(crate) fn broadcast_chat_message(&mut self, sender: Option<ConnId>, message: &Message, kind: ChatKind, display: Option<Text>) {
        let now = now_ms();
        let trusted = message.signature.is_some() && !message.expired_server(now);
        let sender_name = match (&display, sender.and_then(|c| self.players.get(&c))) {
            (Some(t), _) => t.clone(),
            (None, Some(p)) => Text::literal(p.name.clone()),
            (None, None) => Text::literal("Server"),
        };
        let content = Text::literal(message.unsigned_content.clone().unwrap_or_else(|| message.body.content.clone()));
        let chat_message = ChatMessage { kind, sender: sender_name.clone(), target: None, content: content.clone() };
        let line = console_text(&chat_message.to_text());
        if trusted || !self.config.server.console_system_prefix {
            info!("{line}");
        } else {
            info!("[Not Secure] {line}");
        }
        let mut conns: Vec<ConnId> = self.players.keys().copied().collect();
        conns.sort_unstable();
        for c in conns {
            self.deliver_chat(c, message, kind, &sender_name, None, &content);
        }
    }

    /// `ServerPlayer.sendChatMessage(OutgoingChatMessage.create(message), ...)`: to one player, if their chat is on.
    pub(crate) fn deliver_chat(&mut self, to: ConnId, message: &Message, kind: ChatKind, sender: &Text, target: Option<&Text>, content: &Text) {
        let name = self.chat_name(sender).to_nbt().java_ordered();
        let target = target.map(|t| self.chat_name(t).to_nbt().java_ordered());
        let chat_type = kiln_data::synced_id("minecraft:chat_type", kind.id()).unwrap_or(0);
        let Some(p) = self.players.get_mut(&to) else { return };
        // `acceptsChatMessages`: only a player whose chat is fully on.
        if p.client.chat_visibility != CHAT_FULL {
            return;
        }
        if message.is_system() {
            p.send(packets::disguised_chat(&content.to_nbt().java_ordered(), chat_type, &name, target.as_ref()));
            return;
        }
        // The decorated text goes along when it differs from what was signed.
        let unsigned = (content.to_plain() != message.body.content || !matches!(content.to_nbt(), Tag::String(_)))
            .then(|| content.to_nbt().java_ordered());
        let packed: Vec<packets::PackedSignature> = message.body.last_seen.iter().map(|s| p.chat.cache.pack(s)).collect();
        let global_index = p.chat.next_index;
        p.chat.next_index += 1;
        let pkt = packets::player_chat(&PlayerChat {
            global_index,
            sender: message.link.sender,
            index: message.link.index,
            signature: message.signature.as_ref().map(|s| &*s.0),
            content: &message.body.content,
            timestamp_ms: message.body.timestamp_ms,
            salt: message.body.salt,
            last_seen: &packed,
            unsigned_content: unsigned.as_ref(),
            chat_type,
            name: &name,
            target: target.as_ref(),
        });
        p.send(pkt);
        let Some(sig) = &message.signature else { return };
        p.chat.cache.push(&message.body.last_seen, Some(sig));
        p.chat.last_seen.add_pending(sig);
        if p.chat.last_seen.tracked_count() > chat::MAX_PENDING {
            p.disconnect_text(Text::translate("multiplayer.disconnect.too_many_pending_chats", vec![]).to_nbt());
        }
    }

    /// The tick of the players' throttlers (`ServerGamePacketListenerImpl.tick`).
    pub(crate) fn tick_chat_throttles(&mut self) {
        for p in self.players.values_mut() {
            p.chat.chat_throttle.tick();
            p.chat.command_throttle.tick();
        }
    }
}
