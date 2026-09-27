//! Connection handling on the tokio runtime: handshake, status, login and configuration run
//! here; once a player reaches play state, packets are relayed to and from the simulation.

mod auth;
mod cipher;
pub mod lobby;
mod profile;
pub mod proxy;

pub use lobby::LobbyConfig;
pub use profile::GameProfile;
pub use proxy::ProxyMode;

use anyhow::{Context, Result, anyhow, bail};
use auth::{ServerKey, SessionService};
use bytes::{Bytes, BytesMut};
use cipher::{Decryptor, Encryptor};
use kiln_data::packets as ids;
use kiln_data::version;
use kiln_link::{ConnId, JoinInfo, Sink, ToSim};
use kiln_proto::Reader;
use kiln_proto::frame::FrameCodec;
use kiln_proto::packets::{self, login_ext};
use profile::{offline_uuid, valid_name};
use proxy::{ForwardError, Forwarded};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct Config {
    pub bind: SocketAddr,
    pub motd: String,
    pub max_players: usize,
    pub view_distance: u8,
    pub simulation_distance: u8,
    /// `None` disables compression. Behind a proxy `LoginConfig::proxy_compression_threshold` applies.
    pub compression_threshold: Option<usize>,
}

/// How players prove who they are.
pub struct LoginConfig {
    /// Authenticate players with the session server. Behind a proxy the proxy does it instead.
    pub online_mode: bool,
    /// Also have the session server check the client's address.
    pub prevent_proxy_connections: bool,
    pub proxy: ProxyMode,
    /// Compression threshold behind a proxy. `None` (-1, the default) leaves compression to the
    /// proxy, which re-compresses for its clients anyway; a remote proxy may want a high value.
    pub proxy_compression_threshold: Option<usize>,
}

impl Default for LoginConfig {
    fn default() -> Self {
        Self {
            online_mode: false,
            prevent_proxy_connections: false,
            proxy: ProxyMode::None,
            proxy_compression_threshold: None,
        }
    }
}

impl LoginConfig {
    /// Reads the login settings from the environment, until the server has a config file:
    /// `KILN_ONLINE_MODE`, `KILN_PREVENT_PROXY_CONNECTIONS` (true/false), `KILN_PROXY`
    /// (none, velocity, bungeecord), `KILN_VELOCITY_SECRET` or `KILN_VELOCITY_SECRET_FILE`,
    /// `KILN_BUNGEEGUARD_TOKENS` (comma separated) and `KILN_PROXY_COMPRESSION_THRESHOLD` (-1 = off).
    pub fn from_env() -> Result<Self> {
        Self::from_vars(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let flag = |k: &str| -> Result<bool> {
            match var(k).as_deref().map(str::trim) {
                None => Ok(false),
                Some("true" | "1" | "yes") => Ok(true),
                Some("false" | "0" | "no") => Ok(false),
                Some(v) => bail!("{k}: expected true or false, got {v:?}"),
            }
        };
        let proxy = match var("KILN_PROXY").as_deref().map(str::trim) {
            None | Some("none") => ProxyMode::None,
            Some("velocity") => {
                let secret = match (var("KILN_VELOCITY_SECRET"), var("KILN_VELOCITY_SECRET_FILE")) {
                    (Some(s), _) => s,
                    (None, Some(path)) => std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?,
                    (None, None) => {
                        bail!("KILN_PROXY=velocity needs KILN_VELOCITY_SECRET or KILN_VELOCITY_SECRET_FILE")
                    }
                };
                let secret = secret.trim();
                if secret.is_empty() {
                    bail!("the Velocity forwarding secret is empty");
                }
                ProxyMode::Velocity { secret: secret.as_bytes().to_vec() }
            }
            Some("bungeecord") => {
                let tokens = var("KILN_BUNGEEGUARD_TOKENS").unwrap_or_default();
                let tokens = tokens.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect();
                ProxyMode::BungeeCord { tokens }
            }
            Some(v) => bail!("KILN_PROXY: expected none, velocity or bungeecord, got {v:?}"),
        };
        let proxy_compression_threshold = match var("KILN_PROXY_COMPRESSION_THRESHOLD") {
            None => None,
            Some(v) => match v.trim().parse::<i32>() {
                Ok(n) if n < 0 => None,
                Ok(n) => Some(n as usize),
                Err(_) => bail!("KILN_PROXY_COMPRESSION_THRESHOLD: expected a number, got {v:?}"),
            },
        };
        Ok(Self {
            online_mode: flag("KILN_ONLINE_MODE")?,
            prevent_proxy_connections: flag("KILN_PREVENT_PROXY_CONNECTIONS")?,
            proxy,
            proxy_compression_threshold,
        })
    }
}

/// Online-mode state: the RSA key pair and the session server client.
struct Authenticator {
    key: ServerKey,
    session: Arc<SessionService>,
}

pub struct Shared {
    pub config: Config,
    pub login: LoginConfig,
    /// Resource pack, transfers, code of conduct and links.
    pub lobby: LobbyConfig,
    /// Players in the game or past the login capacity check.
    pub online: AtomicUsize,
    pub to_sim: crossbeam_channel::Sender<ToSim>,
    next_conn: AtomicU64,
    registry_packets: Vec<Bytes>,
    tags_packet: Bytes,
    auth: Option<Authenticator>,
}

impl Shared {
    /// Takes the login settings from the environment (`LoginConfig::from_env`) and panics if
    /// they are invalid: silently falling back to offline mode would let anyone in.
    pub fn new(config: Config, to_sim: crossbeam_channel::Sender<ToSim>) -> Self {
        let login = LoginConfig::from_env().unwrap_or_else(|e| panic!("invalid login settings: {e:#}"));
        let lobby = LobbyConfig::from_env().unwrap_or_else(|e| panic!("invalid server settings: {e:#}"));
        Self::with_login(config, login, to_sim).with_lobby(lobby)
    }

