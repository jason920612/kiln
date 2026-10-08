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

mod advancements;
mod blocks;
mod combat;
mod melee;
mod command_data;
mod commands;
mod consume;
mod buckets;
mod use_item;
mod ranged;
mod spear;
mod crossbow;
mod trident;
mod shield;
mod tools;
mod jukebox;
mod golems;
mod glide;
mod slide;
mod end_eye;
mod place_gen;
mod firework;
mod boats;
mod carts;
mod stacks;

mod xp;
mod container;
mod datapacks;
mod tags;
mod zip_pack;
pub mod lobby;
mod diag;
mod digging;
mod dragon_fight;
mod effects;
mod fall;
mod phantom;
mod freeze;
mod entities;
mod entity_world;
mod fishing;
mod gametest;
mod profiles;
mod generation;
mod chunkstats;
mod golem;
mod independent;
pub use generation::generation_totals;
pub use independent::{InjectedDelay, ScheduleMode};
mod hazards;
mod health;
mod mobs;
mod spawner;
mod movement;
mod persist;
mod players;
mod poi;
mod raid;
pub(crate) mod player_stats;
mod recipe_book;
mod plugins;
pub use kiln_plugin_host::ExecMode as PluginMode;
pub use plugins::PluginSettings;
pub(crate) mod portal;
mod region;
mod rng;
mod sculk;
mod heart;
mod mob_spawner;
mod structure_spawns;
mod sleep;
mod stats;
mod trading;
mod leash;
mod trader;
mod waypoints;
mod weather;
mod world_state;
mod wither;
pub mod testing;
#[cfg(test)]
mod combat_parity;
#[cfg(test)]
mod spear_parity;
#[cfg(test)]
mod melee_parity;
mod shoulder;
#[cfg(test)]
mod container_parity;
#[cfg(test)]
mod sculk_parity;
mod enchant;
mod equip;
mod signs;
mod books;
mod pick;
#[cfg(test)]
mod enchant_parity;
#[cfg(test)]
mod effect_parity;
#[cfg(test)]
mod weather_parity;
#[cfg(test)]
mod item_parity;
#[cfg(test)]
mod interact_parity;

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

/// Maps and sets of small keys (connection ids, chunk positions, statistics) looked up per
/// player and tick: a multiplicative hash instead of SipHash. Nothing iterates them in hash
/// order where it could show.
pub(crate) type FastHash = std::hash::BuildHasherDefault<kiln_entity::memory::FastHasher>;
pub(crate) type FastMap<K, V> = HashMap<K, V, FastHash>;
pub(crate) type FastSet<K> = HashSet<K, FastHash>;
use uuid::Uuid;

/// A container block entity for tests and tools: (slot, item name, count) of its non-empty slots,
/// and the furnace values (lit time, lit total, cook progress, cook total).
pub type ContainerView = (Vec<(usize, &'static str, i32)>, [i32; 4]);

/// An open menu for tests and tools: its `minecraft:menu` type and (item name, count) per slot.
pub type MenuView = (&'static str, Vec<Option<(&'static str, i32)>>);

/// A mob's combat state (for tests and tools, see [`Sim::mob_state`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MobState {
    pub health: f32,
    pub alive: bool,
    pub delta: [f64; 3],
    pub fire_ticks: i32,
    pub hurt_time: i32,
    pub damage_cooldown: i32,
    pub last_hurt: f32,
    pub absorption: f32,
    /// Durability damage per slot (feet, legs, chest, head, main hand, off hand, body).
    pub equipment_damage: [Option<i32>; 7],
    /// (effect name, amplifier, duration).
    pub effects: Vec<(&'static str, i32, i32)>,
    pub on_ground: bool,
    pub vehicle: Option<&'static str>,
    pub pos: [f64; 3],
}

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
    /// WASM plugins to load.
    pub plugins: Option<PluginSettings>,
    /// Lockstep (the default: every region ticks every tick, deterministic) or independent
    /// (a region too slow for the tick leaves the lockstep and ticks on its own; see
    /// [`independent`](crate::independent)).
    pub schedule: ScheduleMode,
    /// Tests: a region holding this column sleeps in each of its ticks.
    pub inject_delay: Option<InjectedDelay>,
    /// Storage format of a new world (an existing world keeps its own: Anvil unless marked
    /// native, see `kiln_storage::WorldFormat`).
    pub world_format: kiln_storage::WorldFormat,
    /// Whitelist and ban lists, shared with the login checks.
    pub access: kiln_link::access::SharedAccess,
    /// Keep-alives every 15 s of wall-clock time; `false` sends none (replays and
    /// determinism tests, whose packet streams must not depend on how fast they run).
    pub keep_alive: bool,
    /// How a crowded region's entities tick ([`EntityTicking`]). Serial (vanilla's order) by
    /// default; islands and tiles are faster approximations, opt-in until approved.
    pub entity_ticking: EntityTicking,
    /// Serial entity turns are first tried side by side against the phase's start and kept when
    /// nothing they read changed before their turn (`entities/spec.rs`; the same result as
    /// running them in order). On by default; `KILN_SPECULATE=0` turns it off.
    pub speculate: bool,
    /// The locator bar takes the movers' turns every this many ticks (1: every tick, as
    /// vanilla; more sends fewer, coarser waypoint updates).
    pub locator_interval: u32,
    /// How long the idle workers keep spinning from the start of each tick (`TickPool::
    /// prewake`; zero: they park between windows and pay the wake-up).
    pub prewake: Duration,
    /// Where the data packs publish the feature flags and tags that logins send.
    pub data_sync: std::sync::Arc<kiln_link::DataSync>,
    /// Looks game profiles up for `fetchprofile` (the session service); without one only
    /// online players and offline names resolve.
    pub profile_lookup: Option<std::sync::Arc<dyn kiln_link::ProfileLookup>>,
    /// The simulation's own inbox, for answers that arrive from other threads.
    pub replies: Option<crossbeam_channel::Sender<ToSim>>,
    /// The difficulty the server starts in (`KILN_DIFFICULTY`, 0 peaceful .. 3 hard), as the
    /// dedicated server's `difficulty` property, which it applies over the save's at every start.
    /// Without it a world starts in its saved difficulty, a new one in normal.
    pub difficulty: Option<u8>,
}

/// How a region with many entities ticks them (`entities/islands.rs`). Every choice is
/// deterministic and independent of the workers and the regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityTicking {
    /// One after another in list order, as vanilla.
    Serial,
    /// Groups more than 24 blocks apart side by side (the same result as serial while they do
    /// not meet).
    Islands,
    /// Islands, and 16-block tiles in nine passes when the islands join into one.
    Tiles,
}

impl EntityTicking {
    /// `serial`, `islands` or `tiles`.
    pub fn parse(s: &str) -> Option<EntityTicking> {
        match s {
            "serial" => Some(EntityTicking::Serial),
            "islands" => Some(EntityTicking::Islands),
            "tiles" => Some(EntityTicking::Tiles),
            _ => None,
        }
    }
}

/// Vanilla overworld generation: the seed and the vanilla datapack directory (the data
/// generator's output, holding `data/minecraft/worldgen`).
pub struct NoiseConfig {
    pub seed: i64,
    pub datapack: std::path::PathBuf,
    /// Generation threads.
    pub threads: usize,
}

/// The tick pool's size on a machine with `cores` logical cores: it follows the machine, and
/// leaves a quarter of the cores (at least two) to the rest of the machine (the network,
/// generation and storage threads, and other programs running there): 16 cores tick on 12, 8 on
/// 6, 4 on 2. `KILN_TICK_THREADS` overrides. Idle workers park, so a light tick uses fewer.
pub fn default_workers(cores: usize) -> usize {
    cores.saturating_sub((cores / 4).max(2)).max(1)
}

impl SimConfig {
    /// Defaults for a server on this machine: [`default_workers`] tick, regions on.
    pub fn new(max_players: usize, view_distance: u8, world: Option<std::path::PathBuf>) -> Self {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            max_players,
            view_distance,
            simulation_distance: view_distance,
            world,
            online_mode: false,
            pool: {
                let mut pool = kiln_sched::PoolConfig::new(default_workers(cores));
                // Normal priority: the server shares the machine with its other programs.
                // `KILN_TICK_PRIORITY` raises it (1, 2: above normal, highest; wp40 measured the
                // 1,000-player p99 9.6 -> 6.6 ms at 2 on a busy desktop).
                pool.priority = std::env::var("KILN_TICK_PRIORITY").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
                pool
            },
            unified_regions: false,
            noise: None,
            require_resource_pack: false,
            plugins: None,
            schedule: ScheduleMode::Lockstep,
            inject_delay: None,
            world_format: kiln_storage::WorldFormat::Anvil,
            access: kiln_link::access::AccessLists::new(None).shared(),
            keep_alive: true,
            entity_ticking: EntityTicking::Serial,
            speculate: std::env::var("KILN_SPECULATE").map_or(true, |v| v != "0"),
            locator_interval: 1,
            prewake: Duration::ZERO,
            data_sync: Default::default(),
            profile_lookup: None,
            replies: None,
            difficulty: None,
        }
    }
}

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
/// Tick time spent installing generated chunks per level at most (the chunks players stand in
/// do not wait); the rest are installed in the next ticks.
const INSTALL_BUDGET: Duration = Duration::from_millis(3);

