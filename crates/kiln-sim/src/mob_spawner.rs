//! Mob spawner blocks (`SpawnerBlockEntity`): the block entity's state lives next to the
//! region's other block entities (decoded when its chunk enters the region, following chunks
//! through merges and splits, written back when the chunk is stored), and ticks after the
//! entities of the tick with the region's entities in reach; its behaviour is kiln-entity's
//! ([`kiln_entity::spawner`], compared bit for bit with vanilla by `mob_parity`).

use crate::blocks::{RegionLevel, Ticking};
use crate::entities::SimLevel;
use kiln_blocks::BlockPos;
use kiln_entity::spawner::SpawnerBe;
use kiln_proto::nbt::Tag;
use kiln_world::block_entity::{BlockEntity, type_name};
use kiln_world::chunk::Chunk;
use kiln_world::{Blocks, ChunkPos};
use std::collections::BTreeMap;

const TYPE: &str = "minecraft:mob_spawner";

/// A spawner block entity's live state.
#[derive(Clone, Debug)]
pub(crate) struct SpawnerEntry {
    pub be: SpawnerBe,
    pub type_id: u16,
    /// Changed since its NBT was last written into the chunk.
    pub dirty: bool,
}

impl SpawnerEntry {
    fn load(type_id: u16, nbt: &Tag) -> SpawnerEntry {
        SpawnerEntry { be: SpawnerBe::load(nbt), type_id, dirty: false }
    }

    /// The saved compound (`saveCustomOnly`, with the block entity's `id`).
    pub fn save(&self) -> Tag {
        let mut f = vec![("id".to_owned(), Tag::String(TYPE.to_owned()))];
        f.extend(self.be.save());
        Tag::Compound(f)
    }
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

fn to_entity(p: BlockPos) -> kiln_entity::math::BlockPos {
    kiln_entity::math::BlockPos::new(p.x, p.y, p.z)
}

/// A region's mob spawners.
#[derive(Default)]
pub(crate) struct Spawners {
    pub map: BTreeMap<BlockPos, SpawnerEntry>,
}

impl Spawners {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// A chunk entered the region: its spawners are decoded.
    pub fn chunk_loaded(&mut self, pos: ChunkPos, chunk: &Chunk) {
        for ((x, y, z), be) in chunk.block_entities() {
            if type_name(be.kind) == TYPE {
                let at = BlockPos::new(pos.x * 16 + x as i32, y, pos.z * 16 + z as i32);
                self.map.insert(at, SpawnerEntry::load(be.kind, &be.nbt));
            }
        }
    }

    fn chunk_positions(&self, pos: ChunkPos) -> Vec<BlockPos> {
        let lo = BlockPos::new(pos.x * 16, i32::MIN, pos.z * 16);
        let hi = BlockPos::new(pos.x * 16 + 15, i32::MAX, pos.z * 16 + 15);
        self.map.range(lo..=hi).map(|(p, _)| *p).filter(|p| chunk_of(*p) == pos).collect()
    }

    /// Writes the chunk's changed spawners into its NBT.
    pub fn store(&mut self, pos: ChunkPos, chunk: &mut Chunk) {
        for p in self.chunk_positions(pos) {
            let Some(e) = self.map.get_mut(&p).filter(|e| e.dirty) else { continue };
            e.dirty = false;
            let (x, z) = ((p.x & 15) as usize, (p.z & 15) as usize);
            if chunk.block_entity(x, p.y, z).is_none_or(|old| old.kind != e.type_id) {
                continue;
            }
            let mut out = BlockEntity::new(e.type_id);
            if let (Tag::Compound(o), Tag::Compound(fields)) = (&mut out.nbt, e.save()) {
                o.extend(fields.into_iter().filter(|(k, _)| k != "id"));
            }
            chunk.set_block_entity(x, p.y, z, out);
        }
    }

    /// A chunk left the region (after [`Spawners::store`]).
    pub fn chunk_unloaded(&mut self, pos: ChunkPos) {
        for p in self.chunk_positions(pos) {
            self.map.remove(&p);
        }
    }

