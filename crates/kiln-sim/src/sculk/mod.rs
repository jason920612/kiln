//! Game events and the listeners that hear them (`GameEventDispatcher`,
//! `GameEventListenerRegistry`), and the sculk block entities: sensors and calibrated sensors
//! (`SculkSensorBlockEntity`), shriekers (`SculkShriekerBlockEntity`) and catalysts
//! (`SculkCatalystBlockEntity`). Wardens listen through [`Ear`]s.
//!
//! Region rule: a game event reaches the listeners in the loaded chunks of the region it
//! happens in. A game event reaches at most 32 blocks (a shriek; most reach 16) and the loaded
//! chunks of different regions are at least 256 blocks apart (kiln-region's link distance), so
//! every listener vanilla would notify is in the event's own region, however the world is
//! split, and a listener near a region edge hears exactly what it would in one region.
//!
//! Order: vanilla visits the listener registries of the sections around the event (chunk x,
//! then chunk z, then section y) and within a section the listeners in the order they
//! registered; `BY_DISTANCE` listeners (catalysts) are handled afterwards, nearest first. Kiln
//! keeps block entity listeners of a section in position order and wardens after them in id
//! order (an approximation, I class: vanilla registers in load order). Listeners only compete
//! through ties, so the order rarely shows.
//!
//! The block entities live next to the region's block machinery like containers: decoded when
//! their chunk enters the region, following chunks through merges and splits, written back to
//! the chunk's NBT when it is stored, ticked in position order.

pub(crate) mod catalyst;
pub(crate) mod shrieker;

use crate::blocks::RegionLevel;
use kiln_blocks::behaviour::sculk as blocks;
use kiln_blocks::{BlockPos, Direction, Level};
use kiln_entity::math::Vec3;
use kiln_entity::vibration::{self, Context, Ear, EventSource, Heard, Validity, VibrationData, VibrationInfo};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::world_fx;
use kiln_world::block_entity::{BlockEntity, type_name};
use kiln_world::chunk::Chunk;
use kiln_world::{Blocks, ChunkPos};
use std::collections::{BTreeMap, HashMap};

/// Which listening block entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Sensor,
    Calibrated,
    Shrieker,
    Catalyst,
}

impl Kind {
    fn by_type(name: &str) -> Option<Kind> {
        Some(match name {
            "minecraft:sculk_sensor" => Kind::Sensor,
            "minecraft:calibrated_sculk_sensor" => Kind::Calibrated,
            "minecraft:sculk_shrieker" => Kind::Shrieker,
            "minecraft:sculk_catalyst" => Kind::Catalyst,
            _ => return None,
        })
    }

    /// `getListenerRadius`.
    pub fn radius(self) -> i32 {
        match self {
            Kind::Calibrated => 16,
            _ => 8,
        }
    }
}

/// Saved fields a sculk block entity models; the rest of its NBT is kept as is.
const MODELED: [&str; 10] = ["listener", "last_vibration_frequency", "warning_level", "cursors", "id", "x", "y", "z", "keepPacked", "components"];

/// A listening block entity's live state.
#[derive(Clone, Debug)]
pub(crate) struct SculkBe {
    pub kind: Kind,
    pub type_id: u16,
    /// Sensors and shriekers: `VibrationSystem.Data`.
    pub vibration: VibrationData,
    /// `SculkSensorBlockEntity.lastVibrationFrequency`.
    pub last_frequency: i32,
    /// `SculkShriekerBlockEntity.warningLevel`.
    pub warning_level: i32,
    /// A catalyst's `SculkSpreader` charge cursors.
    pub cursors: Vec<catalyst::Cursor>,
    /// Changed since its NBT was last written into the chunk.
    pub dirty: bool,
    extra: Vec<(String, Tag)>,
}

