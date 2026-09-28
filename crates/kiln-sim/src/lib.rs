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
//! - **L** (parallel): each region streams chunks, runs block ticks, random ticks and block
//!   events, tracks entities, sends movement and light, and flushes its players' packets.

mod blocks;
mod combat;
mod commands;
mod consume;
mod xp;
mod container;
mod datapacks;
pub mod lobby;
mod digging;
mod effects;
mod entities;
mod generation;
mod hazards;
mod health;
mod mobs;
mod spawner;
mod movement;
mod persist;
mod players;
pub(crate) mod portal;
mod region;
mod rng;
mod sleep;
mod stats;
mod trading;
mod weather;
pub mod testing;
#[cfg(test)]
mod combat_parity;
#[cfg(test)]
mod container_parity;
mod enchant;
#[cfg(test)]
mod enchant_parity;
#[cfg(test)]
mod effect_parity;

use bytes::Bytes;
use crossbeam_channel::Receiver;
use kiln_link::{ClientInfo, ConnId, JoinInfo, PlayIn, Property, Sink, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_region::{DefaultCells, RegionId, RegionPolicy, Regionizer, Regions, TopologyEvent};
use kiln_world::chunk::Chunk;
use kiln_world::spawn::LoadChunks;
use kiln_world::{Blocks, Cell, CellStore, ChunkPos, ChunkProvider, Dimension, Terrain};
use region::{Env, RegionOut, RegionWork};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// A container block entity for tests and tools: (slot, item name, count) of its non-empty slots,
/// and the furnace values (lit time, lit total, cook progress, cook total).
pub type ContainerView = (Vec<(usize, &'static str, i32)>, [i32; 4]);

/// An open menu for tests and tools: its `minecraft:menu` type and (item name, count) per slot.
pub type MenuView = (&'static str, Vec<Option<(&'static str, i32)>>);

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
    /// `require-resource-pack` with a server pack set: declining any pack disconnects.
    pub require_resource_pack: bool,
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
            require_resource_pack: false,
        }
    }
}

const TICK: Duration = Duration::from_millis(50);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(30);
const OVERWORLD: &str = "minecraft:overworld";
/// Index of a dimension in `Sim::dims` (the order of [`DIMENSIONS`]).
pub(crate) type DimId = usize;
pub(crate) const OVERWORLD_ID: DimId = 0;
pub(crate) const NETHER_ID: DimId = 1;
pub(crate) const END_ID: DimId = 2;
/// The levels a server runs, by [`DimId`]: level key (also its dimension type's name) and the
/// biome of chunks nothing generated.
pub(crate) const DIMENSIONS: [(&str, &str); 3] = [
    ("minecraft:overworld", "minecraft:plains"),
    ("minecraft:the_nether", "minecraft:nether_wastes"),
    ("minecraft:the_end", "minecraft:the_end"),
];

/// `sea_level` of each level's noise settings, by [`DimId`].
const SEA_LEVELS: [i32; 3] = [63, 32, 0];

/// A level's directory in a 26.x world save (`dimensions/<namespace>/<path>`).
fn dimension_dir(key: &str) -> String {
    let (ns, path) = key.split_once(':').unwrap_or(("minecraft", key));
    format!("dimensions/{ns}/{path}")
}

/// A level key's [`DimId`].
pub(crate) fn dim_id(key: &str) -> Option<DimId> {
    DIMENSIONS.iter().position(|(k, _)| *k == key)
}
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
    /// The level the player is in.
    dim: DimId,
    /// The region (of the player's level) that owns the cell the player stands in (updated
    /// in B0).
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
    /// What an open merchant screen told its villager, for [`trading::apply_events`].
    merchant_events: Vec<(i32, kiln_inventory::merchant::MerchantEvent)>,
    /// What the open menu is on, the menu counter and the ender chest items.
    containers: container::open::PlayerContainers,
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
    /// The level of `respawn`.
    respawn_dim: DimId,
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
    /// `Entity.random` for enchantment effects (thorns damage), Java-exact.
    entity_rng: kiln_javamath::random::LegacyRandom,
    /// The player's stand-in for the level's random (enchantment requirements, unbreaking);
    /// see [`enchant`].
    level_rng: kiln_javamath::random::LegacyRandom,
    /// Enchantment definitions (the loot data of the enabled datapacks).
    loot: Option<std::sync::Arc<kiln_loot::LootData>>,
    /// `remainingFireTicks`: burning while positive, -20 at rest (see [`hazards`]).
    fire_ticks: i32,
    /// The on-fire shared flag as viewers last got it.
    on_fire_flag: bool,
    /// `getAirSupply`.
    air: i32,
    /// Air supply as the player's client last got it.
    air_sent: i32,
    /// `tickCount`: ticks since joining (infinite effects tick on it).
    tick_count: i32,
    /// Active mob effects by `minecraft:mob_effect` id.
    effects: std::collections::BTreeMap<i32, effects::Effect>,
    /// Effects changed: particles, ambience and the invisible and glowing flags go out.
    effects_dirty: bool,
    /// An effect attribute modifier changed: Update Attributes goes out.
    attributes_dirty: bool,
    /// Shared flags changed in a way the player's own client must see (burning, invisible).
    self_meta_dirty: bool,
    /// Where the last block effects pass left the player (`applyEffectsFromBlocks`).
    block_effects_from: [f64; 3],
    /// Sounds the player made this tick, for its viewers.
    pending_sounds: Vec<Bytes>,
    /// `Level.soundSeedGenerator` stand-in for this player's sounds.
    sound_seed: kiln_javamath::random::LegacyRandom,
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
    /// A hit this tick for viewers' damage effect: damage type, attacker, direct entity.
    damaged: Option<(i32, Option<i32>, Option<i32>)>,
    /// Entity events for viewers this tick (item breaks).
    entity_events: Vec<u8>,
    /// `LivingEntity.damageCooldownTime` and `lastHurt`.
    hurt_cooldown: i32,
    last_hurt: f32,
    /// `getAbsorptionAmount`.
    absorption: f32,
    /// The last player that hurt this one and ticks left of its kill credit.
    kill_credit: Option<(String, i32)>,
    combat: health::CombatTracker,
    /// `attackStrengthTicker`: ticks since the last swing or item change.
    attack_ticker: i32,
    /// Equipment at the last player tick ([`combat::SLOTS`] order): attributes come from it.
    equipment_seen: Vec<kiln_item::ItemStack>,
    /// Equipment as viewers last got it (Set Equipment).
    equipment_sent: Vec<kiln_item::ItemStack>,
    /// The server's view of the player's velocity (knockback builds on it).
    vel: [f64; 3],
    /// `syncVelocity`: a hit this tick; the velocity goes to the client and its viewers.
    sync_velocity: bool,
    /// `lastKnownClientMovement`: the last accepted move, zero after a tick without one.
    known_movement: [f64; 3],
    moved_this_tick: bool,
    death_location: Option<[i32; 3]>,
    /// The level of `death_location`.
    death_dim: DimId,
    /// `FoodData.exhaustionLevel` and `tickTimer`.
    exhaustion: f32,
    food_timer: i32,
    /// `experienceLevel`, `experienceProgress`, `totalExperience`.
    xp_level: i32,
    xp_progress: f32,
    xp_total: i32,
    /// `takeXpDelay`: ticks until the next orb can be taken.
    take_xp_delay: i32,
    /// Experience in the last Set Experience.
    sent_xp: Option<(u32, i32, i32)>,
    /// Health, food and whether saturation was zero in the last Set Health.
    sent_health: Option<(u32, i32, bool)>,
    /// An item being used (eaten).
    using: Option<consume::Using>,
    /// The block being broken in survival.
    digging: Option<digging::Dig>,
    /// A break the client finished before the server's clock agreed.
    delayed_destroy: Option<digging::Dig>,
    /// Cookies and resource pack statuses.
    lobby: lobby::PlayerLobby,
    /// `Entity.portalProcess`: the portal the player stands in and for how long.
    portal: Option<portal::PortalProcess>,
    /// `Entity.portalCooldown`.
    portal_cooldown: i32,
    /// `ServerPlayer.wonGame`: left the End through the exit portal, credits rolling.
    won_game: bool,
    /// `ServerPlayer.seenCredits`.
    seen_credits: bool,
    /// A trip noticed while touching blocks (the End's exit portal), for the serial phase.
    pending_travel: Option<portal::Travel>,
    /// The entity the player rides (see [`entities::ride_players`]).
    vehicle: Option<i32>,
    /// `getLastHurtByMob` and `getLastHurtMob` with the game time (tamed animals take their
    /// owner's side).
    last_hurt_by_mob: Option<(i32, i64)>,
    last_hurt_mob: Option<(i32, i64)>,
    /// Sleeping in a bed and the insomnia statistic.
    sleep: sleep::Sleep,
    /// Woke up this tick: the wake-up animation goes out with the movement.
    woke_up: bool,
    /// `RespawnConfig`: the facing at the respawn point and whether it is forced
    /// (`/spawnpoint`; beds and anchors are not).
    respawn_angle: f32,
    respawn_forced: bool,
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
    /// Disconnects with a text component (network NBT), e.g. a translation.
    fn disconnect_text(&mut self, reason: kiln_proto::nbt::Tag) {
        self.flush();
        self.sink.disconnect(packets::play_disconnect_text(reason));
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
            xp_level: self.xp_level,
            enchantment_seed: self.containers.enchantment_seed,
        }
    }

    /// Runs `f` on the open menu (or the inventory menu) and carries out its effects: packets
    /// to the client, dropped items to `spawns`. A menu on block containers needs the region's
    /// containers ([`Player::with_menu_at`]); here `f` gets the inventory menu instead.
    fn with_menu<R>(
        &mut self,
        rules: &kiln_inventory::Rules,
        spawns: &mut Vec<entities::Spawn>,
        f: impl FnOnce(&mut kiln_inventory::Menu, Option<&mut kiln_inventory::Menu>, &mut kiln_inventory::Env) -> R,
    ) -> R {
        self.with_menu_at(rules, spawns, None, f)
    }

    /// Drops one item (or the whole stack) from the selected hotbar slot (`ServerPlayer.drop`).
    fn drop_held(&mut self, all: bool) -> Option<entities::Spawn> {
        let slot = &mut self.inv.items[self.inv.selected];
        if slot.is_empty() {
            return None;
        }
        let dropped = if all { std::mem::replace(slot, kiln_item::ItemStack::empty()) } else { slot.split(1) };
        self.inv.times_changed += 1;
        // `drop(stack, false, true)`: the thrower is kept.
        let mut spawn = self.throw(dropped);
        if let entities::Body::Item { thrower, .. } = &mut spawn.body {
            *thrower = Some(self.uuid.as_u128());
        }
        Some(spawn)
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
            body: entities::Body::Item { stack, pickup_delay: entities::DROP_PICKUP_DELAY, thrower: None },
        }
    }

    /// Moves the player and waits for the client to confirm (`ServerGamePacketListenerImpl.teleport`).
    fn teleport(&mut self, pos: [f64; 3], rot: [f32; 2], now: i64) {
        self.pos = pos;
        self.rot = rot;
        self.block_effects_from = pos;
        self.teleport_id += 1;
        self.awaiting_teleport = Some(self.teleport_id);
        self.teleport_sent = now;
        self.send(packets::player_position(self.teleport_id, pos, rot[0], rot[1]));
        self.tracker.mark_dirty();
    }
}

