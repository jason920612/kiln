//! The simulation thread: owns all game state and runs the 20 TPS tick loop.
//! It never awaits; connections talk to it through channels.

mod interact;
mod stats;

use bytes::Bytes;
use crossbeam_channel::Receiver;
use kiln_link::{ConnId, JoinInfo, PlayIn, Sink, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_world::{ChunkPos, OVERWORLD as OVERWORLD_DIM, World};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct SimConfig {
    pub max_players: usize,
    pub view_distance: u8,
    pub simulation_distance: u8,
}

const TICK: Duration = Duration::from_millis(50);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(30);
const OVERWORLD: &str = "minecraft:overworld";
const MAX_UNACKED_BATCHES: u32 = 10;
/// Player inventory container slots 36..=44 are the hotbar.
const HOTBAR_START: usize = 36;
const INVENTORY_SLOTS: usize = 46;

struct Player {
    name: String,
    #[allow(dead_code)]
    uuid: Uuid,
    #[allow(dead_code)]
    entity_id: i32,
    sink: Box<dyn Sink>,
    pos: [f64; 3],
    rot: [f32; 2],
    on_ground: bool,
    view_distance: i32,
    center: ChunkPos,
    sent_chunks: HashSet<ChunkPos>,
    awaiting_teleport: Option<i32>,
    keep_alive: Option<(i64, Instant)>,
    last_keep_alive: Instant,
    chunks_per_tick: f32,
    unacked_batches: u32,
    /// Item id and count per inventory container slot.
    inventory: [Option<(i32, i32)>; INVENTORY_SLOTS],
    selected: usize,
}

impl Player {
    fn send(&self, p: Bytes) {
        self.sink.send(p);
    }
    fn disconnect(&self, reason: &str) {
        self.sink.disconnect(packets::play_disconnect(reason));
    }
    fn held_item(&self) -> Option<(i32, i32)> {
        self.inventory[HOTBAR_START + self.selected]
    }
}

pub struct Sim {
    config: SimConfig,
    world: World,
    players: HashMap<ConnId, Player>,
    next_entity_id: i32,
    started: Instant,
    stats: stats::TickStats,
}

pub fn run(config: SimConfig, rx: Receiver<ToSim>) {
    let plains = kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains").expect("plains biome");
    let biome_count = kiln_data::registries::SYNCHRONIZED
        .iter()
        .find(|(r, _)| *r == "minecraft:worldgen/biome")
        .map_or(0, |(_, e)| e.len());
    let mut sim = Sim {
        config,
        world: World::flat(OVERWORLD_DIM, plains as u16, biome_count),
        players: HashMap::new(),
        next_entity_id: 1,
        started: Instant::now(),
        stats: stats::TickStats::default(),
    };
    let mut next_tick = Instant::now();
    loop {
        let start = Instant::now();
        loop {
            match rx.try_recv() {
                Ok(msg) => sim.handle(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }
        sim.tick();
        if let Some(report) = sim.stats.record(start.elapsed()) {
            info!("{} players, {} chunks | {report}", sim.players.len(), sim.world.loaded_chunks());
        }

        // Fixed 50 ms cadence; if we fell behind, don't try to catch up.
        next_tick += TICK;
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }
}

impl Sim {
    fn handle(&mut self, msg: ToSim) {
        match msg {
            ToSim::Join(j) => self.join(j),
            ToSim::Leave(conn) => {
                if let Some(p) = self.players.remove(&conn) {
                    self.broadcast_system(yellow(&format!("{} left the game", p.name)));
                }
            }
            ToSim::Packet(conn, pkt) => self.packet(conn, pkt),
        }
    }

    fn join(&mut self, j: JoinInfo) {
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        let spawn = [8.5, self.world.flat_surface_y(), 8.5];
        let view_distance = (j.view_distance as i32).min(self.config.view_distance as i32);
        let dimension_type =
            kiln_data::synced_id("minecraft:dimension_type", OVERWORLD).expect("overworld dimension type");
        let player = Player {
            name: j.name,
            uuid: j.uuid,
            entity_id,
            sink: j.sink,
            pos: spawn,
            rot: [0.0, 0.0],
            on_ground: true,
            view_distance,
            center: ChunkPos::of_block(spawn[0] as i32, spawn[2] as i32),
            sent_chunks: HashSet::new(),
            awaiting_teleport: Some(1),
            keep_alive: None,
            last_keep_alive: Instant::now(),
            chunks_per_tick: 9.0,
            unacked_batches: 0,
            inventory: [None; INVENTORY_SLOTS],
            selected: 0,
        };

        player.send(packets::play_login(&packets::Login {
            entity_id,
            dimensions: &[OVERWORLD],
            max_players: self.config.max_players as i32,
            view_distance: self.config.view_distance as i32,
            simulation_distance: self.config.simulation_distance as i32,
            dimension_type,
            dimension: OVERWORLD,
            game_mode: 1,
            is_flat: true,
            sea_level: 63,
        }));
        player.send(packets::player_position(1, spawn, 0.0, 0.0));
        player.send(packets::set_default_spawn_position(OVERWORLD, [8, spawn[1] as i32, 8], 0.0, 0.0));
        player.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        player.send(packets::set_chunk_cache_center(player.center.x, player.center.z));

        let msg = yellow(&format!("{} joined the game", player.name));
        self.players.insert(j.conn, player);
        self.broadcast_system(msg);
    }

    fn packet(&mut self, conn: ConnId, pkt: PlayIn) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        match pkt {
            PlayIn::AcceptTeleport { id } => {
                if p.awaiting_teleport == Some(id) {
                    p.awaiting_teleport = None;
                }
            }
            PlayIn::KeepAlive { id } => {
                if p.keep_alive.is_some_and(|(k, _)| k == id) {
                    p.keep_alive = None;
                }
            }
            PlayIn::Move { pos, rot, on_ground } => {
                // Movement sent before the client saw our teleport is stale.
                if p.awaiting_teleport.is_some() {
                    return;
                }
                if pos.is_some_and(|v| v.iter().any(|c| !c.is_finite()))
                    || rot.is_some_and(|v| v.iter().any(|c| !c.is_finite()))
                {
                    p.disconnect("Invalid movement");
                    return;
                }
                if let Some(pos) = pos {
                    p.pos = pos;
                }
                if let Some(rot) = rot {
                    p.rot = rot;
                }
                p.on_ground = on_ground;
            }
            PlayIn::ChunkBatchReceived { chunks_per_tick } => {
                p.unacked_batches = p.unacked_batches.saturating_sub(1);
                if chunks_per_tick.is_finite() {
                    p.chunks_per_tick = chunks_per_tick.clamp(0.01, 64.0);
                }
            }
            PlayIn::ClientInformation { view_distance } => {
                p.view_distance = (view_distance as i32).min(self.config.view_distance as i32);
            }
            PlayIn::Chat { message } => {
                let line = format!("<{}> {}", p.name, message);
                info!("{line}");
                self.broadcast_system(kiln_proto::nbt::text(&line));
            }
            // Sent by the client when its "Loading terrain" screen closes.
            PlayIn::PlayerLoaded => info!("{} finished loading terrain", p.name),
            PlayIn::SetCarriedItem { slot } => {
                if (0..9).contains(&slot) {
                    p.selected = slot as usize;
                }
            }
            PlayIn::SetCreativeSlot { slot, item } => {
                if let Some(s) = usize::try_from(slot).ok().filter(|&s| s < INVENTORY_SLOTS) {
                    p.inventory[s] = item.map(|i| (i.item, i.count));
                }
            }
            PlayIn::PlayerAction { action, pos, sequence, .. } => {
                // Creative mode: starting to dig breaks the block instantly.
                const START_DIGGING: i32 = 0;
                if action == START_DIGGING && self.within_reach(conn, pos) {
                    self.set_block(pos, kiln_data::blocks::default_state::AIR);
                }
                self.ack(conn, sequence);
            }
            PlayIn::UseItemOn { hand, pos, face, sequence, .. } => {
                self.use_item_on(conn, hand, pos, face);
                self.ack(conn, sequence);
            }
            PlayIn::Punch => {}
        }
    }

    fn ack(&self, conn: ConnId, sequence: i32) {
        if let Some(p) = self.players.get(&conn) {
            p.send(packets::block_changed_ack(sequence));
        }
    }

    fn within_reach(&self, conn: ConnId, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&conn) else { return false };
        let d: f64 = (0..3).map(|i| (pos[i] as f64 + 0.5 - p.pos[i]).powi(2)).sum();
        d <= 12.0 * 12.0
    }

    fn use_item_on(&mut self, conn: ConnId, hand: i32, pos: [i32; 3], face: i32) {
        let Some(p) = self.players.get(&conn) else { return };
        let item = if hand == 0 { p.held_item() } else { p.inventory[45] };
        let Some((item, _)) = item else { return };
        let Some(block) = interact::block_for_item(item) else { return };
        let Some(off) = interact::offset(face) else { return };
        let yaw = p.rot[0];
        // Place into the clicked block if it is replaceable, otherwise next to it.
        let clicked = self.world.get_block(pos[0], pos[1], pos[2]);
        let target = if clicked.is_some_and(interact::replaceable) {
            pos
        } else {
            [pos[0] + off[0], pos[1] + off[1], pos[2] + off[2]]
        };
        if !self.within_reach(conn, target) {
            return;
        }
        if !self.world.get_block(target[0], target[1], target[2]).is_some_and(interact::replaceable) {
            return;
        }
        self.set_block(target, interact::placement_state(block, face, yaw));
    }

    /// Changes a block and tells everyone who has its chunk.
    fn set_block(&mut self, pos: [i32; 3], state: u16) {
        match self.world.set_block(pos[0], pos[1], pos[2], state) {
            Some(old) if old != state => {
                let chunk = ChunkPos::of_block(pos[0], pos[2]);
                let pkt = packets::block_update(pos, state);
                for p in self.players.values().filter(|p| p.sent_chunks.contains(&chunk)) {
                    p.send(pkt.clone());
                }
            }
            Some(_) => {}
            None => debug!("block change outside the world at {pos:?}"),
        }
    }

    fn broadcast_system(&self, text: Tag) {
        let pkt = packets::system_chat(text, false);
        for p in self.players.values() {
            p.send(pkt.clone());
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let world = &mut self.world;
        for p in self.players.values_mut() {
            if let Some((_, sent)) = p.keep_alive {
                if now - sent > KEEP_ALIVE_TIMEOUT {
                    warn!("{} timed out", p.name);
                    p.disconnect("Timed out");
                    continue;
                }
            } else if now - p.last_keep_alive > KEEP_ALIVE_INTERVAL {
                let id = self.started.elapsed().as_millis() as i64;
                p.keep_alive = Some((id, now));
                p.last_keep_alive = now;
                p.send(packets::keep_alive(id));
            }
            update_chunks(p, world);
        }
    }
}

/// Recenters the player's chunk view and streams missing chunks, nearest first.
fn update_chunks(p: &mut Player, world: &mut World) {
    let center = ChunkPos::of_block(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
    let r = p.view_distance;
    if center != p.center {
        p.center = center;
        p.send(packets::set_chunk_cache_center(center.x, center.z));
        let stale: Vec<_> = p
            .sent_chunks
            .iter()
            .copied()
            .filter(|c| (c.x - center.x).abs() > r || (c.z - center.z).abs() > r)
            .collect();
        for c in stale {
            p.sent_chunks.remove(&c);
            p.send(packets::forget_level_chunk(c.x, c.z));
        }
    }

    if p.unacked_batches >= MAX_UNACKED_BATCHES {
        return;
    }
    let budget = (p.chunks_per_tick.ceil() as usize).max(1);
    let mut missing: Vec<ChunkPos> = Vec::new();
    for x in center.x - r..=center.x + r {
        for z in center.z - r..=center.z + r {
            let c = ChunkPos::new(x, z);
            if !p.sent_chunks.contains(&c) {
                missing.push(c);
            }
        }
    }
    if missing.is_empty() {
        return;
    }
    missing.sort_by_key(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2));
    missing.truncate(budget);

    p.send(packets::chunk_batch_start());
    for &c in &missing {
        p.send(packets::level_chunk_with_light(c.x, c.z, &world.chunk_body(c)));
        p.sent_chunks.insert(c);
    }
    p.send(packets::chunk_batch_finished(missing.len() as i32));
    p.unacked_batches += 1;
}

fn yellow(s: &str) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(s.into())), ("color".into(), Tag::String("yellow".into()))])
}
