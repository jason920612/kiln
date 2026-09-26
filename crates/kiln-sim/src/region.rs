//! Region-local work: what one region does in the parallel phases (design §4.3).
//!
//! A [`RegionWork`] holds a region's cells and its players (disjoint `&mut` borrows, so
//! regions can run on different threads) plus the region's packets for this tick. Nothing
//! here can reach another region's cells or players; effects outside the region go through
//! the outputs the serial phases pick up (chunk requests and unloads).

use crate::movement;
use crate::{HOTBAR_START, INVENTORY_SLOTS, KEEP_ALIVE_INTERVAL, KEEP_ALIVE_TIMEOUT, MAX_UNACKED_BATCHES, Player, interact};
use bytes::Bytes;
use kiln_link::{ConnId, PlayIn};
use kiln_proto::packets;
use kiln_region::CellSet;
use kiln_world::{Blocks, Cell, CellStore, ChunkPos};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Read-only values of the global state that region work needs.
#[derive(Clone, Copy)]
pub(crate) struct Env {
    pub game_time: i64,
    /// The server's view distance: clients may ask for less.
    pub max_view: i32,
    /// `minecraft:player_movement_check`.
    pub movement_check: bool,
    pub biome_count: usize,
    pub now: Instant,
    /// Id for keep-alives sent this tick.
    pub keep_alive_id: i64,
}

/// A block a packet changed; players with its chunk hear about it once the region's
/// packets are applied.
pub(crate) struct BlockChange {
    pub pos: [i32; 3],
    pub state: u16,
    /// Block Entity Data vanilla sends after the change, if any.
    pub data: Option<Bytes>,
}

/// What a region leaves for the next serial phase.
#[derive(Default)]
pub(crate) struct RegionOut {
    /// Chunks its players need that are not loaded: (rank in the player's nearest-first
    /// list, player, chunk).
    pub wanted: Vec<(u32, ConnId, ChunkPos)>,
    /// Loaded chunks no player of the region is near any more.
    pub unload: Vec<ChunkPos>,
    /// CPU time per sub-phase, for the statistics.
    pub times: [Duration; SUB_PHASES.len()],
}

pub(crate) const SUB_PHASES: [&str; 5] = ["connections", "visibility", "movement", "light", "egress"];

pub(crate) struct RegionWork<'a> {
    pub cells: &'a mut CellSet<Cell>,
    /// Sorted by connection id.
    pub players: Vec<&'a mut Player>,
    /// This region's packets for the tick, in arrival order.
    pub packets: Vec<(ConnId, PlayIn)>,
    pub out: RegionOut,
}

