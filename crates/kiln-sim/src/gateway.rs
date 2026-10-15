//! End gateways' block entity clock (`TheEndGatewayBlockEntity.portalTick`): every tick of a
//! ticking chunk `Age` goes up by one, and every 2400 ticks of age (when no cooldown runs) the
//! gateway starts the 40 tick cooldown that keeps entities from entering it. The age lives
//! here, next to the region's other block entities (decoded when the chunk enters the region,
//! following chunks through merges and splits, written back when the chunk is stored); the
//! cooldown that entering sets is kept by the level (`portal.rs`).

use crate::blocks::{RegionLevel, Ticking};
use kiln_blocks::BlockPos;
use kiln_proto::nbt::Tag;
use kiln_world::block_entity::{BlockEntity, type_name};
use kiln_world::chunk::Chunk;
use kiln_world::{Blocks, ChunkPos};
use std::collections::BTreeMap;

const TYPE: &str = "minecraft:end_gateway";
/// `TheEndGatewayBlockEntity.COOLDOWN_TIME`.
pub(crate) const COOLDOWN_TIME: i64 = 40;
/// `TheEndGatewayBlockEntity.portalTick`: the cooldown that starts by itself every this many ticks of age.
const PERIOD: i64 = 2400;

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

fn age_of(be: &BlockEntity) -> i64 {
    be.nbt.get("Age").and_then(Tag::as_i64).unwrap_or(0)
}

/// Whether a gateway of this age is in the cooldown its own clock started (`age % 2400 == 0` starts one of 40 ticks;
/// the one an entity started, if it was running then, is the level's).
pub(crate) fn periodic_cooldown(age: i64) -> bool {
    age >= PERIOD && age % PERIOD < COOLDOWN_TIME
}

/// A region's end gateways: position to age.
#[derive(Default)]
pub(crate) struct Gateways {
    pub map: BTreeMap<BlockPos, i64>,
    /// Ages changed since they were last written into their chunks.
    dirty: bool,
}

impl Gateways {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// A chunk entered the region: its gateways are decoded.
    pub fn chunk_loaded(&mut self, pos: ChunkPos, chunk: &Chunk) {
        for ((x, y, z), be) in chunk.block_entities() {
            if type_name(be.kind) == TYPE {
                self.map.insert(BlockPos::new(pos.x * 16 + x as i32, y, pos.z * 16 + z as i32), age_of(be));
            }
        }
    }

    fn chunk_positions(&self, pos: ChunkPos) -> Vec<BlockPos> {
        let lo = BlockPos::new(pos.x * 16, i32::MIN, pos.z * 16);
        let hi = BlockPos::new(pos.x * 16 + 15, i32::MAX, pos.z * 16 + 15);
        self.map.range(lo..=hi).map(|(p, _)| *p).filter(|p| chunk_of(*p) == pos).collect()
    }

    /// Writes the chunk's gateways' ages into its NBT.
    pub fn store(&mut self, pos: ChunkPos, chunk: &mut Chunk) {
        if !self.dirty {
            return;
        }
        for p in self.chunk_positions(pos) {
            let (x, z) = ((p.x & 15) as usize, (p.z & 15) as usize);
            let Some(mut be) = chunk.block_entity(x, p.y, z).filter(|be| type_name(be.kind) == TYPE).cloned() else { continue };
            let age = self.map[&p];
            if age_of(&be) != age {
                be.merge_fields(vec![("Age".into(), Tag::Long(age))]);
                chunk.set_block_entity(x, p.y, z, be);
            }
        }
    }

    /// A chunk left the region (after [`Gateways::store`]).
    pub fn chunk_unloaded(&mut self, pos: ChunkPos) {
        for p in self.chunk_positions(pos) {
            self.map.remove(&p);
        }
    }

    /// After the chunk set a block at `pos`: a gateway that went away is dropped, a new one starts at its saved age.
    pub fn block_changed(&mut self, pos: BlockPos, now: Option<&BlockEntity>) {
        match now.filter(|be| type_name(be.kind) == TYPE) {
            Some(be) => {
                self.map.entry(pos).or_insert_with(|| age_of(be));
            }
            None => {
                self.map.remove(&pos);
            }
        }
    }

    /// The chunk's block entity at `pos` was replaced from outside (commands): its age is read again.
    pub fn reload(&mut self, pos: BlockPos, be: Option<&BlockEntity>) {
        self.map.remove(&pos);
        self.block_changed(pos, be);
    }

    /// Moves the gateways of chunks `owner` assigns elsewhere into `parts`.
    pub fn split_into(&mut self, parts: &mut [&mut Gateways], owner: impl Fn(ChunkPos) -> usize) {
        for (p, age) in std::mem::take(&mut self.map) {
            parts[owner(chunk_of(p))].map.insert(p, age);
        }
        for part in parts.iter_mut() {
            part.dirty = self.dirty;
        }
    }

    pub fn merge(&mut self, from: Gateways) {
        self.map.extend(from.map);
        self.dirty |= from.dirty;
    }
}

/// After the chunk set a block at `pos` (keeps the gateways in step with block changes).
pub(crate) fn block_set(level: &mut RegionLevel, pos: BlockPos) {
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let now = level.cells.chunk(chunk_of(pos)).and_then(|c| c.block_entity(x, pos.y, z)).cloned();
    level.blocks.gateways.block_changed(pos, now.as_ref());
}

/// `Level.tickBlockEntities` for the gateways in ticking chunks: one tick of age each.
pub(crate) fn tick(level: &mut RegionLevel, ticking: &Ticking) {
    let g = &mut level.blocks.gateways;
    for (p, age) in g.map.iter_mut() {
        if ticking.contains(chunk_of(*p)) {
            *age += 1;
            g.dirty = true;
        }
    }
}
