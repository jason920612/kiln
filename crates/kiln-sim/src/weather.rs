//! Weather and the day cycle.
//!
//! Vanilla 26.3 keeps the weather counters server-wide (`WeatherData`, saved as
//! `data/minecraft/weather.dat`) and each level its own rain and thunder levels, which move
//! 0.01 per tick toward the counters' state (`ServerLevel.advanceWeatherCycle`). Only levels
//! with sky light, no ceiling and not the End have weather, so in practice the overworld runs
//! the cycle and the nether and End stay dry. The rain and thunder levels darken the sky
//! (`minecraft:gameplay/sky_light_level`), which decides when monsters spawn and burn and
//! when players may sleep.
//!
//! Randomness: the cycle and `/weather` without a duration draw from a server-wide stand-in
//! for the overworld's level random (seeded from the world seed), in vanilla's order.

use crate::blocks::BlockEnv;
use crate::{OVERWORLD_ID, Sim};
use bytes::Bytes;
use kiln_blocks::BlockPos;
use kiln_blocks::weather::Climate;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_region::CellSet;
use kiln_world::{Blocks, Cell, ChunkPos};
use std::collections::HashMap;
use std::sync::Arc;

/// Saved data id of the weather counters.
const WEATHER: &str = "weather";

/// `ClientboundGameEventPacket` types.
pub(crate) const STOP_RAINING: u8 = 1;
pub(crate) const START_RAINING: u8 = 2;
pub(crate) const RAIN_LEVEL_CHANGE: u8 = 7;
pub(crate) const THUNDER_LEVEL_CHANGE: u8 = 8;

/// `ServerLevel.RAIN_DELAY`, `RAIN_DURATION`, `THUNDER_DELAY`, `THUNDER_DURATION`
/// (`UniformInt`, inclusive).
pub(crate) const RAIN_DELAY: (i32, i32) = (12000, 180000);
pub(crate) const RAIN_DURATION: (i32, i32) = (12000, 24000);
pub(crate) const THUNDER_DELAY: (i32, i32) = (12000, 180000);
pub(crate) const THUNDER_DURATION: (i32, i32) = (3600, 15600);

/// `UniformInt.sample`: `nextIntBetweenInclusive`.
pub(crate) fn sample(r: &mut impl RandomSource, (min, max): (i32, i32)) -> i32 {
    r.next_int_bounded(max - min + 1) + min
}

/// `Mth.lerp`.
fn lerp(delta: f32, a: f32, b: f32) -> f32 {
    a + delta * (b - a)
}

/// `WeatherData`: the server-wide weather counters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WeatherData {
    pub clear_weather_time: i32,
    pub rain_time: i32,
    pub thunder_time: i32,
    pub raining: bool,
    pub thundering: bool,
}

impl WeatherData {
    pub fn from_nbt(tag: &Tag) -> Self {
        let int = |k: &str| tag.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
        let flag = |k: &str| tag.get(k).and_then(Tag::as_i64).is_some_and(|v| v != 0);
        Self {
            clear_weather_time: int("clear_weather_time"),
            rain_time: int("rain_time"),
            thunder_time: int("thunder_time"),
            raining: flag("raining"),
            thundering: flag("thundering"),
        }
    }

    pub fn to_nbt(&self) -> Tag {
        Tag::Compound(vec![
            ("clear_weather_time".into(), Tag::Int(self.clear_weather_time)),
            ("rain_time".into(), Tag::Int(self.rain_time)),
            ("thunder_time".into(), Tag::Int(self.thunder_time)),
            ("raining".into(), Tag::Byte(self.raining as i8)),
            ("thundering".into(), Tag::Byte(self.thundering as i8)),
        ])
    }

    /// `MinecraftServer.setWeatherParameters`.
    pub fn set(&mut self, clear_time: i32, weather_time: i32, raining: bool, thundering: bool) {
        self.clear_weather_time = clear_time;
        self.rain_time = weather_time;
        self.thunder_time = weather_time;
        self.raining = raining;
        self.thundering = thundering;
    }

    /// `ServerLevel.resetWeatherCycle`.
    pub fn reset_cycle(&mut self) {
        self.rain_time = 0;
        self.raining = false;
        self.thunder_time = 0;
        self.thundering = false;
    }

