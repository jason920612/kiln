//! The simulation thread: owns all game state and runs the 20 TPS tick loop.
//! It never awaits; connections talk to it through channels.
//!
//! # Regions and the tick (design §4.3, lockstep mode)
//!
//! Loaded chunks live in 8×8-chunk cells, and cells belong to regions that the regionizer
//! (`kiln-region`) keeps apart by at least 256 blocks, so regions cannot affect each other
//! within a tick and run in parallel on the tick pool (`kiln-sched`). Players belong to the
//! region that owns the cell they stand in. Each tick:
//!
//! - **B0** (serial): connection events; chunk unloads and loads (requests from the last
//!   tick, spawn points of joining players); the regionizer merges and splits regions;
//!   joins; region membership of every player.
//! - **P** (parallel): each region applies its players' packets in arrival order, up to the
//!   first packet that needs the whole server (chat, commands).
//! - **PX** (serial): those packets and everything their regions received after them, in
//!   arrival order, with access to everything.
//! - **G** (serial): console commands, world time, autosave; players teleported into another
//!   region's loaded cells move there.
//! - **L** (parallel): each region streams chunks, tracks entities, sends movement and light,
//!   and flushes its players' packets.

mod commands;
mod entities;
mod generation;
mod health;
mod interact;
mod movement;
mod persist;
mod players;
mod region;
mod rng;
mod stats;
pub mod testing;

use bytes::Bytes;
use crossbeam_channel::Receiver;
use kiln_link::{ClientInfo, ConnId, JoinInfo, PlayIn, Property, Sink, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_region::{DefaultCells, RegionId, RegionPolicy, Regionizer, Regions, TopologyEvent};
use kiln_world::chunk::Chunk;
use kiln_world::spawn::LoadChunks;
use kiln_world::{Blocks, Cell, CellStore, ChunkPos, ChunkProvider, Dimension, OVERWORLD as OVERWORLD_DIM, Terrain};
use region::{Env, RegionOut, RegionWork};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct SimConfig {
    pub max_players: usize,
    pub view_distance: u8,
    pub simulation_distance: u8,
    /// A vanilla world save to load; a superflat world is used when `None`.
    pub world: Option<std::path::PathBuf>,
    /// Whether players were authenticated with Mojang (sent to clients in Login).
    pub online_mode: bool,
    /// Tick pool: worker count and, for tests, forced strategies and chaos scheduling.
    pub pool: kiln_sched::PoolConfig,
    /// One region per dimension (the vanilla profile) instead of regions around players.
    pub unified_regions: bool,
    /// Vanilla noise terrain for chunks the world does not have (superflat or void otherwise).
    pub noise: Option<NoiseConfig>,
}

/// Vanilla overworld generation: the seed and the vanilla datapack directory (the data
/// generator's output, holding `data/minecraft/worldgen`).
pub struct NoiseConfig {
    pub seed: i64,
    pub datapack: std::path::PathBuf,
    /// Generation threads.
    pub threads: usize,
}

impl SimConfig {
    /// Defaults for a server on this machine: all cores but one tick (at most 7), regions on.
    pub fn new(max_players: usize, view_distance: u8, world: Option<std::path::PathBuf>) -> Self {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            max_players,
            view_distance,
            simulation_distance: view_distance,
            world,
            online_mode: false,
            pool: kiln_sched::PoolConfig::new(cores.saturating_sub(1).clamp(1, 7)),
            unified_regions: false,
            noise: None,
        }
    }
}

const TICK: Duration = Duration::from_millis(50);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(30);
const OVERWORLD: &str = "minecraft:overworld";
const MAX_UNACKED_BATCHES: u32 = 10;
/// Save changed chunks every 5 minutes.
const AUTOSAVE_TICKS: i64 = 6000;
/// Player inventory container slots 36..=44 are the hotbar.
const HOTBAR_START: usize = 36;
const INVENTORY_SLOTS: usize = 46;
/// Chunks loaded or generated per tick at most (requests beyond wait for later ticks).
const CHUNK_LOADS_PER_TICK: usize = 256;

struct Player {
    conn: ConnId,
    name: String,
    uuid: Uuid,
    entity_id: i32,
    properties: Vec<Property>,
    client: ClientInfo,
    game_mode: u8,
    sink: Box<dyn Sink>,
    /// Packets queued this tick; flushed in the egress phase.
    outbox: Vec<Bytes>,
    /// Set once the connection was told to close; the player leaves when it does.
    disconnected: bool,
    /// The region that owns the cell the player stands in (updated in B0).
    region: RegionId,
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
    /// Main slots, equipment and the selected hotbar slot.
    inv: kiln_inventory::PlayerInventory,
    /// Saved inventory entries Kiln could not decode, written back unchanged.
    inv_extra: kiln_inventory::persist::PlayerItemsExtra,
    /// The inventory menu (container 0), always open underneath.
    menu: kiln_inventory::Menu,
    /// A block or entity menu the player has open.
    open_menu: Option<kiln_inventory::Menu>,
    /// Movement packets for this player's viewers.
    tracker: packets::entity::MovementTracker,
    /// Players currently seeing this one (sorted).
    seen_by: Vec<ConnId>,
    /// Section at the last visibility update; `None` forces a re-evaluation.
    section: Option<[i32; 3]>,
    sneaking: bool,
    sprinting: bool,
    /// Shared flags or pose changed since the last broadcast.
    meta_dirty: bool,
    /// Arm swung this tick.
    swung: bool,
    /// Latest tab-completion request, answered once per tick.
    pending_suggestion: Option<(i32, String)>,
    teleport_id: i32,
    respawn: Option<[i32; 3]>,
    /// Saved player data this player was loaded from: tags Kiln does not model are written
    /// back from here.
    saved: kiln_storage::PlayerData,
    /// Position at the start of this tick, for the "moved too quickly" check.
    first_good: [f64; 3],
    /// Move packets with a position received this tick.
    move_packets: u32,
    /// Game time the pending teleport was (re)sent.
    teleport_sent: i64,
    /// Ticks left until movement counts without the client's "loaded" report.
    load_timeout: u32,
    /// A position arrived since the client's last tick end (a second one is a protocol error).
    position_this_tick: bool,
    /// Highest block change sequence to acknowledge in the connection tick (-1: none).
    ack_block_changes: i32,
    /// View distance the sent chunks were last trimmed to.
    applied_view: i32,
    /// The player's random source (item throws), seeded from its UUID.
    rng: rng::Rng,
    health: f32,
    food: i32,
    saturation: f32,
    /// Distance fallen since last on the ground.
    fall_distance: f64,
    /// Flying (creative or spectator), from the client's abilities packet.
    flying: bool,
    /// Dead until the client asks to respawn.
    dead: bool,
    /// Died this tick: viewers see the death animation.
    died: bool,
    /// Damage type of a hit this tick, for viewers' damage effect.
    damaged: Option<i32>,
    death_location: Option<[i32; 3]>,
}