struct Player {
    conn: ConnId,
    name: String,
    uuid: Uuid,
    entity_id: i32,
    properties: Vec<Property>,
    client: ClientInfo,
    game_mode: u8,
    sink: Box<dyn Sink>,
    /// The client's address, for `/ban-ip`.
    address: Option<std::net::IpAddr>,
    /// `ServerPlayer.lastActionTime`, for `/setidletimeout`.
    last_action: Instant,
    /// `ServerPlayer.postEffects` and whether the client has them (`postEffectsDirty`).
    post_effects: Vec<String>,
    post_effects_dirty: bool,
    /// The locator bar: the icon (`locatorBarIcon`), the level whose waypoint manager has the
    /// player, `firstTick`, and the position waypoints were last updated for.
    waypoint_icon: waypoints::Icon,
    waypoint_dim: Option<DimId>,
    waypoint_first_tick: bool,
    waypoint_last_pos: [f64; 3],
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
    /// `horizontalCollision` as the client last reported it (`ServerboundMovePlayerPacket`).
    horizontal_collision: bool,
    view_distance: i32,
    center: ChunkPos,
    sent_chunks: FastSet<ChunkPos>,
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
    /// The crafting grid slots (500..504) `/item` reaches through `player.crafting.N`.
    command_slots: [kiln_item::ItemStack; 4],
    /// Movement packets for this player's viewers.
    tracker: packets::entity::MovementTracker,
    /// Players currently seeing this one (sorted).
    seen_by: Vec<ConnId>,
    /// Section at the last visibility update; `None` forces a re-evaluation.
    section: Option<[i32; 3]>,
    sneaking: bool,
    sprinting: bool,
    /// Gliding with an elytra (shared flag 7) and the ticks it has lasted.
    fall_flying: bool,
    fall_fly_ticks: i32,
    /// `autoSpinAttackTicks` (a riptide throw): the spin attack lasts this many more ticks.
    spin_ticks: i32,
    /// `autoSpinAttackDmg` and `autoSpinAttackItemStack`: what the spin hits with (the trident
    /// as thrown, from the hand `spin_off_hand` names).
    spin_damage: f32,
    spin_item: kiln_item::ItemStack,
    spin_off_hand: bool,
    /// The tick counted down this tick, so the region checks what the spin touched.
    spin_check: bool,
    /// The spin attack pose (a box 0.6 high) the player's last tick settled into
    /// (`updatePlayerPose` runs after `aiStep` counts the spin down).
    spin_pose: bool,
    /// Shared flags or pose changed since the last broadcast.
    meta_dirty: bool,
    /// Arm swung this tick.
    swung: bool,
    /// `LivingEntity.swingState`: ticks into the current swing (-1 when it has just begun) and
    /// its length (0: not swinging), and what viewers are told when a swing begins.
    swing_ticks: i32,
    swing_duration: i32,
    swing_kind: i32,
    swing_wire_duration: i32,
    /// Ticks of use of a `kinetic_weapon` this tick (for [`spear::kinetic_attack`]).
    kinetic_ticks: Option<i32>,
    /// `LivingEntity.recentKineticEnemies`: the entities a charging weapon touched and when.
    recent_stabs: Vec<(i32, i64)>,
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
    /// Sounds the player made this tick, for its viewers.
    pending_sounds: Vec<Bytes>,
    /// `Level.soundSeedGenerator` stand-in for this player's sounds.
    sound_seed: kiln_javamath::random::LegacyRandom,
    health: f32,
    food: i32,
    saturation: f32,
    /// Distance fallen since last on the ground.
    fall_distance: f64,
    /// `Entity.mainSupportingBlockPos`, `onGroundNoBlocks` and `wasTouchingWater` (the landing
    /// block of a fall is found through the first, the water state decides the second's reset).
    main_supporting_block: Option<kiln_entity::math::BlockPos>,
    on_ground_no_blocks: bool,
    was_touching_water: bool,
    /// The server's own body of the player (see [`phantom`]): its velocity, its stuck
    /// multiplier (`makeStuckInBlock`) and the movements of the tick.
    phantom: Option<Box<kiln_entity::entity::Entity>>,
    server_delta: [f64; 3],
    stuck_speed: [f64; 3],
    movements: Vec<phantom::Mv>,
    /// `isEyeInFluid(WATER)` as the last fluid update left it.
    was_eye_in_water: bool,
    /// `Entity.ticksFrozen`, `isInPowderSnow` and the powder snow speed modifier's amount.
    ticks_frozen: i32,
    is_in_powder_snow: bool,
    frost_speed: Option<f64>,
    /// Block changes a player's own tick asks of its region (melted powder snow, trampled
    /// farmland).
    block_edits: Vec<fall::BlockEdit>,
    /// Flying (creative or spectator), from the client's abilities packet.
    flying: bool,
    /// Dead until the client asks to respawn.
    dead: bool,
    /// Died this tick: viewers see the death animation.
    died: bool,
    /// `ServerPlayer.raidOmenPosition`: where bad omen turned into raid omen.
    raid_omen_position: Option<[i32; 3]>,
    /// The raid omen ran out this tick at this position with this amplifier: the serial phase
    /// starts or extends a raid (`Raids.createOrExtendRaid`).
    raid_omen_trigger: Option<([i32; 3], i32)>,
    /// What bad omen needs to know this tick: in a village, and the raid there already has
    /// the most omen levels.
    omen_village: bool,
    omen_raid_full: bool,
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
    /// `Entity.needsSync` after a `push` (a riptide): the tracking players, not the pushed
    /// player's own client, get the motion.
    push_sync: bool,
    /// What sits on the shoulders (`ShoulderEntityLeft`, `ShoulderEntityRight`), when they sat
    /// down (`timeEntitySatOnShoulder`), whether the entity data still has to say so, and the
    /// compounds let go this tick (they become entities in the region's chunk).
    shoulders: [Option<kiln_proto::nbt::Tag>; 2],
    shoulder_time: i64,
    shoulder_dirty: bool,
    released_shoulders: Vec<kiln_proto::nbt::Tag>,
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
    /// Item cooldowns (`ItemCooldowns`): group and the `tick_count` it ends at.
    item_cooldowns: Vec<(String, i32)>,
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
    /// The type of that entity (for the vehicle entity predicates of criteria).
    vehicle_type: Option<&'static str>,
    /// A teleport of the player's own (chorus fruit, an ender pearl) takes it off what it rides
    /// (`Entity.teleport` stops the riding): the region sees to it at the next ride tick.
    dismount_on_teleport: bool,
    /// A saved `RootVehicle` waiting to be put back under the player.
    returning_vehicle: Option<persist::ReturningVehicle>,
    /// `ServerPlayer.levitationStartTime` and `levitationStartPos` (the `levitation` trigger).
    levitation_start: Option<(i32, [f64; 3])>,
    /// `startingToFallPosition` (`fall_from_height`), `enteredNetherPosition`
    /// (`nether_travel`) and `enteredLavaOnVehiclePosition` (`ride_entity_in_lava`).
    starting_to_fall: Option<[f64; 3]>,
    entered_nether: Option<[f64; 3]>,
    entered_lava_on_vehicle: Option<[f64; 3]>,
    /// `ServerPlayer.wardenSpawnTracker`.
    warden_tracker: sculk::shrieker::WardenSpawnTracker,
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
    /// The type of the last mob that hurt the player and when (`getKillCredit`'s fallback).
    last_mob_attacker: Option<(&'static str, i64)>,
    /// `ServerStatsCounter`.
    stats: player_stats::PlayerStats,
    /// `ServerRecipeBook`.
    recipe_book: recipe_book::RecipeBook,
    /// `PlayerAdvancements`.
    advancements: advancements::progress::PlayerAdvancements,
    /// Base values and permanent modifiers `/attribute` set.
    command_attributes: combat::CommandAttributes,
    /// `minecraft:limited_crafting`, kept up to date for the menus (only recipes the player's
    /// recipe book has can be crafted).
    limited_crafting: bool,
}

impl Player {
    fn send(&mut self, p: Bytes) {
        self.outbox.push(p);
    }
    fn flush(&mut self) {
        if !self.outbox.is_empty() {
            // (The next tick's outbox starts with room for as many packets.)
            let n = self.outbox.len();
            self.sink.send_batch(std::mem::replace(&mut self.outbox, Vec::with_capacity(n)));
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
        self.award_stat(player_stats::Stat::item(player_stats::DROPPED, dropped.item()), dropped.count());
        self.award_stat(*player_stats::stat::DROP, 1);
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
        // The inventory menu's armor slots 5 (head) to 8 (feet), and the off hand, 45.
        for (i, s) in self.inv.equipment.iter().take(5).enumerate() {
            let slot = [8, 7, 6, 5, 45][i];
            out[slot] = view(s);
        }
        out
    }

    /// An item thrown from the eyes in the look direction (`LivingEntity.createItemStackToDrop`
    /// with `throwRandomly` false).
    fn throw(&mut self, stack: kiln_item::ItemStack) -> entities::Spawn {
        use kiln_javamath::{mth, trig};
        // `Mth.sin`/`Mth.cos` (the table) for the look angles, `Math.sin`/`Math.cos` for the
        // spread angle, the sums in doubles as the bytecode has them.
        let (yaw, pitch) = ((self.rot[0] * 0.017453292f32) as f64, (self.rot[1] * 0.017453292f32) as f64);
        let (sin_pitch, cos_pitch) = (mth::sin(pitch), mth::cos(pitch));
        let (sin_yaw, cos_yaw) = (mth::sin(yaw), mth::cos(yaw));
        let f = 0.3f32;
        let angle = (self.rng.next_f32() * 6.2831855f32) as f64;
        let spread = (0.02f32 * self.rng.next_f32()) as f64;
        let vel = [
            (-sin_yaw * cos_pitch * f) as f64 + trig::cos(angle) * spread,
            (-sin_pitch * f + 0.1 + (self.rng.next_f32() - self.rng.next_f32()) * 0.1) as f64,
            (cos_yaw * cos_pitch * f) as f64 + trig::sin(angle) * spread,
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
        // A teleport is not movement through blocks, and it ends the server body's momentum.
        self.movements.clear();
        self.server_delta = [0.0; 3];
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
    /// Point of interest chunks (`poi/`), when the world is saved somewhere.
    poi_store: Option<kiln_storage::PoiStore>,
    /// The level's raids (`raids.dat`) and patrol spawner.
    raids: raid::Raids,
    /// The dimension's store when the world is in the native format.
    native: Option<std::sync::Arc<std::sync::Mutex<kiln_storage::NativeStore>>>,
    /// Saved entities of loaded chunks that Kiln does not simulate (mobs, ...), written back
    /// as they were loaded.
    raw_entities: HashMap<ChunkPos, Vec<Tag>>,
    /// End gateways cooling down after a teleport, until this game time
    /// (`TheEndGatewayBlockEntity.teleportCooldown`; gateways do not tick in Kiln).
    gateway_cooldowns: HashMap<[i32; 3], i64>,
    /// Non-player entities that came through a portal, by UUID, until this game time.
    portal_cooldowns: HashMap<u128, i64>,
    /// Regions ticking away from the lockstep (independent mode): their cells and part are
    /// lent out, so the topology and chunks of their cells wait until they are back.
    lent: HashSet<RegionId>,
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
            let started = Instant::now();
            let chunk = self.provider.load_or_generate(pos);
            chunkstats::count(&chunkstats::SYNC_LOADS);
            chunkstats::add_ns(&chunkstats::SYNC_NS, started.elapsed());
            chunkstats::max_ns(&chunkstats::SYNC_MAX_NS, started.elapsed());
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
        native: Option<std::sync::Arc<std::sync::Mutex<kiln_storage::NativeStore>>>,
    ) -> Dim {
        let kind = kiln_data::dimension_type(key).expect("vanilla dimension type");
        let generation = provider.fork_generator().map(|g| generation::GenPool::new(g.as_ref(), provider.dimension, threads));
        let entity_store = match native.clone() {
            Some(store) => Some(kiln_storage::EntityStore::native(store)),
            None => world.map(|dir| kiln_storage::EntityStore::new(dir.join(dimension_dir(key)).join("entities"))),
        };
        let poi_store = match native.clone() {
            Some(store) => Some(kiln_storage::PoiStore::native(store)),
            None => world.map(|dir| kiln_storage::PoiStore::new(dir.join(dimension_dir(key)).join("poi"))),
        };
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
            poi_store,
            raids: raid::Raids::new(),
            native,
            raw_entities: HashMap::new(),
            gateway_cooldowns: HashMap::new(),
            portal_cooldowns: HashMap::new(),
            lent: HashSet::new(),
        }
    }

    /// Whether the cell of `pos` belongs to a region that is lent out.
    fn lent_at(&self, pos: ChunkPos) -> bool {
        !self.lent.is_empty() && self.regions.owner(pos.cell()).is_some_and(|r| self.lent.contains(&r))
    }

    /// Whether `pos` is loaded (a lent region's chunks count as loaded: they are away).
    fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.regions.chunk(pos).is_some() || self.pending.contains_key(&pos) || self.lent_at(pos)
    }

