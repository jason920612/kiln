//! Block behaviour in regions (design §4.3): each region carries the per-level machinery
//! vanilla keeps on `ServerLevel` (scheduled block and fluid ticks by chunk, block events,
//! moving pistons, the level random) and runs kiln-blocks against its own cells through
//! [`RegionLevel`].
//!
//! Partition independence: scheduled ticks, block events and moving pistons belong to the
//! chunk they are in and follow merges and splits; random ticks draw from a random seeded
//! per chunk and game tick, so they do not depend on which region a chunk is in. The level
//! random that scheduled behaviour reads (lava spread delays, piston sound pitch) is kept per
//! region, an approximation (I class).
//!
//! Falling blocks and primed TNT become kiln-entity entities (spawned from the effects in
//! [`finish`]); without loot tables a broken block drops its own item.

use crate::Player;
use crate::entities::{self, Spawn};
use bytes::Bytes;
use kiln_blocks::level::UpdateTrace;
use kiln_blocks::ticks::{ChunkKey, ticks_from_nbt, ticks_to_nbt};
use kiln_blocks::{BlockId, BlockPos, ChunkTicks, Direction, Effect, EntityKind, FluidType, Level, LevelData, LevelTicks, flags};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_link::ConnId;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::{self, world_fx};
use kiln_region::{CellPos, CellSet, RegionPart};
use kiln_world::chunk::{Chunk, LightLayer, SavedTicks};
use kiln_world::{Blocks, Cell, CellStore, ChunkPos};
use smallvec::SmallVec;
use std::collections::BTreeMap;

/// `max-chained-neighbor-updates` default.
const MAX_CHAINED_NEIGHBOR_UPDATES: i32 = 1_000_000;
/// Chunks per cell side.
const CELL_CHUNKS: i32 = kiln_region::CELL_BLOCKS / 16;
/// `Level.MAX_LEVEL_SIZE`: blocks beyond ±30M are outside the world.
const MAX_LEVEL_SIZE: i32 = 30_000_000;

/// A region's share of the level's block machinery.
pub(crate) struct RegionBlocks {
    pub block_ticks: LevelTicks<BlockId>,
    pub fluid_ticks: LevelTicks<FluidType>,
    pub data: LevelData,
    pub random: LegacyRandom,
    sub_tick: i64,
    /// Generation's post-processing and ticks of new chunks, applied once the chunks around
    /// them are loaded (`LevelChunk.postProcessGeneration`, `unpackTicks`).
    generated: Vec<(ChunkPos, kiln_world::chunk::PendingUpdates)>,
    /// Container block entities of the region's chunks, live.
    pub containers: crate::container::Containers,
    /// Raider news for the level's raids, until the next raid tick takes them.
    pub raid_events: Vec<kiln_entity::level::RaidEvent>,
    /// Texts of display entities that wait to be resolved: (entity uuid, text).
    pub text_requests: Vec<(u128, kiln_proto::nbt::Tag)>,
    /// Hives whose nearby bees take a player as their target (`BeehiveBlock.angerNearbyBees`),
    /// for the region, which has the entities.
    pub bee_anger: Vec<BlockPos>,
    /// Game event listeners: sculk block entities and wardens.
    pub sculk: crate::sculk::Sculk,
    /// Creaking heart block entities.
    pub hearts: crate::heart::Hearts,
    /// Who may edit which sign (`SignBlockEntity.playerWhoMayEdit`).
    pub sign_editors: crate::signs::SignEditors,
    /// Mob spawner block entities.
    pub spawners: crate::mob_spawner::Spawners,
    /// New chunks of generation that still want their animals.
    pub initial_mobs: Vec<ChunkPos>,
    /// Command blocks whose scheduled tick came this tick (the serial phase runs their commands).
    pub command_ticks: Vec<BlockPos>,
}

impl Default for RegionBlocks {
    fn default() -> Self {
        Self {
            block_ticks: LevelTicks::new(),
            fluid_ticks: LevelTicks::new(),
            data: LevelData::new(MAX_CHAINED_NEIGHBOR_UPDATES, 0),
            random: LegacyRandom::new(0),
            sub_tick: 0,
            generated: Vec::new(),
            containers: Default::default(),
            raid_events: Vec::new(),
            text_requests: Vec::new(),
            bee_anger: Vec::new(),
            sculk: Default::default(),
            hearts: Default::default(),
            sign_editors: Default::default(),
            spawners: Default::default(),
            initial_mobs: Vec::new(),
            command_ticks: Vec::new(),
        }
    }
}