    /// The counters part of `advanceWeatherCycle` (with `minecraft:advance_weather` on).
    pub fn advance(&mut self, r: &mut impl RandomSource) {
        let mut clear = self.clear_weather_time;
        let mut thunder_time = self.thunder_time;
        let mut rain_time = self.rain_time;
        let mut thundering = self.thundering;
        let mut raining = self.raining;
        if clear > 0 {
            clear -= 1;
            thunder_time = if thundering { 0 } else { 1 };
            rain_time = if raining { 0 } else { 1 };
            thundering = false;
            raining = false;
        } else {
            if thunder_time > 0 {
                thunder_time -= 1;
                if thunder_time == 0 {
                    thundering = !thundering;
                }
            } else if thundering {
                thunder_time = sample(r, THUNDER_DURATION);
            } else {
                thunder_time = sample(r, THUNDER_DELAY);
            }
            if rain_time > 0 {
                rain_time -= 1;
                if rain_time == 0 {
                    raining = !raining;
                }
            } else if raining {
                rain_time = sample(r, RAIN_DURATION);
            } else {
                rain_time = sample(r, RAIN_DELAY);
            }
        }
        self.thunder_time = thunder_time;
        self.rain_time = rain_time;
        self.clear_weather_time = clear;
        self.thundering = thundering;
        self.raining = raining;
    }
}

/// A level's rain and thunder levels (`Level.rainLevel`, `oRainLevel`, ...).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LevelWeather {
    pub rain: f32,
    pub o_rain: f32,
    pub thunder: f32,
    pub o_thunder: f32,
}

impl LevelWeather {
    /// `ServerLevel.prepareWeather`: a loaded level starts at the saved state.
    pub fn prepared(data: &WeatherData) -> Self {
        let mut w = Self::default();
        if data.raining {
            w.rain = 1.0;
            if data.thundering {
                w.thunder = 1.0;
            }
        }
        w
    }

    /// `getRainLevel(1)`.
    pub fn rain_level(&self) -> f32 {
        lerp(1.0, self.o_rain, self.rain)
    }

    /// `getThunderLevel(1)`: scaled by the rain level.
    pub fn thunder_level(&self) -> f32 {
        lerp(1.0, self.o_thunder, self.thunder) * self.rain_level()
    }

    /// `Level.isRaining` for a level that can have weather.
    pub fn is_raining(&self) -> bool {
        self.rain_level() as f64 > 0.2
    }

    /// `Level.isThundering` for a level that can have weather.
    pub fn is_thundering(&self) -> bool {
        self.thunder_level() as f64 > 0.9
    }

