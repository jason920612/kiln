//! Query (GameSpy 4), as vanilla's `QueryThreadGs4` answers it over UDP: a handshake (type 9) is
//! answered with a challenge token, a stat request (type 0) carrying the token with the basic stat,
//! or, when 4 bytes of padding follow the token, the full stat.

use kiln_link::ServerInfo;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Challenge tokens last this long (`pruneChallenges` runs every 30 s).
const CHALLENGE_LIFETIME: Duration = Duration::from_secs(30);
/// The full stat is built at most this often (`lastRulesResponse + 5000`).
const RULES_CACHE: Duration = Duration::from_secs(5);

/// `QueryThreadGs4$RequestChallenge`.
struct Challenge {
    created: Instant,
    ident: [u8; 4],
    token: i32,
    bytes: Vec<u8>,
}

impl Challenge {
    fn new(request: &[u8], token: i32) -> Self {
        let ident = [request[3], request[4], request[5], request[6]];
        // The session id goes through a string (UTF-8) on the way into the reply, as in vanilla.
        let text = format!("\t{}{token}\0", String::from_utf8_lossy(&ident));
        Challenge { created: Instant::now(), ident, token, bytes: text.into_bytes() }
    }
}

/// Everything a stat reply says about the server.
pub struct Facts {
    pub motd: String,
    pub world: String,
    pub players: Vec<String>,
    pub max_players: i32,
    pub port: i32,
    pub host_ip: String,
    pub version: String,
}

pub trait FactSource: Send + Sync {
    fn facts(&self) -> Facts;
}

/// The facts from a live server: `info` for the motd, the roster and the limits.
pub struct LiveFacts {
    pub info: Arc<ServerInfo>,
    pub port: i32,
    pub host_ip: String,
}

impl FactSource for LiveFacts {
    fn facts(&self) -> Facts {
        Facts {
            motd: self.info.motd(),
            world: self.info.level_name.clone(),
            players: self.info.players().into_iter().map(|p| p.name).collect(),
            max_players: self.info.max_players(),
            port: self.port,
            host_ip: self.host_ip.clone(),
            version: self.info.version_name.clone(),
        }
    }
}

/// `NetworkDataOutputStream`: strings are NUL-terminated, shorts little-endian.
#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn byte(&mut self, b: u8) -> &mut Self {
        self.0.push(b);
        self
    }
    fn string(&mut self, s: &str) -> &mut Self {
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
        self
    }
}

/// The reply to a stat request that carried a valid token: the full stat when `full`.
pub fn stat_reply(facts: &Facts, ident: [u8; 4], full: bool) -> Vec<u8> {
    let mut o = Out::default();
    if full {
        o.byte(0);
        o.0.extend_from_slice(&ident);
        o.string("splitnum").byte(128).byte(0);
        for (k, v) in [
            ("hostname", facts.motd.as_str()),
            ("gametype", "SMP"),
            ("game_id", "MINECRAFT"),
            ("version", facts.version.as_str()),
            ("plugins", ""),
            ("map", facts.world.as_str()),
        ] {
            o.string(k).string(v);
        }
        o.string("numplayers").string(&facts.players.len().to_string());
        o.string("maxplayers").string(&facts.max_players.to_string());
        o.string("hostport").string(&facts.port.to_string());
        o.string("hostip").string(&facts.host_ip);
        o.byte(0).byte(1);
        o.string("player_").byte(0);
        for name in &facts.players {
            o.string(name);
        }
        o.byte(0);
    } else {
        o.byte(0);
        o.0.extend_from_slice(&ident);
        o.string(&facts.motd).string("SMP").string(&facts.world);
        o.string(&facts.players.len().to_string()).string(&facts.max_players.to_string());
        o.0.extend_from_slice(&(facts.port as i16).to_le_bytes());
        o.string(&facts.host_ip);
    }
    o.0
}

/// The machine's address for `hostip` when no `server-ip` is set (`InetAddress.getLocalHost().getHostAddress()`).
pub fn local_host_address() -> Option<String> {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())?;
    let name = name.trim();
    std::net::ToSocketAddrs::to_socket_addrs(&(name, 0)).ok()?.next().map(|a| a.ip().to_string())
}

