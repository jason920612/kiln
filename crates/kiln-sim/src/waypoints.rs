//! The locator bar (vanilla `ServerWaypointManager`, `WaypointTransmitter`): each level's
//! players transmit their position to the other players in it, as a block position when
//! the receiver sees their chunk, a chunk position when they are within 332 blocks, else a
//! direction (azimuth). Connections are remade when they break (the transmitter moved more
//! than a block or chunk away from what was sent, or out of view), and follow the
//! `locator_bar` game rule. Players are the transmitters and receivers (vanilla's
//! `waypoint_transmit_range` / `waypoint_receive_range` of players are 6e7).
//!
//! Vanilla updates on every position change; Kiln updates once a tick for the players that
//! moved, which sends the same packets.
//!
//! A crowd makes this the quadratic part of the tick: every mover meets every other player
//! twice (as a transmitter in `updateWaypoint`, as a receiver in `updatePlayer`). A pair's
//! connection only changes when one of the two players takes its turn, and what it sends
//! goes to the receiver, so each receiver's share of the movers' turns (the moving
//! transmitters' turns in connection order, with its own `updatePlayer` turn at its place)
//! depends on nothing but its own connections and one snapshot of the players. The receivers
//! therefore run in parallel on the tick pool, each appending to its own outbox, which gives
//! every receiver the bytes, in the order, the serial loop would. Members of a level have
//! dense slots, connections are small `Copy` values in per-receiver rows indexed by the
//! transmitter's slot, and a transmitter's common packets (its block or chunk with its current
//! icon) are encoded once and shared by all receivers.
//!
//! Most steps in a crowd are quiet (nothing changed that the receiver was told), and which
//! ones are follows from the connections' kinds and the transmitters' movement alone (see
//! [`run_share`]), so a receiver finds the steps to run with set operations over the slots
//! instead of visiting every pair: the work follows the packets sent, not the pairs.

use crate::{DimId, Player, Sim};
use bytes::Bytes;
use kiln_command::scoreboard::Scoreboard;
use kiln_command::selector::SelectorTarget;
use kiln_command::vanilla::misc::TEAM_RGB;
use kiln_link::ConnId;
use kiln_proto::packets::hud::{self, WaypointAt, WaypointOp};
use kiln_sched::Window;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// `WaypointStyleAssets.DEFAULT`.
pub(crate) const DEFAULT_STYLE: &str = "minecraft:default";
/// `WaypointTransmitter.REALLY_FAR_DISTANCE`.
const REALLY_FAR: f32 = 332.0;

/// `Waypoint.Icon`: the style asset and the color (`None`: the team color, else the client's
/// default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Icon {
    pub style: String,
    pub color: Option<i32>,
}

impl Default for Icon {
    fn default() -> Self {
        Self { style: DEFAULT_STYLE.to_owned(), color: None }
    }
}

/// A connection's kind and what the receiver was last told.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Link {
    Block([i32; 3]),
    Chunk([i32; 2]),
    Azimuth(f32),
}

/// One connection: what was sent, and the style and color it was made with
/// (`Icon.cloneAndAssignStyle` when the connection was made) as an index into the level's
/// icons.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Connection {
    link: Link,
    icon: u32,
}

/// A connection as a row keeps it (16 bytes; a crowd's rows hold a million): the position,
/// chunk or angle bits, and the kind (low two bits, 0: no connection) with the icon. An
/// azimuth connection keeps its quiet deadline ([`azimuth_deadline`]) in the other two words.
#[derive(Debug, Clone, Copy)]
struct Stored {
    at: [i32; 3],
    meta: u32,
}

/// Equal connections: the deadline is a cache, not part of what was sent.
impl PartialEq for Stored {
    fn eq(&self, other: &Stored) -> bool {
        let n = if self.meta & 3 == 3 { 1 } else { 3 };
        self.meta == other.meta && self.at[..n] == other.at[..n]
    }
}

impl Stored {
    const NONE: Stored = Stored { at: [0; 3], meta: 0 };

    /// An azimuth connection's quiet deadline (0: none, step in full).
    fn deadline(self) -> f64 {
        f64::from_bits(u64::from(self.at[1] as u32) | (u64::from(self.at[2] as u32) << 32))
    }

    fn set_deadline(&mut self, d: f64) {
        let b = d.to_bits();
        self.at[1] = b as u32 as i32;
        self.at[2] = (b >> 32) as u32 as i32;
    }

    fn of(c: Option<Connection>) -> Stored {
        let Some(c) = c else { return Stored::NONE };
        let (at, kind) = match c.link {
            Link::Block(p) => (p, 1),
            Link::Chunk([x, z]) => ([x, z, 0], 2),
            Link::Azimuth(a) => ([a.to_bits() as i32, 0, 0], 3),
        };
        Stored { at, meta: c.icon << 2 | kind }
    }

    fn get(self) -> Option<Connection> {
        let link = match self.meta & 3 {
            0 => return None,
            1 => Link::Block(self.at),
            2 => Link::Chunk([self.at[0], self.at[1]]),
            _ => Link::Azimuth(f32::from_bits(self.at[0] as u32)),
        };
        Some(Connection { link, icon: self.meta >> 2 })
    }
}

/// A set of slots.
type Bits = Vec<u64>;

fn set_bit(b: &mut Bits, i: usize, on: bool) {
    if b.len() <= i / 64 {
        if !on {
            return;
        }
        b.resize(i / 64 + 1, 0);
    }
    if on {
        b[i / 64] |= 1 << (i % 64);
    } else {
        b[i / 64] &= !(1 << (i % 64));
    }
}

/// Word `i` of a set (zero past its end).
fn word(b: &[u64], i: usize) -> u64 {
    b.get(i).copied().unwrap_or(0)
}

/// The slots in a set, in slot order.
fn each_bit(words: impl Iterator<Item = u64>) -> impl Iterator<Item = usize> {
    words.enumerate().flat_map(|(i, mut w)| {
        std::iter::from_fn(move || {
            (w != 0).then(|| {
                let b = w.trailing_zeros() as usize;
                w &= w - 1;
                i * 64 + b
            })
        })
    })
}

/// A receiver's connections by the transmitter's slot, with the sets of transmitters it has
/// a block, chunk and azimuth connection from (so a turn can tell which steps may send
/// anything without looking at each connection).
#[derive(Debug, Default, Clone)]
struct Row {
    data: Vec<Stored>,
    /// Block, chunk and azimuth connections.
    kinds: [Bits; 3],
    /// Each azimuth connection's quiet deadline ([`azimuth_deadline`]) rounded down to `f32`, 0
    /// elsewhere, side by side (as long as `data`): a turn compares a word of slots at a time
    /// against the odometers instead of visiting each connection.
    deadlines: Vec<f32>,
}

/// Equal connections; the deadlines are a cache.
impl PartialEq for Row {
    fn eq(&self, other: &Row) -> bool {
        self.data == other.data && self.kinds == other.kinds
    }
}