    /// The level part of `advanceWeatherCycle`: both levels step 0.01 toward the counters.
    pub fn step(&mut self, data: &WeatherData) {
        self.o_thunder = self.thunder;
        self.thunder = (self.thunder + if data.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        self.o_rain = self.rain;
        self.rain = (self.rain + if data.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    }
}

/// `Level.canHaveWeather`: sky light, no ceiling, not the End.
pub(crate) fn can_have_weather(dim: crate::DimId) -> bool {
    dim == OVERWORLD_ID
}

/// The overworld `day` timeline's `minecraft:gameplay/sky_light_level` multiplier: 1 by day,
/// 4/15 at night, linear between the keyframes (the timeline repeats every 24000 ticks).
fn sky_light_factor(day_time: i64) -> f32 {
    const KEYS: [(i64, f32); 4] = [(133, 1.0), (11867, 1.0), (13670, 0.26666668), (22330, 0.26666668)];
    let t = day_time.rem_euclid(24000);
    for w in 0..KEYS.len() {
        let (a, b) = (KEYS[w], KEYS[(w + 1) % KEYS.len()]);
        let bt = if b.0 <= a.0 { b.0 + 24000 } else { b.0 };
        let tt = if t < a.0 { t + 24000 } else { t };
        if tt >= a.0 && tt <= bt {
            let f = (tt - a.0) as f32 / (bt - a.0) as f32;
            return a.1 + f * (b.1 - a.1);
        }
    }
    1.0
}

/// The `minecraft:gameplay/sky_light_level` attribute of the overworld: 15 scaled by the day
/// timeline, then the weather layer (`WeatherAttributes`): rain blends toward 4 with alpha
/// 0.3125 by the rain level less the thunder level, thunder toward 4 with alpha 0.52734375
/// by the thunder level.
pub(crate) fn sky_light_level(day_time: i64, weather: &LevelWeather) -> f32 {
    let mut v = 15.0 * sky_light_factor(day_time);
    let thunder = weather.thunder_level();
    let rain = weather.rain_level() - thunder;
    if rain > 0.0 {
        v = lerp(rain, v, lerp(0.3125, v, 4.0));
    }
    if thunder > 0.0 {
        v = lerp(thunder, v, lerp(0.527_343_75, v, 4.0));
    }
    v
}

/// `Level.updateSkyBrightness` for level `dim`: `15 - sky light level`; 0 without a clock.
pub(crate) fn sky_darken(dim: crate::DimId, day_time: i64, weather: &LevelWeather) -> i32 {
    if dim != OVERWORLD_ID {
        // No day timeline modifies their sky light level (and the nether has no sky light).
        return 0;
    }
    (15.0 - sky_light_level(day_time, weather)) as i32
}

/// Biome climates by biome network id, from the datapack's `worldgen/biome`.
#[derive(Default)]
pub(crate) struct Climates {
    /// (base temperature, frozen modifier, has precipitation).
    biomes: HashMap<u16, (f32, bool, bool)>,
}

impl Climates {
    pub fn load(dir: &std::path::Path) -> Option<Self> {
        let biome_dir = dir.join("data/minecraft/worldgen/biome");
        let mut c = Climates::default();
        for entry in std::fs::read_dir(&biome_dir).ok()? {
            let path = entry.ok()?.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Some(id) = kiln_data::synced_id("minecraft:worldgen/biome", &format!("minecraft:{name}")) else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let temperature = json.get("temperature").and_then(|v| v.as_f64()).unwrap_or(0.8) as f32;
            let frozen = json.get("temperature_modifier").and_then(|v| v.as_str()) == Some("frozen");
            let precipitation = json.get("has_precipitation").and_then(|v| v.as_bool()).unwrap_or(false);
            c.biomes.insert(id as u16, (temperature, frozen, precipitation));
        }
        Some(c)
    }

    /// The climate of biome `biome` read at `pos`.
    pub fn climate(&self, biome: u16, sea_level: i32, pos: BlockPos) -> Option<Climate> {
        let &(base, frozen, has_precipitation) = self.biomes.get(&biome)?;
        let temperature = kiln_worldgen::simplex::biome_temperature(base, frozen, sea_level, pos.x, pos.y, pos.z);
        Some(Climate { temperature, has_precipitation })
    }
}

/// What precipitation and `isRainingAt` read in a region: the level's weather and the biome
/// data.
#[derive(Clone)]
pub(crate) struct WeatherEnv {
    pub weather: kiln_blocks::weather::Weather,
    pub climates: Option<Arc<Climates>>,
    /// `BiomeManager` seed (the obfuscated world seed).
    pub zoom_seed: i64,
    pub sea_level: i32,
}

impl Default for WeatherEnv {
    fn default() -> Self {
        Self { weather: Default::default(), climates: None, zoom_seed: 0, sea_level: 63 }
    }
}

/// The stored biome of quart `(qx, qy, qz)` (`getNoiseBiome`, y clamped to the chunk), if its
/// chunk is loaded.
fn noise_biome(cells: &CellSet<Cell>, env: &BlockEnv, qx: i32, qy: i32, qz: i32) -> Option<u16> {
    let chunk = cells.chunk(ChunkPos::new(qx >> 2, qz >> 2))?;
    let min_q = env.min_y >> 2;
    let rel = (qy - min_q).clamp(0, (env.height >> 2) - 1);
    let section = chunk.sections.get((rel >> 2) as usize)?;
    Some(match &section.biomes {
        kiln_world::section::Biomes::Single(b) => *b,
        kiln_world::section::Biomes::Cells(cells) => cells[(((rel & 3) as usize) << 4) | (((qz & 3) as usize) << 2) | (qx & 3) as usize],
    })
}

/// `Level.getBiome(pos)`: the voronoi-zoomed stored biome.
pub(crate) fn biome_at(cells: &CellSet<Cell>, env: &BlockEnv, pos: BlockPos) -> u16 {
    let fallback = noise_biome(cells, env, pos.x >> 2, pos.y >> 2, pos.z >> 2).unwrap_or(0);
    kiln_worldgen::generator::zoomed_biome(env.weather.zoom_seed, pos.x, pos.y, pos.z, &mut |qx, qy, qz| {
        noise_biome(cells, env, qx, qy, qz).unwrap_or(fallback)
    })
}

/// The climate of the biome at `biome_pos`, read at `pos`.
pub(crate) fn climate(cells: &CellSet<Cell>, env: &BlockEnv, biome_pos: BlockPos, pos: BlockPos) -> Option<Climate> {
    let climates = env.weather.climates.as_ref()?;
    climates.climate(biome_at(cells, env, biome_pos), env.weather.sea_level, pos)
}

/// The `MOTION_BLOCKING` heightmap at `(x, z)`.
pub(crate) fn motion_blocking_height(cells: &CellSet<Cell>, env: &BlockEnv, x: i32, z: i32) -> i32 {
    let Some(chunk) = cells.chunk(ChunkPos::of_block(x, z)) else { return env.min_y };
    chunk.column_height((x & 15) as usize, (z & 15) as usize, kiln_data::block_props::motion_blocking)
}

/// `Level.canSeeSky`: full sky light.
pub(crate) fn can_see_sky(cells: &CellSet<Cell>, env: &BlockEnv, pos: BlockPos) -> bool {
    let top = env.min_y + env.height;
    cells.light_at(kiln_world::chunk::LightLayer::Sky, pos.x, pos.y, pos.z).map_or(pos.y >= top, |s| s >= 15)
}

/// `Level.isRainingAt`: raining, under open sky, at or above the rain heightmap, in a biome
/// where it rains (not snows).
pub(crate) fn is_raining_at(cells: &CellSet<Cell>, env: &BlockEnv, pos: BlockPos) -> bool {
    env.weather.weather.raining
        && can_see_sky(cells, env, pos)
        && motion_blocking_height(cells, env, pos.x, pos.z) <= pos.y
        && climate(cells, env, pos, pos).is_some_and(|c| c.precipitation() == kiln_blocks::weather::Precipitation::Rain)
}

/// `Entity.isInRain` for a box standing at `pos` with its top at `top_y`.
pub(crate) fn in_rain(cells: &CellSet<Cell>, env: &BlockEnv, pos: [f64; 3], top_y: f64) -> bool {
    if !env.weather.weather.raining {
        return false;
    }
    let at = BlockPos::new(pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32);
    is_raining_at(cells, env, at) || is_raining_at(cells, env, BlockPos::new(at.x, top_y.floor() as i32, at.z))
}

impl Sim {
    /// Loads the weather counters (`weather.dat`) and prepares each level's weather.
    pub(crate) fn load_weather(&mut self) {
        let data = self.storage.as_ref().and_then(|s| kiln_storage::saved_data::read(&s.dir, WEATHER));
        if let Some(data) = data {
            self.weather = WeatherData::from_nbt(&data);
        }
        for (dim, w) in self.level_weather.iter_mut().enumerate() {
            *w = if can_have_weather(dim) { LevelWeather::prepared(&self.weather) } else { LevelWeather::default() };
        }
    }

    pub(crate) fn save_weather(&mut self) {
        let Some(storage) = &self.storage else { return };
        if let Err(e) = kiln_storage::saved_data::write(&storage.dir, WEATHER, self.weather.to_nbt()) {
            tracing::warn!("failed to save the weather: {e}");
        }
    }

    /// `ServerLevel.advanceWeatherCycle` for every level, with its packets.
    pub(crate) fn tick_weather(&mut self) {
        let advance = self.rule_bool("minecraft:advance_weather");
        for dim in 0..self.level_weather.len() {
            let was_raining = self.is_raining(dim);
            if can_have_weather(dim) {
                if advance {
                    self.weather.advance(&mut self.weather_random);
                }
                let data = self.weather.clone();
                self.level_weather[dim].step(&data);
            }
            let w = self.level_weather[dim];
            if w.o_rain != w.rain {
                self.broadcast_in(dim, packets::game_event(RAIN_LEVEL_CHANGE, w.rain));
            }
            if w.o_thunder != w.thunder {
                self.broadcast_in(dim, packets::game_event(THUNDER_LEVEL_CHANGE, w.thunder));
            }
            // Vanilla sends the start and stop of rain, with the levels, to every player.
            if was_raining != self.is_raining(dim) {
                self.broadcast(packets::game_event(if was_raining { STOP_RAINING } else { START_RAINING }, 0.0));
                self.broadcast(packets::game_event(RAIN_LEVEL_CHANGE, w.rain));
                self.broadcast(packets::game_event(THUNDER_LEVEL_CHANGE, w.thunder));
            }
        }
    }

    /// `Level.isRaining` of level `dim`.
    pub(crate) fn is_raining(&self, dim: crate::DimId) -> bool {
        can_have_weather(dim) && self.level_weather[dim].is_raining()
    }

    /// `Level.isThundering` of level `dim`.
    pub(crate) fn is_thundering(&self, dim: crate::DimId) -> bool {
        can_have_weather(dim) && self.level_weather[dim].is_thundering()
    }

    /// The weather part of `PlayerList.sendLevelInfo` for a player in level `dim`.
    pub(crate) fn weather_packets(&self, dim: crate::DimId) -> Vec<Bytes> {
        if !self.is_raining(dim) {
            return Vec::new();
        }
        let w = &self.level_weather[dim];
        vec![
            packets::game_event(START_RAINING, 0.0),
            packets::game_event(RAIN_LEVEL_CHANGE, w.rain_level()),
            packets::game_event(THUNDER_LEVEL_CHANGE, w.thunder_level()),
        ]
    }

    /// `/weather`: `MinecraftServer.setWeatherParameters` with the given or a random duration
    /// (`WeatherCommand.getDuration`); returns the duration.
    pub(crate) fn command_weather(&mut self, weather: kiln_command::Weather, duration: Option<i32>) -> i32 {
        use kiln_command::Weather as W;
        let range = match weather {
            W::Clear => RAIN_DELAY,
            W::Rain => RAIN_DURATION,
            W::Thunder => THUNDER_DURATION,
        };
        let d = match duration {
            Some(d) if d != -1 => d,
            _ => sample(&mut self.weather_random, range),
        };
        match weather {
            W::Clear => self.weather.set(d, 0, false, false),
            W::Rain => self.weather.set(0, d, true, false),
            W::Thunder => self.weather.set(0, d, true, true),
        }
        d
    }

    /// Sends `pkt` to every player in level `dim` (`PlayerList.broadcastAll(packet, dimension)`).
    pub(crate) fn broadcast_in(&mut self, dim: crate::DimId, pkt: Bytes) {
        for p in self.players.values_mut().filter(|p| p.dim == dim) {
            p.send(pkt.clone());
        }
    }
}

/// The running state of a world clock besides its total ticks (`ServerClockInstance`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClockRun {
    pub partial: f32,
    pub rate: f32,
    pub paused: bool,
}

impl Default for ClockRun {
    fn default() -> Self {
        Self { partial: 0.0, rate: 1.0, paused: false }
    }
}

impl ClockRun {
    /// `ServerClockInstance.tick`: the whole ticks to add.
    pub fn tick(&mut self) -> i64 {
        if self.paused {
            return 0;
        }
        self.partial += self.rate;
        let n = kiln_javamath::math::floor_f32(self.partial);
        self.partial -= n as f32;
        n as i64
    }
}

/// Clocks Kiln runs, by `minecraft:world_clock` id.
const CLOCKS: [&str; 2] = ["minecraft:overworld", "minecraft:the_end"];

/// The time markers of a clock (from its timelines): (id, ticks within the 24000-tick day).
pub(crate) fn time_markers(clock: &str) -> &'static [(&'static str, i64)] {
    if clock == CLOCKS[0] {
        &[
            ("minecraft:day", 1000),
            ("minecraft:midnight", 18000),
            ("minecraft:night", 13000),
            ("minecraft:noon", 6000),
            ("minecraft:roll_village_siege", 18000),
            ("minecraft:wake_up_from_sleep", 0),
        ]
    } else {
        &[]
    }
}

