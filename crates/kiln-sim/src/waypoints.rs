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
//! A crowd makes this the quadratic part of the tick (every mover meets every other player
//! twice), so the per-pair work is kept to arithmetic on small copies: connections are small
//! `Copy` values in per-receiver maps with a cheap hash, positions come from one snapshot of
//! the players, the game rule is read once, and a transmitter's packet is encoded once for all
//! the receivers that get the same bytes. Every receiver's packets keep the order a serial
//! loop over the movers produces.

use crate::{DimId, Player, Sim};
use bytes::Bytes;
use kiln_command::scoreboard::Scoreboard;
use kiln_command::selector::SelectorTarget;
use kiln_link::ConnId;
use kiln_proto::packets::hud::{self, WaypointAt, WaypointOp};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
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

/// A multiplicative hash for connection ids (sequential numbers: the default hasher's
/// protection against crafted keys costs more than the lookups it guards here).
#[derive(Default)]
pub(crate) struct ConnHasher(u64);

impl Hasher for ConnHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type Fast = BuildHasherDefault<ConnHasher>;

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

/// One level's `ServerWaypointManager`.
#[derive(Debug, Default)]
pub(crate) struct WaypointManager {
    /// Transmitters, in the order they started.
    waypoints: Vec<ConnId>,
    transmitting: HashSet<ConnId, Fast>,
    /// Receivers.
    players: Vec<ConnId>,
    receiving: HashSet<ConnId, Fast>,
    /// receiver → (transmitter → connection).
    links: HashMap<ConnId, HashMap<ConnId, Connection, Fast>, Fast>,
    /// The styles connections were made with.
    styles: Vec<String>,
    /// Buckets of vanilla's `HashSet` of transmitters (16, doubling past a load of 0.75,
    /// never shrinking), for its iteration order.
    capacity: usize,
}

impl WaypointManager {
    fn style_id(&mut self, style: &str) -> u32 {
        intern(&mut self.styles, style)
    }
}

fn intern(styles: &mut Vec<String>, style: &str) -> u32 {
    match styles.iter().position(|s| s == style) {
        Some(i) => i as u32,
        None => {
            styles.push(style.to_owned());
            (styles.len() - 1) as u32
        }
    }
}

use kiln_command::vanilla::misc::TEAM_RGB;

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
        Link::Azimuth(_) => ignores(source, receiver) || chunk_visible(source.chunk, receiver) || distance(source, receiver) <= REALLY_FAR,
    }
}