impl Player {
    fn send(&mut self, p: Bytes) {
        self.outbox.push(p);
    }
    fn flush(&mut self) {
        if !self.outbox.is_empty() {
            self.sink.send_batch(std::mem::take(&mut self.outbox));
        }
    }
    fn disconnect(&mut self, reason: &str) {
        self.flush();
        self.sink.disconnect(packets::play_disconnect(reason));
        self.disconnected = true;
    }
    /// Adds a stack to the inventory (`Inventory.add`); returns how many items were taken. The
    /// menus send the changed slots on their next broadcast.
    fn add_to_inventory(&mut self, stack: &mut kiln_item::ItemStack) -> i32 {
        let before = stack.count();
        let infinite = self.game_mode == 1;
        self.inv.add(None, stack, infinite);
        before - stack.count()
    }

    fn player_flags(&self) -> kiln_inventory::PlayerFlags {
        kiln_inventory::PlayerFlags {
            creative: self.game_mode == 1,
            infinite_materials: self.game_mode == 1,
            spectator: self.game_mode == 3,
            dead: false,
            removed: self.disconnected,
        }
    }

    /// Runs `f` on the open menu (or the inventory menu) and carries out its effects: packets
    /// to the client, dropped items to `spawns`.
    fn with_menu<R>(
        &mut self,
        rules: &kiln_inventory::Rules,
        spawns: &mut Vec<entities::Spawn>,
        f: impl FnOnce(&mut kiln_inventory::Menu, Option<&mut kiln_inventory::Menu>, &mut kiln_inventory::Env) -> R,
    ) -> R {
        let mut out = Vec::new();
        let player = self.player_flags();
        let result = {
            let Player { inv, menu, open_menu, .. } = self;
            let mut env = kiln_inventory::Env {
                inventory: inv,
                block: None,
                player,
                rules,
                world: &mut kiln_inventory::NoWorld,
                out: &mut out,
            };
            match open_menu {
                Some(open) => f(open, Some(menu), &mut env),
                None => f(menu, None, &mut env),
            }
        };
        for effect in out {
            if let Some(pkt) = effect.encode() {
                self.send(pkt);
            }
            if let kiln_inventory::Effect::Drop { stack, .. } = effect {
                spawns.push(self.throw(stack));
            }
        }
        result
    }

    /// Drops one item (or the whole stack) from the selected hotbar slot (`ServerPlayer.drop`).
    fn drop_held(&mut self, all: bool) -> Option<entities::Spawn> {
        let slot = &mut self.inv.items[self.inv.selected];
        if slot.is_empty() {
            return None;
        }
        let dropped = if all { std::mem::replace(slot, kiln_item::ItemStack::empty()) } else { slot.split(1) };
        self.inv.times_changed += 1;
        Some(self.throw(dropped))
    }

    /// Item id per inventory menu slot, as the client numbers them (tests and tools).
    fn menu_view(&self) -> Vec<Option<(i32, i32)>> {
        let mut out = vec![None; INVENTORY_SLOTS];
        let view = |s: &kiln_item::ItemStack| (!s.is_empty()).then(|| (s.item(), s.count()));
        for (i, s) in self.inv.items.iter().enumerate() {
            out[if i < 9 { HOTBAR_START + i } else { i }] = view(s);
        }
        out
    }

    /// An item thrown from the eyes in the look direction (`LivingEntity.createItemStackToDrop`
    /// with `throwRandomly` false).
    fn throw(&mut self, stack: kiln_item::ItemStack) -> entities::Spawn {
        let (yaw, pitch) = (self.rot[0].to_radians(), self.rot[1].to_radians());
        let f = 0.3f32;
        let angle = self.rng.next_f32() * std::f32::consts::TAU;
        let spread = 0.02 * self.rng.next_f32();
        let vel = [
            (-yaw.sin() * pitch.cos() * f + angle.cos() * spread) as f64,
            (-pitch.sin() * f + 0.1 + (self.rng.next_f32() - self.rng.next_f32()) * 0.1) as f64,
            (yaw.cos() * pitch.cos() * f + angle.sin() * spread) as f64,
        ];
        let eye_y = self.pos[1] + if self.sneaking { 1.27 } else { 1.62 };
        entities::Spawn {
            kind: &kiln_data::entities::types::ITEM,
            pos: [self.pos[0], eye_y - 0.3, self.pos[2]],
            vel,
            body: entities::Body::Item { stack, pickup_delay: entities::DROP_PICKUP_DELAY },
        }
    }