/// The timelines of a clock and their periods (`None`: not periodic).
pub(crate) fn timelines(clock: &str) -> &'static [(&'static str, Option<i64>)] {
    if clock == CLOCKS[0] {
        &[
            ("minecraft:day", Some(24000)),
            ("minecraft:early_game", None),
            ("minecraft:moon", Some(192000)),
            ("minecraft:villager_schedule", Some(24000)),
        ]
    } else {
        &[]
    }
}

/// `ClockTimeMarker.occursAt` and `resolveTimeToMoveTo` for a marker of the 24000-tick day.
pub(crate) fn marker_move(total: i64, ticks: i64) -> Option<i64> {
    const PERIOD: i64 = 24000;
    if total % PERIOD == ticks {
        return None;
    }
    let d = ticks - total % PERIOD;
    Some(total + if d > 0 { d } else { PERIOD + d })
}

/// `TimeCommand.wrapTime`.
fn wrap_time(t: i64) -> i32 {
    (t % i32::MAX as i64) as i32
}

impl Sim {
    /// Advances the world clocks (`ServerClockManager.tick`, with `minecraft:advance_time`).
    pub(crate) fn tick_clocks(&mut self) {
        if !self.rule_bool("minecraft:advance_time") {
            return;
        }
        self.day_time += self.clock_runs[0].tick();
        self.end_time += self.clock_runs[1].tick();
    }

