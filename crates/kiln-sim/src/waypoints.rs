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

use crate::{DimId, Player, Sim};
use kiln_command::selector::SelectorTarget;
use kiln_link::ConnId;
use kiln_proto::packets::hud::{self, WaypointAt, WaypointOp};
use std::collections::HashMap;

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

#[derive(Debug, Clone)]
struct Connection {
    link: Link,
    /// `Icon.cloneAndAssignStyle` when the connection was made.
    style: String,
    color: Option<i32>,
}

/// One level's `ServerWaypointManager`.
#[derive(Debug, Default)]
pub(crate) struct WaypointManager {
    /// Transmitters, in the order they started.
    waypoints: Vec<ConnId>,
    /// Receivers.
    players: Vec<ConnId>,
    /// (receiver, transmitter) → connection.
    connections: HashMap<(ConnId, ConnId), Connection>,
    /// Buckets of vanilla's `HashSet` of transmitters (16, doubling past a load of 0.75,
    /// never shrinking), for its iteration order.
    capacity: usize,
}

use kiln_command::vanilla::misc::TEAM_RGB;

fn block_of(p: &Player) -> [i32; 3] {
    p.pos.map(|c| c.floor() as i32)
}

fn chunk_of(p: &Player) -> [i32; 2] {
    [(p.pos[0].floor() as i32) >> 4, (p.pos[2].floor() as i32) >> 4]
}

/// `Entity.distanceTo` (single precision, as vanilla compares it).
fn distance(a: &Player, b: &Player) -> f32 {
    let d: f64 = (0..3).map(|i| (a.pos[i] - b.pos[i]).powi(2)).sum();
    (d as f32).sqrt()
}

/// `ChunkTrackingView.isInViewDistance` of the receiver.
fn chunk_visible(chunk: [i32; 2], receiver: &Player) -> bool {
    let dx = i64::from(((chunk[0] - receiver.center.x).abs() - 1).max(0));
    let dz = i64::from(((chunk[1] - receiver.center.z).abs() - 1).max(0));
    dx * dx + dz * dz < i64::from(receiver.view_distance) * i64::from(receiver.view_distance)
}

/// `WaypointTransmitter.doesSourceIgnoreReceiver`: spectators transmit to spectators only.
fn ignores(source: &Player, receiver: &Player) -> bool {
    receiver.game_mode != 3 && source.game_mode == 3
}

/// `EntityAzimuthConnection`'s angle: `atan2` of the receiver-to-source offset turned 90°.
fn azimuth(source: &Player, receiver: &Player) -> f32 {
    let (dx, dz) = (receiver.pos[0] - source.pos[0], receiver.pos[2] - source.pos[2]);
    // `Vec3.rotateClockwise90`: (x, y, z) -> (-z, y, x).
    kiln_command::coords::mth_atan2(dx, -dz) as f32
}

/// `LivingEntity.makeWaypointConnectionWith`.
fn make_connection(source: &Player, receiver: &Player, team_color: Option<i32>) -> Option<Connection> {
    if source.waypoint_first_tick || source.conn == receiver.conn || ignores(source, receiver) {
        return None;
    }
    let link = if distance(source, receiver) > REALLY_FAR {
        Link::Azimuth(azimuth(source, receiver))
    } else if !chunk_visible(chunk_of(source), receiver) {
        Link::Chunk(chunk_of(source))
    } else {
        Link::Block(block_of(source))
    };
    let color = source.waypoint_icon.color.or(team_color);
    Some(Connection { link, style: source.waypoint_icon.style.clone(), color })
}

/// `Connection.isBroken`.
fn is_broken(c: &Connection, source: &Player, receiver: &Player) -> bool {
    match c.link {
        Link::Block(last) => {
            let now = block_of(source);
            (0..3).map(|i| (now[i] - last[i]).abs()).sum::<i32>() > 1 || ignores(source, receiver)
        }
        Link::Chunk(last) => {
            let now = chunk_of(source);
            (now[0] - last[0]).abs().max((now[1] - last[1]).abs()) > 1
                || ignores(source, receiver)
                || chunk_visible(last, receiver)
        }
        Link::Azimuth(_) => {
            ignores(source, receiver) || chunk_visible(chunk_of(source), receiver) || distance(source, receiver) <= REALLY_FAR
        }
    }
}

