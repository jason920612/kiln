//! The management server: vanilla's `net.minecraft.server.jsonrpc` (`management-server-enabled`). A
//! JSON-RPC 2.0 server over WebSocket, TLS by default and authenticated by a secret, with methods for
//! the allowlist, bans, operators, players, the server's state, settings and game rules, and
//! notifications of what happens on the server. See `docs/wp86-mgmt.md`.

mod codec;
mod conn;
mod exec;
mod hub;
mod json;
mod rpc;
mod spec;
mod tls;
mod ws;

#[cfg(test)]
mod tests;

use crate::properties::ServerProperties;
use kiln_link::access::SharedAccess;
use kiln_link::mgmt::{MgmtHost, Notifier};
use kiln_link::{ProfileLookup, ServerInfo};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{error, info};
use uuid::Uuid;

/// How a request fails (the exceptions of `Connection.handleIncomingRequest`).
#[derive(Debug)]
pub enum Fail {
    /// -32602, the text of the codec's error.
    InvalidParams(String),
    /// -32603 with the exception's message (`EncodeJsonRpcException`).
    Encode(String),
    /// -32600.
    InvalidRequest(String),
    /// -32601.
    MethodNotFound(String),
    /// -32603 without data: anything else that went wrong.
    Internal(String),
}

/// Runs work on the simulation thread.
pub trait Runner: Send + Sync {
    /// Queues `job` for the simulation; false if it is gone.
    fn run(&self, job: Box<dyn FnOnce(&mut dyn MgmtHost) + Send>) -> bool;
}

/// `server.properties` as the management server changes it: every setting it sets is written back to the file.
pub struct Settings {
    pub props: Mutex<ServerProperties>,
    pub path: Option<PathBuf>,
}

impl Settings {
    pub fn save(&self, props: &ServerProperties) {
        if let Some(path) = &self.path {
            props.store(path);
        }
    }
}

/// What the methods work with.
pub struct Ctx {
    runner: Arc<dyn Runner>,
    pub info: Arc<ServerInfo>,
    pub access: SharedAccess,
    pub settings: Arc<Settings>,
    lookup: Option<Arc<dyn ProfileLookup>>,
    online_mode: bool,
    pub hub: Arc<hub::Hub>,
    heartbeat: Arc<hub::Heartbeat>,
    /// Profiles looked up so far (`usercache.json`'s role).
    users: Mutex<HashMap<Uuid, kiln_link::access::NameAndId>>,
    /// `NotificationManager.server() != null`: the world is loaded and the server was set up.
    server_set: AtomicBool,
}

impl Ctx {
    /// `JsonRpcLogger.log`: `RPC Connection #<id>: <message>`.
    pub fn log(&self, client: u32, message: &str) {
        info!("RPC Connection #{client}: {message}");
    }

    /// Whether requests other than `rpc.discover` are served yet.
    pub fn server_set(&self) -> bool {
        if self.info.ready.load(Ordering::Relaxed) {
            self.server_set.store(true, Ordering::Relaxed);
        }
        self.server_set.load(Ordering::Relaxed)
    }

    /// Runs `f` on the simulation thread and waits for its result.
    pub fn on_sim<T: Send + 'static>(&self, f: impl FnOnce(&mut dyn MgmtHost) -> T + Send + 'static) -> Result<T, Fail> {
        let (tx, rx) = std::sync::mpsc::channel();
        let queued = self.runner.run(Box::new(move |host| {
            let _ = tx.send(f(host));
        }));
        if !queued {
            return Err(Fail::Internal("the server is not running".into()));
        }
        rx.recv().map_err(|_| Fail::Internal("the server stopped before it answered".into()))
    }

    /// Queues `f` for the simulation thread without waiting (`MinecraftApi.submit` of `server/stop`).
    pub fn on_sim_detached(&self, f: impl FnOnce(&mut dyn MgmtHost) + Send + 'static) {
        let _ = self.runner.run(Box::new(f));
    }
}

