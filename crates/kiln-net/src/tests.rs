//! Login state machine over loopback TCP: Velocity and BungeeCord forwarding, and online mode
//! with encryption against a mock session server. The client side reuses `Conn`.

use super::*;
use crate::auth::tests::{dead_url, hex, test_key, vanilla_vector};
use crate::proxy::tests::{SECRET, velocity_payload};
use bytes::BufMut;
use kiln_link::Property;
use kiln_proto::WriteExt;
use rand_core::OsRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

const NOTCH: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";

fn shared(login: LoginConfig, compression: Option<usize>) -> (Arc<Shared>, crossbeam_channel::Receiver<ToSim>) {
    let config = Config {
        bind: ([127, 0, 0, 1], 0).into(),
        motd: "test".into(),
        max_players: 10,
        view_distance: 8,
        simulation_distance: 8,
        compression_threshold: compression,
    };
    let (to_sim, sim_rx) = crossbeam_channel::unbounded();
    (Arc::new(Shared::with_login(config, login, to_sim)), sim_rx)
}

/// Accepts one connection and reports how `handle` ended.
async fn serve(shared: Arc<Shared>) -> (SocketAddr, tokio::sync::oneshot::Receiver<Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let _ = done_tx.send(handle(stream, peer, shared).await);
    });
    (addr, done_rx)
}

fn packet(id: i32, body: impl FnOnce(&mut BytesMut)) -> BytesMut {
    let mut b = BytesMut::new();
    b.put_varint(id);
    body(&mut b);
    b
}

async fn connect(addr: SocketAddr, host: &str, name: &str) -> Conn {
    let mut c = Conn::new(TcpStream::connect(addr).await.unwrap());
    c.queue(&packet(ids::handshake::serverbound::INTENTION, |b| {
        b.put_varint(version::PROTOCOL);
        b.put_string(host);
        b.put_u16(25565);
        b.put_varint(2);
    }))
    .unwrap();
    c.queue(&packet(ids::login::serverbound::HELLO, |b| {
        b.put_string(name);
        b.put_uuid(Uuid::nil());
    }))
    .unwrap();
    c.flush().await.unwrap();
    c
}

async fn recv(c: &mut Conn) -> (i32, Vec<u8>) {
    let pkt = c.read().await.unwrap();
    let (id, mut r) = split_id(&pkt).unwrap();
    (id, r.rest().to_vec())
}

fn disconnect_reason(id: i32, body: &[u8]) -> String {
    assert_eq!(id, ids::login::clientbound::LOGIN_DISCONNECT, "expected a login disconnect");
    Reader::new(body).string(262144).unwrap().to_owned()
}

/// Reads Login Finished and acknowledges it; returns the profile.
async fn finish_login(c: &mut Conn) -> GameProfile {
    let (mut id, mut body) = recv(c).await;
    if id == ids::login::clientbound::LOGIN_COMPRESSION {
        let t = Reader::new(&body).varint().unwrap() as usize;
        c.rx.set_threshold(Some(t));
        c.tx.set_threshold(Some(t));
        (id, body) = recv(c).await;
    }
    assert_eq!(id, ids::login::clientbound::LOGIN_FINISHED, "expected login finished");
    let mut r = Reader::new(&body);
    let uuid = r.uuid().unwrap();
    let name = r.string(16).unwrap().to_owned();
    let properties = profile::read_properties(&mut r).unwrap();
    let _session = r.uuid().unwrap();
    r.finish().unwrap();
    c.send(&packet(ids::login::serverbound::LOGIN_ACKNOWLEDGED, |_| {})).await.unwrap();
    GameProfile { uuid, name, properties }
}

/// Runs the configuration phase to its end as a client would.
async fn configure_client(c: &mut Conn) {
    use ids::configuration::{clientbound as cb, serverbound as sb};
    loop {
        let (id, body) = recv(c).await;
        if id == cb::SELECT_KNOWN_PACKS {
            c.send(&packet(sb::SELECT_KNOWN_PACKS, |b| b.put_slice(&body))).await.unwrap();
        } else if id == cb::FINISH_CONFIGURATION {
            c.send(&packet(sb::FINISH_CONFIGURATION, |_| {})).await.unwrap();
            return;
        }
    }
}