fn key(pos: ChunkPos) -> ChunkKey {
    (pos.x, pos.z)
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

impl RegionBlocks {
    /// A chunk entered the region's cells: its saved ticks become scheduled and its moving
    /// pistons start ticking.
    pub fn chunk_loaded(&mut self, pos: ChunkPos, chunk: &mut Chunk, game_time: i64) {
        let k = key(pos);
        let (block, fluid) = match chunk.saved_ticks.take() {
            Some(t) => (ticks_from_nbt(&t.block, k, BlockId::by_name), ticks_from_nbt(&t.fluid, k, FluidType::from_name)),
            None => (Vec::new(), Vec::new()),
        };
        self.block_ticks.add_container(k, ChunkTicks::from_saved(block));
        self.block_ticks.unpack(k, game_time);
        self.fluid_ticks.add_container(k, ChunkTicks::from_saved(fluid));
        self.fluid_ticks.unpack(k, game_time);
        if let Some(pending) = chunk.take_pending_updates() {
            self.generated.push((pos, pending));
        }
        self.containers.chunk_loaded(pos, chunk);
        self.sculk.chunk_loaded(pos, chunk);
        self.hearts.chunk_loaded(pos, chunk);
        self.spawners.chunk_loaded(pos, chunk);
        if std::mem::take(&mut chunk.original_mobs) {
            self.initial_mobs.push(pos);
        }
        let moving = kiln_data::blocks::default_state::MOVING_PISTON;
        for ((x, y, z), be) in chunk.block_entities() {
            if chunk.get(x, y, z) == moving {
                let at = BlockPos::new(pos.x * 16 + x as i32, y, pos.z * 16 + z as i32);
                self.data.pistons.insert(at, kiln_blocks::MovingPiston::from_nbt(&be.nbt));
            }
        }
    }

    /// The region rejoins the server's clock `delta` ticks after its own (it ticked away
    /// and missed them): its scheduled ticks keep their distance in region ticks (REG-02).
    pub fn shift_time(&mut self, delta: i64) {
        self.block_ticks.shift(delta);
        self.fluid_ticks.shift(delta);
    }

    /// A chunk leaves the region's cells: its ticks and moving pistons go onto it for saving.
    pub fn chunk_unloaded(&mut self, pos: ChunkPos, chunk: &mut Chunk, game_time: i64) {
        self.store(pos, chunk, game_time);
        self.block_ticks.remove_container(key(pos));
        self.fluid_ticks.remove_container(key(pos));
        // Not applied yet (its neighbours never loaded): generation's updates are dropped.
        self.generated.retain(|(p, _)| *p != pos);
        let gone: Vec<BlockPos> = self.data.pistons.iter().map(|(p, _)| p).filter(|&p| chunk_of(p) == pos).collect();
        for p in gone {
            self.data.pistons.remove(p);
        }
        self.containers.chunk_unloaded(pos);
        self.sculk.chunk_unloaded(pos);
        self.hearts.chunk_unloaded(pos);
        self.initial_mobs.retain(|p| *p != pos);
        self.spawners.chunk_unloaded(pos);
    }

    /// Puts the chunk's scheduled ticks and moving pistons on it in their saved form.
    pub fn store(&mut self, pos: ChunkPos, chunk: &mut Chunk, game_time: i64) {
        self.containers.store(pos, chunk);
        self.sculk.store(pos, chunk);
        self.hearts.store(pos, chunk);
        self.spawners.store(pos, chunk);
        let k = key(pos);
        let block = self.block_ticks.container(k).map(|c| c.pack(game_time)).unwrap_or_default();
        let fluid = self.fluid_ticks.container(k).map(|c| c.pack(game_time)).unwrap_or_default();
        if !block.is_empty() || !fluid.is_empty() || chunk.saved_ticks.is_some() {
            chunk.saved_ticks = Some(Box::new(SavedTicks {
                block: ticks_to_nbt(&block, BlockId::name),
                fluid: ticks_to_nbt(&fluid, FluidType::name),
            }));
        }
        for (p, m) in self.data.pistons.iter().filter(|(p, _)| chunk_of(*p) == pos) {
            let (x, z) = ((p.x & 15) as usize, (p.z & 15) as usize);
            let Some(mut be) = chunk.block_entity(x, p.y, z).cloned() else { continue };
            if let (Tag::Compound(fields), Tag::Compound(extra)) = (&mut be.nbt, m.to_nbt()) {
                for (k, v) in extra {
                    match fields.iter_mut().find(|(f, _)| *f == k) {
                        Some((_, old)) => *old = v,
                        None => fields.push((k, v)),
                    }
                }
            }
            chunk.load_block_entity(x, p.y, z, be);
        }
    }
}

fn move_containers<T: Copy + Eq + std::hash::Hash>(from: &mut LevelTicks<T>, parts: &mut [&mut LevelTicks<T>], owner: impl Fn(ChunkKey) -> usize) {
    let mut chunks: Vec<ChunkKey> = from.chunks().collect();
    chunks.sort_unstable();
    for k in chunks {
        let c = from.remove_container(k).expect("listed container");
        parts[owner(k)].add_container(k, c);
    }
}

impl RegionPart for RegionBlocks {
    fn merge(into: &mut Self, mut from: Self) {
        move_containers(&mut from.block_ticks, &mut [&mut into.block_ticks], |_| 0);
        move_containers(&mut from.fluid_ticks, &mut [&mut into.fluid_ticks], |_| 0);
        for (p, m) in from.data.pistons.take_all() {
            into.data.pistons.insert(p, m);
        }
        for e in from.data.block_events.take_all() {
            into.data.block_events.push(e);
        }
        into.data.torch_toggles.append(&mut from.data.torch_toggles);
        into.generated.append(&mut from.generated);
        into.sub_tick = into.sub_tick.max(from.sub_tick);
        into.containers.merge(std::mem::take(&mut from.containers));
        into.raid_events.append(&mut from.raid_events);
        into.text_requests.append(&mut from.text_requests);
        into.bee_anger.append(&mut from.bee_anger);
        into.sculk.merge(std::mem::take(&mut from.sculk));
        into.hearts.merge(std::mem::take(&mut from.hearts));
        into.sign_editors.merge(std::mem::take(&mut from.sign_editors));
        into.spawners.merge(std::mem::take(&mut from.spawners));
        into.initial_mobs.append(&mut from.initial_mobs);
        into.command_ticks.append(&mut from.command_ticks);
    }

    fn split(mut self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        let mut parts: SmallVec<[Self; 4]> = (0..n)
            .map(|i| Self {
                random: LegacyRandom::new(self.random.next_long() ^ i as i64),
                sub_tick: self.sub_tick,
                ..Self::default()
            })
            .collect();
        let owner = |k: ChunkKey| owner_of(ChunkPos::new(k.0, k.1).cell());
        {
            let mut ticks: SmallVec<[&mut LevelTicks<BlockId>; 4]> = parts.iter_mut().map(|p| &mut p.block_ticks).collect();
            move_containers(&mut self.block_ticks, &mut ticks, owner);
        }
        {
            let mut ticks: SmallVec<[&mut LevelTicks<FluidType>; 4]> = parts.iter_mut().map(|p| &mut p.fluid_ticks).collect();
            move_containers(&mut self.fluid_ticks, &mut ticks, owner);
        }
        let at = |p: BlockPos| owner(p.chunk());
        for (p, m) in self.data.pistons.take_all() {
            parts[at(p)].data.pistons.insert(p, m);
        }
        for e in self.data.block_events.take_all() {
            parts[at(e.pos)].data.block_events.push(e);
        }
        for t in self.data.torch_toggles.drain(..) {
            parts[at(t.pos)].data.torch_toggles.push(t);
        }
        for (c, pending) in self.generated.drain(..) {
            parts[owner((c.x, c.z))].generated.push((c, pending));
        }
        {
            let mut containers: SmallVec<[&mut crate::container::Containers; 4]> = parts.iter_mut().map(|p| &mut p.containers).collect();
            self.containers.split_into(&mut containers, |c| owner((c.x, c.z)));
        }
        parts[0].raid_events = std::mem::take(&mut self.raid_events);
        parts[0].text_requests = std::mem::take(&mut self.text_requests);
        parts[0].bee_anger = std::mem::take(&mut self.bee_anger);
        {
            let mut sculk: SmallVec<[&mut crate::sculk::Sculk; 4]> = parts.iter_mut().map(|p| &mut p.sculk).collect();
            self.sculk.split_into(&mut sculk, |c| owner((c.x, c.z)));
        }
        {
            let mut hearts: SmallVec<[&mut crate::heart::Hearts; 4]> = parts.iter_mut().map(|p| &mut p.hearts).collect();
            self.hearts.split_into(&mut hearts, |c| owner((c.x, c.z)));
        }
        {
            let mut editors: SmallVec<[&mut crate::signs::SignEditors; 4]> = parts.iter_mut().map(|p| &mut p.sign_editors).collect();
            self.sign_editors.split_into(&mut editors, |p| owner(p.chunk()));
        }
        {
            let mut spawners: SmallVec<[&mut crate::mob_spawner::Spawners; 4]> = parts.iter_mut().map(|p| &mut p.spawners).collect();
            self.spawners.split_into(&mut spawners, |c| owner((c.x, c.z)));
        }
        for c in self.initial_mobs.drain(..) {
            parts[owner((c.x, c.z))].initial_mobs.push(c);
        }
        for p in self.command_ticks.drain(..) {
            parts[owner((p.x >> 4, p.z >> 4))].command_ticks.push(p);
        }
        parts[0].random = self.random;
        parts[0].data.rand_value = self.data.rand_value;
        parts
    }

    fn count(&self) -> usize {
        self.block_ticks.chunks().count() + self.fluid_ticks.chunks().count() + self.data.pistons.len() + self.data.block_events.len() + self.containers.len() + self.sculk.len() + self.hearts.len() + self.sign_editors.len() + self.spawners.len() + self.initial_mobs.len()
    }

    fn for_each_cell(&self, f: &mut dyn FnMut(CellPos)) {
        for k in self.block_ticks.chunks().chain(self.fluid_ticks.chunks()) {
            f(ChunkPos::new(k.0, k.1).cell());
        }
    }
}

/// Per-tick values block behaviour reads.
#[derive(Clone)]
pub(crate) struct BlockEnv {
    pub game_time: i64,
    pub rules: kiln_blocks::Rules,
    /// The level these blocks are in.
    pub dim: crate::DimId,
    pub min_y: i32,
    pub height: i32,
    /// `minecraft:random_tick_speed`.
    pub random_tick_speed: i32,
    /// `minecraft:block_drops`.
    pub drops: bool,
    /// Chunks within this distance of a player tick.
    pub simulation_distance: i32,
    /// Seeds the per-chunk random-tick randoms.
    pub seed: i64,
    /// Loot tables for block drops (`None`: blocks drop their own item).
    pub loot: Option<std::sync::Arc<kiln_loot::LootData>>,
    /// Game rules and difficulty for damage to players.
    pub damage: crate::health::DamageRules,
    /// Time of day and game rules for mobs.
    pub mobs: crate::mobs::MobRules,
    /// Biome spawn lists for natural spawning (`None`: no datapack, no natural spawning).
    pub spawn_table: Option<std::sync::Arc<crate::spawner::SpawnTable>>,
    /// Recipes and item rules for menus and furnaces.
    pub menus: std::sync::Arc<kiln_inventory::Rules>,
    /// The trial spawner configs the datapack holds.
    pub trial_configs: std::sync::Arc<crate::mob_spawner::TrialConfigs>,
    /// The level's weather and the biome climates.
    pub weather: crate::weather::WeatherEnv,
    /// `minecraft:fire_spread_radius_around_player` (-1: everywhere).
    pub fire_spread_radius: i32,
    /// `minecraft:send_command_feedback` (a new command block tracks its output by it).
    pub send_command_feedback: bool,
    /// Where the level's non-spectator players stood when the tick began (fire spreads near
    /// them; the same in every region).
    pub fire_watchers: std::sync::Arc<Vec<[f64; 3]>>,
    /// The level's players as they stood when the tick began (vaults detect them).
    pub players: std::sync::Arc<Vec<crate::vault::Near>>,
    /// The level's raids as they stood when the tick began.
    pub raids: std::sync::Arc<Vec<kiln_entity::level::RaidView>>,
    /// The End's dragon fight as the level's entities see it (`None` elsewhere).
    pub dragon_fight: Option<crate::dragon_fight::FightEnv>,
    /// The level's generation pipeline, for the eye of ender's `findNearestMapStructure`
    /// (`None` without generated terrain).
    pub pipeline: Option<std::sync::Arc<kiln_worldgen::pipeline::Pipeline>>,
    /// How a crowded region's entities tick.
    pub entity_ticking: crate::EntityTicking,
    /// Serial entity turns tried side by side first (`SimConfig::speculate`).
    pub speculate: bool,
    /// The level's worldgen, for what grows in it (saplings, bone meal); `None` without a
    /// datapack (nothing grows then).
    pub features: Option<std::sync::Arc<dyn kiln_blocks::feature_host::FeatureHost>>,
}

/// An entity's box for block behaviour that counts entities (pressure plates).
#[derive(Clone, Copy)]
pub(crate) struct EntityBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub living: bool,
    /// `Entity.blocksBuilding`: placed blocks may not overlap it (living entities).
    pub blocks_building: bool,
    /// The player's connection, for a player.
    pub conn: Option<ConnId>,
    /// A monster that keeps players from sleeping nearby (`Monster.isPreventingPlayerRest`).
    pub prevents_rest: bool,
    /// A player as the source of the game events it causes.
    pub player_source: Option<kiln_entity::vibration::EventSource>,
    /// A hanging entity (item frame, painting): its facing and type.
    pub hanging: Option<(kiln_entity::math::Direction, &'static str)>,
}

impl EntityBox {
    pub(crate) fn intersects(&self, min: [f64; 3], max: [f64; 3]) -> bool {
        (0..3).all(|i| self.min[i] < max[i] && self.max[i] > min[i])
    }
}

/// The boxes of a region's players (not spectators) and entities.
pub(crate) fn entity_boxes<'p>(players: impl Iterator<Item = &'p Player>, entities: &entities::Entities) -> Vec<EntityBox> {
    let mut out: Vec<EntityBox> = players
        .filter(|p| p.game_mode != 3 && !p.dead)
        .map(|p| {
            let h = p.dimensions().1 as f64;
            EntityBox {
                min: [p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3],
                max: [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3],
                living: true,
                blocks_building: true,
                conn: Some(p.conn),
                prevents_rest: false,
                player_source: Some(player_source(p)),
                hanging: None,
            }
        })
        .collect();
    out.extend(entities.list.iter().filter(|e| !e.removed && e.phys.is_some()).map(|e| {
        let (min, max, blocks_building) = e.body();
        // Mobs are living entities (pressure plates, lightning targets).
        let living = e.phys.as_deref().and_then(kiln_entity::mob::data).is_some_and(|m| m.health > 0.0);
        EntityBox { min, max, living, blocks_building, conn: None, prevents_rest: e.prevents_rest(), player_source: None, hanging: e.phys.as_deref().and_then(crate::frames::hanging_of) }
    }));
    out
}

/// Player `p` as the source of a game event.
pub(crate) fn player_source(p: &Player) -> kiln_entity::vibration::EventSource {
    let pos = kiln_entity::math::Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
    kiln_entity::vibration::EventSource::player(p.entity_id, p.uuid.as_u128(), pos, p.sneaking, p.game_mode == 3, p.game_mode == 1)
}

/// What block work leaves behind: positions to send to clients and effects to carry out.
#[derive(Default)]
pub(crate) struct BlockOut {
    /// Positions set with `UPDATE_CLIENTS`, in order (may repeat).
    pub changed: Vec<[i32; 3]>,
    /// Effects with the player whose action caused them.
    pub effects: Vec<(Option<ConnId>, Effect)>,
    /// Crack stages to show others: (breaker entity id, position, stage; outside 0..=9
    /// removes the cracks).
    pub destruction: Vec<(i32, [i32; 3], i32)>,
    /// Entities block entities spawned (dropped contents, dispensed items, experience).
    pub spawns: Vec<Spawn>,
    /// Chests, barrels and ender chests whose openers to recount (their scheduled tick), for
    /// the region, which knows the players.
    pub rechecks: Vec<BlockPos>,
    /// The components of container block entities removed this phase, for the loot of their
    /// block (`copy_components` from the block entity: names, shulker box contents).
    pub removed_components: Vec<(BlockPos, Vec<kiln_item::component::Component>)>,
    /// What block entities do to the players in a box (beacons).
    pub player_fx: Vec<PlayerFx>,
    /// Packets for the players within a distance of a point (particles block entities send).
    pub packets: Vec<([f64; 3], f64, Bytes)>,
    /// Criteria triggers for players (by entity id) with only the player condition
    /// (`avoid_vibration`).
    pub triggers: Vec<(i32, &'static str)>,
    /// Sculk shriekers a player (by entity id) set off (`SculkShriekerBlockEntity.tryShriek`),
    /// for the region, which knows the players.
    pub shrieks: Vec<(BlockPos, i32)>,
    /// Sculk shriekers whose shriek ended (`tryRespond`) and their warning level: the region
    /// answers with darkness and maybe a warden.
    pub responds: Vec<(BlockPos, i32)>,
    /// Bells that rang this phase (their block events ran): the region lists what hears them.
    pub bell_events: Vec<BlockPos>,
    /// Block states changed so far (whatever the flags).
    pub edits: u64,
}

/// A block entity's effect on the players whose box meets `min..max`.
pub(crate) enum PlayerFx {
    /// A mob effect (a beacon's power).
    Effect { min: [f64; 3], max: [f64; 3], effect: crate::effects::Effect },
    /// A beacon lit: `construct_beacon` with its levels.
    BeaconActivated { min: [f64; 3], max: [f64; 3], levels: i32 },
}

/// A region's cells and block machinery as kiln-blocks' [`Level`].
pub(crate) struct RegionLevel<'a> {
    pub cells: &'a mut CellSet<Cell>,
    pub blocks: &'a mut RegionBlocks,
    pub env: &'a BlockEnv,
    pub out: &'a mut BlockOut,
    pub bodies: &'a [EntityBox],
    /// The player acting (effects the client shows itself skip it).
    pub actor: Option<ConnId>,
}