impl SculkBe {
    fn load(kind: Kind, type_id: u16, nbt: &Tag) -> SculkBe {
        let int = |k: &str| nbt.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
        let extra = match nbt {
            Tag::Compound(f) => f.iter().filter(|(k, _)| !MODELED.contains(&k.as_str()) || k == "components").cloned().collect(),
            _ => Vec::new(),
        };
        SculkBe {
            kind,
            type_id,
            vibration: VibrationData::from_nbt(nbt.get("listener")),
            last_frequency: int("last_vibration_frequency"),
            warning_level: int("warning_level"),
            cursors: catalyst::load_cursors(nbt.get("cursors")),
            dirty: false,
            extra,
        }
    }

    /// `saveAdditional`.
    pub fn save(&self) -> Tag {
        let mut f = self.extra.clone();
        match self.kind {
            Kind::Sensor | Kind::Calibrated => {
                f.push(("last_vibration_frequency".into(), Tag::Int(self.last_frequency)));
                f.push(("listener".into(), self.vibration.to_nbt()));
            }
            Kind::Shrieker => {
                f.push(("warning_level".into(), Tag::Int(self.warning_level)));
                f.push(("listener".into(), self.vibration.to_nbt()));
            }
            Kind::Catalyst => f.push(("cursors".into(), catalyst::save_cursors(&self.cursors))),
        }
        Tag::Compound(f)
    }
}

type SectionKey = (i32, i32, i32);

fn section_of(p: BlockPos) -> SectionKey {
    (p.x >> 4, p.y >> 4, p.z >> 4)
}

fn section_of_vec(p: Vec3) -> SectionKey {
    (kiln_entity::math::floor(p.x) >> 4, kiln_entity::math::floor(p.y) >> 4, kiln_entity::math::floor(p.z) >> 4)
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

fn center(p: BlockPos) -> Vec3 {
    Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5)
}

fn containing(v: Vec3) -> BlockPos {
    BlockPos::new(kiln_entity::math::floor(v.x), kiln_entity::math::floor(v.y), kiln_entity::math::floor(v.z))
}

/// A region's game event listeners: the sculk block entities of its chunks and its wardens.
#[derive(Default)]
pub(crate) struct Sculk {
    pub map: BTreeMap<BlockPos, SculkBe>,
    /// Block entity listeners by section (`LevelChunk.getListenerRegistry`), in position order.
    sections: HashMap<SectionKey, Vec<BlockPos>>,
    /// Warden listeners (`DynamicGameEventListener`) by entity id, and by section.
    pub wardens: BTreeMap<i32, Ear>,
    warden_sections: HashMap<SectionKey, Vec<i32>>,
    /// Vibrations wardens heard since their last tick, in order. They reach the warden's
    /// selector before its next tick, as they would have when heard (the selector is only read
    /// in the warden's tick).
    pub heard: Vec<(i32, Heard)>,
}

