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
//! therefore run in parallel on the tick pool and their packets are appended afterwards, which
//! gives every receiver the bytes, in the order, the serial loop would. Members of a level have
//! dense slots, connections are small `Copy` values in per-receiver rows indexed by the
//! transmitter's slot, and a transmitter's common packets (its block or chunk with its current
//! icon) are encoded once and shared by all receivers.

use crate::{DimId, Player, Sim};
use bytes::Bytes;
use kiln_command::scoreboard::Scoreboard;
use kiln_command::selector::SelectorTarget;
use kiln_command::vanilla::misc::TEAM_RGB;
use kiln_link::ConnId;
use kiln_proto::packets::hud::{self, WaypointAt, WaypointOp};
use kiln_sched::Window;
use std::collections::HashMap;
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

/// One connection: what was sent, and the style (an index into the level's styles) and color
/// it was made with (`Icon.cloneAndAssignStyle` when the connection was made).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Connection {
    link: Link,
    style: u32,
    color: Option<i32>,
}

/// A transmitter's or receiver's place in a level.
#[derive(Debug, Default)]
struct Member {
    /// `None`: a free slot.
    conn: Option<ConnId>,
    transmitting: bool,
    receiving: bool,
    /// As a receiver: the connection from each transmitter, by the transmitter's slot.
    row: Vec<Option<Connection>>,
}

/// One level's `ServerWaypointManager`.
#[derive(Debug, Default)]
pub(crate) struct WaypointManager {
    /// Transmitters, in the order they started.
    waypoints: Vec<ConnId>,
    /// Receivers.
    players: Vec<ConnId>,
    /// Every transmitter's and receiver's slot.
    slots: HashMap<ConnId, u32>,
    members: Vec<Member>,
    free: Vec<u32>,
    /// The styles connections were made with.
    styles: Vec<String>,
    /// Buckets of vanilla's `HashSet` of transmitters (16, doubling past a load of 0.75,
    /// never shrinking), for its iteration order.
    capacity: usize,
}