    /// Moves the player and waits for the client to confirm (`ServerGamePacketListenerImpl.teleport`).
    fn teleport(&mut self, pos: [f64; 3], rot: [f32; 2], now: i64) {
        self.pos = pos;
        self.rot = rot;
        self.teleport_id += 1;
        self.awaiting_teleport = Some(self.teleport_id);
        self.teleport_sent = now;
        self.send(packets::player_position(self.teleport_id, pos, rot[0], rot[1]));
        self.tracker.mark_dirty();
    }
}

/// A dimension's chunks: loaded cells grouped into regions, and where chunks come from.
struct Dim {
    provider: ChunkProvider,
    regions: Regions<Cell, entities::Entities>,
    regionizer: Regionizer,
    /// Chunks loaded for cells without an owner yet; installed once the regionizer ran.
    pending: HashMap<ChunkPos, Chunk>,
    /// Chunks the regions asked for in their last tick: (rank in the player's list, player,
    /// chunk).
    requests: Vec<(u32, ConnId, ChunkPos)>,
    /// Chunks the regions no longer need.
    unloads: Vec<ChunkPos>,
    /// Entities spawned in a serial phase, waiting for ids.
    spawns: Vec<entities::Spawn>,
    /// Cells emptied by unloads; vacated when the regionizer runs, unless refilled first.
    emptied: Vec<kiln_world::CellPos>,
    /// Generation threads, when missing chunks come from an expensive generator.
    generation: Option<generation::GenPool>,
}

/// Serial access that loads chunks on demand: into their region if the cell has an owner,
/// otherwise pending until the next B0 gives the cell one.
impl LoadChunks for Dim {
    fn dimension(&self) -> Dimension {
        self.provider.dimension
    }

    fn load_chunk(&mut self, pos: ChunkPos) -> &mut Chunk {
        if self.regions.chunk(pos).is_some() {
            return self.regions.chunk_mut(pos).unwrap();
        }
        if !self.pending.contains_key(&pos) {
            let chunk = self.provider.load_or_generate(pos);
            if self.install(pos, chunk) {
                return self.regions.chunk_mut(pos).unwrap();
            }
        }
        self.pending.get_mut(&pos).unwrap()
    }
}

impl Dim {
    fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.regions.chunk(pos).is_some() || self.pending.contains_key(&pos)
    }

    /// Puts a loaded chunk in its region, or pending until its cell gets one. Returns whether
    /// it went straight into a region.
    fn install(&mut self, pos: ChunkPos, chunk: Chunk) -> bool {
        if let Some(cell) = self.regions.cell_mut(pos.cell()) {
            cell.insert(pos, chunk);
            return true;
        }
        self.regionizer.push(TopologyEvent::Occupied(pos.cell()));
        self.pending.insert(pos, chunk);
        false
    }

    /// Loads `pos` from storage now, or queues it for generation off the tick thread.
    /// Returns `false` if it could not even be queued (try again next tick).
    fn request(&mut self, pos: ChunkPos) -> bool {
        let Some(pool) = &mut self.generation else {
            self.load_chunk(pos);
            return true;
        };
        if pool.is_queued(pos) {
            return true;
        }
        match self.provider.load(pos) {
            Some(chunk) => {
                self.install(pos, chunk);
                true
            }
            None => pool.request(pos),
        }
    }

    /// Installs the chunks generation finished since the last tick.
    fn install_generated(&mut self) -> usize {
        let Some(pool) = &mut self.generation else { return 0 };
        let done = pool.finished();
        let n = done.len();
        for (pos, chunk) in done {
            // Loaded synchronously in the meantime (a join or teleport needed it).
            if !self.is_loaded(pos) {
                self.install(pos, chunk);
            }
        }
        n
    }

    /// Saves and drops chunks the regions released; cells left empty are vacated.
    fn unload(&mut self, chunks: Vec<ChunkPos>, keep: &HashSet<ChunkPos>) -> usize {
        let mut unloaded = 0;
        for pos in chunks {
            if keep.contains(&pos) {
                continue;
            }
            let stores = self.provider.stores();
            let Some(cell) = self.regions.cell_mut(pos.cell()) else { continue };
            // Without storage, changed chunks stay loaded or the changes would be lost.
            if !stores && cell.chunk(pos).is_some_and(Chunk::modified) {
                continue;
            }
            if let Some(mut chunk) = cell.remove(pos) {
                self.provider.unload(pos, &mut chunk);
                unloaded += 1;
            }
            if cell.is_empty() {
                self.emptied.push(pos.cell());
            }
        }
        unloaded
    }

    /// Runs the regionizer and installs the chunks that were waiting for their cell's owner.
    /// Returns whether the topology changed.
    fn apply_topology(&mut self, tick: u64) -> bool {
        for pos in std::mem::take(&mut self.emptied) {
            if self.regions.cell(pos).is_some_and(Cell::is_empty) {
                self.regionizer.push(TopologyEvent::Vacated(pos));
            }
        }
        let deltas = self.regionizer.apply(&mut self.regions, tick, &mut DefaultCells);
        for (pos, chunk) in std::mem::take(&mut self.pending) {
            match self.regions.cell_mut(pos.cell()) {
                Some(cell) => {
                    cell.insert(pos, chunk);
                }
                None => {
                    self.regionizer.push(TopologyEvent::Occupied(pos.cell()));
                    self.pending.insert(pos, chunk);
                }
            }
        }
        for d in &deltas {
            debug!("regions: {d:?}");
        }
        !deltas.is_empty()
    }
}

