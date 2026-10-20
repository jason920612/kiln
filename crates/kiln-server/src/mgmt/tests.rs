//! Tests of the management server: the JSON and codec layers, the protocol over a real socket against
//! a fake simulation, authentication, notifications, settings, and TLS with a keystore made by `keytool`.

use super::*;
use kiln_link::access::{AccessLists, NameAndId};
use kiln_link::mgmt::{MgmtText, Notification, OnlinePlayer, RuleValue, Setting, SettingValue};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicUsize;

const SECRET: &str = "abcdefghijABCDEFGHIJ0123456789klmnopqrst";

// ---- json ----

#[test]
fn json_keeps_member_order_and_number_text() {
    let j = json::parse(r#"{"b":1.50,"a":[true,null,"xé\n"],"b":2e3}"#).unwrap();
    // A repeated name keeps its place and takes the later value.
    assert_eq!(j.to_string(), "{\"b\":2e3,\"a\":[true,null,\"xé\\n\"]}");
}

#[test]
fn json_does_not_escape_html_characters_but_escapes_line_separators() {
    let j = json::J::str("a<b>&'c\u{2028}");
    assert_eq!(j.to_string(), "\"a<b>&'c\\u2028\"");
}

#[test]
fn json_parse_errors_read_like_gson() {
    assert_eq!(
        json::parse("not json").unwrap_err().0,
        "com.google.gson.JsonSyntaxException: com.google.gson.stream.MalformedJsonException: Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON at line 1 column 6 path $\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json"
    );
    assert_eq!(
        json::parse(r#"{"jsonrpc":"2.0","id":1,"method":"#).unwrap_err().0,
        "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line 1 column 34 path $.method"
    );
}

// ---- codec ----

#[test]
fn numbers_are_cut_to_ints_as_java_does() {
    for (text, want) in [
        ("5", 5),
        ("2.5", 2),
        ("-2.5", -2),
        ("1e1", 10),
        ("1.5e3", 1500),
        ("3000000000", -1_294_967_296),
        ("2147483648", i32::MIN),
        ("-2147483649", i32::MAX),
        ("1e20", 1_661_992_960),
        ("12345678901234567890123", 1_900_168_395),
    ] {
        assert_eq!(codec::int_value(text), want, "{text}");
    }
}

#[test]
fn uuids_fail_with_the_texts_of_the_jdk() {
    let id = codec::uuid_string("00000000-0000-0000-0000-000000000007").unwrap();
    assert_eq!(id.to_string(), "00000000-0000-0000-0000-000000000007");
    assert_eq!(codec::uuid_string("1-2-3-4-5").unwrap(), Uuid::from_u64_pair(0x0000_0001_0002_0003, 0x0004_0000_0000_0005));
    assert!(codec::uuid_string("+1-2-3-4-5").is_ok());
    for (text, want) in [
        ("nope", "Invalid UUID nope: Invalid UUID string: nope"),
        ("", "Invalid UUID : Invalid UUID string: "),
        ("zzzzzzzz-0000-0000-0000-000000000000", "Invalid UUID zzzzzzzz-0000-0000-0000-000000000000: Error at index 0 in: \"zzzzzzzz\""),
        ("-1-2-3-4", "Invalid UUID -1-2-3-4: For input string: \"\" under radix 16"),
        ("fffffffffffffffff-2-3-4-5", "Invalid UUID fffffffffffffffff-2-3-4-5: Error at index 15 in: \"fffffffffffffffff\""),
        ("00000000-0000-0000-0000-0000000000000000000", "Invalid UUID 00000000-0000-0000-0000-0000000000000000000: UUID string too large"),
    ] {
        assert_eq!(codec::uuid_string(text).unwrap_err(), want);
    }
}

#[test]
fn instants_parse_and_print_as_iso_instant() {
    assert_eq!(codec::parse_instant("2030-01-01T00:00:00Z"), Ok((1_893_456_000, 0)));
    assert_eq!(codec::parse_instant("2030-01-01T00:00:00+01:00"), Ok((1_893_456_000 - 3600, 0)));
    assert_eq!(codec::instant_string(1_893_456_000, 123_000_000), "2030-01-01T00:00:00.123Z");
    assert_eq!(codec::instant_string(1_893_456_000, 0), "2030-01-01T00:00:00Z");
    assert_eq!(codec::parse_instant("+12345-01-01T00:00:00Z").map(|(s, n)| codec::instant_string(s, n)), Ok("+12345-01-01T00:00:00Z".to_owned()));
    for (text, want) in [
        ("bad", "Text 'bad' could not be parsed at index 0"),
        ("2030-01-01", "Text '2030-01-01' could not be parsed at index 10"),
        ("2030-01-01T00:00:00", "Text '2030-01-01T00:00:00' could not be parsed at index 19"),
        ("2030-01-01T00:00Z", "Text '2030-01-01T00:00Z' could not be parsed at index 16"),
        ("2030-13-01T00:00:00Z", "Text '2030-13-01T00:00:00Z' could not be parsed at index 0"),
        ("2030-02-30T00:00:00Z", "Text '2030-02-30T00:00:00Z' could not be parsed at index 0"),
        ("2030-01-01T25:00:00Z", "Text '2030-01-01T25:00:00Z' could not be parsed at index 0"),
        ("2030-01-01T00:00:00Zx", "Text '2030-01-01T00:00:00Zx' could not be parsed, unparsed text found at index 20"),
    ] {
        assert_eq!(codec::parse_instant(text).unwrap_err(), want);
    }
}

#[test]
fn record_errors_come_later_field_first() {
    let j = json::parse(r#"{"name":5,"id":6}"#).unwrap();
    assert_eq!(codec::player_dto(&j).unwrap_err(), "Not a string: 5; Not a string: 6");
    let j = json::parse(r#"[5,6]"#).unwrap();
    assert_eq!(codec::list(&j, codec::player_dto).unwrap_err(), "Not a JSON object: 6; Not a JSON object: 5");
}

// ---- the discovery document ----

#[test]
fn discover_lists_every_method_and_notification() {
    let doc = spec::discover();
    let methods = match doc.get("methods") {
        Some(json::J::Arr(m)) => m,
        _ => panic!("methods"),
    };
    assert_eq!(methods.len(), spec::INCOMING.len() + spec::OUTGOING.len());
    assert_eq!(spec::INCOMING.len(), 68);
    assert_eq!(spec::OUTGOING.len(), 21);
    let text = doc.to_string();
    assert!(text.starts_with("{\"openrpc\":\"1.3.2\",\"info\":{\"title\":\"Minecraft Server JSON-RPC\",\"version\":\"3.1.0\"},\"methods\":[{\"name\":\"minecraft:allowlist\",\"description\":\"Get the allowlist\",\"params\":[],\"result\":{\"name\":\"allowlist\",\"schema\":{\"type\":\"array\",\"items\":{\"$ref\":\"#/components/schemas/player\"}}}},"));
    assert!(text.contains("{\"name\":\"minecraft:notification/server/started\",\"description\":\"Server started\",\"params\":[]}"));
}

// ---- a fake simulation ----

struct Fake {
    access: SharedAccess,
    info: Arc<ServerInfo>,
    players: Vec<OnlinePlayer>,
    kicked: Vec<(Uuid, MgmtText)>,
    messages: Vec<(MgmtText, bool, Option<Vec<Uuid>>)>,
    settings: HashMap<String, SettingValue>,
    rules: Vec<(String, RuleValue, Option<(i32, i32)>)>,
    saves: Vec<bool>,
    halted: bool,
    kick_unlisted_calls: u32,
}

impl MgmtHost for Fake {
    fn access(&self) -> SharedAccess {
        self.access.clone()
    }

    fn online_players(&self) -> Vec<OnlinePlayer> {
        self.players.clone()
    }

    fn kick(&mut self, user: Uuid, reason: &MgmtText, remove: bool) {
        if let Some(i) = self.players.iter().position(|p| p.user.uuid == user) {
            self.kicked.push((user, reason.clone()));
            if remove {
                let gone = self.players.remove(i);
                self.info.roster.write().unwrap().retain(|e| e.uuid != user);
                self.info.notifier.notify(Notification::PlayerLeft(gone.user));
            }
        }
    }

    fn system_message(&mut self, text: &MgmtText, overlay: bool, to: Option<&[Uuid]>) {
        self.messages.push((text.clone(), overlay, to.map(<[Uuid]>::to_vec)));
    }

    fn op(&mut self, user: &NameAndId, level: Option<u8>, bypass: Option<bool>) {
        let mut a = self.access.write().unwrap();
        let bypass = bypass.unwrap_or_else(|| a.op_list.get(&user.uuid.to_string()).is_some_and(|e| e.bypasses_player_limit));
        a.op_add(kiln_link::access::OpEntry { user: user.clone(), level: level.unwrap_or(4), bypasses_player_limit: bypass });
    }

    fn deop(&mut self, user: &NameAndId) {
        self.access.write().unwrap().op_remove(user);
    }

    fn deop_all(&mut self) {
        self.access.write().unwrap().op_clear();
    }

    fn kick_unlisted(&mut self) {
        self.kick_unlisted_calls += 1;
    }

    fn setting(&self, setting: Setting) -> SettingValue {
        self.settings.get(&format!("{setting:?}")).cloned().unwrap_or(match setting {
            Setting::Autosave => SettingValue::Bool(true),
            Setting::Difficulty => SettingValue::Int(1),
            _ => SettingValue::Bool(false),
        })
    }

    fn set_setting(&mut self, setting: Setting, value: SettingValue) {
        self.settings.insert(format!("{setting:?}"), value);
    }

    fn game_rules(&self) -> Vec<(String, RuleValue, Option<(i32, i32)>)> {
        self.rules.clone()
    }

    fn game_rule(&self, key: &str) -> Option<(String, RuleValue, Option<(i32, i32)>)> {
        let id = if key.contains(':') { key.to_owned() } else { format!("minecraft:{key}") };
        self.rules.iter().find(|r| r.0 == id).cloned()
    }

    fn set_game_rule(&mut self, key: &str, value: RuleValue) {
        if let Some(r) = self.rules.iter_mut().find(|r| r.0 == key) {
            r.1 = value;
        }
        self.info.notifier.notify(Notification::GameRuleChanged { key: key.to_owned(), value });
    }

    fn save_everything(&mut self, flush: bool) -> bool {
        self.saves.push(flush);
        self.info.notifier.notify(Notification::ServerSaveStarted);
        self.info.notifier.notify(Notification::ServerSaveCompleted);
        true
    }

    fn plain(&self, text: &MgmtText) -> String {
        match text {
            MgmtText::Literal(s) => s.clone(),
            MgmtText::Translatable { key, .. } => key.clone(),
        }
    }

    fn halt(&mut self) {
        self.halted = true;
        self.info.notifier.notify(Notification::ServerShuttingDown);
    }
}

struct FakeRunner(Mutex<Fake>);

impl Runner for FakeRunner {
    fn run(&self, job: Box<dyn FnOnce(&mut dyn MgmtHost) + Send>) -> bool {
        job(&mut *self.0.lock().unwrap());
        true
    }
}

static DIRS: AtomicUsize = AtomicUsize::new(0);

struct Env {
    server: ManagementServer,
    fake: Arc<FakeRunner>,
    info: Arc<ServerInfo>,
    dir: PathBuf,
}

impl Env {
    fn start() -> Env {
        Env::start_with(|_| {})
    }

    fn start_with(tweak: impl FnOnce(&mut Config)) -> Env {
        let dir = std::env::temp_dir().join(format!("kiln-mgmt-{}-{}", std::process::id(), DIRS.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let props = ServerProperties::parse(crate::javaprops::JavaProps::parse("online-mode=false\n"), || SECRET.to_owned()).unwrap();
        let mut access = AccessLists::new(Some(dir.clone()));
        let info = Arc::new(ServerInfo::new("A Minecraft Server", 20, "world"));
        access.notifier = info.notifier.clone();
        let access = access.shared();
        let fake = Arc::new(FakeRunner(Mutex::new(Fake {
            access: access.clone(),
            info: info.clone(),
            players: Vec::new(),
            kicked: Vec::new(),
            messages: Vec::new(),
            settings: HashMap::new(),
            rules: vec![
                ("minecraft:keep_inventory".into(), RuleValue::Bool(false), None),
                ("minecraft:max_command_forks".into(), RuleValue::Int(65536), Some((0, i32::MAX))),
            ],
            saves: Vec::new(),
            halted: false,
            kick_unlisted_calls: 0,
        })));
        let mut config = Config { host: "127.0.0.1".into(), port: 0, secret: SECRET.into(), tls_enabled: false, tls_keystore: String::new(), tls_keystore_password: String::new(), allowed_origins: String::new(), heartbeat_interval: 0 };
        tweak(&mut config);
        let backend = Backend {
            runner: fake.clone(),
            info: info.clone(),
            access,
            settings: Arc::new(Settings { props: Mutex::new(props), path: Some(dir.join("server.properties")) }),
            lookup: None,
            online_mode: false,
        };
        let server = ManagementServer::start(&config, backend).expect("start");
        Env { server, fake, info, dir }
    }

    fn ready(&self) {
        self.info.ready.store(true, Ordering::Relaxed);
    }

    fn connect(&self) -> Client {
        Client::connect(self.server.local_addr(), &[("Authorization", &format!("Bearer {SECRET}"))]).expect("connect")
    }

    fn with_fake<T>(&self, f: impl FnOnce(&mut Fake) -> T) -> T {
        f(&mut self.fake.0.lock().unwrap())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---- a minimal WebSocket client ----

struct Client {
    sock: TcpStream,
    buf: Vec<u8>,
    pub head: String,
}

impl Client {
    fn connect(addr: SocketAddr, headers: &[(&str, &str)]) -> Result<Client, String> {
        let mut sock = TcpStream::connect(addr).map_err(|e| e.to_string())?;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut req = "GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n".to_owned();
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        sock.write_all(req.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let end = loop {
            if let Some(e) = ws::head_end(&buf) {
                break e;
            }
            let n = sock.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("closed".into());
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
        if !head.starts_with("HTTP/1.1 101") {
            let mut body = buf[end..].to_vec();
            while let Ok(n) = sock.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            return Err(format!("{head}{}", String::from_utf8_lossy(&body)));
        }
        let rest = buf[end..].to_vec();
        Ok(Client { sock, buf: rest, head })
    }

    fn send_frame(&mut self, opcode: u8, fin: bool, payload: &[u8]) {
        let mask = [1u8, 2, 3, 4];
        let mut f = vec![(if fin { 0x80 } else { 0 }) | opcode];
        match payload.len() {
            n if n < 126 => f.push(0x80 | n as u8),
            n => {
                f.push(0x80 | 126);
                f.extend_from_slice(&(n as u16).to_be_bytes());
            }
        }
        f.extend_from_slice(&mask);
        f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.sock.write_all(&f).unwrap();
    }

    fn send(&mut self, text: &str) {
        self.send_frame(1, true, text.as_bytes());
    }

    /// The next text message, or `None` after `wait`; a close shows as `"<closed>"`.
    fn recv(&mut self, wait: Duration) -> Option<String> {
        self.sock.set_read_timeout(Some(wait)).unwrap();
        let mut chunk = [0u8; 4096];
        loop {
            if self.buf.len() >= 2 {
                let (op, mut len) = (self.buf[0] & 0x0f, usize::from(self.buf[1] & 0x7f));
                let mut at = 2;
                if len == 126 && self.buf.len() >= 4 {
                    len = usize::from(u16::from_be_bytes([self.buf[2], self.buf[3]]));
                    at = 4;
                }
                if !(len == 126 && at == 2) && self.buf.len() >= at + len {
                    let payload = self.buf[at..at + len].to_vec();
                    self.buf.drain(..at + len);
                    match op {
                        1 => return Some(String::from_utf8(payload).unwrap()),
                        8 => return Some("<closed>".into()),
                        _ => continue,
                    }
                }
            }
            match self.sock.read(&mut chunk) {
                Ok(0) => return Some("<closed>".into()),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(_) => return None,
            }
        }
    }

    fn text(&mut self) -> String {
        self.recv(Duration::from_secs(5)).expect("a message")
    }

    /// Sends a request and returns the reply with its id.
    fn call(&mut self, request: &str) -> String {
        self.send(request);
        self.text()
    }

    fn quiet(&mut self) -> bool {
        self.recv(Duration::from_millis(300)).is_none()
    }
}

fn req(id: u32, method: &str, params: &str) -> String {
    if params.is_empty() {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"minecraft:{method}"}}"#)
    } else {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"minecraft:{method}","params":{params}}}"#)
    }
}

// ---- startup ----

#[test]
fn start_checks_the_settings_as_vanilla_does() {
    let env = |mutate: fn(&mut Config)| std::panic::catch_unwind(|| Env::start_with(mutate));
    // (The error comes back from `ManagementServer::start`; build one by hand.)
    let dir = std::env::temp_dir();
    let _ = dir;
    let _ = env;
    for (secret, ok) in [("short", false), ("abcdefghijABCDEFGHIJ0123456789klmnopqr!", false), (SECRET, true), ("", false)] {
        assert_eq!(secret_is_valid(secret), ok, "{secret}");
    }
    assert_eq!(host_and_port("127.0.0.1", 0).unwrap(), "127.0.0.1:0");
    assert_eq!(host_and_port("", 25666).unwrap(), ":25666");
    assert_eq!(host_and_port("::1", 25666).unwrap(), "[::1]:25666");
    assert_eq!(host_and_port("[::1]", 25666).unwrap(), "[::1]:25666");
    assert_eq!(host_and_port("a", 70000).unwrap_err().message, "java.lang.IllegalArgumentException: Port out of range: 70000");
    assert_eq!(host_and_port("a:80", 1).unwrap_err().message, "java.lang.IllegalArgumentException: Host has a port: a:80");
}

fn start_error(tweak: impl FnOnce(&mut Config)) -> StartError {
    let info = Arc::new(ServerInfo::new("m", 20, "w"));
    let access = AccessLists::new(None).shared();
    let fake = Arc::new(FakeRunner(Mutex::new(Fake {
        access: access.clone(),
        info: info.clone(),
        players: Vec::new(),
        kicked: Vec::new(),
        messages: Vec::new(),
        settings: HashMap::new(),
        rules: Vec::new(),
        saves: Vec::new(),
        halted: false,
        kick_unlisted_calls: 0,
    })));
    let mut config = Config { host: "127.0.0.1".into(), port: 0, secret: SECRET.into(), tls_enabled: false, tls_keystore: String::new(), tls_keystore_password: String::new(), allowed_origins: String::new(), heartbeat_interval: 0 };
    tweak(&mut config);
    let props = ServerProperties::parse(crate::javaprops::JavaProps::parse(""), || SECRET.to_owned()).unwrap();
    let backend = Backend { runner: fake, info, access, settings: Arc::new(Settings { props: Mutex::new(props), path: None }), lookup: None, online_mode: false };
    ManagementServer::start(&config, backend).err().expect("start fails")
}

#[test]
fn start_fails_with_the_messages_of_vanilla() {
    assert_eq!(start_error(|c| c.secret = "short".into()).to_string(), "java.lang.IllegalStateException: Invalid management server secret, must be 40 alphanumeric characters");
    assert_eq!(start_error(|c| c.port = 70000).to_string(), "java.lang.IllegalArgumentException: Port out of range: 70000");
    let e = start_error(|c| c.tls_enabled = true);
    assert_eq!(e.message, "java.lang.IllegalStateException: Failed to configure TLS for the server management protocol");
    assert_eq!(e.cause.as_deref(), Some("java.lang.IllegalArgumentException: TLS is enabled but keystore is not configured"));
    let e = start_error(|c| {
        c.tls_enabled = true;
        c.tls_keystore = "/nonexistent.p12".into();
    });
    assert_eq!(e.cause.as_deref(), Some("java.lang.IllegalArgumentException: Supplied keystore is not a file or does not exist: '/nonexistent.p12'"));
    let e = start_error(|c| c.host = "no.such.host.invalid".into());
    assert_eq!(e.message, "java.nio.channels.UnresolvedAddressException");
    // A port in use.
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    assert!(start_error(|c| c.port = i32::from(port)).message.contains("Address already in use"));
}

// ---- authentication ----

fn denied(env: &Env, headers: &[(&str, &str)]) -> String {
    Client::connect(env.server.local_addr(), headers).err().expect("refused")
}

#[test]
fn only_the_secret_gets_in() {
    let env = Env::start_with(|c| c.allowed_origins = "https://good.example,https://other.example".into());
    let body = |reason: &str| format!("{{\"error\":\"Unauthorized\",\"message\":\"{reason}\"}}");
    let r = denied(&env, &[]);
    assert!(r.starts_with("HTTP/1.1 401 Unauthorized\r\n") && r.ends_with(&body("Missing API key")), "{r}");
    assert!(r.contains("content-type: application/json\r\n") && r.contains("connection: close\r\n"));
    assert!(denied(&env, &[("Authorization", "Bearer wrong")]).ends_with(&body("Invalid API key")));
    assert!(denied(&env, &[("Authorization", "Bearer")]).ends_with(&body("Missing API key")));
    assert!(denied(&env, &[("Authorization", "bearer abc")]).ends_with(&body("Missing API key")));
    assert!(denied(&env, &[("Authorization", "Basic abc")]).ends_with(&body("Missing API key")));
    // The secret in the sub-protocol needs an origin that is allowed.
    let proto = format!("minecraft-v1, {SECRET}");
    assert!(denied(&env, &[("Sec-WebSocket-Protocol", &proto)]).ends_with(&body("Origin Not Allowed")));
    assert!(denied(&env, &[("Sec-WebSocket-Protocol", &proto), ("Origin", "https://evil.example")]).ends_with(&body("Origin Not Allowed")));
    assert!(denied(&env, &[("Sec-WebSocket-Protocol", "minecraft-v1, nope"), ("Origin", "https://good.example")]).ends_with(&body("Invalid API key")));
    let ok = Client::connect(env.server.local_addr(), &[("Sec-WebSocket-Protocol", &proto), ("Origin", "https://other.example")]).unwrap();
    assert!(ok.head.contains("sec-websocket-protocol: minecraft-v1\r\n"), "{}", ok.head);
    // The bearer token needs no origin and gets no protocol back.
    let ok = env.connect();
    assert!(ok.head.starts_with("HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: upgrade\r\nsec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n"), "{}", ok.head);
}

#[test]
fn an_empty_origin_list_allows_no_origin() {
    let env = Env::start();
    let proto = format!("minecraft-v1, {SECRET}");
    assert!(denied(&env, &[("Sec-WebSocket-Protocol", &proto), ("Origin", "")]).contains("Origin Not Allowed"));
}

// ---- protocol ----

#[test]
fn protocol_errors_are_answered_as_vanilla_answers_them() {
    let env = Env::start();
    let mut c = env.connect();
    assert!(c.call("not json").contains(r#""id":null,"error":{"code":-32700,"message":"Parse error","data":"com.google.gson.JsonSyntaxException: "#));
    for (request, want) in [
        ("5", r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}"#),
        ("null", r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}"#),
        ("[]", "[]"),
        ("{}", r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}"#),
        (r#"{"jsonrpc":"2.0","id":1}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found","data":"Method not found: nope"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":"Minecraft:Allowlist"}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request","data":"Failed to parse method value: Minecraft:Allowlist"}}"#),
        (r#"{"jsonrpc":"2.0","id":{"a":1},"method":"allowlist"}"#, r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request","data":"Invalid request id - only String, Number and NULL supported"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":null}"#, r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error","data":"Unknown error handling request - check server logs for stack trace"}}"#),
        (r#"{"jsonrpc":"2.0","id":"abc","method":"minecraft:allowlist"}"#, "{\"jsonrpc\":\"2.0\",\"id\":\"abc\",\"result\":[]}"),
        (r#"{"id":1.5,"method":"minecraft:allowlist","params":[]}"#, "{\"jsonrpc\":\"2.0\",\"id\":1.5,\"result\":[]}"),
        (r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:allowlist","params":{}}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":"Expected no params, or an empty array"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:allowlist","params":null}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":"Expected no params, or an empty array"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:allowlist/add"}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":"Expected params as array or named"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:allowlist/add","params":[]}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":"Expected exactly one element in the params array"}}"#),
        (r#"{"jsonrpc":"2.0","id":1,"method":"minecraft:server/save","params":{"other":true}}"#, r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":"Params passed by-name, but expected param [flush] does not exist"}}"#),
    ] {
        // Not before the world is loaded: only `rpc.discover` answers then.
        env.ready();
        assert_eq!(c.call(request), want, "{request}");
    }
}

#[test]
fn requests_without_an_id_are_never_answered() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    c.send(r#"{"jsonrpc":"2.0","method":"minecraft:allowlist/add","params":[[{"name":"someone"}]]}"#);
    c.send(r#"{"jsonrpc":"2.0","method":"nope"}"#);
    c.send(r#"{"jsonrpc":"2.0","method":"minecraft:allowlist/add"}"#);
    // The first was carried out; its notification arrives, the others say nothing.
    assert!(c.text().contains("notification/allowlist/added"));
    assert!(c.quiet());
    // Responses and errors from the client are read and ignored.
    c.send(r#"{"jsonrpc":"2.0","id":1,"result":5}"#);
    c.send(r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"x"}}"#);
    assert!(c.quiet());
}

#[test]
fn a_batch_is_answered_by_an_array_without_the_notifications_replies() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    let reply = c.call(r#"[{"jsonrpc":"2.0","id":1,"method":"minecraft:allowlist"},{"jsonrpc":"2.0","method":"minecraft:allowlist"},{"jsonrpc":"2.0","id":3,"method":"nope"}]"#);
    assert_eq!(reply, r#"[{"jsonrpc":"2.0","id":1,"result":[]},{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"Method not found","data":"Method not found: nope"}}]"#);
    // An element that is no object closes the connection.
    assert_eq!(c.call("[1,2]"), "<closed>");
}

#[test]
fn nothing_but_discovery_is_served_before_the_world_is_loaded() {
    let env = Env::start();
    let mut c = env.connect();
    assert_eq!(
        c.call(&req(1, "allowlist", "")),
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request","data":"Method cannot be dispatched pre server initialization: minecraft:allowlist"}}"#
    );
    assert!(c.call(&req(2, "rpc.discover", "")).starts_with(r#"{"jsonrpc":"2.0","id":2,"result":{"openrpc":"1.3.2","#));
    assert!(c.call(r#"{"jsonrpc":"2.0","id":3,"method":"rpc.discover","params":[]}"#).contains(r#""id":3,"result""#));
    env.ready();
    assert_eq!(c.call(&req(4, "allowlist", "")), r#"{"jsonrpc":"2.0","id":4,"result":[]}"#);
}

#[test]
fn websocket_frames_pings_and_big_messages() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    // A message in two fragments, a ping in between.
    let request = req(1, "allowlist", "");
    let (a, b) = request.split_at(10);
    c.send_frame(1, false, a.as_bytes());
    c.send_frame(9, true, b"hi");
    c.send_frame(0, true, b.as_bytes());
    let mut seen = Vec::new();
    for _ in 0..2 {
        // (The pong is no text; the reply is.)
        if let Some(t) = c.recv(Duration::from_secs(5)) {
            seen.push(t);
        }
    }
    assert_eq!(seen, vec![r#"{"jsonrpc":"2.0","id":1,"result":[]}"#.to_owned()]);
    // Binary frames are ignored.
    c.send_frame(2, true, b"\x00\x01");
    assert!(c.quiet());
    // A message over 65536 bytes closes the connection.
    c.send_frame(1, false, &vec![b' '; 40000]);
    c.send_frame(0, true, &vec![b' '; 40000]);
    assert_eq!(c.text(), "<closed>");
}

// ---- the methods ----

#[test]
fn the_lists_and_their_notifications() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    let mut watcher = env.connect();
    let id = |n: &str| offline_uuid(n).to_string();
    // Add by name: an offline server makes the offline profile.
    assert_eq!(
        c.call(&req(1, "allowlist/add", r#"[[{"name":"Alex"}]]"#)),
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":[{{"id":"{}","name":"alex"}}]}}"#, id("alex"))
    );
    // The reply comes before the notification.
    assert_eq!(c.text(), format!(r#"{{"jsonrpc":"2.0","method":"minecraft:notification/allowlist/added","params":[{{"id":"{}","name":"alex"}}]}}"#, id("alex")));
    assert!(watcher.text().contains("notification/allowlist/added"));
    // Adding again announces again.
    c.call(&req(2, "allowlist/add", r#"[[{"name":"alex"}]]"#));
    assert!(c.text().contains("allowlist/added"));
    // An id that no list knows is ignored (no session service offline).
    let unknown = c.call(&req(3, "allowlist/add", r#"[[{"id":"00000000-0000-0000-0000-000000000007","name":"Bob"}]]"#));
    assert!(unknown.contains("alex") && !unknown.contains("bob"));
    assert!(c.quiet());
    // The file is written.
    let file = std::fs::read_to_string(env.dir.join("whitelist.json")).unwrap();
    assert!(file.contains(&id("alex")));
    // Remove, and clear says one removal per entry.
    assert!(c.call(&req(4, "allowlist/remove", r#"[[{"name":"alex"}]]"#)).ends_with(r#""result":[]}"#));
    assert!(c.text().contains("allowlist/removed"));
    assert_eq!(env.with_fake(|f| f.kick_unlisted_calls), 1);
    c.call(&req(5, "allowlist/add", r#"[[{"name":"a1"},{"name":"a2"}]]"#));
    c.text();
    c.text();
    c.call(&req(6, "allowlist/clear", ""));
    assert!(c.text().contains("allowlist/removed") && c.text().contains("allowlist/removed"));
    assert_eq!(env.with_fake(|f| f.kick_unlisted_calls), 1);
}

#[test]
fn bans_carry_reason_source_and_expiry() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    let id = offline_uuid("grief").to_string();
    env.with_fake(|f| f.players.push(OnlinePlayer { user: NameAndId { uuid: offline_uuid("grief"), name: "grief".into() }, address: Some("10.0.0.5".parse().unwrap()) }));
    let reply = c.call(&req(1, "bans/add", r#"[[{"player":{"name":"grief"},"reason":"x","expires":"2999-01-01T00:00:00Z"}]]"#));
    assert_eq!(
        reply,
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":[{{"player":{{"id":"{id}","name":"grief"}},"reason":"x","source":"Management server","expires":"2999-01-01T00:00:00Z"}}]}}"#)
    );
    assert!(c.text().contains(r#"notification/bans/added"#));
    // The banned player is kicked.
    assert_eq!(env.with_fake(|f| f.kicked.clone()), vec![(offline_uuid("grief"), MgmtText::Translatable { key: "multiplayer.disconnect.banned".into(), args: vec![] })]);
    // A bad instant is an invalid parameter.
    assert!(c.call(&req(2, "bans/add", r#"[[{"player":{"name":"p"},"expires":"bad"}]]"#)).contains("Text 'bad' could not be parsed at index 0"));
    assert!(c.call(&req(3, "bans/add", r#"[[{"reason":"no player"}]]"#)).contains(r#"No key player in MapLike[{\"reason\":\"no player\"}]"#));
    // set replaces: removals, then additions.
    c.call(&req(4, "bans/set", r#"[[{"player":{"name":"solo"}}]]"#));
    assert!(c.text().contains("bans/removed"));
    assert!(c.text().contains("bans/added"));
    c.call(&req(5, "bans/clear", ""));
    assert!(c.text().contains("bans/removed"));
    // IP bans.
    let reply = c.call(&req(6, "ip_bans/add", r#"[[{"ip":"10.0.0.5","reason":"spam"},{"ip":"bad ip"}]]"#));
    assert_eq!(reply, r#"{"jsonrpc":"2.0","id":6,"result":[{"ip":"10.0.0.5","reason":"spam","source":"Management server"}]}"#);
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/ip_bans/added","params":[{"ip":"10.0.0.5","reason":"spam","source":"Management server"}]}"#);
    assert_eq!(env.with_fake(|f| f.kicked.last().cloned()).unwrap().1, MgmtText::Translatable { key: "multiplayer.disconnect.ip_banned".into(), args: vec![] });
    c.call(&req(7, "ip_bans/remove", r#"[["10.0.0.5"]]"#));
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/ip_bans/removed","params":["10.0.0.5"]}"#);
}

#[test]
fn operators_get_levels_and_set_compares_whole_entries() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    let id = offline_uuid("boss").to_string();
    let entry = |level: u8, bypass: bool| format!(r#"{{"player":{{"id":"{id}","name":"boss"}},"permissionLevel":{level},"bypassesPlayerLimit":{bypass}}}"#);
    assert_eq!(c.call(&req(1, "operators/add", r#"[[{"player":{"name":"boss"},"permissionLevel":9,"bypassesPlayerLimit":true}]]"#)), format!(r#"{{"jsonrpc":"2.0","id":1,"result":[{}]}}"#, entry(4, true)));
    assert_eq!(c.text(), format!(r#"{{"jsonrpc":"2.0","method":"minecraft:notification/operators/added","params":[{}]}}"#, entry(4, true)));
    c.call(&req(2, "operators/add", r#"[[{"player":{"name":"boss"},"permissionLevel":-3}]]"#));
    assert!(c.text().contains(r#""permissionLevel":0,"bypassesPlayerLimit":true"#));
    // set with the same entry changes nothing; with a different one removes and adds.
    c.call(&req(3, "operators/set", r#"[[{"player":{"name":"boss"},"permissionLevel":0,"bypassesPlayerLimit":true}]]"#));
    assert!(c.quiet());
    c.call(&req(4, "operators/set", r#"[[{"player":{"name":"boss"},"permissionLevel":1}]]"#));
    assert!(c.text().contains("operators/removed"));
    assert!(c.text().contains("operators/added"));
    assert!(c.call(&req(5, "operators/add", r#"[[{"player":{"name":"x"},"permissionLevel":"high"}]]"#)).contains(r#""data":"Not a number: \"high\"""#));
    c.call(&req(6, "operators/clear", ""));
    assert!(c.text().contains("operators/removed"));
}

#[test]
fn players_kick_and_system_messages() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    let alice = offline_uuid("Alice");
    env.with_fake(|f| f.players.push(OnlinePlayer { user: NameAndId { uuid: alice, name: "Alice".into() }, address: None }));
    env.info.roster.write().unwrap().push(kiln_link::RosterEntry { conn: 1, uuid: alice, name: "Alice".into(), allows_listing: true });
    assert_eq!(c.call(&req(1, "players", "")), format!(r#"{{"jsonrpc":"2.0","id":1,"result":[{{"id":"{alice}","name":"Alice"}}]}}"#));
    assert_eq!(c.call(&req(2, "players/kick", r#"[[{"player":{"name":"alice"},"message":{"literal":"bye"}},{"player":{"name":"nobody"}}]]"#)), r#"{"jsonrpc":"2.0","id":2,"result":[{"name":"alice"}]}"#);
    assert_eq!(env.with_fake(|f| f.kicked.clone()), vec![(alice, MgmtText::Literal("bye".into()))]);
    // A kicked player has left the player list when the reply comes, and the notification follows it.
    assert_eq!(c.text(), format!(r#"{{"jsonrpc":"2.0","method":"minecraft:notification/players/left","params":[{{"id":"{alice}","name":"Alice"}}]}}"#));
    assert_eq!(c.call(&req(20, "players", "")), r#"{"jsonrpc":"2.0","id":20,"result":[]}"#);
    env.with_fake(|f| f.players.push(OnlinePlayer { user: NameAndId { uuid: alice, name: "Alice".into() }, address: None }));
    env.info.roster.write().unwrap().push(kiln_link::RosterEntry { conn: 1, uuid: alice, name: "Alice".into(), allows_listing: true });
    c.call(&req(3, "players/kick", &format!(r#"[[{{"player":{{"id":"{alice}"}}}}]]"#)));
    assert_eq!(env.with_fake(|f| f.kicked.last().cloned()).unwrap().1, MgmtText::Translatable { key: "multiplayer.disconnect.kicked".into(), args: vec![] });
    assert!(c.text().contains("players/left"));
    env.with_fake(|f| f.players.push(OnlinePlayer { user: NameAndId { uuid: alice, name: "Alice".into() }, address: None }));
    // System messages: broadcast, or to named players; nothing to say or nobody to tell is false.
    assert_eq!(c.call(&req(4, "server/system_message", r#"[{"message":{"literal":"hi"},"overlay":true}]"#)), r#"{"jsonrpc":"2.0","id":4,"result":true}"#);
    assert_eq!(env.with_fake(|f| f.messages.last().cloned()).unwrap(), (MgmtText::Literal("hi".into()), true, None));
    c.call(&req(5, "server/system_message", r#"[{"message":{"translatable":"k","translatableParams":["a"],"literal":"l"},"overlay":false,"receivingPlayers":[{"name":"alice"},{"name":"zed"}]}]"#));
    assert_eq!(env.with_fake(|f| f.messages.last().cloned()).unwrap(), (MgmtText::Translatable { key: "k".into(), args: vec!["a".into()] }, false, Some(vec![alice])));
    assert_eq!(c.call(&req(6, "server/system_message", r#"[{"message":{},"overlay":false}]"#)), r#"{"jsonrpc":"2.0","id":6,"result":false}"#);
    assert_eq!(c.call(&req(7, "server/system_message", r#"[{"message":{"literal":"x"},"overlay":false,"receivingPlayers":[]}]"#)), r#"{"jsonrpc":"2.0","id":7,"result":false}"#);
    // A receivingPlayers that does not decode is as good as none.
    assert_eq!(c.call(&req(8, "server/system_message", r#"[{"message":{"literal":"x"},"overlay":false,"receivingPlayers":5}]"#)), r#"{"jsonrpc":"2.0","id":8,"result":true}"#);
    assert_eq!(env.with_fake(|f| f.messages.last().cloned()).unwrap().2, None);
}

#[test]
fn server_state_save_and_stop() {
    let env = Env::start();
    let mut c = env.connect();
    env.ready();
    let version = format!(r#""version":{{"name":"{}","protocol":{}}}"#, kiln_data::version::NAME, kiln_data::version::PROTOCOL);
    assert_eq!(c.call(&req(1, "server/status", "")), format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"started":true,{version}}}}}"#));
    assert_eq!(c.call(&req(2, "server/save", "[false]")), r#"{"jsonrpc":"2.0","id":2,"result":true}"#);
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/server/saving"}"#);
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/server/saved"}"#);
    assert_eq!(env.with_fake(|f| f.saves.clone()), vec![false]);
    assert_eq!(c.call(&req(3, "server/stop", "")), r#"{"jsonrpc":"2.0","id":3,"result":true}"#);
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/server/stopping"}"#);
    assert!(env.with_fake(|f| f.halted));
}

#[test]
fn game_rules_are_typed_and_range_checked() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    assert_eq!(
        c.call(&req(1, "gamerules", "")),
        r#"{"jsonrpc":"2.0","id":1,"result":[{"type":"boolean","value":false,"key":"minecraft:keep_inventory"},{"type":"integer","value":65536,"key":"minecraft:max_command_forks"}]}"#
    );
    assert_eq!(
        c.call(&req(2, "gamerules/update", r#"[{"key":"keep_inventory","value":true}]"#)),
        r#"{"jsonrpc":"2.0","id":2,"result":{"type":"boolean","value":true,"key":"minecraft:keep_inventory"}}"#
    );
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/gamerules/updated","params":[{"type":"boolean","value":true,"key":"minecraft:keep_inventory"}]}"#);
    for (params, want) in [
        (r#"[{"key":"keep_inventory","value":"true"}]"#, "Not a boolean: \\\"true\\\""),
        (r#"[{"key":"max_command_forks","value":true}]"#, "Not a number: true"),
        (r#"[{"key":"max_command_forks","value":-5}]"#, "Value -5 outside of range [0:2147483647]"),
        (r#"[{"key":"max_command_forks","value":2147483648}]"#, "Value -2147483648 outside of range [0:2147483647]"),
        (r#"[{"key":"nope","value":5}]"#, "Unknown registry key in ResourceKey[minecraft:root / minecraft:game_rule]: minecraft:nope"),
        (r#"[{"key":"Minecraft:keep_inventory","value":true}]"#, "Not a valid resource location: Minecraft:keep_inventory Non [a-z0-9_.-] character in namespace of identifier: Minecraft:keep_inventory"),
        (r#"[{"value":false}]"#, "No key key in MapLike[{\\\"value\\\":false}]"),
        (r#"[{"key":"keep_inventory"}]"#, "No key value in MapLike[{\\\"key\\\":\\\"keep_inventory\\\"}]"),
        (r#"[5]"#, "Not a JSON object: 5"),
    ] {
        let reply = c.call(&req(3, "gamerules/update", params));
        assert!(reply.contains(&format!(r#""data":"{want}""#)), "{params}: {reply}");
    }
}

// ---- settings ----

#[test]
fn settings_are_stored_mapped_and_written_to_server_properties() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    let set = |c: &mut Client, id: u32, method: &str, value: &str| c.call(&req(id, &format!("serversettings/{method}/set"), &format!("[{value}]")));
    assert_eq!(set(&mut c, 1, "max_players", "5"), r#"{"jsonrpc":"2.0","id":1,"result":5}"#);
    assert_eq!(set(&mut c, 2, "max_players", "3000000000"), r#"{"jsonrpc":"2.0","id":2,"result":-1294967296}"#);
    assert_eq!(set(&mut c, 3, "max_players", "2.5"), r#"{"jsonrpc":"2.0","id":3,"result":2}"#);
    assert!(set(&mut c, 4, "max_players", r#""7""#).contains(r#"Not a number: \"7\""#));
    assert_eq!(set(&mut c, 5, "entity_broadcast_range", "5"), r#"{"jsonrpc":"2.0","id":5,"result":10}"#);
    assert_eq!(set(&mut c, 6, "entity_broadcast_range", "5000"), r#"{"jsonrpc":"2.0","id":6,"result":1000}"#);
    assert_eq!(set(&mut c, 7, "operator_user_permission_level", "7"), r#"{"jsonrpc":"2.0","id":7,"result":4}"#);
    assert_eq!(set(&mut c, 8, "operator_user_permission_level", "-1"), r#"{"jsonrpc":"2.0","id":8,"result":0}"#);
    assert_eq!(set(&mut c, 9, "view_distance", "100"), r#"{"jsonrpc":"2.0","id":9,"result":100}"#);
    assert_eq!(set(&mut c, 10, "motd", r#""Hello §a world""#), "{\"jsonrpc\":\"2.0\",\"id\":10,\"result\":\"Hello §a world\"}");
    assert_eq!(set(&mut c, 11, "game_mode", r#""creative""#), r#"{"jsonrpc":"2.0","id":11,"result":"creative"}"#);
    assert!(set(&mut c, 12, "game_mode", r#""bad""#).contains(r#""data":"Unknown element name:bad""#));
    assert!(set(&mut c, 13, "difficulty", r#""HARD""#).contains(r#""data":"Unknown element name:HARD""#));
    assert!(set(&mut c, 14, "difficulty", "2").contains(r#""data":"Not a string: 2""#));
    assert_eq!(set(&mut c, 15, "difficulty", r#""hard""#), r#"{"jsonrpc":"2.0","id":15,"result":"hard"}"#);
    assert_eq!(c.call(&req(16, "serversettings/difficulty", "")), r#"{"jsonrpc":"2.0","id":16,"result":"hard"}"#);
    assert_eq!(set(&mut c, 17, "use_allowlist", "true"), r#"{"jsonrpc":"2.0","id":17,"result":true}"#);
    assert_eq!(set(&mut c, 18, "enforce_allowlist", "true"), r#"{"jsonrpc":"2.0","id":18,"result":true}"#);
    assert_eq!(env.with_fake(|f| f.kick_unlisted_calls), 2);
    // What the game was told to apply.
    assert_eq!(env.with_fake(|f| f.settings.get("EntityBroadcastRange").cloned()), Some(SettingValue::Int(1000)));
    assert_eq!(env.with_fake(|f| f.settings.get("OperatorPermissionLevel").cloned()), Some(SettingValue::Int(0)));
    // The properties file follows.
    let file = std::fs::read_to_string(env.dir.join("server.properties")).unwrap();
    for line in ["max-players=2", "entity-broadcast-range-percentage=1000", "op-permission-level=0", "view-distance=100", "gamemode=creative", "difficulty=hard", "motd=Hello §a world", "white-list=true", "enforce-whitelist=true"] {
        assert!(file.lines().any(|l| l == line), "{line} in\n{file}");
    }
    // A getter answers from the properties.
    assert_eq!(c.call(&req(19, "serversettings/max_players", "")), r#"{"jsonrpc":"2.0","id":19,"result":2}"#);
}

#[test]
fn the_heartbeat_comes_when_asked_for() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    assert_eq!(c.call(&req(1, "serversettings/status_heartbeat_interval", "")), r#"{"jsonrpc":"2.0","id":1,"result":0}"#);
    assert!(c.quiet());
    assert_eq!(c.call(&req(2, "serversettings/status_heartbeat_interval/set", "[1]")), r#"{"jsonrpc":"2.0","id":2,"result":1}"#);
    let beat = c.recv(Duration::from_secs(3)).expect("a heartbeat");
    let version = format!(r#""version":{{"name":"{}","protocol":{}}}"#, kiln_data::version::NAME, kiln_data::version::PROTOCOL);
    assert_eq!(beat, format!(r#"{{"jsonrpc":"2.0","method":"minecraft:notification/server/status","params":[{{"started":true,{version}}}]}}"#));
    assert_eq!(c.call(&req(3, "serversettings/status_heartbeat_interval/set", "[-1]")), r#"{"jsonrpc":"2.0","id":3,"result":-1}"#);
    assert!(c.recv(Duration::from_millis(1500)).is_none());
}

#[test]
fn a_configured_heartbeat_starts_with_the_server() {
    let env = Env::start_with(|c| c.heartbeat_interval = 1);
    let mut c = env.connect();
    // Before the world is loaded the heartbeat says the server has not started.
    let beat = c.recv(Duration::from_secs(3)).expect("a heartbeat");
    assert!(beat.contains(r#""params":[{"started":false,"version":"#), "{beat}");
}

// ---- what happens to the connections ----

#[test]
fn notifications_reach_every_connection_and_stop_with_the_server() {
    let mut env = Env::start();
    env.ready();
    let mut a = env.connect();
    let mut b = env.connect();
    env.info.notifier.notify(Notification::PlayerJoined(NameAndId { uuid: offline_uuid("x"), name: "x".into() }));
    let want = format!(r#"{{"jsonrpc":"2.0","method":"minecraft:notification/players/joined","params":[{{"id":"{}","name":"x"}}]}}"#, offline_uuid("x"));
    assert_eq!(a.text(), want);
    assert_eq!(b.text(), want);
    env.info.notifier.notify(Notification::ServerShuttingDown);
    env.server.stop();
    assert_eq!(a.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/server/stopping"}"#);
    assert_eq!(a.text(), "<closed>");
    assert_eq!(b.recv(Duration::from_secs(2)).as_deref(), Some(r#"{"jsonrpc":"2.0","method":"minecraft:notification/server/stopping"}"#));
}

#[test]
fn the_server_activity_notification_is_limited_to_one_in_thirty_seconds() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    env.info.notifier.activity();
    env.info.notifier.activity();
    assert_eq!(c.text(), r#"{"jsonrpc":"2.0","method":"minecraft:notification/server/activity"}"#);
    assert!(c.quiet());
}

// ---- TLS ----

fn keytool(dir: &std::path::Path, password: &str) -> Option<PathBuf> {
    let store = dir.join("test.p12");
    let ok = std::process::Command::new("keytool")
        .args(["-genkeypair", "-alias", "mgmt", "-keyalg", "RSA", "-keysize", "2048", "-validity", "2", "-dname", "CN=localhost", "-storetype", "PKCS12", "-storepass", password, "-keypass", password, "-keystore"])
        .arg(&store)
        .output()
        .ok()?
        .status
        .success();
    ok.then_some(store)
}

#[test]
fn tls_serves_from_a_pkcs12_keystore() {
    let dir = std::env::temp_dir().join(format!("kiln-mgmt-tls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let Some(store) = keytool(&dir, "changeit") else {
        eprintln!("keytool is not available: TLS test skipped");
        return;
    };
    let keystore = store.to_string_lossy().into_owned();
    // A wrong password does not start.
    let e = start_error({
        let keystore = keystore.clone();
        move |c| {
            c.tls_enabled = true;
            c.tls_keystore = keystore;
            c.tls_keystore_password = "wrong".into();
        }
    });
    assert_eq!(e.message, "java.lang.IllegalStateException: Failed to configure TLS for the server management protocol");
    let env = Env::start_with(|c| {
        c.tls_enabled = true;
        c.tls_keystore = keystore;
        c.tls_keystore_password = "changeit".into();
    });
    env.ready();
    // A TLS client that trusts anything.
    #[derive(Debug)]
    struct Anything;
    impl rustls::client::danger::ServerCertVerifier for Anything {
        fn verify_server_cert(&self, _: &rustls::pki_types::CertificateDer<'_>, _: &[rustls::pki_types::CertificateDer<'_>], _: &rustls::pki_types::ServerName<'_>, _: &[u8], _: rustls::pki_types::UnixTime) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(&self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(&self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Anything))
        .with_no_client_auth();
    let tcp = TcpStream::connect(env.server.local_addr()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let conn = rustls::ClientConnection::new(Arc::new(config), "localhost".try_into().unwrap()).unwrap();
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let upgrade = format!("GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {SECRET}\r\n\r\n");
    tls.write_all(upgrade.as_bytes()).unwrap();
    let mut got = Vec::new();
    let mut chunk = [0u8; 1024];
    while ws::head_end(&got).is_none() {
        let n = tls.read(&mut chunk).unwrap();
        got.extend_from_slice(&chunk[..n]);
    }
    assert!(String::from_utf8_lossy(&got).starts_with("HTTP/1.1 101 Switching Protocols"));
    // A masked text frame with a request.
    let request = req(1, "allowlist", "");
    let mask = [9u8, 8, 7, 6];
    let mut frame = vec![0x81, 0x80 | request.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(request.bytes().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    tls.write_all(&frame).unwrap();
    let mut reply = Vec::new();
    while reply.len() < 2 || reply.len() < 2 + usize::from(reply[1]) {
        let n = tls.read(&mut chunk).unwrap();
        reply.extend_from_slice(&chunk[..n]);
    }
    assert_eq!(String::from_utf8_lossy(&reply[2..]), r#"{"jsonrpc":"2.0","id":1,"result":[]}"#);
    // Plain text to the TLS port gets nowhere.
    let mut plain = TcpStream::connect(env.server.local_addr()).unwrap();
    plain.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    plain.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
    let mut buf = [0u8; 64];
    assert!(!matches!(plain.read(&mut buf), Ok(n) if n > 0 && buf.starts_with(b"HTTP")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lenient_json_reads_as_gson_reads_it() {
    // (Each case is a text sent to vanilla's server, with the data of its answer.)
    let t = "\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json";
    let mal = |why: &str, at: &str| format!("com.google.gson.JsonSyntaxException: com.google.gson.stream.MalformedJsonException: {why} at {at}{t}");
    for (text, want) in [
        ("]", mal("Unexpected value", "line 1 column 2 path $")),
        ("}", mal("Expected value", "line 1 column 1 path $")),
        (",", mal("Unexpected value", "line 1 column 2 path $")),
        ("[1 2]", mal("Unterminated array", "line 1 column 5 path $[1]")),
        (r#"{"a" 1}"#, mal("Expected ':'", "line 1 column 7 path $.a")),
        (r#"{"a":1,}"#, mal("Expected name", "line 1 column 9 path $.a")),
        (r#"{"a":}"#, mal("Expected value", "line 1 column 6 path $.a")),
        (r#"{"a":1 "b":2}"#, mal("Unterminated object", "line 1 column 9 path $.a")),
        ("foo bar", mal("Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON", "line 1 column 6 path $")),
        ("\"abc", mal("Unterminated string", "line 1 column 5 path $")),
        (r#""ab\qc""#, mal("Invalid escape sequence", "line 1 column 6 path $")),
        (r#""ab\u12""#, mal("Unterminated escape sequence", "line 1 column 6 path $")),
        (r#"{"id":1} // c"#, mal("Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON", "line 1 column 11 path $")),
    ] {
        assert_eq!(json::parse(text).unwrap_err().0, want, "{text}");
    }
    for (text, want) in [
        ("{", "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line 1 column 2 path $."),
        ("[", "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line 1 column 2 path $[0]"),
        ("[1,", "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line 1 column 4 path $[1]"),
        ("[1,2,3", "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line 1 column 7 path $[3]"),
        (r#"{"id":1"#, "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line 1 column 8 path $.id"),
    ] {
        assert_eq!(json::parse(text).unwrap_err().0, want, "{text}");
    }
    // What is read although it is no strict JSON: unquoted names, single quotes, `;` and `=` for `,` and `:`, comments.
    for (text, want) in [
        ("{a:1}", r#"{"a":1}"#),
        ("{'a':'b'}", r#"{"a":"b"}"#),
        (r#"{"a"="b";"c"=>[1,]}"#, r#"{"a":"b","c":[1,null]}"#),
        ("/* c */ {\"a\":1} ", r#"{"a":1}"#),
        ("# c\n[1]", "[1]"),
        ("tru", "\"tru\""),
        ("TRUE", "true"),
        ("01", "\"01\""),
        ("-1.5e3", "-1.5e3"),
        (" // nothing", "null"),
    ] {
        assert_eq!(json::parse(text).unwrap().to_string(), want, "{text}");
    }
}

/// The order the `set` methods remove in is that of Java's `HashSet`, which vanilla showed for these bans.
#[test]
fn set_removes_in_the_order_of_a_java_hash_set() {
    let env = Env::start();
    env.ready();
    let mut c = env.connect();
    c.call(&req(1, "bans/add", r#"[[{"player":{"name":"kt-grief"},"reason":"x","source":"me"}]]"#));
    c.text();
    for (n, (name, expires)) in [("kt-grief2", "2030-01-01T00:00:00Z"), ("kt-grief3", "2030-01-01T00:00:00.123Z"), ("kt-grief5", "2000-01-01T00:00:00Z")].into_iter().enumerate() {
        let reply = c.call(&req(2 + n as u32, "bans/add", &format!(r#"[[{{"player":{{"name":"{name}"}},"expires":"{expires}"}}]]"#)));
        // The milliseconds are kept.
        assert!(reply.contains(&format!(r#""expires":"{expires}""#)), "{reply}");
        c.text();
    }
    c.call(&req(9, "bans/remove", r#"[[{"name":"kt-grief"}]]"#));
    c.text();
    c.call(&req(10, "bans/set", r#"[[{"player":{"name":"kt-solo"}}]]"#));
    let removed: Vec<String> = (0..3).map(|_| c.text()).collect();
    for (got, name) in removed.iter().zip(["kt-grief2", "kt-grief3", "kt-grief5"]) {
        assert!(got.contains("notification/bans/removed") && got.contains(&format!(r#""name":"{name}""#)), "{got}");
    }
    assert!(c.text().contains("notification/bans/added"));
}

/// Sends `text` and returns what comes back until the server closes or says nothing for half a second.
fn raw(addr: SocketAddr, text: &str) -> String {
    let mut sock = TcpStream::connect(addr).unwrap();
    sock.write_all(text.as_bytes()).unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut out = Vec::new();
    let mut chunk = [0u8; 1024];
    while let Ok(n) = sock.read(&mut chunk) {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn handshake_refusals_read_like_nettys() {
    let env = Env::start();
    let auth = format!("Authorization: Bearer {SECRET}\r\n");
    let key = "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
    let ok_headers = format!("Host: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n{key}Sec-WebSocket-Version: 13\r\n{auth}");
    let get = |headers: &str| format!("GET / HTTP/1.1\r\n{headers}\r\n");
    let addr = env.server.local_addr();
    let bad = |why: &str| format!("HTTP/1.1 400 Bad Request\r\n\r\n{why}");
    assert_eq!(raw(addr, &format!("POST / HTTP/1.1\r\n{ok_headers}\r\n")), bad("Invalid WebSocket handshake method: POST"));
    assert_eq!(raw(addr, &format!("HEAD / HTTP/1.1\r\n{ok_headers}\r\n")), bad(""));
    assert_eq!(raw(addr, &get(&format!("Host: x\r\n{key}Sec-WebSocket-Version: 13\r\n{auth}"))), bad("not a WebSocket request: a |Connection| header must include a token 'Upgrade'"));
    assert_eq!(
        raw(addr, &get(&format!("Host: x\r\nUpgrade: websocket\r\nConnection: keep-alive\r\n{key}Sec-WebSocket-Version: 13\r\n{auth}"))),
        bad("not a WebSocket request: a |Connection| header must include a token 'Upgrade'")
    );
    assert_eq!(
        raw(addr, &get(&format!("Host: x\r\nUpgrade: foo\r\nConnection: Upgrade\r\n{key}Sec-WebSocket-Version: 13\r\n{auth}"))),
        bad("not a WebSocket request: an |Upgrade| header must containing the value 'websocket'")
    );
    assert_eq!(
        raw(addr, &get(&format!("Host: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n{auth}"))),
        bad("not a WebSocket request: missing key")
    );
    // A key that is empty or short is taken; the version may be 7, 8 or 13; others get 426 and the connection stays.
    for (version, ok) in [("13", true), ("8", true), ("7", true), ("0", false), ("99", false)] {
        let r = raw(addr, &get(&format!("Host: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key:\r\nSec-WebSocket-Version: {version}\r\n{auth}")));
        if ok {
            assert!(r.starts_with("HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: upgrade\r\nsec-websocket-accept: "), "{r}");
        } else {
            assert_eq!(r, "HTTP/1.1 426 Upgrade Required\r\nsec-websocket-version: 13\r\ncontent-length: 0\r\n\r\n");
        }
    }
    // Without a version: the first draft of the protocol.
    assert_eq!(
        raw(addr, &get(&format!("Host: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n{auth}"))),
        bad("Missing origin header, got only [Host, Upgrade, Connection, Authorization, content-length]")
    );
    assert_eq!(raw(addr, &get(&format!("Host: x\r\n{auth}"))), bad("not a WebSocket handshake request: missing upgrade"));
    assert_eq!(
        raw(addr, &get(&format!("Host: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nOrigin: http://x\r\n{auth}"))),
        "HTTP/1.1 101 Web Socket Protocol Handshake\r\nupgrade: websocket\r\nconnection: upgrade\r\nwebsocket-origin: http://x\r\nwebsocket-location: ws://x/\r\n\r\n"
    );
    // Only "/" is a WebSocket path; the request for another is never answered, but the secret is checked first.
    assert_eq!(raw(addr, &format!("GET /foo HTTP/1.1\r\n{ok_headers}\r\n")), "");
    assert_eq!(raw(addr, &format!("GET /?x=1 HTTP/1.1\r\n{ok_headers}\r\n")), "");
    assert!(raw(addr, "GET /foo HTTP/1.1\r\nHost: x\r\n\r\n").ends_with(r#"{"error":"Unauthorized","message":"Missing API key"}"#));
    // What is no HTTP request at all has no secret either.
    assert!(raw(addr, "hello there\r\n\r\n").ends_with(r#""message":"Missing API key"}"#));
    assert!(raw(addr, "GET / HTTP/1.1\nHost: x\nAuthorization: Bearer nope\n\n").ends_with(r#""message":"Missing API key"}"#));
    // A client waiting for permission to send a body is given it.
    let r = raw(addr, &get(&format!("{ok_headers}Expect: 100-continue\r\n")));
    assert!(r.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 101 Switching Protocols\r\n"), "{r}");
    // The origin must match exactly.
    let proto = format!("Sec-WebSocket-Protocol: minecraft-v1, {SECRET}\r\n");
    let env2 = Env::start_with(|c| c.allowed_origins = "https://good.example".into());
    for origin in ["HTTPS://GOOD.EXAMPLE", "https://good.example/"] {
        let r = raw(env2.server.local_addr(), &get(&format!("{ok_headers_no_auth}{proto}Origin: {origin}\r\n", ok_headers_no_auth = format!("Host: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n{key}Sec-WebSocket-Version: 13\r\n"))));
        assert!(r.ends_with(r#""message":"Origin Not Allowed"}"#), "{r}");
    }
}
