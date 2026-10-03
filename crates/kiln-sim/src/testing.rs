//! In-process connections for tests and synthetic load: players that join without a network.
//! Their sinks count what the simulation sends and remember the latest teleport, which a
//! scripted client must confirm before the server accepts its movement again.

use bytes::Bytes;
use kiln_link::{ClientInfo, ConnId, JoinInfo, PlayIn, Sink, ToSim};
use kiln_proto::codec::Reader;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use uuid::Uuid;

#[derive(Default)]
pub struct SinkStats {
    /// Packets and bytes received, keep-alives left out: those go out every 15 s of wall
    /// time, so on a slow run they would make the counts depend on the machine, not the inputs.
    pub packets: AtomicU64,
    pub bytes: AtomicU64,
    pub disconnected: AtomicBool,
    /// Latest Player Position (teleport) received: id and position.
    pub teleport: Mutex<Option<(i32, [f64; 3])>>,
    /// Latest keep-alive id not answered yet.
    pub keep_alive: Mutex<Option<i64>>,
    /// Packets and bytes per packet id, when `KILN_SINK_IDS` is set or `count_ids` (costs time
    /// per packet).
    pub by_id: Mutex<std::collections::BTreeMap<i32, (u64, u64)>>,
    pub count_ids: AtomicBool,
    /// Every packet sent, once set to `Some` (tests that inspect packets).
    pub log: Mutex<Option<Vec<Bytes>>>,
    /// Order-dependent hash of every packet received (keep-alives left out), when
    /// `KILN_SINK_DIGEST` is set: two runs sent a player the same stream iff the digests agree.
    pub digest: Mutex<u64>,
}

static DIGESTS: AtomicBool = AtomicBool::new(false);

/// Makes every sink hash its packets from now on (as `KILN_SINK_DIGEST` does); call before the
/// first join.
pub fn hash_packets() {
    DIGESTS.store(true, Relaxed);
}

/// Makes the locator bar check every receiver's quick share of the movers' turns against
/// stepping every pair (slow; for tests).
pub fn verify_locator_bar() {
    crate::waypoints::verify(true);
}

fn track_digest() -> bool {
    static TRACK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    DIGESTS.load(Relaxed) || *TRACK.get_or_init(|| std::env::var_os("KILN_SINK_DIGEST").is_some())
}

fn track_ids() -> bool {
    static TRACK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACK.get_or_init(|| std::env::var_os("KILN_SINK_IDS").is_some())
}

impl SinkStats {
    /// Counts a batch of packets: each lock is taken once per batch (a crowd's sinks see tens
    /// of thousands of packets a tick, and this runs inside the measured tick).
    fn record(&self, batch: &[Bytes]) {
        const KEEP_ALIVE: i32 = kiln_data::packets::play::clientbound::KEEP_ALIVE;
        const PLAYER_POSITION: i32 = kiln_data::packets::play::clientbound::PLAYER_POSITION;
        if let Some(log) = self.log.lock().unwrap().as_mut() {
            log.extend(batch.iter().cloned());
        }
        let ids: Vec<Option<i32>> = batch.iter().map(|p| Reader::new(p).varint().ok()).collect();
        if track_digest() {
            // FNV-1a over the length and the bytes.
            let mut d = self.digest.lock().unwrap();
            for (p, id) in batch.iter().zip(&ids) {
                if *id == Some(KEEP_ALIVE) {
                    continue;
                }
                let mut h = *d ^ 0xcbf2_9ce4_8422_2325;
                for b in (p.len() as u32).to_le_bytes().iter().chain(p.iter()) {
                    h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
                }
                *d = h;
            }
        }
        let (mut packets, mut bytes) = (0, 0);
        for (p, id) in batch.iter().zip(&ids) {
            if *id != Some(KEEP_ALIVE) {
                packets += 1;
                bytes += p.len() as u64;
            }
        }
        self.packets.fetch_add(packets, Relaxed);
        self.bytes.fetch_add(bytes, Relaxed);
        if track_ids() || self.count_ids.load(Relaxed) {
            let mut m = self.by_id.lock().unwrap();
            for (p, id) in batch.iter().zip(&ids) {
                if let Some(id) = id {
                    let e = m.entry(*id).or_default();
                    e.0 += 1;
                    e.1 += p.len() as u64;
                }
            }
        }
        for (p, id) in batch.iter().zip(&ids) {
            match *id {
                Some(KEEP_ALIVE) => {
                    let mut r = Reader::new(p);
                    if r.varint().is_ok()
                        && let Ok(k) = r.i64()
                    {
                        *self.keep_alive.lock().unwrap() = Some(k);
                    }
                }
                Some(PLAYER_POSITION) => {
                    let mut r = Reader::new(p);
                    if r.varint().is_ok()
                        && let (Ok(id), Ok(x), Ok(y), Ok(z)) = (r.varint(), r.f64(), r.f64(), r.f64())
                    {
                        *self.teleport.lock().unwrap() = Some((id, [x, y, z]));
                    }
                }
                _ => {}
            }
        }
    }
}