fn packet(op: WaypointOp, source: &Player, c: &Connection) -> bytes::Bytes {
    let at = match c.link {
        Link::Block(p) => WaypointAt::Block(p),
        Link::Chunk(p) => WaypointAt::Chunk(p),
        Link::Azimuth(a) => WaypointAt::Azimuth(a),
    };
    hud::tracked_waypoint(op, source.uuid, &c.style, c.color, at)
}

impl Sim {
    /// `Waypoint.Icon.cloneAndAssignStyle`'s team color: the transmitter's team color (black
    /// drawn as dark gray).
    fn waypoint_team_color(&self, p: &Player) -> Option<i32> {
        let team = self.commands.scoreboard.team_of(&p.name)?;
        team.color.map(|i| if i == 0 { -13_619_152 } else { TEAM_RGB[i] })
    }

    fn locator_bar(&self) -> bool {
        self.rule_bool("minecraft:locator_bar")
    }

    /// Sends `op` for the connection to `receiver`.
    fn send_waypoint(&mut self, receiver: ConnId, source: ConnId, op: WaypointOp, c: &Connection) {
        let Some(s) = self.players.get(&source) else { return };
        let pkt = packet(op, s, c);
        if let Some(r) = self.players.get_mut(&receiver) {
            r.send(pkt);
        }
    }

    /// Untrack packet for a connection whose transmitter may be gone.
    fn send_untrack(&mut self, receiver: ConnId, source_uuid: uuid::Uuid) {
        if let Some(r) = self.players.get_mut(&receiver) {
            r.send(hud::tracked_waypoint(WaypointOp::Untrack, source_uuid, DEFAULT_STYLE, None, WaypointAt::Empty));
        }
    }

    /// `createConnection`: a fresh connection replaces any old one.
    fn create_waypoint_connection(&mut self, dim: DimId, receiver: ConnId, source: ConnId) {
        if receiver == source || !self.locator_bar() {
            return;
        }
        let (Some(s), Some(r)) = (self.players.get(&source), self.players.get(&receiver)) else { return };
        let made = make_connection(s, r, self.waypoint_team_color(s));
        let source_uuid = s.uuid;
        match made {
            Some(c) => {
                self.waypoints[dim].connections.insert((receiver, source), c.clone());
                self.send_waypoint(receiver, source, WaypointOp::Track, &c);
            }
            None => {
                if self.waypoints[dim].connections.remove(&(receiver, source)).is_some() {
                    self.send_untrack(receiver, source_uuid);
                }
            }
        }
    }

    /// `updateConnection`: an intact connection sends what changed; a broken one is remade.
    fn update_waypoint_connection(&mut self, dim: DimId, receiver: ConnId, source: ConnId) {
        if receiver == source || !self.locator_bar() {
            return;
        }
        let Some(c) = self.waypoints[dim].connections.get(&(receiver, source)).cloned() else { return };
        let (Some(s), Some(r)) = (self.players.get(&source), self.players.get(&receiver)) else { return };
        if !is_broken(&c, s, r) {
            let next = match c.link {
                Link::Block(last) => {
                    let now = block_of(s);
                    ((0..3).map(|i| (now[i] - last[i]).abs()).sum::<i32>() > 0).then_some(Link::Block(now))
                }
                Link::Chunk(last) => {
                    let now = chunk_of(s);
                    ((now[0] - last[0]).abs().max((now[1] - last[1]).abs()) > 0).then_some(Link::Chunk(now))
                }
                Link::Azimuth(last) => {
                    let now = azimuth(s, r);
                    ((now - last).abs() > 0.008_726_646).then_some(Link::Azimuth(now))
                }
            };
            if let Some(link) = next {
                let updated = Connection { link, ..c };
                self.send_waypoint(receiver, source, WaypointOp::Update, &updated);
                self.waypoints[dim].connections.insert((receiver, source), updated);
            }
            return;
        }
        let made = make_connection(s, r, self.waypoint_team_color(s));
        let source_uuid = s.uuid;
        match made {
            Some(new) => {
                self.waypoints[dim].connections.insert((receiver, source), new.clone());
                self.send_waypoint(receiver, source, WaypointOp::Track, &new);
            }
            None => {
                self.waypoints[dim].connections.remove(&(receiver, source));
                self.send_untrack(receiver, source_uuid);
            }
        }
    }

