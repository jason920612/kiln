//! Level state the world commands change: each level's world border (`WorldBorder`, saved as
//! `world_border.dat` in the level's folder), the server's tick rate (`ServerTickRateManager`),
//! each level's forced chunks (`chunk_tickets.dat`) and the random sequences
//! (`random_sequences.dat`).
//!
//! The border moves by whole ticks (`MovingBorderExtent`: the remaining ticks count down while
//! the level ticks normally), players outside it take `outside_border` damage in their base
//! tick, and clients hear about every change through the border packets.

use crate::{DIMENSIONS, Sim};
use bytes::Bytes;
use kiln_command::host::{BorderChange, BorderInfo, TickRateInfo, TickRateAction};
use kiln_command::{Text, tr};
use kiln_javamath::random::{RandomSource, Seed128, XoroshiroRandom};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::world_fx as fx;
use kiln_world::spawn::LoadChunks;
use std::collections::{BTreeMap, BTreeSet};

/// `MinecraftServer.getAbsoluteMaxWorldSize` (the `max-world-size` default).
const ABSOLUTE_MAX_SIZE: i32 = 29_999_984;

// ---- world border -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Extent {
    Static(f64),
    Moving(Moving),
}

/// `WorldBorder.MovingBorderExtent`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Moving {
    from: f64,
    to: f64,
    duration: f64,
    /// Ticks left.
    progress: i64,
    size: f64,
    previous: f64,
}

impl Moving {
    fn new(from: f64, to: f64, ticks: i64) -> Self {
        let mut m = Moving { from, to, duration: ticks as f64, progress: ticks, size: 0.0, previous: 0.0 };
        m.size = m.calculate();
        m.previous = m.size;
        m
    }

    fn calculate(&self) -> f64 {
        let t = (self.duration - self.progress as f64) / self.duration;
        if t < 1.0 { self.from + t * (self.to - self.from) } else { self.to }
    }
}

/// A level's world border (`WorldBorder`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Border {
    center: [f64; 2],
    damage_per_block: f64,
    safe_zone: f64,
    warning_time: i32,
    warning_blocks: i32,
    extent: Extent,
    /// Changed since the last save.
    dirty: bool,
}

impl Default for Border {
    fn default() -> Self {
        let d = BorderInfo::default();
        Border {
            center: d.center,
            damage_per_block: d.damage_per_block,
            safe_zone: d.safe_zone,
            warning_time: d.warning_time,
            warning_blocks: d.warning_blocks,
            extent: Extent::Static(d.size),
            dirty: false,
        }
    }
}

/// What a player's base tick needs of its level's border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BorderBox {
    pub min_x: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_z: f64,
    pub safe_zone: f64,
    pub damage_per_block: f64,
}

impl Default for BorderBox {
    fn default() -> Self {
        Border::default().bounds()
    }
}

impl BorderBox {
    /// `isWithinBounds(x, z)`.
    pub fn contains(&self, x: f64, z: f64) -> bool {
        x >= self.min_x && x < self.max_x && z >= self.min_z && z < self.max_z
    }

    /// `isWithinBounds(AABB)`.
    pub fn contains_box(&self, min: [f64; 2], max: [f64; 2]) -> bool {
        self.contains(min[0], min[1]) && self.contains(max[0] - 1.0e-5f32 as f64, max[1] - 1.0e-5f32 as f64)
    }

    /// `getDistanceToBorder(x, z)`: negative outside.
    pub fn distance(&self, x: f64, z: f64) -> f64 {
        let (dz_min, dz_max) = (z - self.min_z, self.max_z - z);
        let (dx_min, dx_max) = (x - self.min_x, self.max_x - x);
        dx_min.min(dx_max).min(dz_min).min(dz_max)
    }

    /// The border part of `LivingEntity.baseTick` for a player with its feet at `pos` and a
    /// box `half_width` wide each way: the `outside_border` damage, if any.
    pub fn damage(&self, pos: [f64; 3], half_width: f64) -> Option<f32> {
        let min = [pos[0] - half_width, pos[2] - half_width];
        let max = [pos[0] + half_width, pos[2] + half_width];
        if self.contains_box(min, max) {
            return None;
        }
        let d = self.distance(pos[0], pos[2]) + self.safe_zone;
        if d < 0.0 && self.damage_per_block > 0.0 {
            Some(((-d * self.damage_per_block).floor() as i32).max(1) as f32)
        } else {
            None
        }
    }
}

impl Border {
    pub fn size(&self) -> f64 {
        match self.extent {
            Extent::Static(s) => s,
            Extent::Moving(m) => m.size,
        }
    }

    pub fn lerp_time(&self) -> i64 {
        match self.extent {
            Extent::Static(_) => 0,
            Extent::Moving(m) => m.progress,
        }
    }

    pub fn lerp_target(&self) -> f64 {
        match self.extent {
            Extent::Static(s) => s,
            Extent::Moving(m) => m.to,
        }
    }

    pub fn info(&self) -> BorderInfo {
        BorderInfo {
            center: self.center,
            size: self.size(),
            lerp_time: self.lerp_time(),
            damage_per_block: self.damage_per_block,
            safe_zone: self.safe_zone,
            warning_time: self.warning_time,
            warning_blocks: self.warning_blocks,
        }
    }

    /// `getMinX()` and friends at partial tick 0: a moving border's previous size.
    pub fn bounds(&self) -> BorderBox {
        let size = match self.extent {
            Extent::Static(s) => s,
            Extent::Moving(m) => m.previous,
        };
        let max = ABSOLUTE_MAX_SIZE as f64;
        let clamp = |v: f64| v.clamp(-max, max);
        BorderBox {
            min_x: clamp(self.center[0] - size / 2.0),
            min_z: clamp(self.center[1] - size / 2.0),
            max_x: clamp(self.center[0] + size / 2.0),
            max_z: clamp(self.center[1] + size / 2.0),
            safe_zone: self.safe_zone,
            damage_per_block: self.damage_per_block,
        }
    }

    /// `WorldBorder.tick`.
    pub fn tick(&mut self) {
        if let Extent::Moving(mut m) = self.extent {
            m.progress -= 1;
            m.previous = m.size;
            m.size = m.calculate();
            self.dirty = true;
            self.extent = if m.progress <= 0 { Extent::Static(m.to) } else { Extent::Moving(m) };
        }
    }