async fn joined(sim: &crossbeam_channel::Receiver<ToSim>) -> JoinInfo {
    for _ in 0..500 {
        match sim.try_recv() {
            Ok(ToSim::Join(j)) => return j,
            Ok(_) => {}
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    panic!("no join reached the simulation");
}

fn velocity() -> LoginConfig {
    LoginConfig { proxy: ProxyMode::Velocity { secret: SECRET.to_vec() }, ..Default::default() }
}

/// Reads the Velocity query and answers it with `payload`; returns the requested version.
async fn answer_velocity(c: &mut Conn, payload: Option<&[u8]>) -> Vec<u8> {
    let (id, body) = recv(c).await;
    assert_eq!(id, ids::login::clientbound::CUSTOM_QUERY);
    let mut r = Reader::new(&body);
    let transaction = r.varint().unwrap();
    assert_eq!(r.string(32767).unwrap(), proxy::VELOCITY_CHANNEL);
    let request = r.rest().to_vec();
    c.send(&packet(ids::login::serverbound::CUSTOM_QUERY_ANSWER, |b| {
        b.put_varint(transaction);
        b.put_bool(payload.is_some());
        if let Some(p) = payload {
            b.put_slice(p);
        }
    }))
    .await
    .unwrap();
    request
}

#[tokio::test]
async fn velocity_forwarded_identity_reaches_the_simulation() {
    let (shared, sim) = shared(velocity(), Some(256));
    let (addr, _done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    let request = answer_velocity(&mut c, Some(&velocity_payload(SECRET, 4, "203.0.113.7", |_| {}))).await;
    assert_eq!(request, [proxy::VELOCITY_MAX_VERSION]);

    // Proxy threshold defaults to -1: no Set Compression before Login Finished.
    let (id, body) = recv(&mut c).await;
    assert_eq!(id, ids::login::clientbound::LOGIN_FINISHED);
    assert_eq!(Reader::new(&body).uuid().unwrap().to_string(), NOTCH);
    c.send(&packet(ids::login::serverbound::LOGIN_ACKNOWLEDGED, |_| {})).await.unwrap();
    configure_client(&mut c).await;

    let j = joined(&sim).await;
    assert_eq!((j.name.as_str(), j.uuid.to_string().as_str()), ("Notch", NOTCH));
    assert_eq!(j.properties.len(), 1);
    assert_eq!(j.properties[0].name, "textures");
    assert_eq!(j.properties[0].signature.as_deref(), Some("c2lnbmF0dXJl"));
}

#[tokio::test]
async fn velocity_echoes_properties_in_login_finished() {
    let (shared, _sim) = shared(velocity(), None);
    let (addr, _done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    answer_velocity(&mut c, Some(&velocity_payload(SECRET, 4, "127.0.0.1", |_| {}))).await;
    let profile = finish_login(&mut c).await;
    assert_eq!(profile.uuid.to_string(), NOTCH);
    assert_eq!(
        profile.properties,
        vec![Property {
            name: "textures".into(),
            value: "eyJ0ZXh0dXJlcyI6e319".into(),
            signature: Some("c2lnbmF0dXJl".into())
        }]
    );
}

#[tokio::test]
async fn velocity_rejects_direct_clients() {
    // A vanilla client answers an unknown query without a payload.
    let (shared, _sim) = shared(velocity(), None);
    let (addr, done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    answer_velocity(&mut c, None).await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("connect with Velocity"));
    assert!(done.await.unwrap().is_err());
}

#[tokio::test]
async fn velocity_rejects_a_forged_answer() {
    let (shared, _sim) = shared(velocity(), None);
    let (addr, done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    answer_velocity(&mut c, Some(&velocity_payload(b"guessed-secret", 4, "127.0.0.1", |_| {}))).await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("Unable to verify"));
    let err = done.await.unwrap().unwrap_err();
    assert!(format!("{err:#}").contains("signature"), "{err:#}");
}

#[tokio::test]
async fn bungeecord_forwarding_with_bungeeguard() {
    let login = LoginConfig { proxy: ProxyMode::BungeeCord { tokens: vec!["s3cret".into()] }, ..Default::default() };
    let (shared, sim) = shared(login, None);
    let (addr, _done) = serve(shared).await;
    let props =
        r#"[{"name":"textures","value":"e30=","signature":"c2ln"},{"name":"bungeeguard-token","value":"s3cret"}]"#;
    // Longer than vanilla's 255-character host limit.
    let host = format!("play.example.com?_id=lobby\0198.51.100.4\0{}\0{props}", NOTCH.replace('-', ""));
    let host = format!("{host}{}", " ".repeat(300));
    let mut c = connect(addr, &host, "Notch").await;
    let profile = finish_login(&mut c).await;
    assert_eq!((profile.name.as_str(), profile.uuid.to_string().as_str()), ("Notch", NOTCH));
    assert_eq!(profile.properties.len(), 1, "the BungeeGuard token must not be forwarded");
    configure_client(&mut c).await;
    assert_eq!(joined(&sim).await.properties[0].value, "e30=");
}

#[tokio::test]
async fn bungeecord_rejects_unforwarded_and_unguarded_logins() {
    let login = || LoginConfig { proxy: ProxyMode::BungeeCord { tokens: vec!["s3cret".into()] }, ..Default::default() };
    let (shared1, _sim1) = shared(login(), None);
    let (addr, _done) = serve(shared1).await;
    let mut c = connect(addr, "play.example.com", "Notch").await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("through its proxy"));

    let (shared2, _sim2) = shared(login(), None);
    let (addr, _done) = serve(shared2).await;
    let host = format!("h\0127.0.0.1\0{NOTCH}\0[]");
    let mut c = connect(addr, &host, "Notch").await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("Unable to verify"));
}

/// Serves one HTTP request with `status` and `body`; returns the endpoint URL and the request line.
fn mock_session(status: &'static str, body: &'static str) -> (String, Arc<Mutex<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/session/minecraft/hasJoined", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(String::new()));
    let record = seen.clone();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut req = Vec::new();
        let mut buf = [0; 1024];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&buf[..n]);
        }
        let text = String::from_utf8_lossy(&req);
        *record.lock().unwrap() = text.lines().next().unwrap_or_default().to_owned();
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(resp.as_bytes()).unwrap();
    });
    (url, seen)
}

