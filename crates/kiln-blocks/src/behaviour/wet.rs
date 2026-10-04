//! Water-bound blocks: corals that die out of water (`BaseCoralPlantTypeBlock` and its
//! subclasses, `CoralBlock`), scaffolding that settles or falls, and sponges that soak up water.
//! Each function follows the vanilla method call by call.

use super::sturdy;
use crate::fluid;
use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind, Support, interface};
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;
use std::collections::{HashSet, VecDeque};

// ---------------------------------------------------------------- corals

fn is_wall_fan(s: u16) -> bool {
    logic::is_instance(s, BlockClass::BaseCoralWallFanBlock)
}

/// Whether the live coral class `s` (plant, fan, wall fan, block) is one that dies out of water.
pub fn is_live_coral(s: u16) -> bool {
    matches!(
        logic::block_class(s),
        BlockClass::CoralPlantBlock | BlockClass::CoralFanBlock | BlockClass::CoralWallFanBlock | BlockClass::CoralBlock
    )
}

/// `BaseCoralPlantTypeBlock` and `CoralBlock` and their subclasses.
pub fn is_coral(s: u16) -> bool {
    logic::is_instance(s, BlockClass::BaseCoralPlantTypeBlock) || logic::block_class(s) == BlockClass::CoralBlock
}

/// `BaseCoralPlantTypeBlock.canSurvive` and the wall fan's.
pub fn coral_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    if is_wall_fan(s) {
        let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
        let behind = pos.relative(facing.opposite());
        sturdy(level.block(behind), facing, Support::Full)
    } else {
        sturdy(level.block(pos.below()), Direction::Up, Support::Full)
    }
}

/// `scanForWater`: water in the block or beside it (blocks of `CoralBlock` have none of their own).
fn scan_for_water<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    if state::get_bool(s, "waterlogged") {
        return true;
    }
    Direction::ALL.iter().any(|&dir| logic::fluid(level.block(pos.relative(dir))).kind == FluidKind::Water)
}

/// The dead block of a live coral (`dead_` + its name).
fn dead_block(s: u16) -> u16 {
    let name = BlockId::of(s).name().replace("minecraft:", "");
    BlockId::by_name(&format!("dead_{name}")).map_or(s, BlockId::default_state)
}

/// `tryScheduleDieTick`: out of water, the coral dies in 60 to 99 ticks.
fn try_schedule_die_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !scan_for_water(level, s, pos) {
        let delay = 60 + level.random().next_int_bounded(40);
        schedule_block_tick(level, pos, BlockId::of(s), delay, TickPriority::Normal);
    }
}

/// `onPlace` of live plants, fans and wall fans.
pub fn coral_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if logic::block_class(s) != BlockClass::CoralBlock {
        try_schedule_die_tick(level, s, pos);
    }
}

/// `tick` of live corals: out of water they turn into their dead block.
pub fn coral_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if logic::block_class(s) == BlockClass::CoralBlock {
        if !Direction::ALL.iter().any(|&dir| logic::fluid(level.block(pos.relative(dir))).kind == FluidKind::Water) {
            crate::set_block(level, pos, dead_block(s), flags::CLIENTS);
        }
        return;
    }
    if !scan_for_water(level, s, pos) {
        let mut dead = state::set_bool(dead_block(s), "waterlogged", false);
        if is_wall_fan(s) {
            dead = state::set(dead, "facing", state::get(s, "facing").unwrap_or("north"));
        }
        crate::set_block(level, pos, dead, flags::CLIENTS);
    }
}

/// `updateShape` of the coral classes (the waterlogged fluid tick is scheduled by the caller
/// first, as every `SimpleWaterloggedBlock` does).
pub fn coral_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if logic::block_class(s) == BlockClass::CoralBlock {
        let wet = Direction::ALL.iter().any(|&d| logic::fluid(level.block(pos.relative(d))).kind == FluidKind::Water);
        if !wet {
            let delay = 60 + level.random().next_int_bounded(40);
            schedule_block_tick(level, pos, BlockId::of(s), delay, TickPriority::Normal);
        }
        return s;
    }
    let live = is_live_coral(s);
    if is_wall_fan(s) {
        let lost = dir.opposite() == state::get_dir(s, "facing").unwrap_or(Direction::North) && !coral_can_survive(level, s, pos);
        if lost {
            return d::AIR;
        }
        if live {
            try_schedule_die_tick(level, s, pos);
        }
        return s;
    }
    if dir == Direction::Down && !coral_can_survive(level, s, pos) {
        return d::AIR;
    }
    if live {
        try_schedule_die_tick(level, s, pos);
    }
    s
}