impl RegionWork<'_> {
    fn index_of(&self, conn: ConnId) -> Option<usize> {
        self.players.binary_search_by_key(&conn, |p| p.conn).ok()
    }

    /// P1: applies the region's packets in arrival order.
    pub fn apply_packets(&mut self, env: &Env) {
        let mut changes = Vec::new();
        for (conn, pkt) in std::mem::take(&mut self.packets) {
            let Some(i) = self.index_of(conn) else { continue };
            local_packet(self.players[i], &mut *self.cells, env, pkt, &mut changes);
        }
        notify_block_changes(self.players.iter_mut().map(|p| &mut **p), &changes);
    }

    /// L: connection upkeep, chunk streaming, tracking, light, then egress.
    pub fn tick(&mut self, env: &Env) {
        let mut lap = Instant::now();
        let mut mark = |times: &mut [Duration; 5], i: usize| {
            let now = Instant::now();
            times[i] += now - lap;
            lap = now;
        };
        for p in self.players.iter_mut() {
            tick_connection(p, env);
            if !p.disconnected {
                update_chunks(p, &mut *self.cells, env, &mut self.out.wanted);
            }
        }
        // The same tick everywhere, so when chunks unload does not depend on the regions.
        if env.game_time % 20 == 0 {
            self.find_unloads();
        }
        mark(&mut self.out.times, 0);
        crate::players::update_visibility(&mut self.players);
        mark(&mut self.out.times, 1);
        crate::players::broadcast_movement(&mut self.players);
        mark(&mut self.out.times, 2);
        self.send_light_updates();
        mark(&mut self.out.times, 3);
        for p in self.players.iter_mut() {
            p.flush();
        }
        mark(&mut self.out.times, 4);
    }

    /// Chunks outside every player's view (plus one chunk of margin) can go.
    fn find_unloads(&mut self) {
        let mut centers: Vec<(ChunkPos, i32)> = self.players.iter().map(|p| (p.center, p.view_distance + 1)).collect();
        centers.sort_unstable_by_key(|&(c, r)| (c, r));
        centers.dedup();
        let mut near = HashSet::new();
        for (o, r) in centers {
            for x in o.x - r..=o.x + r {
                for z in o.z - r..=o.z + r {
                    near.insert(ChunkPos::new(x, z));
                }
            }
        }
        let mut unload = Vec::new();
        self.cells.for_each_cell(&mut |pos, cell| {
            unload.extend(cell.chunks(pos).map(|(c, _)| c).filter(|c| !near.contains(c)));
        });
        self.out.unload = unload;
    }

    /// Update Light for every chunk whose light changed, to players who have it.
    fn send_light_updates(&mut self) {
        for (pos, sky, block) in self.cells.take_light_changes() {
            if !self.players.iter().any(|p| p.sent_chunks.contains(&pos)) {
                continue;
            }
            let Some(body) = self.cells.light_update_body(pos, sky, block) else { continue };
            let pkt = packets::light_update(pos.x, pos.z, &body);
            for p in self.players.iter_mut().filter(|p| p.sent_chunks.contains(&pos)) {
                p.send(pkt.clone());
            }
        }
    }
}

/// Sends Block Update for each change to the players that have the chunk.
pub(crate) fn notify_block_changes<'p>(players: impl Iterator<Item = &'p mut Player>, changes: &[BlockChange]) {
    if changes.is_empty() {
        return;
    }
    let changes: Vec<_> = changes
        .iter()
        .map(|c| (ChunkPos::of_block(c.pos[0], c.pos[2]), packets::block_update(c.pos, c.state), &c.data))
        .collect();
    for p in players {
        for (chunk, pkt, data) in &changes {
            if p.sent_chunks.contains(chunk) {
                p.send(pkt.clone());
                if let Some(d) = data {
                    p.send(d.clone());
                }
            }
        }
    }
}

/// Whether a packet needs the whole server (chat, commands): it and everything its region
/// receives after it this tick run in the serial PX phase.
pub(crate) fn is_exclusive(pkt: &PlayIn) -> bool {
    matches!(pkt, PlayIn::ChatCommand { .. } | PlayIn::CommandSuggestion { .. } | PlayIn::Chat { .. })
}

/// A packet that touches only its player and the world around it.
pub(crate) fn local_packet<W: Blocks + ?Sized>(
    p: &mut Player,
    world: &mut W,
    env: &Env,
    pkt: PlayIn,
    changes: &mut Vec<BlockChange>,
) {
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
        PlayIn::Move { pos, rot, on_ground } => handle_move(p, world, env, pos, rot, on_ground),
        PlayIn::ChunkBatchReceived { chunks_per_tick } => {
            p.unacked_batches = p.unacked_batches.saturating_sub(1);
            if chunks_per_tick.is_finite() {
                p.chunks_per_tick = chunks_per_tick.clamp(0.01, 64.0);
            }
        }
        PlayIn::ClientInformation(info) => {
            p.view_distance = (info.view_distance as i32).min(env.max_view);
            p.section = None;
            p.client = info;
        }
        PlayIn::PlayerInput { flags } => {
            let sneaking = flags & 0x20 != 0;
            if sneaking != p.sneaking {
                p.sneaking = sneaking;
                p.meta_dirty = true;
            }
        }
        PlayIn::PlayerCommand { action } => {
            const START_SPRINTING: i32 = 1;
            const STOP_SPRINTING: i32 = 2;
            let sprinting = match action {
                START_SPRINTING => true,
                STOP_SPRINTING => false,
                _ => p.sprinting,
            };
            if sprinting != p.sprinting {
                p.sprinting = sprinting;
                p.meta_dirty = true;
            }
        }
        // Sent by the client when its "Loading terrain" screen closes.
        PlayIn::PlayerLoaded => {
            p.load_timeout = 0;
            info!("{} finished loading terrain", p.name);
        }
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
            if action == START_DIGGING && within_reach(p, pos) {
                set_block(world, pos, kiln_data::blocks::default_state::AIR, changes);
            }
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::UseItemOn { hand, pos, face, sequence, .. } => {
            use_item_on(p, world, hand, pos, face, changes);
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::Punch => p.swung = true,
        PlayIn::ClientTickEnd => p.position_this_tick = false,
        _ => {}
    }
}