/// What an intact connection sends when its transmitter moved (`Connection.update`).
fn next_link(link: Link, source: &Snap, receiver: &Snap) -> Option<Link> {
    match link {
        Link::Block(last) => {
            let now = source.block;
            ((0..3).map(|i| (now[i] - last[i]).abs()).sum::<i32>() > 0).then_some(Link::Block(now))
        }
        Link::Chunk(last) => {
            let now = source.chunk;
            ((now[0] - last[0]).abs().max((now[1] - last[1]).abs()) > 0).then_some(Link::Chunk(now))
        }
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

/// The style and color a connection made now from `source` gets.
fn fresh(players: &HashMap<ConnId, Player>, scoreboard: &Scoreboard, styles: &mut Vec<String>, source: ConnId) -> (u32, Option<i32>) {
    match players.get(&source) {
        Some(p) => (intern(styles, &p.waypoint_icon.style), p.waypoint_icon.color.or_else(|| team_color(scoreboard, &p.name))),
        None => (intern(styles, DEFAULT_STYLE), None),
    }
}

/// What one transmitter-receiver pair does at the transmitter's or the receiver's move.
enum Step {
    Quiet,
    Send(WaypointOp, Connection),
    Untrack,
}

/// `createConnection` for a pair without a connection, `updateConnection` for one with: an
/// intact connection sends what changed, a broken one is remade, and a pair that cannot have
/// a connection (the transmitter's first tick, a spectator) has none.
fn step_pair(map: &mut HashMap<ConnId, Connection, Fast>, source: &Snap, receiver: &Snap, look: impl FnOnce(ConnId) -> (u32, Option<i32>)) -> Step {
    match map.get_mut(&source.conn) {
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
                    let (style, color) = look(source.conn);
                    *c = Connection { link, style, color };
                    Step::Send(WaypointOp::Track, *c)
                }
                None => {
                    map.remove(&source.conn);
                    Step::Untrack
                }
            }
        }
        None => match make_link(source, receiver) {
            Some(link) => {
                let (style, color) = look(source.conn);
                let c = Connection { link, style, color };
                map.insert(source.conn, c);
                Step::Send(WaypointOp::Track, c)
            }
            None => Step::Quiet,
        },
    }
}

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
        match make_link(&ss, &rs) {
            Some(link) => {
                let color = s.waypoint_icon.color.or_else(|| team_color(&self.commands.scoreboard, &s.name));
                let style = mgr.style_id(&s.waypoint_icon.style);
                let c = Connection { link, style, color };
                mgr.links.entry(receiver).or_default().insert(source, c);
                let pkt = packet(WaypointOp::Track, ss.uuid, &mgr.styles, &c);
                if let Some(r) = self.players.get_mut(&receiver) {
                    r.send(pkt);
                }
            }
            None => {
                if mgr.links.get_mut(&receiver).is_some_and(|m| m.remove(&source).is_some()) {
                    self.send_untrack(receiver, ss.uuid);
                }
            }
        }
    }

    /// `trackWaypoint`.
    pub(crate) fn track_waypoint(&mut self, dim: DimId, source: ConnId) {
        let on = self.locator_bar();
        let m = &mut self.waypoints[dim];
        if m.transmitting.insert(source) {
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
        let mut receivers: Vec<ConnId> = m.links.iter_mut().filter_map(|(r, links)| links.remove(&source).map(|_| *r)).collect();
        receivers.sort_unstable();
        for r in receivers {
            self.send_untrack(r, source_uuid);
        }
        let m = &mut self.waypoints[dim];
        m.waypoints.retain(|w| *w != source);
        m.transmitting.remove(&source);
    }

    /// `addPlayer` when a player enters a level: it receives the level's waypoints and
    /// transmits its own (unless crouching).
    pub(crate) fn waypoints_add_player(&mut self, dim: DimId, conn: ConnId) {
        let on = self.locator_bar();
        let m = &mut self.waypoints[dim];
        if m.receiving.insert(conn) {
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
        let mut sources: Vec<ConnId> = m.links.remove(&conn).map(|links| links.into_keys().collect()).unwrap_or_default();
        sources.sort_unstable();
        for s in sources {
            if let Some(src) = self.players.get(&s).map(|p| p.uuid) {
                self.send_untrack(conn, src);
            }
        }
        self.untrack_waypoint(dim, conn, uuid);
        let m = &mut self.waypoints[dim];
        m.players.retain(|p| *p != conn);
        m.receiving.remove(&conn);
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
            let transmitting = self.waypoints[dim].transmitting.contains(&conn);
            if sneaking && transmitting {
                self.untrack_waypoint(dim, conn, uuid);
            } else if !sneaking && !transmitting {
                self.track_waypoint(dim, conn);
            }
        }
        let t0 = std::time::Instant::now();
        let mut moved: Vec<(DimId, ConnId)> =
            self.players.values().filter(|p| !p.waypoint_first_tick && p.waypoint_last_pos != p.pos).map(|p| (p.dim, p.conn)).collect();
        moved.sort_unstable_by_key(|(_, c)| *c);
        let t1 = std::time::Instant::now();
        if on && !moved.is_empty() {
            self.waypoint_moves(&moved, &conns);
        }
        if std::env::var_os("WP_DBG").is_some() { eprintln!("moved {} reg {:?} moves {:?}", moved.len(), t1 - t0, t1.elapsed()); }
        for p in self.players.values_mut() {
            p.waypoint_last_pos = p.pos;
            p.waypoint_first_tick = false;
        }
    }

    /// The movers' turns, in connection order. A pair's connection depends only on the two
    /// players' positions and its own earlier state, and a receiver's packets come in the
    /// order of the movers' turns, so the packets are collected per receiver and handed over
    /// at the end.
    fn waypoint_moves(&mut self, moved: &[(DimId, ConnId)], conns: &[ConnId]) {
        let snaps: Vec<Snap> = conns.iter().map(|c| Snap::of(&self.players[c])).collect();
        let at: HashMap<ConnId, u32, Fast> = conns.iter().enumerate().map(|(i, &c)| (c, i as u32)).collect();
        // Each level's receivers and transmitters as snapshot indices (`u32::MAX`: not in the game).
        let mut indices: [Option<(Vec<u32>, Vec<u32>)>; 3] = Default::default();
        let idx = |list: &[ConnId]| -> Vec<u32> { list.iter().map(|c| at.get(c).copied().unwrap_or(u32::MAX)).collect() };
        let mut out: Vec<Vec<Bytes>> = vec![Vec::new(); snaps.len()];
        let (players, scoreboard) = (&self.players, &self.commands.scoreboard);
        for &(dim, conn) in moved {
            let Some(&si) = at.get(&conn) else { continue };
            let si = si as usize;
            let WaypointManager { waypoints, transmitting, players: receivers, receiving, links, styles, .. } = &mut self.waypoints[dim];
            let (receiver_at, transmitter_at) = indices[dim].get_or_insert_with(|| (idx(receivers), idx(waypoints)));
            let me = &snaps[si];
            if transmitting.contains(&conn) {
                // `updateWaypoint`: the transmitter's connection to every receiver.
                let look = fresh(players, scoreboard, styles, conn);
                let mut cache: Option<(WaypointOp, Connection, Bytes)> = None;
                let mut untrack: Option<Bytes> = None;
                for (&r, &ri) in receivers.iter().zip(receiver_at.iter()) {
                    if r == conn || ri == u32::MAX {
                        continue;
                    }
                    let map = links.entry(r).or_default();
                    match step_pair(map, me, &snaps[ri as usize], |_| look) {
                        Step::Quiet => {}
                        Step::Send(op, c) => {
                            let bytes = match &cache {
                                Some((o, cc, b)) if *o == op && *cc == c => b.clone(),
                                _ => {
                                    let b = packet(op, me.uuid, styles, &c);
                                    cache = Some((op, c, b.clone()));
                                    b
                                }
                            };
                            out[ri as usize].push(bytes);
                        }
                        Step::Untrack => out[ri as usize].push(untrack.get_or_insert_with(|| untrack_packet(me.uuid)).clone()),
                    }
                }
            }
            if receiving.contains(&conn) {
                // `updatePlayer`: the receiver's connection to every transmitter.
                let map = links.entry(conn).or_default();
                for (&w, &wi) in waypoints.iter().zip(transmitter_at.iter()) {
                    if w == conn || wi == u32::MAX {
                        continue;
                    }
                    let source = &snaps[wi as usize];
                    let step = step_pair(map, source, me, |s| fresh(players, scoreboard, styles, s));
                    match step {
                        Step::Quiet => {}
                        Step::Send(op, c) => out[si].push(packet(op, source.uuid, styles, &c)),
                        Step::Untrack => out[si].push(untrack_packet(source.uuid)),
                    }
                }
            }
        }
        for (conn, packets) in conns.iter().zip(out) {
            if !packets.is_empty()
                && let Some(p) = self.players.get_mut(conn)
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
            let mut broken: Vec<(ConnId, ConnId)> =
                self.waypoints[dim].links.iter().flat_map(|(r, m)| m.keys().map(move |s| (*r, *s))).collect();
            broken.sort_unstable();
            self.waypoints[dim].links.clear();
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