// ---------------------------------------------------------------- scaffolding

/// `ScaffoldingBlock.getDistance`.
fn scaffolding_distance<L: Level + ?Sized>(level: &L, pos: BlockPos) -> i32 {
    let below = level.block(pos.below());
    let mut distance = 7;
    if state::is(below, d::SCAFFOLDING) {
        distance = state::get_int(below, "distance");
    } else if sturdy(below, Direction::Up, Support::Full) {
        return 0;
    }
    for dir in Direction::HORIZONTAL {
        let n = level.block(pos.relative(dir));
        if !state::is(n, d::SCAFFOLDING) {
            continue;
        }
        distance = distance.min(state::get_int(n, "distance") + 1);
        if distance == 1 {
            break;
        }
    }
    distance
}

/// `ScaffoldingBlock.isBottom`.
fn scaffolding_is_bottom<L: Level + ?Sized>(level: &L, pos: BlockPos, distance: i32) -> bool {
    distance > 0 && !state::is(level.block(pos.below()), d::SCAFFOLDING)
}

/// `ScaffoldingBlock.onPlace` / `updateShape`: the next tick re-reads the distance.
pub fn scaffolding_schedule<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
}

/// `ScaffoldingBlock.tick`.
pub fn scaffolding_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let distance = scaffolding_distance(level, pos);
    let bottom = scaffolding_is_bottom(level, pos, distance);
    let new = state::set_bool(state::set_int(s, "distance", distance), "bottom", bottom);
    if state::get_int(new, "distance") == 7 {
        if state::get_int(s, "distance") == 7 {
            // `FallingBlockEntity.fall`: the block leaves as a falling entity.
            level.effect(Effect::FallingBlock { pos, state: state::set_bool(new, "waterlogged", false) });
            crate::set_block(level, pos, fluid::legacy_block(logic::fluid(new)), flags::ALL);
        } else {
            crate::destroy_block(level, pos, true, flags::LIMIT);
        }
    } else if s != new {
        crate::set_block_and_update(level, pos, new);
    }
}

// ---------------------------------------------------------------- sponges

/// `SpongeBlock.removeWaterBreadthFirstSearch`: soaks up the water within 6 blocks (up to 65
/// blocks of it, the sponge's own place counted), true when it found any.
fn remove_water_bfs<L: Level>(level: &mut L, origin: BlockPos) -> bool {
    let mut queue: VecDeque<(BlockPos, i32)> = VecDeque::new();
    let mut visited: HashSet<BlockPos> = HashSet::new();
    queue.push_back((origin, 0));
    let mut count = 0;
    while let Some((pos, depth)) = queue.pop_front() {
        if !visited.insert(pos) {
            continue;
        }
        if pos != origin && !absorb_node(level, pos) {
            continue;
        }
        count += 1;
        if count >= 65 {
            break;
        }
        if depth < 6 {
            for dir in Direction::ALL {
                queue.push_back((pos.relative(dir), depth + 1));
            }
        }
    }
    count > 1
}

/// The traversal's node check: false to skip the block, true when it was water (now removed).
fn absorb_node<L: Level>(level: &mut L, pos: BlockPos) -> bool {
    let s = level.block(pos);
    if logic::fluid(s).kind != FluidKind::Water {
        return false;
    }
    if logic::implements(s, interface::BUCKET_PICKUP) {
        if logic::block_class(s) == BlockClass::LiquidBlock {
            // `LiquidBlock.pickupBlock`: only a source gives a bucket.
            if state::get_int(s, "level") == 0 {
                crate::set_block(level, pos, d::AIR, flags::ALL_IMMEDIATE);
                return true;
            }
        } else if state::get_bool(s, "waterlogged") {
            // `SimpleWaterloggedBlock.pickupBlock`: dry the block, which may then break.
            crate::set_block(level, pos, state::set_bool(s, "waterlogged", false), flags::ALL);
            if !super::support::can_survive(level, s, pos) {
                crate::destroy_block(level, pos, true, flags::LIMIT);
            }
            return true;
        }
    }
    if logic::block_class(s) == BlockClass::LiquidBlock {
        crate::set_block_and_update(level, pos, d::AIR);
        return true;
    }
    if matches!(logic::block_class(s), BlockClass::KelpBlock | BlockClass::KelpPlantBlock | BlockClass::SeagrassBlock | BlockClass::TallSeagrassBlock) {
        level.effect(Effect::Drop { pos, state: s });
        crate::set_block_and_update(level, pos, d::AIR);
        return true;
    }
    false
}