impl Sculk {
    /// No listener at all: game events cost nothing.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty() && self.wardens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    fn insert(&mut self, pos: BlockPos, be: SculkBe) {
        if self.map.insert(pos, be).is_none() {
            let v = self.sections.entry(section_of(pos)).or_default();
            let i = v.binary_search(&pos).unwrap_or_else(|i| i);
            v.insert(i, pos);
        }
    }

    fn remove(&mut self, pos: BlockPos) -> Option<SculkBe> {
        let be = self.map.remove(&pos)?;
        let key = section_of(pos);
        if let Some(v) = self.sections.get_mut(&key) {
            v.retain(|p| *p != pos);
            if v.is_empty() {
                self.sections.remove(&key);
            }
        }
        Some(be)
    }

    /// A chunk entered the region: its listening block entities are decoded.
    pub fn chunk_loaded(&mut self, pos: ChunkPos, chunk: &Chunk) {
        for ((x, y, z), be) in chunk.block_entities() {
            if let Some(kind) = Kind::by_type(type_name(be.kind)) {
                let at = BlockPos::new(pos.x * 16 + x as i32, y, pos.z * 16 + z as i32);
                self.insert(at, SculkBe::load(kind, be.kind, &be.nbt));
            }
        }
    }

    fn chunk_positions(&self, pos: ChunkPos) -> Vec<BlockPos> {
        let lo = BlockPos::new(pos.x * 16, i32::MIN, pos.z * 16);
        let hi = BlockPos::new(pos.x * 16 + 15, i32::MAX, pos.z * 16 + 15);
        self.map.range(lo..=hi).map(|(p, _)| *p).filter(|p| chunk_of(*p) == pos).collect()
    }

    /// Writes the chunk's changed block entities into its NBT.
    pub fn store(&mut self, pos: ChunkPos, chunk: &mut Chunk) {
        for p in self.chunk_positions(pos) {
            let Some(be) = self.map.get_mut(&p).filter(|b| b.dirty) else { continue };
            be.dirty = false;
            let (x, z) = ((p.x & 15) as usize, (p.z & 15) as usize);
            if chunk.block_entity(x, p.y, z).is_none_or(|old| old.kind != be.type_id) {
                continue;
            }
            let mut out = BlockEntity::new(be.type_id);
            if let (Tag::Compound(o), Tag::Compound(fields)) = (&mut out.nbt, be.save()) {
                o.extend(fields);
            }
            chunk.set_block_entity(x, p.y, z, out);
        }
    }

    /// A chunk left the region (after [`Sculk::store`]).
    pub fn chunk_unloaded(&mut self, pos: ChunkPos) {
        for p in self.chunk_positions(pos) {
            self.remove(p);
        }
    }

    /// After the chunk set a block at `pos`: a listener that went away is dropped and a new
    /// one decoded.
    pub fn block_changed(&mut self, pos: BlockPos, now: Option<&BlockEntity>) {
        let now = now.and_then(|be| Some((Kind::by_type(type_name(be.kind))?, be)));
        let kept = match (self.map.get(&pos), now) {
            (Some(l), Some((_, be))) => l.type_id == be.kind,
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !kept {
            self.remove(pos);
        }
        if let Some((kind, be)) = now
            && !self.map.contains_key(&pos)
        {
            self.insert(pos, SculkBe::load(kind, be.kind, &be.nbt));
        }
    }

    /// The chunk's block entity at `pos` was replaced from outside (commands): reload it.
    pub fn reload(&mut self, pos: BlockPos, be: Option<&BlockEntity>) {
        self.remove(pos);
        self.block_changed(pos, be);
    }

    /// Warden `id` listens from `pos` (after its tick; `None`: it is gone).
    pub fn set_warden(&mut self, id: i32, ear: Option<Ear>) {
        if let Some(old) = self.wardens.remove(&id) {
            let key = section_of_vec(old.pos);
            if let Some(v) = self.warden_sections.get_mut(&key) {
                v.retain(|w| *w != id);
                if v.is_empty() {
                    self.warden_sections.remove(&key);
                }
            }
        }
        if let Some(ear) = ear {
            let v = self.warden_sections.entry(section_of_vec(ear.pos)).or_default();
            let i = v.binary_search(&id).unwrap_or_else(|i| i);
            v.insert(i, id);
            self.wardens.insert(id, ear);
        }
    }

    /// Drops the listeners of wardens that are gone (`DynamicGameEventListener.remove`).
    pub fn retain_wardens(&mut self, keep: impl Fn(i32) -> bool) {
        let gone: Vec<i32> = self.wardens.keys().copied().filter(|&id| !keep(id)).collect();
        for id in gone {
            self.set_warden(id, None);
        }
        let wardens = &self.wardens;
        self.heard.retain(|h| wardens.contains_key(&h.0));
    }

    /// The vibrations warden `id` heard since its last tick, in order.
    pub fn take_heard(&mut self, id: i32) -> Vec<Heard> {
        if !self.heard.iter().any(|h| h.0 == id) {
            return Vec::new();
        }
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.heard).into_iter().partition(|h| h.0 == id);
        self.heard = rest;
        mine.into_iter().map(|(_, h)| h).collect()
    }

    /// Moves the listeners of chunks (and wardens of cells) `owner` assigns elsewhere into
    /// `parts`.
    pub fn split_into(&mut self, parts: &mut [&mut Sculk], owner: impl Fn(ChunkPos) -> usize) {
        for (p, be) in std::mem::take(&mut self.map) {
            parts[owner(chunk_of(p))].insert(p, be);
        }
        self.sections.clear();
        for (id, ear) in std::mem::take(&mut self.wardens) {
            let c = ChunkPos::of_block(kiln_entity::math::floor(ear.pos.x), kiln_entity::math::floor(ear.pos.z));
            parts[owner(c)].set_warden(id, Some(ear));
        }
        self.warden_sections.clear();
        let wardens: Vec<BTreeMap<i32, ()>> = parts.iter().map(|p| p.wardens.keys().map(|k| (*k, ())).collect()).collect();
        for h in std::mem::take(&mut self.heard) {
            if let Some(i) = wardens.iter().position(|w| w.contains_key(&h.0)) {
                parts[i].heard.push(h);
            }
        }
    }

    pub fn merge(&mut self, from: Sculk) {
        for (p, be) in from.map {
            self.insert(p, be);
        }
        for (id, ear) in from.wardens {
            self.set_warden(id, Some(ear));
        }
        // Heard vibrations keep their order per warden, which is all that matters.
        self.heard.extend(from.heard);
    }
}

