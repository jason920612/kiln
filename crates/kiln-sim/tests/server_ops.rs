//! wp86: what a dedicated server's `server.properties` and its chat do in the simulation, as vanilla 26.3's do:
//! `pause-when-empty-seconds`, `allow-flight` (the kick for floating), `spawn-protection`, `gamemode`, the roster the server
//! list reads, chat as `PLAYER_CHAT` (and its disconnects), hidden chat, `enforce-secure-profile`, and commands through the
//! RCON source. The vanilla values are from a vanilla 26.3 server run side by side (docs/wp86.md).

use kiln_link::{ConnId, PlayIn, ToSim};
use kiln_proto::Reader;
use kiln_proto::packets::commands::LastSeenUpdate;
use kiln_sim::testing::{self, Client, SinkStats};
use kiln_sim::{Sim, SimConfig};
use std::sync::Arc;

const PLAYER_CHAT: i32 = kiln_data::packets::play::clientbound::PLAYER_CHAT;
const SYSTEM_CHAT: i32 = kiln_data::packets::play::clientbound::SYSTEM_CHAT;
const DISCONNECT: i32 = kiln_data::packets::play::clientbound::DISCONNECT;

struct Joined {
    conn: ConnId,
    stats: Arc<SinkStats>,
    client: Client,
}

fn join(sim: &mut Sim, conn: ConnId, name: &str) -> Joined {
    let (msg, stats) = testing::join(conn, name, 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    assert!(sim.step([msg]));
    Joined { conn, client: Client::new(conn, stats.clone()), stats }
}

/// Ticks everybody's scripted clients `n` times (moving `who[0]` to `to` each tick if given).
fn run(sim: &mut Sim, players: &mut [&mut Joined], n: usize, to: Option<[f64; 3]>) {
    for _ in 0..n {
        let mut inbox = Vec::new();
        for (i, p) in players.iter_mut().enumerate() {
            p.client.tick(if i == 0 { to } else { None }, &mut inbox);
        }
        assert!(sim.step(inbox));
    }
}

fn settle(sim: &mut Sim, players: &mut [&mut Joined]) {
    run(sim, players, 40, None);
}

fn log(p: &Joined) -> Vec<bytes::Bytes> {
    p.stats.log.lock().unwrap().clone().unwrap()
}

fn id_of(p: &[u8]) -> i32 {
    Reader::new(p).varint().unwrap()
}

/// The `PLAYER_CHAT` packets a player got: (global index, sender, chain index, signed?, content).
fn chats(p: &Joined) -> Vec<(i32, uuid::Uuid, i32, bool, String)> {
    log(p)
        .iter()
        .filter(|b| id_of(b) == PLAYER_CHAT)
        .map(|b| {
            let mut r = Reader::new(b);
            r.varint().unwrap();
            let global = r.varint().unwrap();
            let sender = r.uuid().unwrap();
            let index = r.varint().unwrap();
            let signed = r.bool().unwrap();
            if signed {
                r.bytes(256).unwrap();
            }
            (global, sender, index, signed, r.string(256).unwrap().to_owned())
        })
        .collect()
}

fn contains_text(p: &Joined, id: i32, needle: &str) -> bool {
    log(p).iter().any(|b| id_of(b) == id && b.windows(needle.len()).any(|w| w == needle.as_bytes()))
}

fn two_players(config: SimConfig) -> (Sim, Joined, Joined) {
    let mut sim = Sim::new(config);
    let mut a = join(&mut sim, 1, "Alice");
    let mut b = join(&mut sim, 2, "Bob");
    settle(&mut sim, &mut [&mut a, &mut b]);
    (sim, a, b)
}

fn send(sim: &mut Sim, who: ConnId, pkt: PlayIn) {
    assert!(sim.step([ToSim::Packet(who, pkt)]));
}

// ---- pause-when-empty-seconds ----

#[test]
fn an_empty_world_stops_ticking_after_pause_when_empty_seconds() {
    let mut config = SimConfig::new(4, 2, None);
    config.server.pause_when_empty_seconds = 1; // 20 ticks
    let mut sim = Sim::new(config);
    for _ in 0..10 {
        assert!(sim.step([]));
    }
    let t10 = sim.game_time();
    assert_eq!(t10, 10, "ticking while the pause is not due");
    for _ in 0..40 {
        assert!(sim.step([]));
    }
    let frozen = sim.game_time();
    assert!((19..=20).contains(&frozen), "the world stopped at about 20 ticks, not {frozen}");
    for _ in 0..20 {
        assert!(sim.step([]));
    }
    assert_eq!(sim.game_time(), frozen, "no time passes in a paused world");
    // The console still works while it is paused (it is part of the connection tick), and so does RCON.
    let (tx, rx) = std::sync::mpsc::channel();
    let work: Box<dyn FnOnce(&mut dyn kiln_link::AdminHost) + Send> = Box::new(move |h| {
        let _ = tx.send(h.rcon_command("seed"));
    });
    assert!(sim.step([ToSim::Admin(work)]));
    assert!(!rx.try_recv().expect("RCON was answered during the pause").is_empty());
    // A player logging in wakes the world up.
    let mut p = join(&mut sim, 1, "Waker");
    run(&mut sim, &mut [&mut p], 10, None);
    assert!(sim.game_time() > frozen, "time runs again with a player");
}

#[test]
fn pause_is_off_when_the_setting_is_zero() {
    let mut config = SimConfig::new(4, 2, None);
    config.server.pause_when_empty_seconds = 0;
    let mut sim = Sim::new(config);
    for _ in 0..100 {
        assert!(sim.step([]));
    }
    assert_eq!(sim.game_time(), 100);
}

// ---- the roster ----

#[test]
fn the_server_list_roster_follows_logins() {
    let config = SimConfig::new(4, 2, None);
    let info = config.info.clone();
    let mut sim = Sim::new(config);
    let mut a = join(&mut sim, 1, "Alice");
    let mut b = join(&mut sim, 2, "Bob");
    run(&mut sim, &mut [&mut a, &mut b], 3, None);
    let names: Vec<String> = info.players().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["Alice", "Bob"]);
    assert!(sim.step([ToSim::Leave(1)]));
    assert_eq!(info.player_count(), 1);
}