/// `SpongeBlock.tryAbsorbWater`.
pub fn sponge_try_absorb<L: Level>(level: &mut L, pos: BlockPos) {
    if remove_water_bfs(level, pos) {
        crate::set_block(level, pos, d::WET_SPONGE, flags::CLIENTS);
        level.effect(Effect::Sound { pos, sound: "minecraft:block.sponge.absorb", volume: 1.0, pitch: 1.0 });
    }
}

/// `WetSpongeBlock.onPlace`: where water evaporates it dries at once.
pub fn wet_sponge_on_place<L: Level>(level: &mut L, pos: BlockPos) {
    if level.rules().water_evaporates {
        crate::set_block_and_update(level, pos, d::SPONGE);
        level.effect(Effect::LevelEvent { id: 2009, pos, data: 0 });
        let pitch = (level.random().next_float() * 0.2 + 1.0) * 0.7;
        level.effect(Effect::Sound { pos, sound: "minecraft:block.wet_sponge.dries", volume: 1.0, pitch });
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
    fn wet_sponges_dry_where_water_evaporates() {
        let pos = BlockPos::new(0, 70, 0);
        let mut nether = level();
        nether.rules.water_evaporates = true;
        nether.set_raw(pos, d::WET_SPONGE, flags::NONE);
        wet_sponge_on_place(&mut nether, pos);
        assert!(state::is(nether.block(pos), d::SPONGE));
        let mut over = level();
        over.set_raw(pos, d::WET_SPONGE, flags::NONE);
        wet_sponge_on_place(&mut over, pos);
        assert!(state::is(over.block(pos), d::WET_SPONGE));
    }

    #[test]
    fn a_sponge_in_a_pool_takes_the_water_within_six_blocks() {
        let mut l = level();
        let pos = BlockPos::new(0, 70, 0);
        for x in -7..=7 {
            l.set_raw(BlockPos::new(x, 70, 0), d::WATER, flags::NONE);
        }
        l.set_raw(pos, d::SPONGE, flags::NONE);
        sponge_try_absorb(&mut l, pos);
        assert!(state::is(l.block(pos), d::WET_SPONGE));
        // Six blocks either side were soaked up; the seventh stays.
        assert!(is_air(l.block(BlockPos::new(6, 70, 0))) && is_air(l.block(BlockPos::new(-6, 70, 0))));
        assert!(state::is(l.block(BlockPos::new(7, 70, 0)), d::WATER) && state::is(l.block(BlockPos::new(-7, 70, 0)), d::WATER));
    }

    #[test]
    fn a_lone_sponge_stays_dry() {
        let mut l = level();
        let pos = BlockPos::new(0, 70, 0);
        l.set_raw(pos, d::SPONGE, flags::NONE);
        sponge_try_absorb(&mut l, pos);
        assert!(state::is(l.block(pos), d::SPONGE));
    }

    #[test]
    fn scaffolding_distance_counts_from_the_supported_one() {
        let mut l = level();
        let base = BlockPos::new(0, 70, 0);
        for x in 0..3 {
            l.set_raw(base.offset(x, 0, 0), state::set_int(d::SCAFFOLDING, "distance", x), flags::NONE);
        }
        // Over air, the nearest neighbour's distance plus one; over a sturdy block, 0; nothing around, 7.
        assert_eq!(scaffolding_distance(&l, base.offset(1, 0, 0)), 1);
        assert_eq!(scaffolding_distance(&l, base.offset(3, 0, 0)), 3);
        assert_eq!(scaffolding_distance(&l, base.offset(8, 0, 0)), 7);
        l.set_raw(base.offset(3, -1, 0), d::STONE, flags::NONE);
        assert_eq!(scaffolding_distance(&l, base.offset(3, 0, 0)), 0);
    }
}
