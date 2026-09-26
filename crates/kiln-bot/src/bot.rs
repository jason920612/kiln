//! One bot: a task that owns its socket, handles packets as they arrive and runs a 20 Hz
//! client tick. Outgoing packets are batched and written once per wakeup.

use crate::Config;
use crate::behavior::{Mover, Rng};
use crate::metrics::{Shared, Traffic};
use crate::proto::{self, Teleport};
use crate::wire::{self, Frame, Inbound};
use anyhow::{Context, Result};
use bytes::BytesMut;
use kiln_data::packets as ids;
use kiln_proto::{DecodeError, Reader};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

const TICK: Duration = Duration::from_millis(50);
const TICKS_PER_SECOND: f64 = 20.0;
const READ_RESERVE: usize = 32 * 1024;
/// The server sends a keep-alive at least every 15 s.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(60);
const BRAND: &str = "kiln-bot";
/// Longest wait for terrain before confirming the first teleport anyway.
const FIRST_TELEPORT_WAIT: Duration = Duration::from_secs(1);

/// Where the bots connect, resolved once.
pub(crate) struct Target {
    addr: SocketAddr,
    /// Host name as sent in the handshake.
    host: String,
}

impl Target {
    pub async fn resolve(addr: &str) -> Result<Self> {
        let resolved = tokio::net::lookup_host(addr)
            .await
            .with_context(|| format!("resolving {addr}"))?
            .next()
            .with_context(|| format!("{addr} did not resolve"))?;
        let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h).trim_matches(['[', ']']);
        Ok(Self { addr: resolved, host: host.to_owned() })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Login,
    Configuration,
    Play,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Login => "login",
            State::Configuration => "configuration",
            State::Play => "play",
        }
    }
}

/// Whether the bot reads the body of a clientbound packet; everything else is only counted.
fn needs_body(state: State, id: i32) -> bool {
    match state {
        State::Login => true,
        State::Configuration => {
            use ids::configuration::clientbound as cb;
            matches!(id, cb::SELECT_KNOWN_PACKS | cb::KEEP_ALIVE | cb::PING | cb::DISCONNECT | cb::COOKIE_REQUEST)
        }
        State::Play => {
            use ids::play::clientbound as cb;
            matches!(id, cb::PLAYER_POSITION | cb::KEEP_ALIVE | cb::PING | cb::DISCONNECT | cb::COOKIE_REQUEST)
        }
    }
}

/// Outgoing packets, framed into one buffer until the next write.
struct Out {
    threshold: Option<usize>,
    scratch: BytesMut,
    buf: BytesMut,
    packets: u64,
}

impl Out {
    fn send(&mut self, build: impl FnOnce(&mut BytesMut)) {
        self.scratch.clear();
        build(&mut self.scratch);
        wire::encode(self.threshold, &self.scratch, &mut self.buf);
        self.packets += 1;
    }
}

struct Bot {
    name: String,
    cfg: Arc<Config>,
    shared: Arc<Shared>,
    rng: Rng,
    state: State,
    inbound: Inbound,
    out: Out,
    traffic: Traffic,
    started: Instant,
    /// Set while connected; connected time is accounted up to here.
    last_flush: Option<Instant>,
    last_rx: Instant,
    /// When Play Login arrived; the client tick runs from then on.
    world_since: Option<Instant>,
    /// The first teleport, held until terrain starts arriving (see `handle_play`).
    pending_teleport: Option<Teleport>,
    terrain: bool,
    /// Created at the first teleport.
    mover: Option<Mover>,
    joined: bool,
    play_ticks: u64,
    next_chat_tick: u64,
}

/// Runs one bot until it is disconnected or `stop` fires, and records the outcome.
pub(crate) async fn run(
    index: usize,
    cfg: Arc<Config>,
    target: Arc<Target>,
    shared: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
) {
    shared.launched.fetch_add(1, Relaxed);
    let now = Instant::now();
    let mut bot = Bot {
        name: format!("{}{}", cfg.name_prefix, index),
        rng: Rng::new(cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index as u64),
        cfg,
        shared: shared.clone(),
        state: State::Login,
        inbound: Inbound::default(),
        out: Out {
            threshold: None,
            scratch: BytesMut::with_capacity(256),
            buf: BytesMut::with_capacity(1024),
            packets: 0,
        },
        traffic: Traffic::default(),
        started: now,
        last_flush: None,
        last_rx: now,
        world_since: None,
        pending_teleport: None,
        terrain: false,
        mover: None,
        joined: false,
        play_ticks: 0,
        next_chat_tick: 0,
    };
    let reason = bot.session(&target, &mut stop).await.err();
    bot.flush(Instant::now());
    shared.record_end(bot.joined, reason);
}

