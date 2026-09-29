//! Sculk blocks' own behaviour: sculk sensors' phases (`SculkSensorBlock.tick`,
//! `deactivate`, `onPlace`, `affectNeighborsAfterRemoval`), shriekers ending their shriek and
//! catalysts ending their bloom. What their block entities do with vibrations is the level's
//! ([`Level::block_entity_tick`] for the shrieker's answer).

use crate::level::{Effect, Level, schedule_block_tick};
use crate::pos::BlockPos;
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{set_block, set_block_and_update, update_neighbors_at};
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_javamath::random::RandomSource;

/// `SculkSensorBlock.COOLDOWN_TICKS`.
pub const COOLDOWN_TICKS: i32 = 10;

/// `SculkSensorBlock.getActiveTicks` (10 for a calibrated sensor).
pub fn active_ticks(s: u16) -> i32 {
    if logic::block_class(s) == BlockClass::CalibratedSculkSensorBlock { 10 } else { 30 }
}

/// `SculkSensorBlock.canActivate`: inactive.
pub fn can_activate(s: u16) -> bool {
    state::get(s, "sculk_sensor_phase") == Some("inactive")
}

/// A sculk sensor or calibrated sculk sensor.
pub fn is_sensor(s: u16) -> bool {
    logic::is_instance(s, BlockClass::SculkSensorBlock)
}

/// `SculkSensorBlock.updateNeighbours`.
pub fn update_neighbours<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let block = BlockId::of(s);
    update_neighbors_at(level, pos, block);
    update_neighbors_at(level, pos.below(), block);
}

/// `SculkSensorBlock.tick`: an active sensor cools down, a cooling one becomes inactive.
pub fn sensor_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    match state::get(s, "sculk_sensor_phase") {
        Some("active") => deactivate(level, pos, s),
        Some("cooldown") => {
            set_block_and_update(level, pos, state::set(s, "sculk_sensor_phase", "inactive"));
            if !state::get_bool(s, "waterlogged") {
                let pitch = level.random().next_float() * 0.2 + 0.8;
                level.effect(Effect::Sound { pos, sound: "minecraft:block.sculk_sensor.clicking_stop", volume: 1.0, pitch });
            }
        }
        _ => {}
    }
}

/// `SculkSensorBlock.deactivate`.
pub fn deactivate<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let cooled = state::set_int(state::set(s, "sculk_sensor_phase", "cooldown"), "power", 0);
    set_block_and_update(level, pos, cooled);
    schedule_block_tick(level, pos, BlockId::of(s), COOLDOWN_TICKS, TickPriority::Normal);
    update_neighbours(level, pos, s);
}

/// The block half of `SculkSensorBlock.activate`: active with `power`, the tick that ends it,
/// the neighbours told. The resonance, the clicking game event and sound are the caller's.
pub fn activate<L: Level>(level: &mut L, pos: BlockPos, s: u16, power: i32) {
    let active = state::set_int(state::set(s, "sculk_sensor_phase", "active"), "power", power);
    set_block_and_update(level, pos, active);
    schedule_block_tick(level, pos, BlockId::of(s), active_ticks(s), TickPriority::Normal);
    update_neighbours(level, pos, s);
}

/// `SculkSensorBlock.onPlace`: a sensor placed with power and no tick to end it loses it.
pub fn sensor_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if state::same_block(s, old) {
        return;
    }
    if state::get_int(s, "power") > 0 && !level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)) {
        set_block(level, pos, state::set_int(s, "power", 0), 18);
    }
}

/// `SculkSensorBlock.affectNeighborsAfterRemoval`: an active sensor's neighbours lose its power.
pub fn sensor_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get(s, "sculk_sensor_phase") == Some("active") {
        update_neighbours(level, pos, s);
    }
}

/// `SculkShriekerBlock.tick`: the shriek ends and the block entity answers
/// (`SculkShriekerBlockEntity.tryRespond`, the level's).
pub fn shrieker_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "shrieking") {
        let quiet = state::set_bool(s, "shrieking", false);
        set_block_and_update(level, pos, quiet);
        level.block_entity_tick(pos, quiet);
    }
}

/// `SculkCatalystBlock.tick`: the bloom ends.
pub fn catalyst_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "bloom") {
        set_block_and_update(level, pos, state::set_bool(s, "bloom", false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_level::TestLevel;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn sensors_cool_down_and_rest() {
        let mut level = TestLevel::flat(-64, 384, &[d::BEDROCK]);
        let pos = BlockPos::new(0, 64, 0);
        let lamp = pos.above();
        level.set_raw(pos, d::SCULK_SENSOR, 3);
        level.set_raw(lamp, d::REDSTONE_LAMP, 3);
        activate(&mut level, pos, d::SCULK_SENSOR, 12);
        let s = level.block(pos);
        assert_eq!(state::get(s, "sculk_sensor_phase"), Some("active"));
        assert_eq!(state::get_int(s, "power"), 12);
        assert!(level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)));
        sensor_tick(&mut level, s, pos);
        let s = level.block(pos);
        assert_eq!(state::get(s, "sculk_sensor_phase"), Some("cooldown"));
        assert_eq!(state::get_int(s, "power"), 0);
        sensor_tick(&mut level, s, pos);
        assert!(can_activate(level.block(pos)));
        assert_eq!(active_ticks(d::CALIBRATED_SCULK_SENSOR), 10);
        assert!(is_sensor(d::CALIBRATED_SCULK_SENSOR) && !is_sensor(d::SCULK));
    }
}