/// The UUID of an offline player (`UUIDUtil.createOfflinePlayerUUID`): version 3 of `OfflinePlayer:<name>`.
pub fn offline_uuid(name: &str) -> Uuid {
    use md5::{Digest, Md5};
    let mut h: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

/// The `management-server-*` properties (`DedicatedServerProperties`).
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: i32,
    pub secret: String,
    pub tls_enabled: bool,
    pub tls_keystore: String,
    pub tls_keystore_password: String,
    pub allowed_origins: String,
    /// `status-heartbeat-interval`.
    pub heartbeat_interval: i32,
}

impl Config {
    pub fn from_properties(p: &ServerProperties) -> Config {
        Config {
            host: p.management_server_host.clone(),
            port: p.management_server_port,
            secret: p.management_server_secret.clone(),
            tls_enabled: p.management_server_tls_enabled,
            tls_keystore: p.management_server_tls_keystore.clone(),
            tls_keystore_password: p.management_server_tls_keystore_password.clone(),
            allowed_origins: p.management_server_allowed_origins.clone(),
            heartbeat_interval: p.status_heartbeat_interval,
        }
    }
}

/// What the server hands the management server.
pub struct Backend {
    pub runner: Arc<dyn Runner>,
    pub info: Arc<ServerInfo>,
    pub access: SharedAccess,
    pub settings: Arc<Settings>,
    pub lookup: Option<Arc<dyn ProfileLookup>>,
    pub online_mode: bool,
}

