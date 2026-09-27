//! Non-player entities: storage that follows region merges and splits, spawning with network
//! ids that do not depend on how regions split the world, ticking inside their region, and
//! tracking to the region's players (design §6).
//!
//! The behaviour here is a stand-in (gravity, drag, block collision, item age and pickup)
//! until the vanilla-exact entity crate replaces [`tick`].

use crate::Player;
use bytes::Bytes;
use kiln_data::entities::{EntityType, data};
use kiln_link::ConnId;
use kiln_proto::packets::entity::{self, DataValue, EntityData, MoveState, MovementTracker};
use kiln_region::{CellPos, RegionPart};
use kiln_world::{Blocks, ChunkPos};
use smallvec::SmallVec;
use uuid::Uuid;

/// Ticks an item waits before it can be picked up after a player drops it.
pub(crate) const DROP_PICKUP_DELAY: i32 = 40;
/// Items despawn after five minutes.
const ITEM_LIFETIME: i32 = 6000;
const GRAVITY: f64 = 0.04;

pub(crate) enum Body {
    Item { stack: kiln_item::ItemStack, pickup_delay: i32 },
}

pub(crate) struct Entity {
    pub id: i32,
    pub uuid: Uuid,
    pub kind: &'static EntityType,
    /// The owned cell the entity is routed by (where it last was inside its region).
    pub cell: CellPos,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub rot: [f32; 2],
    pub on_ground: bool,
    pub age: i32,
    pub removed: bool,
    pub body: Body,
    tracker: MovementTracker,
    /// Velocity the viewers last got.
    sent_vel: [f64; 3],
    /// Players tracking this entity (sorted).
    pub seen_by: Vec<ConnId>,
    /// Section at the last tracking update; `None` forces a re-evaluation.
    section: Option<[i32; 3]>,
}

/// A spawn requested during a phase; ids are handed out afterwards in canonical order.
pub(crate) struct Spawn {
    pub kind: &'static EntityType,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub body: Body,
}

impl Spawn {
    /// Order of spawns from different regions: by position, then type (never by region).
    fn key(&self) -> ([u64; 3], i32) {
        (self.pos.map(f64::to_bits), self.kind.id)
    }
}

/// Puts spawns in the order ids are assigned in.
pub(crate) fn canonical(mut spawns: Vec<Spawn>) -> Vec<Spawn> {
    spawns.sort_by_key(Spawn::key);
    spawns
}

impl Entity {
    pub fn new(id: i32, spawn: Spawn) -> Self {
        let state = MoveState { pos: spawn.pos, yaw: 0.0, pitch: 0.0, head_yaw: 0.0, on_ground: false };
        Self {
            id,
            uuid: Uuid::from_u64_pair(0x6b69_6c6e_656e_7469, id as u64),
            kind: spawn.kind,
            cell: chunk_of(spawn.pos).cell(),
            pos: spawn.pos,
            vel: spawn.vel,
            rot: [0.0; 2],
            on_ground: false,
            age: 0,
            removed: false,
            body: spawn.body,
            tracker: MovementTracker::new(id, spawn.kind.update_interval, &state),
            sent_vel: spawn.vel,
            seen_by: Vec::new(),
            section: None,
        }
    }

    fn move_state(&self) -> MoveState {
        MoveState { pos: self.pos, yaw: self.rot[0], pitch: self.rot[1], head_yaw: self.rot[0], on_ground: self.on_ground }
    }

    fn metadata(&self) -> EntityData {
        let mut d = EntityData::new();
        match &self.body {
            Body::Item { stack, .. } => {
                let mut bytes = bytes::BytesMut::new();
                stack.write_optional(&mut bytes);
                d.set(data::item_entity::ITEM, &DataValue::EncodedItemStack(bytes.freeze()));
            }
        }
        d
    }

    /// Bundle that makes a new viewer see the entity.
    fn spawn_packets(&self) -> [Bytes; 4] {
        [
            entity::bundle_delimiter(),
            self.tracker.spawn(self.uuid, self.kind.id, self.vel, 0),
            entity::set_entity_data(self.id, &self.metadata()),
            entity::bundle_delimiter(),
        ]
    }

    fn half_width(&self) -> f64 {
        self.kind.width as f64 / 2.0
    }
}

/// A region's entities in id order, which is spawn order (vanilla's tick order is insertion
/// order); merges interleave by id, splits go by each entity's cell.
#[derive(Default)]
pub(crate) struct Entities {
    pub list: Vec<Entity>,
}