// ---- gamemode ----

#[test]
fn the_default_game_mode_is_the_propertys() {
    let mut config = SimConfig::new(4, 2, None);
    config.server.default_game_mode = Some(2);
    let mut sim = Sim::new(config);
    let p = join(&mut sim, 1, "Alice");
    assert_eq!(sim.game_mode(p.conn), Some(2), "adventure, from `gamemode=adventure`");
}

// ---- allow-flight ----

fn hover(allow_flight: bool, ticks: usize) -> bool {
    let mut config = SimConfig::new(4, 2, None);
    config.server.default_game_mode = Some(0);
    config.server.allow_flight = allow_flight;
    let mut sim = Sim::new(config);
    let mut p = join(&mut sim, 1, "Floater");
    settle(&mut sim, &mut [&mut p]);
    let up = [p.client.pos[0], p.client.pos[1] + 10.0, p.client.pos[2]];
    run(&mut sim, &mut [&mut p], ticks, Some(up));
    p.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed)
}

#[test]
fn a_survival_player_hovering_too_long_is_kicked_unless_flight_is_allowed() {
    // 80 ticks of floating is allowed (`getMaximumFlyingTicks` at normal gravity); the 81st kicks.
    assert!(!hover(false, 60), "not yet");
    assert!(hover(false, 120), "kicked for floating too long");
    assert!(!hover(true, 120), "allow-flight=true: never");
}

// ---- spawn-protection ----