    pub fn with_lobby(mut self, lobby: LobbyConfig) -> Self {
        self.lobby = lobby;
        self
    }

    /// `MinecraftServer.isResourcePackRequired`.
    pub fn resource_pack_required(&self) -> bool {
        self.lobby.resource_pack.as_ref().is_some_and(|p| p.required)
    }

    pub fn with_login(config: Config, login: LoginConfig, to_sim: crossbeam_channel::Sender<ToSim>) -> Self {
        // Encoded once and reused for every login.
        let registry_packets = kiln_data::registries::SYNCHRONIZED
            .iter()
            .map(|(reg, entries)| packets::registry_data(reg, entries))
            .collect();
        let tags_packet =
            packets::update_tags(ids::configuration::clientbound::UPDATE_TAGS, kiln_data::registries::TAGS);
        let auth = (login.online_mode && matches!(login.proxy, ProxyMode::None)).then(|| Authenticator {
            key: ServerKey::generate().expect("RSA key generation"),
            session: Arc::new(SessionService::default()),
        });
        Self {
            config,
            login,
            lobby: LobbyConfig::default(),
            online: AtomicUsize::new(0),
            to_sim,
            next_conn: AtomicU64::new(1),
            registry_packets,
            tags_packet,
            auth,
        }
    }

    /// Whether Kiln itself authenticates players with the session server, i.e. the value for
    /// the `onlineMode` flag of the play Login packet (a proxy rewrites it for its clients).
    pub fn authenticates(&self) -> bool {
        self.auth.is_some()
    }

    fn compression_threshold(&self) -> Option<usize> {
        match self.login.proxy {
            ProxyMode::None => self.config.compression_threshold,
            _ => self.login.proxy_compression_threshold,
        }
    }

