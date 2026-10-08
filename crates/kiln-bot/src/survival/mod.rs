//! Survival bots: players that explore, dig, build and wire redstone like people do.
//!
//! An [`Agent`] is one bot's player. It keeps the blocks around it ([`World`]), a body with
//! vanilla movement ([`Body`]), a small model of its hotbar, and a queue of [`Step`]s produced
//! by a role's planner (explorer, miner, builder, redstone engineer). Each client tick it runs
//! the first step, moves its body, and sends what the vanilla client would: a Move Player
//! packet when it moved or turned, Player Input and sprint changes, and the action packets of
//! the step (Player Action to dig, Use Item On to place or open, Swing, Container Click).
//!
//! Items come from console-style commands the bots may run as operators (`/item replace`), so
//! that they place and dig with real tools and blocks; digging, placing, opening containers and
//! eating all go through the real packets.

mod plans;
mod redstone;
mod steps;
#[cfg(test)]
mod tests;
pub mod tools;

use crate::behavior::Rng;
use crate::metrics::{Shared, Traffic};
use crate::physics::{Body, Input};
use crate::proto;
use crate::world::{self, Column, World};
use crate::Out;
use kiln_proto::{DecodeError, Reader};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

pub use steps::Step;

/// `KILN_BOT_TRACE=1`: bots print what they do (steps, digs, placements) to stderr.
pub(crate) fn tracing_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KILN_BOT_TRACE").is_some())
}

macro_rules! trace {
    ($self:expr, $($arg:tt)*) => {
        if $crate::survival::tracing_on() {
            eprintln!("[bot {:?} tick {}] {}", $self.cfg.role, $self.tick_no, format!($($arg)*));
        }
    };
}
pub(crate) use trace;

/// What a bot does with its time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Walks, sprints and swims across far terrain, generating chunks.
    Explorer,
    /// Digs a shaft down, then tunnels, strips ores and lights the way.
    Miner,
    /// Builds and takes down houses and towers block by block.
    Builder,
    /// Builds redstone machines: clocks, hopper lines, piston doors, farms.
    Redstone,
}

/// The six block faces, in the order the protocol numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

    pub fn vec(self) -> [i32; 3] {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    pub fn face(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        ["down", "up", "north", "south", "west", "east"][self as usize]
    }

    /// Yaw and pitch of a player looking this way.
    pub fn look(self) -> (f32, f32) {
        match self {
            Dir::Down => (0.0, 90.0),
            Dir::Up => (0.0, -90.0),
            Dir::South => (0.0, 0.0),
            Dir::West => (90.0, 0.0),
            Dir::North => (180.0, 0.0),
            Dir::East => (-90.0, 0.0),
        }
    }

    pub fn of_vec(v: [i32; 3]) -> Option<Dir> {
        [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East].into_iter().find(|d| d.vec() == v)
    }
}

pub fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Yaw and pitch (degrees) that look from `from` at `to`.
pub fn look_at(from: [f64; 3], to: [f64; 3]) -> (f32, f32) {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let yaw = (-dx).atan2(dz).to_degrees() as f32;
    let pitch = (-dy.atan2(dx.hypot(dz))).to_degrees() as f32;
    (yaw, pitch)
}

/// A block change the server has yet to acknowledge.
#[derive(Debug, Clone)]
enum Pending {
    Dig { pos: [i32; 3], was: u16 },
    Place { pos: [i32; 3], expect: &'static str, facing: Option<Dir>, slot: u8 },
}

/// What the hotbar slots hold, as far as the bot put them there.
#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    item: &'static str,
    count: u32,
}

/// An open container menu.
#[derive(Debug, Clone, Copy)]
struct Window {
    id: i32,
    state_id: i32,
    /// Slots of the menu including the player's 36.
    slots: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Settling on the ground the server put the bot on.
    Settle(u32),
    /// Asked to be teleported high above the site; waiting for its chunk.
    HighAbove,
    /// Teleported onto the site's surface; waiting for the teleport.
    Landing,
    Active,
}