    /// Applies a `/worldborder` change; returns the packet the level's players get
    /// (`PlayerList`'s border listener), if any.
    pub fn apply(&mut self, change: BorderChange) -> Option<Bytes> {
        self.dirty = true;
        match change {
            BorderChange::Center(x, z) => {
                self.center = [x, z];
                Some(fx::set_border_center(x, z))
            }
            BorderChange::Size(size) => {
                self.extent = Extent::Static(size);
                Some(fx::set_border_size(size))
            }
            BorderChange::Lerp { from, to, ticks } => {
                self.extent = if from == to { Extent::Static(to) } else { Extent::Moving(Moving::new(from, to, ticks)) };
                Some(fx::set_border_lerp_size(self.size(), self.lerp_target(), self.lerp_time()))
            }
            BorderChange::DamagePerBlock(v) => {
                self.damage_per_block = v;
                None
            }
            BorderChange::SafeZone(v) => {
                self.safe_zone = v;
                None
            }
            BorderChange::WarningTime(v) => {
                self.warning_time = v;
                Some(fx::set_border_warning_delay(v))
            }
            BorderChange::WarningBlocks(v) => {
                self.warning_blocks = v;
                Some(fx::set_border_warning_distance(v))
            }
        }
    }

    /// `ClientboundInitializeBorderPacket`.
    pub fn init_packet(&self) -> Bytes {
        fx::initialize_border(&fx::WorldBorder {
            center: self.center,
            size: self.size(),
            target_size: self.lerp_target(),
            lerp_ticks: self.lerp_time(),
            absolute_max_size: ABSOLUTE_MAX_SIZE,
            warning_blocks: self.warning_blocks,
            warning_time: self.warning_time,
        })
    }

    /// `WorldBorder.Settings` as saved.
    pub fn to_nbt(&self) -> Tag {
        Tag::Compound(vec![
            ("center_x".into(), Tag::Double(self.center[0])),
            ("center_z".into(), Tag::Double(self.center[1])),
            ("damage_per_block".into(), Tag::Double(self.damage_per_block)),
            ("safe_zone".into(), Tag::Double(self.safe_zone)),
            ("warning_blocks".into(), Tag::Int(self.warning_blocks)),
            ("warning_time".into(), Tag::Int(self.warning_time)),
            ("size".into(), Tag::Double(self.size())),
            ("lerp_time".into(), Tag::Long(self.lerp_time())),
            ("lerp_target".into(), Tag::Double(self.lerp_target())),
        ])
    }

    /// `applyInitialSettings`: a saved move carries on.
    pub fn from_nbt(tag: &Tag) -> Self {
        let d = BorderInfo::default();
        let f = |k: &str, v: f64| tag.get(k).and_then(Tag::as_f64).unwrap_or(v);
        let i = |k: &str, v: i64| tag.get(k).and_then(Tag::as_i64).unwrap_or(v);
        let size = f("size", d.size);
        let lerp_time = i("lerp_time", 0);
        let target = f("lerp_target", size);
        let extent = if lerp_time > 0 && size != target {
            Extent::Moving(Moving::new(size, target, lerp_time))
        } else if lerp_time > 0 {
            Extent::Static(target)
        } else {
            Extent::Static(size)
        };
        Border {
            center: [f("center_x", 0.0), f("center_z", 0.0)],
            damage_per_block: f("damage_per_block", d.damage_per_block),
            safe_zone: f("safe_zone", d.safe_zone),
            warning_time: i("warning_time", d.warning_time as i64) as i32,
            warning_blocks: i("warning_blocks", d.warning_blocks as i64) as i32,
            extent,
            dirty: false,
        }
    }
}

// ---- tick rate ----------------------------------------------------------------------------

/// `ServerTickRateManager` (with `TickRateManager`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TickRate {
    rate: f32,
    nanos_per_tick: i64,
    frozen_ticks_to_run: i32,
    run_game_elements: bool,
    frozen: bool,
    remaining_sprint_ticks: i64,
    sprint_tick_start: Option<std::time::Instant>,
    sprint_time_spent: i64,
    scheduled_sprint_ticks: i64,
    previous_frozen: bool,
}

impl Default for TickRate {
    fn default() -> Self {
        TickRate {
            rate: 20.0,
            nanos_per_tick: 50_000_000,
            frozen_ticks_to_run: 0,
            run_game_elements: true,
            frozen: false,
            remaining_sprint_ticks: 0,
            sprint_tick_start: None,
            sprint_time_spent: 0,
            scheduled_sprint_ticks: 0,
            previous_frozen: false,
        }
    }
}

/// What the tick rate manager wants told after a change.
#[derive(Default)]
pub(crate) struct TickNews {
    /// `updateStateToClients`.
    pub state: bool,
    /// `updateStepTicks`.
    pub steps: bool,
    /// A sprint finished: its report (`commands.tick.sprint.report`).
    pub report: Option<Text>,
}

impl TickRate {
    /// `runsNormally`: whether the levels tick (not frozen, or stepping).
    pub fn runs_normally(&self) -> bool {
        self.run_game_elements
    }

    pub fn is_sprinting(&self) -> bool {
        self.scheduled_sprint_ticks > 0
    }

    pub fn nanos_per_tick(&self) -> i64 {
        self.nanos_per_tick
    }

    pub fn state_packet(&self) -> Bytes {
        fx::ticking_state(self.rate, self.frozen)
    }

    pub fn step_packet(&self) -> Bytes {
        fx::ticking_step(self.frozen_ticks_to_run)
    }

    /// `TickRateManager.tick`, at the start of a server tick.
    pub fn tick(&mut self) {
        self.run_game_elements = !self.frozen || self.frozen_ticks_to_run > 0;
        if self.frozen_ticks_to_run > 0 {
            self.frozen_ticks_to_run -= 1;
        }
    }

    fn set_rate(&mut self, rate: f32) {
        self.rate = rate.max(1.0);
        self.nanos_per_tick = (1_000_000_000f64 / self.rate as f64) as i64;
    }