    /// Logs how players will log in, with warnings for risky or ignored settings.
    fn log_login_mode(&self) {
        let threshold = self.compression_threshold().map_or("off".to_string(), |t| t.to_string());
        let mode = match &self.login.proxy {
            ProxyMode::None if self.authenticates() => "online mode".to_string(),
            ProxyMode::None => "offline mode".to_string(),
            ProxyMode::Velocity { .. } => "Velocity modern forwarding".to_string(),
            ProxyMode::BungeeCord { tokens } if tokens.is_empty() => "BungeeCord legacy forwarding".to_string(),
            ProxyMode::BungeeCord { tokens } => {
                format!("BungeeCord legacy forwarding, {} BungeeGuard token(s)", tokens.len())
            }
        };
        info!("login: {mode}, compression threshold {threshold}");
        match &self.login.proxy {
            ProxyMode::BungeeCord { tokens } if tokens.is_empty() => {
                warn!("BungeeCord forwarding without BungeeGuard tokens: only the proxy may reach this port")
            }
            ProxyMode::Velocity { .. } | ProxyMode::BungeeCord { .. } if self.login.online_mode => {
                info!("online mode is left to the proxy")
            }
            _ => {}
        }
    }
}

/// Messages from the simulation to a connection's writer.
enum Outbound {
    Packet(Bytes),
    /// A tick's packets for this connection, in order.
    Batch(Vec<Bytes>),
    /// Send this packet, then close the connection.
    Disconnect(Bytes),
    /// The client fell too far behind; close without sending more.
    Overflow,
}

impl Outbound {
    fn len(&self) -> usize {
        match self {
            Outbound::Packet(p) | Outbound::Disconnect(p) => p.len(),
            Outbound::Batch(ps) => ps.iter().map(Bytes::len).sum(),
            Outbound::Overflow => 0,
        }
    }
}

/// Unsent packet bytes a connection may have queued before it is closed as too slow. Chunks,
/// the bulk of the traffic, are already paced by the client's chunk batch acknowledgements.
const EGRESS_LIMIT: usize = 64 << 20;

struct ChannelSink {
    tx: mpsc::UnboundedSender<Outbound>,
    /// Bytes handed to the writer and not yet encoded.
    queued: Arc<AtomicUsize>,
}

impl ChannelSink {
    fn push(&self, msg: Outbound) {
        let n = msg.len();
        let msg = if self.queued.fetch_add(n, Ordering::Relaxed) + n > EGRESS_LIMIT { Outbound::Overflow } else { msg };
        let _ = self.tx.send(msg);
    }
}

impl Sink for ChannelSink {
    fn send(&self, packet: Bytes) {
        self.push(Outbound::Packet(packet));
    }
    fn send_batch(&self, packets: Vec<Bytes>) {
        self.push(Outbound::Batch(packets));
    }
    fn disconnect(&self, packet: Bytes) {
        self.push(Outbound::Disconnect(packet));
    }
}

const PRE_PLAY_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn listen(shared: Arc<Shared>) -> Result<()> {
    let listener = TcpListener::bind(shared.config.bind).await?;
    info!("listening on {}", shared.config.bind);
    shared.log_login_mode();
    loop {
        let (stream, addr) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let shared = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, addr, shared).await {
                debug!("{addr}: {e:#}");
            }
        });
    }
}

struct Conn {
    stream: TcpStream,
    rbuf: BytesMut,
    rx: FrameCodec,
    tx: FrameCodec,
    wbuf: BytesMut,
    encrypt: Option<Encryptor>,
    decrypt: Option<Decryptor>,
}

impl Conn {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            rbuf: BytesMut::with_capacity(4096),
            rx: FrameCodec::new(),
            tx: FrameCodec::new(),
            wbuf: BytesMut::with_capacity(4096),
            encrypt: None,
            decrypt: None,
        }
    }

    async fn read(&mut self) -> Result<BytesMut> {
        self.read_within(PRE_PLAY_TIMEOUT).await?.context("timed out")
    }

    /// The next packet, or `None` if none arrives within `limit` (partial data is kept).
    async fn read_within(&mut self, limit: Duration) -> Result<Option<BytesMut>> {
        loop {
            if let Some(p) = self.rx.decode(&mut self.rbuf)? {
                return Ok(Some(p));
            }
            let start = self.rbuf.len();
            let Ok(n) = tokio::time::timeout(limit, self.stream.read_buf(&mut self.rbuf)).await else {
                return Ok(None);
            };
            let n = n?;
            if n == 0 {
                bail!("connection closed");
            }
            if let Some(d) = &mut self.decrypt {
                d.apply(&mut self.rbuf[start..]);
            }
        }
    }

    fn queue(&mut self, pkt: &[u8]) -> Result<()> {
        self.tx.encode(pkt, &mut self.wbuf)?;
        Ok(())
    }

    async fn flush(&mut self) -> Result<()> {
        if let Some(e) = &mut self.encrypt {
            e.apply(&mut self.wbuf);
        }
        self.stream.write_all(&self.wbuf).await?;
        self.wbuf.clear();
        Ok(())
    }

    async fn send(&mut self, pkt: &[u8]) -> Result<()> {
        self.queue(pkt)?;
        self.flush().await
    }

    /// Switches both directions to AES/CFB8 with `secret`, right after the Encryption Response.
    fn enable_encryption(&mut self, secret: &[u8; 16]) {
        debug_assert!(self.wbuf.is_empty(), "queued plaintext would be encrypted");
        let (encrypt, mut decrypt) = cipher::pair(secret);
        // Bytes already buffered arrived after the Encryption Response, so they are ciphertext.
        decrypt.apply(&mut self.rbuf);
        self.encrypt = Some(encrypt);
        self.decrypt = Some(decrypt);
    }
}

fn split_id(pkt: &[u8]) -> Result<(i32, Reader<'_>)> {
    let mut r = Reader::new(pkt);
    let id = r.varint()?;
    Ok((id, r))
}