/// The block entity listeners in the sections an event at `pos` with notification `radius`
/// reaches, in vanilla's visiting order (chunk x, chunk z, section y; position order within).
pub(crate) fn listeners_near(level: &RegionLevel, pos: Vec3, radius: i32) -> Vec<BlockPos> {
    let c = containing(pos);
    let mut out = Vec::new();
    for sx in (c.x - radius) >> 4..=(c.x + radius) >> 4 {
        for sz in (c.z - radius) >> 4..=(c.z + radius) >> 4 {
            if level.cells.chunk(ChunkPos::new(sx, sz)).is_none() {
                continue;
            }
            for sy in (c.y - radius) >> 4..=(c.y + radius) >> 4 {
                if let Some(v) = level.blocks.sculk.sections.get(&(sx, sy, sz)) {
                    out.extend_from_slice(v);
                }
            }
        }
    }
    out
}

/// `getPostableListenerPosition`: the listener at block `p` with `radius` hears an event at
/// `pos` (block to block).
pub(crate) fn within(p: BlockPos, pos: Vec3, radius: i32) -> bool {
    let c = containing(pos);
    let d = [(p.x - c.x) as i64, (p.y - c.y) as i64, (p.z - c.z) as i64];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= radius as i64 * radius as i64
}

/// Whether any listener could hear anything in this region.
pub(crate) fn listening(level: &RegionLevel) -> bool {
    !level.blocks.sculk.is_empty()
}

/// `ServerLevel.gameEvent` → `GameEventDispatcher.post`: game event `event` at `pos` reaches
/// every listener in range, in vanilla's order.
pub(crate) fn post(level: &mut RegionLevel, event: &'static str, pos: Vec3, ctx: Context) {
    if level.blocks.sculk.is_empty() {
        return;
    }
    let radius = vibration::notification_radius(event);
    let c = containing(pos);
    for sx in (c.x - radius) >> 4..=(c.x + radius) >> 4 {
        for sz in (c.z - radius) >> 4..=(c.z + radius) >> 4 {
            if level.cells.chunk(ChunkPos::new(sx, sz)).is_none() {
                continue;
            }
            for sy in (c.y - radius) >> 4..=(c.y + radius) >> 4 {
                let key = (sx, sy, sz);
                let blocks: smallvec::SmallVec<[BlockPos; 8]> = level.blocks.sculk.sections.get(&key).map(|v| v.iter().copied().collect()).unwrap_or_default();
                for p in blocks {
                    let Some(kind) = level.blocks.sculk.map.get(&p).map(|b| b.kind) else { continue };
                    // Catalysts (`BY_DISTANCE`) only react to deaths, which reach them through
                    // [`catalyst::nearest`] when the mob dies.
                    if kind == Kind::Catalyst || !within(p, pos, kind.radius()) {
                        continue;
                    }
                    hear_block(level, p, kind, event, pos, &ctx);
                }
                let wardens: smallvec::SmallVec<[i32; 4]> = level.blocks.sculk.warden_sections.get(&key).map(|v| v.iter().copied().collect()).unwrap_or_default();
                for id in wardens {
                    hear_warden(level, id, event, pos, &ctx);
                }
            }
        }
    }
}