/// Which chunks the bot expects and how long they took (see [`Shared`]'s histograms).
struct ChunkTrack {
    radius: i32,
    center: Option<(i32, i32)>,
    have: HashSet<(i32, i32)>,
    want: HashMap<(i32, i32), Instant>,
    /// Since when the view has been incomplete, and why it changed.
    since: Option<(Instant, ViewChange)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewChange {
    Join,
    Walk,
    Teleport,
}

/// Settings of one survival bot.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub role: Role,
    /// The group's site: the bot teleports near it and stays around it (explorers leave).
    pub site: [f64; 2],
    pub view_distance: i32,
    pub seed: u64,
    pub chat_interval: Option<std::time::Duration>,
    /// Do not teleport to the site (a single bot at the world spawn).
    pub stay: bool,
    /// Explorers turn back towards the site beyond this distance from it (`None`: they roam
    /// freely), so that groups far apart stay apart.
    pub roam: Option<f64>,
}

pub(crate) struct Agent {
    pub(crate) cfg: Settings,
    pub(crate) world: World,
    pub(crate) body: Body,
    pub(crate) rng: Rng,
    shared: Arc<Shared>,
    pub(crate) counts: Traffic,
    pub(crate) entity_id: i32,
    phase: Phase,
    tick_no: u64,
    track: ChunkTrack,
    queue: VecDeque<Step>,
    /// Role planner state.
    pub(crate) plan: plans::State,
    // Protocol bookkeeping.
    seq: i32,
    pending: Vec<(i32, Pending)>,
    held: u8,
    hot: [Slot; 9],
    window: Option<Window>,
    /// Slot update awaited after an `/item replace`.
    awaiting_slot: Option<(u8, u64)>,
    sneaking: bool,
    sprinting: bool,
    last_input: u8,
    sent: Sent,
    expected_tps: u32,
    // Survival state.
    pub(crate) health: f32,
    pub(crate) food: i32,
    dead_since: Option<u64>,
    next_chat: u64,
    chats: u64,
    last_place_tick: u64,
    pub(crate) dig_cooldown: u32,
    pub(crate) site_y: Option<i32>,
    arrived: bool,
    survival_set: bool,
    recent_teleports: VecDeque<[f64; 3]>,
    /// Hostile mobs in view: entity id and last known position.
    mobs: HashMap<i32, [f64; 3]>,
    hostile: Vec<i32>,
    attack_cooldown: u32,
}

/// What the server last heard about the body.
struct Sent {
    pos: [f64; 3],
    rot: (f32, f32),
    on_ground: bool,
    collision: bool,
    reminder: u32,
}

impl Agent {
    pub fn new(cfg: Settings, shared: Arc<Shared>, pos: [f64; 3], yaw: f32, pitch: f32, entity_id: i32) -> Self {
        let mut rng = Rng::new(cfg.seed);
        let radius = cfg.view_distance.clamp(2, 32);
        let chat = cfg.chat_interval.map(|d| (rng.unit() * d.as_secs_f64() * 20.0) as u64 + 40).unwrap_or(u64::MAX);
        // Players of a group arrive within a few blocks of each other, not on one block.
        let (angle, dist) = (rng.range(0.0, std::f64::consts::TAU), 10.0 * rng.unit().sqrt());
        let plan = plans::State::arriving_at([dist * angle.cos(), dist * angle.sin()]);
        Agent {
            body: Body::new(pos, yaw, pitch),
            world: World::default(),
            rng,
            shared,
            counts: Traffic::default(),
            entity_id,
            phase: Phase::Settle(20),
            tick_no: 0,
            track: ChunkTrack { radius, center: None, have: HashSet::new(), want: HashMap::new(), since: None },
            queue: VecDeque::new(),
            plan,
            seq: 0,
            pending: Vec::new(),
            held: 0,
            hot: [Slot::default(); 9],
            window: None,
            awaiting_slot: None,
            sneaking: false,
            sprinting: false,
            last_input: 0,
            sent: Sent { pos, rot: (yaw, pitch), on_ground: false, collision: false, reminder: 0 },
            expected_tps: 0,
            health: 20.0,
            food: 20,
            dead_since: None,
            next_chat: chat,
            chats: 0,
            last_place_tick: 0,
            dig_cooldown: 0,
            site_y: None,
            arrived: false,
            survival_set: false,
            recent_teleports: VecDeque::new(),
            mobs: HashMap::new(),
            hostile: [
                "zombie", "husk", "drowned", "skeleton", "stray", "spider", "cave_spider", "creeper", "zombie_villager", "witch", "slime",
            ]
            .iter()
            .filter_map(|n| kiln_data::entities::by_name(&format!("minecraft:{n}")).map(|t| t.id))
            .collect(),
            attack_cooldown: 0,
            cfg,
        }
    }