impl Bot {
    /// `Ok` when stopped by the run, otherwise why the connection ended.
    async fn session(&mut self, target: &Target, stop: &mut watch::Receiver<bool>) -> Result<(), String> {
        let connect = tokio::time::timeout(self.cfg.join_timeout, TcpStream::connect(target.addr));
        let mut stream = tokio::select! {
            r = connect => match r {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Err(format!("connect: {}", e.kind())),
                Err(_) => return Err("connect: timed out".into()),
            },
            _ = stop.changed() => return Ok(()),
        };
        let _ = stream.set_nodelay(true);
        self.shared.connected.fetch_add(1, Relaxed);
        let now = Instant::now();
        self.last_flush = Some(now);
        self.last_rx = now;
        let result = self.connected(&mut stream, target, stop).await;
        self.shared.connected.fetch_sub(1, Relaxed);
        result.map_err(|e| format!("{}: {e}", self.state.name()))
    }

    async fn connected(
        &mut self,
        stream: &mut TcpStream,
        target: &Target,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), String> {
        let uuid = offline_uuid(&self.name);
        self.out.send(|b| proto::intention(b, &target.host, target.addr.port()));
        self.out.send(|b| proto::hello(b, &self.name, uuid));

        let mut rbuf = BytesMut::with_capacity(READ_RESERVE);
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            if !self.out.buf.is_empty() {
                stream.write_all(&self.out.buf).await.map_err(|e| e.kind().to_string())?;
                self.traffic.tx_bytes += self.out.buf.len() as u64;
                self.out.buf.clear();
            }
            if rbuf.capacity() - rbuf.len() < READ_RESERVE / 4 {
                rbuf.reserve(READ_RESERVE);
            }
            tokio::select! {
                _ = stop.changed() => return Ok(()),
                r = stream.read_buf(&mut rbuf) => {
                    let n = r.map_err(|e| e.kind().to_string())?;
                    if n == 0 {
                        return Err("connection closed by the server".into());
                    }
                    self.traffic.rx_bytes += n as u64;
                    self.last_rx = Instant::now();
                    loop {
                        let state = self.state;
                        let frame = self
                            .inbound
                            .decode(&mut rbuf, |id| needs_body(state, id))
                            .map_err(|e| format!("bad frame: {e}"))?;
                        let Some(frame) = frame else { break };
                        self.traffic.rx_packets += 1;
                        self.handle(frame)?;
                    }
                }
                _ = tick.tick() => self.tick()?,
            }
        }
    }

    fn handle(&mut self, frame: Frame) -> Result<(), String> {
        let body = frame.body.as_deref().unwrap_or_default();
        let mut r = Reader::new(body);
        let id = frame.id;
        let result = match self.state {
            State::Login => self.handle_login(id, &mut r),
            State::Configuration => self.handle_configuration(id, &mut r, body),
            State::Play => self.handle_play(id, &mut r),
        };
        match result {
            Ok(Handled::Continue) => Ok(()),
            Ok(Handled::End(reason)) => Err(reason),
            Err(e) => Err(format!("malformed packet {id:#04x}: {e}")),
        }
    }

    fn handle_login(&mut self, id: i32, r: &mut Reader) -> Result<Handled, DecodeError> {
        use ids::configuration::serverbound as config_sb;
        use ids::login::clientbound as cb;
        match id {
            cb::LOGIN_COMPRESSION => {
                let threshold = usize::try_from(r.varint()?).ok();
                self.inbound.set_threshold(threshold);
                self.out.threshold = threshold;
            }
            cb::LOGIN_FINISHED => {
                // Same order as the vanilla client: acknowledge, then brand and settings.
                self.out.send(proto::login_acknowledged);
                self.state = State::Configuration;
                self.out.send(|b| proto::brand(b, config_sb::CUSTOM_PAYLOAD, BRAND));
                let vd = self.cfg.view_distance;
                self.out.send(|b| proto::client_information(b, config_sb::CLIENT_INFORMATION, vd));
            }
            cb::LOGIN_DISCONNECT => return Ok(Handled::End(proto::read_login_disconnect(r)?)),
            cb::HELLO => return Ok(Handled::End("server requires online-mode authentication".into())),
            cb::CUSTOM_QUERY => {
                let transaction = r.varint()?;
                self.out.send(|b| proto::custom_query_answer(b, transaction));
            }
            cb::COOKIE_REQUEST => {
                let key = r.string(32767)?;
                self.out.send(|b| proto::cookie_response(b, ids::login::serverbound::COOKIE_RESPONSE, key));
            }
            _ => return Ok(Handled::End(format!("unexpected login packet {id:#04x}"))),
        }
        Ok(Handled::Continue)
    }

    fn handle_configuration(&mut self, id: i32, r: &mut Reader, body: &[u8]) -> Result<Handled, DecodeError> {
        use ids::configuration::clientbound as cb;
        use ids::configuration::serverbound as sb;
        match id {
            cb::SELECT_KNOWN_PACKS => self.out.send(|b| proto::select_known_packs(b, body)),
            cb::FINISH_CONFIGURATION => {
                self.out.send(proto::finish_configuration);
                self.state = State::Play;
            }
            cb::KEEP_ALIVE => {
                let id = r.i64()?;
                self.out.send(|b| proto::keep_alive(b, sb::KEEP_ALIVE, id));
            }
            cb::PING => {
                let id = r.i32()?;
                self.out.send(|b| proto::pong(b, sb::PONG, id));
            }
            cb::CODE_OF_CONDUCT => self.out.send(proto::accept_code_of_conduct),
            cb::COOKIE_REQUEST => {
                let key = r.string(32767)?;
                self.out.send(|b| proto::cookie_response(b, sb::COOKIE_RESPONSE, key));
            }
            cb::DISCONNECT => return Ok(Handled::End(proto::read_disconnect(r)?)),
            _ => {}
        }
        Ok(Handled::Continue)
    }

    fn handle_play(&mut self, id: i32, r: &mut Reader) -> Result<Handled, DecodeError> {
        use ids::play::clientbound as cb;
        use ids::play::serverbound as sb;
        match id {
            cb::LEVEL_CHUNK_WITH_LIGHT => self.traffic.chunks += 1,
            cb::SYSTEM_CHAT | cb::PLAYER_CHAT | cb::DISGUISED_CHAT => self.traffic.chat_received += 1,
            cb::LOGIN => self.world_since = Some(Instant::now()),
            // A real client needs a while to set up its level after Login, so its first
            // confirmation reaches the server after the connection's first tick. Vanilla relies
            // on that: a confirmation processed earlier fails its movement check against an
            // unset reference position. Waiting for the first chunk batch reproduces the order.
            cb::PLAYER_POSITION if self.mover.is_none() && !self.terrain => {
                self.pending_teleport = Some(Teleport::read(r)?);
            }
            cb::PLAYER_POSITION => self.teleport(Teleport::read(r)?),
            cb::CHUNK_BATCH_START => {
                self.terrain = true;
                if let Some(t) = self.pending_teleport.take() {
                    self.teleport(t);
                }
            }
            cb::CHUNK_BATCH_FINISHED => {
                let cpt = self.cfg.chunks_per_tick;
                self.out.send(|b| proto::chunk_batch_received(b, cpt));
                // A real client closes its loading screen once it stands in received terrain.
                if !self.joined && self.mover.is_some() {
                    self.out.send(proto::player_loaded);
                    self.joined = true;
                    self.shared.record_join(self.started.elapsed());
                }
            }
            cb::KEEP_ALIVE => {
                let id = r.i64()?;
                self.out.send(|b| proto::keep_alive(b, sb::KEEP_ALIVE, id));
            }
            cb::PING => {
                let id = r.i32()?;
                self.out.send(|b| proto::pong(b, sb::PONG, id));
            }
            cb::START_CONFIGURATION => {
                self.out.send(proto::configuration_acknowledged);
                self.state = State::Configuration;
            }
            cb::COOKIE_REQUEST => {
                let key = r.string(32767)?;
                self.out.send(|b| proto::cookie_response(b, sb::COOKIE_RESPONSE, key));
            }
            cb::DISCONNECT => return Ok(Handled::End(proto::read_disconnect(r)?)),
            _ => {}
        }
        Ok(Handled::Continue)
    }

    /// Confirms a teleport like the 26.3 client: Accept Teleportation carrying the resulting
    /// position and nothing else (a Move Player too would be a second position this tick).
    fn teleport(&mut self, t: Teleport) {
        let (pos, yaw, pitch) = match &self.mover {
            Some(m) => t.apply(m.pos, m.yaw, m.pitch),
            None => t.apply([0.0; 3], 0.0, 0.0),
        };
        self.out.send(|b| proto::accept_teleportation(b, t.id, pos, yaw, pitch));
        if let Some(m) = &mut self.mover {
            self.traffic.teleports += 1;
            m.teleported(pos, yaw, pitch);
            return;
        }
        let origin = *self.shared.origin.get_or_init(|| [pos[0], pos[2]]);
        let behavior = self.cfg.behavior;
        let radius = self.cfg.radius.unwrap_or(behavior.default_radius());
        let mut m = Mover::new(behavior, Rng::new(self.rng.next_u64()), origin, radius, self.cfg.speed);
        m.teleported(pos, yaw, pitch);
        self.mover = Some(m);
        if let Some(every) = self.cfg.chat_interval {
            let every = (every.as_secs_f64() * TICKS_PER_SECOND).max(1.0);
            self.next_chat_tick = self.play_ticks + 1 + (self.rng.unit() * every) as u64;
        }
    }

    fn tick(&mut self) -> Result<(), String> {
        let now = Instant::now();
        if !self.joined && now - self.started > self.cfg.join_timeout {
            return Err("timed out before joining".into());
        }
        if now - self.last_rx > SILENCE_TIMEOUT {
            return Err(format!("nothing received for {} s", SILENCE_TIMEOUT.as_secs()));
        }
        if let Some(since) = self.world_since
            && self.state == State::Play
        {
            if now - since > FIRST_TELEPORT_WAIT
                && let Some(t) = self.pending_teleport.take()
            {
                self.teleport(t);
            }
            self.play_ticks += 1;
            // Vanilla ignores movement until the client reports its terrain loaded.
            if self.joined
                && let Some(m) = &mut self.mover
            {
                // At most one position per client tick: vanilla kicks for a second one.
                if let Some(p) = m.tick() {
                    self.out.send(|b| proto::move_player(b, p, true));
                }
                if let Some(every) = self.cfg.chat_interval
                    && self.play_ticks >= self.next_chat_tick
                {
                    self.traffic.chat_sent += 1;
                    let msg = format!("hello #{} from {}", self.traffic.chat_sent, self.name);
                    let (ts, salt) = (unix_millis(), self.rng.next_u64() as i64);
                    self.out.send(|b| proto::chat(b, &msg, ts, salt));
                    self.next_chat_tick += ((every.as_secs_f64() * TICKS_PER_SECOND) as u64).max(1);
                }
            }
            self.out.send(proto::client_tick_end);
        }
        self.flush(now);
        Ok(())
    }

    /// Moves local counters to the shared totals.
    fn flush(&mut self, now: Instant) {
        if let Some(last) = self.last_flush.replace(now) {
            self.traffic.connected_nanos += now.saturating_duration_since(last).as_nanos() as u64;
        }
        self.traffic.tx_packets += std::mem::take(&mut self.out.packets);
        self.shared.flush(&mut self.traffic);
    }
}

enum Handled {
    Continue,
    End(String),
}

/// UUID the vanilla server assigns in offline mode: v3 of "OfflinePlayer:<name>".
fn offline_uuid(name: &str) -> Uuid {
    use md5::{Digest, Md5};
    let mut h: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

fn unix_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_vanilla() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch").to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }
}