pub struct Sim {
    config: SimConfig,
    /// Recipes and item rules from the vanilla datapack.
    rules: std::sync::Arc<kiln_inventory::Rules>,
    dim: Dim,
    pool: kiln_sched::TickPool,
    /// World spawn block; players appear around it.
    spawn: [i32; 3],
    /// Angles players at the world spawn face.
    spawn_rot: [f32; 2],
    storage: Option<persist::Storage>,
    players: HashMap<ConnId, Player>,
    next_entity_id: i32,
    started: Instant,
    stats: stats::TickStats,
    /// World age in ticks.
    game_time: i64,
    /// The overworld clock (time of day).
    day_time: i64,
    overworld_clock: i32,
    commands: commands::CommandState,
}

/// Operator names from `KILN_OPS` (comma separated).
fn ops_from_env() -> HashSet<String> {
    std::env::var("KILN_OPS")
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

pub fn run(config: SimConfig, rx: Receiver<ToSim>) {
    let mut sim = Sim::new(config);
    let mut next_tick = Instant::now();
    let mut inbox = Vec::new();
    loop {
        loop {
            match rx.try_recv() {
                Ok(msg) => inbox.push(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }
        if !sim.step(inbox.drain(..)) {
            return;
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
    pub fn new(config: SimConfig) -> Sim {
        let plains = kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains").expect("plains biome");
        let biome_count = kiln_data::registries::SYNCHRONIZED
            .iter()
            .find(|(r, _)| *r == "minecraft:worldgen/biome")
            .map_or(0, |(_, e)| e.len());
        let generator = config.noise.as_ref().and_then(|n| {
            let pack = kiln_worldgen::Datapack::load(&n.datapack)
                .map_err(|e| warn!("cannot load the datapack at {}: {e}", n.datapack.display()))
                .ok()?;
            let g = kiln_worldgen::generator::Generator::new(&pack, OVERWORLD, OVERWORLD, n.seed)
                .map_err(|e| warn!("cannot set up overworld generation: {e}"))
                .ok()?;
            info!("overworld generation: seed {}, {} threads", n.seed, n.threads);
            Some(kiln_worldgen::world::NoiseChunks::new(std::sync::Arc::new(g)))
        });
        let (provider, spawn) = match &config.world {
            Some(dir) => {
                let source = kiln_storage::AnvilSource::new(dir.join("dimensions/minecraft/overworld/region"));
                let mut provider =
                    ChunkProvider::with_source(OVERWORLD_DIM, Box::new(source), Terrain::Void, plains as u16, biome_count);
                if let Some(g) = generator {
                    provider = provider.with_generator(Box::new(g));
                }
                let spawn = kiln_storage::read_spawn(dir).unwrap_or([0, 64, 0]);
                info!("loaded world {} (spawn {spawn:?})", dir.display());
                (provider, spawn)
            }
            None => match generator {
                Some(g) => {
                    let mut provider = ChunkProvider::flat(OVERWORLD_DIM, plains as u16, biome_count).with_generator(Box::new(g));
                    let spawn = land_spawn(&mut provider);
                    info!("world spawn {spawn:?}");
                    (provider, spawn)
                }
                None => {
                    let provider = ChunkProvider::flat(OVERWORLD_DIM, plains as u16, biome_count);
                    let surface = provider.flat_surface_y() as i32;
                    (provider, [8, surface, 8])
                }
            },
        };
        let policy = if config.unified_regions { RegionPolicy::unified() } else { RegionPolicy::default() };
        let threads = config.noise.as_ref().map_or(1, |n| n.threads);
        let generation = provider.fork_generator().map(|g| generation::GenPool::new(g.as_ref(), OVERWORLD_DIM, threads));
        let storage = config.world.as_deref().map(persist::Storage::open);
        let level = storage.as_ref().filter(|s| s.level.exists()).map(|s| s.level.state());
        info!(
            "tick pool: {} workers, {} regions",
            config.pool.workers,
            if config.unified_regions { "unified" } else { "split" }
        );
        let rules = std::sync::Arc::new(load_rules(config.noise.as_ref().map(|n| n.datapack.as_path())));
        Sim {
            rules,
            pool: kiln_sched::TickPool::with_config(config.pool.clone()),
            config,
            dim: Dim {
                provider,
                regions: Regions::new(),
                regionizer: Regionizer::new(policy),
                pending: HashMap::new(),
                requests: Vec::new(),
                unloads: Vec::new(),
                spawns: Vec::new(),
                emptied: Vec::new(),
                generation,
            },
            spawn,
            spawn_rot: level.as_ref().map_or([0.0; 2], |l| [l.spawn.yaw, l.spawn.pitch]),
            storage,
            players: HashMap::new(),
            next_entity_id: 1,
            started: Instant::now(),
            stats: stats::TickStats::default(),
            game_time: level.as_ref().map_or(0, |l| l.game_time),
            day_time: level.as_ref().map_or(1000, |l| l.day_time),
            overworld_clock: kiln_data::synced_id("minecraft:world_clock", OVERWORLD).expect("overworld clock"),
            commands: commands::CommandState::new(ops_from_env()),
        }
    }

    /// Runs one tick: applies the connection events received since the last tick, then
    /// simulates and flushes every connection. Returns `false` once the simulation stopped.
    pub fn step(&mut self, inbox: impl IntoIterator<Item = ToSim>) -> bool {
        let start = Instant::now();
        let mut mark = start;
        let mut lap = |stats: &mut stats::TickStats, name| {
            let now = Instant::now();
            stats.phase(name, now - mark);
            mark = now;
        };

        // B0: connection events, chunks, topology, joins, membership.
        let (mut packets, mut joins, mut console, mut leaves) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for msg in inbox {
            match msg {
                ToSim::Join(j) => joins.push(j),
                // A connection that joined and left in the same batch never enters the game.
                ToSim::Leave(conn) => match joins.iter().position(|j: &JoinInfo| j.conn == conn) {
                    Some(i) => drop(joins.remove(i)),
                    None => leaves.push(conn),
                },
                ToSim::Packet(conn, pkt) => packets.push((conn, pkt)),
                ToSim::Console(command) => console.push(command),
                ToSim::Shutdown { done } => {
                    self.shut_down();
                    let _ = done.send(());
                    return false;
                }
            }
        }
        self.maintain_chunks();
        let joining: Vec<_> = joins.into_iter().map(|j| (self.joining(j.uuid), j)).collect();
        for (jn, _) in &joining {
            self.dim.load_chunk(player_chunk(jn.pos));
        }
        let changed = self.dim.apply_topology(self.game_time as u64);
        for (jn, j) in joining {
            self.join(j, jn);
        }
        self.update_membership(changed);
        lap(&mut self.stats, "b0");

        // P: region-local packets in parallel.
        let (local, exclusive) = self.route(packets);
        let env = self.env();
        let outs = self.run_regions(local, |w, env| w.apply_packets(env), env);
        for out in outs {
            self.dim.spawns.extend(out.spawns);
            self.announce_deaths(out.deaths);
        }
        lap(&mut self.stats, "packets");

        // PX: chat, commands and what followed them, in arrival order.
        for (conn, pkt) in exclusive {
            self.exclusive_packet(conn, pkt);
        }
        self.answer_suggestions();
        // Leaves last, so the packets a player sent before leaving still apply.
        for conn in leaves {
            self.leave(conn);
        }
        self.materialize_spawns();
        lap(&mut self.stats, "px");

        // G: console, time, autosave.
        for command in console {
            self.run_console_command(command.trim_start_matches('/'));
        }
        self.tick_global();
        // Players teleported in PX or G tick in their destination's region from now on.
        self.settle_teleported();
        lap(&mut self.stats, "global");

        // L: regions tick in parallel.
        let env = self.env();
        let outs = self.run_regions(BTreeMap::new(), |w, env| w.tick(env), env);
        let mut times = [Duration::ZERO; region::SUB_PHASES.len()];
        for out in outs {
            self.dim.requests.extend(out.wanted);
            self.dim.unloads.extend(out.unload);
            self.dim.spawns.extend(out.spawns);
            self.announce_deaths(out.deaths);
            for (t, d) in times.iter_mut().zip(out.times) {
                *t += d;
            }
        }
        self.materialize_spawns();
        lap(&mut self.stats, "regions");
        // CPU time summed over regions (the "regions" phase is wall time).
        for (name, d) in region::SUB_PHASES.iter().zip(times) {
            self.stats.phase(name, d);
        }

        if let Some(report) = self.stats.record(start.elapsed()) {
            info!(
                "{} players, {} regions, {} chunks | {report}",
                self.players.len(),
                self.dim.regions.len(),
                self.dim.regions.loaded_chunks()
            );
            self.commands.last_report = Some(report.to_string());
        }
        if self.commands.stop_requested {
            self.shut_down();
            return false;
        }
        true
    }

    /// Hash of the simulated state (world age and time, blocks of loaded chunks, players), for
    /// determinism tests. Independent of how regions split the world; connection state such
    /// as keep-alives is left out.
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (self.game_time, self.day_time).hash(&mut h);
        self.dim.regions.hash_blocks(&mut h);
        let mut players: Vec<&Player> = self.players.values().collect();
        players.sort_by_key(|p| p.uuid);
        for p in players {
            p.uuid.hash(&mut h);
            p.pos.map(f64::to_bits).hash(&mut h);
            p.rot.map(f32::to_bits).hash(&mut h);
            (p.game_mode, p.inv.selected, p.menu_view(), p.sneaking, p.sprinting).hash(&mut h);
        }
        h.finish()
    }

    pub fn game_time(&self) -> i64 {
        self.game_time
    }

    /// Block state at a position, if its chunk is loaded.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        self.dim.regions.get_block(x, y, z)
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    pub fn region_count(&self) -> usize {
        self.dim.regions.len()
    }

    /// Positions of the non-player entities, by type name (for tests and tools).
    pub fn entities(&self) -> Vec<(&'static str, [f64; 3])> {
        let mut out: Vec<_> = self.dim.regions.iter().flat_map(|r| r.part().list.iter()).map(|e| (e.id, e.kind.name, e.pos)).collect();
        out.sort_by_key(|&(id, ..)| id);
        out.into_iter().map(|(_, k, p)| (k, p)).collect()
    }

    /// A player's health, and whether it is dead (for tests and tools).
    pub fn health(&self, conn: ConnId) -> Option<(f32, bool)> {
        self.players.get(&conn).map(|p| (p.health, p.dead))
    }

    /// A player's inventory as (item id, count) per container slot.
    pub fn inventory(&self, conn: ConnId) -> Option<Vec<Option<(i32, i32)>>> {
        self.players.get(&conn).map(Player::menu_view)
    }

    /// Timing of the last completed statistics window.
    pub fn last_report(&self) -> Option<&str> {
        self.commands.last_report.as_deref()
    }

    fn env(&self) -> Env {
        Env {
            rules: self.rules.clone(),
            min_y: self.dim.provider.dimension.min_y,
            game_time: self.game_time,
            max_view: self.config.view_distance as i32,
            movement_check: self.rule_bool("minecraft:player_movement_check"),
            biome_count: self.dim.provider.biome_count,
            now: Instant::now(),
            keep_alive_id: self.started.elapsed().as_millis() as i64,
        }
    }

    /// Hands every region its cells, its players (sorted by connection) and its packets, and
    /// runs `f` on each in parallel on the tick pool.
    fn run_regions(
        &mut self,
        mut packets: BTreeMap<RegionId, Vec<(ConnId, PlayIn)>>,
        f: impl Fn(&mut RegionWork, &Env) + Sync,
        env: Env,
    ) -> Vec<RegionOut> {
        let mut buckets: BTreeMap<RegionId, Vec<&mut Player>> = BTreeMap::new();
        for p in self.players.values_mut() {
            buckets.entry(p.region).or_default().push(p);
        }
        let (_, regions) = self.dim.regions.split_mut();
        let mut work: Vec<RegionWork> = regions
            .map(|r| {
                let id = r.id();
                let mut players = buckets.remove(&id).unwrap_or_default();
                players.sort_unstable_by_key(|p| p.conn);
                let packets = packets.remove(&id).unwrap_or_default();
                let (cells, entities) = r.cells_and_part_mut();
                RegionWork { cells, entities, players, packets, out: RegionOut::default() }
            })
            .collect();
        debug_assert!(buckets.is_empty(), "players in regions that do not exist");
        // Rough estimate for the pool's start order: players dominate a region's cost.
        let cost = |w: &RegionWork| {
            20_000 + w.players.len() as u64 * 5_000 + w.entities.list.len() as u64 * 500 + w.packets.len() as u64 * 500
        };
        self.pool.run_units(&mut work, cost, |w, _ctx| f(w, &env));
        work.into_iter().map(|w| w.out).collect()
    }

    /// Splits this tick's packets into each region's local stream and the serial PX stream:
    /// a region's stream stops at its first packet that needs the whole server.
    #[allow(clippy::type_complexity)]
    fn route(
        &self,
        packets: Vec<(ConnId, PlayIn)>,
    ) -> (BTreeMap<RegionId, Vec<(ConnId, PlayIn)>>, Vec<(ConnId, PlayIn)>) {
        let (mut local, mut exclusive) = (BTreeMap::<RegionId, Vec<_>>::new(), Vec::new());
        let mut stopped = HashSet::new();
        for (conn, pkt) in packets {
            let Some(p) = self.players.get(&conn) else { continue };
            if stopped.contains(&p.region) || region::is_exclusive(&pkt) {
                stopped.insert(p.region);
                exclusive.push((conn, pkt));
            } else {
                local.entry(p.region).or_default().push((conn, pkt));
            }
        }
        (local, exclusive)
    }

    /// Unloads what the regions released, then loads what they asked for plus every
    /// player's own chunk (so each player stands in an owned cell after the regionizer runs).
    fn maintain_chunks(&mut self) {
        let keep: HashSet<ChunkPos> = self.players.values().map(|p| player_chunk(p.pos)).collect();
        let unloads = std::mem::take(&mut self.dim.unloads);
        let unloaded = self.dim.unload(unloads, &keep);
        if unloaded > 0 {
            debug!("unloaded {unloaded} chunks");
        }
        self.dim.install_generated();
        // Every player's own chunk, uncapped: each player must stand in an owned cell.
        let mut own: Vec<ChunkPos> = keep.into_iter().collect();
        own.sort_unstable();
        for pos in own {
            if !self.dim.is_loaded(pos) {
                self.dim.load_chunk(pos);
            }
        }
        // Then the requests, players interleaved (everyone's nearest chunk first), in an
        // order that does not depend on how regions split them.
        let mut wanted = std::mem::take(&mut self.dim.requests);
        wanted.sort_unstable_by_key(|&(rank, conn, _)| (rank, conn));
        let mut loads = 0;
        for pos in region::merge_requests(wanted.into_iter().map(|(_, _, c)| c)) {
            if self.dim.is_loaded(pos) {
                continue;
            }
            if loads == CHUNK_LOADS_PER_TICK || !self.dim.request(pos) {
                break;
            }
            loads += 1;
        }
    }

    /// Puts every player in the region that owns its cell, and ends pairings between players
    /// that ended up in different regions.
    fn update_membership(&mut self, topology_changed: bool) {
        let mut moved = topology_changed;
        for p in self.players.values_mut() {
            let owner = self.dim.regions.owner(player_chunk(p.pos).cell());
            if let Some(r) = owner.filter(|&r| r != p.region) {
                p.region = r;
                moved = true;
            }
        }
        if moved {
            self.drop_cross_region_pairs();
        }
    }

    /// After the serial phases: players whose chunk is not loaded (teleported into the gap
    /// between regions) get it loaded and a region now, like joining players, so every
    /// player ticks in the region of its position whatever the topology.
    fn settle_teleported(&mut self) {
        let mut stray: Vec<ChunkPos> = self
            .players
            .values()
            .map(|p| player_chunk(p.pos))
            .filter(|&c| self.dim.regions.owner(c.cell()).is_none())
            .collect();
        let changed = !stray.is_empty() && {
            stray.sort_unstable();
            for c in stray {
                self.dim.load_chunk(c);
            }
            self.dim.apply_topology(self.game_time as u64)
        };
        self.update_membership(changed);
    }

    /// Gives the entities spawned since the last call their ids, in an order that does not
    /// depend on the regions, and puts each in the region owning its cell (spawns in unloaded
    /// chunks are dropped, as vanilla would not add them).
    fn materialize_spawns(&mut self) {
        for spawn in entities::canonical(std::mem::take(&mut self.dim.spawns)) {
            let Some(region) = self.dim.regions.at_mut(entities::chunk_of(spawn.pos).cell()) else { continue };
            let id = self.next_entity_id;
            self.next_entity_id += 1;
            region.part_mut().list.push(entities::Entity::new(id, spawn));
        }
    }

    /// Death messages to everyone (`show_death_messages`), in the order the deaths happened.
    fn announce_deaths(&mut self, deaths: Vec<health::Death>) {
        if deaths.is_empty() || !self.rule_bool("minecraft:show_death_messages") {
            return;
        }
        for d in deaths {
            info!("{} died", self.players.get(&d.conn).map_or("?", |p| p.name.as_str()));
            self.broadcast(packets::system_chat(d.message, false));
        }
    }

    /// `PlayerList.respawn` after death: back at the respawn point with full health, the
    /// client rebuilding its world view from a Respawn packet.
    fn respawn(&mut self, conn: ConnId) {
        let Some(p) = self.players.get(&conn) else { return };
        if !p.dead {
            return;
        }
        let pos = match p.respawn {
            Some(r) => kiln_world::spawn::free_spawn_at(&mut self.dim, r),
            None => self.new_player_position(p.uuid),
        };
        let dimension_type =
            kiln_data::synced_id("minecraft:dimension_type", OVERWORLD).expect("overworld dimension type");
        let is_flat = self.config.world.is_none() && self.config.noise.is_none();
        let (spawn, spawn_rot, now) = (self.spawn, self.spawn_rot, self.game_time);
        let time = self.time_packet();
        let rules = self.rules.clone();
        let p = self.players.get_mut(&conn).unwrap();
        let info = packets::player::SpawnInfo {
            dimension_type,
            dimension: OVERWORLD,
            hashed_seed: 0,
            game_mode: p.game_mode,
            previous_game_mode: None,
            is_debug: false,
            is_flat,
            death_location: p.death_location.map(|d| (OVERWORLD, d)),
            portal_cooldown: 0,
            sea_level: 63,
        };
        p.send(packets::player::respawn(&info, packets::player::respawn_keep::NOTHING));
        p.dead = false;
        p.health = health::MAX_HEALTH;
        p.food = 20;
        p.saturation = 5.0;
        p.fall_distance = 0.0;
        p.sent_chunks.clear();
        p.unacked_batches = 0;
        p.teleport(pos, [0.0, 0.0], now);
        p.center = player_chunk(pos);
        p.send(packets::set_chunk_cache_center(p.center.x, p.center.z));
        p.send(packets::set_default_spawn_position(OVERWORLD, spawn, spawn_rot[0], spawn_rot[1]));
        p.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        p.send(time);
        p.send(p.health_packet());
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.open(env));
        self.dim.spawns.extend(spawns);
        // Viewers saw the death: they get the entity again once tracking re-evaluates it.
        self.untrack_everywhere(conn);
    }

    /// A packet from the serial PX stream.
    fn exclusive_packet(&mut self, conn: ConnId, pkt: PlayIn) {
        match pkt {
            PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::PerformRespawn) => self.respawn(conn),
            PlayIn::ChatCommand { command } => self.run_command(conn, &command),
            PlayIn::CommandSuggestion { id, text } => self.suggest(conn, id, text),
            PlayIn::Chat { message } => {
                let Some(p) = self.players.get_mut(&conn) else { return };
                if commands::has_illegal_chars(&message) {
                    p.disconnect("Illegal characters in chat");
                    return;
                }
                info!("<{}> {}", p.name, message);
                let pkt = players::chat_player(&p.name, &message);
                self.broadcast(pkt);
            }
            pkt => {
                let env = self.env();
                let Some(p) = self.players.get_mut(&conn) else { return };
                let (mut changes, mut deaths) = (Vec::new(), Vec::new());
                let mut fx = region::Fx { changes: &mut changes, spawns: &mut self.dim.spawns, deaths: &mut deaths };
                region::local_packet(p, &mut self.dim.regions, &env, pkt, &mut fx);
                region::notify_block_changes(self.players.values_mut(), &changes);
                self.announce_deaths(deaths);
            }
        }
    }

    fn leave(&mut self, conn: ConnId) {
        if let Some(p) = self.players.remove(&conn) {
            self.save_player(&p);
            self.announce_leave(&p, conn);
            self.broadcast_system(yellow(&format!("{} left the game", p.name)));
        }
    }

    fn shut_down(&mut self) {
        for p in self.players.values_mut() {
            p.disconnect("Server closed");
        }
        self.save();
    }

    fn save(&mut self) {
        let start = Instant::now();
        match self.dim.provider.save_all(&mut self.dim.regions) {
            Ok(0) => {}
            Ok(n) => info!("saved {n} chunks in {:.1} ms", start.elapsed().as_secs_f64() * 1e3),
            Err(e) => warn!("saving the world failed: {e}"),
        }
        for p in self.players.values() {
            self.save_player(p);
        }
        self.save_level();
    }

    fn join(&mut self, j: JoinInfo, joining: persist::Joining) {
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        let spawn = joining.pos;
        let [yaw, pitch] = joining.rot;
        let view_distance = (j.client.view_distance as i32).min(self.config.view_distance as i32);
        let move_state = packets::entity::MoveState { pos: spawn, yaw, pitch, head_yaw: yaw, on_ground: true };
        let dimension_type =
            kiln_data::synced_id("minecraft:dimension_type", OVERWORLD).expect("overworld dimension type");
        let region = self.dim.regions.owner(player_chunk(spawn).cell()).expect("spawn chunk loaded");
        let mut player = Player {
            conn: j.conn,
            name: j.name,
            uuid: j.uuid,
            entity_id,
            properties: j.properties,
            client: j.client,
            game_mode: joining.game_mode,
            sink: j.sink,
            outbox: Vec::new(),
            disconnected: false,
            region,
            pos: spawn,
            rot: joining.rot,
            on_ground: true,
            view_distance,
            center: player_chunk(spawn),
            sent_chunks: HashSet::new(),
            awaiting_teleport: Some(1),
            keep_alive: None,
            last_keep_alive: Instant::now(),
            chunks_per_tick: 9.0,
            unacked_batches: 0,
            inv: joining.inv,
            inv_extra: joining.inv_extra,
            menu: kiln_inventory::Menu::inventory(),
            open_menu: None,
            tracker: packets::entity::MovementTracker::new(
                entity_id,
                kiln_data::entities::types::PLAYER.update_interval,
                &move_state,
            ),
            seen_by: Vec::new(),
            section: None,
            sneaking: false,
            sprinting: false,
            meta_dirty: false,
            swung: false,
            pending_suggestion: None,
            teleport_id: 1,
            respawn: joining.respawn,
            saved: joining.saved,
            first_good: spawn,
            move_packets: 0,
            teleport_sent: self.game_time,
            load_timeout: movement::CLIENT_LOADED_TIMEOUT,
            position_this_tick: false,
            ack_block_changes: -1,
            applied_view: view_distance,
            rng: rng::Rng::new(j.uuid.as_u64_pair().0 ^ j.uuid.as_u64_pair().1),
            health: joining.health,
            food: joining.food,
            saturation: joining.saturation,
            fall_distance: 0.0,
            flying: false,
            dead: joining.health <= 0.0,
            died: false,
            damaged: None,
            death_location: None,
        };

        player.send(packets::play_login(&packets::Login {
            entity_id,
            dimensions: &[OVERWORLD],
            max_players: self.config.max_players as i32,
            view_distance: self.config.view_distance as i32,
            simulation_distance: self.config.simulation_distance as i32,
            dimension_type,
            dimension: OVERWORLD,
            game_mode: player.game_mode,
            is_flat: self.config.world.is_none(),
            sea_level: 63,
            online_mode: self.config.online_mode,
        }));
        player.send(packets::player_position(player.teleport_id, spawn, yaw, pitch));
        let [spawn_yaw, spawn_pitch] = self.spawn_rot;
        player.send(packets::set_default_spawn_position(OVERWORLD, self.spawn, spawn_yaw, spawn_pitch));
        player.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        player.send(packets::set_chunk_cache_center(player.center.x, player.center.z));
        player.send(self.time_packet());
        player.send(packets::set_held_slot(player.inv.selected as i32));
        player.send(player.health_packet());
        player.send(kiln_inventory::recipe::sync::update_recipes(&self.rules.recipes));
        let rules = self.rules.clone();
        let mut spawns = Vec::new();
        player.with_menu(&rules, &mut spawns, |menu, _, env| menu.open(env));

        let msg = yellow(&format!("{} joined the game", player.name));
        self.players.insert(j.conn, player);
        self.send_command_tree(j.conn);
        self.announce_join(j.conn);
        self.broadcast_system(msg);
    }

    fn time_packet(&self) -> Bytes {
        let clock = packets::ClockState { clock: self.overworld_clock, time: self.day_time, fraction: 0.0, rate: 1.0 };
        packets::set_time(self.game_time, &[clock])
    }

    /// Changes a block anywhere (serial phases): light follows, players with the chunk hear
    /// about it. Returns whether the chunk was loaded.
    #[allow(dead_code)]
    fn set_block(&mut self, pos: [i32; 3], state: u16) -> bool {
        let mut changes = Vec::new();
        let loaded = region::set_block(&mut self.dim.regions, pos, state, &mut changes);
        region::notify_block_changes(self.players.values_mut(), &changes);
        loaded
    }

    fn broadcast_system(&mut self, text: Tag) {
        self.broadcast(packets::system_chat(text, false));
    }

    /// Encoded once; every player's outbox shares the same bytes. Serial phases only.
    fn broadcast(&mut self, pkt: Bytes) {
        for p in self.players.values_mut() {
            p.send(pkt.clone());
        }
    }

    /// G: world age and time, autosave.
    fn tick_global(&mut self) {
        self.game_time += 1;
        if self.game_time % AUTOSAVE_TICKS == 0 {
            self.save();
        }
        self.day_time += 1;
        if self.game_time % 20 == 0 {
            let pkt = self.time_packet();
            self.broadcast(pkt);
        }
    }
}

/// A world spawn for a new generated world: the first chunk, spiralling out from the origin,
/// whose centre column is dry land. An approximation of vanilla's climate-based
/// `findSpawnPosition`, which targets land biomes near the origin.
fn land_spawn(provider: &mut ChunkProvider) -> [i32; 3] {
    const RADIUS: i32 = 32;
    let mut rings = vec![(0, 0)];
    for r in 1..=RADIUS {
        for i in -r..r {
            rings.extend([(i, -r), (r, i), (-i, r), (-r, -i)]);
        }
    }
    for (cx, cz) in rings {
        let chunk = provider.load_or_generate(ChunkPos::new(cx, cz));
        let top = chunk.column_height(8, 8, kiln_data::block_props::motion_blocking);
        if top > chunk.min_y() && !kiln_data::blocks_types::has_fluid(chunk.get(8, top - 1, 8)) {
            return [cx * 16 + 8, top, cz * 16 + 8];
        }
    }
    [0, 64, 0]
}

/// Recipes from the datapack at `path`, `KILN_DATAPACK` or `work/generated`; none if absent.
fn load_rules(path: Option<&std::path::Path>) -> kiln_inventory::Rules {
    let dir = path.map(std::path::Path::to_path_buf).or_else(|| std::env::var_os("KILN_DATAPACK").map(Into::into));
    let dir = dir.unwrap_or_else(|| "work/generated".into());
    match kiln_inventory::Rules::load(&dir) {
        Ok(rules) => rules,
        Err(e) => {
            warn!("no recipes ({}: {e})", dir.display());
            kiln_inventory::Rules::with_recipes(Default::default())
        }
    }
}

fn player_chunk(pos: [f64; 3]) -> ChunkPos {
    ChunkPos::of_block(pos[0].floor() as i32, pos[2].floor() as i32)
}

fn yellow(s: &str) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(s.into())), ("color".into(), Tag::String("yellow".into()))])
}