    /// Puts a loaded chunk in its region, or pending until its cell gets one. Returns whether
    /// it went straight into a region.
    fn install(&mut self, pos: ChunkPos, chunk: Chunk) -> bool {
        // A lent region takes its chunks when it is back.
        if self.lent_at(pos) {
            self.pending.insert(pos, chunk);
            return false;
        }
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
        let dt = Instant::now();
        part.1.chunk_loaded(pos, &mut chunk, self.game_time);
        let dt = diag::lap("in.loaded", dt);
        // `PoiManager`: the chunk's saved points of interest, checked against its blocks.
        let stored = self.poi_store.as_mut().and_then(|s| s.load(pos)).map(|t| kiln_world::poi::ChunkPois::from_nbt(&t));
        chunk.init_pois(pos.x, pos.z, stored);
        let dt = diag::lap("in.pois", dt);
        let new = chunk.is_new();
        let generated = std::mem::take(&mut chunk.generated_entities);
        cell.insert(pos, chunk);
        // A generated chunk gets its light from its blocks and loaded neighbours (vanilla's
        // LIGHT status).
        if new {
            kiln_world::light::light_new_chunk(cells, pos);
        }
        let dt = diag::lap("in.light", dt);
        self.load_entities(pos);
        let dt = diag::lap("in.entities", dt);
        self.add_saved_entities(pos, generated);
        diag::lap("in.saved", dt);
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
        let started = Instant::now();
        match self.provider.load(pos) {
            Some(chunk) => {
                let read = started.elapsed();
                chunkstats::count(&chunkstats::DISK_HITS);
                chunkstats::add_ns(&chunkstats::DISK_NS, read);
                chunkstats::max_ns(&chunkstats::DISK_MAX_NS, read);
                self.timed_install(pos, chunk);
                true
            }
            None => {
                chunkstats::count(&chunkstats::DISK_MISSES);
                chunkstats::add_ns(&chunkstats::DISK_MISS_NS, started.elapsed());
                self.generation.as_mut().is_some_and(|p| p.request(pos))
            }
        }
    }

    /// The chunk a player has to stand in (joining, teleported, moved to another level): loaded
    /// now if it is stored (or cheap to make, without background generation); otherwise
    /// generated ahead of every other chunk while the player waits. Whether it is loaded.
    fn request_urgent(&mut self, pos: ChunkPos) -> bool {
        if self.is_loaded(pos) {
            return true;
        }
        let Some(pool) = &self.generation else {
            self.load_chunk(pos);
            return true;
        };
        // A chunk queued already was not stored when it was asked for.
        if !pool.is_queued(pos) {
            let started = Instant::now();
            if let Some(chunk) = self.provider.load(pos) {
                let read = started.elapsed();
                chunkstats::count(&chunkstats::DISK_HITS);
                chunkstats::add_ns(&chunkstats::DISK_NS, read);
                chunkstats::max_ns(&chunkstats::DISK_MAX_NS, read);
                self.timed_install(pos, chunk);
                return true;
            }
            chunkstats::count(&chunkstats::DISK_MISSES);
            chunkstats::add_ns(&chunkstats::DISK_MISS_NS, started.elapsed());
        }
        if let Some(pool) = &mut self.generation {
            pool.request_urgent(pos);
        }
        false
    }

    /// `install`, counted in the chunk statistics.
    fn timed_install(&mut self, pos: ChunkPos, chunk: Chunk) -> bool {
        let started = Instant::now();
        let r = self.install(pos, chunk);
        let took = started.elapsed();
        chunkstats::count(&chunkstats::INSTALLED);
        chunkstats::add_ns(&chunkstats::INSTALL_NS, took);
        chunkstats::max_ns(&chunkstats::INSTALL_MAX_NS, took);
        r
    }