impl RegionPart for Entities {
    fn merge(into: &mut Self, from: Self) {
        let a = std::mem::take(&mut into.list);
        let mut out = Vec::with_capacity(a.len() + from.list.len());
        let (mut a, mut b) = (a.into_iter().peekable(), from.list.into_iter().peekable());
        loop {
            let take_a = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.id < y.id,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            out.push(if take_a { a.next() } else { b.next() }.unwrap());
        }
        into.list = out;
    }

    fn split(self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        let mut parts: SmallVec<[Self; 4]> = (0..n).map(|_| Self::default()).collect();
        for e in self.list {
            parts[owner_of(e.cell)].list.push(e);
        }
        parts
    }

    fn count(&self) -> usize {
        self.list.len()
    }

    fn for_each_cell(&self, f: &mut dyn FnMut(CellPos)) {
        self.list.iter().for_each(|e| f(e.cell));
    }
}

pub(crate) fn chunk_of(pos: [f64; 3]) -> ChunkPos {
    ChunkPos::of_block(pos[0].floor() as i32, pos[2].floor() as i32)
}

/// L8: ticks the region's entities in order. Entities in chunks the region does not have
/// loaded stay put (vanilla does not tick entities in unloaded chunks).
pub(crate) fn tick<W: Blocks + ?Sized>(entities: &mut Entities, world: &W) {
    for e in &mut entities.list {
        if world.chunk(chunk_of(e.pos)).is_none() {
            continue;
        }
        e.age += 1;
        match &mut e.body {
            Body::Item { pickup_delay, .. } => {
                if *pickup_delay > 0 && *pickup_delay != 32767 {
                    *pickup_delay -= 1;
                }
                if e.age >= ITEM_LIFETIME {
                    e.removed = true;
                    continue;
                }
            }
        }
        e.vel[1] -= GRAVITY;
        let moved = collide(world, e.pos, e.half_width(), e.kind.height as f64, e.vel);
        let hit_ground = moved[1] != e.vel[1] && e.vel[1] < 0.0;
        for i in 0..3 {
            if moved[i] != e.vel[i] {
                e.vel[i] = 0.0;
            }
            e.pos[i] += moved[i];
        }
        e.on_ground = hit_ground;
        let friction = if e.on_ground { 0.6 * 0.98 } else { 0.98 };
        e.vel = [e.vel[0] * friction, e.vel[1] * 0.98, e.vel[2] * friction];
        let cell = chunk_of(e.pos).cell();
        if world.cell(cell).is_some() {
            e.cell = cell;
        }
    }
}

/// Movement along y, then x and z, stopped by block collision boxes.
fn collide<W: Blocks + ?Sized>(world: &W, pos: [f64; 3], r: f64, h: f64, delta: [f64; 3]) -> [f64; 3] {
    let mut p = pos;
    let mut out = [0.0; 3];
    for axis in [1, 0, 2] {
        let mut d = delta[axis];
        if d == 0.0 {
            continue;
        }
        let (lo, hi) = (|p: [f64; 3]| [p[0] - r, p[1], p[2] - r], |p: [f64; 3]| [p[0] + r, p[1] + h, p[2] + r]);
        let (min, max) = (lo(p), hi(p));
        // Blocks the box sweeps through.
        let (mut from, mut to) = (min.map(|v| v.floor() as i32 - 1), max.map(|v| v.floor() as i32 + 1));
        if d < 0.0 {
            from[axis] = (min[axis] + d).floor() as i32 - 1;
        } else {
            to[axis] = (max[axis] + d).floor() as i32 + 1;
        }
        for x in from[0]..=to[0] {
            for y in from[1]..=to[1] {
                for z in from[2]..=to[2] {
                    let Some(state) = world.get_block(x, y, z) else { continue };
                    for b in kiln_data::block_props::collision(state) {
                        let bmin = [x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64];
                        let bmax = [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64];
                        let overlaps = (0..3).filter(|&i| i != axis).all(|i| min[i] < bmax[i] && max[i] > bmin[i]);
                        if !overlaps {
                            continue;
                        }
                        if d > 0.0 && max[axis] <= bmin[axis] + 1e-7 {
                            d = d.min(bmin[axis] - max[axis]);
                        } else if d < 0.0 && min[axis] >= bmax[axis] - 1e-7 {
                            d = d.max(bmax[axis] - min[axis]);
                        }
                    }
                }
            }
        }
        out[axis] = d;
        p[axis] += d;
    }
    out
}