    /// `finishTickSprint`.
    fn finish_sprint(&mut self, news: &mut TickNews) {
        let ticks = self.scheduled_sprint_ticks - self.remaining_sprint_ticks;
        let millis = 1f64.max(self.sprint_time_spent as f64) / 1_000_000.0;
        let per_second = (1000.0 * ticks as f64 / millis) as i32;
        let per_tick = if ticks == 0 { (self.nanos_per_tick as f32 / 1_000_000f32) as f64 } else { millis / ticks as f64 };
        let per_tick = kiln_command::vanilla::java_fixed(per_tick, 2);
        self.scheduled_sprint_ticks = 0;
        self.sprint_time_spent = 0;
        news.report = Some(tr!("commands.tick.sprint.report", per_second, per_tick));
        self.remaining_sprint_ticks = 0;
        self.frozen = self.previous_frozen;
        news.state = true;
    }

    /// `checkShouldSprintThisTick`, before each tick while sprinting.
    pub fn check_sprint(&mut self, news: &mut TickNews) -> bool {
        if !self.run_game_elements {
            return false;
        }
        if self.remaining_sprint_ticks > 0 {
            self.sprint_tick_start = Some(std::time::Instant::now());
            self.remaining_sprint_ticks -= 1;
            return true;
        }
        self.finish_sprint(news);
        false
    }

    /// `endTickWork`.
    pub fn end_tick_work(&mut self) {
        if let Some(start) = self.sprint_tick_start {
            self.sprint_time_spent += start.elapsed().as_nanos() as i64;
        }
    }