    /// Installs the chunks generation finished: those players stand in (`first`, sorted) at
    /// once, the rest in position order within [`INSTALL_BUDGET`], so that a burst of finished
    /// chunks spreads over a few ticks instead of holding one up; the others wait, still
    /// counted as in flight.
    fn install_generated(&mut self, first: &[ChunkPos]) {
        let Some(pool) = &mut self.generation else { return };
        pool.collect();
        if !pool.has_ready() {
            return;
        }
        let ready: Vec<ChunkPos> = pool.ready().collect();
        let started = Instant::now();
        let (now, later): (Vec<ChunkPos>, Vec<ChunkPos>) = ready.into_iter().partition(|p| first.binary_search(p).is_ok());
        for pos in now.into_iter().chain(later) {
            if first.binary_search(&pos).is_err() && started.elapsed() >= INSTALL_BUDGET {
                break;
            }
            let Some(chunk) = self.generation.as_mut().and_then(|p| p.take(pos)) else { continue };
            // Loaded synchronously in the meantime (a command needed it).
            if !self.is_loaded(pos) {
                self.timed_install(pos, chunk);
            }
        }
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
                if let (Some(store), Some(p)) = (self.poi_store.as_mut(), chunk.pois.as_deref())
                    && p.dirty
                {
                    store.store(pos, Some(p.to_nbt(kiln_storage::anvil::DATA_VERSION as i32)));
                }
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
        // The regionizer needs every region of the level at home.
        if !self.lent.is_empty() {
            return false;
        }
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
    players: FastMap<ConnId, Player>,
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
    /// The End's dragon fight (`EnderDragonFight`).
    dragon_fight: dragon_fight::DragonFight,
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
    /// Each level's locator bar waypoints (`ServerWaypointManager`).
    waypoints: [waypoints::WaypointManager; 3],
    /// Advancements of the enabled data packs.
    advancements: std::sync::Arc<advancements::Advancements>,
    plugins: Option<plugins::SimPlugins>,
    /// Independent scheduling state: regions ticking away, their clocks.
    independent: independent::Independent,
    /// What each region's packet and tick work took last time (ns), so the biggest start first.
    unit_costs: [FastMap<(DimId, RegionId), u64>; 2],
    /// Some player's post effects may have changed since they were last sent.
    post_effects_pending: bool,
    /// `WanderingTraderSpawner` (the overworld's).
    trader: trader::TraderSpawner,
    /// Borders, tick rate, forced chunks and random sequences (the world commands).
    world: world_state::WorldState,
    /// Joins waiting for the chunk they stand in (generated ahead of everything else); the
    /// client stays on its joining screen meanwhile.
    waiting_joins: Vec<(persist::Joining, JoinInfo)>,
    /// Packets of players in [`LIMBO`], applied once they are placed.
    held_packets: Vec<(ConnId, PlayIn)>,
}

/// The region of players whose chunk is still being generated (teleported or moved to another
/// level into terrain not made yet): they wait outside every region, untouched by the region
/// ticks, until the chunk is in, as a vanilla client waits on its loading screen. Only levels
/// that generate terrain in the background have such players; elsewhere the chunk loads at once.
pub(crate) const LIMBO: RegionId = RegionId(u64::MAX);

/// Operator names from `KILN_OPS` (comma separated).
fn ops_from_env() -> HashSet<String> {
    std::env::var("KILN_OPS")
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

pub fn run(config: SimConfig, rx: Receiver<ToSim>) {
    let mut sim = Sim::new(config);
    sim.sync_ops();
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
        // `/tick sprint`: ticks run back to back until the sprint is over.
        let sprinting = sim.world.tick_rate.is_sprinting() && {
            let mut news = world_state::TickNews::default();
            let sprint = sim.world.tick_rate.check_sprint(&mut news);
            sim.tick_rate_news(news);
            sprint
        };
        if !sim.step(inbox.drain(..)) {
            return;
        }
        if sprinting {
            sim.world.tick_rate.end_tick_work();
            next_tick = Instant::now();
            continue;
        }

        // The tick rate's cadence (50 ms by default); if we fell behind, don't try to catch up.
        next_tick += Duration::from_nanos(sim.world.tick_rate.nanos_per_tick() as u64);
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
        let format = config.world.as_deref().map(|d| kiln_storage::WorldFormat::resolve(d, config.world_format));
        if format == Some(kiln_storage::WorldFormat::Native) {
            info!("world storage: native format");
        }
        // A native world's store per dimension, shared by its chunks and entities.
        let mut native_stores: Vec<Option<std::sync::Arc<std::sync::Mutex<kiln_storage::NativeStore>>>> = Vec::new();
        // Each level's generation pipeline, for `/locate` and `/place`.
        let mut pipelines: Vec<Option<std::sync::Arc<kiln_worldgen::pipeline::Pipeline>>> = Vec::new();
        let providers: Vec<ChunkProvider> = (0..DIMENSIONS.len())
            .map(|id| {
                let (key, biome_name) = DIMENSIONS[id];
                let kind = kiln_data::dimension_type(key).expect("vanilla dimension type");
                let dimension = Dimension { min_y: kind.min_y, height: kind.height };
                let biome = kiln_data::synced_id("minecraft:worldgen/biome", biome_name).expect("default biome") as u16;
                let generator = generator(id);
                pipelines.push(generator.as_ref().map(|g| g.pipeline().clone()));
                match &config.world {
                    Some(dir) => {
                        let source: Box<dyn kiln_world::ChunkSource> = if format == Some(kiln_storage::WorldFormat::Native) {
                            let store = kiln_storage::NativeStore::shared(dir.join(dimension_dir(key)).join("native"));
                            native_stores.push(Some(store.clone()));
                            Box::new(kiln_storage::NativeSource::new(store))
                        } else {
                            native_stores.push(None);
                            Box::new(kiln_storage::AnvilSource::new(dir.join(dimension_dir(key)).join("region")))
                        };
                        let provider = ChunkProvider::with_source(dimension, source, Terrain::Void, biome, biome_count);
                        if id == OVERWORLD_ID {
                            // A world without a saved spawn (a new directory) starts where the
                            // generator would put it, not inside whatever terrain is at (0, 64, 0).
                            let s = kiln_storage::read_spawn(dir)
                                .or_else(|| generator.as_ref().map(|g| initial_spawn(g.pipeline())))
                                .unwrap_or([0, 64, 0]);
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
        // What grows in each level (saplings, bone meal) is placed by the level's own worldgen.
        let feature_hosts: Vec<Option<std::sync::Arc<dyn kiln_blocks::feature_host::FeatureHost>>> = pipelines
            .iter()
            .map(|p| p.as_ref().map(|p| std::sync::Arc::new(kiln_worldgen::host::WorldgenHost::new(p.world().clone())) as std::sync::Arc<dyn kiln_blocks::feature_host::FeatureHost>))
            .collect();
        let spawn = spawn.expect("overworld spawn");
        let policy = if config.unified_regions { RegionPolicy::unified() } else { RegionPolicy::default() };
        let threads = config.noise.as_ref().map_or(1, |n| n.threads);
        let mut storage = config.world.as_deref().map(persist::Storage::open);
        let level = storage.as_ref().filter(|s| s.level.exists()).map(|s| s.level.state());
        // The seed: the generator's, else the save's (`world_gen_settings.dat`); a world that
        // generates with a seed and has none saved gets it written, so vanilla loads the same.
        let seed = match (&config.noise, storage.as_mut()) {
            (Some(n), Some(s)) => {
                s.level.set_seed_if_missing(n.seed);
                n.seed
            }
            (Some(n), None) => n.seed,
            (None, s) => s.and_then(|s| s.level.seed()).unwrap_or(0),
        };
        let game_time = level.as_ref().map_or(0, |l| l.game_time);
        let dims = providers
            .into_iter()
            .enumerate()
            .map(|(id, provider)| {
                let native = native_stores.get_mut(id).and_then(Option::take);
                Dim::new(DIMENSIONS[id].0, provider, policy, threads, game_time, config.world.as_deref(), native)
            })
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
            players: FastMap::default(),
            next_entity_id: 1,
            started: Instant::now(),
            stats: stats::TickStats::default(),
            game_time,
            day_time: level.as_ref().map_or(1000, |l| l.day_time),
            overworld_clock: kiln_data::synced_id("minecraft:world_clock", OVERWORLD).expect("overworld clock"),
            end_time: 0,
            end_clock: kiln_data::synced_id("minecraft:world_clock", "minecraft:the_end").expect("end clock"),
            dragon_fight: dragon_fight::DragonFight::load(None, seed),
            commands: commands::CommandState::new(ops_from_env()),
            weather: Default::default(),
            level_weather: Default::default(),
            weather_random: kiln_javamath::random::LegacyRandom::new(seed ^ 0x7765_6174_6865_72),
            climates: weather::Climates::load(&vanilla_pack).map(std::sync::Arc::new),
            zoom_seed: kiln_worldgen::generator::obfuscate_seed(seed),
            clock_runs: Default::default(),
            sleep_status: Default::default(),
            waypoints: Default::default(),
            advancements: Default::default(),
            plugins: None,
            independent: Default::default(),
            unit_costs: Default::default(),
            post_effects_pending: false,
            trader: Default::default(),
            world: world_state::WorldState { pipelines, feature_hosts, ..Default::default() },
            waiting_joins: Vec::new(),
            held_packets: Vec::new(),
        };
        // Boss bar ids are random per server run, as vanilla draws them from the level random.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        sim.commands.bossbars.seed(now.as_nanos() as u64);
        sim.commands.seed = seed;
        sim.load_admin_state();
        sim.load_scoreboard();
        sim.load_stopwatches();
        sim.load_weather();
        sim.load_world_state();
        sim.load_raids();
        sim.load_trader();
        sim.load_dragon_fight();
        sim.init_packs(vanilla_pack);
        sim.load_plugins();
        // Tables built on first use, built now rather than in the middle of a tick (the path
        // types of every block state: 13 ms the first time a mob looks for a path).
        kiln_entity::mob::path::path_type_from_state(0);
        sim.prepare_spawn();
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

        // The workers spin through the tick's windows instead of parking between them
        // (`SimConfig::prewake`: CPU for latency).
        if !self.config.prewake.is_zero() {
            self.pool.prewake(self.config.prewake);
        }
        // `ServerTickRateManager.tick`: whether the levels run this tick.
        self.world.tick_rate.tick();
        // B0: connection events, chunks, topology, joins, membership.
        let (mut packets, mut joins, mut console, mut leaves) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut profile_results = Vec::new();
        for msg in inbox {
            match msg {
                ToSim::Join(j) => joins.push(j),
                // A connection that joined and left in the same batch never enters the game.
                ToSim::Leave(conn) => match joins.iter().position(|j: &JoinInfo| j.conn == conn) {
                    Some(i) => drop(joins.remove(i)),
                    None => match self.waiting_joins.iter().position(|(_, j)| j.conn == conn) {
                        Some(i) => drop(self.waiting_joins.remove(i)),
                        None => leaves.push(conn),
                    },
                },
                ToSim::Packet(conn, pkt) => packets.push((conn, pkt)),
                ToSim::Console(command) => console.push(command),
                ToSim::Shutdown { done } => {
                    self.shut_down();
                    let _ = done.send(());
                    return false;
                }
                ToSim::ProfileLookup { request, result } => profile_results.push((request, result)),
            }
        }
        // `/kiln use` clicks, as if their players had sent them.
        packets.splice(0..0, std::mem::take(&mut self.commands.injected));
        // Independent mode: regions back from ticking away rejoin; anything that needs the
        // whole server waits for all of them.
        let dt = Instant::now();
        let packets = self.independent_b0(packets, !joins.is_empty() || !leaves.is_empty() || !console.is_empty());
        self.track_idle(&packets);
        let dt = diag::lap("b0.idle", dt);
        self.maintain_chunks();
        let dt = diag::lap("b0.chunks", dt);
        self.rendezvous_for_topology();
        // A join waits (no level yet, the client on its joining screen) until the chunk it
        // stands in is loaded; terrain not made yet is generated ahead of everything else.
        let mut joining = std::mem::take(&mut self.waiting_joins);
        joining.extend(joins.into_iter().map(|j| (self.joining(j.uuid), j)));
        let (joining, waiting): (Vec<_>, Vec<_>) =
            joining.into_iter().partition(|(jn, _)| self.dims[jn.dim].request_urgent(player_chunk(jn.pos)));
        self.waiting_joins = waiting;
        let changed = self.apply_topology();
        let dt = diag::lap("b0.topology", dt);
        self.plugins_b0();
        for (jn, j) in joining {
            let conn = j.conn;
            self.join(j, jn);
            self.plugins_joined(conn);
        }
        let dt = diag::lap("b0.joins", dt);
        self.update_membership(changed);
        diag::lap("b0.membership", dt);
        lap(&mut self.stats, "b0");

        // P: region-local packets in parallel.
        let w0 = kiln_sched::window_ns();
        let dt = Instant::now();
        let packets = self.hold_limbo_packets(packets);
        let (local, exclusive) = self.route(packets);
        let dt = diag::lap("p.route", dt);
        let outs = self.run_regions(0, local, |w, env, ctx| w.apply_packets(env, ctx));
        diag::lap("p.run", dt);
        for (dim, out) in outs {
            self.dims[dim].spawns.extend(out.spawns);
            self.announce_deaths(out.deaths);
        }
        self.deliver_plugin_messages();
        diag::add("packets~", Duration::from_nanos(kiln_sched::window_ns() - w0));
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
        self.deliver_profile_answers();
        for command in console {
            self.run_console_command(command.trim_start_matches('/'));
        }
        for (request, result) in profile_results {
            self.profile_lookup_finished(request, result);
        }
        lap(&mut self.stats, "g.console");
        self.tick_global();
        lap(&mut self.stats, "g.tick_global");
        self.flush_stat_scores();
        lap(&mut self.stats, "g.stat_scores");
        self.advancement_upkeep();
        lap(&mut self.stats, "g.advancements");
        // Players teleported in PX or G tick in their destination's region from now on.
        self.settle_teleported();
        lap(&mut self.stats, "g.settle");
        self.deliver_plugin_messages();
        lap(&mut self.stats, "global");

        // L: regions tick in parallel; in independent mode, regions too slow for the tick
        // tick away on their own.
        self.lend_slow_regions();
        let outs = self.run_regions(1, BTreeMap::new(), |w, env, ctx| w.tick(env, ctx));
        self.note_region_ticks();
        let mut times = [Duration::ZERO; region::SUB_PHASES.len()];
        let mut travels = Vec::new();
        let mut portal_candidates = Vec::new();
        for (dim, out) in outs {
            let d = &mut self.dims[dim];
            d.requests.extend(out.wanted);
            d.unloads.extend(out.unload);
            d.spawns.extend(out.spawns);
            for tag in out.saved_entities {
                let at = tag.get("Pos").and_then(Tag::as_list).map(|l| [0, 1, 2].map(|i| l.get(i).and_then(Tag::as_f64).unwrap_or(0.0)));
                if let Some(at) = at {
                    d.add_saved_entities(entities::chunk_of(at), vec![tag]);
                }
            }
            travels.extend(out.portals);
            portal_candidates.extend(out.portal_candidates.into_iter().map(|id| (dim, id)));
            self.announce_deaths(out.deaths);
            for (t, d) in times.iter_mut().zip(out.times) {
                *t += d;
            }
            for (name, d) in region::SUB_WIN.iter().zip(out.win) {
                diag::add(name, d);
            }
        }
        // Entities from here on have newer ids.
        let first_new = self.next_entity_id;
        self.materialize_spawns();
        // What the dragon and the crystals told the fight.
        let fight = self.dragon_fight_messages();
        // Players whose portal time ran out change level (serially: two levels take part).
        if !travels.is_empty() {
            self.rendezvous();
        }
        travels.sort_unstable_by_key(|t: &portal::Travel| t.conn);
        // Entities in portals: those the regions found near portal blocks and the newer ones,
        // unless blocks or entities may have changed since the regions looked (a fight's
        // portal, players' trips, regions ticking away).
        let looked = travels.is_empty() && !fight && self.dims.iter().all(|d| d.lent.is_empty());
        for t in travels {
            self.travel(t);
        }
        portal_candidates.sort_unstable();
        self.entity_portals(looked.then_some((&portal_candidates[..], first_new)));
        self.materialize_spawns();
        lap(&mut self.stats, "regions");
        // CPU time summed over regions (the "regions" phase is wall time).
        for (name, d) in region::SUB_PHASES.iter().zip(times) {
            self.stats.phase(name, d);
        }

        // Players in limbo get what was sent to them (their regions' flush does not see them).
        for p in self.players.values_mut().filter(|p| p.region == LIMBO) {
            p.flush();
        }

        for (name, d) in diag::take() {
            self.stats.phase(name, d);
        }
        self.record_tick_time(start.elapsed().as_nanos() as i64);
        if let Some(ms) = stats::slow_print_ms() {
            let took = start.elapsed().as_secs_f64() * 1e3;
            let phases = self.stats.take_tick();
            if took > ms {
                info!("slow tick {took:.1} ms ({} players): {phases}", self.players.len());
            }
        }
        stats::trace(start.elapsed().as_micros() as u64, self.players.len());
        if let Some(report) = self.stats.record(start.elapsed()) {
            stats::flush_trace();
            info!(
                "{} players, {} regions, {} chunks | {report}",
                self.players.len(),
                self.region_count(),
                self.dims.iter().map(|d| d.regions.loaded_chunks()).sum::<usize>()
            );
            self.commands.last_report = Some(report.to_string());
            let gen_threads = self.config.noise.as_ref().map_or(0, |n| n.threads);
            info!("{}", chunkstats::line(gen_threads));
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
                let mob = e.phys.as_deref().and_then(|p| kiln_entity::mob::data(p).map(|m| (m.health.to_bits(), m.target, m.y_head_rot.to_bits())));
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
        self.hash_plugins(&mut h);
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
        self.players.get(&conn).map(|p| (p.sleep.pos, p.sleep.counter, p.stats.get(*player_stats::stat::TIME_SINCE_REST)))
    }

    /// A player's respawn point and level.
    pub fn respawn_point(&self, conn: ConnId) -> Option<(Option<[i32; 3]>, &'static str)> {
        self.players.get(&conn).map(|p| (p.respawn, DIMENSIONS[p.respawn_dim].0))
    }

    pub fn game_time(&self) -> i64 {
        self.game_time
    }

    /// The world seed (`/seed`).
    pub fn seed(&self) -> i64 {
        self.commands.seed
    }

    /// Whether the difficulty is locked (`Data.difficulty_settings.locked`).
    pub fn difficulty_locked(&self) -> bool {
        self.commands.difficulty_locked
    }

    /// The permission level a player of this name would have (0 for non-operators).
    pub fn permission_level_of_name(&self, name: &str) -> u8 {
        if self.commands.is_op(name) { self.commands.op_levels.get(name).copied().unwrap_or(4) } else { 0 }
    }

    /// Block state at a position in the overworld, if its chunk is loaded.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        self.dims[OVERWORLD_ID].regions.get_block(x, y, z)
    }

    /// Ticks players spent near the loaded overworld chunk holding `x`, `z` (`InhabitedTime`).
    pub fn inhabited_time_at(&self, x: i32, z: i32) -> Option<i64> {
        self.dims[OVERWORLD_ID].regions.chunk(ChunkPos::of_block(x, z)).map(|c| c.inhabited_time())
    }

    /// The saved form of the live sculk block entity (sensor, shrieker, catalyst) at an
    /// overworld position (for tests and tools).
    pub fn block_entity_nbt(&self, x: i32, y: i32, z: i32) -> Option<kiln_proto::nbt::Tag> {
        let region = self.dims[OVERWORLD_ID].regions.at(ChunkPos::of_block(x, z).cell())?;
        let p = kiln_blocks::BlockPos::new(x, y, z);
        let part = &region.part().1;
        part.sculk
            .map
            .get(&p)
            .map(|b| b.save())
            .or_else(|| part.hearts.map.get(&p).map(|h| h.save()))
            .or_else(|| part.spawners.map.get(&p).map(|s| s.save()))
            .or_else(|| part.containers.map.get(&p).map(|c| c.save()))
    }

    /// The saved form (`saveWithFullMetadata`) of the block entity at an overworld position as
    /// its chunk holds it (container block entities may lag the live container; for tests and
    /// tools).
    pub fn block_entity_saved(&self, x: i32, y: i32, z: i32) -> Option<kiln_proto::nbt::Tag> {
        let chunk = self.dims[OVERWORLD_ID].regions.chunk(ChunkPos::of_block(x, z))?;
        chunk.block_entity((x & 15) as usize, y, (z & 15) as usize).map(|be| be.saved([x, y, z]))
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

    /// Chunk work not done yet: chunks players asked for this tick, queued or being generated,
    /// or waiting for a cell (load tools wait for none before measuring).
    pub fn chunk_backlog(&self) -> usize {
        self.dims.iter().map(|d| d.requests.len() + d.pending.len() + d.generation.as_ref().map_or(0, |g| g.in_flight())).sum()
    }

    pub fn player_count(&self) -> usize {
        self.players.len() + self.independent.players_away()
    }

    /// Regions of all levels.
    pub fn region_count(&self) -> usize {
        self.dims.iter().map(|d| d.regions.len()).sum()
    }

    /// The tick pool's per-worker counters (for load tools).
    pub fn pool_stats(&self) -> Vec<kiln_sched::WorkerStats> {
        self.pool.stats()
    }

    /// Time per tick phase (b0, packets, px, global, regions, then the regions' sub-phases as
    /// summed CPU time) since `reset_phase_totals`.
    pub fn phase_totals(&self) -> Vec<(&'static str, Duration)> {
        self.stats.totals().to_vec()
    }

    pub fn reset_phase_totals(&mut self) {
        self.stats.reset_totals();
    }

    pub fn reset_pool_stats(&self) {
        self.pool.reset_stats();
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

    /// The network ids of the entities of type `name`, in id order (for tests and tools).
    pub fn entity_ids_of(&self, name: &str) -> Vec<i32> {
        let mut out: Vec<i32> = self.dims.iter().flat_map(|d| d.regions.iter()).flat_map(|r| r.part().0.list.iter()).filter(|e| !e.removed && e.kind.name == name).map(|e| e.id).collect();
        out.sort_unstable();
        out
    }

    /// Every simulated entity's vehicle and passengers: (network id, vehicle, passengers), in id
    /// order (for tests and tools).
    pub fn riding(&self) -> Vec<(i32, Option<i32>, Vec<i32>)> {
        let mut out: Vec<_> = self
            .dims
            .iter()
            .flat_map(|d| d.regions.iter())
            .flat_map(|r| r.part().0.list.iter())
            .filter(|e| !e.removed)
            .filter_map(|e| e.phys.as_deref().map(|p| (e.id, p.vehicle, p.passengers.clone())))
            .collect();
        out.sort_by_key(|r| r.0);
        out
    }

    /// A chest or hopper minecart's slots as (slot, item name, count), and the loot table it
    /// still holds unrolled (for tests and tools).
    pub fn cart_items(&self, id: i32) -> Option<(Vec<(usize, &'static str, i32)>, Option<String>)> {
        let e = self.dims.iter().flat_map(|d| d.regions.iter()).flat_map(|r| r.part().0.list.iter()).find(|e| e.id == id && !e.removed)?;
        let c = kiln_entity::ext_entity::container(e.phys.as_deref()?)?;
        Some((c.items.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| (i, s.item_name(), s.count())).collect(), c.loot_table.clone()))
    }

    /// A minecart's own numbers: the furnace's fuel, the TNT's fuse (-1: not primed) and
    /// the hopper's `enabled` (for tests and tools).
    pub fn cart_state(&self, id: i32) -> Option<(i32, i32, bool)> {
        let e = self.dims.iter().flat_map(|d| d.regions.iter()).flat_map(|r| r.part().0.list.iter()).find(|e| e.id == id && !e.removed)?;
        let cart = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::minecart::Minecart>(e.phys.as_deref()?)?;
        Some((cart.fuel, cart.fuse, cart.enabled))
    }

    /// The entity a player rides (for tests and tools).
    pub fn vehicle_of(&self, conn: ConnId) -> Option<i32> {
        self.players.get(&conn)?.vehicle
    }

    /// The raids of a level: (id, status, waves spawned, omen level, center, raiders alive,
    /// boss bar progress) (for tests and tools).
    pub fn raids(&self, dimension: &str) -> Vec<(i32, &'static str, i32, i32, [i32; 3], usize, f32)> {
        let Some(d) = dim_id(dimension) else { return Vec::new() };
        raid::summary(&self.dims[d])
    }

    /// `ServerLevel.sectionsToVillage` at a position of a level (for tests and tools).
    pub fn sections_to_village(&self, dimension: &str, pos: [i32; 3]) -> i32 {
        dim_id(dimension).map_or(7, |d| poi::sections_to_village(&self.dims[d].regions, pos))
    }

    /// The raider state of a mob: (raid id, wave, patrol leader, patrolling, wears the ominous
    /// banner) (for tests and tools).
    pub fn raider(&self, id: i32) -> Option<(Option<i32>, i32, bool, bool, bool)> {
        let e = self.dims.iter().flat_map(|d| d.regions.iter()).flat_map(|r| r.part().0.list.iter()).find(|e| e.id == id)?;
        let m = kiln_entity::mob::data(e.phys.as_deref()?)?;
        let r = kiln_entity::mob::kinds::raider::raider(m)?;
        Some((r.raid, r.wave, r.patrol_leader, r.patrolling, kiln_entity::mob::kinds::raider::is_ominous_banner(&m.equipment[kiln_entity::mob::HEAD])))
    }

    /// Mobs: (network id, type name, position, health), in id order (for tests and tools).
    pub fn mobs(&self) -> Vec<(i32, &'static str, [f64; 3], f32)> {
        let mut out: Vec<_> = self
            .dims
            .iter()
            .flat_map(|d| d.regions.iter())
            .flat_map(|r| r.part().0.list.iter())
            .filter_map(|e| e.phys.as_deref().and_then(|p| kiln_entity::mob::data(p).map(|m| (e.id, e.kind.name, e.pos, m.health))))
            .collect();
        out.sort_by_key(|m| m.0);
        out
    }

    /// A mob's combat state, as `tools/CombatVectors.java` records it (for tests and tools).
    pub fn mob_state(&self, id: i32) -> Option<MobState> {
        let list = self.dims.iter().flat_map(|d| d.regions.iter()).flat_map(|r| r.part().0.list.iter());
        let e = list.clone().find(|e| e.id == id)?;
        let phys = e.phys.as_deref()?;
        let m = kiln_entity::mob::data(phys)?;
        use kiln_entity::mob::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
        let damage = |slot: usize| (!m.equipment[slot].is_empty()).then(|| m.equipment[slot].damage());
        let vehicle = phys.vehicle.and_then(|v| list.clone().find(|o| o.id == v)).map(|o| o.kind.name);
        Some(MobState {
            health: m.health,
            alive: m.health > 0.0 && !e.removed,
            delta: [phys.delta.x, phys.delta.y, phys.delta.z],
            fire_ticks: phys.remaining_fire_ticks,
            hurt_time: m.hurt_time,
            damage_cooldown: m.damage_cooldown,
            last_hurt: m.last_hurt,
            absorption: m.absorption,
            equipment_damage: [damage(FEET), damage(LEGS), damage(CHEST), damage(HEAD), damage(MAINHAND), damage(OFFHAND), None],
            effects: m.effects.values().map(|f| (kiln_entity::effect::effect_type(f.id).map_or("?", |t| t.name), f.amplifier, f.duration)).collect(),
            on_ground: phys.on_ground,
            vehicle,
            pos: e.pos,
        })
    }

    /// The ticks a player's riptide spin has left (for tests and tools).
    pub fn spin_ticks(&self, conn: ConnId) -> Option<i32> {
        Some(self.players.get(&conn)?.spin_ticks)
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

    /// The overworld's fishing bobbers: (owner entity id, biting, bobbing in water) (for tests).
    pub fn fishing_bobbers(&self) -> Vec<(i32, bool, bool)> {
        self.dims[OVERWORLD_ID]
            .regions
            .iter()
            .flat_map(|r| r.part().0.list.iter())
            .filter(|e| !e.removed)
            .filter_map(|e| e.phys.as_deref().and_then(kiln_entity::ext_entity::fishing_hook::get))
            .map(|h| (h.owner, h.biting, h.state == kiln_entity::ext_entity::fishing_hook::State::Bobbing))
            .collect()
    }

    /// Whether a player has criterion `criterion` of advancement `id` (for tests); `None` when
    /// the advancement or criterion is unknown.
    pub fn criterion_done(&self, conn: ConnId, id: &str, criterion: &str) -> Option<bool> {
        let p = self.players.get(&conn)?;
        let data = &p.advancements.data;
        let i = data.get(id)?;
        let c = data.list[i].criterion_index(criterion)?;
        Some(p.advancements.criterion_done(i, c))
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

    /// A player's game mode id (0 survival, 1 creative, 2 adventure, 3 spectator).
    pub fn game_mode(&self, conn: ConnId) -> Option<u8> {
        self.players.get(&conn).map(|p| p.game_mode as u8)
    }

    /// A player's selected hotbar slot.
    pub fn selected_slot(&self, conn: ConnId) -> Option<usize> {
        self.players.get(&conn).map(|p| p.inv.selected)
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

    /// Makes the wandering trader spawner try at the next tick with the highest chance (for tests
    /// and tools: vanilla waits a day).
    pub fn force_trader_attempt(&mut self) {
        self.trader.tick_delay = 1;
        self.trader.spawn_delay = 1200;
        self.trader.spawn_chance = 99;
    }

    /// The wandering trader spawner's (`tickDelay`, `spawnDelay`, `spawnChance`) (for tests and tools).
    pub fn trader_spawner(&self) -> (i32, i32, i32) {
        (self.trader.tick_delay, self.trader.spawn_delay, self.trader.spawn_chance)
    }

    /// The stacks of the overworld's item entities (for tests and tools).
    pub fn item_stacks(&self) -> Vec<kiln_item::ItemStack> {
        self.dims[OVERWORLD_ID]
            .regions
            .iter()
            .flat_map(|r| r.part().0.list.iter())
            .filter(|e| !e.removed)
            .filter_map(|e| match e.phys.as_deref().map(|p| &p.kind) {
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
        let ty = menu.kind.menu_type().or(matches!(menu.kind, kiln_inventory::MenuKind::Mount { .. }).then_some("minecraft:mount"))?;
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
                    Some(container::open::OpenBlock::Cart { .. }) => p.containers.cart.items.get(s.index).cloned(),
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

    /// The stack the cursor carries in a player's open menu (for tests and tools).
    pub fn menu_carried(&self, conn: ConnId) -> Option<(&'static str, i32)> {
        let c = self.players.get(&conn)?.open_menu.as_ref()?.carried();
        (!c.is_empty()).then(|| (c.item_name(), c.count()))
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
            elytra_movement_check: self.rule_bool("minecraft:elytra_movement_check"),
            spectators_generate_chunks: self.rule_bool("minecraft:spectators_generate_chunks"),
            natural_regen: self.rule_bool("minecraft:natural_health_regeneration"),
            biome_count: self.dims[dim].provider.biome_count,
            now: Instant::now(),
            keep_alive_id: self.started.elapsed().as_millis() as i64,
            keep_alive: self.config.keep_alive,
            portal: self.portal_rules(),
            blocks: self.block_env(dim),
            frozen: !self.world.tick_rate.runs_normally(),
            border: self.world.borders[dim].bounds(),
            forced: std::sync::Arc::new(self.world.forced[dim].iter().map(|&[x, z]| ChunkPos::new(x, z)).collect()),
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
                spread_vines: self.rule_bool("minecraft:spread_vines"),
                infiniburn: kind.infiniburn.trim_start_matches('#'),
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
                creaking_active: dim == OVERWORLD_ID && mobs::creaking_active(self.day_time),
                griefing: self.rule_bool("minecraft:mob_griefing"),
                drops: self.rule_bool("minecraft:mob_drops"),
                entity_drops: self.rule_bool("minecraft:entity_drops"),
                spawn_mobs: self.rule_bool("minecraft:spawn_mobs"),
                spawn_monsters: self.rule_bool("minecraft:spawn_monsters"),
                spawn_wardens: self.rule_bool("minecraft:spawn_wardens"),
                spawn_phantoms: self.rule_bool("minecraft:spawn_phantoms"),
                universal_anger: self.rule_bool("minecraft:universal_anger"),
                forgive_dead_players: self.rule_bool("minecraft:forgive_dead_players"),
                ender_pearls_vanish: self.rule_bool("minecraft:ender_pearls_vanish_on_death"),
                explosion_decay: [
                    self.rule_bool("minecraft:block_explosion_drop_decay"),
                    self.rule_bool("minecraft:mob_explosion_drop_decay"),
                    self.rule_bool("minecraft:tnt_explosion_drop_decay"),
                ],
                global_sound_events: self.rule_bool("minecraft:global_sound_events"),
                projectiles_break_blocks: self.rule_bool("minecraft:projectiles_can_break_blocks"),
                spawner_blocks: self.rule_bool("minecraft:spawner_blocks_work"),
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
            fire_spread_radius: self.rule_int("minecraft:fire_spread_radius_around_player"),
            dragon_fight: self.fight_env(dim),
            pipeline: self.world.pipelines.get(dim).cloned().flatten(),
            // Only asked whether any is near (in no order).
            fire_watchers: std::sync::Arc::new(self.players.values().filter(|p| p.dim == dim && p.game_mode != 3).map(|p| p.pos).collect()),
            raids: self.dims[dim].raids.views.clone(),
            entity_ticking: self.config.entity_ticking,
            speculate: self.config.speculate,
            features: self.world.feature_hosts.get(dim).cloned().flatten(),
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
            keep_inventory: self.rule_bool("minecraft:keep_inventory"),
            vanishing: self.rules.equipment_drop_lock(),
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
        kind: usize,
        mut packets: BTreeMap<(DimId, RegionId), Vec<(ConnId, PlayIn)>>,
        f: impl Fn(&mut RegionWork, &Env, &kiln_sched::Ctx<'_>) + Sync,
    ) -> Vec<(DimId, RegionOut)> {
        let dt = Instant::now();
        // The levels with regions to run.
        let envs: Vec<Option<Env>> = (0..self.dims.len()).map(|d| self.dims[d].regions.iter().next().is_some().then(|| self.env(d))).collect();
        let dt = diag::lap("rr.envs", dt);
        let mut buckets: BTreeMap<(DimId, RegionId), Vec<(ConnId, &mut Player)>> = BTreeMap::new();
        for (&conn, p) in self.players.iter_mut().filter(|(_, p)| p.region != LIMBO) {
            buckets.entry((p.dim, p.region)).or_default().push((conn, p));
        }
        let mut hooks = self.plugins.as_mut().map(|pl| pl.hooks()).unwrap_or_default();
        let mut work: Vec<RegionWork> = Vec::new();
        let inject = self.config.inject_delay.as_ref();
        for (dim, d) in self.dims.iter_mut().enumerate() {
            let (_, regions) = d.regions.split_mut();
            let lent = &d.lent;
            work.extend(regions.filter(|r| !lent.contains(&r.id())).map(|r| {
                let key = (dim, r.id());
                let mut keyed = buckets.remove(&key).unwrap_or_default();
                keyed.sort_unstable_by_key(|&(conn, _)| conn);
                let conns: Vec<ConnId> = keyed.iter().map(|&(c, _)| c).collect();
                let players: Vec<&mut Player> = keyed.into_iter().map(|(_, p)| p).collect();
                let packets = packets.remove(&key).unwrap_or_default();
                let (cells, (entities, blocks)) = r.cells_and_part_mut();
                let plugins = hooks.remove(&key);
                let delay = inject.map_or(Duration::ZERO, |i| i.delay_for(dim, cells));
                RegionWork { dim, region: key.1, cells, entities, blocks, players, conns, packets, plugins, delay, out: RegionOut::default() }
            }));
        }
        debug_assert!(buckets.is_empty(), "players in regions that do not exist");
        // The pool starts the biggest first: what the region's work took lately, else a rough
        // estimate (players dominate a crowd's region, entities a spread one's).
        let last = &self.unit_costs[kind];
        let cost = |w: &RegionWork| {
            last.get(&(w.dim, w.region)).copied().unwrap_or_else(|| {
                20_000 + w.players.len() as u64 * 5_000 + w.entities.list.len() as u64 * 500 + w.packets.len() as u64 * 500
            })
        };
        // Speculation (exact either way) pays only in a region that takes longer than its share
        // of the workers' time: elsewhere the workers are busy with other regions anyway and the
        // copies only cost CPU. Once on, it stays on until the region falls well below its share
        // (a region it speeds up would otherwise flip back and forth).
        if kind == 1 {
            let costs: Vec<u64> = work.iter().map(&cost).collect();
            let total: u64 = costs.iter().sum();
            let workers = self.pool.workers() as u64;
            for (w, c) in work.iter_mut().zip(costs) {
                let pace = &mut w.entities.spec;
                pace.wanted = if pace.wanted { c * workers * 10 >= total * 6 } else { c * workers > total };
            }
        }
        let dt = diag::lap("rr.work", dt);
        let report = self.pool.run_units(&mut work, cost, |w, ctx| f(w, envs[w.dim].as_ref().expect("the environment of a level with regions"), ctx));
        diag::lap("rr.units", dt);
        self.independent.last_fork = work.iter().zip(&report.unit_ns).map(|(w, &ns)| (w.dim, w.region, ns)).collect();
        // Smoothed (a quarter of the new time), so one tick held up by the machine does not
        // reorder everything.
        let old = std::mem::take(&mut self.unit_costs[kind]);
        self.unit_costs[kind] =
            self.independent.last_fork.iter().map(|&(d, r, ns)| ((d, r), old.get(&(d, r)).map_or(ns, |&o| (o * 3 + ns) / 4))).collect();
        if kind == 1 && self.game_time % 100 == 0 && std::env::var_os("KILN_TMP_REGIONS").is_some() {
            let mut v: Vec<(u64, usize, String)> = work.iter().zip(&report.unit_ns).map(|(w, &ns)| {
                let mut c: BTreeMap<&str, usize> = BTreeMap::new();
                for e in &w.entities.list { *c.entry(e.kind.name.trim_start_matches("minecraft:")).or_default() += 1; }
                let pos = w.players.first().map(|p| [p.pos[0] as i32, p.pos[1] as i32, p.pos[2] as i32]);
                (ns / 1000, w.players.len(), format!("{pos:?} spec {} {c:?}", w.entities.spec.wanted))
            }).collect();
            v.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            for r in v.iter().take(4) { eprintln!("TMP {r:?}"); }
        }
        if let Some(&max) = report.unit_ns.iter().max() {
            diag::add(["rr.max_unit_p", "rr.max_unit"][kind], Duration::from_nanos(max));
            diag::add(["rr.sum_units_p", "rr.sum_units"][kind], Duration::from_nanos(report.unit_ns.iter().sum()));
        }
        work.into_iter().map(|w| (w.dim, w.out)).collect()
    }

    /// Packets of players in limbo wait with them; those held for players placed since go first.
    fn hold_limbo_packets(&mut self, packets: Vec<(ConnId, PlayIn)>) -> Vec<(ConnId, PlayIn)> {
        if self.held_packets.is_empty() && !self.players.values().any(|p| p.region == LIMBO) {
            return packets;
        }
        let mut all = std::mem::take(&mut self.held_packets);
        all.extend(packets);
        let players = &self.players;
        let (held, go): (Vec<_>, Vec<_>) = all.into_iter().partition(|(c, _)| players.get(c).is_some_and(|p| p.region == LIMBO));
        self.held_packets = held;
        go
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
        // A connection's packets come in a row: its key is looked up once per row, and a row
        // of one region's packets goes into its stream at once.
        let mut last: Option<(ConnId, Option<(DimId, RegionId)>)> = None;
        let mut run: Option<((DimId, RegionId), Vec<(ConnId, PlayIn)>)> = None;
        let flush = |local: &mut BTreeMap<(DimId, RegionId), Vec<(ConnId, PlayIn)>>, run: &mut Option<((DimId, RegionId), Vec<(ConnId, PlayIn)>)>| {
            if let Some((k, v)) = run.take() {
                match local.entry(k) {
                    std::collections::btree_map::Entry::Vacant(e) => {
                        e.insert(v);
                    }
                    std::collections::btree_map::Entry::Occupied(e) => e.into_mut().extend(v),
                }
            }
        };
        for (conn, pkt) in packets {
            let key = match last {
                Some((c, key)) if c == conn => key,
                _ => self.players.get(&conn).map(|p| (p.dim, p.region)),
            };
            last = Some((conn, key));
            let Some(key) = key else { continue };
            if (!stopped.is_empty() && stopped.contains(&key)) || region::is_exclusive(&pkt) {
                flush(&mut local, &mut run);
                stopped.insert(key);
                exclusive.push((conn, pkt));
            } else {
                match &mut run {
                    Some((k, v)) if *k == key => v.push((conn, pkt)),
                    _ => {
                        flush(&mut local, &mut run);
                        run = Some((key, vec![(conn, pkt)]));
                    }
                }
            }
        }
        flush(&mut local, &mut run);
        (local, exclusive)
    }

    /// Per level: unloads what the regions released, then loads what they asked for plus
    /// every player's own chunk (so each player stands in an owned cell after the regionizer
    /// runs).
    fn maintain_chunks(&mut self) {
        // Entities loaded with a chunk have their ids before the chunk can leave again.
        self.materialize_spawns();
        // Every player's own chunk, by level (one pass over the players).
        let mut own_chunks: Vec<Vec<ChunkPos>> = (0..self.dims.len()).map(|_| Vec::new()).collect();
        for p in self.players.values() {
            if let Some(list) = own_chunks.get_mut(p.dim) {
                list.push(player_chunk(p.pos));
            }
        }
        for dim in 0..self.dims.len() {
            // (Sorted and without repeats; a set only when chunks are to unload.)
            let mut keep: Vec<ChunkPos> = std::mem::take(&mut own_chunks[dim]);
            // Force-loaded chunks (`/forceload`) stay, and load like a player's own chunk.
            keep.extend(self.world.forced[dim].iter().map(|&[x, z]| ChunkPos::new(x, z)));
            // The dragon fight's arena stays while its boss bar has players (`TicketType.DRAGON`),
            // and the rest of it loads a few chunks a tick.
            if dim == END_ID && self.dragon_fight.active() {
                keep.extend(self.dragon_fight.arena());
                let mut n = 0;
                for pos in self.dragon_fight.arena() {
                    if n == 4 {
                        break;
                    }
                    let d = &mut self.dims[dim];
                    if !d.is_loaded(pos) && d.request(pos) {
                        n += 1;
                    }
                }
            }
            keep.sort_unstable();
            keep.dedup();
            let dt = Instant::now();
            let unloads = std::mem::take(&mut self.dims[dim].unloads);
            let unloaded = if unloads.is_empty() { Vec::new() } else { self.dims[dim].unload(unloads, &keep.iter().copied().collect()) };
            if !unloaded.is_empty() {
                debug!("unloaded {} chunks of {}", unloaded.len(), self.dims[dim].key);
                let owners = self.owner_uuids();
                let gone = self.dims[dim].store_entities(&unloaded, false, &owners);
                self.forget_entities(gone);
            }
            let dt = diag::lap("ch.unload", dt);
            let d = &mut self.dims[dim];
            d.install_generated(&keep);
            let dt = diag::lap("ch.install_generated", dt);
            // Every player's own chunk, uncapped: each player must stand in an owned cell (or
            // waits in limbo while it is generated).
            for pos in keep {
                d.request_urgent(pos);
            }
            let dt = diag::lap("ch.own", dt);
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
            diag::lap("ch.requests", dt);
        }
    }

    /// Runs every level's regionizer; whether any topology changed.
    fn apply_topology(&mut self) -> bool {
        let tick = self.game_time as u64;
        let mut changed = false;
        for d in &mut self.dims {
            changed |= d.apply_topology(tick);
        }
        self.sync_plugin_regions();
        changed
    }

    /// Puts every player in the region that owns its cell, and ends pairings between players
    /// that ended up in different regions.
    fn update_membership(&mut self, topology_changed: bool) {
        let mut moved = topology_changed;
        let Sim { players, dims, .. } = self;
        for p in players.values_mut() {
            // No owner: the chunk is still being generated.
            let r = dims[p.dim].regions.owner(player_chunk(p.pos).cell()).unwrap_or(LIMBO);
            if r != p.region {
                p.region = r;
                moved = true;
            }
        }
        if moved {
            self.drop_cross_region_pairs();
            self.drop_cross_region_viewers();
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
            stray.dedup();
            let mut loaded = false;
            for (d, c) in stray {
                loaded |= self.dims[d].request_urgent(c);
            }
            loaded && self.apply_topology()
        };
        self.update_membership(changed);
    }

    /// Gives the entities spawned since the last call their ids, in an order that does not
    /// depend on the regions (level by level), and puts each in the region owning its cell
    /// (spawns in unloaded chunks are dropped, as vanilla would not add them).
    fn materialize_spawns(&mut self) {
        let world_seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        // What `finalizeSpawn` does to a new mob's equipment reads the datapack's providers.
        let _enchanting = enchant::install_enchanter(self.loot.as_ref());
        for d in &mut self.dims {
            let mut later = Vec::new();
            // Entities built during a tick carry placeholder (negative) ids that other new
            // entities refer to as their vehicle or passengers: (placeholder, id, chunk).
            let mut placeholders: Vec<(i32, i32, ChunkPos)> = Vec::new();
            for spawn in entities::canonical(std::mem::take(&mut d.spawns)) {
                let chunk = entities::chunk_of(spawn.pos);
                // Into a region ticking away: once it is back.
                if d.lent_at(chunk) {
                    later.push(spawn);
                    continue;
                }
                let Some(region) = d.regions.at_mut(chunk.cell()) else {
                    // A loaded entity outside its chunk's loaded area goes back to storage.
                    match spawn.body {
                        entities::Body::Loaded(e) => {
                            let tag = kiln_entity::persist::save(&e, &|_| None);
                            d.stash_entities(chunk, vec![tag]);
                        }
                        entities::Body::LoadedStack(e, riders) => {
                            let tag = kiln_entity::persist::save_stack(&e, &riders, &|_| None);
                            d.stash_entities(chunk, vec![tag]);
                        }
                        _ => {}
                    }
                    continue;
                };
                let id = self.next_entity_id;
                self.next_entity_id += 1;
                // The ender dragon's parts take the next eight ids (the client numbers them so).
                if spawn.kind.name == "minecraft:ender_dragon" {
                    self.next_entity_id += 8;
                }
                let uuid = entities::fresh_uuid(world_seed, self.game_time, id);
                if let entities::Body::Ready(e) = &spawn.body
                    && e.id < 0
                {
                    placeholders.push((e.id, id, chunk));
                }
                let list = &mut region.part_mut().0.list;
                list.push(entities::Entity::new(id, uuid, spawn));
                // What `finalizeSpawn` made along with the mob (its jockeys) joins right after it.
                if let Some(j) = list.last_mut().and_then(|e| e.jockeys.take()) {
                    entities::add_jockeys(list, id, *j, &mut self.next_entity_id, world_seed, self.game_time);
                }
            }
            d.spawns = later;
            if !placeholders.is_empty() {
                Self::resolve_placeholders(d, &placeholders);
            }
        }
        // Riders that joined with a saved `RootVehicle`.
        if self.players.values().any(|p| p.returning_vehicle.is_some()) {
            self.seat_returning_players();
        }
    }

    /// Replaces the placeholder ids of `placeholders` in the vehicles and passengers of the
    /// entities just added (and of the vehicles they ride, which may be older entities of the
    /// same region: a skeleton trap's rider sits on the trap horse).
    fn resolve_placeholders(d: &mut Dim, placeholders: &[(i32, i32, ChunkPos)]) {
        let real = |id: i32| placeholders.iter().find(|p| p.0 == id).map_or(id, |p| p.1);
        for &(placeholder, id, chunk) in placeholders {
            let Some(region) = d.regions.at_mut(chunk.cell()) else { continue };
            let list = &mut region.part_mut().0.list;
            let Ok(i) = list.binary_search_by_key(&id, |e| e.id) else { continue };
            let vehicle = list[i].phys.as_deref_mut().and_then(|p| {
                p.vehicle = p.vehicle.map(real);
                for x in p.passengers.iter_mut() {
                    *x = real(*x);
                }
                p.vehicle
            });
            if let Some(v) = vehicle
                && let Ok(j) = list.binary_search_by_key(&v, |e| e.id)
                && let Some(vp) = list[j].phys.as_deref_mut()
            {
                for x in vp.passengers.iter_mut() {
                    if *x == placeholder {
                        *x = id;
                    }
                }
            }
        }
        // Leads tied to entities that were not in the level yet (a new knot, a trader): the
        // region's led entities find their holder by its real id.
        let mut cells = Vec::new();
        for &(_, _, chunk) in placeholders {
            let cell = chunk.cell();
            if cells.contains(&cell) {
                continue;
            }
            cells.push(cell);
            let Some(region) = d.regions.at_mut(cell) else { continue };
            for e in region.part_mut().0.list.iter_mut() {
                if let Some(l) = e.phys.as_deref_mut().and_then(|p| p.leash.as_mut())
                    && let Some(h) = l.holder
                    && h < 0
                {
                    l.holder = Some(real(h));
                }
            }
        }
    }

    /// Death messages to everyone (`show_death_messages`), in the order the deaths happened.
    fn announce_deaths(&mut self, deaths: Vec<health::Death>) {
        for d in &deaths {
            self.award_kill_score(d);
        }
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
        let weather = self.level_info_packets(dim);
        let rules = self.rules.clone();
        let keep_inventory = self.rule_bool("minecraft:keep_inventory");
        // Viewers in the old level saw the death (or the player walk into the portal): they
        // forget it and get the entity again once tracking re-evaluates it.
        self.untrack_everywhere(conn);
        self.post_effects_pending = true;
        let p = self.players.get_mut(&conn).unwrap();
        p.send(info_packet);
        // `PlayerList.respawn` sends the post effects again; the new player starts its first
        // tick (no waypoint until it has moved).
        p.post_effects_dirty = true;
        p.waypoint_first_tick = true;
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
            // (`restoreFrom` keeps the experience of a player that keeps its inventory.)
            if !keep_inventory {
                p.xp_level = 0;
                p.xp_progress = 0.0;
                p.xp_total = 0;
            }
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
        if self.dims[dim].request_urgent(chunk) {
            self.apply_topology();
        }
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
            PlayIn::ChatCommand { command } => {
                if !self.plugin_command_denied(conn, &command) {
                    self.run_command(conn, &command);
                }
            }
            PlayIn::CommandSuggestion { id, text } => self.suggest(conn, id, text),
            PlayIn::ResourcePack { id, action } => self.resource_pack_response(conn, id, action),
            PlayIn::CookieResponse(response) => self.cookie_response(conn, response),
            PlayIn::Chat { message } => {
                let Some(p) = self.players.get_mut(&conn) else { return };
                if commands::has_illegal_chars(&message) {
                    p.disconnect("Illegal characters in chat");
                    return;
                }
                if self.plugin_chat(conn, &message) {
                    return;
                }
                let Some(p) = self.players.get(&conn) else { return };
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
                let mut hook = self.plugins.as_mut().and_then(|pl| pl.hook(dim, id));
                if let Some(h) = hook.as_mut()
                    && plugins::deny_packet(h, p, cells, &env, &pkt, &mut d.spawns)
                {
                    return;
                }
                let mut world = region::World { cells: &mut *cells, blocks: &mut part.1 };
                let mut fx = region::Fx { blocks: &mut out, bodies: &bodies, spawns: &mut d.spawns, deaths: &mut deaths };
                let cart = carts::pull(&part.0, p);
                region::local_packet(p, &mut world, &env, pkt, &mut fx);
                carts::push(&mut part.0, p, cart);
                if let Some(h) = hook.as_mut() {
                    plugins::after_packets(h, cells, &env);
                }
                let mut everyone: Vec<&mut Player> = self.players.values_mut().filter(|p| p.dim == dim).collect();
                blocks::finish(cells, out, &mut everyone, &mut d.spawns, &env.blocks);
                self.announce_deaths(deaths);
            }
        }
    }

    fn leave(&mut self, conn: ConnId) {
        self.dragon_fight_left(conn);
        // `ServerPlayer.disconnect`: a sleeper gets out of bed first.
        if let Some(mut p) = self.players.remove(&conn) {
            if p.sleep.pos.is_some() {
                let (dim, pos) = (p.dim, p.pos.map(|c| c.floor() as i32));
                self.with_level_in(dim, pos, |level| sleep::stop_sleep_in_bed(&mut p, level, true, false));
            }
            self.sleep_status[p.dim].dirty = true;
            // `PlayerList.remove`.
            p.award_stat(*player_stats::stat::LEAVE_GAME, 1);
            self.commands.bossbars.player_left(p.uuid);
            if let Some(dim) = p.waypoint_dim {
                self.waypoints_remove_player(dim, conn, p.uuid);
            }
            self.save_player(&p);
            self.player_stops_riding(&p);
            self.plugins_left(&p);
            self.announce_leave(&p, conn);
            self.broadcast_system(yellow(&format!("{} left the game", p.name)));
        }
    }

    fn shut_down(&mut self) {
        stats::flush_trace();
        info!("{}", chunkstats::line(self.config.noise.as_ref().map_or(0, |n| n.threads)));
        self.rendezvous();
        for p in self.players.values_mut() {
            p.disconnect("Server closed");
        }
        self.save();
        // Compactions still copying cell files in the background finish and swap in before the
        // server stops, so a store opened right after (a restart, a tool) sees the final files.
        for d in &self.dims {
            if let Some(store) = &d.native {
                store.lock().unwrap().finish_compactions();
            }
        }
    }

    fn save(&mut self) {
        // Everything is saved together: regions ticking away come back first.
        self.rendezvous();
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
                        if let (Some(store), Some(p)) = (d.poi_store.as_mut(), chunk.pois.as_deref_mut())
                            && p.dirty
                        {
                            store.store(pos, Some(p.to_nbt(kiln_storage::anvil::DATA_VERSION as i32)));
                            p.dirty = false;
                        }
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
            if let Some(Err(e)) = self.dims[dim].poi_store.as_mut().map(kiln_storage::PoiStore::flush) {
                warn!("saving points of interest failed: {e}");
            }
        }
        for p in self.players.values() {
            self.save_player(p);
        }
        self.save_level();
        self.save_weather();
        self.save_world_state();
        self.save_raids();
        self.save_trader();
        self.save_scoreboard();
        self.save_stopwatches();
        self.save_timers();
        self.save_dragon_fight();
        self.save_plugins();
    }

    fn join(&mut self, j: JoinInfo, joining: persist::Joining) {
        self.post_effects_pending = true;
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        let spawn = joining.pos;
        let [yaw, pitch] = joining.rot;
        let view_distance = (j.client.view_distance as i32).min(self.config.view_distance as i32);
        let move_state = packets::entity::MoveState { pos: spawn, yaw, pitch, head_yaw: yaw, on_ground: true };
        let dim = joining.dim;
        let dimension_type = kiln_data::synced_id("minecraft:dimension_type", DIMENSIONS[dim].0).expect("dimension type");
        let region = self.dims[dim].regions.owner(player_chunk(spawn).cell()).expect("spawn chunk loaded");
        let mut recipe_book = recipe_book::RecipeBook::load(joining.saved.raw().get("recipeBook"));
        let shoulders = [shoulder::load(joining.saved.raw(), "ShoulderEntityLeft"), shoulder::load(joining.saved.raw(), "ShoulderEntityRight")];
        let returning_vehicle = persist::returning_vehicle(joining.saved.raw().get("RootVehicle"));
        recipe_book.retain_existing(&self.rules);
        let warden_tracker = sculk::shrieker::WardenSpawnTracker::load(joining.saved.raw().get("warden_spawn_tracker"));
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
            address: j.address,
            last_action: Instant::now(),
            post_effects: persist::saved_post_effects(joining.saved.raw()),
            post_effects_dirty: true,
            waypoint_icon: persist::saved_waypoint_icon(joining.saved.raw()),
            waypoint_dim: None,
            waypoint_first_tick: true,
            waypoint_last_pos: spawn,
            outbox: Vec::new(),
            disconnected: false,
            region,
            pos: spawn,
            rot: joining.rot,
            on_ground: true,
            horizontal_collision: false,
            view_distance,
            center: player_chunk(spawn),
            sent_chunks: FastSet::default(),
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
            command_slots: Default::default(),
            tracker: packets::entity::MovementTracker::new(
                entity_id,
                kiln_data::entities::types::PLAYER.update_interval,
                &move_state,
            ),
            seen_by: Vec::new(),
            section: None,
            sneaking: false,
            sprinting: false,
            fall_flying: false,
            fall_fly_ticks: 0,
            spin_ticks: 0,
            spin_damage: 0.0,
            spin_item: kiln_item::ItemStack::empty(),
            spin_off_hand: false,
            spin_check: false,
            spin_pose: false,
            meta_dirty: false,
            swung: false,
            swing_ticks: 0,
            swing_duration: 0,
            swing_kind: kiln_proto::packets::entity::swing::WHACK,
            swing_wire_duration: kiln_proto::packets::entity::swing::DEFAULT_DURATION,
            kinetic_ticks: None,
            recent_stabs: Vec::new(),
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
            pending_sounds: Vec::new(),
            sound_seed: kiln_javamath::random::LegacyRandom::new(!(j.uuid.as_u64_pair().0 as i64)),
            health: joining.health,
            food: joining.food,
            saturation: joining.saturation,
            fall_distance: 0.0,
            main_supporting_block: None,
            on_ground_no_blocks: false,
            was_touching_water: false,
            phantom: None,
            server_delta: [0.0; 3],
            stuck_speed: [0.0; 3],
            movements: Vec::new(),
            was_eye_in_water: false,
            ticks_frozen: 0,
            is_in_powder_snow: false,
            frost_speed: None,
            block_edits: Vec::new(),
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
            push_sync: false,
            shoulders,
            shoulder_time: 0,
            shoulder_dirty: false,
            released_shoulders: Vec::new(),
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
            item_cooldowns: Vec::new(),
            digging: None,
            delayed_destroy: None,
            lobby: lobby::PlayerLobby::default(),
            portal: None,
            portal_cooldown: joining.portal_cooldown,
            won_game: false,
            seen_credits: joining.seen_credits,
            pending_travel: None,
            vehicle: None,
            vehicle_type: None,
            dismount_on_teleport: false,

            returning_vehicle,
            levitation_start: None,
            raid_omen_position: None,
            raid_omen_trigger: None,
            omen_village: false,
            omen_raid_full: false,
            starting_to_fall: None,
            entered_nether: None,
            entered_lava_on_vehicle: None,
            warden_tracker,
            last_hurt_by_mob: None,
            last_hurt_mob: None,
            sleep: sleep::Sleep::default(),
            woke_up: false,
            respawn_angle: joining.respawn_angle,
            respawn_forced: joining.respawn_forced,
            last_mob_attacker: None,
            stats: self.load_stats(j.uuid),
            recipe_book,
            advancements: self.load_player_advancements(j.uuid),
            command_attributes: combat::CommandAttributes::default(),
            limited_crafting: self.rule_bool("minecraft:limited_crafting"),
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
            hashed_seed: self.zoom_seed,
            hardcore: self.storage.as_ref().is_some_and(|s| s.level.hardcore()),
            reduced_debug_info: self.rule_bool("minecraft:reduced_debug_info"),
            show_death_screen: !self.rule_bool("minecraft:immediate_respawn"),
            limited_crafting: self.rule_bool("minecraft:limited_crafting"),
        }));
        // `PlayerList.placeNewPlayer`: the difficulty follows the login.
        player.send(packets::change_difficulty(self.commands.difficulty as u8, self.commands.difficulty_locked));
        player.send(packets::player_position(player.teleport_id, spawn, yaw, pitch));
        let [spawn_yaw, spawn_pitch] = self.spawn_rot;
        player.send(packets::set_default_spawn_position(OVERWORLD, self.spawn, spawn_yaw, spawn_pitch));
        player.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        player.send(packets::set_chunk_cache_center(player.center.x, player.center.z));
        player.send(self.world.borders[dim].init_packet());
        player.send(self.time_packet());
        // `PlayerList.sendLevelInfo`: the weather of the player's level.
        for pkt in self.weather_packets(player.dim) {
            player.send(pkt);
        }
        // `ServerTickRateManager.updateJoiningPlayer`.
        player.send(self.world.tick_rate.state_packet());
        player.send(self.world.tick_rate.step_packet());
        player.send(packets::set_held_slot(player.inv.selected as i32));
        player.sync_health();
        // `PlayerList.placeNewPlayer`: the saved effects.
        player.send_all_effects();
        player.send(kiln_inventory::recipe::sync::update_recipes(&self.rules.recipes));
        player.send_initial_recipe_book(&self.rules);
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
            hashed_seed: self.zoom_seed,
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
        // Players of regions ticking away get it when they are back.
        self.independent.mail(&pkt);
    }

    /// G: world age and time, autosave.
    fn tick_global(&mut self) {
        let mut mark = Instant::now();
        let mut lap = |stats: &mut stats::TickStats, name| {
            let now = Instant::now();
            stats.phase(name, now - mark);
            mark = now;
        };
        // A frozen game (`/tick freeze`) keeps its time, weather and border; functions run.
        let normal = self.world.tick_rate.runs_normally();
        if normal {
            self.game_time += 1;
            for d in &mut self.dims {
                d.game_time = self.game_time;
            }
        }
        self.tick_functions();
        self.tick_gametests();
        if normal {
            self.tick_clocks();
        }
        if normal && self.game_time % 20 == 0 {
            let pkt = self.time_packet();
            self.broadcast(pkt);
        }
        // The levels' `tick`: the border, the weather, sleeping, then (in the regions) the
        // blocks.
        lap(&mut self.stats, "g.time");
        self.update_sleeping();
        if normal {
            self.tick_borders();
            self.tick_weather();
        }
        lap(&mut self.stats, "g.border_weather");
        self.tick_sleep();
        lap(&mut self.stats, "g.sleep");
        self.send_post_effects();
        lap(&mut self.stats, "g.effects");
        self.tick_waypoints();
        lap(&mut self.stats, "g.waypoints");
        self.tick_raids();
        self.tick_dragon_fight();
        lap(&mut self.stats, "g.raids_dragon");
        // `save-all` asks for a save; `save-off` stops the autosave.
        let autosave = normal && self.commands.auto_save && self.game_time % AUTOSAVE_TICKS == 0;
        if std::mem::take(&mut self.commands.save_requested) || autosave {
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