pub struct QueryServer {
    running: Arc<AtomicBool>,
    addr: SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct State {
    challenges: HashMap<SocketAddr, Challenge>,
    last_prune: Instant,
    rules: Option<(Instant, Vec<u8>)>,
}

impl QueryServer {
    /// `QueryThreadGs4.create`: `None` (with vanilla's log lines) if the port is out of range or taken.
    pub fn start(server_ip: &str, port: i32, facts: Arc<dyn FactSource>, random: Arc<dyn Fn() -> i32 + Send + Sync>) -> Option<QueryServer> {
        if !(1..=65535).contains(&port) {
            warn!("Invalid query port {port} found in server.properties (queries disabled)");
            return None;
        }
        let bind_ip = if server_ip.is_empty() { "0.0.0.0" } else { server_ip };
        let socket = match bind_ip.parse::<IpAddr>().map(|ip| UdpSocket::bind((ip, port as u16))) {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                warn!("Unable to initialise query system on {bind_ip}:{port}: {e}");
                return None;
            }
            Err(_) => {
                warn!("Unable to initialise query system on {bind_ip}:{port}: unknown host");
                return None;
            }
        };
        Self::from_socket(socket, bind_ip, port, facts, random)
    }

    /// Serves Query on a bound socket (`ip` and `port` are only for the log line).
    pub fn from_socket(socket: UdpSocket, ip: &str, port: i32, facts: Arc<dyn FactSource>, random: Arc<dyn Fn() -> i32 + Send + Sync>) -> Option<QueryServer> {
        socket.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
        let addr = socket.local_addr().ok()?;
        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        info!("Thread Query Listener started");
        let ip = ip.to_owned();
        let thread = std::thread::Builder::new()
            .name("Query Listener".into())
            .spawn(move || serve(socket, run, facts, random, ip, port))
            .ok()?;
        Some(QueryServer { running, addr, thread: Some(thread) })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
            info!("Thread Query Listener stopped");
        }
    }
}

impl Drop for QueryServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(socket: UdpSocket, running: Arc<AtomicBool>, facts: Arc<dyn FactSource>, random: Arc<dyn Fn() -> i32 + Send + Sync>, ip: String, port: i32) {
    info!("Query running on {ip}:{port}");
    let mut state = State { challenges: HashMap::new(), last_prune: Instant::now(), rules: None };
    let mut buf = [0u8; 1460];
    while running.load(Ordering::SeqCst) {
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                prune(&mut state);
                if let Err(e) = handle(&socket, &buf[..n], from, &mut state, facts.as_ref(), random.as_ref()) {
                    debug!("query reply to {from} failed: {e}");
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => prune(&mut state),
            Err(_) => {}
        }
    }
}

fn prune(state: &mut State) {
    if state.last_prune.elapsed() < CHALLENGE_LIFETIME {
        return;
    }
    state.last_prune = Instant::now();
    state.challenges.retain(|_, c| c.created.elapsed() < CHALLENGE_LIFETIME);
}

/// `processPacket`.
fn handle(
    socket: &UdpSocket,
    data: &[u8],
    from: SocketAddr,
    state: &mut State,
    facts: &dyn FactSource,
    random: &(dyn Fn() -> i32 + Send + Sync),
) -> std::io::Result<()> {
    // A request starts FE FD, the type, the session id.
    if data.len() < 3 || data[0] != 0xfe || data[1] != 0xfd {
        return Ok(());
    }
    match data[2] {
        9 => {
            if data.len() < 7 {
                return Ok(());
            }
            let challenge = Challenge::new(data, random() & 0xff_ffff);
            let bytes = challenge.bytes.clone();
            state.challenges.insert(from, challenge);
            socket.send_to(&bytes, from)?;
        }
        0 => {
            let Some(c) = state.challenges.get(&from) else { return Ok(()) };
            // The token the client sends back sits at bytes 7..11.
            let token = if data.len() >= 11 { i32::from_be_bytes([data[7], data[8], data[9], data[10]]) } else { 0 };
            if c.token != token {
                return Ok(());
            }
            let ident = c.ident;
            let reply = if data.len() == 15 {
                // The full stat is rebuilt at most every 5 s; in between the old one is repeated for the new session id.
                match &mut state.rules {
                    Some((at, bytes)) if at.elapsed() < RULES_CACHE => {
                        bytes[1..5].copy_from_slice(&ident);
                        bytes.clone()
                    }
                    slot => {
                        let bytes = stat_reply(&facts.facts(), ident, true);
                        *slot = Some((Instant::now(), bytes.clone()));
                        bytes
                    }
                }
            } else {
                stat_reply(&facts.facts(), ident, false)
            };
            socket.send_to(&reply, from)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
