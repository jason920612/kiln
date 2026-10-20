//! Players' chat as vanilla's `ServerGamePacketListenerImpl` handles it: the message chain
//! (`SignedMessageChain`), the acknowledgements of the messages a client has seen
//! (`LastSeenMessagesValidator`), the signature cache (`MessageSignatureCache`), the chat session
//! (`RemoteChatSession`) and `PLAYER_CHAT` packets (`OutgoingChatMessage`).
//!
//! A client without a chat session (offline accounts, and any client on an offline-mode server) chats
//! unsigned: the server accepts the text as it is, unless `enforce-secure-profile` is on in online
//! mode, and sends it on as an unsigned `PLAYER_CHAT` that clients show as "not secure".

use kiln_proto::packets::PackedSignature;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use sha2::Digest;
use uuid::Uuid;

/// `LastSeenMessagesValidator(20)`: how many messages a client acknowledges at a time.
pub const LAST_SEEN: usize = 20;
/// `MessageSignatureCache.createDefault()`.
pub const SIGNATURE_CACHE: usize = 128;
/// `sendPlayerChatMessage`: more unacknowledged messages than this and the client is kicked.
pub const MAX_PENDING: usize = 4096;
/// `PlayerChatMessage.MESSAGE_EXPIRES_AFTER_SERVER`: a message older than this is "not secure".
pub const EXPIRES_AFTER_SERVER_MS: i64 = 5 * 60 * 1000;

/// A 256 byte RSA signature of a message (`MessageSignature`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signature(pub Box<[u8; 256]>);

impl Signature {
    /// `Arrays.hashCode(bytes)`.
    pub fn checksum(&self) -> i32 {
        self.0.iter().fold(1i32, |h, b| h.wrapping_mul(31).wrapping_add(i32::from(*b as i8)))
    }
}

/// `LastSeenMessages.computeChecksum`: `0` means "not checked" on the wire, so a checksum of 0 is 1.
pub fn checksum(entries: &[Signature]) -> u8 {
    let sum = entries.iter().fold(1i32, |h, s| h.wrapping_mul(31).wrapping_add(s.checksum())) as u8;
    if sum == 0 { 1 } else { sum }
}

/// Why `LastSeenMessagesValidator` refused an update (the client is kicked).
#[derive(Debug, PartialEq, Eq)]
pub struct ValidationError(pub String);

#[derive(Clone, Debug)]
struct Tracked {
    signature: Signature,
    pending: bool,
}

/// `LastSeenMessagesValidator`: the messages the server sent that the client has not yet
/// acknowledged, and the last `lastSeenCount` it has acknowledged.
#[derive(Debug)]
pub struct LastSeenValidator {
    tracked: Vec<Option<Tracked>>,
    count: usize,
    last_pending: Option<Signature>,
}

impl LastSeenValidator {
    pub fn new(count: usize) -> Self {
        Self { tracked: vec![None; count], count, last_pending: None }
    }

    /// A signed message was sent to the client: it may acknowledge it from now on.
    pub fn add_pending(&mut self, signature: &Signature) {
        if self.last_pending.as_ref() != Some(signature) {
            self.tracked.push(Some(Tracked { signature: signature.clone(), pending: true }));
            self.last_pending = Some(signature.clone());
        }
    }

    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    /// `applyOffset`: the client no longer needs the oldest `offset` messages.
    pub fn apply_offset(&mut self, offset: i32) -> Result<(), ValidationError> {
        let max = self.tracked.len() as i32 - self.count as i32;
        if offset < 0 || offset > max {
            return Err(ValidationError(format!("Advanced last seen window by {offset} messages, but expected at most {max}")));
        }
        self.tracked.drain(..offset as usize);
        Ok(())
    }