    fn clock_index(&self, clock: Option<&str>) -> Result<usize, kiln_command::CommandError> {
        use kiln_command::{CommandError, tr};
        let id = match clock {
            Some(c) => c.to_owned(),
            None => {
                let dim = crate::dim_id(kiln_command::host::Source::dimension(self)).unwrap_or(OVERWORLD_ID);
                match dim {
                    OVERWORLD_ID => CLOCKS[0].to_owned(),
                    crate::END_ID => CLOCKS[1].to_owned(),
                    _ => return Err(CommandError::new(tr!("commands.time.no_default_clock", crate::DIMENSIONS[dim].0))),
                }
            }
        };
        CLOCKS.iter().position(|c| *c == id).ok_or_else(|| CommandError::new(tr!("commands.time.no_default_clock", id)))
    }

    fn clock_total(&mut self, i: usize) -> &mut i64 {
        if i == 0 { &mut self.day_time } else { &mut self.end_time }
    }

    /// `/time` (`TimeCommand`) on a clock (the source level's default clock when `None`).
    pub(crate) fn command_time(&mut self, clock: Option<&str>, action: &kiln_command::TimeAction) -> Result<i32, kiln_command::CommandError> {
        use kiln_command::{CommandError, TimeAction as A, host::Host, tr};
        if let A::QueryGameTime = action {
            let t = self.game_time;
            self.send_success(tr!("commands.time.query.gametime", t), false);
            return Ok(wrap_time(t));
        }
        let i = self.clock_index(clock)?;
        let name = CLOCKS[i];
        let result = match action {
            A::QueryGameTime => unreachable!(),
            A::Set(t) => {
                if *self.clock_total(i) == *t as i64 {
                    return Err(CommandError::new(tr!("commands.time.set.already_at_time", name, *t)));
                }
                *self.clock_total(i) = *t as i64;
                self.send_success(tr!("commands.time.set.absolute", name, *t), true);
                *t
            }
            A::Add(t) => {
                *self.clock_total(i) += *t as i64;
                let total = *self.clock_total(i);
                self.send_success(tr!("commands.time.set.absolute", name, total), true);
                wrap_time(total)
            }
            A::SetMarker(m) => {
                let Some(&(_, ticks)) = time_markers(name).iter().find(|(id, _)| *id == m.as_str()) else {
                    return Err(CommandError::new(tr!("commands.time.no_time_marker_found", name, m.as_str())));
                };
                let total = *self.clock_total(i);
                let Some(to) = marker_move(total, ticks) else {
                    return Err(CommandError::new(tr!("commands.time.set.already_at_time_marker", name, m.as_str())));
                };
                *self.clock_total(i) = to;
                self.clock_runs[i].partial = 0.0;
                self.send_success(tr!("commands.time.set.time_marker", name, m.as_str()), true);
                wrap_time(to)
            }
            A::QueryTime => {
                let total = *self.clock_total(i);
                self.send_success(tr!("commands.time.query.absolute", name, total), false);
                wrap_time(total)
            }
            A::QueryTimeline { timeline, repetitions } => {
                let Some(&(_, period)) = timelines(name).iter().find(|(id, _)| *id == timeline.as_str()) else {
                    return Err(CommandError::new(tr!("commands.time.wrong_timeline_for_clock", timeline.as_str(), name)));
                };
                let total = *self.clock_total(i);
                let (key, v) = match (repetitions, period) {
                    (false, Some(p)) => ("commands.time.query.timeline", total.rem_euclid(p)),
                    (false, None) => ("commands.time.query.timeline", total),
                    (true, Some(p)) => ("commands.time.query.timeline.repetitions", total.div_euclid(p)),
                    (true, None) => ("commands.time.query.timeline.repetitions", 0),
                };
                self.send_success(tr!(key, timeline.as_str(), v), false);
                wrap_time(v)
            }
            A::Pause | A::Resume => {
                let pause = matches!(action, A::Pause);
                if self.clock_runs[i].paused == pause {
                    let key = if pause { "commands.time.pause.already_paused" } else { "commands.time.pause.already_running" };
                    return Err(CommandError::new(tr!(key, name)));
                }
                self.clock_runs[i].paused = pause;
                self.send_success(tr!(if pause { "commands.time.pause" } else { "commands.time.resume" }, name), true);
                1
            }
            A::Rate(r) => {
                if self.clock_runs[i].rate == *r {
                    return Err(CommandError::new(tr!("commands.time.rate.already_same", name, *r)));
                }
                self.clock_runs[i].rate = *r;
                self.send_success(tr!("commands.time.rate", name, *r), true);
                1
            }
        };
        let pkt = self.time_packet();
        self.broadcast(pkt);
        Ok(result)
    }