impl RegionLevel<'_> {
    fn block_entity_int(&self, pos: BlockPos, key: &str) -> Option<i32> {
        let chunk = self.cells.chunk(chunk_of(pos))?;
        let be = chunk.block_entity((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize)?;
        be.nbt.get(key).and_then(Tag::as_i64).map(|v| v as i32)
    }
}

impl Level for RegionLevel<'_> {
    type Random = LegacyRandom;

    fn block(&self, pos: BlockPos) -> u16 {
        self.cells.get_block(pos.x, pos.y, pos.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR)
    }

    fn set_raw(&mut self, pos: BlockPos, state: u16, flags: u32) -> Option<u16> {
        let old = self.cells.set_block(pos.x, pos.y, pos.z, state)?;
        if old == state {
            return None;
        }
        self.out.edits += 1;
        if flags & flags::CLIENTS != 0 {
            self.out.changed.push([pos.x, pos.y, pos.z]);
        }
        if kiln_data::block_props::has_block_entity(old) || kiln_data::block_props::has_block_entity(state) {
            crate::container::block_set(self, pos, flags, old);
            crate::sculk::block_set(self, pos);
            crate::heart::block_set(self, pos);
            crate::mob_spawner::block_set(self, pos);
        }
        Some(old)
    }

    fn in_bounds(&self, pos: BlockPos) -> bool {
        (self.env.min_y..self.env.min_y + self.env.height).contains(&pos.y)
            && (-MAX_LEVEL_SIZE..MAX_LEVEL_SIZE).contains(&pos.x)
            && (-MAX_LEVEL_SIZE..MAX_LEVEL_SIZE).contains(&pos.z)
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.cells.chunk(chunk_of(pos)).is_some()
    }

    fn min_y(&self) -> i32 {
        self.env.min_y
    }

    fn portals_light(&self) -> bool {
        matches!(self.env.dim, crate::OVERWORLD_ID | crate::NETHER_ID)
    }

    fn game_time(&self) -> i64 {
        self.env.game_time
    }

    fn next_sub_tick(&mut self) -> i64 {
        let s = self.blocks.sub_tick;
        self.blocks.sub_tick += 1;
        s
    }

    fn block_ticks(&mut self) -> &mut LevelTicks<BlockId> {
        &mut self.blocks.block_ticks
    }

    fn fluid_ticks(&mut self) -> &mut LevelTicks<FluidType> {
        &mut self.blocks.fluid_ticks
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.blocks.random
    }

    fn data(&mut self) -> &mut LevelData {
        &mut self.blocks.data
    }

    fn rules(&self) -> &kiln_blocks::Rules {
        &self.env.rules
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        let top = self.env.min_y + self.env.height;
        self.cells.light_at(LightLayer::Sky, pos.x, pos.y, pos.z).map_or(if pos.y >= top { 15 } else { 0 }, i32::from)
    }

    fn sun_angle(&self) -> f32 {
        kiln_blocks::behaviour::daylight::sun_angle(self.env.mobs.day_time)
    }

    fn bell_hit(&mut self, pos: BlockPos, dir: kiln_blocks::Direction) -> bool {
        crate::bell::on_hit(self, pos, dir)
    }

    fn bell_event(&mut self, pos: BlockPos, dir: kiln_blocks::Direction) -> bool {
        crate::bell::trigger_event(self, pos, dir)
    }

    fn beehive_fire(&mut self, pos: BlockPos, state: u16) {
        crate::beehive::neighbour_fire(self, pos, state);
    }

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        let top = self.env.min_y + self.env.height;
        let sky = self.cells.light_at(LightLayer::Sky, pos.x, pos.y, pos.z).map_or(if pos.y >= top { 15 } else { 0 }, i32::from);
        let block = self.cells.light_at(LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from);
        block.max(sky - sky_darken)
    }

    fn effect(&mut self, effect: Effect) {
        let (pos, event, state) = match effect {
            Effect::GameEvent { pos, event } => (pos, event, None),
            Effect::BlockGameEvent { pos, event, state } => (pos, event, Some(state)),
            _ => {
                self.out.effects.push((self.actor, effect));
                return;
            }
        };
        // `level.gameEvent(player, event, pos)`: the acting player is the source.
        if crate::sculk::listening(self) {
            let source = self.actor.and_then(|c| self.bodies.iter().find(|b| b.conn == Some(c))).and_then(|b| b.player_source);
            let at = kiln_entity::math::Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
            crate::sculk::post(self, event, at, kiln_entity::vibration::Context { source, affected_state: state });
        }
    }

    fn comparator_output(&self, pos: BlockPos) -> i32 {
        self.block_entity_int(pos, "OutputSignal").unwrap_or(0)
    }

    fn set_comparator_output(&mut self, pos: BlockPos, value: i32) {
        if self.block_entity_int(pos, "OutputSignal") == Some(value) {
            return;
        }
        let Some(chunk) = self.cells.chunk_mut(chunk_of(pos)) else { return };
        let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
        let Some(mut be) = chunk.block_entity(x, pos.y, z).cloned() else { return };
        if let Tag::Compound(fields) = &mut be.nbt {
            match fields.iter_mut().find(|(k, _)| k == "OutputSignal") {
                Some((_, v)) => *v = Tag::Int(value),
                None => fields.push(("OutputSignal".into(), Tag::Int(value))),
            }
        }
        chunk.set_block_entity(x, pos.y, z, be);
    }

    fn block_entity_analog(&self, pos: BlockPos, state: u16, _dir: Direction) -> i32 {
        if let Some(v) = crate::sculk::analog(self, pos, state) {
            return v;
        }
        if let Some(v) = crate::heart::analog(self, pos, state) {
            return v;
        }
        crate::container::analog(self, pos, state)
    }

    fn jukebox_playing(&self, pos: BlockPos) -> bool {
        crate::jukebox::is_playing(self, pos)
    }

    fn container_openers(&self, pos: BlockPos) -> i32 {
        self.blocks.containers.get(pos).map_or(0, |c| c.openers)
    }

    fn command_block_powered(&mut self, pos: BlockPos, state: u16, powered: bool) {
        crate::command_block::powered_changed(self, pos, state, powered);
    }

    fn crafter_triggered(&mut self, pos: BlockPos, triggered: bool) {
        if let Some(cr) = self.blocks.containers.get_mut(pos).and_then(|c| c.crafter.as_mut()) {
            cr.triggered = triggered;
        }
    }

    fn block_entity_tick(&mut self, pos: BlockPos, state: u16) {
        if kiln_data::block_logic::block_class(state) == kiln_data::block_logic::BlockClass::SculkShriekerBlock {
            crate::sculk::shrieker::try_respond(self, pos);
            return;
        }
        crate::container::scheduled_tick(self, pos, state);
    }

    fn count_entities(&self, min: [f64; 3], max: [f64; 3], kind: EntityKind) -> usize {
        self.bodies
            .iter()
            .filter(|b| match kind {
                EntityKind::Any => true,
                EntityKind::Living => b.living,
                EntityKind::Minecart => false,
            })
            .filter(|b| b.intersects(min, max))
            .count()
    }

    fn trace_update(&mut self, _update: UpdateTrace) {}

    fn height(&self) -> i32 {
        self.env.height
    }

    fn weather(&self) -> kiln_blocks::weather::Weather {
        self.env.weather.weather
    }

    fn motion_blocking_height(&self, x: i32, z: i32) -> i32 {
        crate::weather::motion_blocking_height(self.cells, self.env, x, z)
    }

    fn climate(&self, biome_pos: BlockPos, pos: BlockPos) -> Option<kiln_blocks::weather::Climate> {
        crate::weather::climate(self.cells, self.env, biome_pos, pos)
    }

    fn creaking_active(&self, _pos: BlockPos) -> bool {
        self.env.mobs.creaking_active
    }

    /// `gameplay/turtle_egg_hatch_chance`: certain for 843 ticks around dawn in the overworld,
    /// otherwise the default 1/500.
    fn turtle_egg_hatch_chance(&self, _pos: BlockPos) -> f32 {
        let dawn = self.env.dim == crate::OVERWORLD_ID && (21062..21905).contains(&self.env.mobs.day_time.rem_euclid(24000));
        if dawn { 1.0 } else { 0.002 }
    }

    fn is_raining_at(&self, pos: BlockPos) -> bool {
        crate::weather::is_raining_at(self.cells, self.env, pos)
    }

    fn reseed_random(&mut self, pos: BlockPos) {
        let (random, _) = chunk_random(self.env.seed ^ ((pos.y as i64) << 20), self.env.game_time, ChunkPos::new(pos.x, pos.z));
        self.blocks.random = random;
    }

    fn block_light(&self, pos: BlockPos) -> i32 {
        self.cells.light_at(LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from)
    }

    fn sky_darken(&self) -> i32 {
        self.env.mobs.sky_darken
    }

    fn is_end(&self) -> bool {
        self.env.dim == crate::END_ID
    }

    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        let r = self.env.fire_spread_radius;
        let at = [pos.x as f64, pos.y as f64, pos.z as f64];
        r == -1 || self.env.fire_watchers.iter().any(|p| (0..3).map(|i| (p[i] - at[i]) * (p[i] - at[i])).sum::<f64>().sqrt() < r as f64)
    }

    fn difficulty(&self) -> i32 {
        i32::from(self.env.mobs.difficulty)
    }

    fn increased_fire_burnout(&self, pos: BlockPos) -> bool {
        self.env.weather.climates.as_ref().is_some_and(|c| c.increased_fire_burnout(crate::weather::biome_at(self.cells, self.env, pos)))
    }

    fn feature_host(&self) -> Option<std::sync::Arc<dyn kiln_blocks::feature_host::FeatureHost>> {
        self.env.features.clone()
    }

    fn legacy_random(&mut self) -> Option<&mut LegacyRandom> {
        Some(&mut self.blocks.random)
    }

    fn biome_name(&self, pos: BlockPos) -> Option<String> {
        let id = crate::weather::biome_at(self.cells, self.env, pos);
        let (_, names) = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome")?;
        names.get(id as usize).map(|n| (*n).to_owned())
    }

    fn set_block_entity_data(&mut self, pos: BlockPos, data: &Tag) {
        let Some(chunk) = self.cells.chunk_mut(chunk_of(pos)) else { return };
        let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
        let Some(mut be) = chunk.block_entity(x, pos.y, z).cloned() else { return };
        if let (Tag::Compound(fields), Tag::Compound(extra)) = (&mut be.nbt, data) {
            for (k, v) in extra {
                match fields.iter_mut().find(|(f, _)| f == k) {
                    Some((_, old)) => *old = v.clone(),
                    None => fields.push((k.clone(), v.clone())),
                }
            }
        }
        chunk.set_block_entity(x, pos.y, z, be);
    }
}

