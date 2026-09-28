//! An in-memory [`Level`] for tests and the vanilla differential harness: a flat world of
//! fixed layers, lazily materialised per section, with every touched chunk loaded.

use crate::fluid::FluidType;
use crate::level::{Effect, Level, LevelData, Rules, UpdateTrace};
use crate::pos::BlockPos;
use crate::state::BlockId;
use crate::ticks::{ChunkKey, ChunkTicks, LevelTicks};
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::LegacyRandom;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// A multiplicative hasher for section coordinates (the default SipHash dominates block reads).
#[derive(Default)]
struct SectionHasher(u64);

impl Hasher for SectionHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }

    fn write_i32(&mut self, v: i32) {
        self.0 = (self.0.rotate_left(5) ^ v as u32 as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

/// Materialised sections by section coordinates.
type Sections = HashMap<(i32, i32, i32), Box<[u16; 4096]>, BuildHasherDefault<SectionHasher>>;

pub struct TestLevel {
    sections: Sections,
    /// Block of each layer from `min_y` up; above it, air.
    layers: Vec<u16>,
    pub min_y: i32,
    pub height: i32,
    pub game_time: i64,
    sub_tick: i64,
    pub block_ticks: LevelTicks<BlockId>,
    pub fluid_ticks: LevelTicks<FluidType>,
    loaded: HashSet<ChunkKey>,
    random: LegacyRandom,
    data: LevelData,
    pub rules: Rules,
    /// Effects in the order they happened.
    pub effects: Vec<Effect>,
    /// When set, every update run is recorded here in order.
    pub trace: Option<Vec<UpdateTrace>>,
    /// Comparator block entities' output signals.
    pub comparator_outputs: HashMap<BlockPos, i32>,
    /// Block reads so far (for profiling).
    pub reads: std::cell::Cell<u64>,
    /// The weather precipitation sees.
    pub weather: crate::weather::Weather,
    /// Biome climates for precipitation: (biome position, read position) to the climate.
    #[allow(clippy::type_complexity)]
    pub climate: Option<Box<dyn Fn(BlockPos, BlockPos) -> Option<crate::weather::Climate>>>,
}

impl TestLevel {
    /// A world of `layers` stacked from `min_y` (a superflat preset), air above.
    pub fn flat(min_y: i32, height: i32, layers: &[u16]) -> Self {
        Self {
            sections: HashMap::default(),
            layers: layers.to_vec(),
            min_y,
            height,
            game_time: 0,
            sub_tick: 0,
            block_ticks: LevelTicks::new(),
            fluid_ticks: LevelTicks::new(),
            loaded: HashSet::new(),
            random: LegacyRandom::new(0),
            data: LevelData::new(1_000_000, 0),
            rules: Rules::default(),
            effects: Vec::new(),
            trace: None,
            comparator_outputs: HashMap::new(),
            reads: std::cell::Cell::new(0),
            weather: Default::default(),
            climate: None,
        }
    }

    /// Reseeds the level random (`RandomSource.setSeed`).
    pub fn set_random_seed(&mut self, seed: i64) {
        self.random = LegacyRandom::new(seed);
    }

    /// An empty overworld-height void.
    pub fn void() -> Self {
        Self::flat(-64, 384, &[])
    }

    fn generated(&self, y: i32) -> u16 {
        usize::try_from(y - self.min_y).ok().and_then(|i| self.layers.get(i)).copied().unwrap_or(d::AIR)
    }

    /// Marks chunks as loaded (they get scheduled-tick containers), like `/forceload`.
    pub fn load_chunks(&mut self, min: ChunkKey, max: ChunkKey) {
        for cx in min.0..=max.0 {
            for cz in min.1..=max.1 {
                self.load_chunk((cx, cz));
            }
        }
    }

    fn load_chunk(&mut self, c: ChunkKey) {
        if self.loaded.insert(c) {
            self.block_ticks.add_container(c, ChunkTicks::new());
            self.fluid_ticks.add_container(c, ChunkTicks::new());
        }
    }

    pub fn is_chunk_loaded(&self, c: ChunkKey) -> bool {
        self.loaded.contains(&c)
    }

    pub fn loaded_chunks(&self) -> impl Iterator<Item = ChunkKey> + '_ {
        self.loaded.iter().copied()
    }

    /// Whether the section holds randomly ticking blocks or fluids.
    pub fn section_ticks_randomly(&self, cx: i32, sy: i32, cz: i32) -> bool {
        match self.sections.get(&(cx, sy, cz)) {
            Some(s) => s.iter().any(|&b| crate::tick::randomly_ticks(b)),
            None => (0..16).any(|y| crate::tick::randomly_ticks(self.generated(sy * 16 + y))),
        }
    }

    /// Runs one game tick's block phases in vanilla order (time, block ticks, fluid ticks,
    /// random ticks over loaded chunks in `chunk_order`, block events, then the block-entity
    /// phase for moving pistons).
    pub fn tick(&mut self, random_tick_speed: i32, chunk_order: &[ChunkKey]) {
        self.game_time += 1;
        let loaded = self.loaded.clone();
        crate::tick::run_block_ticks(self, |c| loaded.contains(&c));
        crate::tick::run_fluid_ticks(self, |c| loaded.contains(&c));
        let (min_s, max_s) = (self.min_y >> 4, (self.min_y + self.height - 1) >> 4);
        for &c in chunk_order {
            let sections: Vec<(i32, bool)> = (min_s..=max_s).map(|sy| (sy, self.section_ticks_randomly(c.0, sy, c.1))).collect();
            crate::tick::tick_chunk_blocks(self, c, &sections, random_tick_speed);
        }
        crate::block_events::run_block_events(self, |p| loaded.contains(&p.chunk()));
        crate::behaviour::piston::tick_moving_pistons(self, |p| loaded.contains(&p.chunk()));
    }
}

fn index(pos: BlockPos) -> usize {
    (((pos.y & 15) << 8) | ((pos.z & 15) << 4) | (pos.x & 15)) as usize
}

impl Level for TestLevel {
    type Random = LegacyRandom;

    fn block(&self, pos: BlockPos) -> u16 {
        self.reads.set(self.reads.get() + 1);
        if pos.y < self.min_y || pos.y >= self.min_y + self.height {
            return d::VOID_AIR;
        }
        match self.sections.get(&(pos.x >> 4, pos.y >> 4, pos.z >> 4)) {
            Some(s) => s[index(pos)],
            None => self.generated(pos.y),
        }
    }

    fn set_raw(&mut self, pos: BlockPos, state: u16, _flags: u32) -> Option<u16> {
        let old = self.block(pos);
        if old == state {
            return None;
        }
        let key = (pos.x >> 4, pos.y >> 4, pos.z >> 4);
        if !self.sections.contains_key(&key) {
            let base = pos.y & !15;
            let only_air = (0..16).all(|y| is_air(self.generated(base + y)));
            if only_air && is_air(state) {
                return None;
            }
            let mut blocks = Box::new([d::AIR; 4096]);
            for y in 0..16 {
                let b = self.generated(base + y);
                blocks[(y as usize) << 8..((y as usize) + 1) << 8].fill(b);
            }
            self.sections.insert(key, blocks);
        }
        self.load_chunk(pos.chunk());
        if !crate::state::same_block(old, state) {
            self.comparator_outputs.remove(&pos);
        }
        self.sections.get_mut(&key).unwrap()[index(pos)] = state;
        Some(old)
    }

    fn in_bounds(&self, pos: BlockPos) -> bool {
        pos.y >= self.min_y && pos.y < self.min_y + self.height && pos.x.abs() < 30_000_000 && pos.z.abs() < 30_000_000
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn next_sub_tick(&mut self) -> i64 {
        self.sub_tick += 1;
        self.sub_tick - 1
    }

    fn block_ticks(&mut self) -> &mut LevelTicks<BlockId> {
        &mut self.block_ticks
    }

    fn fluid_ticks(&mut self) -> &mut LevelTicks<FluidType> {
        &mut self.fluid_ticks
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn data(&mut self) -> &mut LevelData {
        &mut self.data
    }

    fn rules(&self) -> &Rules {
        &self.rules
    }

    fn effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    fn comparator_output(&self, pos: BlockPos) -> i32 {
        self.comparator_outputs.get(&pos).copied().unwrap_or(0)
    }

    fn set_comparator_output(&mut self, pos: BlockPos, value: i32) {
        self.comparator_outputs.insert(pos, value);
    }

    fn trace_update(&mut self, update: UpdateTrace) {
        if let Some(t) = &mut self.trace {
            t.push(update);
        }
    }

    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn height(&self) -> i32 {
        self.height
    }

    fn weather(&self) -> crate::weather::Weather {
        self.weather
    }

    fn motion_blocking_height(&self, x: i32, z: i32) -> i32 {
        let top_section = self.sections.keys().filter(|k| k.0 == x >> 4 && k.2 == z >> 4).map(|k| k.1 * 16 + 15).max();
        let top_layer = self.min_y + self.layers.len() as i32 - 1;
        let start = top_section.unwrap_or(i32::MIN).max(top_layer).min(self.min_y + self.height - 1);
        for y in (self.min_y..=start).rev() {
            if kiln_data::block_props::motion_blocking(self.block(BlockPos::new(x, y, z))) {
                return y + 1;
            }
        }
        self.min_y
    }

    fn climate(&self, biome_pos: BlockPos, pos: BlockPos) -> Option<crate::weather::Climate> {
        self.climate.as_ref().and_then(|f| f(biome_pos, pos))
    }
}