    /// The server put the bot at `pos`: that is where its body starts.
    pub fn spawn_at(&mut self, pos: [f64; 3], yaw: f32, pitch: f32) {
        trace!(self, "spawn at {pos:.1?}");
        self.body.teleport(pos, yaw, pitch);
        self.sent.pos = pos;
        self.sent.rot = (yaw, pitch);
    }

    pub fn role(&self) -> Role {
        self.cfg.role
    }

    // ---- events from the connection -----------------------------------------------------

    /// A Level Chunk With Light packet body (after the packet id).
    pub fn on_chunk(&mut self, body: &[u8]) {
        match Column::parse(body) {
            Ok((x, z, col)) => {
                self.world.insert(x, z, col);
                self.chunk_arrived(x, z);
            }
            Err(e) => {
                self.counts.decode_errors += 1;
                self.shared.problem(format!("level chunk: {e}"));
            }
        }
    }

    /// A chunk that arrived but was not parsed (only its coordinates are known).
    pub fn on_chunk_coords(&mut self, x: i32, z: i32) {
        self.chunk_arrived(x, z);
    }

    fn chunk_arrived(&mut self, x: i32, z: i32) {
        self.track.have.insert((x, z));
        let now = Instant::now();
        if let Some(t) = self.track.want.remove(&(x, z)) {
            self.shared.chunk_latency.add(now - t);
        }
        if self.track.want.is_empty()
            && let Some((since, why)) = self.track.since.take()
        {
            let hist = match why {
                ViewChange::Join => &self.shared.area_ready_join,
                ViewChange::Walk => &self.shared.area_ready_walk,
                ViewChange::Teleport => &self.shared.area_ready_teleport,
            };
            hist.add(now - since);
        }
    }

    pub fn on_forget_chunk(&mut self, x: i32, z: i32) {
        self.world.remove(x, z);
        self.track.have.remove(&(x, z));
    }

    /// Set Chunk Cache Center: the view is now around this chunk.
    pub fn on_chunk_center(&mut self, x: i32, z: i32) {
        let now = Instant::now();
        let why = match self.track.center {
            None => ViewChange::Join,
            Some((ox, oz)) if (ox - x).abs() <= 2 && (oz - z).abs() <= 2 => ViewChange::Walk,
            Some(_) => ViewChange::Teleport,
        };
        self.track.center = Some((x, z));
        let r = self.track.radius;
        self.track.want.retain(|(cx, cz), _| (cx - x).abs() <= r && (cz - z).abs() <= r);
        for cx in x - r..=x + r {
            for cz in z - r..=z + r {
                if !self.track.have.contains(&(cx, cz)) {
                    self.track.want.entry((cx, cz)).or_insert(now);
                }
            }
        }
        if !self.track.want.is_empty() && self.track.since.is_none() {
            self.track.since = Some((now, why));
        }
    }

    pub fn on_chunk_radius(&mut self, r: i32) {
        self.track.radius = r.clamp(2, 32);
        if let Some((x, z)) = self.track.center {
            self.on_chunk_center(x, z);
        }
    }

