//! wp86: signed chat. A player's chat session (a key Mojang's services key signed for their profile) is checked, their messages are
//! verified against it and chained (index, last seen acknowledgements), and other players get them as signed `PLAYER_CHAT`
//! packets whose last-seen list names signatures by their slot in the receiver's cache (`MessageSignatureCache`).
//! The keys are made here; the byte layouts are the ones of vanilla's `SignedMessageBody`, `SignedMessageLink` and
//! `PlayerChatMessage.updateSignature`, `ProfilePublicKey.Data.signedPayload` and `LastSeenMessages.computeChecksum`.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::Reader;
use kiln_proto::packets::commands::LastSeenUpdate;
use kiln_sim::testing::{self, Client, SinkStats};
use kiln_sim::{Sim, SimConfig};
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const PLAYER_CHAT: i32 = kiln_data::packets::play::clientbound::PLAYER_CHAT;
const SYSTEM_CHAT: i32 = kiln_data::packets::play::clientbound::SYSTEM_CHAT;
const DISCONNECT: i32 = kiln_data::packets::play::clientbound::DISCONNECT;
const PLAYER_INFO_UPDATE: i32 = kiln_data::packets::play::clientbound::PLAYER_INFO_UPDATE;

fn id_of(p: &[u8]) -> i32 {
    Reader::new(p).varint().unwrap()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

struct Keys {
    services: RsaPrivateKey,
    player: RsaPrivateKey,
}

fn keys() -> Keys {
    let mut rng = rand_core::OsRng;
    Keys { services: RsaPrivateKey::new(&mut rng, 2048).unwrap(), player: RsaPrivateKey::new(&mut rng, 2048).unwrap() }
}

fn der(k: &RsaPrivateKey) -> Vec<u8> {
    k.to_public_key().to_public_key_der().unwrap().as_bytes().to_vec()
}

/// `Arrays.hashCode(byte[])`.
fn array_hash(b: &[u8]) -> i32 {
    b.iter().fold(1i32, |h, x| h.wrapping_mul(31).wrapping_add(i32::from(*x as i8)))
}

/// `LastSeenMessages.computeChecksum`.
fn checksum(seen: &[Vec<u8>]) -> u8 {
    let sum = seen.iter().fold(1i32, |h, s| h.wrapping_mul(31).wrapping_add(array_hash(s))) as u8;
    if sum == 0 { 1 } else { sum }
}

struct Chain {
    sender: Uuid,
    session: Uuid,
    key: RsaPrivateKey,
    next_index: i32,
}

impl Chain {
    /// A signed `Chat` packet for `content`, acknowledging `seen` (the signatures the server sent that the client has seen).
    fn message(&mut self, content: &str, ts: i64, salt: i64, offset: i32, bits: [u8; 3], seen: &[Vec<u8>]) -> PlayIn {
        let mut payload = 1i32.to_be_bytes().to_vec();
        payload.extend_from_slice(self.sender.as_bytes());
        payload.extend_from_slice(self.session.as_bytes());
        payload.extend_from_slice(&self.next_index.to_be_bytes());
        payload.extend_from_slice(&salt.to_be_bytes());
        payload.extend_from_slice(&(ts / 1000).to_be_bytes());
        payload.extend_from_slice(&(content.len() as i32).to_be_bytes());
        payload.extend_from_slice(content.as_bytes());
        payload.extend_from_slice(&(seen.len() as i32).to_be_bytes());
        for s in seen {
            payload.extend_from_slice(s);
        }
        let signature: [u8; 256] = self.key.sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&payload)).unwrap().try_into().unwrap();
        self.next_index += 1;
        PlayIn::Chat {
            message: content.to_owned(),
            timestamp_ms: ts,
            salt,
            signature: Some(Box::new(signature)),
            last_seen: LastSeenUpdate { offset, acknowledged: bits, checksum: checksum(seen) },
        }
    }
}

struct Player {
    uuid: Uuid,
    stats: Arc<SinkStats>,
    client: Client,
}