fn online(url: &str, compression: Option<usize>) -> (Arc<Shared>, crossbeam_channel::Receiver<ToSim>) {
    let (to_sim, sim_rx) = crossbeam_channel::unbounded();
    let config = Config {
        bind: ([127, 0, 0, 1], 0).into(),
        motd: "test".into(),
        max_players: 10,
        view_distance: 8,
        simulation_distance: 8,
        compression_threshold: compression,
    };
    let mut shared = Shared::with_login(config, LoginConfig::default(), to_sim);
    shared.auth = Some(Authenticator { key: test_key(), session: Arc::new(SessionService::with_endpoint(url)) });
    (Arc::new(shared), sim_rx)
}

/// Plays the client half of the encryption handshake; returns the server hash the client
/// would send to the session server's `join`.
async fn client_encrypt(c: &mut Conn) -> String {
    let (id, body) = recv(c).await;
    assert_eq!(id, ids::login::clientbound::HELLO, "expected an encryption request");
    let mut r = Reader::new(&body);
    let server_id = r.string(20).unwrap().to_owned();
    let n = r.len().unwrap();
    let der = r.bytes(n).unwrap().to_vec();
    let n = r.len().unwrap();
    let challenge = r.bytes(n).unwrap().to_vec();
    assert!(r.bool().unwrap(), "should authenticate");
    r.finish().unwrap();

    let secret = [0x3c; 16];
    let public = RsaPublicKey::from_public_key_der(&der).unwrap();
    let enc_secret = public.encrypt(&mut OsRng, Pkcs1v15Encrypt, &secret).unwrap();
    let enc_challenge = public.encrypt(&mut OsRng, Pkcs1v15Encrypt, &challenge).unwrap();
    c.send(&packet(ids::login::serverbound::KEY, |b| {
        b.put_varint(enc_secret.len() as i32);
        b.put_slice(&enc_secret);
        b.put_varint(enc_challenge.len() as i32);
        b.put_slice(&enc_challenge);
    }))
    .await
    .unwrap();
    c.enable_encryption(&secret);
    auth::server_hash(&server_id, &secret, &der)
}