/// `VibrationSystem.Listener.handleGameEvent` of the sensor or shrieker at `p`.
fn hear_block(level: &mut RegionLevel, p: BlockPos, kind: Kind, event: &'static str, from: Vec3, ctx: &Context) {
    let Some(be) = level.blocks.sculk.map.get(&p) else { return };
    if be.vibration.current.is_some() {
        return;
    }
    let (tag, avoid) = match kind {
        Kind::Shrieker => ("minecraft:shrieker_can_listen", false),
        _ => ("minecraft:vibrations", true),
    };
    match vibration::is_valid_vibration(event, ctx, tag, avoid) {
        Validity::Valid => {}
        Validity::Invalid => return,
        Validity::Avoided { player } => {
            level.out.triggers.push((player, "minecraft:avoid_vibration"));
            return;
        }
    }
    let dest = center(p);
    if !can_receive(level, p, kind, containing(from), event, ctx) || occluded(level, from, dest) {
        return;
    }
    let now = level.env.game_time;
    if let Some(be) = level.blocks.sculk.map.get_mut(&p) {
        be.vibration.schedule(event, from, dest, ctx.source, None, now);
        be.dirty = true;
    }
}

/// `Warden.VibrationUser`'s side of `handleGameEvent`, noted for the warden's next tick.
fn hear_warden(level: &mut RegionLevel, id: i32, event: &'static str, from: Vec3, ctx: &Context) {
    let Some(ear) = level.blocks.sculk.wardens.get(&id).copied() else { return };
    let (c, e) = (containing(from), containing(ear.pos));
    let d = [(c.x - e.x) as i64, (c.y - e.y) as i64, (c.z - e.z) as i64];
    if ear.busy || d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 16 * 16 {
        return;
    }
    match vibration::is_valid_vibration(event, ctx, "minecraft:warden_can_listen", true) {
        Validity::Valid => {}
        Validity::Invalid => return,
        Validity::Avoided { player } => {
            level.out.triggers.push((player, "minecraft:avoid_vibration"));
            return;
        }
    }
    // `canReceiveVibration`: not a living entity the warden may not target.
    let untargetable = ctx.source.is_some_and(|s| {
        s.living && (s.untargetable || matches!(s.type_name, "minecraft:warden" | "minecraft:armor_stand"))
    });
    if !ear.can_hear || untargetable || occluded(level, from, ear.pos) {
        return;
    }
    let now = level.env.game_time;
    level.blocks.sculk.heard.push((id, Heard { event, from, to: ear.pos, source: ctx.source, tick: now }));
}

/// `Listener.isOccluded` against the region's blocks.
fn occluded(level: &RegionLevel, from: Vec3, to: Vec3) -> bool {
    let block = |p: kiln_entity::math::BlockPos| level.block(BlockPos::new(p.x, p.y, p.z));
    vibration::is_occluded(&block, from, to)
}