async fn handle(stream: TcpStream, addr: SocketAddr, shared: Arc<Shared>) -> Result<()> {
    let mut conn = Conn::new(stream);
    let pkt = conn.read().await?;
    let (id, mut r) = split_id(&pkt)?;
    if id != ids::handshake::serverbound::INTENTION {
        bail!("expected handshake, got packet {id}");
    }
    let protocol = r.varint()?;
    // Legacy forwarding appends the player's address, UUID and properties to the host.
    let max_host = match shared.login.proxy {
        ProxyMode::BungeeCord { .. } => proxy::MAX_FORWARDED_HOST,
        _ => proxy::MAX_HOST,
    };
    let host = r.string(max_host)?.to_owned();
    let _port = r.u16()?;
    let intent = r.varint()?;
    r.finish()?;

    match intent {
        1 => status(&mut conn, &shared, protocol).await,
        2 => login(conn, addr, &shared, protocol, &host).await,
        // `ServerHandshakePacketListenerImpl`: refused before login when transfers are off.
        3 if !shared.lobby.accepts_transfers => {
            conn.send(&login_ext::login_disconnect_translated("multiplayer.disconnect.transfers_disabled")).await?;
            bail!("transfer refused: accepts-transfers is off")
        }
        3 => login(conn, addr, &shared, protocol, &host).await,
        n => bail!("unknown handshake intent {n}"),
    }
}

async fn status(conn: &mut Conn, shared: &Shared, _client_protocol: i32) -> Result<()> {
    loop {
        let pkt = conn.read().await?;
        let (id, mut r) = split_id(&pkt)?;
        match id {
            ids::status::serverbound::STATUS_REQUEST => {
                let json = serde_json::json!({
                    "version": { "name": version::NAME, "protocol": version::PROTOCOL },
                    "players": {
                        "max": shared.config.max_players,
                        "online": shared.online.load(Ordering::Relaxed),
                        "sample": [],
                    },
                    "description": { "text": shared.config.motd },
                    "enforcesSecureChat": false,
                });
                conn.send(&packets::status_response(&json.to_string())).await?;
            }
            ids::status::serverbound::PING_REQUEST => {
                let ts = r.i64()?;
                conn.send(&packets::status_pong(ts)).await?;
                return Ok(());
            }
            other => bail!("unexpected status packet {other}"),
        }
    }
}

async fn login(mut conn: Conn, addr: SocketAddr, shared: &Shared, protocol: i32, host: &str) -> Result<()> {
    let pkt = conn.read().await?;
    let (id, mut r) = split_id(&pkt)?;
    if id != ids::login::serverbound::HELLO {
        bail!("expected login start, got packet {id}");
    }
    let name = login_ext::read_login_start(&mut r)?.name.to_owned();

    if protocol != version::PROTOCOL {
        let msg = format!("Kiln runs Minecraft {} (protocol {}).", version::NAME, version::PROTOCOL);
        conn.send(&packets::login_disconnect(&msg)).await?;
        bail!("{name}: unsupported protocol {protocol}");
    }
    if !valid_name(&name) {
        conn.send(&packets::login_disconnect("Invalid username.")).await?;
        bail!("invalid username {name:?}");
    }
    // Reserved until the connection ends, so concurrent logins cannot overfill the server.
    let Some(_slot) = PlayerSlot::reserve(shared) else {
        conn.send(&packets::login_disconnect("The server is full.")).await?;
        bail!("{name}: server full");
    };

    let (profile, remote, via) = match &shared.login.proxy {
        ProxyMode::Velocity { secret } => {
            let fwd = velocity_forwarding(&mut conn, secret, &name, addr).await?;
            (fwd.profile, fwd.address, " via Velocity")
        }
        ProxyMode::BungeeCord { tokens } => match proxy::bungee_parse(host, &name, tokens) {
            Ok(fwd) => (fwd.profile, fwd.address, " via BungeeCord"),
            Err(e) => {
                let msg = match e {
                    ForwardError::Missing => "This server only accepts connections through its proxy.",
                    _ => "Unable to verify player details.",
                };
                conn.send(&packets::login_disconnect(msg)).await?;
                return Err(refused(&name, addr, "BungeeCord forwarding", &e));
            }
        },
        ProxyMode::None => match &shared.auth {
            Some(auth) => (authenticate(&mut conn, shared, auth, &name, addr).await?, addr.ip(), ""),
            None => (GameProfile { uuid: offline_uuid(&name), name, properties: Vec::new() }, addr.ip(), ""),
        },
    };

    if let Some(t) = shared.compression_threshold() {
        conn.send(&packets::login_compression(t as i32)).await?;
        conn.rx.set_threshold(Some(t));
        conn.tx.set_threshold(Some(t));
    }
    let finished = packets::login_finished(profile.uuid, &profile.name, &profile.wire_properties(), Uuid::new_v4());
    conn.send(&finished).await?;

    let pkt = conn.read().await?;
    let (id, _) = split_id(&pkt)?;
    if id != ids::login::serverbound::LOGIN_ACKNOWLEDGED {
        bail!("expected login acknowledged, got packet {id}");
    }

    let client = configure(&mut conn, shared, &profile.name).await?;
    info!(
        "{} ({}) joined from {remote}{via}, {} profile properties",
        profile.name,
        profile.uuid,
        profile.properties.len()
    );
    play(conn, shared, profile, remote, client).await
}

