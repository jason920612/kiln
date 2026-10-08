//! What spreads, melts and dies on its own: grass and mycelium (`SpreadingSnowyBlock`), snow
//! layers, ice and frosted ice. Each function follows the vanilla method call by call: the
//! order and number of random draws is part of the behaviour.

use super::connect;
use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use kiln_data::block_logic::{self as logic, FluidKind};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

/// `Mth.nextInt(random, min, max)`: `min` to `max` inclusive.
pub(crate) fn mth_next_int<R: RandomSource + ?Sized>(random: &mut R, min: i32, max: i32) -> i32 {
    if min >= max { min } else { random.next_int_bounded(max - min + 1) + min }
}

/// `LightEngine.isEmptyShape`.
fn empty_shape(s: u16) -> bool {
    !block_props::can_occlude(s) || !block_props::uses_shape_for_light_occlusion(s)
}

/// `LightEngine.getLightDampeningInto(from, to, dir, to's dampening)`: 16 when the two
/// occlusion shapes close the face between them, else the dampening of `to`.
pub fn light_dampening_into(from: u16, to: u16, dir: Direction) -> i32 {
    let (from_empty, to_empty) = (empty_shape(from), empty_shape(to));
    let dampening = i32::from(block_props::light_dampening(to));
    if from_empty && to_empty {
        return dampening;
    }
    let closes = (!from_empty && block_props::face_full(from, dir as u8)) || (!to_empty && block_props::face_full(to, dir.opposite() as u8));
    if closes { 16 } else { dampening }
}

/// `SpreadingSnowyBlock.canStayAlive`.
fn can_stay_alive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let above = level.block(pos.above());
    if state::is(above, d::SNOW) && state::get_int(above, "layers") == 1 {
        return true;
    }
    let f = logic::fluid(above);
    if !f.is_empty() && f.amount == 8 {
        return false;
    }
    light_dampening_into(s, above, Direction::Up) < 15
}

/// `SpreadingSnowyBlock.canPropagate`.
fn can_propagate<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    can_stay_alive(level, s, pos) && logic::fluid(level.block(pos.above())).kind != FluidKind::Water
}

/// `SpreadingSnowyBlock.randomTick` (grass block, mycelium): dies to dirt when covered, and
/// spreads to four random dirt blocks around it in light.
pub fn spreading_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !can_stay_alive(level, s, pos) {
        crate::set_block_and_update(level, pos, d::DIRT);
        return;
    }
    if level.max_local_raw_brightness(pos.above()) >= 9 {
        let spread = BlockId::of(s).default_state();
        for _ in 0..4 {
            let dx = level.random().next_int_bounded(3) - 1;
            let dy = level.random().next_int_bounded(5) - 3;
            let dz = level.random().next_int_bounded(3) - 1;
            let target = pos.offset(dx, dy, dz);
            if state::is(level.block(target), d::DIRT) && can_propagate(level, spread, target) {
                let snowy = connect::snowy_setting(level.block(target.above()));
                crate::set_block_and_update(level, target, state::set_bool(spread, "snowy", snowy));
            }
        }
    }
}

/// `SnowLayerBlock.randomTick`: block light above 11 melts the layers away.
pub fn snow_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.block_light(pos) > 11 {
        level.effect(Effect::Drop { pos, state: s });
        crate::remove_block(level, pos, false);
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_destroy", state: s });
    }
}

/// `IceBlock.randomTick` (also frosted ice's).
pub fn ice_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.block_light(pos) > 11 - i32::from(block_props::light_dampening(s)) {
        melt(level, s, pos);
    }
}

/// `IceBlock.melt`: water, or nothing where water evaporates.
pub fn melt<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.rules().water_evaporates {
        crate::remove_block(level, pos, false);
        return;
    }
    crate::set_block_and_update(level, pos, d::WATER);
    crate::update::neighbor_changed_at(level, pos, BlockId::of(d::WATER));
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_destroy", state: s });
}

/// `FrostedIceBlock.onPlace`: the first fade check in 60 to 120 ticks.
pub fn frosted_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let delay = mth_next_int(level.random(), 60, 120);
    schedule_block_tick(level, pos, BlockId::of(s), delay, TickPriority::Normal);
}

/// `FrostedIceBlock.fewerNeigboursThan`.
fn fewer_neighbours_than<L: Level + ?Sized>(level: &L, pos: BlockPos, n: i32) -> bool {
    let mut count = 0;
    for dir in Direction::ALL {
        if state::is(level.block(pos.relative(dir)), d::FROSTED_ICE) {
            count += 1;
            if count >= n {
                return false;
            }
        }
    }
    true
}

/// `FrostedIceBlock.slightlyMelt`: one more age, or the melt (true) at the last one.
fn slightly_melt<L: Level>(level: &mut L, s: u16, pos: BlockPos) -> bool {
    let age = state::get_int(s, "age");
    if age < 3 {
        crate::set_block(level, pos, state::set_int(s, "age", age + 1), flags::CLIENTS);
        return false;
    }
    melt(level, s, pos);
    true
}