/// Changes a block in a loaded chunk; returns whether the chunk was loaded.
pub(crate) fn set_block<W: Blocks + ?Sized>(world: &mut W, pos: [i32; 3], state: u16, changes: &mut Vec<BlockChange>) -> bool {
    let Some(old) = world.set_block(pos[0], pos[1], pos[2], state) else { return false };
    if old != state {
        let data = world.block_entity_data(pos[0], pos[1], pos[2]);
        let data = data.map(|(kind, tag)| packets::block_entity_data(pos, kind as i32, &tag));
        changes.push(BlockChange { pos, state, data });
    }
    true
}

fn within_reach(p: &Player, pos: [i32; 3]) -> bool {
    let d: f64 = (0..3).map(|i| (pos[i] as f64 + 0.5 - p.pos[i]).powi(2)).sum();
    d <= 12.0 * 12.0
}

fn use_item_on<W: Blocks + ?Sized>(
    p: &Player,
    world: &mut W,
    hand: i32,
    pos: [i32; 3],
    face: i32,
    changes: &mut Vec<BlockChange>,
) {
    let item = if hand == 0 { p.inventory[HOTBAR_START + p.selected] } else { p.inventory[45] };
    let Some((item, _)) = item else { return };
    let Some(block) = interact::block_for_item(item) else { return };
    let Some(off) = interact::offset(face) else { return };
    // Place into the clicked block if it is replaceable, otherwise next to it.
    let clicked = world.get_block(pos[0], pos[1], pos[2]);
    let target =
        if clicked.is_some_and(interact::replaceable) { pos } else { [pos[0] + off[0], pos[1] + off[1], pos[2] + off[2]] };
    if !within_reach(p, target) || !world.get_block(target[0], target[1], target[2]).is_some_and(interact::replaceable) {
        return;
    }
    set_block(world, target, interact::placement_state(block, face, p.rot[0]), changes);
}

fn handle_move<W: Blocks + ?Sized>(
    p: &mut Player,
    world: &W,
    env: &Env,
    pos: Option<[f64; 3]>,
    rot: Option<[f32; 2]>,
    on_ground: bool,
) {
    let now = env.game_time;
    if movement::invalid(pos, rot) {
        p.disconnect("Invalid movement");
        return;
    }
    if pos.is_some() {
        // The 26.3 client sends at most one position per client tick.
        if p.position_this_tick {
            p.disconnect("Invalid movement");
            return;
        }
        p.position_this_tick = true;
    }
    if p.load_timeout > 0 {
        return;
    }
    let rot = rot.map_or(p.rot, movement::normalize_rotation);
    if p.awaiting_teleport.is_some() {
        // Movement sent before the client saw our teleport is stale; only the view turns.
        p.rot = rot;
        if now - p.teleport_sent > movement::TELEPORT_RESEND_TICKS {
            p.teleport(p.pos, rot, now);
        }
        return;
    }
    let to = pos.map_or(p.pos, movement::clamp_position);
    p.move_packets += 1;
    if env.movement_check && movement::too_fast(p.first_good, to, 0.0, p.move_packets, false) {
        let d = [to[0] - p.first_good[0], to[1] - p.first_good[1], to[2] - p.first_good[2]];
        warn!("{} moved too quickly! {d:?}", p.name);
        p.teleport(p.pos, p.rot, now);
        return;
    }
    // Spectators have no physics.
    if p.game_mode != 3 && to != p.pos {
        let old = movement::Aabb::player(p.pos, movement::MIN_POSE_HEIGHT);
        let new = movement::Aabb::player(to, movement::MIN_POSE_HEIGHT);
        if movement::collides_with_anything_new(world, old, new) {
            p.teleport(p.pos, rot, now);
            return;
        }
    }
    p.pos = to;
    p.rot = rot;
    p.on_ground = on_ground;
}