/// Velocity modern forwarding: ask for the player's details on `velocity:player_info` and
/// accept only an answer signed with the shared secret.
async fn velocity_forwarding(conn: &mut Conn, secret: &[u8], name: &str, addr: SocketAddr) -> Result<Forwarded> {
    let transaction = i32::from_be_bytes(auth::random_bytes()) & i32::MAX;
    conn.send(&login_ext::custom_query(transaction, proxy::VELOCITY_CHANNEL, &proxy::velocity_request())).await?;
    let pkt = conn.read().await?;
    let (id, mut r) = split_id(&pkt)?;
    let result = if id != ids::login::serverbound::CUSTOM_QUERY_ANSWER {
        Err(ForwardError::Missing)
    } else {
        match login_ext::read_custom_query_answer(&mut r)? {
            (answer, Some(payload)) if answer == transaction => proxy::velocity_verify(secret, payload),
            _ => Err(ForwardError::Missing),
        }
    };
    match result {
        Ok(fwd) => {
            let p = &fwd.profile;
            debug!("{addr}: Velocity vouches for {} ({}) connecting from {}", p.name, p.uuid, fwd.address);
            Ok(fwd)
        }
        Err(e) => {
            let msg = match e {
                ForwardError::Missing => "This server requires you to connect with Velocity.",
                _ => "Unable to verify player details.",
            };
            conn.send(&packets::login_disconnect(msg)).await?;
            Err(refused(name, addr, "Velocity forwarding", &e))
        }
    }
}

/// Logs a login refused over forwarding data, loudly when it points at a wrong secret or
/// token or at someone forging it, and returns the error that ends the connection.
fn refused(name: &str, addr: SocketAddr, what: &str, e: &ForwardError) -> anyhow::Error {
    let err = anyhow!("refused {name} from {addr}: {what}: {e}");
    match e {
        ForwardError::Missing => info!("{err}"),
        _ => warn!("{err}"),
    }
    err
}

/// Online mode: encryption handshake, then the session server confirms the client joined
/// with our server hash and supplies the profile.
async fn authenticate(
    conn: &mut Conn,
    shared: &Shared,
    auth: &Authenticator,
    name: &str,
    addr: SocketAddr,
) -> Result<GameProfile> {
    let challenge: [u8; 4] = auth::random_bytes();
    conn.send(&login_ext::encryption_request("", auth.key.public_der(), &challenge, true)).await?;
    let pkt = conn.read().await?;
    let (id, mut r) = split_id(&pkt)?;
    if id != ids::login::serverbound::KEY {
        bail!("{name}: expected encryption response, got packet {id}");
    }
    let response = login_ext::read_encryption_response(&mut r)?;
    let secret: [u8; 16] = match (auth.key.decrypt(response.challenge), auth.key.decrypt(response.shared_secret)) {
        (Ok(c), Ok(s)) if c == challenge => {
            s.try_into().map_err(|_| anyhow!("{name}: shared secret is not 16 bytes"))?
        }
        _ => bail!("{name}: invalid encryption response"),
    };
    conn.enable_encryption(&secret);

    let hash = auth::server_hash("", &secret, auth.key.public_der());
    let ip = shared.login.prevent_proxy_connections.then_some(addr.ip());
    let (session, user) = (auth.session.clone(), name.to_owned());
    let lookup = tokio::task::spawn_blocking(move || session.has_joined(&user, &hash, ip));
    let result = match tokio::time::timeout(auth::AUTH_TIMEOUT, lookup).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(anyhow!("session lookup failed: {e}")),
        Err(_) => Err(anyhow!("session server timed out")),
    };
    let reason = match result {
        Ok(Some(profile)) => match auth::verify_profile(&profile, name) {
            Ok(()) => return Ok(profile),
            Err(why) => anyhow!("{name} from {addr}: {why}"),
        },
        Ok(None) => anyhow!("{name} from {addr} failed to verify username"),
        Err(e) => {
            let e = e.context(format!("{name} from {addr}: authentication servers unavailable"));
            warn!("{e:#}");
            conn.send(&login_ext::login_disconnect_translated("multiplayer.disconnect.authservers_down")).await?;
            return Err(e);
        }
    };
    info!("{reason}");
    conn.send(&login_ext::login_disconnect_translated("multiplayer.disconnect.unverified_username")).await?;
    Err(reason)
}