/// `FrostedIceBlock.tick`.
pub fn frosted_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let id = BlockId::of(s);
    if level.random().next_int_bounded(3) == 0 || fewer_neighbours_than(level, pos, 4) {
        let brightness = if level.is_end() { level.block_light(pos) } else { level.max_local_raw_brightness(pos) };
        if brightness > 11 - state::get_int(s, "age") - i32::from(block_props::light_dampening(s)) && slightly_melt(level, s, pos) {
            for dir in Direction::ALL {
                let n = pos.relative(dir);
                let ns = level.block(n);
                if state::is(ns, d::FROSTED_ICE) && !slightly_melt(level, ns, n) {
                    let delay = mth_next_int(level.random(), 20, 40);
                    schedule_block_tick(level, n, id, delay, TickPriority::Normal);
                }
            }
            return;
        }
    }
    let delay = mth_next_int(level.random(), 20, 40);
    schedule_block_tick(level, pos, id, delay, TickPriority::Normal);
}

/// `FrostedIceBlock.neighborChanged`: a frosted ice block that lost its neighbours melts.
pub fn frosted_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId) {
    if source == BlockId::of(d::FROSTED_ICE) && fewer_neighbours_than(level, pos, 2) {
        melt(level, s, pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Level;
    use crate::test_level::TestLevel;
    use kiln_data::blocks_types::is_air;

    fn level() -> TestLevel {
        let mut l = TestLevel::flat(-64, 384, &[d::STONE]);
        l.load_chunks((-2, -2), (2, 2));
        l
    }

    #[test]
    fn ice_evaporates_where_water_does() {
        // `IceBlock.melt`: nothing but air where `water_evaporates` (the nether), else water.
        let (mut wet, mut nether) = (level(), level());
        nether.rules.water_evaporates = true;
        let pos = BlockPos::new(0, 70, 0);
        for l in [&mut wet, &mut nether] {
            l.set_raw(pos, d::ICE, flags::NONE);
            l.block_brightness.insert(pos, 15);
            ice_random_tick(l, d::ICE, pos);
        }
        assert!(state::is(wet.block(pos), d::WATER));
        assert!(is_air(nether.block(pos)));
    }

    #[test]
    fn ice_needs_more_than_ten_block_light() {
        // `getBrightness(BLOCK) > 11 - lightDampening` and ice dampens by 1.
        let mut l = level();
        let pos = BlockPos::new(0, 70, 0);
        l.set_raw(pos, d::ICE, flags::NONE);
        l.block_brightness.insert(pos, 10);
        ice_random_tick(&mut l, d::ICE, pos);
        assert!(state::is(l.block(pos), d::ICE));
        l.block_brightness.insert(pos, 11);
        ice_random_tick(&mut l, d::ICE, pos);
        assert!(state::is(l.block(pos), d::WATER));
    }

    #[test]
    fn frosted_ice_in_the_end_reads_block_light_only() {
        // `FrostedIceBlock.tick`: brightness is the block light in the End, the sky included elsewhere.
        let pos = BlockPos::new(0, 70, 0);
        let frosted = state::set_int(d::FROSTED_ICE, "age", 3);
        for (end, melts) in [(false, true), (true, false)] {
            let mut l = level();
            l.end = end;
            l.set_raw(pos, frosted, flags::NONE);
            l.default_brightness = 15; // full sky light everywhere
            // Skip the one-in-three roll and the neighbour count: five neighbours would stop the
            // melt, so give it none and try until the roll lets the check through.
            for _ in 0..40 {
                if !state::is(l.block(pos), d::FROSTED_ICE) {
                    break;
                }
                let current = l.block(pos);
                frosted_tick(&mut l, current, pos);
            }
            assert_eq!(!state::is(l.block(pos), d::FROSTED_ICE), melts, "end {end}");
        }
    }

    #[test]
    fn grass_dies_under_a_full_block_and_lives_under_glass() {
        let mut l = level();
        let grass = BlockPos::new(0, 70, 0);
        l.set_raw(grass, d::GRASS_BLOCK, flags::NONE);
        l.set_raw(grass.above(), d::GLASS, flags::NONE);
        spreading_random_tick(&mut l, d::GRASS_BLOCK, grass);
        assert!(state::is(l.block(grass), d::GRASS_BLOCK));
        l.set_raw(grass.above(), d::STONE, flags::NONE);
        spreading_random_tick(&mut l, d::GRASS_BLOCK, grass);
        assert!(state::is(l.block(grass), d::DIRT));
    }

    #[test]
    fn grass_under_one_layer_of_snow_survives_but_not_under_two() {
        let mut l = level();
        let grass = BlockPos::new(0, 70, 0);
        l.set_raw(grass, d::GRASS_BLOCK, flags::NONE);
        l.set_raw(grass.above(), d::SNOW, flags::NONE);
        spreading_random_tick(&mut l, d::GRASS_BLOCK, grass);
        assert!(state::is(l.block(grass), d::GRASS_BLOCK));
        l.set_raw(grass.above(), state::set_int(d::SNOW, "layers", 2), flags::NONE);
        spreading_random_tick(&mut l, d::GRASS_BLOCK, grass);
        assert!(state::is(l.block(grass), d::DIRT));
    }
}