/// `User.canReceiveVibration` of the sensor or shrieker at `p` for a vibration from block `at`.
fn can_receive(level: &RegionLevel, p: BlockPos, kind: Kind, at: BlockPos, event: &str, ctx: &Context) -> bool {
    let s = level.block(p);
    match kind {
        Kind::Shrieker => !kiln_blocks::state::get_bool(s, "shrieking") && ctx.source.is_some_and(|s| s.player.is_some()),
        Kind::Calibrated => {
            // `getBackSignal`: the signal into the sensor's back selects one frequency.
            let dir = kiln_blocks::state::get_dir(s, "facing").map_or(Direction::South, Direction::opposite);
            let wanted = kiln_blocks::redstone::signal(level, p.relative(dir), dir);
            (wanted == 0 || vibration::frequency(event) == wanted) && sensor_can_receive(s, p, at, event)
        }
        Kind::Sensor => sensor_can_receive(s, p, at, event),
        Kind::Catalyst => false,
    }
}

/// `SculkSensorBlockEntity.VibrationUser.canReceiveVibration`.
fn sensor_can_receive(s: u16, p: BlockPos, at: BlockPos, event: &str) -> bool {
    if at == p && matches!(event, "minecraft:block_destroy" | "minecraft:block_place") {
        return false;
    }
    vibration::frequency(event) != 0 && blocks::can_activate(s)
}

/// `SculkSensorBlock.stepOn`: an entity (not a warden) on an inactive sensor makes it hear a
/// step from where the entity stands, whatever else it heard (`forceScheduleVibration`).
pub(crate) fn step_on(level: &mut RegionLevel, p: BlockPos, source: EventSource) {
    let Some(kind) = level.blocks.sculk.map.get(&p).map(|b| b.kind) else { return };
    let s = level.block(p);
    match kind {
        Kind::Sensor | Kind::Calibrated => {
            if source.type_name == "minecraft:warden" || !blocks::can_activate(s) {
                return;
            }
            let ctx = Context { source: None, affected_state: Some(s) };
            if !can_receive(level, p, kind, p, "minecraft:step", &ctx) {
                return;
            }
            let now = level.env.game_time;
            if let Some(be) = level.blocks.sculk.map.get_mut(&p) {
                be.vibration.schedule("minecraft:step", source.pos, center(p), Some(source), None, now);
                be.dirty = true;
            }
        }
        Kind::Shrieker => {
            // `SculkShriekerBlock.stepOn`: the player it stands for makes it shriek.
            if let Some(player) = source.player {
                level.out.shrieks.push((p, player));
            }
        }
        Kind::Catalyst => {}
    }
}

/// `Level.tickBlockEntities` for the sculk block entities in ticking chunks, in position order:
/// sensors and shriekers run `VibrationSystem.Ticker.tick`.
pub(crate) fn tick_block_entities(level: &mut RegionLevel, ticking: &crate::blocks::Ticking) {
    let due: Vec<(BlockPos, Kind)> = level
        .blocks
        .sculk
        .map
        .iter()
        .filter(|(p, _)| ticking.contains(chunk_of(**p)))
        .map(|(p, b)| (*p, b.kind))
        .collect();
    for (p, kind) in due {
        if level.blocks.sculk.map.get(&p).is_none_or(|b| b.kind != kind) {
            continue;
        }
        match kind {
            Kind::Catalyst => catalyst::tick(level, p),
            _ => tick_listener(level, p, kind, ticking),
        }
    }
}

/// `VibrationSystem.Ticker.tick` for the sensor or shrieker at `p`.
fn tick_listener(level: &mut RegionLevel, p: BlockPos, kind: Kind, ticking: &crate::blocks::Ticking) {
    let now = level.env.game_time;
    let dest = center(p);
    let Some(be) = level.blocks.sculk.map.get_mut(&p) else { return };
    if be.vibration.current.is_none() && be.vibration.selector.current.is_none() {
        return;
    }
    let t = be.vibration.tick(now, dest, vibration::travel_time);
    if t.changed {
        be.dirty = true;
    }
    for (at, left) in t.particles {
        send_vibration_particle(level, at, world_fx::PositionSource::Block([p.x, p.y, p.z]), left);
    }
    if !t.arrived {
        return;
    }
    // `requiresAdjacentChunksToBeTicking`.
    let c = chunk_of(p);
    let adjacent = (-1..=1).all(|dx| {
        (-1..=1).all(|dz| {
            let n = ChunkPos::new(c.x + dx, c.z + dz);
            ticking.contains(n) && level.cells.chunk(n).is_some()
        })
    });
    if !adjacent {
        return;
    }
    // While it reacts, the vibration is still current, so the listener hears nothing (its own
    // clicking and resonance included), as in vanilla.
    let Some(info) = level.blocks.sculk.map.get(&p).and_then(|b| b.vibration.current.clone()) else { return };
    on_receive(level, p, kind, &info);
    if let Some(be) = level.blocks.sculk.map.get_mut(&p) {
        be.vibration.received();
        be.dirty = true;
    }
}