/// The configuration tasks after the registries (`ServerConfigurationPacketListenerImpl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigTask {
    /// Sent the code of conduct; waiting for `accept_code_of_conduct`.
    CodeOfConduct,
    /// Pushed the server resource pack; waiting for a terminal status.
    ResourcePack,
}

/// Configuration phase (`startConfiguration`): brand, server links, features and the known
/// packs, then the tasks in vanilla's order: registries, code of conduct, resource pack.
/// Returns the client's settings.
async fn configure(conn: &mut Conn, shared: &Shared, name: &str) -> Result<packets::ClientInfo> {
    use ids::configuration::serverbound as sb;
    use packets::common::{self, Phase};
    let core = ("minecraft", "core", version::NAME);
    conn.queue(&packets::config_brand("kiln"))?;
    if !shared.lobby.links.is_empty() {
        conn.queue(&server_links(Phase::Configuration, &shared.lobby))?;
    }
    conn.queue(&packets::update_enabled_features(&["minecraft:vanilla"]))?;
    conn.queue(&packets::select_known_packs(&[core]))?;
    conn.flush().await?;

    let mut client = packets::ClientInfo { view_distance: shared.config.view_distance, ..Default::default() };
    let mut language = String::from("en_us");
    let mut registries_sent = false;
    let mut tasks: Vec<ConfigTask> = Vec::new();
    // `keepConnectionAlive`: a player may read the code of conduct or download the pack for
    // as long as the client keeps answering keep-alives.
    let mut keep_alive: Option<i64> = None;
    loop {
        let Some(pkt) = conn.read_within(KEEP_ALIVE_INTERVAL).await? else {
            if keep_alive.is_some() || tasks.is_empty() {
                let reason = kiln_proto::nbt::Tag::Compound(vec![(
                    "translate".into(),
                    kiln_proto::nbt::Tag::String("disconnect.timeout".into()),
                )]);
                conn.send(&packets::config_disconnect_text(&reason)).await?;
                bail!("timed out while configuring");
            }
            let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as i64);
            conn.send(&common::keep_alive_configuration(id)).await?;
            keep_alive = Some(id);
            continue;
        };
        let (id, mut r) = split_id(&pkt)?;
        match id {
            sb::KEEP_ALIVE => {
                let answer = r.i64()?;
                if keep_alive != Some(answer) {
                    let reason = kiln_proto::nbt::Tag::Compound(vec![(
                        "translate".into(),
                        kiln_proto::nbt::Tag::String("disconnect.timeout".into()),
                    )]);
                    conn.send(&packets::config_disconnect_text(&reason)).await?;
                    bail!("wrong keep-alive answer while configuring");
                }
                keep_alive = None;
            }
            sb::CLIENT_INFORMATION => {
                let mut peek = r;
                language = peek.string(16).map(str::to_owned).unwrap_or(language);
                client = packets::read_client_information(&mut r)?;
            }
            sb::ACCEPT_CODE_OF_CONDUCT if tasks.first() == Some(&ConfigTask::CodeOfConduct) => {
                tasks.remove(0);
                if next_task(conn, shared, &tasks, &language).await? {
                    return Ok(client);
                }
            }
            sb::RESOURCE_PACK => {
                let (pack, action) = common::read_resource_pack_response(&mut r)?;
                // `ServerCommonPacketListenerImpl.handleResourcePackResponse`.
                if action == common::ResourcePackAction::Declined && shared.resource_pack_required() {
                    info!("Disconnecting {name} due to resource pack {pack} rejection");
                    let reason = kiln_proto::nbt::Tag::Compound(vec![(
                        "translate".into(),
                        kiln_proto::nbt::Tag::String("multiplayer.requiredTexturePrompt.disconnect".into()),
                    )]);
                    conn.send(&packets::config_disconnect_text(&reason)).await?;
                    bail!("declined the required resource pack");
                }
                if action.is_terminal() && tasks.first() == Some(&ConfigTask::ResourcePack) {
                    tasks.remove(0);
                    if next_task(conn, shared, &tasks, &language).await? {
                        return Ok(client);
                    }
                }
            }
            sb::COOKIE_RESPONSE => {
                // Kiln asks for no cookies while configuring (`handleCookieResponse`).
                let reason = kiln_proto::nbt::Tag::Compound(vec![(
                    "translate".into(),
                    kiln_proto::nbt::Tag::String("multiplayer.disconnect.unexpected_query_response".into()),
                )]);
                conn.send(&packets::config_disconnect_text(&reason)).await?;
                bail!("unexpected cookie response");
            }
            sb::SELECT_KNOWN_PACKS if !registries_sent => {
                let n = r.len()?;
                let mut knows_core = false;
                for _ in 0..n.min(64) {
                    let pack = (r.string(32767)?, r.string(32767)?, r.string(32767)?);
                    knows_core |= pack == core;
                }
                if !knows_core {
                    let msg = format!("Kiln requires the Minecraft {} client.", version::NAME);
                    conn.send(&packets::config_disconnect(&msg)).await?;
                    bail!("client does not know {core:?}");
                }
                for p in &shared.registry_packets {
                    conn.queue(p)?;
                }
                conn.queue(&shared.tags_packet)?;
                registries_sent = true;
                if shared.lobby.code_of_conduct_for(&language).is_some() {
                    tasks.push(ConfigTask::CodeOfConduct);
                }
                if shared.lobby.resource_pack.is_some() {
                    tasks.push(ConfigTask::ResourcePack);
                }
                if next_task(conn, shared, &tasks, &language).await? {
                    return Ok(client);
                }
            }
            sb::FINISH_CONFIGURATION if registries_sent && tasks.is_empty() => return Ok(client),
            sb::CUSTOM_PAYLOAD | sb::PONG => {}
            other => debug!("ignoring configuration packet {other}"),
        }
    }
}

