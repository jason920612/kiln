//! Precipitation in the block phase (`ServerLevel.tickPrecipitation`): water freezing, snow
//! layers piling up and cauldrons filling, from the biome climate the [`Level`] reports.
//!
//! Snow layers that grow under an entity do not push it up (`Block.pushEntitiesUp`): block
//! behaviour has no entities.

use crate::level::Level;
use crate::pos::BlockPos;
use crate::state;
use crate::tags;
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

/// A biome's climate at a position: `Biome.getTemperature(pos, seaLevel)` (height adjusted,
/// with the frozen modifier) and `hasPrecipitation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    pub temperature: f32,
    pub has_precipitation: bool,
}

/// `Biome.Precipitation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
}

impl Climate {
    /// `Biome.warmEnoughToRain`.
    pub fn warm_enough_to_rain(&self) -> bool {
        self.temperature >= 0.15
    }

    /// `Biome.getPrecipitationAt`.
    pub fn precipitation(&self) -> Precipitation {
        if !self.has_precipitation {
            Precipitation::None
        } else if !self.warm_enough_to_rain() {
            Precipitation::Snow
        } else {
            Precipitation::Rain
        }
    }
}

/// Weather and game rules precipitation reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    /// `Level.isRaining`: rain level above 0.2 in a level that has weather.
    pub raining: bool,
    /// `Level.isThundering`.
    pub thundering: bool,
    /// `minecraft:max_snow_accumulation_height`.
    pub max_snow_height: i32,
}

impl Default for Weather {
    /// Clear skies, default rules.
    fn default() -> Self {
        Self { raining: false, thundering: false, max_snow_height: 1 }
    }
}

/// `Level.getHeightmapPos(MOTION_BLOCKING, pos)`.
pub fn surface<L: Level + ?Sized>(level: &L, pos: BlockPos) -> BlockPos {
    BlockPos::new(pos.x, level.motion_blocking_height(pos.x, pos.z), pos.z)
}

fn inside_build_height<L: Level + ?Sized>(level: &L, y: i32) -> bool {
    y >= level.min_y() && y < level.min_y() + level.height()
}

/// `Biome.shouldFreeze(level, pos, false)` with the biome's climate at `pos`.
pub fn should_freeze<L: Level + ?Sized>(level: &L, climate: Climate, pos: BlockPos) -> bool {
    if climate.warm_enough_to_rain() || !inside_build_height(level, pos.y) || level.block_light(pos) >= 10 {
        return false;
    }
    let s = level.block(pos);
    let f = logic::fluid(s);
    f.kind == FluidKind::Water && f.source && logic::block_class(s) == BlockClass::LiquidBlock
}

/// `Biome.shouldSnow` with the biome's climate at `pos`.
pub fn should_snow<L: Level + ?Sized>(level: &L, climate: Climate, pos: BlockPos) -> bool {
    if climate.precipitation() != Precipitation::Snow || !inside_build_height(level, pos.y) || level.block_light(pos) >= 10 {
        return false;
    }
    let s = level.block(pos);
    (kiln_data::blocks_types::is_air(s) || state::same_block(s, d::SNOW)) && snow_can_survive(level, pos)
}

/// `SnowLayerBlock.canSurvive`.
pub fn snow_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let below = level.block(pos.below());
    if tags::is(below, "minecraft:cannot_support_snow_layer") {
        return false;
    }
    if tags::is(below, "minecraft:support_override_snow_layer") {
        return true;
    }
    top_face_full(below) || (state::same_block(below, d::SNOW) && state::get_int(below, "layers") == 8)
}

/// `Block.isFaceFull(collisionShape, UP)`, for shapes whose top face is one box.
fn top_face_full(s: u16) -> bool {
    block_props::collision(s).iter().any(|b| b[0] <= 0.0 && b[2] <= 0.0 && b[3] >= 1.0 && b[4] >= 1.0 && b[5] >= 1.0)
}

/// `ServerLevel.tickPrecipitation(pos)`, `pos` from `getBlockRandomPos` at y 0.
pub fn tick_precipitation<L: Level>(level: &mut L, pos: BlockPos) {
    let top = surface(level, pos);
    let below = top.below();
    let weather = level.weather();
    // The biome at the surface, with its temperature where it is read.
    if let Some(c) = level.climate(top, below)
        && should_freeze(level, c, below)
    {
        crate::set_block(level, below, d::ICE, crate::flags::ALL);
    }
    if !weather.raining {
        return;
    }
    let max = weather.max_snow_height;
    if max > 0
        && let Some(c) = level.climate(top, top)
        && should_snow(level, c, top)
    {
        let s = level.block(top);
        if state::same_block(s, d::SNOW) {
            let layers = state::get_int(s, "layers");
            if layers < max.min(8) {
                crate::set_block(level, top, state::set_int(s, "layers", layers + 1), crate::flags::ALL);
            }
        } else {
            crate::set_block(level, top, d::SNOW, crate::flags::ALL);
        }
    }
    let Some(c) = level.climate(top, below) else { return };
    let precipitation = c.precipitation();
    if precipitation != Precipitation::None {
        let s = level.block(below);
        handle_precipitation(level, s, below, precipitation);
    }
}

/// `CauldronBlock.shouldHandlePrecipitation`: 5% of rain, 10% of snow.
fn should_handle<L: Level>(level: &mut L, p: Precipitation) -> bool {
    match p {
        Precipitation::Rain => level.random().next_float() < 0.05,
        Precipitation::Snow => level.random().next_float() < 0.1,
        Precipitation::None => false,
    }
}

