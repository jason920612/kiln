//! Chunk columns: sections, light, heightmaps, block entities and a cached encoded packet body.

use crate::block_entity::BlockEntity;
use crate::section::{Section, bits_for, pack};
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::block_props::{block_entity_type, has_block_entity, keeps_block_entity};
use kiln_data::blocks_types::{block_of, is_air};
use kiln_proto::WriteExt;
use std::collections::BTreeMap;

static EPOCHS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_epoch() -> u64 {
    EPOCHS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Light of one section.
#[derive(Clone)]
pub enum Light {
    Zero,
    Full,
    Nibbles(Box<[u8; 2048]>),
}

impl Light {
    /// The 2048-byte nibble array (index `i` in byte `i / 2`, low nibble first).
    pub fn to_bytes(&self) -> Box<[u8; 2048]> {
        match self {
            Light::Zero => Box::new([0; 2048]),
            Light::Full => Box::new([0xff; 2048]),
            Light::Nibbles(n) => n.clone(),
        }
    }

    fn get(&self, i: usize) -> u8 {
        match self {
            Light::Zero => 0,
            Light::Full => 15,
            Light::Nibbles(n) => (n[i >> 1] >> ((i & 1) * 4)) & 0xf,
        }
    }

    fn set(&mut self, i: usize, v: u8) {
        if self.get(i) == v {
            return;
        }
        if !matches!(self, Light::Nibbles(_)) {
            let fill = if matches!(self, Light::Full) { 0xff } else { 0 };
            *self = Light::Nibbles(Box::new([fill; 2048]));
        }
        let Light::Nibbles(n) = self else { unreachable!() };
        let shift = (i & 1) * 4;
        n[i >> 1] = (n[i >> 1] & !(0xf << shift)) | (v << shift);
    }
}

pub struct Chunk {
    pub sections: Vec<Section>,
    min_y: i32,
    /// Sky light for sections -1..=len (index 0 is the section below the world).
    sky: Vec<Light>,
    /// Block light, same indexing as `sky`.
    block: Vec<Light>,
    /// Per column: highest non-air block + 1, relative to `min_y` (0 = empty column).
    surface: Box<[u16; 256]>,
    version: u32,
    saved_version: u32,
    /// A number no other chunk (or earlier state of this one) ever had: drawn when the chunk is
    /// made and again whenever a block changes, so a result computed from this chunk's blocks
    /// stays valid exactly while the number is the same ([`Chunk::block_epoch`]).
    epoch: u64,
    /// Block changes since the chunk was loaded or generated (light changes not counted).
    edits: u32,
    /// Whether stored light is complete: generated here, or loaded with light. A chunk
    /// saved without light gets its sky light from the heightmap and no block light.
    light_trusted: bool,
    cached: Option<(u32, Bytes)>,
    /// The whole Level Chunk With Light packet for the cached body, once one was sent.
    cached_packet: Option<(u32, Bytes)>,
    /// Light sections changed since the last Update Light, per layer.
    light_dirty: [u64; 2],
    /// By [`Chunk::block_index`].
    block_entities: BTreeMap<u32, BlockEntity>,
    /// Scheduled ticks in their saved form: read from the save, taken by the simulation when
    /// the chunk loads, and put back before the chunk is saved.
    pub saved_ticks: Option<Box<SavedTicks>>,
    /// Structure starts and references (chunk NBT `structures`) of a generated chunk; a
    /// loaded chunk keeps its own among the preserved save fields.
    pub structures: Option<Box<kiln_proto::nbt::Tag>>,
    /// Updates owed since generation, handed to the simulation when the chunk becomes full.
    pending: Option<Box<PendingUpdates>>,
    /// Entities generation placed (end crystals, ...), in saved form, for the simulation to
    /// add when the chunk loads.
    pub generated_entities: Vec<kiln_proto::nbt::Tag>,
    /// A chunk generation made that still wants its animals (`NaturalSpawner.spawnMobsForChunkGeneration`); the
    /// simulation makes them when it takes the chunk in.
    pub original_mobs: bool,
    /// Points of interest once the simulation took the chunk in ([`Chunk::init_pois`]); block
    /// changes keep them up to date from then on.
    pub pois: Option<Box<crate::poi::ChunkPois>>,
    /// Ticks players spent near this chunk (`ChunkAccess.inhabitedTime`, saved as
    /// `InhabitedTime`); regional difficulty grows with it.
    inhabited_time: i64,
    /// What `inhabited_time` was when the chunk was loaded or last saved.
    inhabited_saved: i64,
}

/// A chunk is written again for its inhabited time alone once it grew this much (a minute of
/// a player nearby): vanilla's chunks are saved when anything changes in them, which a chunk
/// players stay near does all the time; Kiln writes the unchanged ones less eagerly.
pub const INHABITED_SAVE_TICKS: i64 = 1200;

/// Work a freshly generated chunk leaves for the simulation, to run once the chunk (and its
/// neighbours) tick: vanilla's `ProtoChunk.postProcessing` positions (a fluid there starts
/// flowing — `FluidState.tick` — and other blocks update their shape from their neighbours,
/// `Block.updateFromNeighbourShapes`) and the block and fluid ticks scheduled during
/// generation (e.g. springs, lakes). Positions are absolute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingUpdates {
    pub post_process: Vec<[i32; 3]>,
    /// Block ticks: position, block name, delay in ticks.
    pub block_ticks: Vec<([i32; 3], &'static str, i32)>,
    /// Fluid ticks: position, fluid name, delay in ticks.
    pub fluid_ticks: Vec<([i32; 3], &'static str, i32)>,
}

impl PendingUpdates {
    pub fn is_empty(&self) -> bool {
        self.post_process.is_empty() && self.block_ticks.is_empty() && self.fluid_ticks.is_empty()
    }
}

/// A chunk's `block_ticks` and `fluid_ticks` lists as chunk NBT stores them.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedTicks {
    pub block: kiln_proto::nbt::Tag,
    pub fluid: kiln_proto::nbt::Tag,
}

impl Chunk {
    /// A chunk whose sky light is derived from its blocks and which has no block light.
    pub fn new(sections: Vec<Section>, min_y: i32) -> Self {
        Self::with_light(sections, min_y, None, None)
    }

    /// A copy of what saving writes (blocks, light, block entities, ticks, structures, points of
    /// interest, inhabited time), for encoding on another thread while the chunk plays on.
    /// Packet caches are left behind.
    pub fn snapshot(&self) -> Chunk {
        Chunk {
            sections: self.sections.clone(),
            min_y: self.min_y,
            sky: self.sky.clone(),
            block: self.block.clone(),
            surface: self.surface.clone(),
            version: self.version,
            saved_version: self.saved_version,
            epoch: next_epoch(),
            edits: self.edits,
            light_trusted: self.light_trusted,
            cached: None,
            cached_packet: None,
            light_dirty: [0, 0],
            block_entities: self.block_entities.clone(),
            saved_ticks: self.saved_ticks.clone(),
            structures: self.structures.clone(),
            pending: self.pending.clone(),
            generated_entities: self.generated_entities.clone(),
            original_mobs: self.original_mobs,
            pois: self.pois.clone(),
            inhabited_time: self.inhabited_time,
            inhabited_saved: self.inhabited_saved,
        }
    }

    /// A chunk with stored light (e.g. from a world save); missing layers are derived.
    pub fn with_light(sections: Vec<Section>, min_y: i32, sky: Option<Vec<Light>>, block: Option<Vec<Light>>) -> Self {
        let n = sections.len();
        let mut c = Self {
            sections,
            min_y,
            sky: vec![Light::Zero; n + 2],
            block: block.filter(|b| b.len() == n + 2).unwrap_or_else(|| vec![Light::Zero; n + 2]),
            surface: Box::new([0; 256]),
            version: 0,
            saved_version: 0,
            epoch: next_epoch(),
            edits: 0,
            light_trusted: true,
            cached: None,
            cached_packet: None,
            light_dirty: [0, 0],
            block_entities: BTreeMap::new(),
            saved_ticks: None,
            structures: None,
            pending: None,
            generated_entities: Vec::new(),
            original_mobs: false,
            pois: None,
            inhabited_time: 0,
            inhabited_saved: 0,
        };
        for x in 0..16 {
            for z in 0..16 {
                c.surface[(z << 4) | x] = c.column_top(x, z);
            }
        }
        match sky.filter(|s| s.len() == n + 2) {
            Some(s) => c.sky = s,
            None => {
                for li in 0..c.sky.len() {
                    c.sky[li] = c.section_sky(li);
                }
            }
        }
        c
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    /// Updates owed since generation ([`PendingUpdates`]), if any.
    pub fn pending_updates(&self) -> Option<&PendingUpdates> {
        self.pending.as_deref()
    }

    pub fn set_pending_updates(&mut self, updates: PendingUpdates) {
        self.pending = (!updates.is_empty()).then(|| Box::new(updates));
    }

    /// Takes the owed updates, e.g. when the chunk starts ticking.
    pub fn take_pending_updates(&mut self) -> Option<PendingUpdates> {
        self.pending.take().map(|p| *p)
    }

    /// Sky light per section, index 0 being the section below the world.
    pub fn sky_light(&self) -> &[Light] {
        &self.sky
    }

    /// Block light, indexed like [`Chunk::sky_light`].
    pub fn block_light(&self) -> &[Light] {
        &self.block
    }

    /// Whether blocks changed since the chunk was loaded or created.
    pub fn modified(&self) -> bool {
        self.version != 0
    }

    pub fn light_trusted(&self) -> bool {
        self.light_trusted
    }

    pub fn set_light_trusted(&mut self, trusted: bool) {
        self.light_trusted = trusted;
    }

    /// Whether any block changed since the chunk was loaded or generated.
    pub fn edited(&self) -> bool {
        self.edits != 0
    }

    /// Biomes changed in place (`/fillbiome`): the chunk saves and re-encodes.
    pub fn mark_biomes_changed(&mut self) {
        self.version += 1;
    }

    pub fn needs_save(&self) -> bool {
        self.version != self.saved_version || self.inhabited_time - self.inhabited_saved >= INHABITED_SAVE_TICKS
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
        self.inhabited_saved = self.inhabited_time;
    }

    /// `ChunkAccess.getInhabitedTime`: ticks players spent near the chunk.
    pub fn inhabited_time(&self) -> i64 {
        self.inhabited_time
    }

    /// `LevelChunk.incrementInhabitedTime` (one tick of a player being near).
    pub fn increment_inhabited_time(&mut self) {
        self.inhabited_time += 1;
    }

    /// A loaded chunk's saved `InhabitedTime`.
    pub fn set_inhabited_time(&mut self, ticks: i64) {
        self.inhabited_time = ticks;
        self.inhabited_saved = ticks;
    }

    /// Marks a newly generated chunk as unsaved.
    pub fn mark_new(&mut self) {
        self.saved_version = u32::MAX;
    }

    /// Generated here and never saved (not read from a save).
    pub fn is_new(&self) -> bool {
        self.saved_version == u32::MAX
    }

    /// Sky light of light section `li` (0 = below the world) from the surface heights.
    fn section_sky(&self, li: usize) -> Light {
        let base = (li as i32 - 1) * 16;
        let (lo, hi) = self.surface.iter().fold((u16::MAX, 0), |(lo, hi), &t| (lo.min(t), hi.max(t)));
        if hi as i32 <= base {
            return Light::Full;
        }
        if lo as i32 >= base + 16 {
            return Light::Zero;
        }
        let mut n = Box::new([0u8; 2048]);
        for i in 0..4096 {
            let (ly, col) = ((i >> 8) as i32, i & 0xff);
            if base + ly >= self.surface[col] as i32 {
                n[i >> 1] |= 15 << ((i & 1) * 4);
            }
        }
        Light::Nibbles(n)
    }

    /// The y above the column's highest non-air block (the chunk's bottom if all air): what
    /// `column_height(x, z, |s| !is_air(s))` finds, from the heights kept up to date.
    pub fn surface_y(&self, x: usize, z: usize) -> i32 {
        self.min_y + self.surface[(z << 4) | x] as i32
    }

    fn column_top(&self, x: usize, z: usize) -> u16 {
        for rel in (0..self.height()).rev() {
            let (s, ly) = ((rel >> 4) as usize, (rel & 15) as usize);
            if !self.sections[s].is_empty() && !is_air(self.sections[s].get(x, ly, z)) {
                return rel as u16 + 1;
            }
        }
        0
    }

    pub fn height(&self) -> i32 {
        self.sections.len() as i32 * 16
    }

    fn section_of(&self, y: i32) -> Option<(usize, usize)> {
        let rel = y - self.min_y;
        (rel >= 0 && rel < self.height()).then(|| ((rel >> 4) as usize, (rel & 15) as usize))
    }

    /// Identifies the chunk's block states: equal numbers read at different times mean no block
    /// changed in between (and it is the same chunk, not one loaded in its place).
    pub fn block_epoch(&self) -> u64 {
        self.epoch
    }

    /// Whether any block with a fluid may lie in `y0..=y1` (absolute): false means none does.
    pub fn may_have_fluid(&self, y0: i32, y1: i32) -> bool {
        let (lo, hi) = (y0.max(self.min_y), y1.min(self.min_y + self.height() - 1));
        if lo > hi {
            return false;
        }
        (((lo - self.min_y) >> 4)..=((hi - self.min_y) >> 4)).any(|s| self.sections[s as usize].has_fluids())
    }

    /// Reads the block states of `x0..=x1` by `y0..=y1` (absolute) by `z0..=z1` (x and z within
    /// the chunk, 0..16) into `out`: the state at `(x, y, z)` goes to
    /// `origin + (y - y0) * layer + (z - z0) * row + (x - x0)`. Outside the world it is void air.
    #[allow(clippy::too_many_arguments)]
    pub fn read_box(
        &self,
        (x0, x1): (usize, usize),
        (y0, y1): (i32, i32),
        (z0, z1): (usize, usize),
        out: &mut [u16],
        origin: usize,
        row: usize,
        layer: usize,
    ) {
        let len = x1 - x0 + 1;
        for y in y0..=y1 {
            let base = origin + (y - y0) as usize * layer;
            match self.section_of(y) {
                Some((s, ly)) => {
                    let blocks = &self.sections[s].blocks;
                    for z in z0..=z1 {
                        let at = base + (z - z0) * row;
                        blocks.read_row(crate::section::block_index(x0, ly, z), &mut out[at..at + len]);
                    }
                }
                None => {
                    for z in z0..=z1 {
                        let at = base + (z - z0) * row;
                        out[at..at + len].fill(kiln_data::blocks::default_state::VOID_AIR);
                    }
                }
            }
        }
    }

    /// `x`, `z` are within the chunk (0..16); `y` is absolute.
    pub fn get(&self, x: usize, y: i32, z: usize) -> u16 {
        match self.section_of(y) {
            Some((s, ly)) => self.sections[s].get(x, ly, z),
            None => kiln_data::blocks::default_state::VOID_AIR,
        }
    }

    /// Whether `pred` holds for any block of the box `x0..=x1` by `z0..=z1` (within the chunk,
    /// 0..16) and `y0..=y1` (absolute); outside the world blocks read as void air. Sections whose
    /// palette has nothing `pred` accepts are skipped (`PalettedContainer.maybeHas`).
    pub fn any_block_in(&self, (x0, x1): (usize, usize), (y0, y1): (i32, i32), (z0, z1): (usize, usize), pred: &dyn Fn(u16) -> bool) -> bool {
        if (y0 < self.min_y || y1 >= self.min_y + self.height()) && pred(kiln_data::blocks::default_state::VOID_AIR) {
            return true;
        }
        let (lo, hi) = (y0.max(self.min_y), y1.min(self.min_y + self.height() - 1));
        if lo > hi {
            return false;
        }
        for s in ((lo - self.min_y) >> 4)..=((hi - self.min_y) >> 4) {
            let section = &self.sections[s as usize];
            if !section.blocks.maybe_has(pred) {
                continue;
            }
            let base = self.min_y + s * 16;
            for y in lo.max(base)..=hi.min(base + 15) {
                let ly = (y - base) as usize;
                for z in z0..=z1 {
                    for x in x0..=x1 {
                        if pred(section.get(x, ly, z)) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Returns the previous state, or `None` if `y` is outside the world.
    /// Light is not updated here; the world's light engine does that.
    ///
    /// Block entities follow vanilla's `LevelChunk.setBlockState`: replacing the block with a
    /// different one drops its block entity (unless the new block keeps it), and a block that
    /// needs a block entity gets a default one if it has none of the right type.
    pub fn set(&mut self, x: usize, y: i32, z: usize, state: u16) -> Option<u16> {
        let (s, ly) = self.section_of(y)?;
        let old = self.sections[s].set(x, ly, z, state);
        if old != state {
            self.epoch = next_epoch();
            self.version += 1;
            self.edits += 1;
            if is_air(old) != is_air(state) {
                self.surface[(z << 4) | x] = self.column_top(x, z);
            }
            let key = self.block_index(x, y, z);
            let same_block = block_of(old).first == block_of(state).first;
            if !same_block && has_block_entity(old) && !keeps_block_entity(state, old) {
                self.block_entities.remove(&key);
            }
            if let Some(kind) = block_entity_type(state)
                && self.block_entities.get(&key).is_none_or(|be| be.kind != kind)
            {
                self.block_entities.insert(key, BlockEntity::new(kind));
            }
            // `ServerLevel.updatePOIOnBlockStateChange`.
            if let Some(p) = self.pois.as_mut() {
                let (a, b) = (crate::poi::type_of(old), crate::poi::type_of(state));
                if a != b {
                    let pos = [p.cx * 16 + x as i32, y, p.cz * 16 + z as i32];
                    if a.is_some() {
                        p.remove(pos);
                    }
                    if let Some(k) = b {
                        p.add(pos, k);
                    }
                }
            }
        }
        Some(old)
    }

    /// Takes the chunk's points of interest in (`PoiManager.checkConsistencyWithBlocks` for
    /// each section): `stored` as saved, sections saved invalid rescanned from the blocks,
    /// sections with no saved data scanned when their palette holds a point of interest.
    pub fn init_pois(&mut self, cx: i32, cz: i32, stored: Option<crate::poi::ChunkPois>) {
        let mut p = stored.unwrap_or_default();
        p.cx = cx;
        p.cz = cz;
        let min_section = self.min_y >> 4;
        for (i, section) in self.sections.iter().enumerate() {
            let sy = min_section + i as i32;
            let known = p.sections.get(&sy).map(|s| s.valid);
            if known == Some(true) {
                continue;
            }
            if known.is_none() && !section.blocks.maybe_has(crate::poi::has_poi) {
                continue;
            }
            // A saved invalid section keeps its records' tickets where they still apply.
            let old = p.sections.remove(&sy).map(|s| s.records).unwrap_or_default();
            let mut fresh = crate::poi::Section { valid: true, records: Default::default() };
            for ly in 0..16 {
                for lz in 0..16 {
                    for lx in 0..16 {
                        let s = section.get(lx, ly, lz);
                        let Some(kind) = crate::poi::type_of(s) else { continue };
                        let pos = [cx * 16 + lx as i32, sy * 16 + ly as i32, cz * 16 + lz as i32];
                        let key = ((lx << 8) | (lz << 4) | ly) as u16;
                        let r = old.get(&key).filter(|r| r.kind == kind).cloned().unwrap_or_else(|| crate::poi::Record::new(pos, kind));
                        fresh.records.insert(key, r);
                    }
                }
            }
            p.sections.insert(sy, fresh);
            p.dirty = true;
        }
        self.pois = Some(Box::new(p));
    }

    fn block_index(&self, x: usize, y: i32, z: usize) -> u32 {
        (((y - self.min_y) as u32) << 8) | ((z as u32) << 4) | x as u32
    }

    fn block_at_index(&self, i: u32) -> (usize, i32, usize) {
        ((i & 15) as usize, (i >> 8) as i32 + self.min_y, ((i >> 4) & 15) as usize)
    }

    pub fn block_entity(&self, x: usize, y: i32, z: usize) -> Option<&BlockEntity> {
        self.section_of(y)?;
        self.block_entities.get(&self.block_index(x, y, z))
    }

    /// Sets or replaces the block entity at a position inside the world; the caller keeps it
    /// consistent with the block there.
    pub fn set_block_entity(&mut self, x: usize, y: i32, z: usize, be: BlockEntity) {
        if self.section_of(y).is_some() {
            self.block_entities.insert(self.block_index(x, y, z), be);
            self.version += 1;
        }
    }

    /// Adds a block entity read with the chunk; unlike [`Chunk::set_block_entity`] this is
    /// not a change to save.
    pub fn load_block_entity(&mut self, x: usize, y: i32, z: usize, be: BlockEntity) {
        if self.section_of(y).is_some() {
            self.block_entities.insert(self.block_index(x, y, z), be);
            self.cached = None;
            self.cached_packet = None;
        }
    }

    pub fn remove_block_entity(&mut self, x: usize, y: i32, z: usize) -> Option<BlockEntity> {
        self.section_of(y)?;
        let be = self.block_entities.remove(&self.block_index(x, y, z));
        if be.is_some() {
            self.version += 1;
        }
        be
    }

    /// Block entities with their chunk-local x, absolute y and local z.
    pub fn block_entities(&self) -> impl Iterator<Item = ((usize, i32, usize), &BlockEntity)> {
        self.block_entities.iter().map(|(&i, be)| (self.block_at_index(i), be))
    }

    /// Height of the first block above the column's topmost block matching `pred` (the value a
    /// vanilla heightmap stores, as an absolute y; `min_y` for an empty column).
    pub fn column_height(&self, x: usize, z: usize, pred: impl Fn(u16) -> bool) -> i32 {
        // Top section down, skipping the all-air ones whole.
        let top = ((self.height() + 15) >> 4) as usize;
        for s in (0..top.min(self.sections.len())).rev() {
            if self.sections[s].is_empty() {
                continue;
            }
            for ly in (0..16).rev() {
                let rel = (s as i32) << 4 | ly;
                if rel < self.height() && pred(self.sections[s].get(x, ly as usize, z)) {
                    return self.min_y + rel + 1;
                }
            }
        }
        self.min_y
    }

    /// Whether the block at local `x`, `z` and absolute `y` ticks randomly.
    pub fn ticks_randomly_at(&self, x: usize, y: i32, z: usize) -> bool {
        let rel = y - self.min_y;
        rel >= 0 && self.sections.get((rel >> 4) as usize).is_some_and(|s| s.ticks_randomly_at(x, (rel & 15) as usize, z))
    }

    /// Whether light is stored at absolute `y`: the chunk's height plus one section each side.
    pub fn in_light_range(&self, y: i32) -> bool {
        self.light_section(y).is_some()
    }

    /// Light section index (0 = below the world) of absolute `y`, if stored.
    fn light_section(&self, y: i32) -> Option<usize> {
        let li = ((y - self.min_y) >> 4) + 1;
        (li >= 0 && (li as usize) < self.sky.len()).then_some(li as usize)
    }

    /// Light level at local `x`, `z` and absolute `y` (sections -1..=n are stored).
    pub fn light(&self, layer: LightLayer, x: usize, y: i32, z: usize) -> u8 {
        let Some(li) = self.light_section(y) else {
            return if layer == LightLayer::Sky && y >= self.min_y { 15 } else { 0 };
        };
        let l = match layer {
            LightLayer::Sky => &self.sky[li],
            LightLayer::Block => &self.block[li],
        };
        l.get((((y - self.min_y) & 15) as usize) << 8 | (z << 4) | x)
    }

    /// Sets a light level; returns whether it changed.
    pub fn set_light(&mut self, layer: LightLayer, x: usize, y: i32, z: usize, v: u8) -> bool {
        let Some(li) = self.light_section(y) else { return false };
        let i = (((y - self.min_y) & 15) as usize) << 8 | (z << 4) | x;
        let l = match layer {
            LightLayer::Sky => &mut self.sky[li],
            LightLayer::Block => &mut self.block[li],
        };
        if l.get(i) == v {
            return false;
        }
        l.set(i, v);
        self.light_dirty[layer as usize] |= 1 << li;
        self.version += 1;
        true
    }

    /// Light sections changed since the last call, as bit masks (sky, block).
    pub fn take_light_dirty(&mut self) -> (u64, u64) {
        let d = self.light_dirty;
        self.light_dirty = [0, 0];
        (d[0], d[1])
    }

    /// Encodes an Update Light payload for the given section masks (after the chunk coords).
    pub fn encode_light_update(&self, sky: u64, block: u64, b: &mut BytesMut) {
        put_light_data(b, &self.sky, sky, &self.block, block);
    }

    /// Forgets the encoded packet (it is made again when next needed).
    pub fn drop_packet_cache(&mut self) {
        self.cached = None;
        self.cached_packet = None;
    }

    /// Chunk Data body after the coordinates; re-encoded only when the chunk changed.
    pub fn packet_body(&mut self, biome_count: usize) -> Bytes {
        if let Some((v, body)) = &self.cached {
            if *v == self.version {
                return body.clone();
            }
        }
        let body = self.encode(biome_count);
        self.cached = Some((self.version, body.clone()));
        body
    }

    /// The Level Chunk With Light packet for this chunk at `(x, z)`, encoded once per version
    /// and shared by every player it goes to.
    pub fn packet(&mut self, x: i32, z: i32, biome_count: usize) -> Bytes {
        if let Some((v, p)) = &self.cached_packet
            && *v == self.version
        {
            return p.clone();
        }
        // The body is kept inside the packet only (one copy of the chunk's encoding).
        let body = match self.cached.take() {
            Some((v, body)) if v == self.version => body,
            _ => self.encode(biome_count),
        };
        let p = kiln_proto::packets::level_chunk_with_light(x, z, &body);
        self.cached_packet = Some((self.version, p.clone()));
        p
    }

    fn encode(&self, biome_count: usize) -> Bytes {
        let mut b = BytesMut::with_capacity(16 * 1024);

        let bits = bits_for(self.height() as usize + 1);
        let heights = pack(&self.surface.map(|h| h as u64), bits);
        const WORLD_SURFACE: i32 = 1;
        const MOTION_BLOCKING: i32 = 4;
        const MOTION_BLOCKING_NO_LEAVES: i32 = 5;
        b.put_varint(3);
        for kind in [WORLD_SURFACE, MOTION_BLOCKING, MOTION_BLOCKING_NO_LEAVES] {
            b.put_varint(kind);
            b.put_varint(heights.len() as i32);
            for l in &heights {
                b.put_u64(*l);
            }
        }

        let mut data = BytesMut::with_capacity(8 * 1024);
        for s in &self.sections {
            s.encode(&mut data, biome_count);
        }
        b.put_varint(data.len() as i32);
        b.put_slice(&data);

        b.put_varint(self.block_entities.len() as i32);
        for (&i, be) in &self.block_entities {
            let (x, y, z) = self.block_at_index(i);
            b.put_u8(((x << 4) | z) as u8);
            b.put_i16(y as i16);
            b.put_varint(be.kind as i32);
            match be.update_tag(self.get(x, y, z)) {
                Some(tag) => tag.write_network(&mut b),
                None => b.put_u8(0), // an absent tag (TAG_End)
            }
        }

        let all = (1u64 << self.sky.len()) - 1;
        put_light_data(&mut b, &self.sky, all, &self.block, all);
        b.freeze()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightLayer {
    Sky = 0,
    Block = 1,
}

/// Light Data for the sections selected by `sky_sel`/`block_sel`: all-zero sections go in
/// the empty masks, the rest are sent as arrays.
fn put_light_data(b: &mut BytesMut, sky: &[Light], sky_sel: u64, block: &[Light], block_sel: u64) {
    let masks = |layer: &[Light], sel: u64| {
        let (mut data, mut empty) = (0u64, 0u64);
        for (i, l) in layer.iter().enumerate().filter(|(i, _)| sel & (1 << i) != 0) {
            match l {
                Light::Zero => empty |= 1 << i,
                Light::Nibbles(n) if n.iter().all(|&v| v == 0) => empty |= 1 << i,
                _ => data |= 1 << i,
            }
        }
        (data, empty)
    };
    let (sky_mask, empty_sky) = masks(sky, sky_sel);
    let (block_mask, empty_block) = masks(block, block_sel);
    b.put_bitset(&[sky_mask]);
    b.put_bitset(&[block_mask]);
    b.put_bitset(&[empty_sky]);
    b.put_bitset(&[empty_block]);
    for (layer, mask) in [(sky, sky_mask), (block, block_mask)] {
        b.put_varint(mask.count_ones() as i32);
        for (i, l) in layer.iter().enumerate() {
            if mask & (1 << i) == 0 {
                continue;
            }
            b.put_varint(2048);
            match l {
                Light::Full => b.put_bytes(0xff, 2048),
                Light::Nibbles(n) => b.put_slice(&n[..]),
                Light::Zero => unreachable!("zero sections go in the empty mask"),
            }
        }
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;

    /// The kept heights say what scanning the column finds, through any edits.
    #[test]
    fn surface_y_matches_a_column_scan() {
        let d = kiln_data::blocks::default_state::STONE;
        let mut c = Chunk::new((0..4).map(|_| Section::filled(kiln_data::blocks::default_state::AIR, 0)).collect(), -64);
        let mut seed = 99u64;
        for _ in 0..5000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let (x, z, y) = ((seed >> 20) as usize & 15, (seed >> 24) as usize & 15, -64 + ((seed >> 28) % 64) as i32);
            let state = if (seed >> 40) % 3 == 0 { kiln_data::blocks::default_state::AIR } else { d };
            c.set(x, y, z, state);
        }
        for x in 0..16 {
            for z in 0..16 {
                assert_eq!(c.surface_y(x, z), c.column_height(x, z, |s| !is_air(s)), "column {x},{z}");
            }
        }
    }
}

#[cfg(test)]
mod any_block_tests {
    use super::*;

    fn chunk() -> Chunk {
        Chunk::new((0..3).map(|_| Section::filled(0, 0)).collect(), -64)
    }

    #[test]
    fn any_block_in_finds_blocks_by_box_and_skips_empty_sections() {
        let mut c = chunk();
        let fire = kiln_data::blocks::default_state::SOUL_FIRE;
        let is_fire = |s: u16| s == fire;
        assert!(!c.any_block_in((0, 15), (-64, -17), (0, 15), &is_fire));
        c.set(3, -50, 5, fire);
        assert!(c.any_block_in((0, 15), (-64, -17), (0, 15), &is_fire));
        assert!(c.any_block_in((3, 3), (-50, -50), (5, 5), &is_fire));
        for (x, y, z) in [(2, -50, 5), (3, -51, 5), (3, -50, 6), (4, -50, 5), (3, -49, 5)] {
            assert!(!c.any_block_in((x, x), (y, y), (z, z), &is_fire), "{x} {y} {z}");
        }
        // Another section, a box across two: found in either part.
        assert!(c.any_block_in((0, 15), (-70, -40), (0, 15), &is_fire));
        assert!(!c.any_block_in((0, 15), (-40, -17), (0, 15), &is_fire));
        // Outside the world blocks are void air.
        let void = kiln_data::blocks::default_state::VOID_AIR;
        assert!(c.any_block_in((0, 0), (-80, -80), (0, 0), &|s| s == void));
        assert!(!c.any_block_in((0, 0), (-80, -80), (0, 0), &is_fire));
        assert!(c.any_block_in((0, 15), (-100, 100), (0, 15), &is_fire));
    }
}