    pub fn on_block_update(&mut self, pos: [i32; 3], state: u16) {
        self.counts.blocks_seen_changed += 1;
        if tracing_on() {
            let n = world::name(state);
            if ["observer", "lamp", "piston", "hopper", "repeater", "lever"].iter().any(|k| n.contains(k)) {
                let b = kiln_data::blocks_types::block_of(state);
                let props: Vec<String> = b.properties.iter().map(|p| format!("{}={}", p.name, b.property(state, p.name).unwrap_or("?"))).collect();
                trace!(self, "block {pos:?} -> {n} {}", props.join(","));
            }
        }
        self.world.set(pos[0], pos[1], pos[2], state);
    }

    /// Section Blocks Update body: section position, then (state << 12 | local position) varlongs.
    pub fn on_section_update(&mut self, r: &mut Reader) -> Result<(), DecodeError> {
        let packed = r.i64()?;
        let (sx, sy, sz) = ((packed >> 42) as i32, (packed << 44 >> 44) as i32, (packed << 22 >> 42) as i32);
        let n = r.varint()?;
        for _ in 0..n {
            let v = r.varlong()?;
            let (state, local) = ((v >> 12) as u16, v & 0xfff);
            let (x, z, y) = (((local >> 8) & 15) as i32, ((local >> 4) & 15) as i32, (local & 15) as i32);
            self.world.set(sx * 16 + x, sy * 16 + y, sz * 16 + z, state);
        }
        self.counts.blocks_seen_changed += n.max(0) as u64;
        Ok(())
    }

    /// A teleport from the server; `ours` when it answers a `/tp` the bot ran.
    pub fn on_teleport(&mut self, pos: [f64; 3], yaw: f32, pitch: f32) {
        if self.recent_teleports.contains(&pos) {
            // The server asks again when a confirmation takes more than a second (a slow tick),
            // and the repeat can arrive after the next teleport: neither a correction nor the
            // answer to a `/tp` still to come.
            self.counts.teleport_resends += 1;
        } else if self.expected_tps > 0 {
            self.expected_tps -= 1;
            self.counts.own_teleports += 1;
        } else {
            self.counts.teleports += 1;
            self.shared.problem(format!("server correction at tick {}: to {pos:.1?}", self.tick_no));
        }
        if self.recent_teleports.len() == 8 {
            self.recent_teleports.pop_front();
        }
        self.recent_teleports.push_back(pos);
        self.body.teleport(pos, yaw, pitch);
        self.sent.pos = pos;
        self.sent.rot = (yaw, pitch);
        self.sent.reminder = 0;
        // The step in progress assumed another position.
        self.queue.clear();
        self.plan.reset();
        if self.phase == Phase::Landing {
            self.phase = Phase::Settle(30);
        }
    }

    pub fn on_health(&mut self, health: f32, food: i32) {
        if (health - self.health).abs() > 0.01 || food != self.food {
            trace!(self, "health {health} food {food} at {:.1?} on_ground {}", self.body.pos, self.body.on_ground);
        }
        self.health = health;
        self.food = food;
        if health <= 0.0 && self.dead_since.is_none() {
            self.dead_since = Some(self.tick_no);
            self.counts.deaths += 1;
            self.queue.clear();
            self.plan.reset();
        }
    }

    pub fn on_respawn(&mut self) {
        self.counts.respawns += 1;
        self.dead_since = None;
        self.health = 20.0;
        self.food = 20;
        self.queue.clear();
        self.plan.reset();
        self.window = None;
        self.sneaking = false;
        // The server clears the slots: the bot's kit is gone.
        self.hot = [Slot::default(); 9];
        self.held = 0;
        self.phase = Phase::Settle(20);
        self.expected_tps += 1; // the respawn position is not a correction
    }