    /// `applyUpdate`: the offset, then the acknowledged set over the window; returns the messages
    /// the client says it has seen, oldest first.
    pub fn apply_update(&mut self, offset: i32, acknowledged: [u8; 3], checksum_byte: u8) -> Result<Vec<Signature>, ValidationError> {
        self.apply_offset(offset)?;
        let bits = u32::from(acknowledged[0]) | u32::from(acknowledged[1]) << 8 | u32::from(acknowledged[2]) << 16;
        // `BitSet.length()`: one past the highest set bit.
        let length = 32 - bits.leading_zeros() as usize;
        if length > self.count {
            return Err(ValidationError(format!("Last seen update contained {length} messages, but maximum window size is {}", self.count)));
        }
        let mut seen = Vec::with_capacity(bits.count_ones() as usize);
        for i in 0..self.count {
            let acked = bits >> i & 1 == 1;
            let entry = self.tracked[i].clone();
            if acked {
                let Some(entry) = entry else {
                    return Err(ValidationError(format!("Last seen update acknowledged unknown or incorrectly sized message at index {i}")));
                };
                seen.push(entry.signature.clone());
                self.tracked[i] = Some(Tracked { signature: entry.signature, pending: false });
            } else {
                if let Some(entry) = entry
                    && !entry.pending
                {
                    return Err(ValidationError(format!("Last seen update tried to unacknowledge a message {}", i)));
                }
                self.tracked[i] = None;
            }
        }
        // `Update.verifyChecksum`: 0 skips the check.
        if checksum_byte != 0 && checksum_byte != checksum(&seen) {
            return Err(ValidationError("Checksum mismatch on last seen update: the client and server must have desynced".to_owned()));
        }
        Ok(seen)
    }
}

/// `MessageSignatureCache`: the last 128 signatures sent, so later messages name them by slot.
#[derive(Debug)]
pub struct SignatureCache {
    entries: Vec<Option<Signature>>,
}

impl Default for SignatureCache {
    fn default() -> Self {
        Self { entries: vec![None; SIGNATURE_CACHE] }
    }
}

impl SignatureCache {
    /// The slot holding `signature`, or the whole signature.
    pub fn pack(&self, signature: &Signature) -> PackedSignature {
        match self.entries.iter().position(|e| e.as_ref() == Some(signature)) {
            Some(i) => PackedSignature::Cached(i as i32),
            None => PackedSignature::Full(signature.0.clone()),
        }
    }

    /// `push(body, signature)`: the message's last-seen signatures and its own go in at the front, the oldest fall out
    /// unless the new messages hold them already.
    pub fn push(&mut self, last_seen: &[Signature], own: Option<&Signature>) {
        let mut queue: std::collections::VecDeque<Signature> = last_seen.iter().cloned().collect();
        if let Some(own) = own {
            queue.push_back(own.clone());
        }
        let incoming: std::collections::HashSet<Vec<u8>> = queue.iter().map(|s| s.0.to_vec()).collect();
        let mut i = 0;
        while !queue.is_empty() && i < self.entries.len() {
            let old = self.entries[i].take();
            self.entries[i] = queue.pop_back();
            if let Some(old) = old
                && !incoming.contains(&old.0.to_vec())
            {
                queue.push_front(old);
            }
            i += 1;
        }
    }
}

/// `SignedMessageLink`: where a message sits in its sender's chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub index: i32,
    pub sender: Uuid,
    pub session: Uuid,
}

impl Link {
    pub fn root(sender: Uuid, session: Uuid) -> Self {
        Self { index: 0, sender, session }
    }

    /// The link of an unsigned message (`unsigned`): no session.
    pub fn unsigned(sender: Uuid) -> Self {
        Self::root(sender, Uuid::nil())
    }

    pub fn advance(&self) -> Option<Link> {
        (self.index != i32::MAX).then(|| Link { index: self.index + 1, ..self.clone() })
    }

    fn update(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.sender.as_bytes());
        out.extend_from_slice(self.session.as_bytes());
        out.extend_from_slice(&self.index.to_be_bytes());
    }
}

/// `SignedMessageBody`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub content: String,
    pub timestamp_ms: i64,
    pub salt: i64,
    pub last_seen: Vec<Signature>,
}

impl Body {
    /// `SignedMessageBody.unsigned`: now, no salt, nothing seen.
    pub fn unsigned(content: &str, now_ms: i64) -> Self {
        Self { content: content.to_owned(), timestamp_ms: now_ms, salt: 0, last_seen: Vec::new() }
    }

    fn update(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.salt.to_be_bytes());
        out.extend_from_slice(&self.timestamp_ms.div_euclid(1000).to_be_bytes());
        out.extend_from_slice(&(self.content.len() as i32).to_be_bytes());
        out.extend_from_slice(self.content.as_bytes());
        out.extend_from_slice(&(self.last_seen.len() as i32).to_be_bytes());
        for s in &self.last_seen {
            out.extend_from_slice(&s.0[..]);
        }
    }
}

/// What a signature covers (`PlayerChatMessage.updateSignature`): the version 1, the link, the body.
pub fn signed_payload(link: &Link, body: &Body) -> Vec<u8> {
    let mut out = 1i32.to_be_bytes().to_vec();
    link.update(&mut out);
    body.update(&mut out);
    out
}

