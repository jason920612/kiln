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
use std::collections::{BTreeMap, HashMap};

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
        let moving = kiln_data::blocks::default_state::MOVING_PISTON;
        for ((x, y, z), be) in chunk.block_entities() {
            if chunk.get(x, y, z) == moving {
                let at = BlockPos::new(pos.x * 16 + x as i32, y, pos.z * 16 + z as i32);
                self.data.pistons.insert(at, kiln_blocks::MovingPiston::from_nbt(&be.nbt));
            }
        }
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
    }

    /// Puts the chunk's scheduled ticks and moving pistons on it in their saved form.
    pub fn store(&self, pos: ChunkPos, chunk: &mut Chunk, game_time: i64) {
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
        parts[0].random = self.random;
        parts[0].data.rand_value = self.data.rand_value;
        parts
    }

    fn count(&self) -> usize {
        self.block_ticks.chunks().count() + self.fluid_ticks.chunks().count() + self.data.pistons.len() + self.data.block_events.len()
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
}

impl EntityBox {
    fn intersects(&self, min: [f64; 3], max: [f64; 3]) -> bool {
        (0..3).all(|i| self.min[i] < max[i] && self.max[i] > min[i])
    }
}

/// The boxes of a region's players (not spectators) and entities.
pub(crate) fn entity_boxes<'p>(players: impl Iterator<Item = &'p Player>, entities: &entities::Entities) -> Vec<EntityBox> {
    let mut out: Vec<EntityBox> = players
        .filter(|p| p.game_mode != 3 && !p.dead)
        .map(|p| {
            let h = if p.sneaking { 1.5 } else { 1.8 };
            EntityBox {
                min: [p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3],
                max: [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3],
                living: true,
                blocks_building: true,
                conn: Some(p.conn),
            }
        })
        .collect();
    out.extend(entities.list.iter().filter(|e| !e.removed && e.phys.is_some()).map(|e| {
        let (min, max, blocks_building) = e.body();
        EntityBox { min, max, living: false, blocks_building, conn: None }
    }));
    out
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
        if flags & flags::CLIENTS != 0 {
            self.out.changed.push([pos.x, pos.y, pos.z]);
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

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        let top = self.env.min_y + self.env.height;
        let sky = self.cells.light_at(LightLayer::Sky, pos.x, pos.y, pos.z).map_or(if pos.y >= top { 15 } else { 0 }, i32::from);
        let block = self.cells.light_at(LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from);
        block.max(sky - sky_darken)
    }

    fn effect(&mut self, effect: Effect) {
        self.out.effects.push((self.actor, effect));
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
}

/// Chunks within simulation distance of a player: a bit per chunk of each cell.
pub(crate) struct Ticking(HashMap<CellPos, u64>);

impl Ticking {
    pub fn around(centers: impl Iterator<Item = ChunkPos>, r: i32) -> Self {
        let mut centers: Vec<ChunkPos> = centers.collect();
        centers.sort_unstable();
        centers.dedup();
        let mut cells: HashMap<CellPos, u64> = HashMap::new();
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
    apply_generated(level);
    let can_tick = |k: ChunkKey| ticking.contains(ChunkPos::new(k.0, k.1));
    kiln_blocks::tick::run_block_ticks(level, can_tick);
    kiln_blocks::tick::run_fluid_ticks(level, can_tick);
    let speed = level.env.random_tick_speed;
    if speed > 0 {
        let mut chunks: Vec<ChunkPos> = Vec::new();
        level.cells.for_each_cell(&mut |pos, cell| {
            chunks.extend(cell.chunks(pos).filter(|(c, chunk)| ticking.contains(*c) && chunk.sections.iter().any(|s| s.is_randomly_ticking())).map(|(c, _)| c));
        });
        chunks.sort_unstable();
        let mut sections = Vec::new();
        for c in chunks {
            let Some(chunk) = level.cells.chunk(c) else { continue };
            let min_section = chunk.min_y() >> 4;
            sections.clear();
            sections.extend(chunk.sections.iter().enumerate().map(|(i, s)| (min_section + i as i32, s.is_randomly_ticking())));
            let (random, rand_value) = chunk_random(level.env.seed, level.env.game_time, c);
            let saved = std::mem::replace(&mut level.blocks.random, random);
            let saved_value = std::mem::replace(&mut level.blocks.data.rand_value, rand_value);
            kiln_blocks::tick::tick_chunk_blocks(level, key(c), &sections, speed);
            level.blocks.random = saved;
            level.blocks.data.rand_value = saved_value;
        }
    }
    kiln_blocks::block_events::run_block_events(level, |p| ticking.contains(chunk_of(p)));
}

/// Generation's leftovers for new chunks whose neighbours are all loaded, in chunk order:
/// blocks marked for post-processing take their shape from their neighbours, and the
/// scheduled block and fluid ticks start. Vanilla sets them with flags 20 before any player
/// has the chunk; Kiln may have sent it already, so clients hear about the change.
fn apply_generated(level: &mut RegionLevel) {
    if level.blocks.generated.is_empty() {
        return;
    }
    let mut pending = std::mem::take(&mut level.blocks.generated);
    pending.sort_by_key(|(c, _)| *c);
    let mut later = Vec::new();
    for (c, updates) in pending {
        let ready = (-1..=1).all(|dx| (-1..=1).all(|dz| level.cells.chunk(ChunkPos::new(c.x + dx, c.z + dz)).is_some()));
        if !ready {
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

/// `Entity.checkInsideBlocks` for pressure plates: every body standing in a plate presses it.
pub(crate) fn press_plates(level: &mut RegionLevel) {
    let mut plates = Vec::new();
    for b in level.bodies {
        let lo = [b.min[0] + 1e-5, b.min[1] + 1e-5, b.min[2] + 1e-5].map(|c| c.floor() as i32);
        let hi = [b.max[0] - 1e-5, b.max[1] - 1e-5, b.max[2] - 1e-5].map(|c| c.floor() as i32);
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let pos = BlockPos::new(x, y, z);
                    if kiln_data::block_logic::is_instance(level.block(pos), kiln_data::block_logic::BlockClass::BasePressurePlateBlock) {
                        plates.push(pos);
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
}

/// Sends what block work changed and carries out its effects: Block Update / Section Blocks
/// Update (and block entity data) to players with the chunk, particles, sounds and block
/// events to players near them, drops, falling blocks and primed TNT to `spawns`.
pub(crate) fn finish(cells: &CellSet<Cell>, out: BlockOut, players: &mut [&mut Player], spawns: &mut Vec<Spawn>, env: &BlockEnv) {
    send_changes(cells, &out.changed, players);
    for (breaker, pos, stage) in out.destruction {
        // `ServerLevel.destroyBlockProgress`: other players within 32 blocks.
        let pkt = world_fx::block_destruction(breaker, pos, u8::try_from(stage).ok());
        send_near(players, BlockPos::new(pos[0], pos[1], pos[2]), 32.0, &pkt, |p| p.entity_id != breaker);
    }
    for (i, (actor, effect)) in out.effects.into_iter().enumerate() {
        let others = |p: &&mut Player| Some(p.conn) != actor;
        match effect {
            Effect::Drop { pos, state } => {
                if env.drops {
                    // The breaking player's held item is the tool; other breaks use an empty hand.
                    let tool = actor.and_then(|c| players.iter().find(|p| p.conn == c)).map(|p| p.inv.selected_item().clone());
                    match &env.loot {
                        Some(loot) => spawns.extend(block_drops(loot, pos, state, tool, env, i)),
                        None => spawns.extend(drop_stand_in(pos, state, env, i)),
                    }
                }
            }
            Effect::LevelEvent { id, pos, data } => {
                let pkt = world_fx::level_event(id, [pos.x, pos.y, pos.z], data, false);
                send_near(players, pos, 64.0, &pkt, |_| true);
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
            Effect::PrimedTnt { pos } => spawns.push(Spawn {
                kind: &kiln_data::entities::types::TNT,
                pos: [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5],
                vel: [0.0; 3],
                body: entities::Body::Tnt,
            }),
            // Entities carried by pistons and vibrations are not simulated yet.
            Effect::PistonMove { .. } | Effect::GameEvent { .. } => {}
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
    let id = kiln_data::builtin_id("minecraft:sound_event", sound)?;
    let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    Some(world_fx::sound(&world_fx::Sound::Registered(id), source, at, volume, pitch, effect_hash(env, pos, i) as i64))
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
    (sound, 2f32.powf((note - 12) as f32 / 12.0))
}

/// What a broken block drops (`Block.getDrops` with the block loot table), each stack popped
/// like `Block.popResource`.
fn block_drops(
    loot: &kiln_loot::LootData,
    pos: BlockPos,
    state: u16,
    tool: Option<kiln_item::ItemStack>,
    env: &BlockEnv,
    i: usize,
) -> Vec<Spawn> {
    let Some(table_id) = loot.block_table(BlockId::of(state).name()) else { return Vec::new() };
    let Some(table) = loot.table(&table_id) else { return Vec::new() };
    // A player break also sets `this_entity` (the player).
    let player = tool.is_some();
    let ctx = BreakContext {
        tool: tool.unwrap_or_else(kiln_item::ItemStack::empty),
        player,
        state,
        origin: [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5],
    };
    // Vanilla draws block drops from the server-wide random sequence of the table; parallel
    // regions cannot share one without the order depending on the partition, so each drop gets
    // its own seed from the position and tick (an approximation, I class).
    let items = {
        let seed = (effect_hash(env, pos, i) | 1) as i64;
        let (mut sequences, mut level) = (kiln_loot::RandomSequences::new(0), kiln_javamath::random::LegacyRandom::new(seed));
        let mut rng = table.random(seed, &mut sequences, &mut level);
        loot.random_items(&table_id, &ctx, rng.source())
    };
    items
        .into_iter()
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(k, stack)| pop_resource(pos, stack, effect_hash(env, pos, i.wrapping_mul(64).wrapping_add(k))))
        .collect()
}

/// The loot context of a block broken at `origin` (`LootContextParamSets.BLOCK`).
struct BreakContext {
    tool: kiln_item::ItemStack,
    player: bool,
    state: u16,
    origin: [f64; 3],
}

impl kiln_loot::LootContext for BreakContext {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        self.player && target == kiln_loot::EntityTarget::This
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
        };
        let pick = kiln_item::ItemStack::of("minecraft:diamond_pickaxe", 1);
        let drops = |state: u16, tool: Option<kiln_item::ItemStack>| -> Vec<&'static str> {
            block_drops(&loot, BlockPos::new(0, 64, 0), state, tool, &env, 0)
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