/// A dimension's chunks: loaded cells grouped into regions, and where chunks come from.
/// Regions of different dimensions never merge: each dimension has its own regionizer.
struct Dim {
    /// Level key, e.g. `minecraft:the_nether`.
    key: &'static str,
    /// The level's dimension type.
    kind: &'static kiln_data::DimensionType,
    provider: ChunkProvider,
    regions: Regions<Cell, (entities::Entities, blocks::RegionBlocks)>,
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
    /// World age, for the scheduled ticks of chunks that load or unload.
    game_time: i64,
    /// Entity chunks (`entities/`), when the world is saved somewhere.
    entity_store: Option<kiln_storage::EntityStore>,
    /// Saved entities of loaded chunks that Kiln does not simulate (mobs, ...), written back
    /// as they were loaded.
    raw_entities: HashMap<ChunkPos, Vec<Tag>>,
    /// End gateways cooling down after a teleport, until this game time
    /// (`TheEndGatewayBlockEntity.teleportCooldown`; gateways do not tick in Kiln).
    gateway_cooldowns: HashMap<[i32; 3], i64>,
    /// Non-player entities that came through a portal, by UUID, until this game time.
    portal_cooldowns: HashMap<u128, i64>,
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
    fn new(
        key: &'static str,
        provider: ChunkProvider,
        policy: RegionPolicy,
        threads: usize,
        game_time: i64,
        world: Option<&std::path::Path>,
    ) -> Dim {
        let kind = kiln_data::dimension_type(key).expect("vanilla dimension type");
        let generation = provider.fork_generator().map(|g| generation::GenPool::new(g.as_ref(), provider.dimension, threads));
        let entity_store = world.map(|dir| kiln_storage::EntityStore::new(dir.join(dimension_dir(key)).join("entities")));
        Dim {
            key,
            kind,
            provider,
            regions: Regions::new(),
            regionizer: Regionizer::new(policy),
            pending: HashMap::new(),
            requests: Vec::new(),
            unloads: Vec::new(),
            spawns: Vec::new(),
            emptied: Vec::new(),
            generation,
            game_time,
            entity_store,
            raw_entities: HashMap::new(),
            gateway_cooldowns: HashMap::new(),
            portal_cooldowns: HashMap::new(),
        }
    }

    fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.regions.chunk(pos).is_some() || self.pending.contains_key(&pos)
    }

    /// Puts a loaded chunk in its region, or pending until its cell gets one. Returns whether
    /// it went straight into a region.
    fn install(&mut self, pos: ChunkPos, chunk: Chunk) -> bool {
        match self.put(pos, chunk) {
            Ok(()) => true,
            Err(chunk) => {
                self.regionizer.push(TopologyEvent::Occupied(pos.cell()));
                self.pending.insert(pos, chunk);
                false
            }
        }
    }

    /// Puts a chunk into the region owning its cell, whose block machinery takes its
    /// scheduled ticks; gives the chunk back if the cell has no region.
    #[allow(clippy::result_large_err)]
    fn put(&mut self, pos: ChunkPos, mut chunk: Chunk) -> Result<(), Chunk> {
        let Some(region) = self.regions.at_mut(pos.cell()) else { return Err(chunk) };
        let (cells, part) = region.cells_and_part_mut();
        let Some(cell) = cells.get_mut(pos.cell()) else { return Err(chunk) };
        part.1.chunk_loaded(pos, &mut chunk, self.game_time);
        let new = chunk.is_new();
        let generated = std::mem::take(&mut chunk.generated_entities);
        cell.insert(pos, chunk);
        // A generated chunk gets its light from its blocks and loaded neighbours (vanilla's
        // LIGHT status).
        if new {
            kiln_world::light::light_new_chunk(cells, pos);
        }
        self.load_entities(pos);
        self.add_saved_entities(pos, generated);
        Ok(())
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

    /// Saves and drops chunks the regions released; cells left empty are vacated. Returns
    /// the chunks that unloaded.
    fn unload(&mut self, chunks: Vec<ChunkPos>, keep: &HashSet<ChunkPos>) -> Vec<ChunkPos> {
        let mut unloaded = Vec::new();
        for pos in chunks {
            if keep.contains(&pos) {
                continue;
            }
            let stores = self.provider.stores();
            let Some(region) = self.regions.at_mut(pos.cell()) else { continue };
            let (cells, part) = region.cells_and_part_mut();
            let Some(cell) = cells.get_mut(pos.cell()) else { continue };
            // Without storage, changed chunks stay loaded or the changes would be lost.
            if !stores && cell.chunk(pos).is_some_and(Chunk::modified) {
                continue;
            }
            if let Some(mut chunk) = cell.remove(pos) {
                part.1.chunk_unloaded(pos, &mut chunk, self.game_time);
                self.provider.unload(pos, &mut chunk);
                unloaded.push(pos);
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
            if let Err(chunk) = self.put(pos, chunk) {
                self.regionizer.push(TopologyEvent::Occupied(pos.cell()));
                self.pending.insert(pos, chunk);
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
    /// Loot tables from the vanilla datapack (block drops), if it was found.
    loot: Option<std::sync::Arc<kiln_loot::LootData>>,
    /// Biome spawn lists from the vanilla datapack (natural mob spawning).
    spawn_table: Option<std::sync::Arc<spawner::SpawnTable>>,
    /// The levels, by [`DimId`].
    dims: Vec<Dim>,
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
    /// The End's clock (`minecraft:the_end`, the End's `default_clock`).
    end_time: i64,
    end_clock: i32,
    /// The End's exit portal and first gateway were checked this run ([`Sim::prepare_end`]).
    end_prepared: bool,
    commands: commands::CommandState,
    /// The server-wide weather counters (`weather.dat`).
    weather: weather::WeatherData,
    /// Each level's rain and thunder levels, by [`DimId`].
    level_weather: [weather::LevelWeather; 3],
    /// Stand-in for the overworld's level random (weather cycle, `/weather` durations).
    weather_random: kiln_javamath::random::LegacyRandom,
    /// Biome climates from the datapack (precipitation, `isRainingAt`).
    climates: Option<std::sync::Arc<weather::Climates>>,
    /// `BiomeManager`'s obfuscated world seed.
    zoom_seed: i64,
    /// Rate, fraction and pause of the overworld and End clocks.
    clock_runs: [weather::ClockRun; 2],
    /// Each level's sleeping players (`ServerLevel.sleepStatus`).
    sleep_status: [sleep::SleepStatus; 3],
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
        let biome_count = kiln_data::registries::SYNCHRONIZED
            .iter()
            .find(|(r, _)| *r == "minecraft:worldgen/biome")
            .map_or(0, |(_, e)| e.len());
        let pack = config.noise.as_ref().and_then(|n| {
            kiln_worldgen::Datapack::load(&n.datapack)
                .map_err(|e| warn!("cannot load the datapack at {}: {e}", n.datapack.display()))
                .ok()
        });
        // Vanilla generation per level, when a datapack is configured.
        let generator = |id: DimId| -> Option<kiln_worldgen::FullChunks> {
            let (pack, n) = (pack.as_ref()?, config.noise.as_ref()?);
            let key = DIMENSIONS[id].0;
            let world = match id {
                OVERWORLD_ID => kiln_worldgen::Worldgen::overworld(pack, n.seed, true),
                NETHER_ID => kiln_worldgen::Worldgen::nether(pack, n.seed, true),
                _ => kiln_worldgen::Worldgen::end(pack, n.seed, true),
            };
            let world = world.map_err(|e| warn!("cannot set up {key} generation: {e}")).ok()?;
            info!("{key} generation: seed {}, {} threads, features and structures", n.seed, n.threads);
            let pipeline = std::sync::Arc::new(kiln_worldgen::Pipeline::new(std::sync::Arc::new(world)));
            Some(kiln_worldgen::FullChunks::new(pipeline))
        };
        let mut spawn = None;
        let providers: Vec<ChunkProvider> = (0..DIMENSIONS.len())
            .map(|id| {
                let (key, biome_name) = DIMENSIONS[id];
                let kind = kiln_data::dimension_type(key).expect("vanilla dimension type");
                let dimension = Dimension { min_y: kind.min_y, height: kind.height };
                let biome = kiln_data::synced_id("minecraft:worldgen/biome", biome_name).expect("default biome") as u16;
                let generator = generator(id);
                match &config.world {
                    Some(dir) => {
                        let source = kiln_storage::AnvilSource::new(dir.join(dimension_dir(key)).join("region"));
                        let provider = ChunkProvider::with_source(dimension, Box::new(source), Terrain::Void, biome, biome_count);
                        if id == OVERWORLD_ID {
                            let s = kiln_storage::read_spawn(dir).unwrap_or([0, 64, 0]);
                            info!("loaded world {} (spawn {s:?})", dir.display());
                            spawn = Some(s);
                        }
                        match generator {
                            Some(g) => provider.with_generator(Box::new(g)),
                            None => provider,
                        }
                    }
                    None => match generator {
                        Some(g) => {
                            if id == OVERWORLD_ID {
                                let s = initial_spawn(g.pipeline());
                                info!("world spawn {s:?}");
                                spawn = Some(s);
                            }
                            ChunkProvider::flat(dimension, biome, biome_count).with_generator(Box::new(g))
                        }
                        None => {
                            let provider = ChunkProvider::flat(dimension, biome, biome_count);
                            if id == OVERWORLD_ID {
                                spawn = Some([8, provider.flat_surface_y() as i32, 8]);
                            }
                            provider
                        }
                    },
                }
            })
            .collect();
        let spawn = spawn.expect("overworld spawn");
        let policy = if config.unified_regions { RegionPolicy::unified() } else { RegionPolicy::default() };
        let threads = config.noise.as_ref().map_or(1, |n| n.threads);
        let storage = config.world.as_deref().map(persist::Storage::open);
        let level = storage.as_ref().filter(|s| s.level.exists()).map(|s| s.level.state());
        let game_time = level.as_ref().map_or(0, |l| l.game_time);
        let dims = providers
            .into_iter()
            .enumerate()
            .map(|(id, provider)| Dim::new(DIMENSIONS[id].0, provider, policy, threads, game_time, config.world.as_deref()))
            .collect();
        info!(
            "tick pool: {} workers, {} regions",
            config.pool.workers,
            if config.unified_regions { "unified" } else { "split" }
        );
        let datapack = config.noise.as_ref().map(|n| n.datapack.as_path());
        let vanilla_pack = datapack_dir(datapack);
        let rules = std::sync::Arc::new(load_rules(datapack));
        let loot = load_loot(datapack);
        let spawn_table = spawner::SpawnTable::load(&vanilla_pack).map(std::sync::Arc::new);
        let seed = config.noise.as_ref().map_or(0, |n| n.seed);
        let mut sim = Sim {
            rules,
            loot,
            spawn_table,
            pool: kiln_sched::TickPool::with_config(config.pool.clone()),
            config,
            dims,
            spawn,
            spawn_rot: level.as_ref().map_or([0.0; 2], |l| [l.spawn.yaw, l.spawn.pitch]),
            storage,
            players: HashMap::new(),
            next_entity_id: 1,
            started: Instant::now(),
            stats: stats::TickStats::default(),
            game_time,
            day_time: level.as_ref().map_or(1000, |l| l.day_time),
            overworld_clock: kiln_data::synced_id("minecraft:world_clock", OVERWORLD).expect("overworld clock"),
            end_time: 0,
            end_clock: kiln_data::synced_id("minecraft:world_clock", "minecraft:the_end").expect("end clock"),
            end_prepared: false,
            commands: commands::CommandState::new(ops_from_env()),
            weather: Default::default(),
            level_weather: Default::default(),
            weather_random: kiln_javamath::random::LegacyRandom::new(seed ^ 0x7765_6174_6865_72),
            climates: weather::Climates::load(&vanilla_pack).map(std::sync::Arc::new),
            zoom_seed: kiln_worldgen::generator::obfuscate_seed(seed),
            clock_runs: Default::default(),
            sleep_status: Default::default(),
        };
        // Boss bar ids are random per server run, as vanilla draws them from the level random.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        sim.commands.bossbars.seed(now.as_nanos() as u64);
        sim.load_scoreboard();
        sim.load_weather();
        sim.init_packs(vanilla_pack);
        sim
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
        // `/kiln use` clicks, as if their players had sent them.
        packets.splice(0..0, std::mem::take(&mut self.commands.injected));
        self.maintain_chunks();
        let joining: Vec<_> = joins.into_iter().map(|j| (self.joining(j.uuid), j)).collect();
        for (jn, _) in &joining {
            self.dims[jn.dim].load_chunk(player_chunk(jn.pos));
        }
        let changed = self.apply_topology();
        for (jn, j) in joining {
            let in_end = jn.dim == END_ID;
            self.join(j, jn);
            if in_end {
                self.prepare_end();
            }
        }
        self.update_membership(changed);
        lap(&mut self.stats, "b0");

        // P: region-local packets in parallel.
        let (local, exclusive) = self.route(packets);
        let outs = self.run_regions(local, |w, env| w.apply_packets(env));
        for (dim, out) in outs {
            self.dims[dim].spawns.extend(out.spawns);
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
        let outs = self.run_regions(BTreeMap::new(), |w, env| w.tick(env));
        let mut times = [Duration::ZERO; region::SUB_PHASES.len()];
        let mut travels = Vec::new();
        for (dim, out) in outs {
            let d = &mut self.dims[dim];
            d.requests.extend(out.wanted);
            d.unloads.extend(out.unload);
            d.spawns.extend(out.spawns);
            travels.extend(out.portals);
            self.announce_deaths(out.deaths);
            for (t, d) in times.iter_mut().zip(out.times) {
                *t += d;
            }
        }
        self.materialize_spawns();
        // Players whose portal time ran out change level (serially: two levels take part).
        travels.sort_unstable_by_key(|t: &portal::Travel| t.conn);
        for t in travels {
            self.travel(t);
        }
        self.entity_portals();
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
                self.region_count(),
                self.dims.iter().map(|d| d.regions.loaded_chunks()).sum::<usize>()
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
        for (id, d) in self.dims.iter().enumerate() {
            id.hash(&mut h);
            d.regions.hash_blocks(&mut h);
        }
        let mut players: Vec<&Player> = self.players.values().collect();
        players.sort_by_key(|p| p.uuid);
        // Entities (items, mobs, projectiles...) in id order: ids do not depend on the regions.
        let mut ents: Vec<_> = self
            .dims
            .iter()
            .flat_map(|d| d.regions.iter())
            .flat_map(|r| r.part().0.list.iter())
            .map(|e| {
                let mob = e.phys.as_ref().and_then(|p| kiln_entity::mob::data(p).map(|m| (m.health.to_bits(), m.target, m.y_head_rot.to_bits())));
                (e.id, e.kind.id, e.pos.map(f64::to_bits), e.vel.map(f64::to_bits), mob)
            })
            .collect();
        ents.sort_unstable_by_key(|e| e.0);
        ents.hash(&mut h);
        // Container block entities by position.
        let mut containers: Vec<_> = self
            .dims
            .iter()
            .enumerate()
            .flat_map(|(d, dim)| dim.regions.iter().map(move |r| (d, r)))
            .flat_map(|(d, r)| r.part().1.containers.map.iter().map(move |(p, c)| (d, *p, c)))
            .map(|(d, p, c)| {
                let items: Vec<(i32, i32)> = c.items.iter().map(|s| (s.item(), s.count())).collect();
                (d, p, items, c.cooldown, [c.lit_remaining, c.lit_total, c.cook_timer, c.cook_total], c.openers)
            })
            .collect();
        containers.sort_unstable_by_key(|c| (c.0, c.1));
        containers.hash(&mut h);
        for p in players {
            p.uuid.hash(&mut h);
            (p.dim, p.portal_cooldown, p.portal.as_ref().map(|t| t.time)).hash(&mut h);
            p.pos.map(f64::to_bits).hash(&mut h);
            p.rot.map(f32::to_bits).hash(&mut h);
            (p.game_mode, p.inv.selected, p.menu_view(), p.sneaking, p.sprinting).hash(&mut h);
            (p.health.to_bits(), p.dead, p.food, p.saturation.to_bits(), p.exhaustion.to_bits()).hash(&mut h);
            (p.hurt_cooldown, p.last_hurt.to_bits(), p.absorption.to_bits(), p.attack_ticker).hash(&mut h);
            p.vel.map(f64::to_bits).hash(&mut h);
            (p.fire_ticks, p.air, p.tick_count).hash(&mut h);
            for e in p.effects.values() {
                (e.id, e.duration, e.amplifier, e.ambient, e.visible, e.show_icon, e.hidden.is_some()).hash(&mut h);
            }
        }
        h.finish()
    }

    /// The overworld clock (time of day).
    pub fn day_time(&self) -> i64 {
        self.day_time
    }

    /// The overworld's weather: (raining, thundering, rain level, thunder level) with the
    /// levels as `getRainLevel(1)` and `getThunderLevel(1)`.
    pub fn overworld_weather(&self) -> (bool, bool, f32, f32) {
        let w = &self.level_weather[OVERWORLD_ID];
        (self.is_raining(OVERWORLD_ID), self.is_thundering(OVERWORLD_ID), w.rain_level(), w.thunder_level())
    }

    /// The weather counters: (clear time, rain time, thunder time, raining, thundering).
    pub fn weather_counters(&self) -> (i32, i32, i32, bool, bool) {
        let w = &self.weather;
        (w.clear_weather_time, w.rain_time, w.thunder_time, w.raining, w.thundering)
    }

    /// The bed a player sleeps in, and its sleep counter and `time_since_rest`.
    pub fn sleep_state(&self, conn: ConnId) -> Option<(Option<[i32; 3]>, i32, i32)> {
        self.players.get(&conn).map(|p| (p.sleep.pos, p.sleep.counter, p.sleep.time_since_rest))
    }

    /// A player's respawn point and level.
    pub fn respawn_point(&self, conn: ConnId) -> Option<(Option<[i32; 3]>, &'static str)> {
        self.players.get(&conn).map(|p| (p.respawn, DIMENSIONS[p.respawn_dim].0))
    }

    pub fn game_time(&self) -> i64 {
        self.game_time
    }

    /// Block state at a position in the overworld, if its chunk is loaded.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        self.dims[OVERWORLD_ID].regions.get_block(x, y, z)
    }

    /// Block state at a position in the level `dimension` (e.g. `minecraft:the_nether`), if
    /// its chunk is loaded.
    pub fn block_in(&self, dimension: &str, x: i32, y: i32, z: i32) -> Option<u16> {
        self.dims[dim_id(dimension)?].regions.get_block(x, y, z)
    }

    /// The level a player is in, and where (for tests and tools).
    pub fn player_level(&self, conn: ConnId) -> Option<(&'static str, [f64; 3])> {
        self.players.get(&conn).map(|p| (DIMENSIONS[p.dim].0, p.pos))
    }

    /// Loaded chunks per level, in [`DIMENSIONS`] order (for tests and tools).
    pub fn loaded_chunks(&self) -> Vec<usize> {
        self.dims.iter().map(|d| d.regions.loaded_chunks()).collect()
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Regions of all levels.
    pub fn region_count(&self) -> usize {
        self.dims.iter().map(|d| d.regions.len()).sum()
    }

    /// Positions of the non-player entities, by type name (for tests and tools).
    pub fn entities(&self) -> Vec<(&'static str, [f64; 3])> {
        let mut out: Vec<_> = self
            .dims
            .iter()
            .flat_map(|d| d.regions.iter())
            .flat_map(|r| r.part().0.list.iter())
            .map(|e| (e.id, e.kind.name, e.pos))
            .collect();
        out.sort_by_key(|&(id, ..)| id);
        out.into_iter().map(|(_, k, p)| (k, p)).collect()
    }

    /// Positions of the non-player entities of one level, by type name (for tests and tools).
    pub fn entities_in(&self, dimension: &str) -> Vec<(&'static str, [f64; 3])> {
        let Some(d) = dim_id(dimension) else { return Vec::new() };
        let mut out: Vec<_> =
            self.dims[d].regions.iter().flat_map(|r| r.part().0.list.iter()).map(|e| (e.id, e.kind.name, e.pos)).collect();
        out.sort_by_key(|&(id, ..)| id);
        out.into_iter().map(|(_, k, p)| (k, p)).collect()
    }

    /// Mobs: (network id, type name, position, health), in id order (for tests and tools).
    pub fn mobs(&self) -> Vec<(i32, &'static str, [f64; 3], f32)> {
        let mut out: Vec<_> = self
            .dims
            .iter()
            .flat_map(|d| d.regions.iter())
            .flat_map(|r| r.part().0.list.iter())
            .filter_map(|e| e.phys.as_ref().and_then(|p| kiln_entity::mob::data(p).map(|m| (e.id, e.kind.name, e.pos, m.health))))
            .collect();
        out.sort_by_key(|m| m.0);
        out
    }

    /// A player's health, and whether it is dead (for tests and tools).
    pub fn health(&self, conn: ConnId) -> Option<(f32, bool)> {
        self.players.get(&conn).map(|p| (p.health, p.dead))
    }

    /// Durability damage of the item in an inventory menu slot (5-8 armor from the head down,
    /// 9-35 main, 36-44 hotbar, 45 offhand), for tests and tools.
    pub fn item_damage(&self, conn: ConnId, menu_slot: usize) -> Option<i32> {
        use kiln_item::component::EquipmentSlot as S;
        let p = self.players.get(&conn)?;
        let stack = match menu_slot {
            5 => p.inv.equipped(S::Head),
            6 => p.inv.equipped(S::Chest),
            7 => p.inv.equipped(S::Legs),
            8 => p.inv.equipped(S::Feet),
            9..=35 => &p.inv.items[menu_slot],
            36..=44 => &p.inv.items[menu_slot - HOTBAR_START],
            45 => p.inv.equipped(S::OffHand),
            _ => return None,
        };
        (!stack.is_empty()).then(|| stack.damage())
    }

    /// A player's entity id (for tests and tools that attack or interact with it).
    pub fn entity_id(&self, conn: ConnId) -> Option<i32> {
        self.players.get(&conn).map(|p| p.entity_id)
    }

    /// A player's active effects: (effect name, amplifier, duration), in registry order (for
    /// tests and tools).
    pub fn effects(&self, conn: ConnId) -> Option<Vec<(&'static str, i32, i32)>> {
        let p = self.players.get(&conn)?;
        Some(p.effects.values().map(|e| (kiln_item::registry::MOB_EFFECT.name(e.id).unwrap_or("?"), e.amplifier, e.duration)).collect())
    }

    /// A player's remaining fire ticks and air supply (for tests and tools).
    pub fn fire_and_air(&self, conn: ConnId) -> Option<(i32, i32)> {
        self.players.get(&conn).map(|p| (p.fire_ticks, p.air))
    }

    /// A player's experience level, progress and total points (for tests and tools).
    pub fn experience(&self, conn: ConnId) -> Option<(i32, f32, i32)> {
        self.players.get(&conn).map(|p| (p.xp_level, p.xp_progress, p.xp_total))
    }

    /// A player's food level and saturation (for tests and tools).
    pub fn food(&self, conn: ConnId) -> Option<(i32, f32)> {
        self.players.get(&conn).map(|p| (p.food, p.saturation))
    }

    /// A player's inventory as (item id, count) per container slot.
    pub fn inventory(&self, conn: ConnId) -> Option<Vec<Option<(i32, i32)>>> {
        self.players.get(&conn).map(Player::menu_view)
    }

    /// A player's open merchant screen: container id, the villager, and (item id, count) of the
    /// payment and result slots.
    pub fn merchant_screen(&self, conn: ConnId) -> Option<(i32, i32, Vec<Option<(i32, i32)>>)> {
        let menu = self.players.get(&conn)?.open_menu.as_ref()?;
        let st = menu.merchant_state()?;
        Some((menu.container_id, st.merchant, st.items.iter().map(|s| (!s.is_empty()).then(|| (s.item(), s.count()))).collect()))
    }

    /// The contents of the container block entity at an overworld position: (slot, item name,
    /// count) of each non-empty slot, and the furnace values (lit time, lit total, cook
    /// progress, cook total) for furnaces (for tests and tools).
    pub fn container_at(&self, pos: [i32; 3]) -> Option<ContainerView> {
        let region = self.dims[OVERWORLD_ID].regions.at(ChunkPos::of_block(pos[0], pos[2]).cell())?;
        let c = region.part().1.containers.get(kiln_blocks::BlockPos::new(pos[0], pos[1], pos[2]))?;
        let items = c.items.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| (i, s.item_name(), s.count())).collect();
        Some((items, [c.lit_remaining, c.lit_total, c.cook_timer, c.cook_total]))
    }

    /// The stacks of the overworld's item entities (for tests and tools).
    pub fn item_stacks(&self) -> Vec<kiln_item::ItemStack> {
        self.dims[OVERWORLD_ID]
            .regions
            .iter()
            .flat_map(|r| r.part().0.list.iter())
            .filter(|e| !e.removed)
            .filter_map(|e| match e.phys.as_ref().map(|p| &p.kind) {
                Some(kiln_entity::EntityKind::Item(d)) => Some(d.stack.clone()),
                _ => None,
            })
            .collect()
    }

    /// A player's open menu: its `minecraft:menu` type and its slots as (item name, count) (for
    /// tests and tools).
    pub fn open_menu(&self, conn: ConnId) -> Option<MenuView> {
        let p = self.players.get(&conn)?;
        let menu = p.open_menu.as_ref()?;
        let ty = menu.kind.menu_type()?;
        // The menu's own containers (crafting grid, inputs, result), read through a scratch
        // environment when the menu has no block container.
        let own: Option<Vec<kiln_item::ItemStack>> = menu.slots().iter().all(|s| s.source != kiln_inventory::Source::Block).then(|| {
            let (mut inv, mut out) = (p.inv.clone(), Vec::new());
            let env = kiln_inventory::Env {
                inventory: &mut inv,
                block: None,
                player: p.player_flags(),
                rules: &self.rules,
                world: &mut kiln_inventory::NoWorld,
                out: &mut out,
            };
            menu.items(&env)
        });
        let items = menu.slots().iter().enumerate().map(|(i, s)| {
            let stack = match s.source {
                _ if own.is_some() => own.as_ref().and_then(|o| o.get(i).cloned()),
                kiln_inventory::Source::Player => p.inv.items.get(s.index).cloned(),
                kiln_inventory::Source::Block => match &p.containers.open {
                    Some(container::open::OpenBlock::EnderChest { .. }) => p.containers.ender.items.get(s.index).cloned(),
                    Some(container::open::OpenBlock::Containers { first, second }) => {
                        let region = self.dims[p.dim].regions.at(ChunkPos::of_block(first.0.x, first.0.z).cell());
                        region.and_then(|r| {
                            let cs = &r.part().1.containers;
                            let a = cs.get(first.0)?;
                            if s.index < a.items.len() {
                                a.items.get(s.index).cloned()
                            } else {
                                cs.get(second.as_ref()?.0)?.items.get(s.index - a.items.len()).cloned()
                            }
                        })
                    }
                    _ => None,
                },
                _ => None,
            };
            stack.filter(|s| !s.is_empty()).map(|s| (s.item_name(), s.count()))
        });
        Some((ty, items.collect()))
    }

    /// Timing of the last completed statistics window.
    pub fn last_report(&self) -> Option<&str> {
        self.commands.last_report.as_deref()
    }

    fn env(&self, dim: DimId) -> Env {
        Env {
            rules: self.rules.clone(),
            dim,
            min_y: self.dims[dim].provider.dimension.min_y,
            game_time: self.game_time,
            max_view: self.config.view_distance as i32,
            movement_check: self.rule_bool("minecraft:player_movement_check"),
            natural_regen: self.rule_bool("minecraft:natural_health_regeneration"),
            biome_count: self.dims[dim].provider.biome_count,
            now: Instant::now(),
            keep_alive_id: self.started.elapsed().as_millis() as i64,
            portal: self.portal_rules(),
            blocks: self.block_env(dim),
        }
    }

    fn block_env(&self, dim: DimId) -> blocks::BlockEnv {
        let d = self.dims[dim].provider.dimension;
        let kind = self.dims[dim].kind;
        blocks::BlockEnv {
            game_time: self.game_time,
            rules: kiln_blocks::Rules {
                water_source_conversion: self.rule_bool("minecraft:water_source_conversion"),
                lava_source_conversion: self.rule_bool("minecraft:lava_source_conversion"),
                fast_lava: kind.fast_lava,
                water_evaporates: kind.water_evaporates,
                tnt_explodes: self.rule_bool("minecraft:tnt_explodes"),
            },
            dim,
            min_y: d.min_y,
            height: d.height,
            random_tick_speed: self.rule_int("minecraft:random_tick_speed"),
            drops: self.rule_bool("minecraft:block_drops"),
            simulation_distance: self.config.simulation_distance as i32,
            seed: self.config.noise.as_ref().map_or(0, |n| n.seed),
            loot: self.loot.clone(),
            damage: self.damage_rules(),
            mobs: mobs::MobRules {
                day_time: self.day_time,
                sky_darken: weather::sky_darken(dim, self.day_time, &self.level_weather[dim]),
                monsters_burn: mobs::monsters_burn(self.day_time),
                griefing: self.rule_bool("minecraft:mob_griefing"),
                drops: self.rule_bool("minecraft:mob_drops"),
                spawn_mobs: self.rule_bool("minecraft:spawn_mobs"),
                spawn_monsters: self.rule_bool("minecraft:spawn_monsters"),
                cramming: self.rule_int("minecraft:max_entity_cramming"),
                difficulty: self.commands.difficulty as u8,
                spawn_point: self.spawn,
            },
            spawn_table: self.spawn_table.clone(),
            menus: self.rules.clone(),
            weather: weather::WeatherEnv {
                weather: kiln_blocks::weather::Weather {
                    raining: self.is_raining(dim),
                    thundering: self.is_thundering(dim),
                    max_snow_height: self.rule_int("minecraft:max_snow_accumulation_height"),
                },
                climates: self.climates.clone(),
                zoom_seed: self.zoom_seed,
                sea_level: SEA_LEVELS[dim],
            },
        }
    }

    pub(crate) fn damage_rules(&self) -> health::DamageRules {
        health::DamageRules {
            pvp: self.rule_bool("minecraft:pvp"),
            fall: self.rule_bool("minecraft:fall_damage"),
            fire: self.rule_bool("minecraft:fire_damage"),
            freeze: self.rule_bool("minecraft:freeze_damage"),
            drowning: self.rule_bool("minecraft:drowning_damage"),
            difficulty: self.commands.difficulty as u8,
        }
    }

    /// Runs block work at `pos` of level `dim` in the region that owns it (serial phases), then
    /// sends what changed to everyone in the level who has the chunk and carries out the
    /// effects. `None` if the position's cell has no region (its chunk is not loaded).
    pub(crate) fn with_level_in<R>(&mut self, dim: DimId, pos: [i32; 3], f: impl FnOnce(&mut blocks::RegionLevel) -> R) -> Option<R> {
        let env = self.block_env(dim);
        let Sim { dims, players, .. } = self;
        let d = &mut dims[dim];
        let region = d.regions.at_mut(ChunkPos::of_block(pos[0], pos[2]).cell())?;
        let id = region.id();
        let (cells, part) = region.cells_and_part_mut();
        let bodies = blocks::entity_boxes(players.values().filter(|p| p.dim == dim && p.region == id), &part.0);
        let mut out = blocks::BlockOut::default();
        let result = {
            let mut level =
                blocks::RegionLevel { cells: &mut *cells, blocks: &mut part.1, env: &env, out: &mut out, bodies: &bodies, actor: None };
            f(&mut level)
        };
        let mut everyone: Vec<&mut Player> = players.values_mut().filter(|p| p.dim == dim).collect();
        blocks::finish(cells, out, &mut everyone, &mut d.spawns, &env);
        Some(result)
    }

    /// Hands every region of every level its cells, its players (sorted by connection) and
    /// its packets, and runs `f` on each in parallel on the tick pool. Returns each region's
    /// output with its level.
    fn run_regions(
        &mut self,
        mut packets: BTreeMap<(DimId, RegionId), Vec<(ConnId, PlayIn)>>,
        f: impl Fn(&mut RegionWork, &Env) + Sync,
    ) -> Vec<(DimId, RegionOut)> {
        let envs: Vec<Env> = (0..self.dims.len()).map(|d| self.env(d)).collect();
        let mut buckets: BTreeMap<(DimId, RegionId), Vec<&mut Player>> = BTreeMap::new();
        for p in self.players.values_mut() {
            buckets.entry((p.dim, p.region)).or_default().push(p);
        }
        let mut work: Vec<RegionWork> = Vec::new();
        for (dim, d) in self.dims.iter_mut().enumerate() {
            let (_, regions) = d.regions.split_mut();
            work.extend(regions.map(|r| {
                let key = (dim, r.id());
                let mut players = buckets.remove(&key).unwrap_or_default();
                players.sort_unstable_by_key(|p| p.conn);
                let packets = packets.remove(&key).unwrap_or_default();
                let (cells, (entities, blocks)) = r.cells_and_part_mut();
                RegionWork { dim, cells, entities, blocks, players, packets, out: RegionOut::default() }
            }));
        }
        debug_assert!(buckets.is_empty(), "players in regions that do not exist");
        // Rough estimate for the pool's start order: players dominate a region's cost.
        let cost = |w: &RegionWork| {
            20_000 + w.players.len() as u64 * 5_000 + w.entities.list.len() as u64 * 500 + w.packets.len() as u64 * 500
        };
        self.pool.run_units(&mut work, cost, |w, _ctx| f(w, &envs[w.dim]));
        work.into_iter().map(|w| (w.dim, w.out)).collect()
    }

    /// Splits this tick's packets into each region's local stream and the serial PX stream:
    /// a region's stream stops at its first packet that needs the whole server.
    #[allow(clippy::type_complexity)]
    fn route(
        &self,
        packets: Vec<(ConnId, PlayIn)>,
    ) -> (BTreeMap<(DimId, RegionId), Vec<(ConnId, PlayIn)>>, Vec<(ConnId, PlayIn)>) {
        let (mut local, mut exclusive) = (BTreeMap::<(DimId, RegionId), Vec<_>>::new(), Vec::new());
        let mut stopped = HashSet::new();
        for (conn, pkt) in packets {
            let Some(p) = self.players.get(&conn) else { continue };
            let key = (p.dim, p.region);
            if stopped.contains(&key) || region::is_exclusive(&pkt) {
                stopped.insert(key);
                exclusive.push((conn, pkt));
            } else {
                local.entry(key).or_default().push((conn, pkt));
            }
        }
        (local, exclusive)
    }

    /// Per level: unloads what the regions released, then loads what they asked for plus
    /// every player's own chunk (so each player stands in an owned cell after the regionizer
    /// runs).
    fn maintain_chunks(&mut self) {
        // Entities loaded with a chunk have their ids before the chunk can leave again.
        self.materialize_spawns();
        for dim in 0..self.dims.len() {
            let keep: HashSet<ChunkPos> = self.players.values().filter(|p| p.dim == dim).map(|p| player_chunk(p.pos)).collect();
            let unloads = std::mem::take(&mut self.dims[dim].unloads);
            let unloaded = self.dims[dim].unload(unloads, &keep);
            if !unloaded.is_empty() {
                debug!("unloaded {} chunks of {}", unloaded.len(), self.dims[dim].key);
                let owners = self.owner_uuids();
                let gone = self.dims[dim].store_entities(&unloaded, false, &owners);
                self.forget_entities(gone);
            }
            let d = &mut self.dims[dim];
            d.install_generated();
            // Every player's own chunk, uncapped: each player must stand in an owned cell.
            let mut own: Vec<ChunkPos> = keep.into_iter().collect();
            own.sort_unstable();
            for pos in own {
                if !d.is_loaded(pos) {
                    d.load_chunk(pos);
                }
            }
            // Then the requests, players interleaved (everyone's nearest chunk first), in an
            // order that does not depend on how regions split them.
            let mut wanted = std::mem::take(&mut d.requests);
            wanted.sort_unstable_by_key(|&(rank, conn, _)| (rank, conn));
            let mut loads = 0;
            for pos in region::merge_requests(wanted.into_iter().map(|(_, _, c)| c)) {
                if d.is_loaded(pos) {
                    continue;
                }
                if loads == CHUNK_LOADS_PER_TICK || !d.request(pos) {
                    break;
                }
                loads += 1;
            }
        }
    }

    /// Runs every level's regionizer; whether any topology changed.
    fn apply_topology(&mut self) -> bool {
        let tick = self.game_time as u64;
        let mut changed = false;
        for d in &mut self.dims {
            changed |= d.apply_topology(tick);
        }
        changed
    }

    /// Puts every player in the region that owns its cell, and ends pairings between players
    /// that ended up in different regions.
    fn update_membership(&mut self, topology_changed: bool) {
        let mut moved = topology_changed;
        let Sim { players, dims, .. } = self;
        for p in players.values_mut() {
            let owner = dims[p.dim].regions.owner(player_chunk(p.pos).cell());
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
    /// between regions, or into another level) get it loaded and a region now, like joining
    /// players, so every player ticks in the region of its position whatever the topology.
    fn settle_teleported(&mut self) {
        let mut stray: Vec<(DimId, ChunkPos)> = self
            .players
            .values()
            .map(|p| (p.dim, player_chunk(p.pos)))
            .filter(|&(d, c)| self.dims[d].regions.owner(c.cell()).is_none())
            .collect();
        let changed = !stray.is_empty() && {
            stray.sort_unstable();
            for (d, c) in stray {
                self.dims[d].load_chunk(c);
            }
            self.apply_topology()
        };
        self.update_membership(changed);
    }

    /// Gives the entities spawned since the last call their ids, in an order that does not
    /// depend on the regions (level by level), and puts each in the region owning its cell
    /// (spawns in unloaded chunks are dropped, as vanilla would not add them).
    fn materialize_spawns(&mut self) {
        let world_seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        for d in &mut self.dims {
            for spawn in entities::canonical(std::mem::take(&mut d.spawns)) {
                let chunk = entities::chunk_of(spawn.pos);
                let Some(region) = d.regions.at_mut(chunk.cell()) else {
                    // A loaded entity outside its chunk's loaded area goes back to storage.
                    if let entities::Body::Loaded(e) = spawn.body {
                        let tag = kiln_entity::persist::save(&e, &|_| None);
                        d.stash_entities(chunk, vec![tag]);
                    }
                    continue;
                };
                let id = self.next_entity_id;
                self.next_entity_id += 1;
                let uuid = entities::fresh_uuid(world_seed, self.game_time, id);
                region.part_mut().0.list.push(entities::Entity::new(id, uuid, spawn));
            }
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

    /// `PerformRespawn`: after the End's credits (`wonGame`) the player comes back with
    /// everything it had; after death, back at the respawn point with full health.
    fn respawn(&mut self, conn: ConnId) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        if p.won_game {
            p.won_game = false;
            self.respawn_player(conn, true);
        } else if p.dead {
            self.respawn_player(conn, false);
        }
    }

    /// `PlayerList.respawn`: the player appears at its respawn point (or the world spawn),
    /// the client rebuilding its world view from a Respawn packet. `keep_all` (returning from
    /// the End) keeps health, food, effects and the rest; otherwise the player is a fresh one.
    fn respawn_player(&mut self, conn: ConnId, keep_all: bool) {
        let Some(p) = self.players.get(&conn) else { return };
        // `ServerPlayer.findRespawnAndUseSpawnBlock`: beside the bed or charged anchor, at a
        // forced point, or (the block is gone) at the world spawn with a notice.
        let (respawn, respawn_dim, forced, uuid, angle) = (p.respawn, p.respawn_dim, p.respawn_forced, p.uuid, p.respawn_angle);
        let mut rot = [0.0, 0.0];
        let mut lost = false;
        let mut depleted = None;
        let (dim, pos) = match respawn {
            Some(r) => {
                self.dims[respawn_dim].load_chunk(player_chunk([r[0] as f64, r[1] as f64, r[2] as f64]));
                let charges_before = self.dims[respawn_dim].regions.chunk(ChunkPos::of_block(r[0], r[2])).map(|c| c.get((r[0] & 15) as usize, r[1], (r[2] & 15) as usize));
                match self.with_level_in(respawn_dim, r, |level| sleep::find_respawn(level, r, forced)) {
                    Some(sleep::RespawnAt::Block(v, facing)) => {
                        rot = [facing, 0.0];
                        if charges_before.is_some_and(|s| kiln_data::block_logic::block_class(s) == kiln_data::block_logic::BlockClass::RespawnAnchorBlock) && !forced {
                            depleted = Some(r);
                        }
                        (respawn_dim, v)
                    }
                    Some(sleep::RespawnAt::Invalid) => {
                        lost = true;
                        (OVERWORLD_ID, self.new_player_position(uuid))
                    }
                    Some(sleep::RespawnAt::Forced) | None => {
                        rot = [angle, 0.0];
                        (respawn_dim, kiln_world::spawn::free_spawn_at(&mut self.dims[respawn_dim], r))
                    }
                }
            }
            None => (OVERWORLD_ID, self.new_player_position(uuid)),
        };
        let info_packet = {
            let p = &self.players[&conn];
            let keep = if keep_all { packets::player::respawn_keep::ATTRIBUTE_MODIFIERS } else { packets::player::respawn_keep::NOTHING };
            packets::player::respawn(&self.spawn_info(dim, p), keep)
        };
        let (spawn, spawn_rot, now) = (self.spawn, self.spawn_rot, self.game_time);
        let time = self.time_packet();
        let weather = self.weather_packets(dim);
        let rules = self.rules.clone();
        // Viewers in the old level saw the death (or the player walk into the portal): they
        // forget it and get the entity again once tracking re-evaluates it.
        self.untrack_everywhere(conn);
        let p = self.players.get_mut(&conn).unwrap();
        p.send(info_packet);
        self.sleep_status[p.dim].dirty = true;
        self.sleep_status[dim].dirty = true;
        p.dim = dim;
        p.dead = false;
        p.using = None;
        p.fall_distance = 0.0;
        p.portal = None;
        if !keep_all {
            p.health = health::MAX_HEALTH;
            p.food = 20;
            p.saturation = 5.0;
            p.exhaustion = 0.0;
            p.food_timer = 0;
            p.xp_level = 0;
            p.xp_progress = 0.0;
            p.xp_total = 0;
            // A fresh `ServerPlayer`: no cooldowns, credit or tracked hits carry over.
            p.hurt_cooldown = 0;
            p.last_hurt = 0.0;
            p.absorption = 0.0;
            // A fresh `ServerPlayer`: no effects, fire or lost air.
            p.effects.clear();
            p.fire_ticks = -hazards::FIRE_IMMUNE_TICKS;
            p.air = hazards::MAX_AIR;
            p.kill_credit = None;
            p.combat = health::CombatTracker::default();
            p.attack_ticker = 0;
            p.portal_cooldown = 0;
            // Its tick count too (phantoms' insomnia counts from it).
            p.tick_count = 0;
        }
        p.effects_dirty = true;
        p.attributes_dirty = true;
        p.self_meta_dirty = true;
        p.vel = [0.0; 3];
        p.sync_velocity = false;
        p.sent_chunks.clear();
        p.unacked_batches = 0;
        if lost {
            // `NO_RESPAWN_BLOCK_AVAILABLE`: "You have no home bed or charged respawn anchor...".
            p.respawn = None;
            p.send(packets::game_event(0, 0.0));
        }
        if let Some(at) = depleted {
            let id = kiln_data::builtin_id("minecraft:sound_event", "minecraft:block.respawn_anchor.deplete").unwrap_or(0);
            let seed = kiln_javamath::random::RandomSource::next_long(&mut p.sound_seed);
            let at = [at[0] as f64, at[1] as f64, at[2] as f64];
            p.send(packets::world_fx::sound(&packets::world_fx::Sound::Registered(id), packets::world_fx::SoundSource::Blocks, at, 1.0, 1.0, seed));
        }
        p.teleport(pos, rot, now);
        p.block_effects_from = pos;
        p.center = player_chunk(pos);
        p.send(packets::set_chunk_cache_center(p.center.x, p.center.z));
        p.send(packets::set_default_spawn_position(OVERWORLD, spawn, spawn_rot[0], spawn_rot[1]));
        p.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        p.send(time);
        for w in weather {
            p.send(w);
        }
        p.sent_health = None;
        p.sync_health();
        p.sent_xp = None;
        p.sync_experience();
        if keep_all {
            p.send(packets::set_held_slot(p.inv.selected as i32));
            p.send_all_effects();
        }
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.open(env));
        self.dims[dim].spawns.extend(spawns);
        self.place_player(conn);
    }

    /// Loads the chunk a player (just moved to another level) stands in and puts it in the
    /// region owning it, so it ticks there from now on.
    pub(crate) fn place_player(&mut self, conn: ConnId) {
        let Some(p) = self.players.get(&conn) else { return };
        let (dim, chunk) = (p.dim, player_chunk(p.pos));
        self.dims[dim].load_chunk(chunk);
        self.apply_topology();
        if let Some(r) = self.dims[dim].regions.owner(chunk.cell())
            && let Some(p) = self.players.get_mut(&conn)
        {
            p.region = r;
        }
        self.update_membership(true);
    }

    /// A packet from the serial PX stream.
    fn exclusive_packet(&mut self, conn: ConnId, pkt: PlayIn) {
        match pkt {
            PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::PerformRespawn) => self.respawn(conn),
            PlayIn::ChatCommand { command } => self.run_command(conn, &command),
            PlayIn::CommandSuggestion { id, text } => self.suggest(conn, id, text),
            PlayIn::ResourcePack { id, action } => self.resource_pack_response(conn, id, action),
            PlayIn::CookieResponse(response) => self.cookie_response(conn, response),
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
            // A region packet queued behind a serial one: it runs here, in the player's region.
            pkt => {
                let Some((dim, id)) = self.players.get(&conn).map(|p| (p.dim, p.region)) else { return };
                let env = self.env(dim);
                let d = &mut self.dims[dim];
                let Some(region) = d.regions.get_mut(id) else { return };
                let (cells, part) = region.cells_and_part_mut();
                let bodies = blocks::entity_boxes(self.players.values().filter(|p| p.dim == dim && p.region == id), &part.0);
                let (mut out, mut deaths) = (blocks::BlockOut::default(), Vec::new());
                let p = self.players.get_mut(&conn).unwrap();
                let mut world = region::World { cells: &mut *cells, blocks: &mut part.1 };
                let mut fx = region::Fx { blocks: &mut out, bodies: &bodies, spawns: &mut d.spawns, deaths: &mut deaths };
                region::local_packet(p, &mut world, &env, pkt, &mut fx);
                let mut everyone: Vec<&mut Player> = self.players.values_mut().filter(|p| p.dim == dim).collect();
                blocks::finish(cells, out, &mut everyone, &mut d.spawns, &env.blocks);
                self.announce_deaths(deaths);
            }
        }
    }

    fn leave(&mut self, conn: ConnId) {
        // `ServerPlayer.disconnect`: a sleeper gets out of bed first.
        if let Some(mut p) = self.players.remove(&conn) {
            if p.sleep.pos.is_some() {
                let (dim, pos) = (p.dim, p.pos.map(|c| c.floor() as i32));
                self.with_level_in(dim, pos, |level| sleep::stop_sleep_in_bed(&mut p, level, true, false));
            }
            self.sleep_status[p.dim].dirty = true;
            self.commands.bossbars.player_left(p.uuid);
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
        // Entities waiting for their ids are saved with the rest.
        self.materialize_spawns();
        // Scheduled ticks and moving pistons go onto their chunks first.
        let owners = self.owner_uuids();
        for dim in 0..self.dims.len() {
            let d = &mut self.dims[dim];
            for r in d.regions.iter_mut() {
                let (cells, part) = r.cells_and_part_mut();
                for (cell_pos, cell) in cells.iter_mut() {
                    for (pos, chunk) in cell.chunks_mut(cell_pos) {
                        part.1.store(pos, chunk, self.game_time);
                    }
                }
            }
            match d.provider.save_all(&mut d.regions) {
                Ok(0) => {}
                Ok(n) => info!("saved {n} chunks of {} in {:.1} ms", d.key, start.elapsed().as_secs_f64() * 1e3),
                Err(e) => warn!("saving {} failed: {e}", d.key),
            }
            let gone = d.store_entities(&[], true, &owners);
            self.forget_entities(gone);
            match self.dims[dim].flush_entities() {
                Ok(0) => {}
                Ok(n) => debug!("saved {n} entity chunks"),
                Err(e) => warn!("saving entities failed: {e}"),
            }
        }
        for p in self.players.values() {
            self.save_player(p);
        }
        self.save_level();
        self.save_weather();
        self.save_scoreboard();
        self.save_timers();
    }

    fn join(&mut self, j: JoinInfo, joining: persist::Joining) {
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        let spawn = joining.pos;
        let [yaw, pitch] = joining.rot;
        let view_distance = (j.client.view_distance as i32).min(self.config.view_distance as i32);
        let move_state = packets::entity::MoveState { pos: spawn, yaw, pitch, head_yaw: yaw, on_ground: true };
        let dim = joining.dim;
        let dimension_type = kiln_data::synced_id("minecraft:dimension_type", DIMENSIONS[dim].0).expect("dimension type");
        let region = self.dims[dim].regions.owner(player_chunk(spawn).cell()).expect("spawn chunk loaded");
        let mut player = Player {
            dim,
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
            merchant_events: Vec::new(),
            containers: container::open::PlayerContainers::load(joining.saved.raw()),
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
            respawn_dim: joining.respawn_dim,
            saved: joining.saved,
            first_good: spawn,
            move_packets: 0,
            teleport_sent: self.game_time,
            load_timeout: movement::CLIENT_LOADED_TIMEOUT,
            position_this_tick: false,
            ack_block_changes: -1,
            applied_view: view_distance,
            rng: rng::Rng::new(j.uuid.as_u64_pair().0 ^ j.uuid.as_u64_pair().1),
            entity_rng: kiln_javamath::random::LegacyRandom::new(j.uuid.as_u64_pair().0 as i64),
            level_rng: kiln_javamath::random::LegacyRandom::new(j.uuid.as_u64_pair().1 as i64),
            loot: self.loot.clone(),
            fire_ticks: joining.fire_ticks,
            on_fire_flag: false,
            air: joining.air,
            air_sent: hazards::MAX_AIR,
            tick_count: 0,
            effects: joining.effects,
            effects_dirty: true,
            attributes_dirty: true,
            self_meta_dirty: true,
            block_effects_from: spawn,
            pending_sounds: Vec::new(),
            sound_seed: kiln_javamath::random::LegacyRandom::new(!(j.uuid.as_u64_pair().0 as i64)),
            health: joining.health,
            food: joining.food,
            saturation: joining.saturation,
            fall_distance: 0.0,
            flying: false,
            dead: joining.health <= 0.0,
            died: false,
            damaged: None,
            entity_events: Vec::new(),
            hurt_cooldown: 0,
            last_hurt: 0.0,
            absorption: joining.absorption,
            kill_credit: None,
            combat: health::CombatTracker::default(),
            attack_ticker: 0,
            equipment_seen: vec![kiln_item::ItemStack::empty(); combat::SLOTS.len()],
            equipment_sent: vec![kiln_item::ItemStack::empty(); combat::SLOTS.len()],
            vel: [0.0; 3],
            sync_velocity: false,
            known_movement: [0.0; 3],
            moved_this_tick: false,
            death_location: None,
            death_dim: OVERWORLD_ID,
            exhaustion: joining.exhaustion,
            food_timer: joining.food_timer,
            xp_level: joining.xp_level,
            xp_progress: joining.xp_progress,
            xp_total: joining.xp_total,
            take_xp_delay: 0,
            sent_xp: None,
            sent_health: None,
            using: None,
            digging: None,
            delayed_destroy: None,
            lobby: lobby::PlayerLobby::default(),
            portal: None,
            portal_cooldown: joining.portal_cooldown,
            won_game: false,
            seen_credits: joining.seen_credits,
            pending_travel: None,
            vehicle: None,
            last_hurt_by_mob: None,
            last_hurt_mob: None,
            sleep: sleep::Sleep { time_since_rest: joining.time_since_rest, ..Default::default() },
            woke_up: false,
            respawn_angle: joining.respawn_angle,
            respawn_forced: joining.respawn_forced,
        };

        player.send(packets::play_login(&packets::Login {
            entity_id,
            dimensions: &DIMENSIONS.map(|(k, _)| k),
            max_players: self.config.max_players as i32,
            view_distance: self.config.view_distance as i32,
            simulation_distance: self.config.simulation_distance as i32,
            dimension_type,
            dimension: DIMENSIONS[dim].0,
            game_mode: player.game_mode,
            is_flat: self.is_flat(dim),
            sea_level: SEA_LEVELS[dim],
            online_mode: self.config.online_mode,
        }));
        player.send(packets::player_position(player.teleport_id, spawn, yaw, pitch));
        let [spawn_yaw, spawn_pitch] = self.spawn_rot;
        player.send(packets::set_default_spawn_position(OVERWORLD, self.spawn, spawn_yaw, spawn_pitch));
        player.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        player.send(packets::set_chunk_cache_center(player.center.x, player.center.z));
        player.send(self.time_packet());
        // `PlayerList.sendLevelInfo`: the weather of the player's level.
        for pkt in self.weather_packets(player.dim) {
            player.send(pkt);
        }
        player.send(packets::set_held_slot(player.inv.selected as i32));
        player.sync_health();
        // `PlayerList.placeNewPlayer`: the saved effects.
        player.send_all_effects();
        player.send(kiln_inventory::recipe::sync::update_recipes(&self.rules.recipes));
        let rules = self.rules.clone();
        let mut spawns = Vec::new();
        player.with_menu(&rules, &mut spawns, |menu, _, env| menu.open(env));

        let msg = yellow(&format!("{} joined the game", player.name));
        let uuid = player.uuid;
        for pkt in self.commands.scoreboard.join_packets() {
            player.send(pkt);
        }
        self.sleep_status[player.dim].dirty = true;
        self.players.insert(j.conn, player);
        self.send_command_tree(j.conn);
        self.announce_join(j.conn);
        self.broadcast_system(msg);
        self.commands.bossbars.player_joined(uuid);
        self.flush_scoreboard();
    }

    /// `ServerLevel.isFlat` (a superflat generator): Kiln's test worlds' overworld.
    fn is_flat(&self, dim: DimId) -> bool {
        dim == OVERWORLD_ID && self.config.world.is_none() && self.config.noise.is_none()
    }

    /// `ServerPlayer.createCommonSpawnInfo` for the level `dim`.
    pub(crate) fn spawn_info(&self, dim: DimId, p: &Player) -> packets::player::SpawnInfo<'static> {
        let key = DIMENSIONS[dim].0;
        packets::player::SpawnInfo {
            dimension_type: kiln_data::synced_id("minecraft:dimension_type", key).expect("dimension type"),
            dimension: key,
            hashed_seed: 0,
            game_mode: p.game_mode,
            previous_game_mode: None,
            is_debug: false,
            is_flat: self.is_flat(dim),
            death_location: p.death_location.map(|d| (DIMENSIONS[p.death_dim].0, d)),
            portal_cooldown: p.portal_cooldown,
            sea_level: SEA_LEVELS[dim],
        }
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
        for d in &mut self.dims {
            d.game_time = self.game_time;
        }
        self.tick_functions();
        self.tick_clocks();
        if self.game_time % 20 == 0 {
            let pkt = self.time_packet();
            self.broadcast(pkt);
        }
        // The levels' `tick`: the weather, sleeping, then (in the regions) the blocks.
        self.update_sleeping();
        self.tick_weather();
        self.tick_sleep();
        if self.game_time % AUTOSAVE_TICKS == 0 {
            self.save();
        }
    }
}

/// The world spawn of a new generated world (`MinecraftServer.setInitialSpawn`): the
/// generator's origin chunk (`NoiseSpawnFinder`), then the first standable column spiralling
/// over the chunks around it.
fn initial_spawn(pipeline: &kiln_worldgen::Pipeline) -> [i32; 3] {
    let mut gs = kiln_worldgen::GenScratch::default();
    let origin = pipeline.world().generator.spawn_origin(&mut gs);
    let p = kiln_worldgen::spawn::initial_spawn(origin, |x, z| kiln_worldgen::spawn::spawn_pos_in_chunk(&pipeline.full(&mut gs, x, z)));
    [p.x, p.y, p.z]
}

/// The built-in data: the datapack at `path`, `KILN_DATAPACK` or `work/generated`.
pub(crate) fn datapack_dir(path: Option<&std::path::Path>) -> std::path::PathBuf {
    let dir = path.map(std::path::Path::to_path_buf).or_else(|| std::env::var_os("KILN_DATAPACK").map(Into::into));
    dir.unwrap_or_else(|| "work/generated".into())
}

/// Recipes from the datapack at `path`, `KILN_DATAPACK` or `work/generated`; none if absent.
fn load_rules(path: Option<&std::path::Path>) -> kiln_inventory::Rules {
    let dir = datapack_dir(path);
    match kiln_inventory::Rules::load(&dir) {
        Ok(rules) => rules,
        Err(e) => {
            warn!("no recipes ({}: {e})", dir.display());
            kiln_inventory::Rules::with_recipes(Default::default())
        }
    }
}

/// Loot tables from the datapack at `path`, `KILN_DATAPACK` or `work/generated`; none if absent
/// (blocks then drop their own item).
fn load_loot(path: Option<&std::path::Path>) -> Option<std::sync::Arc<kiln_loot::LootData>> {
    let dir = path.map(std::path::Path::to_path_buf).or_else(|| std::env::var_os("KILN_DATAPACK").map(Into::into));
    let dir = dir.unwrap_or_else(|| "work/generated".into());
    if !dir.join("data").is_dir() {
        warn!("no loot tables ({} has no data/)", dir.display());
        return None;
    }
    match kiln_loot::LootData::load_lenient(&dir) {
        Ok(data) => {
            for e in data.errors.iter().take(10) {
                warn!("loot: {e}");
            }
            info!("loot: {} tables", data.table_ids().len());
            Some(std::sync::Arc::new(data))
        }
        Err(e) => {
            warn!("no loot tables ({}: {e})", dir.display());
            None
        }
    }
}

fn player_chunk(pos: [f64; 3]) -> ChunkPos {
    ChunkPos::of_block(pos[0].floor() as i32, pos[2].floor() as i32)
}

fn yellow(s: &str) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(s.into())), ("color".into(), Tag::String("yellow".into()))])
}