    /// Add Entity: hostile mobs are tracked from here on.
    pub fn on_add_entity(&mut self, r: &mut Reader) -> Result<(), DecodeError> {
        let id = r.varint()?;
        r.uuid()?;
        let kind = r.varint()?;
        let pos = [r.f64()?, r.f64()?, r.f64()?];
        if self.hostile.contains(&kind) {
            self.mobs.insert(id, pos);
        }
        Ok(())
    }

    /// Move Entity Pos (with or without rotation): a relative move in 1/4096 blocks, possibly in steps.
    pub fn on_entity_move(&mut self, r: &mut Reader) -> Result<(), DecodeError> {
        let id = r.varint()?;
        if !self.mobs.contains_key(&id) {
            return Ok(());
        }
        let props = r.varint()?;
        let stepped = props >> 1 > 0;
        let steps = (props >> 1).max(1);
        let mut d = [0.0f64; 3];
        for _ in 0..steps {
            if stepped {
                r.varint()?;
            }
            for c in &mut d {
                *c += r.i16()? as f64 / 4096.0;
            }
        }
        if let Some(p) = self.mobs.get_mut(&id) {
            for i in 0..3 {
                p[i] += d[i];
            }
        }
        Ok(())
    }

    /// Entity Position Sync: an absolute position (the last step of a stepped path).
    pub fn on_entity_sync(&mut self, r: &mut Reader) -> Result<(), DecodeError> {
        let id = r.varint()?;
        if !self.mobs.contains_key(&id) {
            return Ok(());
        }
        let stepped = r.varint()? != 0;
        let n = if stepped { r.varint()? } else { 1 };
        let mut pos = [0.0; 3];
        for _ in 0..n {
            pos = [r.f64()?, r.f64()?, r.f64()?];
            if stepped {
                r.varint()?;
            }
        }
        self.mobs.insert(id, pos);
        Ok(())
    }

    pub fn on_entity_teleport(&mut self, r: &mut Reader) -> Result<(), DecodeError> {
        let id = r.varint()?;
        if !self.mobs.contains_key(&id) {
            return Ok(());
        }
        let pos = [r.f64()?, r.f64()?, r.f64()?];
        for _ in 0..3 {
            r.f64()?;
        }
        r.f32()?;
        r.f32()?;
        let relative = r.i32()?;
        if relative & 7 == 0 {
            self.mobs.insert(id, pos);
        }
        Ok(())
    }

    pub fn on_remove_entities(&mut self, r: &mut Reader) -> Result<(), DecodeError> {
        let n = r.varint()?;
        for _ in 0..n {
            let id = r.varint()?;
            self.mobs.remove(&id);
        }
        Ok(())
    }

    pub fn on_open_screen(&mut self, id: i32) {
        self.window = Some(Window { id, state_id: 0, slots: 0 });
        self.counts.containers_opened += 1;
    }

    pub fn on_container_content(&mut self, id: i32, state_id: i32, slots: i32) {
        if let Some(w) = self.window.as_mut().filter(|w| w.id == id) {
            w.state_id = state_id;
            w.slots = slots;
        }
    }

    /// Set Player Inventory: inventory slot 0..9 are the hotbar.
    pub fn on_inventory_slot(&mut self, slot: i32) {
        if let Some((s, _)) = self.awaiting_slot
            && slot == s as i32
        {
            self.awaiting_slot = None;
        }
    }

    pub fn on_container_slot(&mut self, id: i32, state_id: i32, slot: i32) {
        if id == 0 {
            if let Some((s, _)) = self.awaiting_slot
                && slot == 36 + s as i32
            {
                self.awaiting_slot = None;
            }
            return;
        }
        if let Some(w) = self.window.as_mut().filter(|w| w.id == id) {
            w.state_id = state_id;
        }
    }

    pub fn on_container_close(&mut self) {
        self.window = None;
    }