struct TestSink(Arc<SinkStats>);

impl Sink for TestSink {
    fn send(&self, packet: Bytes) {
        self.0.record(std::slice::from_ref(&packet));
    }
    fn send_batch(&self, packets: Vec<Bytes>) {
        self.0.record(&packets);
    }
    fn disconnect(&self, packet: Bytes) {
        self.0.record(std::slice::from_ref(&packet));
        self.0.disconnected.store(true, Relaxed);
    }
}

/// A join message for a player named `name` (UUID derived from `conn`), and the stats of its
/// connection.
pub fn join(conn: ConnId, name: &str, view_distance: u8) -> (ToSim, Arc<SinkStats>) {
    let stats = Arc::new(SinkStats::default());
    let msg = ToSim::Join(JoinInfo {
        conn,
        name: name.to_owned(),
        uuid: Uuid::from_u64_pair(0x6b69_6c6e, conn),
        properties: Vec::new(),
        client: ClientInfo { view_distance, ..ClientInfo::default() },
        sink: Box::new(TestSink(stats.clone())),
        address: None,
    });
    (msg, stats)
}

/// A scripted client: confirms teleports like the 26.3 client and reports its position.
pub struct Client {
    pub conn: ConnId,
    pub stats: Arc<SinkStats>,
    pub pos: [f64; 3],
    confirmed: i32,
    loaded: bool,
}

impl Client {
    pub fn new(conn: ConnId, stats: Arc<SinkStats>) -> Self {
        Self { conn, stats, pos: [0.0; 3], confirmed: 0, loaded: false }
    }

    /// Packets for this client tick: a keep-alive answer, a teleport confirmation (which replaces movement this
    /// tick), the loaded report once, or a move to `to` if given; then Client Tick End.
    pub fn tick(&mut self, to: Option<[f64; 3]>, out: &mut Vec<ToSim>) {
        // Answered at once, so a slow simulation does not time scripted clients out.
        if let Some(id) = self.stats.keep_alive.lock().unwrap().take() {
            out.push(ToSim::Packet(self.conn, PlayIn::KeepAlive { id }));
        }
        let teleport = *self.stats.teleport.lock().unwrap();
        if let Some((id, pos)) = teleport.filter(|(id, _)| *id != self.confirmed) {
            self.confirmed = id;
            self.pos = pos;
            out.push(ToSim::Packet(self.conn, PlayIn::AcceptTeleport { id }));
        } else if !self.loaded {
            self.loaded = true;
            out.push(ToSim::Packet(self.conn, PlayIn::PlayerLoaded));
        } else if let Some(to) = to {
            self.pos = to;
            out.push(ToSim::Packet(self.conn, PlayIn::Move { pos: Some(to), rot: None, on_ground: true, horizontal_collision: false }));
        }
        out.push(ToSim::Packet(self.conn, PlayIn::ClientTickEnd));
    }

    /// Whether the server's latest teleport has been confirmed.
    pub fn settled(&self) -> bool {
        self.loaded && self.stats.teleport.lock().unwrap().is_none_or(|(id, _)| id == self.confirmed)
    }
}

/// Walking speed in blocks per tick.
pub const WALK_PER_TICK: f64 = 4.317 / 20.0;

/// xorshift64*: scripts depend only on their seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A scripted player that walks between random points around a centre, pausing at each
/// (crowd) or not (walk).
pub struct Walker {
    pub client: Client,
    pub center: [f64; 2],
    target: [f64; 2],
    pause: u32,
    rng: Rng,
}

impl Walker {
    pub fn new(client: Client, center: [f64; 2], seed: u64) -> Self {
        let rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0xbf58_476d_1ce4_e5b9));
        Self { client, center, target: center, pause: 0, rng }
    }

    /// The next position to report, or `None` to stand still this tick.
    pub fn next_position(&mut self, radius: f64, walk: bool) -> Option<[f64; 3]> {
        if self.pause > 0 {
            self.pause -= 1;
            return None;
        }
        let [x, y, z] = self.client.pos;
        let (dx, dz) = (self.target[0] - x, self.target[1] - z);
        let d = dx.hypot(dz);
        if d < WALK_PER_TICK {
            let (a, r) = (self.rng.unit() * std::f64::consts::TAU, radius * self.rng.unit().sqrt());
            self.target = [self.center[0] + r * a.cos(), self.center[1] + r * a.sin()];
            if !walk {
                self.pause = (self.rng.unit() * 60.0) as u32;
            }
            return None;
        }
        Some([x + dx / d * WALK_PER_TICK, y, z + dz / d * WALK_PER_TICK])
    }

    /// Scripted packets for this client tick.
    pub fn tick(&mut self, radius: f64, walk: bool, out: &mut Vec<ToSim>) {
        let to = if self.client.settled() { self.next_position(radius, walk) } else { None };
        self.client.tick(to, out);
    }
}

