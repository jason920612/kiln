//! Runs bots against a scripted server built from kiln-proto's server-side codec and checks
//! both what the server receives and what the bots report.

use bytes::BytesMut;
use kiln_bot::{Behavior, Config};
use kiln_data::packets as ids;
use kiln_proto::packets::{self as pk, PlayIn};
use kiln_proto::{FrameCodec, Reader};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SPAWN: [f64; 3] = [8.5, -60.0, 8.5];
const TELEPORT_ID: i32 = 5;

struct Conn {
    stream: TcpStream,
    rbuf: BytesMut,
    rx: FrameCodec,
    tx: FrameCodec,
}

impl Conn {
    async fn recv(&mut self) -> Option<(i32, Vec<u8>)> {
        loop {
            if let Some(p) = self.rx.decode(&mut self.rbuf).unwrap() {
                let mut r = Reader::new(&p);
                let id = r.varint().unwrap();
                return Some((id, r.rest().to_vec()));
            }
            match self.stream.read_buf(&mut self.rbuf).await {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    async fn expect(&mut self, id: i32) -> Vec<u8> {
        let (got, body) = self.recv().await.expect("bot hung up");
        assert_eq!(got, id, "unexpected packet");
        body
    }

    async fn send(&mut self, packet: &[u8]) {
        let mut out = BytesMut::new();
        self.tx.encode(packet, &mut out).unwrap();
        self.stream.write_all(&out).await.unwrap();
    }

    /// Closes like a server that sent a disconnect: our side shuts down, then whatever the
    /// client still sends is read until it closes. Dropping a socket with unread input makes
    /// Windows reset the connection, which can reach the client before the disconnect packet.
    async fn close(&mut self) {
        use tokio::io::AsyncReadExt;
        let _ = self.stream.shutdown().await;
        let mut buf = [0u8; 4096];
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while matches!(self.stream.read(&mut buf).await, Ok(n) if n > 0) {}
        })
        .await;
    }

    fn compress(&mut self, threshold: usize) {
        self.rx.set_threshold(Some(threshold));
        self.tx.set_threshold(Some(threshold));
    }
}

/// What the server saw from one bot that reached play.
#[derive(Debug, Default)]
struct Seen {
    name: String,
    view_distance: u8,
    brand: bool,
    config_keep_alive: Option<i64>,
    teleport_echo: Option<(i32, [f64; 3], f32, f32)>,
    positions: Vec<[f64; 3]>,
    tick_ends: usize,
    batch_acks: usize,
    loaded_after_batch: bool,
    moves_before_loaded: usize,
    /// Position packets beyond the first between two Client Tick Ends (vanilla kicks for these).
    extra_positions_per_tick: usize,
    play_keep_alive: Option<i64>,
    chats: Vec<String>,
}

async fn serve(stream: TcpStream, port: u16, seen: Arc<Mutex<Vec<Seen>>>) {
    let mut c = Conn { stream, rbuf: BytesMut::new(), rx: FrameCodec::new(), tx: FrameCodec::new() };
    let mut s = Seen::default();

    let body = c.expect(ids::handshake::serverbound::INTENTION).await;
    let mut r = Reader::new(&body);
    assert_eq!(r.varint().unwrap(), kiln_data::version::PROTOCOL);
    assert_eq!(r.string(255).unwrap(), "127.0.0.1");
    assert_eq!(r.u16().unwrap(), port);
    assert_eq!(r.varint().unwrap(), 2, "login intent");
    let body = c.expect(ids::login::serverbound::HELLO).await;
    let mut r = Reader::new(&body);
    s.name = r.string(16).unwrap().to_owned();
    let uuid = r.uuid().unwrap();
    r.finish().unwrap();

    if s.name.ends_with('2') {
        c.send(&pk::login_disconnect("The server is full.")).await;
        c.close().await;
        return;
    }
    c.send(&pk::login_compression(256)).await;
    c.compress(256);
    c.send(&pk::login_finished(uuid, &s.name, &[], uuid)).await;
    c.expect(ids::login::serverbound::LOGIN_ACKNOWLEDGED).await;

    use ids::configuration::serverbound as csb;
    let brand = c.expect(csb::CUSTOM_PAYLOAD).await;
    s.brand = Reader::new(&brand).string(32767).unwrap() == "minecraft:brand";
    let info = c.expect(csb::CLIENT_INFORMATION).await;
    s.view_distance = pk::read_client_information(&mut Reader::new(&info)).unwrap().view_distance;

    let offer = pk::select_known_packs(&[("minecraft", "core", "26.3"), ("minecraft", "extra", "1")]);
    c.send(&offer).await;
    assert_eq!(c.expect(csb::SELECT_KNOWN_PACKS).await, offer[1..], "known packs echoed");
    // Large enough to be compressed: the bot only reads its id.
    c.send(&pk::update_tags(ids::configuration::clientbound::UPDATE_TAGS, kiln_data::registries::TAGS)).await;
    let mut ka = BytesMut::new();
    kiln_proto::WriteExt::put_varint(&mut ka, ids::configuration::clientbound::KEEP_ALIVE);
    bytes::BufMut::put_i64(&mut ka, 123);
    c.send(&ka).await;
    s.config_keep_alive = Some(Reader::new(&c.expect(csb::KEEP_ALIVE).await).i64().unwrap());
    c.send(&pk::finish_configuration()).await;
    c.expect(csb::FINISH_CONFIGURATION).await;

    let login = pk::Login {
        entity_id: 1,
        dimensions: &["minecraft:overworld"],
        max_players: 10,
        view_distance: 2,
        simulation_distance: 2,
        dimension_type: 0,
        dimension: "minecraft:overworld",
        game_mode: 1,
        is_flat: true,
        sea_level: 63,
        online_mode: false,
        hashed_seed: 0,
        hardcore: false,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
    };
    c.send(&pk::play_login(&login)).await;
    c.send(&pk::player_position(TELEPORT_ID, SPAWN, 0.0, 0.0)).await;
    c.send(&pk::chunk_batch_start()).await;
    c.send(&pk::level_chunk_with_light(0, 0, &[0u8; 5000])).await;
    c.send(&pk::chunk_batch_finished(1)).await;
    c.send(&pk::keep_alive(77)).await;

    let kick_at = s.name.ends_with('1').then(|| Instant::now() + Duration::from_millis(1500));
    let mut positions_this_tick = 0;
    loop {
        let timeout = kick_at.map_or(Duration::from_secs(30), |t| t.saturating_duration_since(Instant::now()));
        let Ok(next) = tokio::time::timeout(timeout, c.recv()).await else {
            c.send(&pk::play_disconnect("Bye")).await;
            c.close().await;
            break;
        };
        let Some((id, body)) = next else { break };
        if id == ids::play::serverbound::CLIENT_TICK_END {
            s.tick_ends += 1;
            positions_this_tick = 0;
            continue;
        }
        if id == ids::play::serverbound::ACCEPT_TELEPORTATION {
            let mut r = Reader::new(&body);
            let echo = (
                r.varint().unwrap(),
                [r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()],
                r.f32().unwrap(),
                r.f32().unwrap(),
            );
            r.finish().unwrap();
            s.teleport_echo = Some(echo);
            continue;
        }
        match pk::decode_play(id, &mut Reader::new(&body)).unwrap() {
            Some(PlayIn::Move { pos, .. }) => {
                if !s.loaded_after_batch {
                    s.moves_before_loaded += 1;
                }
                if let Some(p) = pos {
                    s.positions.push(p);
                    positions_this_tick += 1;
                    s.extra_positions_per_tick += usize::from(positions_this_tick > 1);
                }
            }
            Some(PlayIn::ChunkBatchReceived { .. }) => s.batch_acks += 1,
            Some(PlayIn::PlayerLoaded) => s.loaded_after_batch = s.batch_acks == 1 && s.teleport_echo.is_some(),
            Some(PlayIn::KeepAlive { id }) => s.play_keep_alive = Some(id),
            Some(PlayIn::Chat { message }) => s.chats.push(message),
            _ => {}
        }
    }
    seen.lock().unwrap().push(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bots_play_the_protocol_and_report_outcomes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let server_seen = seen.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(serve(stream, port, server_seen.clone()));
        }
    });

    let config = Config {
        addr: format!("127.0.0.1:{port}"),
        count: 3,
        rate: 100.0,
        behavior: Behavior::Walk,
        duration: Duration::from_secs(3),
        view_distance: 3,
        name_prefix: "Test".into(),
        chat_interval: Some(Duration::from_millis(500)),
        ..Config::default()
    };
    let report = kiln_bot::run(config).await.unwrap();

    assert_eq!(report.launched, 3);
    assert_eq!((report.joined, report.failed, report.dropped, report.online), (2, 1, 1, 1), "{report}");
    assert_eq!(
        report.disconnect_reasons,
        [("login: The server is full.".to_string(), 1), ("play: Bye".to_string(), 1)]
    );
    assert_eq!(report.traffic.chunks, 2);
    assert!(report.join_ms_p50.is_some() && report.join_ms_p99.is_some());
    assert!(report.traffic.rx_packets > 0 && report.rx_bytes_per_sec_per_bot > 0.0);

    // The server side of the stayer is recorded once the bot hangs up at the end of the run.
    let deadline = Instant::now() + Duration::from_secs(5);
    while seen.lock().unwrap().len() < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for s in seen.iter() {
        assert_eq!(s.view_distance, 3);
        assert!(s.brand);
        assert_eq!(s.config_keep_alive, Some(123));
        assert_eq!(s.play_keep_alive, Some(77));
        assert_eq!(s.teleport_echo, Some((TELEPORT_ID, SPAWN, 0.0, 0.0)));
        assert_eq!(s.batch_acks, 1);
        assert!(s.loaded_after_batch, "{}: player_loaded after the teleport and the first batch ack", s.name);
        assert_eq!(s.moves_before_loaded, 0, "vanilla ignores movement before Player Loaded");
        assert_eq!(s.extra_positions_per_tick, 0);
        // After the confirmation, one step per tick on flat ground, starting at the spawn.
        assert!(s.positions.len() >= 15, "{}: {} moves", s.name, s.positions.len());
        let first_step = (s.positions[0][0] - SPAWN[0]).hypot(s.positions[0][2] - SPAWN[2]);
        assert!((first_step - kiln_bot::behavior::WALK_SPEED / 20.0).abs() < 1e-6);
        for w in s.positions.windows(2) {
            assert_eq!(w[1][1], SPAWN[1]);
            let step = (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]);
            assert!((step - kiln_bot::behavior::WALK_SPEED / 20.0).abs() < 1e-6, "step {step}");
        }
        assert!(s.tick_ends + 2 >= s.positions.len(), "a tick end per tick");
        assert!(!s.chats.is_empty() && s.chats[0].starts_with("hello #1 from Test"));
    }
    assert_eq!(report.traffic.chat_sent as usize, seen.iter().map(|s| s.chats.len()).sum::<usize>());
}
