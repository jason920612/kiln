//! Mob spawner blocks (`SpawnerBlockEntity`) and trial spawners (`TrialSpawnerBlockEntity`): the
//! block entity's state lives next to the region's other block entities (decoded when its chunk
//! enters the region, following chunks through merges and splits, written back when the chunk is
//! stored), and ticks after the entities of the tick with the region's entities in reach; its
//! behaviour is kiln-entity's ([`kiln_entity::spawner`] and [`kiln_entity::trial_spawner`], the
//! first compared bit for bit with vanilla by `mob_parity`, the second by `interact_parity`).

use crate::blocks::{RegionLevel, Ticking};
use crate::entities::SimLevel;
use kiln_blocks::{BlockPos, Level as _};
use kiln_entity::spawner::SpawnerBe;
use kiln_entity::trial_spawner::TrialBe;
use kiln_proto::nbt::Tag;
use kiln_world::block_entity::{BlockEntity, type_name};
use kiln_world::chunk::Chunk;
use kiln_world::{Blocks, ChunkPos};
use std::collections::BTreeMap;

const TYPE: &str = "minecraft:mob_spawner";
const TRIAL_TYPE: &str = kiln_entity::trial_spawner::TYPE;

fn ours(name: &str) -> bool {
    name == TYPE || name == TRIAL_TYPE
}

/// The two kinds of spawner.
#[derive(Clone, Debug)]
pub(crate) enum Be {
    Mob(SpawnerBe),
    Trial(TrialBe),
}

/// A spawner block entity's live state.
#[derive(Clone, Debug)]
pub(crate) struct SpawnerEntry {
    pub be: Be,
    pub type_id: u16,
    /// Changed since its NBT was last written into the chunk.
    pub dirty: bool,
}

impl SpawnerEntry {
    fn load(type_id: u16, nbt: &Tag) -> SpawnerEntry {
        let be = if type_name(type_id) == TRIAL_TYPE { Be::Trial(TrialBe::load(nbt)) } else { Be::Mob(SpawnerBe::load(nbt)) };
        SpawnerEntry { be, type_id, dirty: false }
    }