    /// `ServerClockManager.createFullSyncPacket`: every clock, whatever the player's level;
    /// clocks do not run (rate 0) while paused or with `minecraft:advance_time` off.
    pub(crate) fn time_packet(&self) -> Bytes {
        let advance = self.rule_bool("minecraft:advance_time");
        let state = |i: usize, clock: i32, time: i64| {
            let run = &self.clock_runs[i];
            packets::ClockState { clock, time, fraction: run.partial, rate: if run.paused || !advance { 0.0 } else { run.rate } }
        };
        let mut clocks = [state(0, self.overworld_clock, self.day_time), state(1, self.end_clock, self.end_time)];
        clocks.sort_by_key(|c| c.clock);
        packets::set_time(self.game_time, &clocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_javamath::random::LegacyRandom;

    #[test]
    fn markers_move_to_the_next_occurrence() {
        assert_eq!(marker_move(23999, 0), Some(24000));
        assert_eq!(marker_move(24000, 0), None);
        assert_eq!(marker_move(13000, 1000), Some(25000));
        assert_eq!(marker_move(500, 1000), Some(1000));
    }

    #[test]
    fn nbt_round_trip() {
        let w = WeatherData { clear_weather_time: 5, rain_time: 7, thunder_time: 9, raining: true, thundering: false };
        assert_eq!(WeatherData::from_nbt(&w.to_nbt()), w);
    }

    #[test]
    fn clear_time_holds_the_weather_off() {
        let mut w = WeatherData { clear_weather_time: 2, raining: true, thundering: true, ..Default::default() };
        let mut r = LegacyRandom::new(1);
        w.advance(&mut r);
        assert_eq!((w.clear_weather_time, w.rain_time, w.thunder_time, w.raining, w.thundering), (1, 0, 0, false, false));
        w.advance(&mut r);
        assert_eq!((w.clear_weather_time, w.rain_time, w.thunder_time), (0, 1, 1));
        // Then the counters run out and flip the weather on.
        w.advance(&mut r);
        assert!(w.raining && w.thundering);
    }

    #[test]
    fn levels_step_toward_the_counters() {
        let data = WeatherData { raining: true, ..Default::default() };
        let mut l = LevelWeather::default();
        for _ in 0..21 {
            l.step(&data);
        }
        assert!(l.is_raining() && !l.is_thundering());
        assert!((l.rain - 0.21).abs() < 1e-5);
    }

    #[test]
    fn thunderstorms_darken_the_sky_enough_to_sleep() {
        let clear = LevelWeather::default();
        let rain = LevelWeather { rain: 1.0, o_rain: 1.0, ..Default::default() };
        let thunder = LevelWeather { rain: 1.0, o_rain: 1.0, thunder: 1.0, o_thunder: 1.0 };
        assert_eq!(sky_darken(OVERWORLD_ID, 6000, &clear), 0);
        assert_eq!(sky_darken(OVERWORLD_ID, 6000, &rain), 3);
        assert_eq!(sky_darken(OVERWORLD_ID, 6000, &thunder), 5);
        assert_eq!(sky_darken(OVERWORLD_ID, 18000, &clear), 11);
    }
}