/// `User.onReceiveVibration`.
fn on_receive(level: &mut RegionLevel, p: BlockPos, kind: Kind, info: &VibrationInfo) {
    let origin = containing(info.pos);
    let distance = vibration::distance_between_in_blocks(to_entity_pos(origin), to_entity_pos(p));
    match kind {
        Kind::Sensor | Kind::Calibrated => {
            let s = level.block(p);
            if !blocks::can_activate(s) {
                return;
            }
            let frequency = vibration::frequency(info.event);
            if let Some(be) = level.blocks.sculk.map.get_mut(&p) {
                be.last_frequency = frequency;
            }
            let power = vibration::redstone_strength_for_distance(distance, kind.radius());
            activate(level, p, s, power, frequency, info.source);
        }
        Kind::Shrieker => {
            // `tryShriek(tryGetPlayer(projectileOwner ?: sourceEntity))`.
            if let Some(player) = info.source.and_then(|s| s.player) {
                level.out.shrieks.push((p, player));
            }
        }
        Kind::Catalyst => {}
    }
}

fn to_entity_pos(p: BlockPos) -> kiln_entity::math::BlockPos {
    kiln_entity::math::BlockPos::new(p.x, p.y, p.z)
}

/// `NoteBlock.getPitchFromNote` of `SculkSensorBlock.RESONANCE_PITCH_BEND`'s tones.
fn resonance_pitch(frequency: i32) -> f32 {
    const TONES: [i32; 16] = [0, 0, 2, 4, 6, 7, 9, 10, 12, 14, 15, 18, 19, 21, 22, 24];
    2f32.powf((TONES[frequency.clamp(0, 15) as usize] - 12) as f32 / 12.0)
}

/// `SculkSensorBlock.activate`: active with the power of the distance, then resonance, the
/// clicking game event and sound.
fn activate(level: &mut RegionLevel, p: BlockPos, s: u16, power: i32, frequency: i32, source: Option<EventSource>) {
    blocks::activate(level, p, s, power);
    // `tryResonateVibration`.
    for dir in Direction::ALL {
        let rp = p.relative(dir);
        let rs = level.block(rp);
        if vibration::resonator(rs) {
            post(level, vibration::resonance_event(frequency), center(rp), Context { source, affected_state: Some(rs) });
            level.effect(kiln_blocks::Effect::Sound { pos: rp, sound: "minecraft:block.amethyst_block.resonate", volume: 1.0, pitch: resonance_pitch(frequency) });
        }
    }
    post(level, "minecraft:sculk_sensor_tendrils_clicking", center(p), Context { source, affected_state: None });
    if !kiln_blocks::state::get_bool(s, "waterlogged") {
        use kiln_javamath::random::RandomSource;
        let pitch = level.random().next_float() * 0.2 + 0.8;
        level.effect(kiln_blocks::Effect::Sound { pos: p, sound: "minecraft:block.sculk_sensor.clicking", volume: 1.0, pitch });
    }
}