/// `d` rounded down to `f32` (a deadline that comes no later).
fn round_down(d: f64) -> f32 {
    let f = d as f32;
    if f64::from(f) > d { f.next_down() } else { f }
}

impl Row {
    fn get(&self, s: usize) -> Option<Connection> {
        self.data.get(s).and_then(|d| d.get())
    }

    /// Replaces the connection; returns the old one.
    fn put(&mut self, s: usize, c: Option<Connection>) -> Option<Connection> {
        if self.data.len() <= s {
            if c.is_none() {
                return None;
            }
            self.data.resize(s + 1, Stored::NONE);
            self.deadlines.resize(s + 1, 0.0);
        }
        let new = Stored::of(c);
        let old = std::mem::replace(&mut self.data[s], new);
        self.deadlines[s] = 0.0;
        let (was, now) = ((old.meta & 3) as usize, (new.meta & 3) as usize);
        if was != now {
            if was != 0 {
                set_bit(&mut self.kinds[was - 1], s, false);
            }
            if now != 0 {
                set_bit(&mut self.kinds[now - 1], s, true);
            }
        }
        old.get()
    }

    /// The slots of the transmitters this receiver has a connection from.
    fn sources(&self) -> Vec<usize> {
        let n = self.kinds.iter().map(Vec::len).max().unwrap_or(0);
        each_bit((0..n).map(|i| word(&self.kinds[0], i) | word(&self.kinds[1], i) | word(&self.kinds[2], i))).collect()
    }
}

/// A transmitter's or receiver's place in a level.
#[derive(Debug, Default)]
struct Member {
    /// `None`: a free slot.
    conn: Option<ConnId>,
    transmitting: bool,
    receiving: bool,
    /// As a receiver: the connection from each transmitter, by the transmitter's slot.
    row: Row,
}

/// One level's `ServerWaypointManager`.
#[derive(Debug, Default)]
pub(crate) struct WaypointManager {
    /// Transmitters, in the order they started.
    waypoints: Vec<ConnId>,
    /// Receivers.
    players: Vec<ConnId>,
    /// Every transmitter's and receiver's slot.
    slots: crate::FastMap<ConnId, u32>,
    members: Vec<Member>,
    free: Vec<u32>,
    /// Transmitters (by slot) a connection was made from this tick before the movers' turns:
    /// their connections may hold a position other than their last tick's.
    dirty: Vec<bool>,
    /// The styles and colors connections were made with.
    icons: Vec<(String, Option<i32>)>,
    /// Buckets of vanilla's `HashSet` of transmitters (16, doubling past a load of 0.75,
    /// never shrinking), for its iteration order.
    capacity: usize,
    /// By slot: how far the member has moved, summed over the positions the turns saw (blocks;
    /// it only grows), and the position of the last turns.
    odo: Vec<f64>,
    odo_pos: Vec<Option<[f64; 3]>>,
}

impl WaypointManager {
    fn icon_id(&mut self, style: &str, color: Option<i32>) -> u32 {
        match self.icons.iter().position(|(s, c)| s == style && *c == color) {
            Some(i) => i as u32,
            None => {
                self.icons.push((style.to_owned(), color));
                (self.icons.len() - 1) as u32
            }
        }
    }

    fn slot(&self, conn: ConnId) -> Option<usize> {
        self.slots.get(&conn).map(|&s| s as usize)
    }

    /// The member's slot, taking a free one for a newcomer.
    fn join(&mut self, conn: ConnId) -> usize {
        if let Some(s) = self.slot(conn) {
            return s;
        }
        let s = match self.free.pop() {
            Some(s) => s as usize,
            None => {
                self.members.push(Member::default());
                self.members.len() - 1
            }
        };
        self.members[s].conn = Some(conn);
        self.slots.insert(conn, s as u32);
        self.mark_dirty(s);
        s
    }

    /// Adds how far each member moved since the last turns to its odometer (at every turns,
    /// before any step, so a deadline set at one turns holds at the next).
    fn advance_odometers(&mut self, snaps: &[Option<Snap>]) {
        self.odo.resize(snaps.len(), 0.0);
        self.odo_pos.resize(snaps.len(), None);
        for (s, snap) in snaps.iter().enumerate() {
            let Some(snap) = snap else { continue };
            if let Some(last) = self.odo_pos[s] {
                let d2: f64 = (0..3).map(|i| (snap.pos[i] - last[i]).powi(2)).sum();
                self.odo[s] += d2.sqrt();
            }
            self.odo_pos[s] = Some(snap.pos);
        }
    }

    /// Frees the slot of a member that neither transmits nor receives any more.
    fn leave_if_idle(&mut self, s: usize) {
        let m = &mut self.members[s];
        if m.transmitting || m.receiving {
            return;
        }
        if let Some(conn) = m.conn.take() {
            self.slots.remove(&conn);
        }
        m.row = Row::default();
        for m in &mut self.members {
            m.row.put(s, None);
        }
        self.free.push(s as u32);
    }

    /// Replaces the connection; returns the old one.
    fn set(&mut self, receiver: usize, source: usize, c: Option<Connection>) -> Option<Connection> {
        self.members[receiver].row.put(source, c)
    }

    fn mark_dirty(&mut self, s: usize) {
        if self.dirty.len() <= s {
            self.dirty.resize(s + 1, false);
        }
        self.dirty[s] = true;
    }
}

/// What the locator bar reads of a player: its position as vanilla compares it.
#[derive(Debug, Clone, Copy)]
struct Snap {
    conn: ConnId,
    pos: [f64; 3],
    block: [i32; 3],
    chunk: [i32; 2],
    /// The chunk the receiver's view is centered on, and its view distance.
    center: [i32; 2],
    view: i32,
    mode: u8,
    first_tick: bool,
    /// Its block (chunk) differs from the one at the last tick's turns.
    block_moved: bool,
    chunk_moved: bool,
}

impl Snap {
    fn of(p: &Player) -> Snap {
        let block = p.pos.map(|c| c.floor() as i32);
        let last = p.waypoint_last_pos.map(|c| c.floor() as i32);
        Snap {
            conn: p.conn,
            pos: p.pos,
            block,
            chunk: [block[0] >> 4, block[2] >> 4],
            center: [p.center.x, p.center.z],
            view: p.view_distance,
            mode: p.game_mode,
            first_tick: p.waypoint_first_tick,
            block_moved: last != block,
            chunk_moved: [last[0] >> 4, last[2] >> 4] != [block[0] >> 4, block[2] >> 4],
        }
    }
}

/// `Entity.distanceTo` (single precision, as vanilla compares it).
fn distance(a: &Snap, b: &Snap) -> f32 {
    let d: f64 = (0..3).map(|i| (a.pos[i] - b.pos[i]).powi(2)).sum();
    (d as f32).sqrt()
}

