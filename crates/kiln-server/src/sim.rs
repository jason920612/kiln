//! The simulation thread: owns all game state and runs the 20 TPS tick loop.
//! It never awaits; connections talk to it through channels.

use crate::net::{ConnId, Outbound, Shared};
use crate::packets::{self, PlayIn};
use crate::world::{self, FlatWorld};
use bytes::Bytes;
use crossbeam_channel::Receiver;
use kiln_proto::nbt::Tag;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};
use uuid::Uuid;

pub enum ToSim {
    Join(JoinInfo),
    Packet(ConnId, PlayIn),
    Leave(ConnId),
}

pub struct JoinInfo {
    pub conn: ConnId,
    pub name: String,
    pub uuid: Uuid,
    pub view_distance: u8,
    pub out: UnboundedSender<Outbound>,
}

const TICK: Duration = Duration::from_millis(50);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(30);
const OVERWORLD: &str = "minecraft:overworld";
const MAX_UNACKED_BATCHES: u32 = 10;

struct Player {
    name: String,
    #[allow(dead_code)]
    uuid: Uuid,
    #[allow(dead_code)]
    entity_id: i32,
    out: UnboundedSender<Outbound>,
    pos: [f64; 3],
    rot: [f32; 2],
    on_ground: bool,
    view_distance: i32,
    center: (i32, i32),
    sent_chunks: HashSet<(i32, i32)>,
    awaiting_teleport: Option<i32>,
    keep_alive: Option<(i64, Instant)>,
    last_keep_alive: Instant,
    chunks_per_tick: f32,
    unacked_batches: u32,
}

impl Player {
    fn send(&self, p: Bytes) {
        let _ = self.out.send(Outbound::Packet(p));
    }
    fn disconnect(&self, reason: &str) {
        let _ = self.out.send(Outbound::Disconnect(packets::play_disconnect(reason)));
    }
}

pub struct Sim {
    shared: Arc<Shared>,
    world: FlatWorld,
    players: HashMap<ConnId, Player>,
    next_entity_id: i32,
    started: Instant,
    tick_count: u64,
    tick_nanos: u64,
}

pub fn run(shared: Arc<Shared>, rx: Receiver<ToSim>) {
    let plains = kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains").expect("plains biome");
    let mut sim = Sim {
        shared,
        world: FlatWorld::new(plains),
        players: HashMap::new(),
        next_entity_id: 1,
        started: Instant::now(),
        tick_count: 0,
        tick_nanos: 0,
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
        sim.tick_nanos += start.elapsed().as_nanos() as u64;
        sim.tick_count += 1;
        if sim.tick_count % 600 == 0 {
            let mspt = sim.tick_nanos as f64 / 600.0 / 1e6;
            info!("{} players, {:.3} mspt (avg over 30 s)", sim.players.len(), mspt);
            sim.tick_nanos = 0;
        }

        // Fixed 50 ms cadence; if we fell behind, don't try to catch up.
        next_tick += TICK;
        let now = Instant::now();
        if next_tick > now {
            // Wake early to drain packets that arrive mid-sleep in the next tick.
            std::thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }
}

fn chunk_of(pos: [f64; 3]) -> (i32, i32) {
    ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4)
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
        let spawn = [8.5, world::SURFACE_Y, 8.5];
        let view_distance = (j.view_distance as i32).min(self.shared.config.view_distance as i32);
        let dimension_type =
            kiln_data::synced_id("minecraft:dimension_type", OVERWORLD).expect("overworld dimension type");
        let player = Player {
            name: j.name,
            uuid: j.uuid,
            entity_id,
            out: j.out,
            pos: spawn,
            rot: [0.0, 0.0],
            on_ground: true,
            view_distance,
            center: chunk_of(spawn),
            sent_chunks: HashSet::new(),
            awaiting_teleport: Some(1),
            keep_alive: None,
            last_keep_alive: Instant::now(),
            chunks_per_tick: 9.0,
            unacked_batches: 0,
        };

        player.send(packets::play_login(&packets::Login {
            entity_id,
            dimensions: &[OVERWORLD],
            max_players: self.shared.config.max_players as i32,
            view_distance: self.shared.config.view_distance as i32,
            simulation_distance: self.shared.config.simulation_distance as i32,
            dimension_type,
            dimension: OVERWORLD,
            game_mode: 1,
            is_flat: true,
            sea_level: 63,
        }));
        player.send(packets::player_position(1, spawn, 0.0, 0.0));
        player.send(packets::set_default_spawn_position(OVERWORLD, [8, spawn[1] as i32, 8], 0.0, 0.0));
        player.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        player.send(packets::set_chunk_cache_center(player.center.0, player.center.1));

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
                if let Some(pos) = pos {
                    if pos.iter().any(|v| !v.is_finite()) {
                        p.disconnect("Invalid movement");
                        return;
                    }
                    p.pos = pos;
                }
                if let Some(rot) = rot {
                    if rot.iter().any(|v| !v.is_finite()) {
                        p.disconnect("Invalid movement");
                        return;
                    }
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
                p.view_distance = (view_distance as i32).min(self.shared.config.view_distance as i32);
            }
            PlayIn::Chat { message } => {
                let line = format!("<{}> {}", p.name, message);
                info!("{line}");
                self.broadcast_system(kiln_proto::nbt::text(&line));
            }
            // Sent by the client when its "Loading terrain" screen closes.
            PlayIn::PlayerLoaded => info!("{} finished loading terrain", p.name),
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
        for p in self.players.values_mut() {
            // Keep-alive.
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

            update_chunks(p, &self.world);
        }
    }
}

/// Recenters the player's chunk view and streams missing chunks, nearest first.
fn update_chunks(p: &mut Player, world: &FlatWorld) {
    let center = chunk_of(p.pos);
    let r = p.view_distance;
    if center != p.center {
        p.center = center;
        p.send(packets::set_chunk_cache_center(center.0, center.1));
        let (cx, cz) = center;
        let stale: Vec<_> =
            p.sent_chunks.iter().copied().filter(|&(x, z)| (x - cx).abs() > r || (z - cz).abs() > r).collect();
        for (x, z) in stale {
            p.sent_chunks.remove(&(x, z));
            p.send(packets::forget_level_chunk(x, z));
        }
    }

    if p.unacked_batches >= MAX_UNACKED_BATCHES {
        return;
    }
    let budget = (p.chunks_per_tick.ceil() as usize).max(1);
    let (cx, cz) = center;
    let mut missing: Vec<(i32, i32)> = Vec::new();
    for x in cx - r..=cx + r {
        for z in cz - r..=cz + r {
            if !p.sent_chunks.contains(&(x, z)) {
                missing.push((x, z));
            }
        }
    }
    if missing.is_empty() {
        return;
    }
    missing.sort_by_key(|&(x, z)| (x - cx).pow(2) + (z - cz).pow(2));
    missing.truncate(budget);

    p.send(packets::chunk_batch_start());
    for &(x, z) in &missing {
        p.send(packets::level_chunk_with_light(x, z, world.chunk_body(x, z)));
        p.sent_chunks.insert((x, z));
    }
    p.send(packets::chunk_batch_finished(missing.len() as i32));
    p.unacked_batches += 1;
}

fn yellow(s: &str) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(s.into())), ("color".into(), Tag::String("yellow".into()))])
}