/// Players touching items that can be picked up take them into their inventory. `players`
/// is the region's, sorted by connection.
pub(crate) fn pickups(entities: &mut Entities, players: &mut [&mut Player]) {
    for e in &mut entities.list {
        let Body::Item { stack, pickup_delay } = &mut e.body;
        if e.removed || *pickup_delay > 0 {
            continue;
        }
        let r = e.kind.width as f64 / 2.0;
        let lo = [e.pos[0] - r, e.pos[1], e.pos[2] - r];
        let hi = [e.pos[0] + r, e.pos[1] + e.kind.height as f64, e.pos[2] + r];
        // The player's box inflated by (1, 0.5, 1), as in vanilla's player `aiStep`.
        let touching = |p: &Player| {
            let pmin = [p.pos[0] - 1.3, p.pos[1] - 0.5, p.pos[2] - 1.3];
            let pmax = [p.pos[0] + 1.3, p.pos[1] + 2.3, p.pos[2] + 1.3];
            (0..3).all(|i| pmin[i] < hi[i] && pmax[i] > lo[i])
        };
        let Some(i) = players.iter().position(|p| !p.disconnected && p.game_mode != 3 && touching(p)) else {
            continue;
        };
        let taken = players[i].add_to_inventory(stack);
        if taken == 0 {
            continue;
        }
        let pkt = entity::take_item_entity(e.id, players[i].entity_id, taken);
        players[i].send(pkt.clone());
        for v in &e.seen_by {
            if let Ok(j) = players.binary_search_by_key(v, |q| q.conn)
                && j != i
            {
                players[j].send(pkt.clone());
            }
        }
        if stack.is_empty() {
            e.removed = true;
        }
    }
}

/// Tracks the region's entities to its players (sorted by connection) with vanilla's
/// triggers: an entity whose section changed is re-evaluated against every player, and every
/// entity against the players in `movers` (whose section changed). Then sends movement and
/// velocity, and removes dead entities from their viewers and the region.
pub(crate) fn track(entities: &mut Entities, players: &mut [&mut Player], movers: &[ConnId]) {
    let sees = |p: &Player, e: &Entity| {
        let range = (e.kind.tracking_range as f64 * 16.0).min(p.view_distance as f64 * 16.0);
        let (dx, dz) = (p.pos[0] - e.pos[0], p.pos[2] - e.pos[2]);
        let (pc, ec) = (chunk_of(p.pos), chunk_of(e.pos));
        !e.removed
            && dx * dx + dz * dz <= range * range
            && (pc.x - ec.x).abs() <= p.view_distance
            && (pc.z - ec.z).abs() <= p.view_distance
    };
    let index = |players: &[&mut Player], conn: ConnId| players.binary_search_by_key(&conn, |p| p.conn).ok();
    for e in &mut entities.list {
        let section = e.pos.map(|c| c.floor() as i32 >> 4);
        let moved = e.section != Some(section);
        e.section = Some(section);
        let (mut added, mut removed) = (Vec::new(), Vec::new());
        let mut check = |p: &Player, seen: bool| match (sees(p, e), seen) {
            (true, false) => added.push(p.conn),
            (false, true) => removed.push(p.conn),
            _ => {}
        };
        if moved || e.removed {
            for p in players.iter() {
                check(p, e.seen_by.binary_search(&p.conn).is_ok());
            }
        } else {
            for &m in movers {
                if let Some(i) = index(players, m) {
                    check(players[i], e.seen_by.binary_search(&m).is_ok());
                }
            }
        }
        if !added.is_empty() {
            let spawn = e.spawn_packets();
            for &c in &added {
                if let Some(i) = index(players, c) {
                    spawn.iter().for_each(|pkt| players[i].send(pkt.clone()));
                }
            }
        }
        if !removed.is_empty() {
            let despawn = entity::remove_entities(&[e.id]);
            for &c in &removed {
                if let Some(i) = index(players, c) {
                    players[i].send(despawn.clone());
                }
            }
        }
        e.seen_by.retain(|v| removed.binary_search(v).is_err());
        e.seen_by.extend(added);
        e.seen_by.sort_unstable();
        // Players that left the region or the game are no longer viewers.
        e.seen_by.retain(|&v| index(players, v).is_some());

        if e.removed {
            continue;
        }
        let mut packets = e.tracker.tick(&e.move_state());
        // `ServerEntity.sendChanges`: velocity on update ticks when it changed.
        if e.kind.track_deltas && e.age % e.kind.update_interval.max(1) == 0 {
            let d: f64 = (0..3).map(|i| (e.vel[i] - e.sent_vel[i]).powi(2)).sum();
            let still = e.vel.iter().all(|&v| v == 0.0);
            if d > 1.0e-7 || (d > 0.0 && still) {
                e.sent_vel = e.vel;
                packets.push(entity::set_entity_motion(e.id, e.vel));
            }
        }
        for v in &e.seen_by {
            if let Some(i) = index(players, *v) {
                packets.iter().for_each(|pkt| players[i].send(pkt.clone()));
            }
        }
    }
    entities.list.retain(|e| !e.removed);
}
