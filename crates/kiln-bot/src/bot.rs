//! One bot: a task that owns its socket, handles packets as they arrive and runs a 20 Hz
//! client tick. Outgoing packets are batched and written once per wakeup.

use crate::Config;
use crate::behavior::{Behavior, Mover, Rng};
use crate::metrics::{Shared, Traffic};
use crate::proto::{self, Teleport};
use crate::survival::{Agent, Settings};
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
/// Bots farther than this from their group centre teleport there when `teleport_to_group` is set.
const GROUP_TELEPORT_DISTANCE: f64 = 16.0;

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
fn needs_body(state: State, id: i32, survival: bool) -> bool {
    match state {
        State::Login => true,
        State::Configuration => {
            use ids::configuration::clientbound as cb;
            matches!(id, cb::SELECT_KNOWN_PACKS | cb::KEEP_ALIVE | cb::PING | cb::DISCONNECT | cb::COOKIE_REQUEST)
        }
        State::Play => {
            use ids::play::clientbound as cb;
            matches!(id, cb::PLAYER_POSITION | cb::KEEP_ALIVE | cb::PING | cb::DISCONNECT | cb::COOKIE_REQUEST)
                || (survival
                    && matches!(
                        id,
                        cb::LOGIN
                            | cb::LEVEL_CHUNK_WITH_LIGHT
                            | cb::BLOCK_UPDATE
                            | cb::SECTION_BLOCKS_UPDATE
                            | cb::FORGET_LEVEL_CHUNK
                            | cb::SET_CHUNK_CACHE_CENTER
                            | cb::SET_CHUNK_CACHE_RADIUS
                            | cb::SET_HEALTH
                            | cb::RESPAWN
                            | cb::OPEN_SCREEN
                            | cb::CONTAINER_SET_CONTENT
                            | cb::CONTAINER_SET_SLOT
                            | cb::CONTAINER_CLOSE
                            | cb::SET_PLAYER_INVENTORY
                            | cb::ADD_ENTITY
                            | cb::REMOVE_ENTITIES
                            | cb::MOVE_ENTITY_POS
                            | cb::MOVE_ENTITY_POS_ROT
                            | cb::ENTITY_POSITION_SYNC
                            | cb::TELEPORT_ENTITY
                            | cb::BLOCK_CHANGED_ACK
                    ))
        }
    }
}

/// Outgoing packets, framed into one buffer until the next write.
pub(crate) struct Out {
    threshold: Option<usize>,
    scratch: BytesMut,
    buf: BytesMut,
    packets: u64,
}

impl Out {
    pub(crate) fn new(threshold: Option<usize>) -> Self {
        Out { threshold, scratch: BytesMut::with_capacity(256), buf: BytesMut::with_capacity(1024), packets: 0 }
    }

    /// The framed packets written so far (tests decode them with the server's codec).
    #[cfg(test)]
    pub(crate) fn take_framed(&mut self) -> BytesMut {
        self.buf.split()
    }

    pub(crate) fn send(&mut self, build: impl FnOnce(&mut BytesMut)) {
        self.scratch.clear();
        build(&mut self.scratch);
        wire::encode(self.threshold, &self.scratch, &mut self.buf);
        self.packets += 1;
    }
}