/// `ChunkTrackingView.isInViewDistance` of the receiver.
fn chunk_visible(chunk: [i32; 2], receiver: &Snap) -> bool {
    let dx = i64::from(((chunk[0] - receiver.center[0]).abs() - 1).max(0));
    let dz = i64::from(((chunk[1] - receiver.center[1]).abs() - 1).max(0));
    dx * dx + dz * dz < i64::from(receiver.view) * i64::from(receiver.view)
}

/// `WaypointTransmitter.doesSourceIgnoreReceiver`: spectators transmit to spectators only.
fn ignores(source: &Snap, receiver: &Snap) -> bool {
    receiver.mode != 3 && source.mode == 3
}

/// `EntityAzimuthConnection`'s angle: `atan2` of the receiver-to-source offset turned 90°.
fn azimuth(source: &Snap, receiver: &Snap) -> f32 {
    let (dx, dz) = (receiver.pos[0] - source.pos[0], receiver.pos[2] - source.pos[2]);
    // `Vec3.rotateClockwise90`: (x, y, z) -> (-z, y, x).
    kiln_command::coords::mth_atan2(dx, -dz) as f32
}

/// `LivingEntity.makeWaypointConnectionWith`: the link a new connection starts with.
fn make_link(source: &Snap, receiver: &Snap) -> Option<Link> {
    if source.first_tick || source.conn == receiver.conn || ignores(source, receiver) {
        return None;
    }
    Some(if distance(source, receiver) > REALLY_FAR {
        Link::Azimuth(azimuth(source, receiver))
    } else if !chunk_visible(source.chunk, receiver) {
        Link::Chunk(source.chunk)
    } else {
        Link::Block(source.block)
    })
}

/// `Connection.isBroken`.
fn is_broken(link: Link, source: &Snap, receiver: &Snap) -> bool {
    match link {
        Link::Block(last) => {
            let now = source.block;
            (0..3).map(|i| (now[i] - last[i]).abs()).sum::<i32>() > 1 || ignores(source, receiver)
        }
        Link::Chunk(last) => {
            let now = source.chunk;
            (now[0] - last[0]).abs().max((now[1] - last[1]).abs()) > 1
                || ignores(source, receiver)
                || chunk_visible(last, receiver)
        }
        Link::Azimuth(_) => {
            ignores(source, receiver) || chunk_visible(source.chunk, receiver) || distance(source, receiver) <= REALLY_FAR
        }
    }
}

/// How far an azimuth must turn (radians) before an intact connection sends it.
const AZIMUTH_STEP: f32 = 0.008_726_646;
/// A bound on how far `Mth.atan2` strays from the true angle (radians; under 1e-5 measured,
/// see the tests), with room to spare.
const ATAN2_ERROR: f64 = 1e-4;

/// The odometer sum (transmitter's plus receiver's, [`WaypointManager::advance_odometers`])
/// below which the step of an intact azimuth connection is certainly quiet: `now` is the
/// angle the step just computed and `told` the one the receiver has. Until the two have
/// moved `budget` blocks between them, the offset from the transmitter to the receiver has
/// moved at most that far, so the distance stays beyond `REALLY_FAR` and the true angle
/// turns by at most `asin(budget / horizontal) <= pi/2 * budget / horizontal`; with the
/// table's error on both computed angles (and the rounding to `f32`) that keeps the change
/// within `AZIMUTH_STEP`, and away from the jump at +-pi, as the full step would find. 0:
/// no deadline (the next step runs in full).
fn azimuth_deadline(odo: f64, src: &Snap, me: &Snap, now: f32, told: f32) -> f64 {
    let d: [f64; 3] = std::array::from_fn(|i| me.pos[i] - src.pos[i]);
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let far = dist - f64::from(REALLY_FAR) - 0.05;
    let slack = f64::from(AZIMUTH_STEP) - f64::from((now - told).abs()) - 2.0 * ATAN2_ERROR - 1e-5;
    let cut = std::f64::consts::PI - f64::from(now.abs()) - ATAN2_ERROR - 1e-5;
    // An offset `horizontal` long moved by `b` turns by at most `asin(b / horizontal)`.
    let turn = horizontal * slack.min(cut).min(std::f64::consts::FRAC_PI_2).sin() * (1.0 - 1e-9);

    let budget = far.min(turn) - 1e-6;
    if budget <= 0.0 { 0.0 } else { odo + budget }
}

/// What an intact connection sends when its transmitter moved (`Connection.update`).
fn next_link(link: Link, source: &Snap, receiver: &Snap) -> Option<Link> {
    match link {
        Link::Block(last) => (source.block != last).then_some(Link::Block(source.block)),
        Link::Chunk(last) => (source.chunk != last).then_some(Link::Chunk(source.chunk)),
        Link::Azimuth(last) => {
            let now = azimuth(source, receiver);
            ((now - last).abs() > AZIMUTH_STEP).then_some(Link::Azimuth(now))
        }
    }
}

fn packet(op: WaypointOp, uuid: Uuid, icons: &[(String, Option<i32>)], c: &Connection) -> Bytes {
    let at = match c.link {
        Link::Block(p) => WaypointAt::Block(p),
        Link::Chunk(p) => WaypointAt::Chunk(p),
        Link::Azimuth(a) => WaypointAt::Azimuth(a),
    };
    let (style, color) = &icons[c.icon as usize];
    hud::tracked_waypoint(op, uuid, style, *color, at)
}

fn untrack_packet(uuid: Uuid) -> Bytes {
    hud::tracked_waypoint(WaypointOp::Untrack, uuid, DEFAULT_STYLE, None, WaypointAt::Empty)
}

/// `Waypoint.Icon.cloneAndAssignStyle`'s team color: the transmitter's team color (black
/// drawn as dark gray).
fn team_color(scoreboard: &Scoreboard, name: &str) -> Option<i32> {
    let team = scoreboard.team_of(name)?;
    team.color.map(|i| if i == 0 { -13_619_152 } else { TEAM_RGB[i] })
}

/// What one transmitter-receiver pair does at the transmitter's or the receiver's turn.
enum Step {
    Quiet,
    Send(WaypointOp, Connection),
    Untrack,
}

/// `createConnection` for a pair without a connection, `updateConnection` for one with: an
/// intact connection sends what changed, a broken one is remade, and a pair that cannot have
/// a connection (the transmitter's first tick, a spectator) has none.
fn step_pair(entry: &mut Option<Connection>, source: &Snap, receiver: &Snap, fresh: u32) -> Step {
    match entry {
        Some(c) => {
            if !is_broken(c.link, source, receiver) {
                return match next_link(c.link, source, receiver) {
                    Some(link) => {
                        c.link = link;
                        Step::Send(WaypointOp::Update, *c)
                    }
                    None => Step::Quiet,
                };
            }
            match make_link(source, receiver) {
                Some(link) => {
                    *c = Connection { link, icon: fresh };
                    Step::Send(WaypointOp::Track, *c)
                }
                None => {
                    *entry = None;
                    Step::Untrack
                }
            }
        }
        None => match make_link(source, receiver) {
            Some(link) => {
                let c = Connection { link, icon: fresh };
                *entry = Some(c);
                Step::Send(WaypointOp::Track, c)
            }
            None => Step::Quiet,
        },
    }
}