    /// A `/tick` action; the flag is the manager method's result.
    pub fn apply(&mut self, action: TickRateAction, news: &mut TickNews) -> bool {
        match action {
            TickRateAction::Rate(rate) => {
                self.set_rate(rate);
                news.state = true;
                true
            }
            TickRateAction::Freeze(frozen) => {
                if frozen {
                    if self.is_sprinting() {
                        self.apply(TickRateAction::StopSprinting, news);
                    }
                    if self.frozen_ticks_to_run > 0 {
                        self.apply(TickRateAction::StopStepping, news);
                    }
                }
                self.frozen = frozen;
                news.state = true;
                true
            }
            TickRateAction::Step(ticks) => {
                if !self.frozen {
                    return false;
                }
                self.frozen_ticks_to_run = ticks;
                news.steps = true;
                true
            }
            TickRateAction::StopStepping => {
                if self.frozen_ticks_to_run > 0 {
                    self.frozen_ticks_to_run = 0;
                    news.steps = true;
                    true
                } else {
                    false
                }
            }
            TickRateAction::Sprint(ticks) => {
                let was = self.remaining_sprint_ticks > 0;
                self.sprint_time_spent = 0;
                self.scheduled_sprint_ticks = ticks as i64;
                self.remaining_sprint_ticks = ticks as i64;
                self.previous_frozen = self.frozen;
                self.frozen = false;
                news.state = true;
                was
            }
            TickRateAction::StopSprinting => {
                if self.remaining_sprint_ticks > 0 {
                    self.finish_sprint(news);
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn info(&self) -> TickRateInfo {
        TickRateInfo {
            rate: self.rate,
            nanos_per_tick: self.nanos_per_tick,
            frozen: self.frozen,
            sprinting: self.is_sprinting(),
            ..TickRateInfo::default()
        }
    }
}

// ---- random sequences ---------------------------------------------------------------------

/// `RandomSequences`: named Xoroshiro sources seeded from the world seed and their names.
#[derive(Debug, Clone)]
pub(crate) struct RandomSequences {
    salt: i32,
    include_world_seed: bool,
    include_sequence_id: bool,
    sequences: BTreeMap<String, XoroshiroRandom>,
}

impl Default for RandomSequences {
    fn default() -> Self {
        RandomSequences { salt: 0, include_world_seed: true, include_sequence_id: true, sequences: BTreeMap::new() }
    }
}

impl RandomSequences {
    /// `createSequence(id, worldSeed, salt, includeWorldSeed, includeSequenceId)`.
    fn create(id: &str, world_seed: i64, salt: i32, world: bool, with_id: bool) -> XoroshiroRandom {
        let seed = (if world { world_seed } else { 0 }) ^ salt as i64;
        let mut s = Seed128::upgrade_unmixed(seed);
        if with_id {
            let h = Seed128::from_hash_of(id);
            s = s.xor(h.lo, h.hi);
        }
        XoroshiroRandom::from_seed128(s.mixed())
    }

    /// `get(id, worldSeed)`: the sequence, created with the defaults if new.
    pub fn get(&mut self, id: &str, world_seed: i64) -> &mut XoroshiroRandom {
        let (salt, w, i) = (self.salt, self.include_world_seed, self.include_sequence_id);
        self.sequences.entry(id.to_owned()).or_insert_with(|| Self::create(id, world_seed, salt, w, i))
    }

    pub fn reset(&mut self, id: &str, world_seed: i64, params: Option<(i32, bool, bool)>) {
        let (salt, w, i) = params.unwrap_or((self.salt, self.include_world_seed, self.include_sequence_id));
        self.sequences.insert(id.to_owned(), Self::create(id, world_seed, salt, w, i));
    }

    pub fn clear(&mut self, defaults: Option<(i32, bool, bool)>) -> i32 {
        if let Some((salt, w, i)) = defaults {
            (self.salt, self.include_world_seed, self.include_sequence_id) = (salt, w, i);
        }
        let n = self.sequences.len() as i32;
        self.sequences.clear();
        n
    }

    pub fn ids(&self) -> Vec<String> {
        self.sequences.keys().cloned().collect()
    }

    pub fn to_nbt(&self) -> Tag {
        let mut fields = vec![("salt".to_owned(), Tag::Int(self.salt))];
        if !self.include_world_seed {
            fields.push(("include_world_seed".into(), Tag::Byte(0)));
        }
        if !self.include_sequence_id {
            fields.push(("include_sequence_id".into(), Tag::Byte(0)));
        }
        let sequences = self
            .sequences
            .iter()
            .map(|(id, r)| {
                let s = r.state();
                (id.clone(), Tag::Compound(vec![("source".into(), Tag::LongArray(vec![s.lo, s.hi]))]))
            })
            .collect();
        fields.push(("sequences".into(), Tag::Compound(sequences)));
        Tag::Compound(fields)
    }

    pub fn from_nbt(tag: &Tag) -> Self {
        let flag = |k: &str| tag.get(k).and_then(Tag::as_i64).is_none_or(|v| v != 0);
        let mut sequences = BTreeMap::new();
        if let Some(Tag::Compound(entries)) = tag.get("sequences") {
            for (id, entry) in entries {
                if let Some([lo, hi]) = entry.get("source").and_then(Tag::as_long_array).and_then(|a| <[i64; 2]>::try_from(a).ok())
                {
                    sequences.insert(id.clone(), XoroshiroRandom::from_seed128(Seed128 { lo, hi }));
                }
            }
        }
        RandomSequences {
            salt: tag.get("salt").and_then(Tag::as_i64).unwrap_or(0) as i32,
            include_world_seed: flag("include_world_seed"),
            include_sequence_id: flag("include_sequence_id"),
            sequences,
        }
    }
}

// ---- all of it ----------------------------------------------------------------------------

/// The state above, kept by the [`Sim`].
pub(crate) struct WorldState {
    /// Each level's border, by [`crate::DimId`].
    pub borders: [Border; 3],
    pub tick_rate: TickRate,
    /// Each level's force-loaded chunks.
    pub forced: [BTreeSet<[i32; 2]>; 3],
    pub sequences: RandomSequences,
    /// The level random `/random` draws from without a sequence.
    pub level_random: XoroshiroRandom,
    /// Last tick durations (nanoseconds), for `/tick query`.
    pub tick_times: Vec<i64>,
    pub tick_index: usize,
    pub forced_dirty: bool,
    /// Each level's generation pipeline (none for flat levels), for `/locate`.
    pub pipelines: Vec<Option<std::sync::Arc<kiln_worldgen::pipeline::Pipeline>>>,
    /// Each level's worldgen as block behaviour uses it (what grows: saplings, bone meal).
    pub feature_hosts: Vec<Option<std::sync::Arc<dyn kiln_blocks::feature_host::FeatureHost>>>,
    /// The structure templates (`StructureTemplateManager`).
    pub templates: crate::structure_block::Templates,
}

impl Default for WorldState {
    fn default() -> Self {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        WorldState {
            feature_hosts: Vec::new(),
            templates: Default::default(),
            borders: Default::default(),
            tick_rate: TickRate::default(),
            forced: Default::default(),
            sequences: RandomSequences::default(),
            level_random: XoroshiroRandom::new(now.as_nanos() as i64),
            tick_times: vec![0; 100],
            tick_index: 0,
            forced_dirty: false,
            pipelines: Vec::new(),
        }
    }
}

const WORLD_BORDER: &str = "world_border";
const CHUNK_TICKETS: &str = "chunk_tickets";
const RANDOM_SEQUENCES: &str = "random_sequences";

impl Sim {
    fn level_dir(&self, dim: crate::DimId) -> Option<std::path::PathBuf> {
        let storage = self.storage.as_ref()?;
        Some(storage.dir.join(crate::dimension_dir(DIMENSIONS[dim].0)))
    }

    /// Loads the borders, forced chunks and random sequences of a saved world.
    pub(crate) fn load_world_state(&mut self) {
        for dim in 0..DIMENSIONS.len() {
            let Some(dir) = self.level_dir(dim) else { continue };
            if let Some(data) = kiln_storage::saved_data::read(&dir, WORLD_BORDER) {
                self.world.borders[dim] = Border::from_nbt(&data);
            }
            if let Some(Tag::List(tickets)) = kiln_storage::saved_data::read(&dir, CHUNK_TICKETS).and_then(|d| d.get("tickets").cloned()) {
                for t in &tickets {
                    let forced = t.get("type").and_then(Tag::as_str) == Some("minecraft:forced");
                    if let (true, Some(Tag::IntArray(p))) = (forced, t.get("chunk_pos")) {
                        if let [x, z] = p[..] {
                            self.world.forced[dim].insert([x, z]);
                        }
                    }
                }
            }
        }
        if let Some(data) = self.storage.as_ref().and_then(|s| kiln_storage::saved_data::read(&s.dir, RANDOM_SEQUENCES)) {
            self.world.sequences = RandomSequences::from_nbt(&data);
        }
    }

    pub(crate) fn save_world_state(&mut self) {
        for dim in 0..DIMENSIONS.len() {
            let Some(dir) = self.level_dir(dim) else { continue };
            if self.world.borders[dim].dirty {
                if let Err(e) = kiln_storage::saved_data::write(&dir, WORLD_BORDER, self.world.borders[dim].to_nbt()) {
                    tracing::warn!("failed to save the world border: {e}");
                }
                self.world.borders[dim].dirty = false;
            }
            if self.world.forced_dirty {
                let tickets = self.world.forced[dim]
                    .iter()
                    .map(|&[x, z]| {
                        Tag::Compound(vec![
                            ("type".into(), Tag::String("minecraft:forced".into())),
                            ("level".into(), Tag::Int(31)),
                            ("chunk_pos".into(), Tag::IntArray(vec![x, z])),
                        ])
                    })
                    .collect();
                let data = Tag::Compound(vec![("tickets".into(), Tag::List(tickets))]);
                if let Err(e) = kiln_storage::saved_data::write(&dir, CHUNK_TICKETS, data) {
                    tracing::warn!("failed to save the chunk tickets: {e}");
                }
            }
        }
        self.world.forced_dirty = false;
        self.maps.lock().unwrap_or_else(|e| e.into_inner()).save();
        if let Some(storage) = &self.storage {
            if let Err(e) = kiln_storage::saved_data::write(&storage.dir, RANDOM_SEQUENCES, self.world.sequences.to_nbt()) {
                tracing::warn!("failed to save the random sequences: {e}");
            }
        }
    }

    /// `PlayerList.sendLevelInfo` on a level change: the level's border, then its weather.
    pub(crate) fn level_info_packets(&self, dim: crate::DimId) -> Vec<Bytes> {
        let mut out = vec![self.world.borders[dim].init_packet()];
        out.extend(self.weather_packets(dim));
        out
    }

    /// The levels' part of the tick: borders move while the game runs normally.
    pub(crate) fn tick_borders(&mut self) {
        if !self.world.tick_rate.runs_normally() {
            return;
        }
        for b in &mut self.world.borders {
            b.tick();
        }
    }

    /// `/worldborder`.
    pub(crate) fn change_border(&mut self, dim: crate::DimId, change: BorderChange) {
        if let Some(pkt) = self.world.borders[dim].apply(change) {
            self.broadcast_in(dim, pkt);
        }
    }

    /// `ServerLevel.setChunkForced`: loads and keeps the chunk, or lets it go.
    pub(crate) fn set_forced(&mut self, dim: crate::DimId, chunk: [i32; 2], forced: bool) -> bool {
        let changed = if forced { self.world.forced[dim].insert(chunk) } else { self.world.forced[dim].remove(&chunk) };
        if changed {
            self.world.forced_dirty = true;
            if forced {
                self.dims[dim].load_chunk(kiln_world::ChunkPos::new(chunk[0], chunk[1]));
            }
        }
        changed
    }

    /// Tells every player about a tick rate change and reports a finished sprint.
    pub(crate) fn tick_rate_news(&mut self, news: TickNews) {
        if news.state {
            let pkt = self.world.tick_rate.state_packet();
            self.broadcast(pkt);
        }
        if news.steps {
            let pkt = self.world.tick_rate.step_packet();
            self.broadcast(pkt);
        }
        if let Some(report) = news.report {
            // `createCommandSourceStack().sendSuccess(report, true)`: the console and operators.
            tracing::info!(target: "kiln_sim::commands", "{}", crate::commands::console_text(&report));
            let admin = tr!("chat.type.admin", Text::literal("Server"), report).color("gray").italic();
            let pkt = kiln_proto::packets::system_chat(admin.to_nbt(), false);
            let ops: Vec<_> = self.players.iter().filter(|(_, p)| self.commands.is_op(&p.name)).map(|(c, _)| *c).collect();
            for c in ops {
                if let Some(p) = self.players.get_mut(&c) {
                    p.send(pkt.clone());
                }
            }
        }
    }

    /// Records a tick's duration for `/tick query`.
    pub(crate) fn record_tick_time(&mut self, nanos: i64) {
        let w = &mut self.world;
        w.tick_times[w.tick_index % 100] = nanos;
        w.tick_index += 1;
    }

    /// `/random` without a sequence: the level random.
    pub(crate) fn level_random_between(&mut self, min: i32, max: i32) -> i32 {
        self.world.level_random.next_int_bounded(max.wrapping_sub(min).wrapping_add(1)) + min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_moves_by_ticks() {
        let mut b = Border::default();
        b.apply(BorderChange::Size(100.0));
        b.apply(BorderChange::Lerp { from: 100.0, to: 50.0, ticks: 10 });
        assert_eq!((b.size(), b.lerp_time(), b.lerp_target()), (100.0, 10, 50.0));
        b.tick();
        assert_eq!(b.size(), 95.0);
        assert_eq!(b.bounds().max_x, 50.0, "bounds use the previous size");
        for _ in 0..9 {
            b.tick();
        }
        assert_eq!((b.size(), b.lerp_time()), (50.0, 0));
        assert_eq!(Border::from_nbt(&b.to_nbt()).info(), b.info());
    }

    #[test]
    fn border_damage() {
        let mut b = Border::default();
        b.apply(BorderChange::Size(10.0));
        let bx = b.bounds();
        assert_eq!(bx.damage([0.0, 0.0, 0.0], 0.3), None);
        // Five blocks of buffer: 6 blocks out is one block past it.
        assert_eq!(bx.damage([11.0, 0.0, 0.0], 0.3), Some(1.0));
        assert_eq!(bx.damage([30.0, 0.0, 0.0], 0.3), Some(4.0));
        assert_eq!(bx.damage([5.5, 0.0, 0.0], 0.3), None, "inside the buffer");
    }

    #[test]
    fn random_sequence_matches_vanilla_seeding() {
        // `RandomSequence(seed, id)`: upgradeSeedTo128bitUnmixed(seed) xor md5(id), mixed.
        let mut s = RandomSequences::default();
        let a = s.get("minecraft:test", 1).next_long();
        let mut t = RandomSequences::default();
        assert_eq!(t.get("minecraft:test", 1).next_long(), a);
        assert_ne!(t.get("minecraft:other", 1).next_long(), a);
        let back = RandomSequences::from_nbt(&s.to_nbt());
        assert_eq!(back.ids(), vec!["minecraft:test".to_owned()]);
    }

    #[test]
    fn tick_rate_freeze_and_step() {
        let mut t = TickRate::default();
        let mut news = TickNews::default();
        assert!(!t.apply(TickRateAction::Step(1), &mut news));
        t.apply(TickRateAction::Freeze(true), &mut news);
        t.tick();
        assert!(!t.runs_normally());
        assert!(t.apply(TickRateAction::Step(2), &mut news));
        t.tick();
        assert!(t.runs_normally());
        t.tick();
        assert!(t.runs_normally());
        t.tick();
        assert!(!t.runs_normally());
        assert!(!t.apply(TickRateAction::StopSprinting, &mut news));
        assert!(!t.apply(TickRateAction::Sprint(5), &mut news));
        assert!(t.is_sprinting());
        assert!(t.apply(TickRateAction::StopSprinting, &mut news));
        assert!(news.report.is_some());
        assert!(t.frozen, "a sprint restores the frozen state");
    }
}

// ---- locate, fillbiome --------------------------------------------------------------------

/// `Mth.outFromOrigin(origin, lower, upper, step)`: the origin (clamped), then alternately
/// above and below it.
pub(crate) fn out_from_origin(origin: i32, lower: i32, upper: i32, step: i32) -> Vec<i32> {
    let start = origin.clamp(lower, upper);
    let mut out = Vec::new();
    let mut v = start;
    loop {
        let d = (start - v).abs();
        if !(start - d >= lower || start + d <= upper) {
            break;
        }
        out.push(v);
        let up = v <= start;
        let can_go_up = start + d + step <= upper;
        v = if !up || !can_go_up {
            let next = start - d - if up { step } else { 0 };
            if next >= lower { next } else { start + d + step }
        } else {
            start + d + step
        };
    }
    out
}

/// `BlockPos.spiralAround(ZERO, radius, EAST, SOUTH)`: the centre, then rings outward.
pub(crate) fn spiral(radius: i32) -> impl Iterator<Item = [i32; 2]> {
    const DIRS: [[i32; 2]; 4] = [[1, 0], [0, 1], [-1, 0], [0, -1]];
    let legs = 4 * radius;
    let (mut cursor, mut leg, mut leg_size, mut leg_index) = ([0, 1], -1i32, 0, 0);
    std::iter::from_fn(move || {
        let d = DIRS[((leg + 4) % 4) as usize];
        cursor = [cursor[0] + d[0], cursor[1] + d[1]];
        if leg_index >= leg_size {
            if leg >= legs {
                return None;
            }
            leg += 1;
            leg_index = 0;
            leg_size = leg / 2 + 1;
        }
        leg_index += 1;
        Some(cursor)
    })
}

/// `PoiTypes`: each point of interest type's blocks (all their states, beds by their head).
const POI_TYPES: &[(&str, &[&str])] = &[
    ("minecraft:armorer", &["minecraft:blast_furnace"]),
    ("minecraft:butcher", &["minecraft:smoker"]),
    ("minecraft:cartographer", &["minecraft:cartography_table"]),
    ("minecraft:cleric", &["minecraft:brewing_stand"]),
    ("minecraft:farmer", &["minecraft:composter"]),
    ("minecraft:fisherman", &["minecraft:barrel"]),
    ("minecraft:fletcher", &["minecraft:fletching_table"]),
    (
        "minecraft:leatherworker",
        &["minecraft:cauldron", "minecraft:water_cauldron", "minecraft:lava_cauldron", "minecraft:powder_snow_cauldron"],
    ),
    ("minecraft:librarian", &["minecraft:lectern"]),
    ("minecraft:mason", &["minecraft:stonecutter"]),
    ("minecraft:shepherd", &["minecraft:loom"]),
    ("minecraft:toolsmith", &["minecraft:smithing_table"]),
    ("minecraft:weaponsmith", &["minecraft:grindstone"]),
    ("minecraft:home", &["#minecraft:beds"]),
    ("minecraft:meeting", &["minecraft:bell"]),
    ("minecraft:beehive", &["minecraft:beehive"]),
    ("minecraft:bee_nest", &["minecraft:bee_nest"]),
    ("minecraft:nether_portal", &["minecraft:nether_portal"]),
    ("minecraft:lodestone", &["minecraft:lodestone"]),
    ("minecraft:test_instance", &["minecraft:test_instance_block"]),
    ("minecraft:lightning_rod", &["#minecraft:lightning_rods"]),
];

/// The point of interest type of a block state, if it is one (computed once for all states).
fn poi_type(state: u16) -> Option<&'static str> {
    static TYPES: std::sync::OnceLock<Vec<Option<&'static str>>> = std::sync::OnceLock::new();
    let types = TYPES.get_or_init(|| (0..kiln_data::blocks::STATE_COUNT as u32).map(|s| compute_poi_type(s as u16)).collect());
    types.get(state as usize).copied().flatten()
}

fn compute_poi_type(state: u16) -> Option<&'static str> {
    let block = kiln_data::blocks_types::block_of(state);
    let id = kiln_data::builtin_id("minecraft:block", block.name)?;
    let poi = POI_TYPES.iter().find_map(|(poi, blocks)| {
        blocks
            .iter()
            .any(|b| match b.strip_prefix('#') {
                Some(tag) => kiln_command::blocks::registry_tag("minecraft:block", tag).is_some_and(|ids| ids.contains(&id)),
                None => *b == block.name,
            })
            .then_some(*poi)
    })?;
    // Beds count by their head half only.
    if poi == "minecraft:home" && block.property(state, "part") != Some("head") {
        return None;
    }
    Some(poi)
}

impl Sim {
    /// `ServerLevel.findClosestBiome3d(origin, 6400, 32, 64)` over the level's biome source.
    pub(crate) fn locate_biome(
        &mut self,
        dim: crate::DimId,
        origin: [i32; 3],
        matches: &dyn Fn(&str) -> bool,
    ) -> Option<([i32; 3], String)> {
        let d = self.dims[dim].provider.dimension;
        let ys = out_from_origin(origin[1], d.min_y + 1, d.min_y + d.height, 64);
        match self.world.pipelines.get(dim).cloned().flatten() {
            None => {
                // A flat world's fixed biome source.
                let biome = DIMENSIONS[dim].1;
                if !matches(biome) {
                    return None;
                }
                Some(([origin[0], ys[0], origin[2]], biome.to_owned()))
            }
            Some(pipeline) => {
                let g = &pipeline.world().generator;
                let possible: Vec<u16> = g.possible_biomes();
                if !possible.iter().any(|&b| matches(&g.biomes[b as usize].name)) {
                    return None;
                }
                let mut gs = kiln_worldgen::generator::GenScratch::default();
                let mut last = None;
                for [sx, sz] in spiral(6400 / 32) {
                    let (x, z) = (origin[0] + sx * 32, origin[2] + sz * 32);
                    for &y in &ys {
                        let b = g.point_biome(gs.point_context(), &mut last, x >> 2, y >> 2, z >> 2);
                        let name = &g.biomes[b as usize].name;
                        if possible.contains(&b) && matches(name) {
                            return Some(([x, y, z], name.clone()));
                        }
                    }
                }
                None
            }
        }
    }

    /// `ChunkGenerator.findNearestMapStructure(level, structures, origin, 100, false)` over the
    /// level's generator: the nearest start of any of `structures` (ids).
    pub(crate) fn locate_structure(&mut self, dim: crate::DimId, origin: [i32; 3], structures: &[String]) -> Option<([i32; 3], String)> {
        let pipeline = self.world.pipelines.get(dim).cloned().flatten()?;
        let mut gs = kiln_worldgen::generator::GenScratch::default();
        pipeline.find_nearest_structure(&mut gs, structures, origin, 100)
    }

    /// `PoiManager.findClosestWithType(types, origin, 256, ANY)` over the loaded chunks'
    /// blocks: the nearest (3D) point of interest within 256 blocks.
    pub(crate) fn locate_poi(
        &mut self,
        dim: crate::DimId,
        origin: [i32; 3],
        matches: &dyn Fn(&str) -> bool,
    ) -> Option<([i32; 3], String)> {
        use kiln_world::Blocks;
        use kiln_world::section::BlockContainer;
        let wanted: Vec<&str> = POI_TYPES.iter().map(|(p, _)| *p).filter(|p| matches(p)).collect();
        if wanted.is_empty() {
            return None;
        }
        let radius = 256i64;
        let (ocx, ocz) = (origin[0] >> 4, origin[2] >> 4);
        let r = (256 >> 4) + 1;
        let mut best: Option<(i64, [i32; 3], &'static str)> = None;
        let regions = &self.dims[dim].regions;
        for cx in ocx - r..=ocx + r {
            for cz in ocz - r..=ocz + r {
                let Some(chunk) = regions.chunk(kiln_world::ChunkPos::new(cx, cz)) else { continue };
                for (si, section) in chunk.sections.iter().enumerate() {
                    let candidate = match &section.blocks {
                        BlockContainer::Single(s) => poi_type(*s).is_some_and(|p| wanted.contains(&p)),
                        BlockContainer::Nibble { palette, .. } | BlockContainer::Byte { palette, .. } => {
                            palette.iter().any(|&s| poi_type(s).is_some_and(|p| wanted.contains(&p)))
                        }
                        BlockContainer::Direct(_) => true,
                    };
                    if !candidate {
                        continue;
                    }
                    let base_y = chunk.min_y() + si as i32 * 16;
                    for i in 0..4096usize {
                        let state = section.blocks.get(i);
                        let Some(p) = poi_type(state).filter(|p| wanted.contains(p)) else { continue };
                        let pos = [cx * 16 + (i & 15) as i32, base_y + (i >> 8) as i32, cz * 16 + ((i >> 4) & 15) as i32];
                        let d: i64 = (0..3).map(|k| (pos[k] - origin[k]) as i64).map(|v| v * v).sum();
                        if d <= radius * radius && best.is_none_or(|(bd, _, _)| d < bd) {
                            best = Some((d, pos, p));
                        }
                    }
                }
            }
        }
        best.map(|(_, pos, p)| (pos, p.to_owned()))
    }

    /// `FillBiomeCommand.fill`: sets `biome` in the quart cells of the loaded chunks inside
    /// `[min, max]` whose biome `filter` accepts; resends the changed chunks' biomes. `None`
    /// when a chunk is not loaded.
    pub(crate) fn fill_biome(
        &mut self,
        dim: crate::DimId,
        min: [i32; 3],
        max: [i32; 3],
        biome: &str,
        filter: &dyn Fn(&str) -> bool,
    ) -> Option<i32> {
        use kiln_world::Blocks;
        use kiln_world::section::Biomes;
        let names = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome")?.1;
        let new = names.iter().position(|b| *b == biome)? as u16;
        let chunks: Vec<kiln_world::ChunkPos> = ((min[2] >> 4)..=(max[2] >> 4))
            .flat_map(|z| ((min[0] >> 4)..=(max[0] >> 4)).map(move |x| kiln_world::ChunkPos::new(x, z)))
            .collect();
        let regions = &mut self.dims[dim].regions;
        if chunks.iter().any(|&c| regions.chunk(c).is_none()) {
            return None;
        }
        let inside = |x: i32, y: i32, z: i32| {
            (min[0]..=max[0]).contains(&x) && (min[1]..=max[1]).contains(&y) && (min[2]..=max[2]).contains(&z)
        };
        let mut count = 0;
        let mut changed = Vec::new();
        for &c in &chunks {
            let chunk = regions.chunk_mut(c).expect("loaded");
            let min_y = chunk.min_y();
            let before = count;
            for (si, section) in chunk.sections.iter_mut().enumerate() {
                for i in 0..64usize {
                    let (qx, qz, qy) = (i & 3, (i >> 2) & 3, i >> 4);
                    let (x, y, z) = (c.x * 16 + qx as i32 * 4, min_y + si as i32 * 16 + qy as i32 * 4, c.z * 16 + qz as i32 * 4);
                    let current = match &section.biomes {
                        Biomes::Single(b) => *b,
                        Biomes::Cells(cells) => cells[i],
                    };
                    if !inside(x, y, z) || !names.get(current as usize).is_some_and(|n| filter(n)) || current == new {
                        continue;
                    }
                    count += 1;
                    if let Biomes::Single(b) = section.biomes {
                        section.biomes = Biomes::Cells(Box::new([b; 64]));
                    }
                    if let Biomes::Cells(cells) = &mut section.biomes {
                        cells[i] = new;
                    }
                }
            }
            if count != before {
                chunk.mark_biomes_changed();
                changed.push(c);
            }
        }
        if count > 0 {
            self.resend_biomes(dim, &changed);
        }
        Some(count)
    }

    /// `ChunkMap.resendBiomesForChunks`: Chunk Biomes to the players who have the chunks.
    fn resend_biomes(&mut self, dim: crate::DimId, chunks: &[kiln_world::ChunkPos]) {
        use kiln_world::Blocks;
        let biome_count = self.dims[dim].provider.biome_count;
        let data: Vec<(kiln_world::ChunkPos, Vec<u8>)> = chunks
            .iter()
            .filter_map(|&c| {
                let chunk = self.dims[dim].regions.chunk(c)?;
                let mut b = bytes::BytesMut::new();
                for s in &chunk.sections {
                    s.biomes.encode(&mut b, biome_count);
                }
                Some((c, b.to_vec()))
            })
            .collect();
        for p in self.players.values_mut().filter(|p| p.dim == dim) {
            let mine: Vec<([i32; 2], Vec<u8>)> =
                data.iter().filter(|(c, _)| p.sent_chunks.contains(c)).map(|(c, d)| ([c.x, c.z], d.clone())).collect();
            if !mine.is_empty() {
                p.send(fx::chunks_biomes(&mine));
            }
        }
    }
}

#[cfg(test)]
mod locate_tests {
    use super::*;

    #[test]
    fn out_from_origin_alternates() {
        assert_eq!(out_from_origin(0, -2, 2, 1), vec![0, 1, -1, 2, -2]);
        assert_eq!(out_from_origin(100, -63, 320, 64), vec![100, 164, 36, 228, -28, 292]);
    }

    #[test]
    fn spiral_order() {
        let v: Vec<[i32; 2]> = spiral(1).collect();
        assert_eq!(v, vec![[0, 0], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0], [-1, -1], [0, -1], [1, -1]]);
    }
}

// ---- data pack registries the world commands look up --------------------------------------

/// Ids of the vanilla data pack's `data/minecraft/<kind>/*.json` (e.g. `worldgen/structure`),
/// sorted; read once.
pub(crate) fn worldgen_ids(kind: &'static str) -> &'static Vec<String> {
    static IDS: std::sync::Mutex<BTreeMap<&'static str, &'static Vec<String>>> = std::sync::Mutex::new(BTreeMap::new());
    let mut map = IDS.lock().expect("ids lock");
    map.entry(kind).or_insert_with(|| {
        let dir = crate::datapack_dir(None).join("data/minecraft").join(kind);
        let mut ids: Vec<String> = Vec::new();
        collect_json(&dir, "", &mut ids);
        ids.sort();
        Box::leak(Box::new(ids))
    })
}

fn collect_json(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if path.is_dir() {
            collect_json(&path, &format!("{prefix}{name}/"), out);
        } else if let Some(stem) = name.strip_suffix(".json") {
            out.push(format!("minecraft:{prefix}{stem}"));
        }
    }
}

/// The entries of tag `tag` of `kind` (nested tags expanded), or `None` without such a tag.
pub(crate) fn worldgen_tag(kind: &str, tag: &str) -> Option<Vec<String>> {
    let (ns, path) = tag.split_once(':').unwrap_or(("minecraft", tag));
    let file = crate::datapack_dir(None).join(format!("data/{ns}/tags/{kind}/{path}.json"));
    let text = std::fs::read_to_string(file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut out = Vec::new();
    for v in json.get("values")?.as_array()? {
        let id = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.get("id")?.as_str()?.to_owned(),
        };
        match id.strip_prefix('#') {
            Some(inner) => out.extend(worldgen_tag(kind, inner).unwrap_or_default()),
            None => out.push(if id.contains(':') { id } else { format!("minecraft:{id}") }),
        }
    }
    Some(out)
}

// ---- place --------------------------------------------------------------------------------

/// `StructureTemplateManager` over the vanilla data (the pack's `data/*/structure`, then the
/// server jar).
fn templates() -> &'static kiln_worldgen::structure::template::TemplateManager {
    static M: std::sync::OnceLock<kiln_worldgen::structure::template::TemplateManager> = std::sync::OnceLock::new();
    M.get_or_init(|| kiln_worldgen::structure::template::TemplateManager::near(&crate::datapack_dir(None)))
}

impl Sim {
    /// `/place template`: `StructureTemplate.placeInWorld` with the rotation and mirror about
    /// the template's corner at `pos`, block entity data loaded, `strict` placing states as
    /// they are. Integrity below 1 drops blocks (`BlockRotProcessor`) with a legacy random
    /// seeded from `seed`.
    pub(crate) fn place_template(
        &mut self,
        dim: crate::DimId,
        id: &str,
        pos: [i32; 3],
        rotation: u8,
        mirror: u8,
        integrity: f32,
        seed: i32,
        strict: bool,
    ) -> Result<(), kiln_command::CommandError> {
        use kiln_command::Host;
        use kiln_worldgen::pos::BlockPos;
        use kiln_worldgen::structure::template::transform;
        use kiln_worldgen::structure::transform::{Mirror, Rotation, mirror as mirror_state, rotate};
        let roots = self.commands.packs.roots();
        let template = templates().find_in(&roots, id).unwrap_or_default();
        if template.palettes.is_empty() && template.size == [0, 0, 0] {
            return Err(kiln_command::CommandError::new(tr!("commands.place.template.invalid", id)));
        }
        let dimension = DIMENSIONS[dim].0;
        let [sx, sy, sz] = template.size;
        let far = [pos[0] + sx, pos[1] + sy, pos[2] + sz];
        for cx in (pos[0] >> 4).min(far[0] >> 4)..=(pos[0] >> 4).max(far[0] >> 4) {
            for cz in (pos[2] >> 4).min(far[2] >> 4)..=(pos[2] >> 4).max(far[2] >> 4) {
                if !self.is_chunk_loaded(dimension, cx, cz) {
                    return Err(kiln_command::CommandError::pos_unloaded());
                }
            }
        }
        let fail = || kiln_command::CommandError::new(tr!("commands.place.template.failed"));
        let Some(palette) = template.palettes.first() else { return Err(fail()) };
        if palette.blocks.is_empty() || sx < 1 || sy < 1 || sz < 1 {
            return Err(fail());
        }
        let rotation = [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::CounterClockwise90][rotation as usize];
        let mirror = [Mirror::None, Mirror::LeftRight, Mirror::FrontBack][mirror as usize];
        let mut random = kiln_javamath::random::LegacyRandom::new(seed as i64);
        let flags = kiln_command::UpdateFlags(
            kiln_command::UpdateFlags::CLIENTS | if strict { kiln_command::UpdateFlags::STRICT } else { 0 },
        );
        for b in &palette.blocks {
            if integrity < 1.0 && random.next_float() > integrity {
                continue;
            }
            let p = transform(b.pos, mirror, rotation, BlockPos::new(0, 0, 0));
            let at = [p.x + pos[0], p.y + pos[1], p.z + pos[2]];
            let state = rotate(mirror_state(b.state, mirror), rotation);
            self.set_block(dimension, at, state, b.nbt.as_deref(), flags);
        }
        Ok(())
    }
}

impl Sim {
    /// The world border of level `dimension`: `(center, size, remaining ticks of a move)`.
    pub fn world_border_of(&self, dimension: &str) -> Option<([f64; 2], f64, i64)> {
        let b = &self.world.borders[crate::dim_id(dimension)?];
        Some((b.center, b.size(), b.lerp_time()))
    }

    /// Force-loaded chunks of level `dimension`.
    pub fn forced_chunks_of(&self, dimension: &str) -> Vec<[i32; 2]> {
        crate::dim_id(dimension).map(|d| self.world.forced[d].iter().copied().collect()).unwrap_or_default()
    }

    /// Whether the levels run this tick (`/tick freeze` stops them).
    pub fn runs_normally(&self) -> bool {
        self.world.tick_rate.runs_normally()
    }
}

impl Sim {
    /// `StructureTemplateManager.get`: a template of the enabled packs or the vanilla data.
    pub(crate) fn find_template(&self, id: &str) -> Option<std::sync::Arc<kiln_worldgen::structure::template::Template>> {
        templates().find_in(&self.commands.packs.roots(), id)
    }
}