const PROFILE: &str = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{"name":"textures","value":"e30=","signature":"c2ln"}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn online_mode_encrypts_and_authenticates() {
    let (url, seen) = mock_session("200 OK", PROFILE);
    let (shared, sim) = online(&url, Some(64));
    let (addr, _done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    let hash = client_encrypt(&mut c).await;

    // Compression and Login Finished arrive encrypted; the registries after them are
    // compressed and encrypted, so a full configuration phase exercises both layers.
    let profile = finish_login(&mut c).await;
    assert_eq!(profile.uuid.to_string(), NOTCH);
    assert_eq!(profile.properties[0].signature.as_deref(), Some("c2ln"));
    assert_eq!(c.rx.threshold(), Some(64));
    configure_client(&mut c).await;
    let j = joined(&sim).await;
    assert_eq!(j.uuid.to_string(), NOTCH);
    assert_eq!(j.properties.len(), 1);

    let request = seen.lock().unwrap().clone();
    let query: HashMap<&str, &str> = request
        .split_whitespace()
        .nth(1)
        .and_then(|target| target.split_once('?'))
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).collect())
        .unwrap_or_default();
    assert_eq!(query.get("username"), Some(&"Notch"), "{request}");
    assert_eq!(query.get("serverId"), Some(&hash.as_str()), "{request}");
    assert!(!query.contains_key("ip"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn online_mode_rejects_unverified_players() {
    // The session server answers 204 when the client never called `join`.
    let (url, _seen) = mock_session("204 No Content", "");
    let (shared, _sim) = online(&url, None);
    let (addr, done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    client_encrypt(&mut c).await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("multiplayer.disconnect.unverified_username"));
    assert!(done.await.unwrap().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn online_mode_rejects_a_profile_for_another_name() {
    let (url, _seen) = mock_session("200 OK", PROFILE);
    let (shared, _sim) = online(&url, None);
    let (addr, _done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "jeb_").await;
    client_encrypt(&mut c).await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("unverified_username"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn online_mode_reports_unreachable_session_server() {
    let (shared, _sim) = online(&dead_url(), None);
    let (addr, _done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    client_encrypt(&mut c).await;
    let (id, body) = recv(&mut c).await;
    assert!(disconnect_reason(id, &body).contains("authservers_down"));
}

#[tokio::test]
async fn online_mode_rejects_a_wrong_challenge() {
    let (shared, _sim) = online(&dead_url(), None);
    let (addr, done) = serve(shared).await;
    let mut c = connect(addr, "127.0.0.1", "Notch").await;
    let (id, body) = recv(&mut c).await;
    assert_eq!(id, ids::login::clientbound::HELLO);
    let mut r = Reader::new(&body);
    r.string(20).unwrap();
    let n = r.len().unwrap();
    let public = RsaPublicKey::from_public_key_der(r.bytes(n).unwrap()).unwrap();
    let enc = |data: &[u8]| public.encrypt(&mut OsRng, Pkcs1v15Encrypt, data).unwrap();
    let (secret, challenge) = (enc(&[1; 16]), enc(b"nope"));
    c.send(&packet(ids::login::serverbound::KEY, |b| {
        b.put_varint(secret.len() as i32);
        b.put_slice(&secret);
        b.put_varint(challenge.len() as i32);
        b.put_slice(&challenge);
    }))
    .await
    .unwrap();
    let err = done.await.unwrap().unwrap_err();
    assert!(format!("{err:#}").contains("invalid encryption response"), "{err:#}");
}

#[test]
fn login_config_from_vars() {
    let vars = |pairs: &[(&str, &str)]| {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        LoginConfig::from_vars(move |k| map.get(k).cloned())
    };
    let c = vars(&[]).unwrap();
    assert!(!c.online_mode && matches!(c.proxy, ProxyMode::None) && c.proxy_compression_threshold.is_none());

    let c = vars(&[("KILN_PROXY", "velocity"), ("KILN_VELOCITY_SECRET", " abc\n")]).unwrap();
    assert!(matches!(c.proxy, ProxyMode::Velocity { ref secret } if secret == b"abc"));
    assert!(vars(&[("KILN_PROXY", "velocity")]).is_err());
    assert!(vars(&[("KILN_PROXY", "velocity"), ("KILN_VELOCITY_SECRET", "  ")]).is_err());

    let c = vars(&[("KILN_PROXY", "bungeecord"), ("KILN_BUNGEEGUARD_TOKENS", "a, b,,")]).unwrap();
    assert!(matches!(c.proxy, ProxyMode::BungeeCord { ref tokens } if tokens == &["a", "b"]));

    let c = vars(&[("KILN_ONLINE_MODE", "true"), ("KILN_PROXY_COMPRESSION_THRESHOLD", "512")]).unwrap();
    assert!(c.online_mode);
    assert_eq!(c.proxy_compression_threshold, Some(512));
    assert_eq!(vars(&[("KILN_PROXY_COMPRESSION_THRESHOLD", "-1")]).unwrap().proxy_compression_threshold, None);
    assert!(vars(&[("KILN_ONLINE_MODE", "maybe")]).is_err());
    assert!(vars(&[("KILN_PROXY", "waterfall")]).is_err());
}

#[test]
fn decodes_vanilla_encoded_login_packets() {
    let answer = hex(vanilla_vector("query_answer"));
    let (id, payload) = login_ext::read_custom_query_answer(&mut Reader::new(&answer)).unwrap();
    assert_eq!((id, payload), (300, Some(&[0xAA, 0xBB][..])));
    let empty = hex(vanilla_vector("query_answer_empty"));
    assert_eq!(login_ext::read_custom_query_answer(&mut Reader::new(&empty)).unwrap(), (300, None));
    let start = hex(vanilla_vector("login_start"));
    let start = login_ext::read_login_start(&mut Reader::new(&start)).unwrap();
    assert_eq!((start.name, start.uuid.to_string().as_str()), ("Notch", NOTCH));
}

#[test]
fn cipher_sits_below_framing() {
    // Frames encoded then encrypted in odd-sized chunks decode after decryption in other chunks.
    let (mut enc, _) = cipher::pair(&[9; 16]);
    let (_, mut dec) = cipher::pair(&[9; 16]);
    let mut tx = FrameCodec::new();
    tx.set_threshold(Some(64));
    let mut wire = BytesMut::new();
    let packets: Vec<Vec<u8>> = vec![vec![1, 2, 3], vec![0x42; 5000], vec![7; 64]];
    for p in &packets {
        tx.encode(p, &mut wire).unwrap();
    }
    for chunk in wire.chunks_mut(37) {
        enc.apply(chunk);
    }
    let mut rx = FrameCodec::new();
    rx.set_threshold(Some(64));
    let mut rbuf = BytesMut::new();
    let mut got = Vec::new();
    for chunk in wire.chunks(101) {
        let start = rbuf.len();
        rbuf.put_slice(chunk);
        dec.apply(&mut rbuf[start..]);
        while let Some(p) = rx.decode(&mut rbuf).unwrap() {
            got.push(p.to_vec());
        }
    }
    assert_eq!(got, packets);
}

#[test]
fn authenticates_only_without_a_proxy() {
    let online = |proxy| LoginConfig { online_mode: true, proxy, ..Default::default() };
    assert!(!shared(LoginConfig::default(), None).0.authenticates());
    assert!(!shared(online(ProxyMode::Velocity { secret: SECRET.to_vec() }), None).0.authenticates());
    assert!(shared(online(ProxyMode::None), None).0.authenticates());
}

/// Writes the clientbound login packets Kiln builds (bodies without the packet id) for
/// `tools/vanilla_decode.py`: `KILN_DUMP_DIR=work/wp-proxy cargo test -p kiln-net -- --ignored`.
#[test]
#[ignore]
fn dump_login_packets() {
    let dir = std::path::PathBuf::from(std::env::var("KILN_DUMP_DIR").expect("KILN_DUMP_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let key = test_key();
    let profile = GameProfile {
        uuid: Uuid::parse_str(NOTCH).unwrap(),
        name: "Notch".into(),
        properties: vec![
            Property { name: "textures".into(), value: "e30=".into(), signature: Some("c2ln".into()) },
            Property { name: "unsigned".into(), value: "x".into(), signature: None },
        ],
    };
    let session = Uuid::from_u128(1);
    let packets = [
        ("encryption_request.bin", login_ext::encryption_request("", key.public_der(), &[1, 2, 3, 4], true)),
        ("custom_query.bin", login_ext::custom_query(12345, proxy::VELOCITY_CHANNEL, &proxy::velocity_request())),
        ("login_disconnect.bin", login_ext::login_disconnect_translated("multiplayer.disconnect.unverified_username")),
        (
            "login_finished.bin",
            packets::login_finished(profile.uuid, &profile.name, &profile.wire_properties(), session),
        ),
    ];
    for (file, pkt) in packets {
        let (_, mut r) = split_id(&pkt).unwrap();
        std::fs::write(dir.join(file), r.rest()).unwrap();
    }
}
