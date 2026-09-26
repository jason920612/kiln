//! Connection handling on the tokio runtime: handshake, status, login and configuration run
//! here; once a player reaches play state, packets are relayed to and from the simulation.

mod auth;
mod cipher;
mod profile;
pub mod proxy;

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
        Self::with_login(config, login, to_sim)
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
    /// Send this packet, then close the connection.
    Disconnect(Bytes),
}

struct ChannelSink(mpsc::UnboundedSender<Outbound>);

impl Sink for ChannelSink {
    fn send(&self, packet: Bytes) {
        let _ = self.0.send(Outbound::Packet(packet));
    }
    fn disconnect(&self, packet: Bytes) {
        let _ = self.0.send(Outbound::Disconnect(packet));
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
        loop {
            if let Some(p) = self.rx.decode(&mut self.rbuf)? {
                return Ok(p);
            }
            let start = self.rbuf.len();
            let n = tokio::time::timeout(PRE_PLAY_TIMEOUT, self.stream.read_buf(&mut self.rbuf))
                .await
                .context("timed out")??;
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
        2 | 3 => login(conn, addr, &shared, protocol, &host).await,
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
    if shared.online.load(Ordering::Relaxed) >= shared.config.max_players {
        conn.send(&packets::login_disconnect("The server is full.")).await?;
        bail!("{name}: server full");
    }

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

    let client = configure(&mut conn, shared).await?;
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

/// Configuration phase; returns the client's view distance.
async fn configure(conn: &mut Conn, shared: &Shared) -> Result<packets::ClientInfo> {
    use ids::configuration::serverbound as sb;
    let core = ("minecraft", "core", version::NAME);
    conn.queue(&packets::config_brand("kiln"))?;
    conn.queue(&packets::update_enabled_features(&["minecraft:vanilla"]))?;
    conn.queue(&packets::select_known_packs(&[core]))?;
    conn.flush().await?;

    let mut client = packets::ClientInfo { view_distance: shared.config.view_distance, ..Default::default() };
    let mut registries_sent = false;
    loop {
        let pkt = conn.read().await?;
        let (id, mut r) = split_id(&pkt)?;
        match id {
            sb::CLIENT_INFORMATION => client = packets::read_client_information(&mut r)?,
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
                conn.queue(&packets::finish_configuration())?;
                conn.flush().await?;
                registries_sent = true;
            }
            sb::FINISH_CONFIGURATION if registries_sent => return Ok(client),
            sb::CUSTOM_PAYLOAD | sb::KEEP_ALIVE | sb::PONG | sb::RESOURCE_PACK => {}
            other => debug!("ignoring configuration packet {other}"),
        }
    }
}

async fn play(conn: Conn, shared: &Shared, profile: GameProfile, remote: IpAddr, client: packets::ClientInfo) -> Result<()> {
    let conn_id = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    let Conn { stream, mut rbuf, mut rx, tx, wbuf, encrypt, mut decrypt } = conn;
    let (reader, writer) = stream.into_split();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let name = profile.name.clone();

    shared.online.fetch_add(1, Ordering::Relaxed);
    let joined = shared
        .to_sim
        .send(ToSim::Join(JoinInfo {
            conn: conn_id,
            name: profile.name,
            uuid: profile.uuid,
            properties: profile.properties,
            client,
            sink: Box::new(ChannelSink(out_tx)),
        }))
        .is_ok();

    let writer_task = tokio::spawn(write_loop(writer, tx, wbuf, encrypt, out_rx));
    let result = if joined {
        read_loop(reader, &mut rbuf, &mut rx, decrypt.as_mut(), conn_id, &shared.to_sim).await
    } else {
        Err(anyhow!("simulation is not running"))
    };

    let _ = shared.to_sim.send(ToSim::Leave(conn_id));
    shared.online.fetch_sub(1, Ordering::Relaxed);
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
) -> Result<()> {
    while let Some(msg) = out.recv().await {
        let mut close = false;
        let mut next = Some(msg);
        // Coalesce everything queued so far into one write.
        while let Some(msg) = next {
            match msg {
                Outbound::Packet(p) => tx.encode(&p, &mut wbuf)?,
                Outbound::Disconnect(p) => {
                    tx.encode(&p, &mut wbuf)?;
                    close = true;
                    break;
                }
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