/// Offset of group `g`'s centre from the shared centre: groups fill a square grid.
pub fn group_offset(g: usize, groups: usize, spacing: f64) -> [f64; 2] {
    let cols = (groups as f64).sqrt().ceil().max(1.0) as usize;
    let rows = groups.div_ceil(cols);
    let at = |i: usize, n: usize| (i as f64 - (n - 1) as f64 / 2.0) * spacing;
    [at(g % cols, cols), at(g / cols, rows)]
}

/// Players that crouch, go spectator, and leave and rejoin while a crowd runs (locator bar and
/// tracking churn), scripted from the tick number so two builds can be compared.
pub struct Churn {
    crouching: Vec<bool>,
    spectators: Vec<(usize, usize)>,
    retired: Vec<Arc<SinkStats>>,
    next_conn: u64,
}

impl Churn {
    pub fn new(players: usize) -> Self {
        Self { crouching: vec![false; players], spectators: Vec::new(), retired: Vec::new(), next_conn: players as u64 }
    }

    /// Adds tick `k`'s events to `inbox`.
    pub fn tick(&mut self, k: usize, walkers: &mut Vec<Walker>, inbox: &mut Vec<ToSim>, groups: usize, spacing: f64, view_distance: u8, y: f64) {
        let n = walkers.len();
        for i in 0..n {
            // About 2.5% of the players flip their crouch each tick.
            if (i * 7 + k) % 40 == 0 {
                self.crouching[i] = !self.crouching[i];
                let flags = if self.crouching[i] { 0x20 } else { 0 };
                inbox.push(ToSim::Packet(walkers[i].client.conn, PlayIn::PlayerInput { flags }));
            }
        }
        if k % 50 == 0 {
            let i = (k / 50 * 13) % n;
            inbox.push(ToSim::Console(format!("gamemode spectator W{}", walkers[i].client.conn - 1)));
            self.spectators.push((k + 25, i));
        }
        while let Some(&(when, i)) = self.spectators.first() {
            if when > k {
                break;
            }
            self.spectators.remove(0);
            inbox.push(ToSim::Console(format!("gamemode survival W{}", walkers[i].client.conn - 1)));
        }
        if k % 97 == 96 {
            // One player leaves; a new one takes its place in the group.
            let i = (k / 97 * 31) % n;
            let old = walkers.remove(i);
            self.crouching.remove(i);
            self.retired.push(old.client.stats.clone());
            inbox.push(ToSim::Leave(old.client.conn));
            self.next_conn += 1;
            let name = format!("W{}", self.next_conn - 1);
            let (msg, stats) = join(self.next_conn, &name, view_distance);
            inbox.push(msg);
            let [ox, oz] = group_offset(i % groups, groups, spacing);
            let center = [8.5 + ox, 8.5 + oz];
            inbox.push(ToSim::Console(format!("tp {name} {} {y} {}", center[0], center[1])));
            walkers.insert(i, Walker::new(Client::new(self.next_conn, stats), center, self.next_conn));
            self.crouching.insert(i, false);
        }
    }

    /// One hash of every player's packet stream (needs `KILN_SINK_DIGEST`), the players who
    /// left included, combined in connection order.
    pub fn stream_digest(&self, walkers: &[Walker]) -> u64 {
        stream_digest(walkers, &self.retired)
    }
}

/// One hash of the packet streams of `walkers` and of `retired` connections (needs
/// `KILN_SINK_DIGEST`).
pub fn stream_digest(walkers: &[Walker], retired: &[Arc<SinkStats>]) -> u64 {
    let mut all: Vec<(u64, u64)> = walkers
        .iter()
        .map(|w| (w.client.conn, *w.client.stats.digest.lock().unwrap()))
        .chain(retired.iter().map(|r| (0, *r.digest.lock().unwrap())))
        .collect();
    all.sort_unstable();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (c, d) in &all {
        for b in c.to_le_bytes().iter().chain(d.to_le_bytes().iter()) {
            h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}