impl WaypointManager {
    fn style_id(&mut self, style: &str) -> u32 {
        match self.styles.iter().position(|s| s == style) {
            Some(i) => i as u32,
            None => {
                self.styles.push(style.to_owned());
                (self.styles.len() - 1) as u32
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
        s
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
        m.row = Vec::new();
        for m in &mut self.members {
            if let Some(c) = m.row.get_mut(s) {
                *c = None;
            }
        }
        self.free.push(s as u32);
    }

    fn connection(&self, receiver: usize, source: usize) -> Option<Connection> {
        self.members[receiver].row.get(source).copied().flatten()
    }

    /// Replaces the connection; returns the old one.
    fn set(&mut self, receiver: usize, source: usize, c: Option<Connection>) -> Option<Connection> {
        let row = &mut self.members[receiver].row;
        if row.len() <= source {
            if c.is_none() {
                return None;
            }
            row.resize(source + 1, None);
        }
        std::mem::replace(&mut row[source], c)
    }
}

/// What the locator bar reads of a player: its position as vanilla compares it.
#[derive(Debug, Clone, Copy)]
struct Snap {
    conn: ConnId,
    uuid: Uuid,
    pos: [f64; 3],
    block: [i32; 3],
    chunk: [i32; 2],
    /// The chunk the receiver's view is centered on, and its view distance.
    center: [i32; 2],
    view: i32,
    mode: u8,
    first_tick: bool,
}

impl Snap {
    fn of(p: &Player) -> Snap {
        let block = p.pos.map(|c| c.floor() as i32);
        Snap {
            conn: p.conn,
            uuid: p.uuid,
            pos: p.pos,
            block,
            chunk: [block[0] >> 4, block[2] >> 4],
            center: [p.center.x, p.center.z],
            view: p.view_distance,
            mode: p.game_mode,
            first_tick: p.waypoint_first_tick,
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

/// What an intact connection sends when its transmitter moved (`Connection.update`).
fn next_link(link: Link, source: &Snap, receiver: &Snap) -> Option<Link> {
    match link {
        Link::Block(last) => (source.block != last).then_some(Link::Block(source.block)),
        Link::Chunk(last) => (source.chunk != last).then_some(Link::Chunk(source.chunk)),
        Link::Azimuth(last) => {
            let now = azimuth(source, receiver);
            ((now - last).abs() > 0.008_726_646).then_some(Link::Azimuth(now))
        }
    }
}

fn packet(op: WaypointOp, uuid: Uuid, styles: &[String], c: &Connection) -> Bytes {
    let at = match c.link {
        Link::Block(p) => WaypointAt::Block(p),
        Link::Chunk(p) => WaypointAt::Chunk(p),
        Link::Azimuth(a) => WaypointAt::Azimuth(a),
    };
    hud::tracked_waypoint(op, uuid, &styles[c.style as usize], c.color, at)
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
fn step_pair(entry: &mut Option<Connection>, source: &Snap, receiver: &Snap, fresh: (u32, Option<i32>)) -> Step {
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
                    *c = Connection { link, style: fresh.0, color: fresh.1 };
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
                let c = Connection { link, style: fresh.0, color: fresh.1 };
                *entry = Some(c);
                Step::Send(WaypointOp::Track, c)
            }
            None => Step::Quiet,
        },
    }
}

/// A level member during the movers' turns: its snapshot and, as a transmitter, the style and
/// color a new connection gets and its packets shared by all receivers.
struct Info {
    snap: Snap,
    fresh: (u32, Option<i32>),
    /// Track and update of its block and of its chunk with `fresh`, and its untrack.
    shared: [OnceLock<Bytes>; 5],
}

impl Info {
    /// The packet for a step of a connection from this transmitter.
    fn bytes(&self, op: WaypointOp, c: &Connection, styles: &[String]) -> Bytes {
        let op_i = match op {
            WaypointOp::Track => 0,
            WaypointOp::Update => 1,
            WaypointOp::Untrack => return self.untrack(),
        };
        let shared = (c.style, c.color) == self.fresh
            && match c.link {
                Link::Block(b) => b == self.snap.block,
                Link::Chunk(ch) => ch == self.snap.chunk,
                Link::Azimuth(_) => false,
            };
        if !shared {
            return packet(op, self.snap.uuid, styles, c);
        }
        let i = op_i * 2 + usize::from(matches!(c.link, Link::Chunk(_)));
        self.shared[i].get_or_init(|| packet(op, self.snap.uuid, styles, c)).clone()
    }

    fn untrack(&self) -> Bytes {
        self.shared[4].get_or_init(|| untrack_packet(self.snap.uuid)).clone()
    }
}

/// A mover's turn as one receiver sees it.
#[derive(Clone, Copy)]
struct Turn {
    slot: usize,
    transmitting: bool,
}

/// One receiver's share of the movers' turns: its row (moved out of the level while the
/// receivers run) and the packets it gets.
struct Share {
    slot: usize,
    row: Mutex<Vec<Option<Connection>>>,
}

/// Runs one receiver's share: the moving transmitters' turns in connection order and, at its
/// own place among them, its own `updatePlayer` turn over every transmitter.
fn run_share(share: &Share, turns: &[Turn], transmitters: &[usize], info: &[Option<Info>], styles: &[String]) -> Vec<Bytes> {
    let r = share.slot;
    let Some(me) = info[r].as_ref() else { return Vec::new() };
    let mut row = share.row.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if row.len() < info.len() {
        row.resize(info.len(), None);
    }
    let mut out = Vec::new();
    let mut pair = |row: &mut Vec<Option<Connection>>, s: usize| {
        let Some(src) = info[s].as_ref() else { return };
        match step_pair(&mut row[s], &src.snap, &me.snap, src.fresh) {
            Step::Quiet => {}
            Step::Send(op, c) => out.push(src.bytes(op, &c, styles)),
            Step::Untrack => out.push(src.untrack()),
        }
    };
    for t in turns {
        if t.slot == r {
            // `updatePlayer`: this receiver's connection to every transmitter.
            for &w in transmitters {
                if w != r {
                    pair(&mut row, w);
                }
            }
        } else if t.transmitting {
            // `updateWaypoint` of the mover: its connection to this receiver.
            pair(&mut row, t.slot);
        }
    }
    out
}

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
        let (ss, rs) = (Snap::of(s), Snap::of(r));
        let mgr = &mut self.waypoints[dim];
        let (Some(ri), Some(si)) = (mgr.slot(receiver), mgr.slot(source)) else { return };
        match make_link(&ss, &rs) {
            Some(link) => {
                let color = s.waypoint_icon.color.or_else(|| team_color(&self.commands.scoreboard, &s.name));
                let style = mgr.style_id(&s.waypoint_icon.style);
                let c = Connection { link, style, color };
                mgr.set(ri, si, Some(c));
                let pkt = packet(WaypointOp::Track, ss.uuid, &mgr.styles, &c);
                if let Some(r) = self.players.get_mut(&receiver) {
                    r.send(pkt);
                }
            }
            None => {
                if mgr.set(ri, si, None).is_some() {
                    self.send_untrack(receiver, ss.uuid);
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
            if let Some(c) = member.row.get_mut(s)
                && c.take().is_some()
            {
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
        let mut sources: Vec<ConnId> = row
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some())
            .filter_map(|(s, _)| m.members[s].conn)
            .collect();
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
        let mut conns: Vec<ConnId> = self.players.keys().copied().collect();
        conns.sort_unstable();
        for &conn in &conns {
            let p = &self.players[&conn];
            let (dim, uuid, registered, sneaking) = (p.dim, p.uuid, p.waypoint_dim, p.sneaking);
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
        if on {
            let mut moved: [Vec<ConnId>; 3] = Default::default();
            for &conn in &conns {
                let p = &self.players[&conn];
                if !p.waypoint_first_tick && p.waypoint_last_pos != p.pos {
                    moved[p.dim].push(conn);
                }
            }
            for (dim, moved) in moved.iter().enumerate() {
                if !moved.is_empty() {
                    self.waypoint_moves(dim, moved);
                }
            }
        }
        for p in self.players.values_mut() {
            p.waypoint_last_pos = p.pos;
            p.waypoint_first_tick = false;
        }
    }

    /// The movers' turns in one level (`moved`: in connection order).
    fn waypoint_moves(&mut self, dim: DimId, moved: &[ConnId]) {
        let (players, scoreboard, pool) = (&self.players, &self.commands.scoreboard, &mut self.pool);
        let mgr = &mut self.waypoints[dim];
        // The styles new connections from each transmitter get, interned before the receivers
        // run.
        let mut fresh: Vec<(u32, Option<i32>)> = vec![(0, None); mgr.members.len()];
        for i in 0..mgr.members.len() {
            let m = &mgr.members[i];
            if let (true, Some(p)) = (m.transmitting, m.conn.and_then(|c| players.get(&c))) {
                let color = p.waypoint_icon.color.or_else(|| team_color(scoreboard, &p.name));
                fresh[i] = (mgr.style_id(&p.waypoint_icon.style), color);
            }
        }
        let info: Vec<Option<Info>> = mgr
            .members
            .iter()
            .zip(&fresh)
            .map(|(m, &fresh)| {
                let p = players.get(&m.conn?)?;
                Some(Info { snap: Snap::of(p), fresh, shared: Default::default() })
            })
            .collect();
        let turns: Vec<Turn> = moved
            .iter()
            .filter_map(|c| mgr.slot(*c))
            .map(|slot| Turn { slot, transmitting: mgr.members[slot].transmitting })
            .collect();
        let transmitters: Vec<usize> = mgr.waypoints.iter().filter_map(|c| mgr.slot(*c)).collect();
        let any_transmitting = turns.iter().any(|t| t.transmitting);
        let mut moved_slot = vec![false; mgr.members.len()];
        for t in &turns {
            moved_slot[t.slot] = true;
        }
        // Receivers that have turns to take: all of them when a transmitter moved, else the
        // moving receivers.
        let slots: Vec<usize> =
            mgr.players.iter().filter_map(|c| mgr.slot(*c)).filter(|&s| any_transmitting || moved_slot[s]).collect();
        let shares: Vec<Share> =
            slots.into_iter().map(|slot| Share { slot, row: Mutex::new(std::mem::take(&mut mgr.members[slot].row)) }).collect();
        let styles = &mgr.styles;
        let out = pool.serial(|ctx| {
            ctx.map_indexed_with(SHARE_WINDOW, &shares, |_, share| run_share(share, &turns, &transmitters, &info, styles))
        });
        for (share, packets) in shares.into_iter().zip(out) {
            let m = &mut mgr.members[share.slot];
            m.row = share.row.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !packets.is_empty()
                && let Some(p) = m.conn.and_then(|c| self.players.get_mut(&c))
            {
                p.outbox.extend(packets);
            }
        }
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
                for (s, c) in row.iter().enumerate() {
                    if let (Some(_), Some(rc), Some(sc)) = (c, m.members[r].conn, m.members[s].conn) {
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
}