    /// The saved compound (`saveCustomOnly`, with the block entity's `id`).
    pub fn save(&self) -> Tag {
        let (id, fields) = match &self.be {
            Be::Mob(b) => (TYPE, b.save()),
            Be::Trial(b) => (TRIAL_TYPE, b.save()),
        };
        let mut f = vec![("id".to_owned(), Tag::String(id.to_owned()))];
        f.extend(fields);
        Tag::Compound(f)
    }
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

fn to_entity(p: BlockPos) -> kiln_entity::math::BlockPos {
    kiln_entity::math::BlockPos::new(p.x, p.y, p.z)
}

/// A region's spawners.
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
            if ours(type_name(be.kind)) {
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
        let now = now.filter(|be| ours(type_name(be.kind)));
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
    // Only the mob spawners a player is near do anything: found without taking them out.
    let due: Vec<BlockPos> = level.blocks.spawners.map.iter().filter(|(p, _)| ticking.contains(chunk_of(**p))).map(|(p, _)| *p).collect();
    for p in due {
        let Some(entry) = sim.level.region().and_then(|l| l.blocks.spawners.map.get(&p)) else { continue };
        match &entry.be {
            Be::Mob(b) => {
                let range = b.required_player_range;
                if !kiln_entity::spawner::player_near(sim, to_entity(p), range) {
                    continue;
                }
            }
            // A trial spawner works its state machine whether anyone is near or not.
            Be::Trial(_) => {}
        }
        let Some(mut e) = sim.level.region().and_then(|l| l.blocks.spawners.map.remove(&p)) else { continue };
        // What the spawner draws for its entities' seeds depends on the spawner alone.
        (sim.current, sim.seeds) = (0, (p.x as u32 as u64) << 40 ^ (p.y as u32 as u64) << 20 ^ p.z as u32 as u64 ^ 0x53_5057_4e);
        match &mut e.be {
            Be::Mob(b) => {
                kiln_entity::spawner::tick(sim, to_entity(p), b);
                e.dirty = true;
            }
            Be::Trial(b) => {
                kiln_entity::trial_spawner::tick(sim, to_entity(p), b);
                if std::mem::take(&mut b.changed) {
                    e.dirty = true;
                }
            }
        }
        let (trial, updated) = match &mut e.be {
            Be::Trial(b) => (true, std::mem::take(&mut b.updated)),
            Be::Mob(_) => (false, false),
        };
        let sync = trial && e.dirty;
        if let Some(l) = sim.level.region() {
            l.blocks.spawners.map.insert(p, e);
            // The chunk's block entity follows (a block change this tick sends it to the players that have the chunk,
            // `Level.sendBlockUpdated` asks for it too).
            if sync {
                let chunk_pos = chunk_of(p);
                if let Some(chunk) = l.cells.chunk_mut(chunk_pos) {
                    l.blocks.spawners.store(chunk_pos, chunk);
                }
            }
            if updated {
                l.out.changed.push([p.x, p.y, p.z]);
            }
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
    let configs = level.env.trial_configs.clone();
    let e = level.blocks.spawners.map.get_mut(&pos)?;
    let trial = match &mut e.be {
        Be::Mob(b) => {
            b.set_entity_id(entity_type, &mut r);
            false
        }
        // `TrialSpawnerBlockEntity.setEntityId`: its data starts over and only that entity spawns.
        Be::Trial(b) => {
            b.override_entity(entity_type, &|k| configs.get(k));
            true
        }
    };
    e.dirty = true;
    // ... and the trial spawner goes inactive (that block change sends the block entity along).
    let mut state_changed = false;
    if trial {
        let s = level.block(pos);
        if let Some(new) = kiln_data::blocks_types::block_of(s).with_property(s, "trial_spawner_state", "inactive")
            && new != s
        {
            kiln_blocks::set_block_and_update(level, pos, new);
            state_changed = true;
        }
    }
    // The chunk's block entity follows, and the players that have the chunk get it.
    let chunk_pos = chunk_of(pos);
    if let Some(chunk) = level.cells.chunk_mut(chunk_pos) {
        level.blocks.spawners.store(chunk_pos, chunk);
    }
    // `Level.sendBlockUpdated`.
    if !state_changed {
        level.out.changed.push([pos.x, pos.y, pos.z]);
    }
    Some(true)
}

/// The `trial_spawner` configs of a datapack (`data/<namespace>/trial_spawner/**.json`), by key.
#[derive(Debug, Default)]
pub(crate) struct TrialConfigs {
    map: std::collections::HashMap<String, std::sync::Arc<kiln_entity::trial_spawner::Config>>,
}

/// JSON as the NBT `NbtOps` makes of it (a number with a fraction is a double, any other an int
/// or long; booleans are bytes).
fn json_tag(v: &serde_json::Value) -> Tag {
    use serde_json::Value as J;
    match v {
        J::Null => Tag::Compound(Vec::new()),
        J::Bool(b) => Tag::Byte(i8::from(*b)),
        J::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => Tag::Int(i as i32),
            Some(i) => Tag::Long(i),
            None => Tag::Double(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => Tag::String(s.clone()),
        J::Array(a) => Tag::List(a.iter().map(json_tag).collect()),
        J::Object(o) => Tag::Compound(o.iter().map(|(k, v)| (k.clone(), json_tag(v))).collect()),
    }
}

impl TrialConfigs {
    /// Reads the configs of the datapack at `dir`.
    pub fn load(dir: &std::path::Path) -> TrialConfigs {
        let mut map = std::collections::HashMap::new();
        let Ok(namespaces) = std::fs::read_dir(dir.join("data")) else { return TrialConfigs { map } };
        for ns in namespaces.flatten() {
            let root = ns.path().join("trial_spawner");
            let ns_name = ns.file_name().to_string_lossy().into_owned();
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&d) else { continue };
                for e in entries.flatten() {
                    let path = e.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|x| x == "json") {
                        let Ok(rel) = path.strip_prefix(&root) else { continue };
                        let key = format!("{ns_name}:{}", rel.with_extension("").to_string_lossy().replace('\\', "/"));
                        let Ok(text) = std::fs::read_to_string(&path) else { continue };
                        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                        if let Some(c) = kiln_entity::trial_spawner::Config::parse(&json_tag(&json)) {
                            map.insert(key, std::sync::Arc::new(c));
                        }
                    }
                }
            }
        }
        TrialConfigs { map }
    }

    pub fn get(&self, key: &str) -> Option<std::sync::Arc<kiln_entity::trial_spawner::Config>> {
        self.map.get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}