struct Bot {
    index: usize,
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
    /// Survival bots: the player (created at Play Login).
    agent: Option<Agent>,
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
        index,
        name: format!("{}{}", cfg.name_prefix, index),
        rng: Rng::new(cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index as u64),
        cfg,
        shared: shared.clone(),
        state: State::Login,
        inbound: Inbound::default(),
        out: Out::new(None),
        traffic: Traffic::default(),
        started: now,
        last_flush: None,
        last_rx: now,
        world_since: None,
        pending_teleport: None,
        terrain: false,
        mover: None,
        agent: None,
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
                        let survival = self.cfg.behavior == Behavior::Survival;
                        let frame = self
                            .inbound
                            .decode(&mut rbuf, |id| needs_body(state, id, survival))
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
            cb::LEVEL_CHUNK_WITH_LIGHT => {
                self.traffic.chunks += 1;
                if let Some(a) = &mut self.agent {
                    a.on_chunk(r.rest());
                }
            }
            cb::BLOCK_UPDATE if self.agent.is_some() => {
                let pos = kiln_proto::packets::read_position(r)?;
                let state = r.varint()?;
                self.agent.as_mut().unwrap().on_block_update(pos, state as u16);
            }
            cb::SECTION_BLOCKS_UPDATE if self.agent.is_some() => self.agent.as_mut().unwrap().on_section_update(r)?,
            cb::FORGET_LEVEL_CHUNK if self.agent.is_some() => {
                let v = r.i64()?;
                self.agent.as_mut().unwrap().on_forget_chunk(v as i32, (v >> 32) as i32);
            }
            cb::SET_CHUNK_CACHE_CENTER if self.agent.is_some() => {
                let (x, z) = (r.varint()?, r.varint()?);
                self.agent.as_mut().unwrap().on_chunk_center(x, z);
            }
            cb::SET_CHUNK_CACHE_RADIUS if self.agent.is_some() => {
                let radius = r.varint()?;
                self.agent.as_mut().unwrap().on_chunk_radius(radius);
            }
            cb::SET_HEALTH if self.agent.is_some() => {
                let health = r.f32()?;
                let food = r.varint()?;
                self.agent.as_mut().unwrap().on_health(health, food);
            }
            cb::RESPAWN if self.agent.is_some() => self.agent.as_mut().unwrap().on_respawn(),
            cb::OPEN_SCREEN if self.agent.is_some() => {
                let id = r.varint()?;
                self.agent.as_mut().unwrap().on_open_screen(id);
            }
            cb::CONTAINER_SET_CONTENT if self.agent.is_some() => {
                let (id, state, n) = (r.varint()?, r.varint()?, r.varint()?);
                self.agent.as_mut().unwrap().on_container_content(id, state, n);
            }
            cb::CONTAINER_SET_SLOT if self.agent.is_some() => {
                let (id, state, slot) = (r.varint()?, r.varint()?, r.i16()?);
                self.agent.as_mut().unwrap().on_container_slot(id, state, slot as i32);
            }
            cb::CONTAINER_CLOSE if self.agent.is_some() => self.agent.as_mut().unwrap().on_container_close(),
            cb::ADD_ENTITY if self.agent.is_some() => self.agent.as_mut().unwrap().on_add_entity(r)?,
            cb::REMOVE_ENTITIES if self.agent.is_some() => self.agent.as_mut().unwrap().on_remove_entities(r)?,
            cb::MOVE_ENTITY_POS | cb::MOVE_ENTITY_POS_ROT if self.agent.is_some() => self.agent.as_mut().unwrap().on_entity_move(r)?,
            cb::ENTITY_POSITION_SYNC if self.agent.is_some() => self.agent.as_mut().unwrap().on_entity_sync(r)?,
            cb::TELEPORT_ENTITY if self.agent.is_some() => self.agent.as_mut().unwrap().on_entity_teleport(r)?,
            cb::SET_PLAYER_INVENTORY if self.agent.is_some() => {
                let slot = r.varint()?;
                self.agent.as_mut().unwrap().on_inventory_slot(slot);
            }
            cb::BLOCK_CHANGED_ACK if self.agent.is_some() => {
                let seq = r.varint()?;
                self.agent.as_mut().unwrap().on_ack(seq);
            }
            cb::SYSTEM_CHAT | cb::PLAYER_CHAT | cb::DISGUISED_CHAT => self.traffic.chat_received += 1,
            cb::LOGIN => {
                self.world_since = Some(Instant::now());
                if self.cfg.behavior == Behavior::Survival {
                    let entity_id = r.i32()?;
                    let _hardcore = r.bool()?;
                    for _ in 0..r.varint()? {
                        r.string(256)?;
                    }
                    let _max_players = r.varint()?;
                    let view = r.varint()?;
                    self.agent = Some(self.make_agent(entity_id, view));
                }
            }
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

    fn make_agent(&self, entity_id: i32, server_view: i32) -> Agent {
        let cfg = &self.cfg;
        let (group, groups) = cfg.group_of(self.index);
        let [cx, cz] = cfg.center.unwrap_or([0.0, 0.0]);
        let [dx, dz] = cfg.group_offset_in(group, groups);
        let role = cfg.roles[self.index % cfg.roles.len()];
        let settings = Settings {
            role,
            site: [cx + dx, cz + dz],
            view_distance: server_view.min(cfg.view_distance as i32),
            seed: cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (self.index as u64).wrapping_mul(0xd1b5_4a32_d192_ed03),
            chat_interval: cfg.chat_interval,
            stay: groups <= 1 && cfg.group_size.is_none(),
            roam: cfg.radius,
        };
        Agent::new(settings, self.shared.clone(), [0.0; 3], 0.0, 0.0, entity_id)
    }

    /// Confirms a teleport like the 26.3 client: Accept Teleportation carrying the resulting
    /// position and nothing else (a Move Player too would be a second position this tick).
    fn teleport(&mut self, t: Teleport) {
        if let Some(agent) = &mut self.agent {
            let spawned = self.mover.is_some();
            let (pos, yaw, pitch) = if spawned {
                t.apply(agent.body.pos, agent.body.yaw, agent.body.pitch)
            } else {
                t.apply([0.0; 3], 0.0, 0.0)
            };
            self.out.send(|b| proto::accept_teleportation(b, t.id, pos, yaw, pitch));
            if spawned {
                agent.on_teleport(pos, yaw, pitch);
            } else {
                agent.spawn_at(pos, yaw, pitch);
                // The bot has a body now; the connection counts it as spawned like a walker.
                self.mover = Some(Mover::new(Behavior::Idle, Rng::new(1), [pos[0], pos[2]], 0.0, 0.0));
            }
            return;
        }
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
        let [cx, cz] = *self.shared.origin.get_or_init(|| [pos[0], pos[2]]);
        let [dx, dz] = self.cfg.group_offset(self.index % self.cfg.groups);
        let origin = [cx + dx, cz + dz];
        if self.cfg.teleport_to_group && (origin[0] - pos[0]).hypot(origin[1] - pos[2]) > GROUP_TELEPORT_DISTANCE {
            let command = format!("tp @s {:.1} ~ {:.1}", origin[0], origin[1]);
            self.out.send(|b| proto::chat_command(b, &command));
        }
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
                && let Some(agent) = &mut self.agent
            {
                agent.tick(&mut self.out);
                let counts = std::mem::take(&mut agent.counts);
                self.traffic.merge(&counts);
            } else if self.joined
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

pub(crate) fn unix_millis() -> i64 {
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
