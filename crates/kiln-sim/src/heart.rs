//! Creaking hearts (`CreakingHeartBlockEntity`): the block entity's state lives next to the
//! region's other block entities (decoded when its chunk enters the region, following chunks
//! through merges and splits, written back when the chunk is stored), and ticks after the
//! entities of the tick with the region's entities in reach; its behaviour is kiln-entity's
//! ([`kiln_entity::mob::kinds::creaking_heart`]).
//!
//! A heart holds its creaking by UUID. Losing the heart (its block replaced or broken, by
//! anything) lets the creaking go: the block phase leaves a [`Release`] and the next entity
//! phase tears the creaking down (`preRemoveSideEffects`), or, for a player breaking the block
//! (`playerWillDestroy`), makes it twitch and die.

use crate::blocks::{RegionLevel, Ticking};
use crate::entities::SimLevel;
use kiln_blocks::BlockPos;
use kiln_entity::mob::kinds::creaking_heart::{self as heart, HeartBe};
use kiln_proto::nbt::Tag;
use kiln_world::block_entity::{BlockEntity, type_name};
use kiln_world::chunk::Chunk;
use kiln_world::{Blocks, ChunkPos};
use std::collections::BTreeMap;

const TYPE: &str = "minecraft:creaking_heart";

/// A creaking whose heart is gone: to let go (`source`: `None`) or to kill (the breaking
/// player's entity id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub uuid: u128,
    pub pos: BlockPos,
    pub source: Option<i32>,
}

/// A heart block entity's live state.
#[derive(Clone, Debug)]
pub(crate) struct HeartEntry {
    pub be: HeartBe,
    pub type_id: u16,
    /// Changed since its NBT was last written into the chunk.
    pub dirty: bool,
    /// The saved fields the heart does not model.
    extra: Vec<(String, Tag)>,
}

impl HeartEntry {
    fn load(type_id: u16, nbt: &Tag) -> HeartEntry {
        let uuid = match nbt.get("creaking") {
            Some(Tag::IntArray(a)) if a.len() == 4 => Some(a.iter().fold(0u128, |acc, x| (acc << 32) | (*x as u32 as u128))),
            _ => None,
        };
        let extra = match nbt {
            Tag::Compound(f) => f.iter().filter(|(k, _)| !matches!(k.as_str(), "creaking" | "id" | "x" | "y" | "z")).cloned().collect(),
            _ => Vec::new(),
        };
        HeartEntry { be: HeartBe::load(uuid), type_id, dirty: false, extra }
    }

    /// `saveAdditional`.
    pub fn save(&self) -> Tag {
        let mut f = self.extra.clone();
        if let Some(u) = self.be.saved_uuid() {
            f.push(("creaking".into(), Tag::IntArray(vec![(u >> 96) as u32 as i32, (u >> 64) as u32 as i32, (u >> 32) as u32 as i32, u as u32 as i32])));
        }
        Tag::Compound(f)
    }
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

fn to_entity(p: BlockPos) -> kiln_entity::math::BlockPos {
    kiln_entity::math::BlockPos::new(p.x, p.y, p.z)
}

/// A region's creaking hearts.
#[derive(Default)]
pub(crate) struct Hearts {
    pub map: BTreeMap<BlockPos, HeartEntry>,
    /// Creakings that lost their heart in the block phase, in order.
    pub released: Vec<Release>,
}

impl Hearts {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty() && self.released.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// A chunk entered the region: its hearts are decoded.
    pub fn chunk_loaded(&mut self, pos: ChunkPos, chunk: &Chunk) {
        for ((x, y, z), be) in chunk.block_entities() {
            if type_name(be.kind) == TYPE {
                let at = BlockPos::new(pos.x * 16 + x as i32, y, pos.z * 16 + z as i32);
                self.map.insert(at, HeartEntry::load(be.kind, &be.nbt));
            }
        }
    }

    fn chunk_positions(&self, pos: ChunkPos) -> Vec<BlockPos> {
        let lo = BlockPos::new(pos.x * 16, i32::MIN, pos.z * 16);
        let hi = BlockPos::new(pos.x * 16 + 15, i32::MAX, pos.z * 16 + 15);
        self.map.range(lo..=hi).map(|(p, _)| *p).filter(|p| chunk_of(*p) == pos).collect()
    }

    /// Writes the chunk's changed hearts into its NBT.
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
                o.extend(fields);
            }
            chunk.set_block_entity(x, p.y, z, out);
        }
    }

    /// A chunk left the region (after [`Hearts::store`]).
    pub fn chunk_unloaded(&mut self, pos: ChunkPos) {
        for p in self.chunk_positions(pos) {
            self.map.remove(&p);
        }
    }

    /// After the chunk set a block at `pos`: a heart that went away lets its creaking go and
    /// is dropped; a new one is decoded.
    pub fn block_changed(&mut self, pos: BlockPos, now: Option<&BlockEntity>) {
        let now = now.filter(|be| type_name(be.kind) == TYPE);
        let kept = match (self.map.get(&pos), now) {
            (Some(h), Some(be)) => h.type_id == be.kind,
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !kept && let Some(h) = self.map.remove(&pos) {
            self.release(pos, h.be.saved_uuid(), None);
        }
        if let Some(be) = now
            && !self.map.contains_key(&pos)
        {
            self.map.insert(pos, HeartEntry::load(be.kind, &be.nbt));
        }
    }

    /// The chunk's block entity at `pos` was replaced from outside (commands): reload it.
    pub fn reload(&mut self, pos: BlockPos, be: Option<&BlockEntity>) {
        self.map.remove(&pos);
        self.block_changed(pos, be);
    }

    fn release(&mut self, pos: BlockPos, uuid: Option<u128>, source: Option<i32>) {
        if let Some(uuid) = uuid {
            self.released.push(Release { uuid, pos, source });
        }
    }

    /// Moves the hearts of chunks `owner` assigns elsewhere into `parts`.
    pub fn split_into(&mut self, parts: &mut [&mut Hearts], owner: impl Fn(ChunkPos) -> usize) {
        for (p, e) in std::mem::take(&mut self.map) {
            parts[owner(chunk_of(p))].map.insert(p, e);
        }
        for r in std::mem::take(&mut self.released) {
            parts[owner(chunk_of(r.pos))].released.push(r);
        }
    }

    pub fn merge(&mut self, from: Hearts) {
        self.map.extend(from.map);
        self.released.extend(from.released);
    }
}