/// `sendParticles(new VibrationParticleOption(source, ticks), at, 1, 0, 0, 0, 0)`.
pub(crate) fn send_vibration_particle(level: &mut RegionLevel, at: Vec3, dest: world_fx::PositionSource, ticks: i32) {
    let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:vibration") else { return };
    let pkt = world_fx::level_particles(&world_fx::LevelParticles {
        particle: world_fx::Particle {
            kind,
            options: world_fx::ParticleOptions::Vibration { destination: dest, arrival_ticks: ticks },
        },
        override_limiter: false,
        always_show: false,
        pos: [at.x, at.y, at.z],
        offset: [0.0; 3],
        max_speed: [0.0; 3],
        count: 1,
        randomization: world_fx::ParticleRandomization::Default,
    });
    level.out.packets.push(([at.x, at.y, at.z], 32.0, pkt));
}

/// `getAnalogOutputSignal` of a sculk sensor: the last frequency while active.
pub(crate) fn analog(level: &RegionLevel, pos: BlockPos, s: u16) -> Option<i32> {
    let be = level.blocks.sculk.map.get(&pos)?;
    matches!(be.kind, Kind::Sensor | Kind::Calibrated)
        .then(|| if kiln_blocks::state::get(s, "sculk_sensor_phase") == Some("active") { be.last_frequency } else { 0 })
}

/// `Block.stepOn` for players standing on a sensor or shrieker (`applyEffectsFromBlocks`,
/// once a player tick).
pub(crate) fn players_step_on(level: &mut RegionLevel, players: &[&mut crate::Player]) {
    if level.blocks.sculk.map.is_empty() {
        return;
    }
    for p in players.iter().filter(|p| p.on_ground && !p.dead && !p.disconnected && p.game_mode != 3) {
        // `getOnPosLegacy`: 0.2 below the feet.
        let on = BlockPos::new(p.pos[0].floor() as i32, (p.pos[1] - 0.2).floor() as i32, p.pos[2].floor() as i32);
        if level.blocks.sculk.map.get(&on).is_some_and(|b| b.kind != Kind::Catalyst) {
            step_on(level, on, crate::blocks::player_source(p));
        }
    }
}

/// What shriekers left for the region this phase: the ones players set off shriek (with the
/// players' warning levels), the ones whose shriek ended answer with darkness, a reply sound
/// or a warden.
pub(crate) fn requests(level: &mut RegionLevel, players: &mut [&mut crate::Player], entities: &crate::entities::Entities, spawns: &mut Vec<crate::entities::Spawn>) {
    if level.out.shrieks.is_empty() && level.out.responds.is_empty() {
        return;
    }
    let wardens: Vec<[f64; 3]> = entities.list.iter().filter(|e| !e.removed && e.kind.name == "minecraft:warden").map(|e| e.pos).collect();
    for (pos, player) in std::mem::take(&mut level.out.shrieks) {
        shrieker::try_shriek(level, players, &wardens, pos, player);
    }
    for (pos, warning) in std::mem::take(&mut level.out.responds) {
        let bodies = level.bodies;
        let occupied = |at: [f64; 3]| {
            let (min, max) = ([at[0] - 0.45, at[1], at[2] - 0.45], [at[0] + 0.45, at[1] + 2.9, at[2] + 0.45]);
            bodies.iter().any(|b| b.intersects(min, max))
        };
        match shrieker::answer(level, pos, warning, &occupied) {
            Some(shrieker::Answer::Warden(at)) => shrieker::summon_warden(level, at, spawns),
            Some(shrieker::Answer::Sound { at, sound }) => shrieker::reply(level, at, sound),
            None => {}
        }
        shrieker::darkness_around(players, [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5], 40.0);
    }
}

/// Keeps the region's listeners in step with a block change at `pos`.
pub(crate) fn block_set(level: &mut RegionLevel, pos: BlockPos) {
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let now = level.cells.chunk(chunk_of(pos)).and_then(|c| c.block_entity(x, pos.y, z));
    level.blocks.sculk.block_changed(pos, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resonance_pitches() {
        assert_eq!(resonance_pitch(1), 0.5);
        assert_eq!(resonance_pitch(8), 1.0);
        assert_eq!(resonance_pitch(15), 2.0);
    }
}