/// Vanilla's keep-alive period (`ServerCommonPacketListenerImpl.LATENCY_CHECK_INTERVAL`).
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Starts the first pending configuration task, or ends configuration when none is left
/// (`JoinWorldTask` sends `finish_configuration`). Returns whether configuration is over for
/// the server; the client still answers with `finish_configuration`.
async fn next_task(conn: &mut Conn, shared: &Shared, tasks: &[ConfigTask], language: &str) -> Result<bool> {
    use packets::common::{self, Phase};
    match tasks.first() {
        Some(ConfigTask::CodeOfConduct) => {
            let text = shared.lobby.code_of_conduct_for(language).unwrap_or_default();
            conn.send(&common::code_of_conduct(text)).await?;
        }
        Some(ConfigTask::ResourcePack) => {
            let pack = shared.lobby.resource_pack.as_ref().expect("resource pack task without a pack");
            conn.send(&resource_pack_push(Phase::Configuration, pack)).await?;
        }
        None => {
            conn.send(&packets::finish_configuration()).await?;
            // The client's `finish_configuration` answer is read by the caller's loop.
            return wait_finish(conn).await;
        }
    }
    Ok(false)
}

/// After `finish_configuration`: reads until the client acknowledges it.
async fn wait_finish(conn: &mut Conn) -> Result<bool> {
    use ids::configuration::serverbound as sb;
    loop {
        let pkt = conn.read().await?;
        let (id, _) = split_id(&pkt)?;
        match id {
            sb::FINISH_CONFIGURATION => return Ok(true),
            sb::CUSTOM_PAYLOAD | sb::KEEP_ALIVE | sb::PONG | sb::CLIENT_INFORMATION | sb::RESOURCE_PACK => {}
            other => debug!("ignoring configuration packet {other}"),
        }
    }
}

/// The server resource pack push (`ServerResourcePackConfigurationTask`).
pub fn resource_pack_push(phase: packets::common::Phase, pack: &lobby::ServerResourcePack) -> Bytes {
    use packets::common::{self, ResourcePack};
    common::resource_pack_push(
        phase,
        &ResourcePack { id: pack.id, url: &pack.url, hash: &pack.hash, required: pack.required, prompt: pack.prompt.as_ref() },
    )
}

/// `ClientboundServerLinksPacket` for the configured links.
pub fn server_links(phase: packets::common::Phase, lobby: &LobbyConfig) -> Bytes {
    use packets::common::{self, KnownLink, LinkLabel};
    const KNOWN: [KnownLink; 10] = [
        KnownLink::BugReport,
        KnownLink::CommunityGuidelines,
        KnownLink::Support,
        KnownLink::Status,
        KnownLink::Feedback,
        KnownLink::Community,
        KnownLink::Website,
        KnownLink::Forums,
        KnownLink::News,
        KnownLink::Announcements,
    ];
    let links: Vec<(LinkLabel, &str)> = lobby
        .links
        .iter()
        .map(|(kind, url)| {
            let label = match kind {
                lobby::LinkKind::Known(i) => LinkLabel::Known(KNOWN[*i as usize]),
                lobby::LinkKind::Custom(t) => LinkLabel::Custom(t),
            };
            (label, url.as_str())
        })
        .collect();
    common::server_links(phase, &links)
}