fn join(sim: &mut Sim, conn: u64, name: &str) -> Player {
    let (msg, stats) = testing::join(conn, name, 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    assert!(sim.step([msg]));
    Player { uuid: Uuid::from_u64_pair(0x6b69_6c6e, conn), client: Client::new(conn, stats.clone()), stats }
}

fn run(sim: &mut Sim, players: &mut [&mut Player], n: usize) {
    for _ in 0..n {
        let mut inbox = Vec::new();
        for p in players.iter_mut() {
            p.client.tick(None, &mut inbox);
        }
        assert!(sim.step(inbox));
    }
}

fn log(p: &Player) -> Vec<bytes::Bytes> {
    p.stats.log.lock().unwrap().clone().unwrap()
}

/// (global index, chain index, signature, content, last-seen entries: `Ok(slot)` or `Err(())` for a full signature).
type Seen = Vec<Result<i32, ()>>;

fn signed_chats(p: &Player) -> Vec<(i32, i32, Option<Vec<u8>>, String, Seen)> {
    log(p)
        .iter()
        .filter(|b| id_of(b) == PLAYER_CHAT)
        .map(|b| {
            let mut r = Reader::new(b);
            r.varint().unwrap();
            let global = r.varint().unwrap();
            r.uuid().unwrap();
            let index = r.varint().unwrap();
            let sig = r.bool().unwrap().then(|| r.bytes(256).unwrap().to_vec());
            let content = r.string(256).unwrap().to_owned();
            r.i64().unwrap();
            r.i64().unwrap();
            let n = r.varint().unwrap();
            let seen = (0..n)
                .map(|_| match r.varint().unwrap() {
                    0 => {
                        r.bytes(256).unwrap();
                        Err(())
                    }
                    v => Ok(v - 1),
                })
                .collect();
            (global, index, sig, content, seen)
        })
        .collect()
}

fn said(p: &Player, id: i32, needle: &str) -> bool {
    log(p).iter().any(|b| id_of(b) == id && b.windows(needle.len()).any(|w| w == needle.as_bytes()))
}

fn session_update(k: &Keys, profile: Uuid, session: Uuid, expires_at_ms: i64) -> PlayIn {
    let key = der(&k.player);
    let mut payload = profile.as_bytes().to_vec();
    payload.extend_from_slice(&expires_at_ms.to_be_bytes());
    payload.extend_from_slice(&key);
    let key_signature = k.services.sign(Pkcs1v15Sign::new::<Sha1>(), &Sha1::digest(&payload)).unwrap();
    PlayIn::ChatSessionUpdate { session_id: session, expires_at_ms, key, key_signature }
}

fn setup(enforce: bool) -> (Sim, Keys, Player, Player) {
    let k = keys();
    let mut config = SimConfig::new(4, 2, None);
    config.server.enforce_secure_profile = enforce;
    *config.info.services_keys.write().unwrap() = vec![der(&k.services)];
    let mut sim = Sim::new(config);
    let mut a = join(&mut sim, 1, "Alice");
    let mut b = join(&mut sim, 2, "Bob");
    run(&mut sim, &mut [&mut a, &mut b], 40);
    (sim, k, a, b)
}

#[test]
fn a_chat_session_and_signed_messages_are_verified_chained_and_relayed() {
    let (mut sim, k, mut a, mut b) = setup(true);
    let session = Uuid::from_u128(0x5e55_1011);
    let chain_key = k.player.clone();
    assert!(sim.step([ToSim::Packet(1, session_update(&k, a.uuid, session, now_ms() + 3_600_000))]));
    run(&mut sim, &mut [&mut a, &mut b], 2);
    // Everybody learns of the session: Player Info Update with the INITIALIZE_CHAT action (bit 1).
    assert!(log(&b).iter().any(|p| id_of(p) == PLAYER_INFO_UPDATE && p[1] == 0b10), "Bob was told of Alice's chat session");
    let mut chain = Chain { sender: a.uuid, session, key: chain_key, next_index: 0 };

    // First message: nothing acknowledged.
    let t0 = now_ms();
    assert!(sim.step([ToSim::Packet(1, chain.message("signed hello", t0, 42, 0, [0; 3], &[]))]));
    run(&mut sim, &mut [&mut a, &mut b], 2);
    let (got_a, got_b) = (signed_chats(&a), signed_chats(&b));
    assert_eq!(got_a.len(), 1);
    assert_eq!(got_b.len(), 1);
    let (global, index, sig, content, seen) = got_b[0].clone();
    assert_eq!((global, index, content.as_str(), seen.len()), (0, 0, "signed hello", 0));
    let sig1 = sig.expect("relayed with its signature");
    assert_eq!(sig1.len(), 256);
    assert!(!a.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed) && !said(&a, SYSTEM_CHAT, "chat.disabled"));

    // Second message acknowledges the first: the window moved one message, bit 19 is the newest. Bob sees the signature by its
    // cache slot, because he was sent it before.
    let seen1 = vec![sig1.clone()];
    assert!(sim.step([ToSim::Packet(1, chain.message("second", t0 + 10, 43, 1, [0, 0, 8], &seen1))]));
    run(&mut sim, &mut [&mut a, &mut b], 2);
    let second = signed_chats(&b).pop().unwrap();
    assert_eq!((second.0, second.1, second.3.as_str()), (1, 1, "second"));
    assert_eq!(second.4, vec![Ok(0)], "the acknowledged message by slot 0 of Bob's signature cache");
    assert!(!a.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed), "a correct chain keeps Alice connected");

    // A message signed for the wrong place in the chain (index 1 again) is refused, and the chain stays broken.
    chain.next_index = 1;
    let sig2 = signed_chats(&b)[1].2.clone().unwrap();
    let seen2 = vec![sig1.clone(), sig2.clone()];
    assert!(sim.step([ToSim::Packet(1, chain.message("replayed", t0 + 20, 44, 1, [0, 0, 12], &seen2))]));
    run(&mut sim, &mut [&mut a, &mut b], 2);
    assert!(said(&a, SYSTEM_CHAT, "chat.disabled.invalid_signature"), "told in red");
    assert_eq!(signed_chats(&b).len(), 2, "not relayed");
    chain.next_index = 2;
    assert!(sim.step([ToSim::Packet(1, chain.message("after the break", t0 + 30, 45, 1, [0, 0, 12], &seen2))]));
    run(&mut sim, &mut [&mut a, &mut b], 2);
    assert!(said(&a, SYSTEM_CHAT, "chat.disabled.chain_broken"));
    assert_eq!(signed_chats(&b).len(), 2);
}