fn break_below_spawn(protection: i32, op_exists: bool) -> bool {
    let mut config = SimConfig::new(4, 2, None);
    config.server.default_game_mode = Some(1);
    config.server.spawn_protection = protection;
    let mut sim = Sim::new(config);
    let mut p = join(&mut sim, 1, "Alice");
    settle(&mut sim, &mut [&mut p]);
    if op_exists {
        sim.console_now("op Boss");
    }
    let (x, y, z) = (p.client.pos[0].floor() as i32, p.client.pos[1].floor() as i32 - 1, p.client.pos[2].floor() as i32);
    let before = sim.block_at(x, y, z);
    assert!(before.is_some_and(|b| b != 0), "something under the player's feet");
    send(&mut sim, 1, PlayIn::PlayerAction { action: 0, pos: [x, y, z], face: 1, sequence: 1 });
    run(&mut sim, &mut [&mut p], 3, None);
    let protected = sim.block_at(x, y, z) == before;
    if protected {
        assert!(contains_text(&p, SYSTEM_CHAT, "build.spawn_protection"), "told in red over the hotbar");
    }
    protected
}

#[test]
fn only_operators_change_blocks_near_the_world_spawn() {
    assert!(break_below_spawn(16, true), "protected: an operator exists and the player is not one");
    assert!(!break_below_spawn(0, true), "radius 0: no protection");
    assert!(!break_below_spawn(16, false), "nobody is an operator: vanilla protects nothing");
}

// ---- chat ----

#[test]
fn chat_is_a_player_chat_packet_unsigned_without_a_session() {
    let (mut sim, mut a, mut b) = two_players(SimConfig::new(4, 2, None));
    send(&mut sim, 1, PlayIn::chat("hello world"));
    send(&mut sim, 1, PlayIn::chat("second"));
    run(&mut sim, &mut [&mut a, &mut b], 2, None);
    for who in [&a, &b] {
        let got = chats(who);
        assert_eq!(got.len(), 2, "both messages reached everybody, the sender too");
        let alice = got[0].1;
        assert_eq!(got[0], (0, alice, 0, false, "hello world".to_owned()));
        assert_eq!(got[1], (1, alice, 0, false, "second".to_owned()), "the global index counts up, the chain index of unsigned chat stays 0");
    }
    assert!(!contains_text(&a, SYSTEM_CHAT, "hello world"), "not a system message any more");
}

#[test]
fn say_through_a_signed_command_packet_is_a_player_chat_too() {
    let (mut sim, mut a, mut b) = two_players(SimConfig::new(4, 2, None));
    sim.console_now("op Alice");
    let cmd = kiln_proto::packets::commands::SignedChatCommand {
        command: "say hi there".into(),
        timestamp: 1,
        salt: 0,
        signatures: Vec::new(),
        last_seen: LastSeenUpdate { offset: 0, acknowledged: [0; 3], checksum: 0 },
    };
    send(&mut sim, 1, PlayIn::ChatCommandSigned(Box::new(cmd)));
    run(&mut sim, &mut [&mut a, &mut b], 2, None);
    let got = chats(&b);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].3, got[0].4.as_str()), (false, "hi there"));
}

fn disconnected_with(p: &Joined, key: &str) -> bool {
    p.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed) && contains_text(p, DISCONNECT, key)
}

#[test]
fn bad_chat_gets_vanillas_disconnects() {
    let (mut sim, a, _b) = two_players(SimConfig::new(4, 2, None));
    send(&mut sim, 1, PlayIn::chat("a\u{a7}b"));
    assert!(disconnected_with(&a, "multiplayer.disconnect.illegal_characters"));

    let (mut sim, a, _b) = two_players(SimConfig::new(4, 2, None));
    send(&mut sim, 1, PlayIn::ChatAck { offset: 5 });
    assert!(disconnected_with(&a, "multiplayer.disconnect.chat_validation_failed"), "an acknowledgement of messages never sent");

    let (mut sim, a, _b) = two_players(SimConfig::new(4, 2, None));
    let mut m = PlayIn::chat("hi");
    if let PlayIn::Chat { last_seen, .. } = &mut m {
        last_seen.acknowledged = [0, 0, 8];
    }
    send(&mut sim, 1, m);
    assert!(disconnected_with(&a, "multiplayer.disconnect.chat_validation_failed"), "an update past the window");

    let (mut sim, a, _b) = two_players(SimConfig::new(4, 2, None));
    let mut m = PlayIn::chat("hi");
    if let PlayIn::Chat { last_seen, .. } = &mut m {
        last_seen.checksum = 7;
    }
    send(&mut sim, 1, m);
    assert!(disconnected_with(&a, "multiplayer.disconnect.chat_validation_failed"), "a wrong checksum");
}