    /// Block Changed Ack: every change up to `seq` has been decided.
    pub fn on_ack(&mut self, seq: i32) {
        let done: Vec<Pending> = {
            let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending).into_iter().partition(|(s, _)| *s <= seq);
            self.pending = keep;
            done.into_iter().map(|(_, p)| p).collect()
        };
        for p in done {
            match p {
                Pending::Dig { pos, was } => {
                    let now = self.world.get(pos[0], pos[1], pos[2]).unwrap_or(was);
                    trace!(self, "ack dig {pos:?}: {} -> {}", world::name(was), world::name(now));
                    if now != was {
                        self.counts.dig_done += 1;
                    } else {
                        self.counts.dig_rejected += 1;
                    }
                }
                Pending::Place { pos, expect, facing, slot } => {
                    let now = self.world.get(pos[0], pos[1], pos[2]).unwrap_or(0);
                    trace!(self, "ack place {pos:?}: wanted {expect}, found {}", world::name(now));
                    let info = kiln_data::blocks_types::block_of(now);
                    if info.name == expect {
                        self.counts.placed += 1;
                        if let Some(s) = self.hot.get_mut(slot as usize) {
                            s.count = s.count.saturating_sub(1);
                        }
                        if let Some(f) = facing {
                            let got = info.property(now, "facing");
                            if got != Some(f.name()) {
                                self.counts.place_wrong_state += 1;
                                self.shared.problem(format!("{expect} placed facing {got:?}, wanted {}", f.name()));
                            }
                        }
                    } else {
                        self.counts.place_rejected += 1;
                        self.shared.problem(format!("placing {expect} at {pos:?} left {}", info.name));
                    }
                }
            }
        }
    }

    // ---- helpers for steps --------------------------------------------------------------

    fn next_seq(&mut self) -> i32 {
        self.seq += 1;
        self.seq
    }

    pub(crate) fn command(&mut self, out: &mut Out, command: &str) {
        self.counts.commands += 1;
        out.send(|b| proto::chat_command(b, command));
    }

    pub(crate) fn select(&mut self, out: &mut Out, slot: u8) {
        if self.held != slot {
            self.held = slot;
            out.send(|b| proto::set_carried_item(b, slot));
        }
    }

    /// The first tick that something may be done: not dead, not teleporting.
    fn can_act(&self) -> bool {
        self.dead_since.is_none() && self.awaiting_slot.is_none()
    }

    // ---- the client tick -----------------------------------------------------------------

    /// One client tick: the step in progress, the body, and the packets that follow.
    pub fn tick(&mut self, out: &mut Out) {
        self.tick_no += 1;
        let mut input = Input::default();
        if let Some(dead) = self.dead_since {
            // The death screen: respawn after a moment.
            if self.tick_no - dead == 30 {
                out.send(proto::perform_respawn);
            }
            self.send_pose(out, input);
            return;
        }
        if let Some((_, deadline)) = self.awaiting_slot
            && self.tick_no > deadline
        {
            self.awaiting_slot = None;
        }
        let here = self.body.block_pos();
        let loaded = self.world.has_chunk(here[0] >> 4, here[2] >> 4);
        match self.phase {
            Phase::Settle(n) => {
                if n == 0 && !self.survival_set {
                    // Operators join in the server's default game mode; play survival.
                    self.survival_set = true;
                    self.command(out, "gamemode survival");
                } else if n == 0 {
                    self.phase = if self.cfg.stay || self.near_site() {
                        Phase::Active
                    } else {
                        let [x, z] = self.site_target();
                        let command = format!("tp @s {x:.1} 300.0 {z:.1}");
                        self.expected_tps += 1;
                        self.command(out, &command);
                        Phase::HighAbove
                    };
                } else {
                    self.phase = Phase::Settle(n - 1);
                }
            }
            Phase::HighAbove => {
                // Stand in the air until the site's chunk is here, then pick the surface.
                let [x, z] = self.site_target();
                let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                if self.expected_tps == 0 && self.world.has_chunk(bx >> 4, bz >> 4) {
                    match self.world.surface_y(bx, bz, 319) {
                        Some(y) => {
                            let top = self.world.block(bx, y, bz);
                            if world::is_lava(top) || y < world::MIN_Y + 2 {
                                // Not a place to stand: try somewhere else around the site.
                                let s = self.rng.range(8.0, 48.0);
                                let a = self.rng.range(0.0, std::f64::consts::TAU);
                                self.plan.site_shift = [self.plan.site_shift[0] + s * a.cos(), self.plan.site_shift[1] + s * a.sin()];
                                let [x, z] = self.site_target();
                                let command = format!("tp @s {x:.1} 300.0 {z:.1}");
                                self.expected_tps += 1;
                                self.command(out, &command);
                            } else {
                                let command = format!("tp @s {x:.1} {:.1} {z:.1}", y as f64 + 1.0);
                                self.site_y = Some(y + 1);
                                self.expected_tps += 1;
                                self.command(out, &command);
                                self.phase = Phase::Landing;
                            }
                        }
                        None => {}
                    }
                }
            }
            Phase::Landing => {}
            Phase::Active => {
                if !self.arrived {
                    self.arrived = true;
                    self.shared.arrived.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                if loaded {
                    self.act(out, &mut input);
                }
            }
        }
        // Waiting for a teleport, the bot stays where it is (it would fall from the sky while a
        // slow server answers).
        let holding = matches!(self.phase, Phase::HighAbove | Phase::Landing);
        if loaded && !holding {
            let before = self.body.pos;
            self.body.tick(&self.world, input);
            if self.phase == Phase::Active {
                self.counts.walk_ticks += input.forward as u64;
                let d = (self.body.pos[0] - before[0]).hypot(self.body.pos[2] - before[2]);
                self.counts.walked_dm += (d * 10.0) as u64;
            }
        } else if self.phase == Phase::Active && !loaded {
            self.counts.stall_ticks += 1;
        }
        self.send_pose(out, input);
        self.chat(out);
    }

    fn near_site(&self) -> bool {
        let [x, z] = self.site_target();
        (self.body.pos[0] - x).hypot(self.body.pos[2] - z) < 24.0
    }

    pub(crate) fn site_target(&self) -> [f64; 2] {
        let s = self.plan.site_shift;
        [self.cfg.site[0] + s[0] + self.plan.arrival_offset[0], self.cfg.site[1] + s[1] + self.plan.arrival_offset[1]]
    }

    fn chat(&mut self, out: &mut Out) {
        if self.tick_no >= self.next_chat && self.phase == Phase::Active {
            let every = self.cfg.chat_interval.map_or(u64::MAX / 4, |d| (d.as_secs_f64() * 20.0) as u64);
            self.chats += 1;
            self.counts.chat_sent += 1;
            let msg = ["hello", "anyone have iron?", "nice build", "found diamonds", "brb", "lag?"][self.rng.next_u64() as usize % 6];
            let (ts, salt) = (crate::unix_millis(), self.rng.next_u64() as i64);
            out.send(|b| proto::chat(b, msg, ts, salt));
            self.next_chat = self.tick_no + (every as f64 * self.rng.range(0.5, 1.5)) as u64;
        }
    }

    /// Fights the nearest hostile mob within a few blocks; returns whether it did.
    fn fight(&mut self, out: &mut Out, input: &mut Input) -> bool {
        if self.attack_cooldown > 0 {
            self.attack_cooldown -= 1;
        }
        let me = self.body.pos;
        let near = self
            .mobs
            .iter()
            .map(|(id, p)| (*id, *p, (p[0] - me[0]).hypot(p[2] - me[2])))
            .filter(|(_, p, d)| *d < 6.0 && (p[1] - me[1]).abs() < 3.0)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((id, p, dist)) = near else { return false };
        let (yaw, pitch) = look_at(self.body.eye(), [p[0], p[1] + 1.0, p[2]]);
        self.body.yaw = yaw;
        self.body.pitch = pitch.clamp(-90.0, 90.0);
        self.select(out, 7);
        if dist > 2.7 {
            input.forward = true;
            input.sprint = true;
            input.jump = self.body.horizontal_collision && self.body.on_ground;
        } else if self.attack_cooldown == 0 && self.held == 7 {
            out.send(|b| proto::attack(b, id));
            out.send(proto::punch);
            self.counts.attacks += 1;
            self.attack_cooldown = 12;
        }
        true
    }

    /// Runs the step queue: plans when empty, executes the first step.
    fn act(&mut self, out: &mut Out, input: &mut Input) {
        if self.dig_cooldown > 0 {
            self.dig_cooldown -= 1;
        }
        if !self.can_act() {
            return;
        }
        // A real player fights what comes at them; the planner covers eating.
        if self.fight(out, input) {
            return;
        }
        if self.queue.is_empty() {
            let steps = self.plan_next();
            self.queue.extend(steps);
            if self.queue.is_empty() {
                return;
            }
        }
        let mut step = self.queue.pop_front().expect("queue not empty");
        let before = if tracing_on() { Some(format!("{step:?}")) } else { None };
        match self.exec(&mut step, out, input) {
            steps::Res::Working => self.queue.push_front(step),
            steps::Res::Done => {
                if let Some(b) = before {
                    trace!(self, "done {}", b.chars().take(100).collect::<String>());
                }
            }
            steps::Res::Expand(v) => {
                if let Some(b) = before {
                    trace!(self, "expand {} into {} steps", b.chars().take(60).collect::<String>(), v.len());
                }
                for s in v.into_iter().rev() {
                    self.queue.push_front(s);
                }
            }
            steps::Res::Failed(why) => {
                trace!(self, "FAILED {why}: {step:?}");
                self.shared.problem(format!("{:?} step failed: {why}", self.cfg.role));
                self.queue.clear();
                self.plan.failed();
            }
        }
    }

    /// Sends the position, rotation, input and sprint changes the vanilla client would.
    fn send_pose(&mut self, out: &mut Out, input: Input) {
        let sprint = input.sprint && input.forward;
        let mut flags = 0u8;
        if input.forward {
            flags |= proto::input::FORWARD;
        }
        if input.jump {
            flags |= proto::input::JUMP;
        }
        if self.sneaking {
            flags |= proto::input::SNEAK;
        }
        if sprint {
            flags |= proto::input::SPRINT;
        }
        if flags != self.last_input {
            self.last_input = flags;
            out.send(|b| proto::player_input(b, flags));
        }
        if sprint != self.sprinting {
            self.sprinting = sprint;
            let id = self.entity_id;
            out.send(|b| proto::start_sprinting(b, id, sprint));
        }
        let b = &self.body;
        let s = &mut self.sent;
        s.reminder += 1;
        let [dx, dy, dz] = [0, 1, 2].map(|i| b.pos[i] - s.pos[i]);
        let moved = dx * dx + dy * dy + dz * dz > 4.0e-8 || s.reminder >= 20;
        let rotated = (b.yaw, b.pitch) != s.rot;
        let status = b.on_ground != s.on_ground || b.horizontal_collision != s.collision;
        let m = match (moved, rotated) {
            (true, true) => Some(proto::Move::PosRot(b.pos, b.yaw, b.pitch)),
            (true, false) => Some(proto::Move::Pos(b.pos)),
            (false, true) => Some(proto::Move::Rot(b.yaw, b.pitch)),
            (false, false) if status => Some(proto::Move::StatusOnly),
            (false, false) => None,
        };
        if let Some(m) = m {
            let (ground, coll) = (b.on_ground, b.horizontal_collision);
            out.send(|buf| proto::move_player_flags(buf, m, ground, coll));
            if moved {
                s.pos = b.pos;
                s.reminder = 0;
            }
            if rotated {
                s.rot = (b.yaw, b.pitch);
            }
            s.on_ground = ground;
            s.collision = coll;
        }
    }
}
