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