    /// After the chunk set a block at `pos`: a spawner that went away is dropped; a new one is
    /// decoded.
    pub fn block_changed(&mut self, pos: BlockPos, now: Option<&BlockEntity>) {
        let now = now.filter(|be| type_name(be.kind) == TYPE);
        let kept = match (self.map.get(&pos), now) {
            (Some(s), Some(be)) => s.type_id == be.kind,
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !kept {
            self.map.remove(&pos);
        }
        if let Some(be) = now
            && !self.map.contains_key(&pos)
        {
            self.map.insert(pos, SpawnerEntry::load(be.kind, &be.nbt));
        }
    }

    /// The chunk's block entity at `pos` was replaced from outside (commands): reload it.
    pub fn reload(&mut self, pos: BlockPos, be: Option<&BlockEntity>) {
        self.map.remove(&pos);
        self.block_changed(pos, be);
    }

    /// Moves the spawners of chunks `owner` assigns elsewhere into `parts`.
    pub fn split_into(&mut self, parts: &mut [&mut Spawners], owner: impl Fn(ChunkPos) -> usize) {
        for (p, e) in std::mem::take(&mut self.map) {
            parts[owner(chunk_of(p))].map.insert(p, e);
        }
    }

    pub fn merge(&mut self, from: Spawners) {
        self.map.extend(from.map);
    }
}

/// After the chunk set a block at `pos` (keeps the spawners in step with block changes).
pub(crate) fn block_set(level: &mut RegionLevel, pos: BlockPos) {
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let now = level.cells.chunk(chunk_of(pos)).and_then(|c| c.block_entity(x, pos.y, z)).cloned();
    level.blocks.spawners.block_changed(pos, now.as_ref());
}

/// `Level.tickBlockEntities` for the spawners in ticking chunks, in position order.
pub(crate) fn tick_all(sim: &mut SimLevel, ticking: &Ticking) {
    let Some(level) = sim.level.region() else { return };
    // Only the spawners a player is near do anything: found without taking them out.
    let due: Vec<BlockPos> = level.blocks.spawners.map.iter().filter(|(p, _)| ticking.contains(chunk_of(**p))).map(|(p, _)| *p).collect();
    for p in due {
        let Some(range) = sim.level.region().and_then(|l| l.blocks.spawners.map.get(&p)).map(|e| e.be.required_player_range) else { continue };
        if !kiln_entity::spawner::player_near(sim, to_entity(p), range) {
            continue;
        }
        let Some(mut e) = sim.level.region().and_then(|l| l.blocks.spawners.map.remove(&p)) else { continue };
        // What the spawner draws for its entities' seeds depends on the spawner alone.
        (sim.current, sim.seeds) = (0, (p.x as u32 as u64) << 40 ^ (p.y as u32 as u64) << 20 ^ p.z as u32 as u64 ^ 0x53_5057_4e);
        kiln_entity::spawner::tick(sim, to_entity(p), &mut e.be);
        e.dirty = true;
        if let Some(l) = sim.level.region() {
            l.blocks.spawners.map.insert(p, e);
        }
    }
}

/// `SpawnEggItem.useOn` on a block that is a spawner: `Some(true)` when the egg went into it
/// (the block's data change goes out), `Some(false)` when the `spawner_blocks_work` rule is off
/// (the player is told), `None` when the block is no spawner.
pub(crate) fn use_egg(level: &mut RegionLevel, pos: BlockPos, entity_type: &str) -> Option<bool> {
    level.blocks.spawners.map.get(&pos)?;
    if !level.env.mobs.spawner_blocks {
        return Some(false);
    }
    // `setEntityId(type, level.getRandom())`: the draw (when nothing was chosen yet) comes from a
    // random of the spawner and the tick.
    let mut r = kiln_entity::spawner::egg_random(level.env.seed, level.env.game_time, to_entity(pos));
    let e = level.blocks.spawners.map.get_mut(&pos)?;
    e.be.set_entity_id(entity_type, &mut r);
    e.dirty = true;
    // `Level.sendBlockUpdated`: the chunk's block entity follows, and the players that have the chunk get it.
    let chunk_pos = chunk_of(pos);
    if let Some(chunk) = level.cells.chunk_mut(chunk_pos) {
        level.blocks.spawners.store(chunk_pos, chunk);
    }
    level.out.changed.push([pos.x, pos.y, pos.z]);
    Some(true)
}