/// Chunks within simulation distance of a player: a bit per chunk of each cell.
pub(crate) struct Ticking(crate::FastMap<CellPos, u64>);

impl Ticking {
    pub fn around(centers: impl Iterator<Item = ChunkPos>, r: i32) -> Self {
        let mut centers: Vec<ChunkPos> = centers.collect();
        centers.sort_unstable();
        centers.dedup();
        let mut cells: crate::FastMap<CellPos, u64> = Default::default();
        for c in centers {
            let (x0, x1, z0, z1) = (c.x - r, c.x + r, c.z - r, c.z + r);
            for cx in x0.div_euclid(CELL_CHUNKS)..=x1.div_euclid(CELL_CHUNKS) {
                for cz in z0.div_euclid(CELL_CHUNKS)..=z1.div_euclid(CELL_CHUNKS) {
                    let (bx, bz) = (cx * CELL_CHUNKS, cz * CELL_CHUNKS);
                    let (lx0, lx1) = (x0.max(bx) - bx, x1.min(bx + CELL_CHUNKS - 1) - bx);
                    let (lz0, lz1) = (z0.max(bz) - bz, z1.min(bz + CELL_CHUNKS - 1) - bz);
                    let row = ((1u64 << (lx1 - lx0 + 1)) - 1) << lx0;
                    let mask = (lz0..=lz1).fold(0u64, |m, lz| m | row << (lz * CELL_CHUNKS));
                    *cells.entry(ChunkPos::new(bx, bz).cell()).or_default() |= mask;
                }
            }
        }
        Self(cells)
    }

    /// Adds one chunk (a force-loaded one).
    pub fn add_chunk(&mut self, c: ChunkPos) {
        let bit = c.z.rem_euclid(CELL_CHUNKS) * CELL_CHUNKS + c.x.rem_euclid(CELL_CHUNKS);
        *self.0.entry(c.cell()).or_default() |= 1 << bit;
    }

    /// Chunks within `r` of `center` tick too (the dragon fight's arena).
    pub fn add(&mut self, center: ChunkPos, r: i32) {
        let other = Ticking::around(std::iter::once(center), r);
        for (cell, mask) in other.0 {
            *self.0.entry(cell).or_default() |= mask;
        }
    }

    /// These chunks and those within `r` of `center` (the dragon fight's arena).
    pub fn with_arena(&self, center: ChunkPos, r: i32) -> Ticking {
        let mut t = Ticking(self.0.clone());
        t.add(center, r);
        t
    }

    pub fn contains(&self, c: ChunkPos) -> bool {
        let bit = c.z.rem_euclid(CELL_CHUNKS) * CELL_CHUNKS + c.x.rem_euclid(CELL_CHUNKS);
        self.0.get(&c.cell()).is_some_and(|m| m & (1 << bit) != 0)
    }
}

