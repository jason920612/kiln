//! Target blocks (`TargetBlock`), big dripleaves (`BigDripleafBlock`) and copper bulbs
//! (`CopperBulbBlock`).

use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{set_block, set_block_and_update};
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

// ---------------------------------------------------------------------- target

/// `TargetBlock.onPlace`: a powered target placed anew with no reset pending goes out (quietly).
pub fn target_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if state::same_block(old, s) {
        return;
    }
    if state::get_int(s, "power") > 0 && !level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)) {
        set_block(level, pos, state::set_int(s, "power", 0), flags::CLIENTS | flags::KNOWN_SHAPE);
    }
}

/// `TargetBlock.tick`: the reset after a hit.
pub fn target_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_int(s, "power") != 0 {
        set_block_and_update(level, pos, state::set_int(s, "power", 0));
    }
}

// ---------------------------------------------------------------------- big dripleaf

/// `BigDripleafBlock.canSurvive`: on a dripleaf, its stem or something `#supports_big_dripleaf`.
pub fn dripleaf_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let below = level.block(pos.below());
    state::same_block(below, s) || state::is(below, d::BIG_DRIPLEAF_STEM) || tags::is(below, "minecraft:supports_big_dripleaf")
}

/// `BigDripleafBlock.updateShape`: it goes with its support (before its water is looked after); a
/// dripleaf above turns it into stem.
pub fn dripleaf_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    if dir == Direction::Down && !dripleaf_can_survive(level, s, pos) {
        return d::AIR;
    }
    crate::fluid::tick_water_if_waterlogged(level, s, pos);
    if dir == Direction::Up && state::same_block(neighbor, s) {
        return state::with_properties_of(d::BIG_DRIPLEAF_STEM, s);
    }
    s
}

/// `playTiltSound`: one draw from the level random.
fn play_tilt_sound<L: Level>(level: &mut L, pos: BlockPos, sound: &'static str) {
    let pitch = level.random().next_float() * (1.2f32 - 0.8f32) + 0.8f32;
    level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch });
}

/// `setTilt`: the leaf tilts (clients only; a game event when it starts to give).
fn set_tilt<L: Level>(level: &mut L, s: u16, pos: BlockPos, tilt: &str) {
    let old = state::get(s, "tilt").unwrap_or("none");
    set_block(level, pos, state::set(s, "tilt", tilt), flags::CLIENTS);
    if matches!(tilt, "partial" | "full") && tilt != old {
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    }
}

/// `resetTilt`.
fn reset_tilt<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    set_tilt(level, s, pos, "none");
    if state::get(s, "tilt") != Some("none") {
        play_tilt_sound(level, pos, "minecraft:block.big_dripleaf.tilt_up");
    }
}

/// `setTiltAndScheduleTick`: the next stage comes after 10 ticks (100 for a full tilt).
fn set_tilt_and_schedule<L: Level>(level: &mut L, s: u16, pos: BlockPos, tilt: &str, sound: Option<&'static str>) {
    set_tilt(level, s, pos, tilt);
    if let Some(sound) = sound {
        play_tilt_sound(level, pos, sound);
    }
    let delay = match tilt {
        "unstable" | "partial" => 10,
        "full" => 100,
        _ => -1,
    };
    if delay != -1 {
        schedule_block_tick(level, pos, BlockId::of(s), delay, TickPriority::Normal);
    }
}

/// `BigDripleafBlock.neighborChanged`: power straightens the leaf.
pub fn dripleaf_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if has_neighbor_signal(level, pos) {
        reset_tilt(level, s, pos);
    }
}

/// `BigDripleafBlock.tick`: unstable, partial, full, then up again.
pub fn dripleaf_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if has_neighbor_signal(level, pos) {
        reset_tilt(level, s, pos);
        return;
    }
    match state::get(s, "tilt") {
        Some("unstable") => set_tilt_and_schedule(level, s, pos, "partial", Some("minecraft:block.big_dripleaf.tilt_down")),
        Some("partial") => set_tilt_and_schedule(level, s, pos, "full", Some("minecraft:block.big_dripleaf.tilt_down")),
        Some("full") => reset_tilt(level, s, pos),
        _ => {}
    }
}

/// `BigDripleafBlock.entityInside` for an entity that can tilt it (standing on it): an upright leaf
/// with no power becomes unstable.
pub fn dripleaf_entity_inside<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get(s, "tilt") == Some("none") && !has_neighbor_signal(level, pos) {
        set_tilt_and_schedule(level, s, pos, "unstable", None);
    }
}

// ---------------------------------------------------------------------- copper bulb

/// `CopperBulbBlock.checkAndFlip`: a new redstone signal flips the bulb.
pub fn bulb_check_and_flip<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let powered = has_neighbor_signal(level, pos);
    if powered == state::get_bool(s, "powered") {
        return;
    }
    let mut new = s;
    if !state::get_bool(s, "powered") {
        new = state::set_bool(new, "lit", !state::get_bool(new, "lit"));
        let sound = if state::get_bool(new, "lit") { "minecraft:block.copper_bulb.turn_on" } else { "minecraft:block.copper_bulb.turn_off" };
        level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
    }
    set_block_and_update(level, pos, state::set_bool(new, "powered", powered));
}

/// `CopperBulbBlock.onPlace`.
pub fn bulb_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if !state::same_block(old, s) {
        bulb_check_and_flip(level, s, pos);
    }
}

// ---------------------------------------------------------------------- big dripleaf stem

/// `BigDripleafStemBlock.canSurvive`: stem or `#supports_big_dripleaf` below, stem or leaf above.
pub fn stem_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let below = level.block(pos.below());
    let above = level.block(pos.above());
    (state::same_block(below, s) || tags::is(below, "minecraft:supports_big_dripleaf")) && (state::same_block(above, s) || state::is(above, d::BIG_DRIPLEAF))
}

/// `BigDripleafStemBlock.updateShape`: a stem that lost its support or its leaf breaks next tick.
pub fn stem_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if (dir == Direction::Down || dir == Direction::Up) && !stem_can_survive(level, s, pos) {
        schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
    }
    crate::fluid::tick_water_if_waterlogged(level, s, pos);
    s
}

/// `BigDripleafStemBlock.tick`.
pub fn stem_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !stem_can_survive(level, s, pos) {
        crate::update::destroy_block(level, pos, true, 512);
    }
}