/// A transmitter during the movers' turns: the icon a new connection from it gets, and its
/// packets shared by all receivers.
struct Tx {
    uuid: Uuid,
    block: [i32; 3],
    chunk: [i32; 2],
    fresh: u32,
    /// Track and update of its block and of its chunk with `fresh`, and its untrack.
    shared: [OnceLock<Bytes>; 5],
}

impl Tx {
    /// The packet for a step of a connection from this transmitter.
    fn bytes(&self, op: WaypointOp, c: &Connection, icons: &[(String, Option<i32>)]) -> Bytes {
        let op_i = match op {
            WaypointOp::Track => 0,
            WaypointOp::Update => 1,
            WaypointOp::Untrack => return self.untrack(),
        };
        let shared = c.icon == self.fresh
            && match c.link {
                Link::Block(b) => b == self.block,
                Link::Chunk(ch) => ch == self.chunk,
                Link::Azimuth(_) => false,
            };
        if !shared {
            return packet(op, self.uuid, icons, c);
        }
        let i = op_i * 2 + usize::from(matches!(c.link, Link::Chunk(_)));
        self.shared[i].get_or_init(|| packet(op, self.uuid, icons, c)).clone()
    }

    fn untrack(&self) -> Bytes {
        self.shared[4].get_or_init(|| untrack_packet(self.uuid)).clone()
    }
}

/// One receiver's share of the movers' turns: its row (moved out of the level while the
/// receivers run) and its player's outbox, which gets its packets.
struct Share<'p> {
    slot: usize,
    row: Mutex<Row>,
    outbox: Mutex<Option<&'p mut Vec<Bytes>>>,
}

/// The level during the movers' turns, by slot: the members' snapshots (`None`: gone), the
/// transmitters' icons and packets, and sets of slots for telling the steps that may send
/// anything from the quiet ones.
struct Turns<'a> {
    /// Each mover's turn, in connection order (`u32::MAX`: not moving), by slot.
    rank: Vec<u32>,
    /// Each transmitter's place in their order (`u32::MAX`: not transmitting), by slot.
    tpos: Vec<u32>,
    /// The movers (with whether they transmit) and the transmitters in their orders, for
    /// [`verify`].
    movers: Vec<(u32, bool)>,
    transmitters: Vec<u32>,
    snaps: Vec<Option<Snap>>,
    tx: Vec<Option<Tx>>,
    icons: &'a [(String, Option<i32>)],
    /// Set `i` (`words` long from `i * words`): the moving transmitters whose turn comes
    /// before turn `i` (the last: all).
    before: Vec<u64>,
    /// Present transmitters, and those of them that are spectators.
    present: Bits,
    spectators: Bits,
    /// Transmitters whose every block (chunk) connection holds their current block (chunk):
    /// they have not changed block (chunk) since the last tick's turns and no connection was
    /// made from them since.
    settled_block: Bits,
    settled_chunk: Bits,
    /// Present transmitters by chunk, for the chunks a receiver sees.
    by_chunk: crate::FastMap<[i32; 2], Vec<u32>>,
    /// [`visible_set`] of each view (centre chunk and distance) a receiver has: a crowd's
    /// receivers share a few.
    views: crate::FastMap<([i32; 2], i32), Bits>,
    words: usize,
    /// The members' odometers, by slot.
    odo: &'a [f64],
}

/// The transmitters in the chunks `me` sees (`ChunkTrackingView.isInViewDistance`).
fn visible_set(by_chunk: &crate::FastMap<[i32; 2], Vec<u32>>, words: usize, me: &Snap) -> Bits {
    let mut vis = vec![0u64; words];
    let v = me.view.max(0);
    let mut add = |chunk: &[i32; 2], slots: &Vec<u32>| {
        if chunk_visible(*chunk, me) {
            for &s in slots {
                vis[s as usize / 64] |= 1 << (s % 64);
            }
        }
    };
    let side = 2 * i64::from(v) + 1;
    if (side * side) as usize <= by_chunk.len() {
        for x in me.center[0] - v..=me.center[0] + v {
            for z in me.center[1] - v..=me.center[1] + v {
                if let Some(slots) = by_chunk.get(&[x, z]) {
                    add(&[x, z], slots);
                }
            }
        }
    } else {
        for (chunk, slots) in by_chunk {
            add(chunk, slots);
        }
    }
    vis
}

/// Runs one receiver's share: the moving transmitters' turns in connection order and, at its
/// own place among them, its own `updatePlayer` turn over every transmitter.
///
/// A block or chunk connection always holds the transmitter's block or chunk of its last step.
/// While that is still the transmitter's current one (it has not changed block since, or its
/// connection to this receiver was already stepped this tick), the step of an intact block
/// connection is quiet, and so is that of a chunk connection while the receiver does not see
/// the chunk (`is_broken` false, nothing to update), unless the transmitter is a spectator.
/// The share finds the other steps with set operations over the slots, in three parts (the
/// turns before its own, its own, the turns after), and runs only those in full, in order.
fn run_share(share: &Share, t: &Turns) {
    let r = share.slot;
    let Some(me) = t.snaps[r].as_ref() else { return };
    let mut row = share.row.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let row = &mut *row;
    let mut outbox = share.outbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(out) = outbox.as_deref_mut() else { return };
    if VERIFY.load(std::sync::atomic::Ordering::Relaxed) {
        let (before, sent) = (row.clone(), out.len());
        quick_share(row, r, me, t, out);
        let (mut want_row, mut want) = (before, Vec::new());
        every_step(&mut want_row, r, me, t, &mut want);
        assert!(out[sent..] == want[..] && *row == want_row, "locator bar: the quick share of slot {r} differs from stepping every pair");
        return;
    }
    let sent = out.len();
    let steps = quick_share(row, r, me, t, out);
    COUNTS[0].fetch_add(steps, std::sync::atomic::Ordering::Relaxed);
    COUNTS[1].fetch_add((out.len() - sent) as u64, std::sync::atomic::Ordering::Relaxed);
}

/// Steps run in full and packets sent by the shares since the last turns (`KILN_PHASE_DETAIL`).
static COUNTS: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