#[test]
fn a_session_key_not_signed_by_the_services_key_is_refused() {
    let (mut sim, mut k, mut a, _b) = setup(false);
    k.services = RsaPrivateKey::new(&mut rand_core::OsRng, 2048).unwrap(); // signs with a key the server does not know
    assert!(sim.step([ToSim::Packet(1, session_update(&k, a.uuid, Uuid::from_u128(1), now_ms() + 3_600_000))]));
    run(&mut sim, &mut [&mut a], 2);
    assert!(said(&a, DISCONNECT, "multiplayer.disconnect.invalid_public_key_signature"));
}

#[test]
fn an_older_session_key_than_the_one_in_use_is_refused() {
    let (mut sim, k, mut a, _b) = setup(false);
    let later = now_ms() + 7_200_000;
    assert!(sim.step([ToSim::Packet(1, session_update(&k, a.uuid, Uuid::from_u128(1), later))]));
    assert!(sim.step([ToSim::Packet(1, session_update(&k, a.uuid, Uuid::from_u128(2), later - 1000))]));
    run(&mut sim, &mut [&mut a], 2);
    assert!(said(&a, DISCONNECT, "multiplayer.disconnect.expired_public_key"));
}

#[test]
fn without_the_services_keys_a_session_is_ignored_and_enforced_chat_is_refused() {
    let k = keys();
    let mut config = SimConfig::new(4, 2, None);
    config.server.enforce_secure_profile = true;
    let mut sim = Sim::new(config);
    let mut a = join(&mut sim, 1, "Alice");
    run(&mut sim, &mut [&mut a], 40);
    assert!(sim.step([ToSim::Packet(1, session_update(&k, a.uuid, Uuid::from_u128(1), now_ms() + 3_600_000))]));
    assert!(!a.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed));
    assert!(sim.step([ToSim::Packet(1, PlayIn::chat("unsigned"))]));
    run(&mut sim, &mut [&mut a], 2);
    assert!(said(&a, SYSTEM_CHAT, "chat.disabled.missingProfileKey"));
}