#[test]
fn too_many_messages_in_a_short_time_is_spam() {
    let (mut sim, a, _b) = two_players(SimConfig::new(4, 2, None));
    // 20 per message against a 200 threshold: the eleventh within the same moment kicks.
    let inbox: Vec<ToSim> = (0..12).map(|i| ToSim::Packet(1, PlayIn::chat(format!("spam {i}")))).collect();
    assert!(sim.step(inbox));
    assert!(disconnected_with(&a, "disconnect.spam"));
}

#[test]
fn a_player_with_chat_hidden_gets_no_chat_and_is_told_so() {
    let (mut sim, mut a, mut b) = two_players(SimConfig::new(4, 2, None));
    let hidden = kiln_proto::packets::ClientInfo { chat_visibility: 2, ..Default::default() };
    send(&mut sim, 2, PlayIn::ClientInformation(hidden));
    send(&mut sim, 1, PlayIn::chat("anyone there"));
    run(&mut sim, &mut [&mut a, &mut b], 2, None);
    assert_eq!(chats(&a).len(), 1);
    assert!(chats(&b).is_empty(), "Bob has chat hidden");
    send(&mut sim, 2, PlayIn::chat("can I?"));
    run(&mut sim, &mut [&mut a, &mut b], 2, None);
    assert!(contains_text(&b, SYSTEM_CHAT, "chat.disabled.options"), "told his chat is off");
    assert_eq!(chats(&a).len(), 1, "and nothing was sent on");
}

#[test]
fn enforced_secure_chat_refuses_unsigned_messages() {
    let mut config = SimConfig::new(4, 2, None);
    config.server.enforce_secure_profile = true;
    let (mut sim, mut a, mut b) = two_players(config);
    send(&mut sim, 1, PlayIn::chat("unsigned"));
    run(&mut sim, &mut [&mut a, &mut b], 2, None);
    assert!(chats(&b).is_empty());
    assert!(contains_text(&a, SYSTEM_CHAT, "chat.disabled.missingProfileKey"), "the sender is told");
}

// ---- RCON ----

#[test]
fn an_rcon_command_runs_at_level_four_and_returns_what_it_printed() {
    let (mut sim, _a, _b) = two_players(SimConfig::new(4, 2, None));
    let ask = |sim: &mut Sim, command: &str| {
        let (tx, rx) = std::sync::mpsc::channel();
        let command = command.to_owned();
        let work: Box<dyn FnOnce(&mut dyn kiln_link::AdminHost) + Send> = Box::new(move |h| {
            let _ = tx.send(h.rcon_command(&command));
        });
        assert!(sim.step([ToSim::Admin(work)]));
        rx.try_recv().expect("answered in the same tick")
    };
    let list = ask(&mut sim, "list");
    assert!(list.contains("Alice") && list.contains("Bob"), "{list}");
    assert!(ask(&mut sim, "/list").contains("Alice"), "a leading slash is allowed");
    assert!(ask(&mut sim, "nonsense").contains("nonsense"), "the error text, with where it went wrong");
    assert_eq!(ask(&mut sim, "gamerule minecraft:keep_inventory true").is_empty(), false);
    // The command's feedback is not left behind for the next command.
    assert!(!ask(&mut sim, "difficulty").contains("keep_inventory"));
}

#[test]
fn the_legacy_announce_key_sets_the_advancement_message_rule() {
    let mut config = SimConfig::new(4, 2, None);
    config.server.announce_advancements = Some(false);
    let mut sim = Sim::new(config);
    let (tx, rx) = std::sync::mpsc::channel();
    let work: Box<dyn FnOnce(&mut dyn kiln_link::AdminHost) + Send> = Box::new(move |h| {
        let _ = tx.send(h.rcon_command("gamerule minecraft:show_advancement_messages"));
    });
    assert!(sim.step([ToSim::Admin(work)]));
    assert!(rx.try_recv().unwrap().contains("false"));
}