/// Checks every quick share against stepping every pair ([`verify`]).
static VERIFY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From now on, every receiver's share is checked against stepping every pair, as the
/// straightforward loop over the movers does (tests).
pub fn verify(on: bool) {
    VERIFY.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// A receiver's share the straightforward way: every moving transmitter's turn and its own
/// turn over every transmitter, each pair stepped in full.
fn every_step(row: &mut Row, r: usize, me: &Snap, t: &Turns, out: &mut Vec<Bytes>) {
    for &(m, transmitting) in &t.movers {
        let m = m as usize;
        if m == r {
            for &w in &t.transmitters {
                if w as usize != r && t.snaps[w as usize].is_some() {
                    full_step(row, r, w as usize, me, t, out, false);
                }
            }
        } else if transmitting {
            full_step(row, r, m, me, t, out, false);
        }
    }
}

/// Returns how many steps it ran in full.
fn quick_share(row: &mut Row, r: usize, me: &Snap, t: &Turns, out: &mut Vec<Bytes>) -> u64 {
    let steps = std::cell::Cell::new(0u64);
    let own;
    let vis: &Bits = match t.views.get(&(me.center, me.view)) {
        Some(v) => v,
        None => {
            own = visible_set(&t.by_chunk, t.words, me);
            &own
        }
    };
    let own = t.rank[r];
    let w = t.words;
    let earlier_than = |i: usize| &t.before[i * w..(i + 1) * w];
    let all_movers = earlier_than(t.before.len() / w.max(1) - 1);
    // Azimuth connections that neither the spectator rule nor the view can break: their steps
    // are quiet before their deadlines (`full_step`).
    let unbreakable = |i: usize| !vis[i] & if me.mode == 3 { !0 } else { !t.spectators[i] };
    let odo_r = t.odo[r];
    // The steps of `among` that may send anything; `stepped`: whose connection to this
    // receiver was stepped already this tick.
    let open = |row: &Row, among: &dyn Fn(usize) -> u64, stepped: &dyn Fn(usize) -> u64| -> Vec<usize> {
        let words = (0..w).map(|i| {
            let (k1, k2) = (word(&row.kinds[0], i), word(&row.kinds[1], i));
            let st = stepped(i);
            let quiet = t.present[i]
                & !t.spectators[i]
                & ((k1 & (t.settled_block[i] | st)) | (k2 & (t.settled_chunk[i] | st) & !vis[i]));
            let mut open = among(i) & t.present[i] & !quiet;
            let azimuths = open & word(&row.kinds[2], i) & unbreakable(i);
            if azimuths != 0 {
                // The word's connections before their deadlines, all 64 compared at once.
                let at = (i * 64).min(row.deadlines.len());
                let deadlines = &row.deadlines[at..(at + 64).min(row.deadlines.len())];
                let odo = &t.odo[at.min(t.odo.len())..(at + deadlines.len()).min(t.odo.len())];
                let mut before = 0u64;
                for (k, (&o, &d)) in odo.iter().zip(deadlines).enumerate() {
                    before |= u64::from(o + odo_r < f64::from(d)) << k;
                }
                open &= !(azimuths & before);
            }
            open
        });
        let open: Vec<usize> = each_bit(words).filter(|&s| s != r).collect();
        prefetch(row, &open);
        steps.set(steps.get() + open.len() as u64);
        open
    };
    let by_rank = |mut v: Vec<usize>| {
        v.sort_unstable_by_key(|&s| t.rank[s]);
        v
    };
    if own == u32::MAX {
        // Not moving: the moving transmitters' turns.
        for s in by_rank(open(row, &|i| all_movers[i], &|_| 0)) {
            full_step(row, r, s, me, t, out, true);
        }
        return steps.get();
    }
    let earlier = earlier_than(own as usize);
    // `updateWaypoint` of the movers before this receiver: their connections to it.
    for s in by_rank(open(row, &|i| earlier[i], &|_| 0)) {
        full_step(row, r, s, me, t, out, true);
    }
    // `updatePlayer`: this receiver's connection to every transmitter, in their order.
    let mut mine = open(row, &|_| !0, &|i| earlier[i]);
    mine.retain(|&s| t.tpos[s] != u32::MAX);
    mine.sort_unstable_by_key(|&s| t.tpos[s]);
    for s in mine {
        full_step(row, r, s, me, t, out, true);
    }
    // `updateWaypoint` of the movers after it (all stepped at its turn).
    for s in by_rank(open(row, &|i| all_movers[i] & !earlier[i], &|_| !0)) {
        full_step(row, r, s, me, t, out, true);
    }
    steps.get()
}

/// Asks for the connections about to be stepped (scattered over a row that other work has
/// pushed out of the cache since the last tick) all at once, rather than one miss at a time.
fn prefetch(row: &Row, slots: &[usize]) {
    #[cfg(target_arch = "x86_64")]
    for &s in slots {
        if let Some(d) = row.data.get(s) {
            // SAFETY: a prefetch of a valid address has no effect beyond the cache.
            unsafe { std::arch::x86_64::_mm_prefetch(std::ptr::from_ref(d).cast::<i8>(), std::arch::x86_64::_MM_HINT_T0) };
        }
    }
    let _ = (row, slots);
}

/// One step of the connection from transmitter `s` to the receiver `me` (slot `r`) in full.
/// `quick`: an intact azimuth connection whose step is certainly quiet ([`azimuth_deadline`])
/// is left alone (the straightforward path for [`verify`] steps it).
fn full_step(row: &mut Row, r: usize, s: usize, me: &Snap, t: &Turns, out: &mut Vec<Bytes>, quick: bool) {
    let (Some(src), Some(tx)) = (t.snaps[s].as_ref(), t.tx[s].as_ref()) else { return };
    // An azimuth connection is intact while neither the spectator rule nor the receiver's view
    // breaks it and the transmitter stays beyond `REALLY_FAR` (`is_broken`); then it sends the
    // angle once it turned by more than `AZIMUTH_STEP` (`next_link`).
    if quick
        && let Some(stored) = row.data.get(s).copied()
        && stored.meta & 3 == 3
        && !ignores(src, me)
        && !chunk_visible(src.chunk, me)
    {
        let odo = t.odo[s] + t.odo[r];
        if odo < stored.deadline() {
            return;
        }
        if distance(src, me) > REALLY_FAR {
            let last = f32::from_bits(stored.at[0] as u32);
            let now = azimuth(src, me);
            let (mut new, told) = if (now - last).abs() > AZIMUTH_STEP {
                let c = Connection { link: Link::Azimuth(now), icon: stored.meta >> 2 };
                out.push(tx.bytes(WaypointOp::Update, &c, t.icons));
                (Stored::of(Some(c)), now)
            } else {
                (stored, last)
            };
            let deadline = azimuth_deadline(odo, src, me, now, told);
            new.set_deadline(deadline);
            row.data[s] = new;
            row.deadlines[s] = round_down(deadline);
            return;
        }
    }
    let mut c = row.get(s);
    match step_pair(&mut c, src, me, tx.fresh) {
        Step::Quiet => return,
        Step::Send(op, c) => out.push(tx.bytes(op, &c, t.icons)),
        Step::Untrack => out.push(tx.untrack()),
    }
    row.put(s, c);
}

/// Window hint: a member's snapshot, a few cache misses.
const SNAP_WINDOW: Window = Window::new().item_ns(300);

/// Window hint: a receiver's share is a few microseconds in a crowd.
const SHARE_WINDOW: Window = Window::new().item_ns(5_000);

impl Sim {
    fn locator_bar(&self) -> bool {
        self.rule_bool("minecraft:locator_bar")
    }

    /// Untrack packet for a connection whose transmitter may be gone.
    fn send_untrack(&mut self, receiver: ConnId, source_uuid: Uuid) {
        if let Some(r) = self.players.get_mut(&receiver) {
            r.send(untrack_packet(source_uuid));
        }
    }

    /// `createConnection`: a fresh connection replaces any old one.
    fn create_waypoint_connection(&mut self, dim: DimId, receiver: ConnId, source: ConnId, on: bool) {
        if receiver == source || !on {
            return;
        }
        let (Some(s), Some(r)) = (self.players.get(&source), self.players.get(&receiver)) else { return };
        let (ss, rs, uuid) = (Snap::of(s), Snap::of(r), s.uuid);
        let mgr = &mut self.waypoints[dim];
        let (Some(ri), Some(si)) = (mgr.slot(receiver), mgr.slot(source)) else { return };
        match make_link(&ss, &rs) {
            Some(link) => {
                let color = s.waypoint_icon.color.or_else(|| team_color(&self.commands.scoreboard, &s.name));
                let icon = mgr.icon_id(&s.waypoint_icon.style, color);
                let c = Connection { link, icon };
                mgr.set(ri, si, Some(c));
                mgr.mark_dirty(si);
                let pkt = packet(WaypointOp::Track, uuid, &mgr.icons, &c);
                if let Some(r) = self.players.get_mut(&receiver) {
                    r.send(pkt);
                }
            }
            None => {
                if mgr.set(ri, si, None).is_some() {
                    self.send_untrack(receiver, uuid);
                }
            }
        }
    }

    /// `trackWaypoint`.
    pub(crate) fn track_waypoint(&mut self, dim: DimId, source: ConnId) {
        let on = self.locator_bar();
        let m = &mut self.waypoints[dim];
        let s = m.join(source);
        if !m.members[s].transmitting {
            m.members[s].transmitting = true;
            m.waypoints.push(source);
            m.capacity = m.capacity.max(16);
            if m.waypoints.len() > m.capacity * 3 / 4 {
                m.capacity *= 2;
            }
        }
        for r in self.waypoints[dim].players.clone() {
            self.create_waypoint_connection(dim, r, source, on);
        }
    }

    /// `untrackWaypoint`.
    pub(crate) fn untrack_waypoint(&mut self, dim: DimId, source: ConnId, source_uuid: Uuid) {
        let m = &mut self.waypoints[dim];
        let Some(s) = m.slot(source) else { return };
        let mut receivers: Vec<ConnId> = Vec::new();
        for member in &mut m.members {
            if member.row.put(s, None).is_some() {
                receivers.extend(member.conn);
            }
        }
        receivers.sort_unstable();
        for r in receivers {
            self.send_untrack(r, source_uuid);
        }
        let m = &mut self.waypoints[dim];
        if m.members[s].transmitting {
            m.members[s].transmitting = false;
            m.waypoints.retain(|w| *w != source);
        }
        m.leave_if_idle(s);
    }

    /// `addPlayer` when a player enters a level: it receives the level's waypoints and
    /// transmits its own (unless crouching).
    pub(crate) fn waypoints_add_player(&mut self, dim: DimId, conn: ConnId) {
        let on = self.locator_bar();
        let m = &mut self.waypoints[dim];
        let s = m.join(conn);
        if !m.members[s].receiving {
            m.members[s].receiving = true;
            m.players.push(conn);
        }
        for w in self.waypoints[dim].waypoints.clone() {
            self.create_waypoint_connection(dim, conn, w, on);
        }
        if self.players.get(&conn).is_some_and(|p| !p.sneaking) {
            self.track_waypoint(dim, conn);
        }
    }

    /// `/waypoint list`: the level's transmitters' names.
    /// In `HashSet` order: entities hash to their ids (`Entity.hashCode`), so by bucket
    /// `(id ^ id >>> 16) & (capacity - 1)`, then by insertion.
    pub(crate) fn waypoint_names(&self, dim: DimId) -> Vec<kiln_command::Text> {
        let m = &self.waypoints[dim];
        let mut players: Vec<(usize, usize, &Player)> = m
            .waypoints
            .iter()
            .enumerate()
            .filter_map(|(i, c)| self.players.get(c).map(|p| (i, p)))
            .map(|(i, p)| {
                let h = p.entity_id as u32;
                (((h ^ (h >> 16)) as usize) & (m.capacity.max(16) - 1), i, p)
            })
            .collect();
        players.sort_unstable_by_key(|(bucket, i, _)| (*bucket, *i));
        players
            .into_iter()
            .map(|(_, _, p)| crate::commands::PlayerRef::of(p.conn, p, &self.commands.scoreboard).display_name())
            .collect()
    }

    /// `removePlayer` when a player leaves a level (or the server).
    pub(crate) fn waypoints_remove_player(&mut self, dim: DimId, conn: ConnId, uuid: Uuid) {
        let m = &mut self.waypoints[dim];
        let Some(slot) = m.slot(conn) else { return };
        let row = std::mem::take(&mut m.members[slot].row);
        let mut sources: Vec<ConnId> = row.sources().into_iter().filter_map(|s| m.members[s].conn).collect();
        sources.sort_unstable();
        for s in sources {
            if let Some(src) = self.players.get(&s).map(|p| p.uuid) {
                self.send_untrack(conn, src);
            }
        }
        self.untrack_waypoint(dim, conn, uuid);
        let m = &mut self.waypoints[dim];
        if let Some(slot) = m.slot(conn) {
            m.members[slot].receiving = false;
            m.leave_if_idle(slot);
        }
        m.players.retain(|p| *p != conn);
    }

    /// Once a tick: `updateWaypoint` and `updatePlayer` for the players that moved, then the
    /// joined players' first tick ends.
    pub(crate) fn tick_waypoints(&mut self) {
        let on = self.locator_bar();
        // One pass over the players: what each needs, in connection order.
        let mut looked: Vec<(ConnId, DimId, Uuid, Option<DimId>, bool, bool)> = self
            .players
            .iter()
            .map(|(&c, p)| (c, p.dim, p.uuid, p.waypoint_dim, p.sneaking, !p.waypoint_first_tick && p.waypoint_last_pos != p.pos))
            .collect();
        looked.sort_unstable_by_key(|l| l.0);
        for &(conn, dim, uuid, registered, sneaking, _) in &looked {
            if registered != Some(dim) {
                if let Some(old) = registered {
                    self.waypoints_remove_player(old, conn, uuid);
                }
                self.waypoints_add_player(dim, conn);
                self.players.get_mut(&conn).expect("player").waypoint_dim = Some(dim);
            }
            // Crouching zeroes `waypoint_transmit_range` (`updatePlayerAttributes`).
            let m = &self.waypoints[dim];
            let transmitting = m.slot(conn).is_some_and(|s| m.members[s].transmitting);
            if sneaking && transmitting {
                self.untrack_waypoint(dim, conn, uuid);
            } else if !sneaking && !transmitting {
                self.track_waypoint(dim, conn);
            }
        }
        // `SimConfig::locator_interval`: the movers' turns every so many ticks (positions and
        // the connections made meanwhile wait for them).
        let interval = i64::from(self.config.locator_interval.max(1));
        let turns = !on || self.game_time % interval == 0;
        if on && turns {
            let mut moved: [Vec<ConnId>; 3] = Default::default();
            for &(conn, dim, _, _, _, mover) in &looked {
                if mover {
                    moved[dim].push(conn);
                }
            }
            for (dim, moved) in moved.iter().enumerate() {
                if !moved.is_empty() {
                    self.waypoint_moves(dim, moved);
                }
            }
        }
        for p in self.players.values_mut() {
            if turns {
                p.waypoint_last_pos = p.pos;
            }
            p.waypoint_first_tick = false;
        }
        if turns {
            for m in &mut self.waypoints {
                m.dirty.iter_mut().for_each(|d| *d = false);
            }
        }
    }

    /// The movers' turns in one level (`moved`: in connection order).
    fn waypoint_moves(&mut self, dim: DimId, moved: &[ConnId]) {
        let t0 = std::time::Instant::now();
        let (players, scoreboard, pool) = (&self.players, &self.commands.scoreboard, &mut self.pool);
        let mgr = &mut self.waypoints[dim];
        // The icons new connections from each transmitter get, interned before the receivers
        // run, and the members' snapshots.
        // (Side by side: a crowd's players are a thousand scattered structures.)
        let looked: Vec<(Option<Snap>, Option<(Uuid, &str, Option<i32>, Option<u32>)>)> = {
            let icons = &mgr.icons;
            pool.serial(|ctx| {
                ctx.map_indexed_with(SNAP_WINDOW, &mgr.members, |_, m| {
                    let p = m.conn.and_then(|c| players.get(&c));
                    let tx = match (m.transmitting, p) {
                        (true, Some(p)) => {
                            let color = p.waypoint_icon.color.or_else(|| team_color(scoreboard, &p.name));
                            let style = p.waypoint_icon.style.as_str();
                            let known = icons.iter().position(|(s, c)| s == style && *c == color).map(|i| i as u32);
                            Some((p.uuid, style, color, known))
                        }
                        _ => None,
                    };
                    (p.map(Snap::of), tx)
                })
            })
        };
        let mut tx: Vec<Option<Tx>> = Vec::with_capacity(looked.len());
        let mut snaps: Vec<Option<Snap>> = Vec::with_capacity(looked.len());
        for (snap, t) in looked {
            tx.push(t.map(|(uuid, style, color, known)| {
                let fresh = known.unwrap_or_else(|| mgr.icon_id(style, color));
                let snap = snap.expect("snapshot");
                Tx { uuid, block: snap.block, chunk: snap.chunk, fresh, shared: Default::default() }
            }));
            snaps.push(snap);
        }
        let ta = std::time::Instant::now();
        mgr.advance_odometers(&snaps);
        let n = mgr.members.len();
        let words = n.div_ceil(64);
        let mut rank = vec![u32::MAX; n];
        let mut turns: Vec<u32> = Vec::new();
        let mut all_movers: Vec<(u32, bool)> = Vec::new();
        let mut movers = 0;
        for &c in moved {
            let Some(slot) = mgr.slot(c) else { continue };
            rank[slot] = movers;
            movers += 1;
            let transmitting = mgr.members[slot].transmitting && snaps[slot].is_some();
            all_movers.push((slot as u32, transmitting));
            if transmitting {
                turns.push(slot as u32);
            }
        }
        // A moving receiver's own turn sits between the moving transmitters' turns: `before`
        // is indexed by the movers' order, so it counts the transmitting ones ahead of each.
        let mut before: Vec<u64> = Vec::with_capacity((movers as usize + 1) * words);
        let mut acc = vec![0u64; words];
        let mut next = turns.iter().peekable();
        for i in 0..=movers {
            while let Some(&&s) = next.peek() {
                if rank[s as usize] >= i {
                    break;
                }
                acc[s as usize / 64] |= 1 << (s % 64);
                next.next();
            }
            before.extend_from_slice(&acc);
        }
        let transmitters: Vec<u32> = mgr.waypoints.iter().filter_map(|c| mgr.slot(*c)).map(|s| s as u32).collect();
        let mut tpos = vec![u32::MAX; n];
        for (i, &s) in transmitters.iter().enumerate() {
            tpos[s as usize] = i as u32;
        }
        let (mut present, mut spectators, mut settled_block, mut settled_chunk) =
            (vec![0u64; words], vec![0u64; words], vec![0u64; words], vec![0u64; words]);
        let mut by_chunk: crate::FastMap<[i32; 2], Vec<u32>> = Default::default();
        for &s in &transmitters {
            let s = s as usize;
            let Some(snap) = snaps[s] else { continue };
            let dirty = mgr.dirty.get(s).copied().unwrap_or(false);
            set_bit(&mut present, s, true);
            set_bit(&mut spectators, s, snap.mode == 3);
            set_bit(&mut settled_block, s, !snap.block_moved && !dirty);
            set_bit(&mut settled_chunk, s, !snap.chunk_moved && !dirty);
            by_chunk.entry(snap.chunk).or_default().push(s as u32);
        }
        let td = std::time::Instant::now();
        let any_transmitting = !turns.is_empty();
        // Receivers that have turns to take: all of them when a transmitter moved, else the
        // moving receivers.
        let slots: Vec<usize> =
            mgr.players.iter().filter_map(|c| mgr.slot(*c)).filter(|&s| any_transmitting || rank[s] != u32::MAX).collect();
        // The receivers' outboxes, by slot: each share appends to its own.
        let mut outboxes: Vec<Option<&mut Vec<Bytes>>> = (0..n).map(|_| None).collect();
        for p in self.players.values_mut() {
            if let Some(s) = mgr.slot(p.conn) {
                outboxes[s] = Some(&mut p.outbox);
            }
        }
        let shares: Vec<Share> = slots
            .into_iter()
            .map(|slot| Share {
                slot,
                row: Mutex::new(std::mem::take(&mut mgr.members[slot].row)),
                outbox: Mutex::new(outboxes[slot].take()),
            })
            .collect();
        let mut views: crate::FastMap<([i32; 2], i32), Bits> = Default::default();
        for share in &shares {
            if let Some(me) = snaps[share.slot].as_ref() {
                views.entry((me.center, me.view)).or_insert_with(|| visible_set(&by_chunk, words, me));
            }
        }
        let turns = Turns {
            rank,
            tpos,
            movers: all_movers,
            transmitters,
            snaps,
            tx,
            icons: &mgr.icons,
            before,
            present,
            spectators,
            settled_block,
            settled_chunk,
            by_chunk,
            views,
            words,
            odo: &mgr.odo,
        };
        let t1 = std::time::Instant::now();
        pool.serial(|ctx| ctx.map_indexed_with(SHARE_WINDOW, &shares, |_, share| run_share(share, &turns)));
        let t2 = std::time::Instant::now();
        drop(turns);
        for share in shares {
            mgr.members[share.slot].row = share.row.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        // As thousands per tick.
        for (i, name) in ["w.steps_k", "w.sent_k"].into_iter().enumerate() {
            crate::diag::add(name, std::time::Duration::from_nanos(COUNTS[i].swap(0, std::sync::atomic::Ordering::Relaxed) * 1000));
        }
        crate::diag::add("w.snaps", ta - t0);
        crate::diag::add("w.sets", td - ta);
        crate::diag::add("w.prepare", t1 - td);
        crate::diag::add("w.shares", t2 - t1);
        crate::diag::lap("w.after", t2);
    }

    /// `/waypoint modify`: `mutateIcon` (untrack, change, track again).
    pub(crate) fn set_waypoint_icon(&mut self, conn: ConnId, change: impl FnOnce(&mut Icon)) -> bool {
        let Some(p) = self.players.get_mut(&conn) else { return false };
        change(&mut p.waypoint_icon);
        let (dim, uuid) = (p.dim, p.uuid);
        self.untrack_waypoint(dim, conn, uuid);
        self.track_waypoint(dim, conn);
        true
    }

    /// The `locator_bar` game rule changed (`MinecraftServer.onGameRuleChanged`): connections
    /// break when it turns off and are made again when it turns on.
    pub(crate) fn locator_bar_changed(&mut self) {
        let on = self.locator_bar();
        for dim in 0..self.waypoints.len() {
            let m = &mut self.waypoints[dim];
            let mut broken: Vec<(ConnId, ConnId)> = Vec::new();
            for r in 0..m.members.len() {
                let row = std::mem::take(&mut m.members[r].row);
                for s in row.sources() {
                    if let (Some(rc), Some(sc)) = (m.members[r].conn, m.members[s].conn) {
                        broken.push((rc, sc));
                    }
                }
            }
            broken.sort_unstable();
            for (r, s) in broken {
                if let Some(uuid) = self.players.get(&s).map(|p| p.uuid) {
                    self.send_untrack(r, uuid);
                }
            }
            if on {
                for w in self.waypoints[dim].waypoints.clone() {
                    for r in self.waypoints[dim].players.clone() {
                        self.create_waypoint_connection(dim, r, w, on);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azimuth_points_from_the_receiver() {
        // Vanilla's angle for a transmitter due north (-z) of the receiver.
        let (dx, dz) = (0.0f64, 500.0f64);
        let a = kiln_command::coords::mth_atan2(dx, -dz) as f32;
        assert!((a.abs() - std::f32::consts::PI).abs() < 1e-3);
    }

    /// xorshift for the sweeps below.
    fn rng(state: &mut u64) -> f64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `ATAN2_ERROR` holds with room: the table arctangent against the true one, over every
    /// direction and many lengths.
    #[test]
    fn table_atan2_stays_close_to_the_true_angle() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut worst = 0f64;
        for i in 0..2_000_000 {
            let a = (i as f64 / 2_000_000.0 - 0.5) * std::f64::consts::TAU + rng(&mut state) * 1e-6;
            let len = 10f64.powf(rng(&mut state) * 9.0 - 3.0);
            let (y, x) = (a.sin() * len, a.cos() * len);
            let d = kiln_command::coords::mth_atan2(y, x) - y.atan2(x);
            let d = [d, d - std::f64::consts::TAU, d + std::f64::consts::TAU].into_iter().map(f64::abs).fold(f64::MAX, f64::min);
            worst = worst.max(d);
        }
        assert!(worst < ATAN2_ERROR / 10.0, "table atan2 off by {worst}");
    }

    fn snap_at(pos: [f64; 3], view: i32) -> Snap {
        let block = pos.map(|c| c.floor() as i32);
        Snap {
            conn: 0,
            pos,
            block,
            chunk: [block[0] >> 4, block[2] >> 4],
            center: [block[0] >> 4, block[2] >> 4],
            view,
            mode: 0,
            first_tick: false,
            block_moved: true,
            chunk_moved: true,
        }
    }

    /// Whenever the deadline says an azimuth step is quiet, the full step finds it quiet:
    /// two players far apart walk, run and jump about (now and then right through the
    /// transmitter's far side, crossing the angle's jump at +-pi, and into `REALLY_FAR`).
    #[test]
    fn azimuth_deadline_only_skips_quiet_steps() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let (mut skipped, mut quiet_run) = (0u64, 0u64);
        for case in 0..400 {
            let far = 340.0 + rng(&mut state) * 2000.0;
            let a = rng(&mut state) * std::f64::consts::TAU;
            let mut src = [0.0, 64.0, 0.0];
            // Half the cases sit right on the jump at +-pi (the receiver due south).
            let mut me = if case % 2 == 0 { [far * a.cos(), 64.0, far * a.sin()] } else { [0.0, 64.0, far] };
            let (mut odo_s, mut odo_r) = (0f64, 0f64);
            let mut c = Some(Connection { link: Link::Azimuth(azimuth(&snap_at(src, 2), &snap_at(me, 2))), icon: 0 });
            let mut deadline = 0.0;
            for step in 0..3000 {
                let speed = if step % 97 == 0 { 8.0 } else if step % 7 == 0 { 0.0 } else { 0.3 };
                for (p, odo) in [(&mut src, &mut odo_s), (&mut me, &mut odo_r)] {
                    let d = [(rng(&mut state) - 0.5) * speed, (rng(&mut state) - 0.5) * speed * 0.2, (rng(&mut state) - 0.5) * speed];
                    for i in 0..3 {
                        p[i] += d[i];
                    }
                    *odo += (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                }
                let (s, r) = (snap_at(src, 2), snap_at(me, 2));
                let odo = odo_s + odo_r;
                let before = c;
                let full = step_pair(&mut c, &s, &r, 0);
                if odo < deadline {
                    skipped += 1;
                    assert!(matches!(full, Step::Quiet) && c == before, "case {case} step {step}: skipped a step that sends");
                }
                deadline = match (before, c) {
                    (Some(Connection { link: Link::Azimuth(told0), .. }), Some(Connection { link: Link::Azimuth(told), .. }))
                        if !is_broken(Link::Azimuth(told0), &s, &r) =>
                    {
                        quiet_run += u64::from(matches!(full, Step::Quiet));
                        azimuth_deadline(odo, &s, &r, azimuth(&s, &r), told)
                    }
                    _ => 0.0,
                };
                if !matches!(c, Some(Connection { link: Link::Azimuth(_), .. })) {
                    c = Some(Connection { link: Link::Azimuth(azimuth(&s, &r)), icon: 0 });
                    deadline = 0.0;
                }
            }
        }
        // The deadline does skip most of the quiet steps.
        assert!(skipped * 2 > quiet_run, "skipped {skipped} of {quiet_run} quiet steps");
    }
}