/// `PlayerChatMessage` as the server passes it around (the filter mask is always pass-through).
#[derive(Clone, Debug)]
pub struct Message {
    pub link: Link,
    pub signature: Option<Signature>,
    pub body: Body,
    /// The decorated text when it differs from the signed content.
    pub unsigned_content: Option<String>,
}

impl Message {
    pub fn unsigned(sender: Uuid, content: &str, now_ms: i64) -> Self {
        Self { link: Link::unsigned(sender), signature: None, body: Body::unsigned(content, now_ms), unsigned_content: None }
    }

    /// `PlayerChatMessage.system`: a message the server itself says, with nobody's signature.
    pub fn system(content: &str, now_ms: i64) -> Self {
        Self::unsigned(Uuid::nil(), content, now_ms)
    }

    /// Whether the server says it (such a message is sent disguised).
    pub fn is_system(&self) -> bool {
        self.link.sender.is_nil()
    }

    /// `hasExpiredServer`.
    pub fn expired_server(&self, now_ms: i64) -> bool {
        self.body.timestamp_ms + EXPIRES_AFTER_SERVER_MS < now_ms
    }
}

/// `SignedMessageChain.DecodeException`: the translation key of what is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    MissingProfileKey,
    ChainBroken,
    ExpiredProfileKey,
    InvalidSignature,
    OutOfOrderChat,
}

impl DecodeError {
    pub fn key(self) -> &'static str {
        match self {
            DecodeError::MissingProfileKey => "chat.disabled.missingProfileKey",
            DecodeError::ChainBroken => "chat.disabled.chain_broken",
            DecodeError::ExpiredProfileKey => "chat.disabled.expiredProfileKey",
            DecodeError::InvalidSignature => "chat.disabled.invalid_signature",
            DecodeError::OutOfOrderChat => "chat.disabled.out_of_order_chat",
        }
    }
}

/// A validated chat session (`RemoteChatSession`): the id and the profile's public key.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: Uuid,
    pub expires_at_ms: i64,
    pub key_der: Vec<u8>,
    pub key_signature: Vec<u8>,
    key: RsaPublicKey,
}

/// `ProfilePublicKey.createValidated` failed.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// "multiplayer.disconnect.invalid_public_key_signature"
    InvalidSignature,
}

impl Session {
    /// What Mojang signs: the profile's UUID (two big-endian longs), the expiry in milliseconds, the key's X.509 encoding.
    pub fn signed_payload(profile: Uuid, expires_at_ms: i64, key_der: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + key_der.len());
        out.extend_from_slice(profile.as_bytes());
        out.extend_from_slice(&expires_at_ms.to_be_bytes());
        out.extend_from_slice(key_der);
        out
    }

    /// `RemoteChatSession.Data.validate`: the key must be signed by one of Mojang's services keys
    /// (`SHA1withRSA`) for this profile.
    pub fn validate(
        profile: Uuid,
        id: Uuid,
        expires_at_ms: i64,
        key_der: &[u8],
        key_signature: &[u8],
        services_keys: &[RsaPublicKey],
    ) -> Result<Session, SessionError> {
        let key = RsaPublicKey::from_public_key_der(key_der).map_err(|_| SessionError::InvalidSignature)?;
        let payload = Self::signed_payload(profile, expires_at_ms, key_der);
        let digest = sha1::Sha1::digest(&payload);
        let ok = services_keys.iter().any(|k| k.verify(Pkcs1v15Sign::new::<sha1::Sha1>(), &digest, key_signature).is_ok());
        if !ok {
            return Err(SessionError::InvalidSignature);
        }
        Ok(Session { id, expires_at_ms, key_der: key_der.to_vec(), key_signature: key_signature.to_vec(), key })
    }

    /// The same session as data, comparing keys (`Objects.equals(oldData, newData)`).
    pub fn same_key(&self, expires_at_ms: i64, key_der: &[u8], key_signature: &[u8]) -> bool {
        self.expires_at_ms == expires_at_ms && self.key_der == key_der && self.key_signature == key_signature
    }
}

/// The player's `SignedMessageChain.Decoder`.
#[derive(Debug)]
pub enum Decoder {
    /// Before a chat session, or without one: accepts text as it is unless secure chat is enforced.
    Unsigned { enforce: bool },
    /// After a chat session update: verifies every message against the session's key.
    Keyed(Chain),
}