/// Why the server could not start (vanilla's start-up fails with the same message).
#[derive(Debug)]
pub struct StartError {
    /// The exception: `java.lang.IllegalStateException: ...`.
    pub message: String,
    /// What it was caused by.
    pub cause: Option<String>,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(c) = &self.cause {
            write!(f, "\nCaused by: {c}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StartError {}

fn fail(message: &str) -> StartError {
    StartError { message: message.to_owned(), cause: None }
}

/// `SecurityConfig.isValid`: 40 letters and digits.
pub fn secret_is_valid(secret: &str) -> bool {
    secret.len() == 40 && secret.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `HostAndPort.toString` of `HostAndPort.fromParts(host, port)`.
fn host_and_port(host: &str, port: i32) -> Result<String, StartError> {
    if !(0..=65535).contains(&port) {
        return Err(fail(&format!("java.lang.IllegalArgumentException: Port out of range: {port}")));
    }
    let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    match bare.matches(':').count() {
        0 => Ok(format!("{bare}:{port}")),
        1 => Err(fail(&format!("java.lang.IllegalArgumentException: Host has a port: {host}"))),
        _ => Ok(format!("[{bare}]:{port}")),
    }
}

/// The running management server.
pub struct ManagementServer {
    shared: Arc<conn::Shared>,
    addr: SocketAddr,
    heartbeat: Arc<hub::Heartbeat>,
    acceptor: Option<std::thread::JoinHandle<()>>,
    workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    stopped: bool,
}

static NEXT_CONNECTION: AtomicU32 = AtomicU32::new(1);

impl ManagementServer {
    /// `JsonRpc.create`: validates the settings, logs, starts listening and the heartbeat. Registers itself
    /// as the notification service in `backend.info`.
    pub fn start(config: &Config, backend: Backend) -> Result<ManagementServer, StartError> {
        if !secret_is_valid(&config.secret) {
            return Err(fail("java.lang.IllegalStateException: Invalid management server secret, must be 40 alphanumeric characters"));
        }
        let shown = host_and_port(&config.host, config.port)?;
        info!("Starting json RPC server on {shown}");
        let tls = if config.tls_enabled {
            let password = tls::keystore_password(&config.tls_keystore_password);
            match tls::server_config(&config.tls_keystore, &password) {
                Ok(c) => Some(c),
                Err(cause) => {
                    for line in tls::INSTRUCTIONS {
                        info!("{line}");
                    }
                    return Err(StartError {
                        message: "java.lang.IllegalStateException: Failed to configure TLS for the server management protocol".into(),
                        cause: Some(cause),
                    });
                }
            }
        } else {
            None
        };
        let bind_host = config.host.trim_start_matches('[').trim_end_matches(']');
        let listener = bind(bind_host, config.port as u16).map_err(|e| StartError { message: e, cause: None })?;
        let addr = listener.local_addr().map_err(|e| fail(&e.to_string()))?;

        let hub = Arc::new(hub::Hub::default());
        let notifier: Arc<dyn Notifier> = hub.clone();
        let heartbeat = hub::Heartbeat::start(notifier.clone(), i64::from(config.heartbeat_interval));
        backend.info.notifier.set(Some(notifier));
        let ctx = Arc::new(Ctx {
            runner: backend.runner,
            info: backend.info,
            access: backend.access,
            settings: backend.settings,
            lookup: backend.lookup,
            online_mode: backend.online_mode,
            hub,
            heartbeat: heartbeat.clone(),
            users: Mutex::default(),
            server_set: AtomicBool::new(false),
        });
        let shared = Arc::new(conn::Shared {
            ctx,
            secret: config.secret.clone(),
            origins: config.allowed_origins.split(',').map(str::to_owned).collect(),
            stopping: AtomicBool::new(false),
        });
        info!("Json-RPC Management connection listening on {}:{}", config.host, addr.port());
        let workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> = Arc::default();
        let acceptor = {
            let (shared, workers) = (shared.clone(), workers.clone());
            std::thread::Builder::new()
                .name("Management server IO #0".into())
                .spawn(move || accept_loop(&listener, &shared, tls, &workers))
                .map_err(|e| fail(&e.to_string()))?
        };
        Ok(ManagementServer { shared, addr, heartbeat, acceptor: Some(acceptor), workers, stopped: false })
    }

    /// The address it listens on.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// `ManagementServer.stop`: no new connections; the clients get what is pending, then are closed.
    pub fn stop(&mut self) {
        if std::mem::replace(&mut self.stopped, true) {
            return;
        }
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.heartbeat.stop();
        self.shared.ctx.info.notifier.set(None);
        if let Some(t) = self.acceptor.take() {
            let _ = t.join();
        }
        let workers: Vec<_> = self.workers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).drain(..).collect();
        for w in workers {
            let _ = w.join();
        }
        self.shared.ctx.hub.clear();
    }
}

impl Drop for ManagementServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn bind(host: &str, port: u16) -> Result<TcpListener, String> {
    use std::net::ToSocketAddrs;
    let ip: IpAddr = if host.is_empty() {
        [0, 0, 0, 0].into()
    } else if let Ok(ip) = host.parse() {
        ip
    } else {
        (host, port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|a| a.ip())
            .ok_or_else(|| "java.nio.channels.UnresolvedAddressException".to_owned())?
    };
    TcpListener::bind((ip, port)).map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse => format!("java.net.BindException: Address already in use: bind ({e})"),
        _ => format!("java.net.SocketException: {e}"),
    })
}

fn accept_loop(listener: &TcpListener, shared: &Arc<conn::Shared>, tls: Option<Arc<rustls::ServerConfig>>, workers: &Mutex<Vec<std::thread::JoinHandle<()>>>) {
    let _ = listener.set_nonblocking(true);
    while !shared.stopping.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_nodelay(true);
                let id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
                let wire: Result<Box<dyn conn::Wire>, ()> = match &tls {
                    Some(config) => match rustls::ServerConnection::new(config.clone()) {
                        Ok(c) => Ok(Box::new(conn::TlsWire(rustls::StreamOwned::new(c, stream)))),
                        Err(_) => Err(()),
                    },
                    None => Ok(Box::new(stream) as Box<dyn conn::Wire>),
                };
                let Ok(wire) = wire else { continue };
                let shared = shared.clone();
                let handle = std::thread::Builder::new().name(format!("Management server IO #{id}")).spawn(move || conn::serve(wire, peer, id, shared));
                match handle {
                    Ok(h) => {
                        let mut w = workers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        w.retain(|t| !t.is_finished());
                        w.push(h);
                    }
                    Err(e) => error!("could not start a management connection: {e}"),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[allow(dead_code)]
fn _unused(_: TcpStream) {}