/// A random for one chunk's random ticks this game tick.
fn chunk_random(seed: i64, game_time: i64, c: ChunkPos) -> (LegacyRandom, i32) {
    let mut h = seed as u64 ^ (game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (c.x as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ ((c.z as u32 as u64) << 32).wrapping_mul(0x1656_67B1_9E37_79F9);
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (LegacyRandom::new(h as i64), (h >> 32) as i32)
}

/// The block phases of `ServerLevel.tick` for a region: scheduled block ticks, scheduled
/// fluid ticks, random ticks in ticking chunks, then block events. Moving pistons tick
/// later, after the entities ([`tick_pistons`]).
pub(crate) fn tick_blocks(level: &mut RegionLevel, ticking: &Ticking) {
    let dt = std::time::Instant::now();
    apply_generated(level);
    let dt = crate::diag::lap("tb.generated", dt);
    let can_tick = |k: ChunkKey| ticking.contains(ChunkPos::new(k.0, k.1));
    kiln_blocks::tick::run_block_ticks(level, can_tick);
    let dt = crate::diag::lap("tb.block_ticks", dt);
    kiln_blocks::tick::run_fluid_ticks(level, can_tick);
    let dt = crate::diag::lap("tb.fluid_ticks", dt);
    let speed = level.env.random_tick_speed;
    // Every ticking chunk rolls for precipitation while it rains (and freezes water in any
    // weather); otherwise only chunks with randomly ticking sections have work.
    let precipitation = level.env.weather.climates.is_some();
    if speed > 0 {
        let mut chunks: Vec<ChunkPos> = Vec::new();
        level.cells.for_each_cell(&mut |pos, cell| {
            chunks.extend(
                cell.chunks(pos)
                    .filter(|(c, chunk)| ticking.contains(*c) && (precipitation || chunk.sections.iter().any(|s| s.is_randomly_ticking())))
                    .map(|(c, _)| c),
            );
        });
        chunks.sort_unstable();
        crate::diag::add("tb.list", dt.elapsed());
        let mut sections = Vec::new();
        for c in chunks {
            let Some(chunk) = level.cells.chunk(c) else { continue };
            let min_section = chunk.min_y() >> 4;
            sections.clear();
            sections.extend(chunk.sections.iter().enumerate().map(|(i, s)| (min_section + i as i32, s.is_randomly_ticking())));
            let (random, rand_value) = chunk_random(level.env.seed, level.env.game_time, c);
            let saved = std::mem::replace(&mut level.blocks.random, random);
            let saved_value = std::mem::replace(&mut level.blocks.data.rand_value, rand_value);
            tick_thunder(level, c);
            tick_chunk_blocks(level, c, &sections, speed);
            level.blocks.random = saved;
            level.blocks.data.rand_value = saved_value;
        }
    }
    let dt = crate::diag::lap("tb.random", dt);
    kiln_blocks::block_events::run_block_events(level, |p| ticking.contains(chunk_of(p)));
    // `SignBlockEntity.tick`: editing locks of players who left.
    crate::signs::tick(level);
    crate::diag::lap("tb.events", dt);
}

/// [`kiln_blocks::tick::tick_chunk_blocks`] for chunk `c` with the picked blocks looked up in
/// the sections' random tick bits: most picks land on blocks that do not tick, and those cost
/// one bit read instead of a level lookup and a block read. The same picks, in the same order, reading the blocks as they are at the
/// moment of each pick.
fn tick_chunk_blocks(level: &mut RegionLevel, c: ChunkPos, sections: &[(i32, bool)], speed: i32) {
    let (x, z) = (c.x * 16, c.z * 16);
    for _ in 0..speed {
        if level.blocks.random.next_int_bounded(48) == 0 {
            let pos = kiln_blocks::tick::block_random_pos(level, x, 0, z, 15);
            kiln_blocks::weather::tick_precipitation(level, pos);
        }
    }
    if speed <= 0 {
        return;
    }
    let mut chunk = level.cells.chunk(c);
    for &(sy, ticking) in sections {
        if !ticking {
            continue;
        }
        for _ in 0..speed {
            // `block_random_pos`.
            let data = &mut level.blocks.data;
            data.rand_value = data.rand_value.wrapping_mul(3).wrapping_add(1013904223);
            let j = data.rand_value >> 2;
            let pos = BlockPos::new(x + (j & 15), sy * 16 + ((j >> 16) & 15), z + (j >> 8 & 15));
            // (Lava, the one fluid that ticks randomly, is a randomly ticking block too.)
            if chunk.is_some_and(|ch| ch.ticks_randomly_at((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize)) {
                kiln_blocks::tick::random_tick_at(level, pos);
                chunk = level.cells.chunk(c);
            }
        }
    }
}

/// `ServerLevel.tickThunder` for chunk `c`, with the chunk's random: during a thunderstorm one
/// chance in 100000 per tick of a bolt at a random column's surface (drawn to a lightning rod
/// within 128 blocks, or to a living entity under open sky near the column). With
/// `spawn_mobs`, a chance of the effective difficulty in 100 makes it a skeleton trap: a trap
/// horse at the block (its goal springs the trap when a player comes near) and a harmless bolt.
fn tick_thunder(level: &mut RegionLevel, c: ChunkPos) {
    let w = level.env.weather.weather;
    if !(w.raining && w.thundering) || level.blocks.random.next_int_bounded(100000) != 0 {
        return;
    }
    let pos = kiln_blocks::tick::block_random_pos(level, c.x * 16, 0, c.z * 16, 15);
    let target = lightning_target(level, pos);
    if !crate::weather::is_raining_at(level.cells, level.env, target) {
        return;
    }
    let env = level.env;
    let ctx = crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, crate::spawner::moon_brightness(env.mobs.day_time));
    let trap = env.mobs.spawn_mobs
        && level.blocks.random.next_double() < ctx.effective_difficulty as f64 * 0.01
        && !kiln_blocks::tags::is(level.block(target.below()), "minecraft:lightning_rods");
    if trap {
        // The horse's own random is not the chunk's.
        let horse = trap_horse(target, effect_hash(level.env, target, 0x7472) as i64);
        let corner = [target.x as f64, target.y as f64, target.z as f64];
        level.out.spawns.push(Spawn { kind: &kiln_data::entities::types::SKELETON_HORSE, pos: corner, vel: [0.0; 3], body: entities::Body::Ready(Box::new(horse)) });
    }
    let at = [target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5];
    level.out.spawns.push(Spawn { kind: &kiln_data::entities::types::LIGHTNING_BOLT, pos: at, vel: [0.0; 3], body: entities::Body::Lightning { visual_only: trap } });
}

/// The trap horse of `ServerLevel.tickThunder`: `SKELETON_HORSE.create(level, EVENT)`,
/// `setTrap(true)`, `setAge(0)`, `setPos` at the corner of block `target`, no `finalizeSpawn`;
/// `seed` seeds its own random.
pub(crate) fn trap_horse(target: BlockPos, seed: i64) -> kiln_entity::Entity {
    let mut horse = kiln_entity::mob::new(kiln_entity::mob::MobKind::SkeletonHorse, 0, 0, seed);
    if let Some(m) = kiln_entity::mob::data_mut(&mut horse) {
        kiln_entity::mob::kinds::skeleton_horse::set_trap(m, true);
    }
    horse.set_pos(kiln_entity::math::Vec3::new(target.x as f64, target.y as f64, target.z as f64));
    horse
}

/// `ServerLevel.findLightningTargetAround`.
fn lightning_target(level: &mut RegionLevel, pos: BlockPos) -> BlockPos {
    let top = BlockPos::new(pos.x, crate::weather::motion_blocking_height(level.cells, level.env, pos.x, pos.z), pos.z);
    if let Some(rod) = find_lightning_rod(level, top) {
        return rod.above();
    }
    let max_y = level.env.min_y + level.env.height - 1;
    let (min, max) = ([top.x as f64 - 3.0, top.y as f64 - 3.0, top.z as f64 - 3.0], [top.x as f64 + 4.0, max_y as f64 + 2.0 + 3.0, top.z as f64 + 4.0]);
    let living: Vec<BlockPos> = level
        .bodies
        .iter()
        .filter(|b| b.living && b.intersects(min, max))
        .map(|b| BlockPos::new(((b.min[0] + b.max[0]) / 2.0).floor() as i32, b.min[1].floor() as i32, ((b.min[2] + b.max[2]) / 2.0).floor() as i32))
        .filter(|&p| crate::weather::can_see_sky(level.cells, level.env, p))
        .collect();
    if !living.is_empty() {
        let i = level.blocks.random.next_int_bounded(living.len() as i32) as usize;
        return living[i];
    }
    if top.y == level.env.min_y - 1 { top.above().above() } else { top }
}

/// `findLightningRod`: the nearest lightning rod within 128 blocks that is the top block of its
/// column (the POI search over loaded chunks).
fn find_lightning_rod(level: &RegionLevel, center: BlockPos) -> Option<BlockPos> {
    const R: i32 = 128;
    let is_rod = |s: u16| kiln_blocks::tags::is(s, "minecraft:lightning_rods");
    let mut best: Option<(i64, BlockPos)> = None;
    for cx in (center.x - R) >> 4..=(center.x + R) >> 4 {
        for cz in (center.z - R) >> 4..=(center.z + R) >> 4 {
            let Some(chunk) = level.cells.chunk(ChunkPos::new(cx, cz)) else { continue };
            for (si, section) in chunk.sections.iter().enumerate() {
                let maybe = match &section.blocks {
                    kiln_world::section::BlockContainer::Single(s) => is_rod(*s),
                    kiln_world::section::BlockContainer::Nibble { palette, .. } | kiln_world::section::BlockContainer::Byte { palette, .. } => {
                        palette.iter().any(|&s| is_rod(s))
                    }
                    kiln_world::section::BlockContainer::Direct(_) => true,
                };
                if !maybe {
                    continue;
                }
                let y0 = chunk.min_y() + si as i32 * 16;
                for i in 0..4096usize {
                    let (x, y, z) = (i & 15, (i >> 8) as i32, (i >> 4) & 15);
                    if !is_rod(section.get(x, y as usize, z)) {
                        continue;
                    }
                    let p = BlockPos::new(cx * 16 + x as i32, y0 + y, cz * 16 + z as i32);
                    let d = [(p.x - center.x) as i64, (p.y - center.y) as i64, (p.z - center.z) as i64];
                    let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                    if d2 > (R as i64) * (R as i64) || best.is_some_and(|(b, _)| b <= d2) {
                        continue;
                    }
                    // `WORLD_SURFACE`: the rod is the column's top block.
                    let surface = chunk.column_height(x, z, |s| !kiln_data::blocks_types::is_air(s));
                    if p.y == surface - 1 {
                        best = Some((d2, p));
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Generation's leftovers for new chunks whose neighbours are all loaded, in chunk order:
/// blocks marked for post-processing take their shape from their neighbours, and the
/// scheduled block and fluid ticks start. Vanilla sets them with flags 20 before any player
/// has the chunk; Kiln may have sent it already, so clients hear about the change.
/// Region tick time spent on generation leftovers per tick at most (the rest wait a tick).
const GENERATED_BUDGET: std::time::Duration = std::time::Duration::from_millis(1);

fn apply_generated(level: &mut RegionLevel) {
    if level.blocks.generated.is_empty() {
        return;
    }
    let mut pending = std::mem::take(&mut level.blocks.generated);
    pending.sort_by_key(|(c, _)| *c);
    let mut later = Vec::new();
    // A burst of chunks becoming full (players arriving somewhere new) spreads over a few ticks:
    // in vanilla too a chunk's leftovers run whenever the chunk pipeline promotes it.
    let started = std::time::Instant::now();
    for (c, updates) in pending {
        let ready = (-1..=1).all(|dx| (-1..=1).all(|dz| level.cells.chunk(ChunkPos::new(c.x + dx, c.z + dz)).is_some()));
        if !ready || started.elapsed() >= GENERATED_BUDGET {
            later.push((c, updates));
            continue;
        }
        for p in updates.post_process {
            let pos = BlockPos::new(p[0], p[1], p[2]);
            let s = level.block(pos);
            let shaped = kiln_blocks::update::update_from_neighbour_shapes(level, s, pos);
            if shaped != s {
                kiln_blocks::set_block(level, pos, shaped, flags::KNOWN_SHAPE | flags::CLIENTS);
            }
        }
        for (p, name, delay) in updates.block_ticks {
            if let Some(block) = BlockId::by_name(name) {
                kiln_blocks::schedule_block_tick(level, BlockPos::new(p[0], p[1], p[2]), block, delay, kiln_blocks::TickPriority::Normal);
            }
        }
        for (p, name, delay) in updates.fluid_ticks {
            if let Some(fluid) = FluidType::from_name(name) {
                kiln_blocks::schedule_fluid_tick(level, BlockPos::new(p[0], p[1], p[2]), fluid, delay);
            }
        }
    }
    level.blocks.generated = later;
}

/// `Level.tickBlockEntities` for moving pistons.
pub(crate) fn tick_pistons(level: &mut RegionLevel, ticking: &Ticking) {
    if level.blocks.data.pistons.is_empty() {
        return;
    }
    kiln_blocks::tick_moving_pistons(level, |p| ticking.contains(chunk_of(p)));
}

/// `Entity.checkInsideBlocks` for pressure plates and tripwires: every body standing in a plate
/// presses it, every body whose box meets a tripwire's shape presses that.
pub(crate) fn press_plates(level: &mut RegionLevel) {
    use kiln_data::block_logic::{BlockClass, is_instance};
    let mut plates = Vec::new();
    let mut wires = Vec::new();
    let is_plate = |s: u16| is_instance(s, BlockClass::BasePressurePlateBlock) || is_instance(s, BlockClass::TripWireBlock);
    // Whether a section's palette has a plate, looked at once per section: most bodies stand
    // in sections without any and skip the block lookups.
    let mut may_have: crate::FastMap<(i32, i32, i32), bool> = Default::default();
    let min_y = level.env.min_y;
    for b in level.bodies {
        let lo = [b.min[0] + 1e-5, b.min[1] + 1e-5, b.min[2] + 1e-5].map(|c| c.floor() as i32);
        let hi = [b.max[0] - 1e-5, b.max[1] - 1e-5, b.max[2] - 1e-5].map(|c| c.floor() as i32);
        let mut any = false;
        for sx in lo[0] >> 4..=hi[0] >> 4 {
            for sy in (lo[1] - min_y) >> 4..=(hi[1] - min_y) >> 4 {
                for sz in lo[2] >> 4..=hi[2] >> 4 {
                    any |= *may_have.entry((sx, sy, sz)).or_insert_with(|| {
                        level.cells.chunk(ChunkPos::new(sx, sz)).and_then(|c| usize::try_from(sy).ok().and_then(|i| c.sections.get(i))).is_some_and(|s| s.blocks.maybe_has(is_plate))
                    });
                }
            }
        }
        if !any {
            continue;
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let pos = BlockPos::new(x, y, z);
                    let s = level.block(pos);
                    if is_instance(s, BlockClass::BasePressurePlateBlock) {
                        plates.push(pos);
                    } else if is_instance(s, BlockClass::TripWireBlock) {
                        // `getEntityInsideCollisionShape` is the string's shape: a flat strip when
                        // attached, a low slab otherwise.
                        let (y0, y1) = if kiln_blocks::state::get_bool(s, "attached") { (1.0 / 16.0, 2.5 / 16.0) } else { (0.0, 0.5) };
                        let (by0, by1) = (y as f64 + y0, y as f64 + y1);
                        if b.min[1] + 1e-5 < by1 && b.max[1] - 1e-5 > by0 {
                            wires.push(pos);
                        }
                    }
                }
            }
        }
    }
    plates.sort_unstable();
    plates.dedup();
    for pos in plates {
        kiln_blocks::redstone::components::plate_entity_inside(level, pos);
    }
    wires.sort_unstable();
    wires.dedup();
    for pos in wires {
        kiln_blocks::behaviour::tripwire::wire_entity_inside(level, pos, false);
    }
}

/// Sends what block work changed and carries out its effects: Block Update / Section Blocks
/// Update (and block entity data) to players with the chunk, particles, sounds and block
/// events to players near them, drops, falling blocks and primed TNT to `spawns`.
pub(crate) fn finish(cells: &CellSet<Cell>, mut out: BlockOut, players: &mut [&mut Player], spawns: &mut Vec<Spawn>, env: &BlockEnv) {
    send_changes(cells, &out.changed, players);
    spawns.append(&mut out.spawns);
    for (at, range, pkt) in std::mem::take(&mut out.packets) {
        for p in players.iter_mut().filter(|p| !p.disconnected && (0..3).map(|i| (p.pos[i] - at[i]).powi(2)).sum::<f64>() < range * range) {
            p.send(pkt.clone());
        }
    }
    for (id, trigger) in std::mem::take(&mut out.triggers) {
        if let Some(p) = players.iter_mut().find(|p| p.entity_id == id) {
            p.fire(trigger, None, |c, _, _| matches!(c.trigger, crate::advancements::criteria::Trigger::Player));
        }
    }
    for fx in std::mem::take(&mut out.player_fx) {
        let (min, max) = match &fx {
            PlayerFx::Effect { min, max, .. } | PlayerFx::BeaconActivated { min, max, .. } => (*min, *max),
        };
        // The player's box (0.6 wide, 1.8 tall).
        let inside = |p: &Player| {
            let (lo, hi) = ([p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3], [p.pos[0] + 0.3, p.pos[1] + 1.8, p.pos[2] + 0.3]);
            (0..3).all(|i| lo[i] < max[i] && hi[i] > min[i])
        };
        for p in players.iter_mut().filter(|p| !p.dead && !p.disconnected && inside(p)) {
            match &fx {
                PlayerFx::Effect { effect, .. } => {
                    p.add_effect(effect.clone());
                }
                PlayerFx::BeaconActivated { levels, .. } => {
                    let levels = *levels;
                    p.fire_conds("minecraft:construct_beacon", None, |c, _, _| kiln_loot::predicate::item::int_bounds(&c.ints("level"), levels));
                }
            }
        }
    }
    for (breaker, pos, stage) in out.destruction {
        // `ServerLevel.destroyBlockProgress`: other players within 32 blocks.
        let pkt = world_fx::block_destruction(breaker, pos, u8::try_from(stage).ok());
        send_near(players, BlockPos::new(pos[0], pos[1], pos[2]), 32.0, &pkt, |p| p.entity_id != breaker);
    }
    // Drops are seeded by their position and how many drops that position had before in this
    // batch, not by the batch index (which depends on how regions split the world).
    let mut drops_at: std::collections::HashMap<BlockPos, usize> = std::collections::HashMap::new();
    for (i, (actor, effect)) in std::mem::take(&mut out.effects).into_iter().enumerate() {
        let others = |p: &&mut Player| Some(p.conn) != actor;
        match effect {
            effect @ (Effect::Drop { .. } | Effect::ExplosionDrop { .. } | Effect::EntityDrop { .. }) => {
                let by_entity = matches!(effect, Effect::EntityDrop { .. });
                let (pos, state, explosion) = match effect {
                    Effect::Drop { pos, state } | Effect::EntityDrop { pos, state } => (pos, state, None),
                    Effect::ExplosionDrop { pos, state, radius } => (pos, state, Some(radius)),
                    _ => unreachable!("matched above"),
                };
                let i = {
                    let n = drops_at.entry(pos).or_insert(0);
                    *n += 1;
                    *n - 1
                };
                if env.drops {
                    // The breaking player's held item is the tool; other breaks use an empty hand.
                    let tool = actor.and_then(|c| players.iter().find(|p| p.conn == c)).map(|p| p.inv.selected_item().clone());
                    let tool = if by_entity { Some(kiln_item::ItemStack::empty()) } else { tool };
                    let components = out.removed_components.iter().rev().find(|(p, _)| *p == pos).map(|(_, c)| c.clone());
                    // `InfestedBlock.spawnAfterBreak`: a silverfish comes out unless the tool has
                    // silk touch (`#prevents_infested_spawns`).
                    let silk = tool.as_ref().and_then(|t| t.get(kiln_item::keys::ENCHANTMENTS)).is_some_and(|e| {
                        kiln_item::registry::ENCHANTMENT.id("minecraft:silk_touch").is_some_and(|id| e.level(id) > 0)
                    });
                    if !silk && kiln_entity::mob::kinds::silverfish::is_infested(state) {
                        let at = [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5];
                        spawns.push(crate::mobs::spawn(kiln_entity::mob::MobKind::Silverfish, at, Some(0.0), None));
                    }
                    // `Block.dropResources` then `spawnAfterBreak(.., dropExperience = true)` for a
                    // player's break: ores, sculk and spawners pop experience (explosions and
                    // other breaks pass false, or have no tool).
                    if let (Some(loot), Some(tool)) = (&env.loot, tool.as_ref()) {
                        spawns.extend(block_experience_orbs(loot, pos, state, tool, env, i));
                    }
                    match &env.loot {
                        Some(loot) => spawns.extend(block_drops(loot, pos, state, tool, components, env, i, explosion)),
                        None => spawns.extend(drop_stand_in(pos, state, env, i)),
                    }
                }
            }
            Effect::LevelEvent { id, pos, data } => {
                let pkt = world_fx::level_event(id, [pos.x, pos.y, pos.z], data, false);
                send_near(players, pos, 64.0, &pkt, |_| true);
            }
            // Approximation as for the entities' global events: every player of the region.
            Effect::GlobalLevelEvent { id, pos, data } => {
                let pkt = world_fx::level_event(id, [pos.x, pos.y, pos.z], data, true);
                for p in players.iter_mut() {
                    p.send(pkt.clone());
                }
            }
            Effect::ActorLevelEvent { id, pos, data } => {
                let pkt = world_fx::level_event(id, [pos.x, pos.y, pos.z], data, false);
                send_near(players, pos, 64.0, &pkt, others);
            }
            Effect::Sound { pos, sound, volume, pitch } => {
                if let Some(pkt) = sound_packet(sound, world_fx::SoundSource::Blocks, pos, volume, pitch, env, i) {
                    send_near(players, pos, 16.0 * volume.max(1.0) as f64, &pkt, |_| true);
                }
            }
            Effect::ActorSound { pos, sound, volume, pitch } => {
                if let Some(pkt) = sound_packet(sound, world_fx::SoundSource::Blocks, pos, volume, pitch, env, i) {
                    send_near(players, pos, 16.0 * volume.max(1.0) as f64, &pkt, others);
                }
            }
            // `JukeboxSongPlayer.spawnMusicParticles`: a note 1.2 above the block's bottom center.
            Effect::MusicNote { pos, color } => {
                if let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:note") {
                    let pkt = world_fx::level_particles(&world_fx::LevelParticles {
                        particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::None },
                        override_limiter: false,
                        always_show: false,
                        pos: [pos.x as f64 + 0.5, pos.y as f64 + 1.2000000476837158, pos.z as f64 + 0.5],
                        offset: [color, 0.0, 0.0],
                        max_speed: [1.0, 0.0, 0.0],
                        count: 0,
                        randomization: world_fx::ParticleRandomization::Default,
                    });
                    send_near(players, pos, 32.0, &pkt, |_| true);
                }
            }
            Effect::NoteBlock { pos, instrument, note } => {
                let (sound, pitch) = note_sound(instrument, note);
                if let Some(pkt) = sound_packet(sound, world_fx::SoundSource::Records, pos, 3.0, pitch, env, i) {
                    send_near(players, pos, 48.0, &pkt, |_| true);
                }
            }
            Effect::BlockEvent { pos, block, a, b } => {
                let pkt = world_fx::block_event([pos.x, pos.y, pos.z], a as u8, b as u8, block.0 as i32);
                send_near(players, pos, 64.0, &pkt, |_| true);
            }
            // `FallingBlockEntity.fall` (the block is already gone) and `TntBlock.prime`.
            Effect::FallingBlock { pos, state } => spawns.push(Spawn {
                kind: &kiln_data::entities::types::FALLING_BLOCK,
                pos: [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5],
                vel: [0.0; 3],
                body: entities::Body::FallingBlock { state },
            }),
            Effect::FallingStalactite { pos, state, per_distance } => spawns.push(Spawn {
                kind: &kiln_data::entities::types::FALLING_BLOCK,
                pos: [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5],
                vel: [0.0; 3],
                body: entities::Body::FallingStalactite { state, per_distance },
            }),
            // `TurtleEggBlock.randomTick`: baby turtles whose home is the egg, side by side.
            Effect::HatchTurtles { pos, eggs } => {
                for k in 0..eggs {
                    let h = effect_hash(env, pos, 0x7475 + k as usize);
                    let mut e = kiln_entity::mob::new(kiln_entity::mob::MobKind::Turtle, 0, 0, h as i64);
                    if let Some(mut md) = kiln_entity::mob::data(&e).cloned() {
                        kiln_entity::mob::set_age(&mut e, &mut md, -24000);
                        kiln_entity::mob::kinds::turtle::set_home(&mut md, kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z));
                        if let Some(slot) = kiln_entity::mob::data_mut(&mut e) {
                            *slot = md;
                        }
                    }
                    let at = [pos.x as f64 + 0.3 + k as f64 * 0.2, pos.y as f64, pos.z as f64 + 0.3];
                    e.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
                    e.y_rot = 0.0;
                    e.x_rot = 0.0;
                    e.set_old_pos_and_rot();
                    spawns.push(Spawn { kind: &kiln_data::entities::types::TURTLE, pos: at, vel: [0.0; 3], body: entities::Body::Ready(Box::new(e)) });
                }
            }
            // `DriedGhastBlock.spawnGhastling`: the happy ghast is not a mob Kiln has yet.
            Effect::HatchGhastling { .. } => {}
            Effect::PrimedTnt { pos } => spawns.push(Spawn {
                kind: &kiln_data::entities::types::TNT,
                pos: [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5],
                vel: [0.0; 3],
                body: entities::Body::Tnt,
            }),
            // `SnifferEggBlock.tick`: a baby sniffer at the egg's center, facing a random way.
            Effect::HatchSniffer { pos } => {
                let h = effect_hash(env, pos, 0x736e);
                let yaw = kiln_entity::mob::mth::wrap_degrees((h >> 40) as f32 / (1u64 << 24) as f32 * 360.0);
                let mut e = kiln_entity::mob::new(kiln_entity::mob::MobKind::Sniffer, 0, 0, h as i64);
                if let Some(mut md) = kiln_entity::mob::data(&e).cloned() {
                    kiln_entity::mob::set_age(&mut e, &mut md, -48000);
                    md.y_head_rot = yaw;
                    md.y_body_rot = yaw;
                    if let Some(slot) = kiln_entity::mob::data_mut(&mut e) {
                        *slot = md;
                    }
                }
                let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
                e.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
                e.y_rot = yaw;
                e.set_old_pos_and_rot();
                spawns.push(Spawn { kind: &kiln_data::entities::types::SNIFFER, pos: at, vel: [0.0; 3], body: entities::Body::Ready(Box::new(e)) });
            }
            // `FrogspawnBlock.spawnTadpoles`: tadpoles in the water below the spawn, kept forever.
            Effect::HatchFrogspawn { pos, tadpoles } => {
                for (k, (dx, dz, yaw)) in tadpoles.into_iter().enumerate() {
                    let h = effect_hash(env, pos, 0x7470 + k);
                    let mut e = kiln_entity::mob::new(kiln_entity::mob::MobKind::Tadpole, 0, 0, h as i64);
                    if let Some(md) = kiln_entity::mob::data_mut(&mut e) {
                        md.persistence_required = true;
                    }
                    let at = [pos.x as f64 + dx, pos.y as f64 - 0.5, pos.z as f64 + dz];
                    e.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
                    e.y_rot = yaw as f32;
                    e.x_rot = 0.0;
                    e.set_old_pos_and_rot();
                    spawns.push(Spawn { kind: &kiln_data::entities::types::TADPOLE, pos: at, vel: [0.0; 3], body: entities::Body::Ready(Box::new(e)) });
                }
            }
            // Entities carried by pistons are not simulated yet; game events went to their
            // listeners when they happened.
            Effect::PistonMove { .. } | Effect::GameEvent { .. } | Effect::BlockGameEvent { .. } => {}
        }
    }
}

fn send_near(players: &mut [&mut Player], pos: BlockPos, range: f64, pkt: &Bytes, filter: impl Fn(&&mut Player) -> bool) {
    let c = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    for p in players.iter_mut().filter(|p| filter(p)) {
        let d2: f64 = (0..3).map(|i| (p.pos[i] - c[i]).powi(2)).sum();
        if d2 < range * range {
            p.send(pkt.clone());
        }
    }
}

/// A deterministic value for effect `i` at `pos` this tick (sound seeds, drop jitter).
fn effect_hash(env: &BlockEnv, pos: BlockPos, i: usize) -> u64 {
    let mut h = (env.game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.seed as u64;
    for v in [pos.x as i64, pos.y as i64, pos.z as i64, i as i64] {
        h = (h ^ v as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
    }
    h
}

fn sound_packet(sound: &str, source: world_fx::SoundSource, pos: BlockPos, volume: f32, pitch: f32, env: &BlockEnv, i: usize) -> Option<Bytes> {
    let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    sound_packet_at_pos(sound, source, at, volume, pitch, effect_hash(env, pos, i) as i64)
}

fn sound_packet_at_pos(sound: &str, source: world_fx::SoundSource, at: [f64; 3], volume: f32, pitch: f32, seed: i64) -> Option<Bytes> {
    let id = kiln_data::builtin_id("minecraft:sound_event", sound)?;
    Some(world_fx::sound(&world_fx::Sound::Registered(id), source, at, volume, pitch, seed))
}

/// A block sound at exact coordinates (`Level.playSound(null, x, y, z, ...)`).
pub(crate) fn sound_packet_at(sound: &str, at: [f64; 3], volume: f32, pitch: f32, env: &BlockEnv, salt: usize) -> Option<Bytes> {
    let p = BlockPos::new(at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    sound_packet_at_pos(sound, world_fx::SoundSource::Blocks, at, volume, pitch, effect_hash(env, p, salt) as i64)
}

/// `NoteBlock.triggerEvent`: the instrument's sound, pitched by the note for tunable ones.
fn note_sound(instrument: &str, note: i32) -> (&'static str, f32) {
    const HEADS: [(&str, &str); 6] = [
        ("zombie", "minecraft:block.note_block.imitate.zombie"),
        ("skeleton", "minecraft:block.note_block.imitate.skeleton"),
        ("creeper", "minecraft:block.note_block.imitate.creeper"),
        ("dragon", "minecraft:block.note_block.imitate.ender_dragon"),
        ("wither_skeleton", "minecraft:block.note_block.imitate.wither_skeleton"),
        ("piglin", "minecraft:block.note_block.imitate.piglin"),
    ];
    if let Some((_, s)) = HEADS.iter().find(|(n, _)| *n == instrument) {
        return (s, 1.0);
    }
    let sound = kiln_data::builtin_entries("minecraft:sound_event")
        .and_then(|e| e.iter().find(|s| s.strip_prefix("minecraft:block.note_block.") == Some(instrument)).copied())
        .unwrap_or("minecraft:block.note_block.harp");
    (sound, kiln_javamath::pow::pow(2.0, (note - 12) as f64 / 12.0) as f32)
}

/// `Block.spawnAfterBreak` of a block a player broke with `tool`: the experience orbs
/// (`popExperience` at the block's centre). The level random is stood in for by a random seeded
/// from the position and tick, like the drops.
fn block_experience_orbs(loot: &kiln_loot::LootData, pos: BlockPos, state: u16, tool: &kiln_item::ItemStack, env: &BlockEnv, i: usize) -> Vec<Spawn> {
    use kiln_javamath::random::LegacyRandom;
    if !env.drops || kiln_loot::block_xp::xp_rule(BlockId::of(state).name()).is_none() {
        return Vec::new();
    }
    let mut rng = LegacyRandom::new((effect_hash(env, pos, i.wrapping_add(0x58)) | 1) as i64);
    let amount = loot.block_experience(BlockId::of(state).name(), tool, &mut rng);
    let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    let mut spawns = Vec::new();
    crate::container::furnace::award_experience(at, amount, &mut rng, &mut spawns);
    spawns
}

/// What a broken block drops (`Block.getDrops` with the block loot table), each stack popped
/// like `Block.popResource`.
fn block_drops(
    loot: &kiln_loot::LootData,
    pos: BlockPos,
    state: u16,
    tool: Option<kiln_item::ItemStack>,
    block_entity: Option<Vec<kiln_item::component::Component>>,
    env: &BlockEnv,
    i: usize,
    explosion: Option<f32>,
) -> Vec<Spawn> {
    // Vanilla draws block drops from the server-wide random sequence of the table; parallel
    // regions cannot share one without the order depending on the partition, so each drop gets
    // its own seed from the position and tick (an approximation, I class).
    let origin = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    let seed = (effect_hash(env, pos, i) | 1) as i64;
    let items = block_items_with(loot, origin, state, tool, block_entity, seed, explosion);
    items
        .into_iter()
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(k, stack)| pop_resource(pos, stack, effect_hash(env, pos, i.wrapping_mul(64).wrapping_add(k))))
        .collect()
}

/// The stacks the loot table of `state`'s block rolls for a context at `origin` with `tool`
/// (`Block.getDrops`); a tool means an entity broke it (`this_entity` is set). `seed` stands in
/// for the table's random sequence.
pub(crate) fn block_items(
    loot: &kiln_loot::LootData,
    origin: [f64; 3],
    state: u16,
    tool: Option<kiln_item::ItemStack>,
    block_entity: Option<Vec<kiln_item::component::Component>>,
    seed: i64,
) -> Vec<kiln_item::ItemStack> {
    block_items_with(loot, origin, state, tool, block_entity, seed, None)
}

/// [`block_items`] for a block an explosion destroyed: `explosion` is the `explosion_radius`
/// parameter, set when the drops decay (`survives_explosion`, `explosion_decay`).
pub(crate) fn block_items_with(
    loot: &kiln_loot::LootData,
    origin: [f64; 3],
    state: u16,
    tool: Option<kiln_item::ItemStack>,
    block_entity: Option<Vec<kiln_item::component::Component>>,
    seed: i64,
    explosion: Option<f32>,
) -> Vec<kiln_item::ItemStack> {
    let Some(table_id) = loot.block_table(BlockId::of(state).name()) else { return Vec::new() };
    let Some(table) = loot.table(&table_id) else { return Vec::new() };
    // A player break also sets `this_entity` (the player).
    let player = tool.is_some();
    let ctx = BreakContext { tool: tool.unwrap_or_else(kiln_item::ItemStack::empty), player, state, origin, block_entity, explosion };
    let (mut sequences, mut level) = (kiln_loot::RandomSequences::new(0), kiln_javamath::random::LegacyRandom::new(seed));
    let mut rng = table.random(seed, &mut sequences, &mut level);
    loot.random_items(&table_id, &ctx, rng.source())
}

/// The loot context of a block broken at `origin` (`LootContextParamSets.BLOCK`).
pub(crate) struct BreakContext {
    pub tool: kiln_item::ItemStack,
    pub player: bool,
    pub state: u16,
    pub origin: [f64; 3],
    /// The components of the block's block entity (`collectComponents`), if it had one.
    pub block_entity: Option<Vec<kiln_item::component::Component>>,
    /// `explosion_radius`: set when an explosion with drop decay broke the block.
    pub explosion: Option<f32>,
}

impl kiln_loot::LootContext for BreakContext {
    fn explosion_radius(&self) -> Option<f32> {
        self.explosion
    }
    /// `DecoratedPotBlock`'s `sherds` dynamic drop: the sherds, left, back, front, right.
    fn dynamic_drops(&self, name: &kiln_item::ident::Identifier, sink: &mut dyn FnMut(kiln_item::ItemStack)) {
        if name.as_str() != "minecraft:sherds" {
            return;
        }
        let Some(kiln_item::component::PotDecorations { back, left, right, front }) =
            self.block_entity.as_ref().and_then(|c| c.iter().find_map(|c| if let kiln_item::component::Component::PotDecorations(d) = c { Some(d.clone()) } else { None }))
        else {
            return;
        };
        for sherd in [left, back, front, right].into_iter().flatten() {
            sink(sherd.create());
        }
    }
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        self.player && target == kiln_loot::EntityTarget::This
    }
    /// The empty predicate (snow, chorus flowers: "broken by something") matches any entity.
    fn entity_matches(&self, target: kiln_loot::EntityTarget, predicate: &kiln_loot::predicate::EntityPredicate) -> bool {
        self.player && target == kiln_loot::EntityTarget::This && predicate.parts.is_empty()
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn block_state(&self) -> Option<u16> {
        Some(self.state)
    }
    fn tool(&self) -> Option<&kiln_item::ItemStack> {
        Some(&self.tool)
    }
    fn has_block_entity(&self) -> bool {
        self.block_entity.is_some()
    }
    fn components(&self, source: kiln_loot::Source) -> Option<Vec<kiln_item::component::Component>> {
        match source {
            kiln_loot::Source::BlockEntity => self.block_entity.clone(),
            _ => None,
        }
    }
    fn custom_name(&self, source: kiln_loot::Source) -> Option<Option<kiln_item::Text>> {
        let components = self.block_entity.as_ref().filter(|_| source == kiln_loot::Source::BlockEntity)?;
        Some(components.iter().find_map(|c| match c {
            kiln_item::component::Component::CustomName(t) => Some(t.clone()),
            _ => None,
        }))
    }
}

/// `Block.popResource`: an item entity jittered around the block centre, half an item's
/// height lower, with a pickup delay of 10.
fn pop_resource(pos: BlockPos, stack: kiln_item::ItemStack, h: u64) -> Spawn {
    let unit = |shift: u32| ((h >> shift) & 0xFFFF) as f64 / 65536.0;
    let at = [pos.x as f64 + 0.25 + unit(0) * 0.5, pos.y as f64 + 0.25 + unit(16) * 0.5 - 0.125, pos.z as f64 + 0.25 + unit(32) * 0.5];
    Spawn {
        kind: &kiln_data::entities::types::ITEM,
        pos: at,
        vel: [unit(48) * 0.2 - 0.1, 0.2, unit(8) * 0.2 - 0.1],
        body: entities::Body::Item { stack, pickup_delay: 10, thrower: None },
    }
}

/// Stand-in for loot tables: the block's own item, from a whole block (not the upper half
/// of a door or plant, nor a bed's head).
fn drop_stand_in(pos: BlockPos, state: u16, env: &BlockEnv, i: usize) -> Option<Spawn> {
    use kiln_blocks::state;
    if state::get(state, "half") == Some("upper") || state::get(state, "part") == Some("head") {
        return None;
    }
    let stack = kiln_item::ItemStack::of(BlockId::of(state).name(), 1)?;
    let h = effect_hash(env, pos, i);
    let unit = |shift: u32| ((h >> shift) & 0xFFFF) as f64 / 65536.0;
    // `Block.popResource`: jittered around the centre, half an item's height lower.
    let at = [pos.x as f64 + 0.25 + unit(0) * 0.5, pos.y as f64 + 0.25 + unit(16) * 0.5 - 0.125, pos.z as f64 + 0.25 + unit(32) * 0.5];
    Some(Spawn {
        kind: &kiln_data::entities::types::ITEM,
        pos: at,
        vel: [unit(48) * 0.2 - 0.1, 0.2, unit(8) * 0.2 - 0.1],
        body: entities::Body::Item { stack, pickup_delay: 10, thrower: None },
    })
}

/// Block Update for sections with one change, Section Blocks Update for more, then Block
/// Entity Data for changed blocks with an entity, to players that have the chunk
/// (`ChunkHolder.broadcastChanges`).
pub(crate) fn send_changes(cells: &CellSet<Cell>, changed: &[[i32; 3]], players: &mut [&mut Player]) {
    if changed.is_empty() {
        return;
    }
    let mut sections: BTreeMap<[i32; 3], Vec<[i32; 3]>> = BTreeMap::new();
    for &p in changed {
        let s = sections.entry([p[0] >> 4, p[1] >> 4, p[2] >> 4]).or_default();
        if !s.contains(&p) {
            s.push(p);
        }
    }
    for (section, positions) in sections {
        let chunk = ChunkPos::new(section[0], section[2]);
        if !players.iter().any(|p| p.sent_chunks.contains(&chunk)) {
            continue;
        }
        let state = |p: [i32; 3]| cells.get_block(p[0], p[1], p[2]).unwrap_or(0);
        let mut pkts = vec![if let [p] = positions[..] {
            packets::block_update(p, state(p))
        } else {
            let blocks: Vec<([u8; 3], u32)> = positions.iter().map(|&p| ([(p[0] & 15) as u8, (p[1] & 15) as u8, (p[2] & 15) as u8], state(p) as u32)).collect();
            world_fx::section_blocks_update(section, &blocks)
        }];
        for &p in &positions {
            if let Some((kind, tag)) = cells.block_entity_data(p[0], p[1], p[2]) {
                pkts.push(packets::block_entity_data(p, kind as i32, &tag));
            }
        }
        for pl in players.iter_mut().filter(|pl| pl.sent_chunks.contains(&chunk)) {
            for pkt in &pkts {
                pl.send(pkt.clone());
            }
        }
    }
}

/// Face id (`Direction.get3DDataValue`) to direction.
pub(crate) fn direction(face: i32) -> Option<Direction> {
    (0..6).contains(&face).then(|| Direction::from_index(face as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_blocks::{ScheduledTick, TickPriority};

    /// The random tick fast path picks blocks by their random tick bit alone: every state whose
    /// fluid ticks randomly (lava) must tick randomly as a block too.
    #[test]
    fn lava_states_tick_randomly() {
        for state in 0..kiln_data::blocks::STATE_COUNT as u16 {
            if kiln_data::block_logic::fluid(state).kind == kiln_data::block_logic::FluidKind::Lava {
                assert!(kiln_blocks::tick::randomly_ticks(state), "state {state}");
            }
        }
    }

    fn with_ticks(chunks: &[(i32, i32)]) -> RegionBlocks {
        let mut b = RegionBlocks::default();
        for (i, &(x, z)) in chunks.iter().enumerate() {
            b.block_ticks.add_container((x, z), ChunkTicks::new());
            b.fluid_ticks.add_container((x, z), ChunkTicks::new());
            let pos = BlockPos::new(x * 16 + 1, 0, z * 16 + 2);
            let tick = ScheduledTick { kind: BlockId::of(kiln_data::blocks::default_state::REPEATER), pos, trigger: 10 + i as i64, priority: TickPriority::Normal, sub: i as i64 };
            assert!(b.block_ticks.schedule(tick));
        }
        b
    }

    /// What a thunderstorm's trap spawns: an adult, wild skeleton horse at the block's corner
    /// with the trap goal, not yet sprung.
    #[test]
    fn thunder_trap_horse_is_a_wild_trap() {
        let e = trap_horse(BlockPos::new(-3, 70, 5), 99);
        assert_eq!(e.position(), kiln_entity::math::Vec3::new(-3.0, 70.0, 5.0));
        let m = kiln_entity::mob::data(&e).expect("a mob");
        assert_eq!(m.kind, kiln_entity::mob::MobKind::SkeletonHorse);
        assert!(kiln_entity::mob::kinds::skeleton_horse::is_trap(m));
        assert!(!kiln_entity::mob::kinds::horse::is_tamed(m));
        assert!(!m.baby());
        assert!(m.goals.goals.iter().any(|g| g.priority == 1 && g.goal.name() == "SkeletonTrapGoal"));
        // Saved as a trap (`SkeletonTrap`), and it comes back as one.
        let tag = kiln_entity::persist::save(&e, &|_| None);
        assert_eq!(tag.get("SkeletonTrap").and_then(|t| t.as_i64()), Some(1));
        let back = kiln_entity::persist::load(&tag, 5, 1).expect("loads");
        assert!(kiln_entity::mob::kinds::skeleton_horse::is_trap(kiln_entity::mob::data(&back).unwrap()));
    }

    #[test]
    fn block_drops_come_from_loot_tables() {
        use kiln_data::blocks::default_state as d;
        let work = std::env::var_os("KILN_WORK")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        if !dir.join("data").is_dir() {
            return;
        }
        let loot = kiln_loot::LootData::load(&dir).unwrap();
        let env = BlockEnv {
            game_time: 0,
            rules: kiln_blocks::Rules {
                water_source_conversion: true,
                lava_source_conversion: false,
                fast_lava: false,
                water_evaporates: false,
                tnt_explodes: true,
                spread_vines: true,
                infiniburn: "minecraft:infiniburn_overworld",
            },
            dim: crate::OVERWORLD_ID,
            min_y: -64,
            height: 384,
            random_tick_speed: 3,
            drops: true,
            simulation_distance: 10,
            seed: 0,
            loot: None,
            damage: crate::health::DamageRules::default(),
            mobs: Default::default(),
            spawn_table: None,
            menus: Default::default(),
            trial_configs: Default::default(),
            weather: Default::default(),
            fire_spread_radius: 128,
            send_command_feedback: true,
            fire_watchers: Default::default(),
            players: Default::default(),
            raids: Default::default(),
            dragon_fight: None,
            pipeline: None,
            entity_ticking: crate::EntityTicking::Serial,
            speculate: true,
            features: None,
        };
        let pick = kiln_item::ItemStack::of("minecraft:diamond_pickaxe", 1);
        let drops = |state: u16, tool: Option<kiln_item::ItemStack>| -> Vec<&'static str> {
            block_drops(&loot, BlockPos::new(0, 64, 0), state, tool, None, &env, 0, None)
                .into_iter()
                .map(|s| {
                    let entities::Body::Item { stack, .. } = s.body else { panic!("not an item") };
                    stack.item_name()
                })
                .collect()
        };
        assert_eq!(drops(d::STONE, pick.clone()), ["minecraft:cobblestone"]);
        assert_eq!(drops(d::WALL_TORCH, None), ["minecraft:torch"]);
        assert!(drops(d::GLASS, pick).is_empty());
        assert!(drops(d::WATER, None).is_empty());
        // Ores pop experience orbs for a player's break, silk touch takes it away, and plain
        // blocks have none (`spawnAfterBreak`).
        let orbs = |state: u16, tool: &kiln_item::ItemStack, i: usize| -> i32 {
            block_experience_orbs(&loot, BlockPos::new(0, 64, 0), state, tool, &env, i)
                .into_iter()
                .map(|s| match s.body {
                    entities::Body::Ready(e) => match e.kind {
                        kiln_entity::EntityKind::ExperienceOrb(o) => o.value,
                        _ => panic!("not an orb"),
                    },
                    _ => panic!("not a ready entity"),
                })
                .sum()
        };
        let pick = kiln_item::ItemStack::of("minecraft:diamond_pickaxe", 1).unwrap();
        let mut silk = pick.clone();
        silk.insert(kiln_item::keys::ENCHANTMENTS, {
            let mut e = kiln_item::component::Enchantments::default();
            e.0.push((kiln_item::registry::ENCHANTMENT.id("minecraft:silk_touch").unwrap(), 1));
            e
        });
        let diamond: Vec<i32> = (0..40).map(|i| orbs(d::DIAMOND_ORE, &pick, i)).collect();
        assert!(diamond.iter().all(|&v| (3..=7).contains(&v)), "{diamond:?}");
        assert!(diamond.iter().collect::<std::collections::HashSet<_>>().len() > 2);
        assert!((0..40).all(|i| orbs(d::DIAMOND_ORE, &silk, i) == 0));
        assert!((0..40).all(|i| orbs(d::STONE, &pick, i) == 0));
        assert!((0..40).all(|i| orbs(d::SPAWNER, &silk, i) >= 15));
    }

    #[test]
    fn ticks_follow_their_chunks_through_splits_and_merges() {
        // Chunks in cells (0,0) and (4,0): 32 chunks apart.
        let chunks = [(0, 0), (1, 1), (32, 0), (33, 2)];
        let blocks = with_ticks(&chunks);
        assert_eq!(blocks.block_ticks.count(), 4);
        let parts = blocks.split(&|c: CellPos| usize::from(c.x != 0), 2);
        assert_eq!(parts.len(), 2);
        let owned = |b: &RegionBlocks| {
            let mut v: Vec<ChunkKey> = b.block_ticks.chunks().collect();
            v.sort_unstable();
            v
        };
        assert_eq!(owned(&parts[0]), vec![(0, 0), (1, 1)]);
        assert_eq!(owned(&parts[1]), vec![(32, 0), (33, 2)]);
        assert_eq!(parts[0].block_ticks.count() + parts[1].block_ticks.count(), 4);
        let mut parts = parts.into_iter();
        let mut merged = parts.next().unwrap();
        RegionBlocks::merge(&mut merged, parts.next().unwrap());
        assert_eq!(owned(&merged), vec![(0, 0), (1, 1), (32, 0), (33, 2)]);
        assert_eq!(merged.block_ticks.count(), 4);
        assert_eq!(merged.fluid_ticks.chunks().count(), 4);
        // Merged ticks still run when due.
        merged.block_ticks.collect(12, 100, |_| true);
        let mut ran = 0;
        while merged.block_ticks.next_to_run().is_some() {
            ran += 1;
        }
        assert_eq!(ran, 3);
    }

    #[test]
    fn ticking_chunks_are_within_the_distance_of_a_player() {
        let t = Ticking::around([ChunkPos::new(0, 0), ChunkPos::new(0, 0), ChunkPos::new(-20, 5)].into_iter(), 2);
        for (x, z, want) in [(0, 0, true), (2, -2, true), (3, 0, false), (-2, 2, true), (-22, 7, true), (-23, 5, false), (-18, 3, true), (-18, 2, false)] {
            assert_eq!(t.contains(ChunkPos::new(x, z)), want, "{x},{z}");
        }
    }
}