/// One of `max_players` places, held from login until the connection closes.
struct PlayerSlot<'a>(&'a AtomicUsize);

impl<'a> PlayerSlot<'a> {
    fn reserve(shared: &'a Shared) -> Option<Self> {
        let max = shared.config.max_players;
        shared.online.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < max).then_some(n + 1)).ok()?;
        Some(Self(&shared.online))
    }
}

impl Drop for PlayerSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn play(conn: Conn, shared: &Shared, profile: GameProfile, remote: IpAddr, client: packets::ClientInfo) -> Result<()> {
    let conn_id = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    let Conn { stream, mut rbuf, mut rx, tx, wbuf, encrypt, mut decrypt } = conn;
    let (reader, writer) = stream.into_split();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let name = profile.name.clone();

    let joined = shared
        .to_sim
        .send(ToSim::Join(JoinInfo {
            conn: conn_id,
            name: profile.name,
            uuid: profile.uuid,
            properties: profile.properties,
            client,
            sink: Box::new(ChannelSink { tx: out_tx, queued: queued.clone() }),
        }))
        .is_ok();

    let mut writer_task = tokio::spawn(write_loop(writer, tx, wbuf, encrypt, out_rx, queued));
    let result = if joined {
        // The writer ends after a kick (disconnect packet sent) or when the client falls behind.
        tokio::select! {
            r = read_loop(reader, &mut rbuf, &mut rx, decrypt.as_mut(), conn_id, &shared.to_sim) => r,
            w = &mut writer_task => w.map_err(|e| anyhow!("writer: {e}")).and_then(|r| r),
        }
    } else {
        Err(anyhow!("simulation is not running"))
    };

    let _ = shared.to_sim.send(ToSim::Leave(conn_id));
    writer_task.abort();
    let why = result.as_ref().err().map_or("disconnected".to_string(), |e| format!("{e:#}"));
    info!("{name} ({remote}) left: {why}");
    Ok(())
}

async fn read_loop(
    mut reader: OwnedReadHalf,
    rbuf: &mut BytesMut,
    rx: &mut FrameCodec,
    mut decrypt: Option<&mut Decryptor>,
    conn: ConnId,
    to_sim: &crossbeam_channel::Sender<ToSim>,
) -> Result<()> {
    loop {
        while let Some(pkt) = rx.decode(rbuf)? {
            let (id, mut r) = split_id(&pkt)?;
            match packets::decode_play(id, &mut r) {
                Ok(Some(p)) => {
                    if to_sim.send(ToSim::Packet(conn, p)).is_err() {
                        bail!("simulation stopped");
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("bad play packet {id}: {e}");
                    bail!("malformed packet");
                }
            }
        }
        let start = rbuf.len();
        if reader.read_buf(rbuf).await? == 0 {
            bail!("connection closed");
        }
        if let Some(d) = decrypt.as_deref_mut() {
            d.apply(&mut rbuf[start..]);
        }
    }
}

async fn write_loop(
    mut writer: OwnedWriteHalf,
    mut tx: FrameCodec,
    mut wbuf: BytesMut,
    mut encrypt: Option<Encryptor>,
    mut out: mpsc::UnboundedReceiver<Outbound>,
    queued: Arc<AtomicUsize>,
) -> Result<()> {
    while let Some(msg) = out.recv().await {
        let mut close = false;
        let mut next = Some(msg);
        // Coalesce everything queued so far into one write.
        while let Some(msg) = next {
            queued.fetch_sub(msg.len(), Ordering::Relaxed);
            match msg {
                Outbound::Packet(p) => tx.encode(&p, &mut wbuf)?,
                Outbound::Batch(ps) => {
                    for p in &ps {
                        tx.encode(p, &mut wbuf)?;
                    }
                }
                Outbound::Disconnect(p) => {
                    tx.encode(&p, &mut wbuf)?;
                    close = true;
                    break;
                }
                Outbound::Overflow => bail!("more than {} MiB of packets waiting to be sent", EGRESS_LIMIT >> 20),
            }
            next = out.try_recv().ok();
        }
        if let Some(e) = &mut encrypt {
            e.apply(&mut wbuf);
        }
        writer.write_all(&wbuf).await?;
        wbuf.clear();
        if close {
            writer.shutdown().await?;
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