/// After the chunk set a block at `pos` (keeps the hearts in step with block changes).
pub(crate) fn block_set(level: &mut RegionLevel, pos: BlockPos) {
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let now = level.cells.chunk(chunk_of(pos)).and_then(|c| c.block_entity(x, pos.y, z)).cloned();
    level.blocks.hearts.block_changed(pos, now.as_ref());
}

/// `CreakingHeartBlock.playerWillDestroy` (a player is about to break the block at `pos`): the
/// creaking dies twitching at the next entity phase, and a natural heart pays experience to a
/// survival player (`tryAwardExperience`).
pub(crate) fn player_will_destroy(level: &mut RegionLevel, pos: BlockPos, state: u16, player: i32, survival: bool) {
    if !heart::is_heart(state) {
        return;
    }
    let uuid = level.blocks.hearts.map.get_mut(&pos).and_then(|e| {
        let u = e.be.saved_uuid();
        e.be.link = None;
        e.dirty = true;
        u
    });
    level.blocks.hearts.release(pos, uuid, Some(player));
    // `tryAwardExperience`: natural hearts, not for creative or spectator players.
    if survival && kiln_blocks::state::get(state, "natural") == Some("true") && level.env.drops {
        use kiln_javamath::random::RandomSource;
        let mut rng = crate::container::pos_random(level, pos, 0x4845_5852);
        let amount = rng.next_int_bounded(5) + 20;
        let mut spawns = Vec::new();
        crate::container::furnace::award_experience([pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5], amount, &mut rng, &mut spawns);
        level.out.spawns.extend(spawns);
    }
}

/// `getAnalogOutputSignal` of a heart: what the block entity last computed (0 uprooted).
pub(crate) fn analog(level: &RegionLevel, pos: BlockPos, state: u16) -> Option<i32> {
    if !heart::is_heart(state) {
        return None;
    }
    Some(if heart::heart_state(state) == "uprooted" { 0 } else { level.blocks.hearts.map.get(&pos).map_or(0, |e| e.be.output_signal) })
}

/// The start of the entity phase: creakings that lost their heart are let go or killed.
pub(crate) fn process_released(sim: &mut SimLevel) {
    if sim.level.blocks.hearts.released.is_empty() {
        return;
    }
    for r in std::mem::take(&mut sim.level.blocks.hearts.released) {
        let mut be = HeartBe::load(Some(r.uuid));
        // The creaking may still be spawning (a fresh spawn is not in the list yet): it stays,
        // and, having no heart, dies of it at its first tick.
        be.ticks_existed = 30;
        let source = r.source.map(|p| kiln_entity::mob::DamageSource {
            kind: kiln_entity::level::DamageKind::PlayerAttack,
            attacker: Some(p),
            direct: Some(p),
            pos: None,
            attacker_is_player: true,
        });
        heart::remove_protector(sim, to_entity(r.pos), &mut be, source);
    }
}

/// `Level.tickBlockEntities` for the hearts in ticking chunks, in position order.
pub(crate) fn tick_all(sim: &mut SimLevel, ticking: &Ticking) {
    let due: Vec<BlockPos> = sim.level.blocks.hearts.map.keys().filter(|p| ticking.contains(chunk_of(**p))).copied().collect();
    for p in due {
        let Some(mut e) = sim.level.blocks.hearts.map.remove(&p) else { continue };
        let before = e.be.saved_uuid();
        // What the heart draws for its entities' seeds depends on the heart alone.
        (sim.current, sim.seeds) = (0, (p.x as u32 as u64) << 40 ^ (p.y as u32 as u64) << 20 ^ p.z as u32 as u64);
        heart::tick(sim, to_entity(p), &mut e.be);
        if e.be.saved_uuid() != before {
            e.dirty = true;
        }
        sim.level.blocks.hearts.map.insert(p, e);
    }
}

/// Whether the heart at `pos` holds creaking `id` (`uuid`).
pub(crate) fn protects(level: &RegionLevel, pos: BlockPos, id: i32, uuid: u128) -> bool {
    level.blocks.hearts.map.get(&pos).is_some_and(|e| e.be.protects(id, uuid))
}

/// Runs the heart at `pos` (taken out of the map) on `f`.
pub(crate) fn with_heart<R>(sim: &mut SimLevel, pos: BlockPos, f: impl FnOnce(&mut SimLevel, &mut HeartBe) -> R) -> Option<R> {
    let mut e = sim.level.blocks.hearts.map.remove(&pos)?;
    let before = e.be.saved_uuid();
    let r = f(sim, &mut e.be);
    if e.be.saved_uuid() != before {
        e.dirty = true;
    }
    sim.level.blocks.hearts.map.insert(pos, e);
    Some(r)
}