/// `SignedMessageChain` with its decoder: the next link expected and the time of the last message.
#[derive(Debug)]
pub struct Chain {
    session: Session,
    sender: Uuid,
    next_link: Option<Link>,
    last_timestamp_ms: i64,
}

impl Decoder {
    pub fn keyed(session: Session, sender: Uuid) -> Self {
        let next_link = Some(Link::root(sender, session.id));
        Decoder::Keyed(Chain { session, sender, next_link, last_timestamp_ms: 0 })
    }

    /// `Decoder.unpack`.
    pub fn unpack(&mut self, sender: Uuid, signature: Option<Signature>, body: Body, now_ms: i64) -> Result<Message, DecodeError> {
        match self {
            Decoder::Unsigned { enforce } => {
                if *enforce {
                    return Err(DecodeError::MissingProfileKey);
                }
                Ok(Message::unsigned(sender, &body.content, now_ms))
            }
            Decoder::Keyed(chain) => chain.unpack(signature, body, now_ms),
        }
    }

    pub fn session(&self) -> Option<&Session> {
        match self {
            Decoder::Keyed(c) => Some(&c.session),
            Decoder::Unsigned { .. } => None,
        }
    }
}

impl Chain {
    fn unpack(&mut self, signature: Option<Signature>, body: Body, now_ms: i64) -> Result<Message, DecodeError> {
        let Some(signature) = signature else { return Err(DecodeError::MissingProfileKey) };
        if self.session.expires_at_ms < now_ms {
            return Err(DecodeError::ExpiredProfileKey);
        }
        let Some(link) = self.next_link.clone() else { return Err(DecodeError::ChainBroken) };
        if body.timestamp_ms < self.last_timestamp_ms {
            self.next_link = None;
            return Err(DecodeError::OutOfOrderChat);
        }
        self.last_timestamp_ms = body.timestamp_ms;
        let message = Message { link: link.clone(), signature: Some(signature), body, unsigned_content: None };
        if !self.verify(&message) {
            self.next_link = None;
            return Err(DecodeError::InvalidSignature);
        }
        if message.expired_server(now_ms) {
            tracing::warn!("Received expired chat: '{}'. Is the client/server system time unsynchronized?", message.body.content);
        }
        self.next_link = link.advance();
        let _ = self.sender;
        Ok(message)
    }

    fn verify(&self, m: &Message) -> bool {
        let Some(sig) = &m.signature else { return false };
        let digest = sha2::Sha256::digest(signed_payload(&m.link, &m.body));
        self.session.key.verify(Pkcs1v15Sign::new::<sha2::Sha256>(), &digest, &sig.0[..]).is_ok()
    }
}

/// `TickThrottler`: counts messages and commands per player, falling by one each tick.
#[derive(Debug)]
pub struct Throttler {
    step: i32,
    threshold: i32,
    count: i32,
}

impl Throttler {
    pub fn new(step: i32, threshold: i32) -> Self {
        Self { step, threshold, count: 0 }
    }

    pub fn increment(&mut self) {
        self.count += self.step;
    }

    pub fn tick(&mut self) {
        if self.count > 0 {
            self.count -= 1;
        }
    }

    pub fn under_threshold(&self) -> bool {
        self.threshold <= 0 || self.count < self.threshold
    }
}

/// A player's chat state (the fields of `ServerGamePacketListenerImpl`).
#[derive(Debug)]
pub struct ChatState {
    pub decoder: Decoder,
    pub last_seen: LastSeenValidator,
    pub cache: SignatureCache,
    /// `nextChatIndex`: `globalIndex` of the next `PLAYER_CHAT` sent to this player.
    pub next_index: i32,
    pub chat_throttle: Throttler,
    pub command_throttle: Throttler,
}

impl ChatState {
    pub fn new(enforce_secure_profile: bool, command_seconds: i32, chat_seconds: i32) -> Self {
        Self {
            decoder: Decoder::Unsigned { enforce: enforce_secure_profile },
            last_seen: LastSeenValidator::new(LAST_SEEN),
            cache: SignatureCache::default(),
            next_index: 0,
            chat_throttle: Throttler::new(20, 20 * chat_seconds),
            command_throttle: Throttler::new(20, 20 * command_seconds),
        }
    }

    pub fn session(&self) -> Option<&Session> {
        self.decoder.session()
    }

    /// `Decoder.setChainBroken`: later signed messages are refused until a new session.
    pub fn break_chain(&mut self) {
        if let Decoder::Keyed(c) = &mut self.decoder {
            c.next_link = None;
        }
    }
}

#[cfg(test)]
mod tests;
