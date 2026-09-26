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
    pub packets: AtomicU64,
    pub bytes: AtomicU64,
    pub disconnected: AtomicBool,
    /// Latest Player Position (teleport) received: id and position.
    pub teleport: Mutex<Option<(i32, [f64; 3])>>,
}

impl SinkStats {
    fn record(&self, p: &Bytes) {
        self.packets.fetch_add(1, Relaxed);
        self.bytes.fetch_add(p.len() as u64, Relaxed);
        let mut r = Reader::new(p);
        if r.varint().ok() == Some(kiln_data::packets::play::clientbound::PLAYER_POSITION)
            && let (Ok(id), Ok(x), Ok(y), Ok(z)) = (r.varint(), r.f64(), r.f64(), r.f64())
        {
            *self.teleport.lock().unwrap() = Some((id, [x, y, z]));
        }
    }
}

struct TestSink(Arc<SinkStats>);

impl Sink for TestSink {
    fn send(&self, packet: Bytes) {
        self.0.record(&packet);
    }
    fn send_batch(&self, packets: Vec<Bytes>) {
        packets.iter().for_each(|p| self.0.record(p));
    }
    fn disconnect(&self, packet: Bytes) {
        self.0.record(&packet);
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

    /// Packets for this client tick: a teleport confirmation (which replaces movement this
    /// tick), the loaded report once, or a move to `to` if given.
    pub fn tick(&mut self, to: Option<[f64; 3]>, out: &mut Vec<ToSim>) {
        let teleport = *self.stats.teleport.lock().unwrap();
        if let Some((id, pos)) = teleport.filter(|(id, _)| *id != self.confirmed) {
            self.confirmed = id;
            self.pos = pos;
            out.push(ToSim::Packet(self.conn, PlayIn::AcceptTeleport { id }));
            return;
        }
        if !self.loaded {
            self.loaded = true;
            out.push(ToSim::Packet(self.conn, PlayIn::PlayerLoaded));
            return;
        }
        if let Some(to) = to {
            self.pos = to;
            out.push(ToSim::Packet(self.conn, PlayIn::Move { pos: Some(to), rot: None, on_ground: true }));
        }
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