/// Start of a connection's tick: block change acks (after the block updates they
/// acknowledge, like vanilla's connection tick), movement bookkeeping and keep-alives.
fn tick_connection(p: &mut Player, env: &Env) {
    if p.ack_block_changes >= 0 {
        p.send(packets::block_changed_ack(p.ack_block_changes));
        p.ack_block_changes = -1;
    }
    p.first_good = p.pos;
    p.move_packets = 0;
    p.load_timeout = p.load_timeout.saturating_sub(1);
    if let Some((_, sent)) = p.keep_alive {
        if env.now - sent > KEEP_ALIVE_TIMEOUT {
            warn!("{} timed out", p.name);
            p.disconnect("Timed out");
        }
    } else if env.now - p.last_keep_alive > KEEP_ALIVE_INTERVAL {
        p.keep_alive = Some((env.keep_alive_id, env.now));
        p.last_keep_alive = env.now;
        p.send(packets::keep_alive(env.keep_alive_id));
    }
}

/// Recenters the player's chunk view and streams missing loaded chunks, nearest first;
/// missing chunks that are not loaded yet are requested.
fn update_chunks(p: &mut Player, cells: &mut CellSet<Cell>, env: &Env, wanted: &mut Vec<(u32, ConnId, ChunkPos)>) {
    let center = ChunkPos::of_block(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
    let r = p.view_distance;
    // A smaller view distance forgets chunks too (vanilla `updateChunkTracking`).
    if center != p.center || r != p.applied_view {
        if center != p.center {
            p.center = center;
            p.send(packets::set_chunk_cache_center(center.x, center.z));
        }
        p.applied_view = r;
        let mut stale: Vec<_> =
            p.sent_chunks.iter().copied().filter(|c| (c.x - center.x).abs() > r || (c.z - center.z).abs() > r).collect();
        stale.sort_unstable();
        for c in stale {
            p.sent_chunks.remove(&c);
            p.send(packets::forget_level_chunk(c.x, c.z));
        }
    }
    if p.unacked_batches >= MAX_UNACKED_BATCHES {
        return;
    }
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
    let budget = (p.chunks_per_tick.ceil() as usize).max(1);
    let mut batch = Vec::new();
    // Ask for about what the client takes in the next tick or two, nearest first.
    let mut asked = 0;
    for c in missing {
        match cells.chunk_mut(c) {
            Some(chunk) if batch.len() < budget => batch.push((c, chunk.packet_body(env.biome_count))),
            Some(_) => {}
            None if asked < 2 * budget => {
                wanted.push((asked as u32, p.conn, c));
                asked += 1;
            }
            None => {}
        }
    }
    if batch.is_empty() {
        return;
    }
    p.send(packets::chunk_batch_start());
    for (c, body) in &batch {
        p.send(packets::level_chunk_with_light(c.x, c.z, body));
        p.sent_chunks.insert(*c);
    }
    p.send(packets::chunk_batch_finished(batch.len() as i32));
    p.unacked_batches += 1;
}

/// De-duplicated union of chunk requests, keeping the first occurrence.
pub(crate) fn merge_requests(requests: impl IntoIterator<Item = ChunkPos>) -> Vec<ChunkPos> {
    let mut seen = HashSet::new();
    requests.into_iter().filter(|c| seen.insert(*c)).collect()
}