    /// `trackWaypoint`.
    pub(crate) fn track_waypoint(&mut self, dim: DimId, source: ConnId) {
        let m = &mut self.waypoints[dim];
        if !m.waypoints.contains(&source) {
            m.waypoints.push(source);
            m.capacity = m.capacity.max(16);
            if m.waypoints.len() > m.capacity * 3 / 4 {
                m.capacity *= 2;
            }
        }
        for r in self.waypoints[dim].players.clone() {
            self.create_waypoint_connection(dim, r, source);
        }
    }

    /// `untrackWaypoint`.
    pub(crate) fn untrack_waypoint(&mut self, dim: DimId, source: ConnId, source_uuid: uuid::Uuid) {
        let receivers: Vec<ConnId> =
            self.waypoints[dim].connections.keys().filter(|(_, s)| *s == source).map(|(r, _)| *r).collect();
        for r in receivers {
            self.waypoints[dim].connections.remove(&(r, source));
            self.send_untrack(r, source_uuid);
        }
        self.waypoints[dim].waypoints.retain(|w| *w != source);
    }

    /// `addPlayer` when a player enters a level: it receives the level's waypoints and
    /// transmits its own (unless crouching).
    pub(crate) fn waypoints_add_player(&mut self, dim: DimId, conn: ConnId) {
        if !self.waypoints[dim].players.contains(&conn) {
            self.waypoints[dim].players.push(conn);
        }
        for w in self.waypoints[dim].waypoints.clone() {
            self.create_waypoint_connection(dim, conn, w);
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
    pub(crate) fn waypoints_remove_player(&mut self, dim: DimId, conn: ConnId, uuid: uuid::Uuid) {
        let sources: Vec<ConnId> =
            self.waypoints[dim].connections.keys().filter(|(r, _)| *r == conn).map(|(_, s)| *s).collect();
        for s in sources {
            if let Some(src) = self.players.get(&s).map(|p| p.uuid) {
                self.send_untrack(conn, src);
            }
            self.waypoints[dim].connections.remove(&(conn, s));
        }
        self.untrack_waypoint(dim, conn, uuid);
        self.waypoints[dim].players.retain(|p| *p != conn);
    }

    /// Once a tick: `updateWaypoint` and `updatePlayer` for the players that moved, then the
    /// joined players' first tick ends.
    pub(crate) fn tick_waypoints(&mut self) {
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
            let transmitting = self.waypoints[dim].waypoints.contains(&conn);
            if sneaking && transmitting {
                self.untrack_waypoint(dim, conn, uuid);
            } else if !sneaking && !transmitting {
                self.track_waypoint(dim, conn);
            }
        }
        let moved: Vec<(DimId, ConnId)> = self
            .players
            .values()
            .filter(|p| !p.waypoint_first_tick && p.waypoint_last_pos != p.pos)
            .map(|p| (p.dim, p.conn))
            .collect();
        let mut moved = moved;
        moved.sort_unstable_by_key(|(_, c)| *c);
        for (dim, conn) in moved {
            if self.waypoints[dim].waypoints.contains(&conn) {
                // `updateWaypoint`.
                for r in self.waypoints[dim].players.clone() {
                    if self.waypoints[dim].connections.contains_key(&(r, conn)) {
                        self.update_waypoint_connection(dim, r, conn);
                    } else {
                        self.create_waypoint_connection(dim, r, conn);
                    }
                }
            }
            if self.waypoints[dim].players.contains(&conn) {
                // `updatePlayer`.
                for w in self.waypoints[dim].waypoints.clone() {
                    if self.waypoints[dim].connections.contains_key(&(conn, w)) {
                        self.update_waypoint_connection(dim, conn, w);
                    } else {
                        self.create_waypoint_connection(dim, conn, w);
                    }
                }
            }
        }
        for p in self.players.values_mut() {
            p.waypoint_last_pos = p.pos;
            p.waypoint_first_tick = false;
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
        for dim in 0..self.waypoints.len() {
            let keys: Vec<(ConnId, ConnId)> = self.waypoints[dim].connections.keys().copied().collect();
            for (r, s) in keys {
                self.waypoints[dim].connections.remove(&(r, s));
                if let Some(uuid) = self.players.get(&s).map(|p| p.uuid) {
                    self.send_untrack(r, uuid);
                }
            }
            if self.locator_bar() {
                for w in self.waypoints[dim].waypoints.clone() {
                    for r in self.waypoints[dim].players.clone() {
                        self.create_waypoint_connection(dim, r, w);
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