/// `Block.handlePrecipitation`: empty cauldrons take water or powder snow, and water or powder
/// snow cauldrons fill up under their own kind of weather.
pub fn handle_precipitation<L: Level>(level: &mut L, s: u16, pos: BlockPos, p: Precipitation) {
    match logic::block_class(s) {
        BlockClass::CauldronBlock => {
            if !should_handle(level, p) {
                return;
            }
            let filled = if p == Precipitation::Rain { d::WATER_CAULDRON } else { d::POWDER_SNOW_CAULDRON };
            crate::set_block(level, pos, filled, crate::flags::ALL);
            level.effect(crate::Effect::GameEvent { pos, event: "minecraft:block_change" });
        }
        BlockClass::LayeredCauldronBlock => {
            let kind = if state::same_block(s, d::POWDER_SNOW_CAULDRON) { Precipitation::Snow } else { Precipitation::Rain };
            if !should_handle(level, p) || state::get_int(s, "level") == 3 || p != kind {
                return;
            }
            let next = state::set_int(s, "level", state::get_int(s, "level") + 1);
            crate::set_block(level, pos, next, crate::flags::ALL);
            level.effect(crate::Effect::GameEvent { pos, event: "minecraft:block_change" });
        }
        _ => {}
    }
}

/// What a lightning bolt does to the block it strikes (`LightningBolt.powerLightningRod`, then
/// `clearCopperOnLightningStrike`).
pub fn lightning_strike<L: Level>(level: &mut L, pos: BlockPos) {
    power_rod(level, pos);
    clear_copper(level, pos);
}

/// The block name one oxidation stage back (`WeatheringCopper.getPrevious`), or all the way
/// back (`getFirst`).
fn copper_stage(name: &str, first: bool) -> Option<String> {
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    let (prefix, rest) = ["exposed_", "weathered_", "oxidized_"].iter().find_map(|p| path.strip_prefix(p).map(|r| (*p, r)))?;
    let back = if first { "" } else { match prefix { "oxidized_" => "weathered_", "weathered_" => "exposed_", _ => "" } };
    // `exposed_copper` goes back to `copper_block`.
    let rest = if back.is_empty() && rest == "copper" { "copper_block" } else { rest };
    Some(format!("minecraft:{back}{rest}"))
}

/// `state` as the block `name`, keeping the properties both have.
fn with_block(state: u16, name: &str) -> Option<u16> {
    let target = state::BlockId::by_name(name)?.default_state();
    let from = &kiln_data::blocks::BLOCKS[logic::block_index(state)];
    Some(from.properties.iter().fold(target, |s, p| state::get(state, p.name).map_or(s, |v| state::set(s, p.name, v))))
}

fn is_weathering(s: u16) -> bool {
    logic::implements(s, kiln_data::block_logic::interface::WEATHERING_COPPER)
}

/// `LightningBolt.clearCopperOnLightningStrike`: struck weathering copper goes back to its
/// first stage (waxed copper stays), then three to five random walks of 1 to 8 steps each take
/// oxidation off the copper they step on, with the scrape particles.
fn clear_copper<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    let name = kiln_data::blocks::BLOCKS[logic::block_index(s)].name;
    let waxed = name.contains("waxed_") && !name.contains("unwaxed");
    let weathering = is_weathering(s);
    if !weathering && !waxed {
        return;
    }
    level.reseed_random(pos);
    if weathering
        && let Some(first) = copper_stage(name, true).and_then(|n| with_block(s, &n))
    {
        crate::set_block_and_update(level, pos, first);
    }
    let walks = level.random().next_int_bounded(3) + 3;
    for _ in 0..walks {
        let steps = level.random().next_int_bounded(8) + 1;
        let mut at = pos;
        for _ in 0..steps {
            match random_step_cleaning_copper(level, at) {
                Some(next) => at = next,
                None => break,
            }
        }
    }
}

/// `randomStepCleaningCopper`: up to 10 random blocks of the cube around `pos`; the first
/// weathering copper one loses a stage of oxidation.
fn random_step_cleaning_copper<L: Level>(level: &mut L, pos: BlockPos) -> Option<BlockPos> {
    for _ in 0..10 {
        let dx = level.random().next_int_bounded(3);
        let dy = level.random().next_int_bounded(3);
        let dz = level.random().next_int_bounded(3);
        let at = BlockPos::new(pos.x - 1 + dx, pos.y - 1 + dy, pos.z - 1 + dz);
        let s = level.block(at);
        if is_weathering(s) {
            let name = kiln_data::blocks::BLOCKS[logic::block_index(s)].name;
            if let Some(prev) = copper_stage(name, false).and_then(|n| with_block(s, &n)) {
                crate::set_block_and_update(level, at, prev);
            }
            level.effect(crate::Effect::LevelEvent { id: 3002, pos: at, data: -1 });
            return Some(at);
        }
    }
    None
}

/// `LightningRodBlock.onLightningStrike`: powered for 8 ticks, with the spark particles.
fn power_rod<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    if !logic::is_instance(s, BlockClass::LightningRodBlock) {
        return;
    }
    crate::set_block(level, pos, state::set_bool(s, "powered", true), crate::flags::ALL);
    rod_neighbours(level, s, pos);
    crate::schedule_block_tick(level, pos, crate::BlockId::of(s), 8, crate::TickPriority::Normal);
    let axis = state::get_dir(s, "facing").map_or(1, |f| f.axis() as i32);
    level.effect(crate::Effect::LevelEvent { id: 3002, pos, data: axis });
}

fn rod_neighbours<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let behind = state::get_dir(s, "facing").unwrap_or(crate::pos::Direction::Up).opposite();
    crate::update::update_neighbors_at(level, pos.relative(behind), crate::BlockId::of(s));
}

/// `LightningRodBlock.tick`: the power goes off.
pub fn rod_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    crate::set_block(level, pos, state::set_bool(s, "powered", false), crate::flags::ALL);
    rod_neighbours(level, s, pos);
}
