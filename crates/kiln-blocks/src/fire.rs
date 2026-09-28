//! Fire (`FireBlock`, `SoulFireBlock`, `BaseFireBlock.getState`): flammability from
//! `FireBlock.bootStrap`'s registrations, the side faces fire takes from burnable
//! neighbours, the scheduled tick that ages it, burns neighbours away and spreads it, and
//! rain putting it out.

use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::state;
use crate::tags;
use kiln_data::block_logic::{self as logic, Support};
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

/// `FireBlock.getIgniteOdds(BlockState)`: waterlogged blocks do not catch fire.
pub fn ignite_odds(s: u16) -> i32 {
    if state::get_bool(s, "waterlogged") { 0 } else { logic::flammability(s).0 }
}

/// `FireBlock.getBurnOdds(BlockState)`.
pub fn burn_odds(s: u16) -> i32 {
    if state::get_bool(s, "waterlogged") { 0 } else { logic::flammability(s).1 }
}

/// `FireBlock.canBurn`.
pub fn can_burn(s: u16) -> bool {
    ignite_odds(s) > 0
}

/// `FireBlock.isValidFireLocation`: a burnable block on any side.
fn valid_fire_location<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    Direction::ALL.iter().any(|&dir| can_burn(level.block(pos.relative(dir))))
}

/// `FireBlock.getStateForPlacement(BlockGetter, BlockPos)`: on a burnable or sturdy floor
/// plain fire; otherwise a face toward each burnable side and up.
pub fn fire_for_placement<L: Level + ?Sized>(level: &L, pos: BlockPos) -> u16 {
    let below = level.block(pos.below());
    if can_burn(below) || crate::behaviour::sturdy(below, Direction::Up, Support::Full) {
        return d::FIRE;
    }
    let mut s = d::FIRE;
    for dir in Direction::ALL {
        if dir == Direction::Down {
            continue;
        }
        s = state::set_bool(s, dir.name(), can_burn(level.block(pos.relative(dir))));
    }
    s
}

/// `BaseFireBlock.getState`: soul fire on soul fire base blocks, else fire.
pub fn fire_state<L: Level + ?Sized>(level: &L, pos: BlockPos) -> u16 {
    if soul_fire_survives_on(level.block(pos.below())) { d::SOUL_FIRE } else { fire_for_placement(level, pos) }
}

/// `SoulFireBlock.canSurviveOnBlock`.
pub fn soul_fire_survives_on(below: u16) -> bool {
    tags::is(below, "minecraft:soul_fire_base_blocks")
}

/// `FireBlock.canSurvive`: a sturdy floor or something burnable beside it.
pub fn fire_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    crate::behaviour::sturdy(level.block(pos.below()), Direction::Up, Support::Full) || valid_fire_location(level, pos)
}

/// `BaseFireBlock.canSurvive` for either kind of fire.
pub fn can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    if state::is(s, d::SOUL_FIRE) { soul_fire_survives_on(level.block(pos.below())) } else { fire_can_survive(level, pos) }
}

/// `FireBlock.getStateWithAge`: the fire `BaseFireBlock.getState` would place, with `age`
/// when that is plain fire.
fn state_with_age<L: Level + ?Sized>(level: &L, pos: BlockPos, age: i32) -> u16 {
    let s = fire_state(level, pos);
    if state::is(s, d::FIRE) { state::set_int(s, "age", age) } else { s }
}

/// `FireBlock.updateShape` / `SoulFireBlock.updateShape`: faces follow the neighbours; fire
/// that cannot survive goes out.
pub fn update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> u16 {
    if !can_survive(level, s, pos) {
        return d::AIR;
    }
    if state::is(s, d::SOUL_FIRE) { s } else { state_with_age(level, pos, state::get_int(s, "age")) }
}

/// `FireBlock.getFireTickDelay`: 30 to 39 ticks.
pub fn schedule_fire_tick<L: Level>(level: &mut L, pos: BlockPos) {
    level.reseed_random(pos);
    let delay = 30 + level.random().next_int_bounded(10);
    crate::schedule_block_tick(level, pos, crate::BlockId::of(d::FIRE), delay, crate::TickPriority::Normal);
}

/// `FireBlock.isNearRain`.
fn near_rain<L: Level>(level: &L, pos: BlockPos) -> bool {
    level.is_raining_at(pos)
        || [Direction::West, Direction::East, Direction::North, Direction::South].iter().any(|&d| level.is_raining_at(pos.relative(d)))
}

/// `FireBlock.getIgniteOdds(LevelReader, BlockPos)`: for an empty position, the best ignite
/// odds around it.
fn ignite_odds_at<L: Level>(level: &L, pos: BlockPos) -> i32 {
    if !is_air(level.block(pos)) {
        return 0;
    }
    Direction::ALL.iter().fold(0, |m, &dir| ignite_odds(level.block(pos.relative(dir))).max(m))
}

/// `FireBlock.checkBurnOut`: the block at `pos` burns (becomes fire or goes away) with odds
/// burn odds in `chance`; burning TNT primes.
fn check_burn_out<L: Level>(level: &mut L, pos: BlockPos, chance: i32, age: i32) {
    let odds = burn_odds(level.block(pos));
    if level.random().next_int_bounded(chance) >= odds {
        return;
    }
    let old = level.block(pos);
    if level.random().next_int_bounded(age + 10) < 5 && !level.is_raining_at(pos) {
        let aged = (age + level.random().next_int_bounded(5) / 4).min(15);
        let fire = state_with_age(level, pos, aged);
        crate::set_block_and_update(level, pos, fire);
    } else {
        crate::remove_block(level, pos, false);
    }
    if logic::block_class(old) == logic::BlockClass::TntBlock {
        crate::redstone::devices::prime(level, pos);
    }
}

/// `FireBlock.tick`: reschedule; nothing more unless a player is close enough
/// (`fire_spread_radius_around_player`). Fire that cannot survive is removed (vanilla goes on
/// with the tick regardless); rain puts out fire off infiniburn blocks; the fire ages, burns
/// out off a burnable neighbour or a sturdy floor once older than 3, burns the blocks
/// beside it and spreads to empty positions up to 4 above and 1 below and around.
pub fn fire_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    schedule_fire_tick(level, pos);
    if !level.can_spread_fire_around(pos) {
        return;
    }
    let mut s = s;
    if !fire_can_survive(level, pos) {
        crate::remove_block(level, pos, false);
    }
    let below = level.block(pos.below());
    let infiniburn = tags::is(below, level.rules().infiniburn);
    let age = state::get_int(s, "age");
    if !infiniburn && level.weather().raining && near_rain(level, pos) && level.random().next_float() < 0.2 + age as f32 * 0.03 {
        crate::remove_block(level, pos, false);
        return;
    }
    let aged = 15.min(age + level.random().next_int_bounded(3) / 2);
    if age != aged {
        s = state::set_int(s, "age", aged);
        crate::set_block(level, pos, s, crate::flags::NONE);
    }
    if !infiniburn {
        if !valid_fire_location(level, pos) {
            let floor = level.block(pos.below());
            if !crate::behaviour::sturdy(floor, Direction::Up, Support::Full) || age > 3 {
                crate::remove_block(level, pos, false);
            }
            return;
        }
        if age == 15 && level.random().next_int_bounded(4) == 0 && !can_burn(level.block(pos.below())) {
            crate::remove_block(level, pos, false);
            return;
        }
    }
    let increased = level.increased_fire_burnout(pos);
    let penalty = if increased { -50 } else { 0 };
    check_burn_out(level, pos.relative(Direction::East), 300 + penalty, age);
    check_burn_out(level, pos.relative(Direction::West), 300 + penalty, age);
    check_burn_out(level, pos.below(), 250 + penalty, age);
    check_burn_out(level, pos.above(), 250 + penalty, age);
    check_burn_out(level, pos.relative(Direction::North), 300 + penalty, age);
    check_burn_out(level, pos.relative(Direction::South), 300 + penalty, age);
    let difficulty = level.difficulty();
    for dx in -1..=1 {
        for dz in -1..=1 {
            for dy in -1..=4 {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue;
                }
                let rate = if dy > 1 { 100 + (dy - 1) * 100 } else { 100 };
                let at = pos.offset(dx, dy, dz);
                let odds = ignite_odds_at(level, at);
                if odds <= 0 {
                    continue;
                }
                let mut chance = (odds + 40 + difficulty * 7) / (age + 30);
                if increased {
                    chance /= 2;
                }
                if chance > 0 && level.random().next_int_bounded(rate) <= chance {
                    if level.weather().raining && near_rain(level, at) {
                        continue;
                    }
                    let spread_age = 15.min(age + level.random().next_int_bounded(5) / 4);
                    let fire = state_with_age(level, at, spread_age);
                    crate::set_block_and_update(level, at, fire);
                }
            }
        }
    }
}

/// `LavaFluid.isFlammable`: a loaded block lava sets alight.
fn lava_ignites<L: Level>(level: &L, pos: BlockPos) -> bool {
    let inside = pos.y >= level.min_y() && pos.y < level.min_y() + level.height();
    if inside && !level.is_loaded(pos) {
        return false;
    }
    logic::ignited_by_lava(level.block(pos))
}

/// `LavaFluid.randomTick` (lava's random tick: vanilla runs it for the block and again for
/// its fluid): up to two steps upward looking for air beside something lava ignites, or
/// three tries at setting fire on top of such blocks beside the lava.
pub fn lava_random_tick<L: Level>(level: &mut L, pos: BlockPos) {
    if !level.can_spread_fire_around(pos) {
        return;
    }
    let steps = level.random().next_int_bounded(3);
    if steps > 0 {
        let mut at = pos;
        for _ in 0..steps {
            let dx = level.random().next_int_bounded(3) - 1;
            let dz = level.random().next_int_bounded(3) - 1;
            at = at.offset(dx, 1, dz);
            if !(level.in_bounds(at) && level.is_loaded(at)) {
                return;
            }
            let s = level.block(at);
            if is_air(s) {
                if Direction::ALL.iter().any(|&dir| lava_ignites(level, at.relative(dir))) {
                    let fire = fire_state(level, at);
                    crate::set_block_and_update(level, at, fire);
                    return;
                }
            } else if tags::is(s, "minecraft:blocks_lava_fire_spread") {
                return;
            }
        }
    } else {
        for _ in 0..3 {
            let dx = level.random().next_int_bounded(3) - 1;
            let dz = level.random().next_int_bounded(3) - 1;
            let at = pos.offset(dx, 0, dz);
            if !(level.in_bounds(at) && level.is_loaded(at)) {
                return;
            }
            if is_air(level.block(at.above())) && lava_ignites(level, at) {
                // Vanilla reads the fire's state at the burning block, not above it.
                let fire = fire_state(level, at);
                crate::set_block_and_update(level, at.above(), fire);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestLevel;
    use std::collections::BTreeMap;

    fn floor_level() -> TestLevel {
        let mut level = TestLevel::flat(-64, 384, &[d::BEDROCK]);
        level.load_chunks((-4, -4), (12, 4));
        level
    }

    fn put(level: &mut TestLevel, pos: BlockPos, s: u16) {
        level.set_raw(pos, s, crate::flags::NONE);
    }

    /// One round of the vectors: every fire of the area (y, z, x order) still fire ticks.
    fn round(level: &mut TestLevel, x0: i32, z0: i32) {
        let mut fires = Vec::new();
        for y in 99..110 {
            for z in z0..z0 + 16 {
                for x in x0..x0 + 16 {
                    let p = BlockPos::new(x, y, z);
                    if state::is(level.block(p), d::FIRE) {
                        fires.push(p);
                    }
                }
            }
        }
        for p in fires {
            let s = level.block(p);
            if state::is(s, d::FIRE) {
                fire_tick(level, s, p);
            }
        }
    }

    fn snapshot(level: &TestLevel, x0: i32, z0: i32) -> BTreeMap<(i32, i32, i32), u16> {
        let mut out = BTreeMap::new();
        for y in 99..110 {
            for z in z0..z0 + 16 {
                for x in x0..x0 + 16 {
                    let s = level.block(BlockPos::new(x, y, z));
                    if if y == 99 { state::is(s, d::STONE) } else { is_air(s) } {
                        continue;
                    }
                    out.insert((x - x0, y, z - z0), s);
                }
            }
        }
        out
    }

    fn parse(v: &serde_json::Value) -> BTreeMap<(i32, i32, i32), u16> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let at = |i: usize| e[i].as_i64().unwrap() as i32;
                let s = e[3].as_str().unwrap();
                ((at(0), at(1), at(2)), state::parse_state(s).unwrap_or_else(|| panic!("state {s}")))
            })
            .collect()
    }

    /// Replays `tools/FireVectors.java` (`KILN_FIRE_VECTORS`, `tools/fire_vectors.py`).
    #[test]
    fn fire_parity() {
        let Some(path) = std::env::var_os("KILN_FIRE_VECTORS") else {
            eprintln!("skipped: set KILN_FIRE_VECTORS (tools/fire_vectors.py)");
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let (mut rounds_ok, mut rounds_total, mut failed) = (0, 0, Vec::new());
        for line in text.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let name = v["name"].as_str().unwrap().to_string();
            assert_ne!(name, "error", "{line}");
            let (x0, z0) = (v["x0"].as_i64().unwrap() as i32, v["z0"].as_i64().unwrap() as i32);
            let mut level = floor_level();
            level.difficulty = v["difficulty"].as_i64().unwrap() as i32;
            for x in x0..x0 + 16 {
                for z in z0..z0 + 16 {
                    put(&mut level, BlockPos::new(x, 99, z), d::STONE);
                }
            }
            for ((x, y, z), s) in parse(&v["initial"]) {
                put(&mut level, BlockPos::new(x0 + x, y, z0 + z), s);
            }
            level.set_random_seed(v["seed"].as_i64().unwrap());
            let mut first_bad = None;
            for (r, want) in v["rounds"].as_array().unwrap().iter().enumerate() {
                round(&mut level, x0, z0);
                let probe = level.random().next_long();
                let expected = parse(&want[0]);
                let got = snapshot(&level, x0, z0);
                rounds_total += 1;
                let probe_ok = probe == want[1].as_i64().unwrap();
                if got == expected && probe_ok {
                    rounds_ok += 1;
                } else if first_bad.is_none() {
                    let diff: Vec<String> = expected
                        .iter()
                        .filter(|(k, s)| got.get(k) != Some(s))
                        .map(|(k, s)| format!("{k:?} want {} got {:?}", state::state_string(*s), got.get(k).map(|g| state::state_string(*g))))
                        .chain(got.iter().filter(|(k, _)| !expected.contains_key(k)).map(|(k, g)| format!("{k:?} want air got {}", state::state_string(*g))))
                        .take(6)
                        .collect();
                    first_bad = Some(format!("{name} round {r}: probe ok {probe_ok}; {diff:?}"));
                }
            }
            if let Some(b) = first_bad {
                failed.push(b);
            }
        }
        eprintln!("fire parity: {rounds_ok}/{rounds_total} rounds match");
        assert!(failed.is_empty(), "{failed:#?}");
    }

    #[test]
    fn flammability_table() {
        assert_eq!(logic::flammability(d::OAK_PLANKS), (5, 20));
        assert_eq!(ignite_odds(d::STONE), 0);
        let wet = state::set_bool(d::OAK_STAIRS, "waterlogged", true);
        assert_eq!(ignite_odds(wet), 0);
        assert!(can_burn(d::OAK_STAIRS));
    }

    #[test]
    fn fire_spreads_and_burns_planks_away() {
        let mut level = floor_level();
        for x in -3..=3 {
            for z in -3..=3 {
                put(&mut level, BlockPos::new(x, 99, z), d::STONE);
                put(&mut level, BlockPos::new(x, 100, z), d::OAK_PLANKS);
            }
        }
        put(&mut level, BlockPos::new(0, 101, 0), d::FIRE);
        let planks = |l: &TestLevel| {
            (-3..=3).flat_map(|x| (-3..=3).map(move |z| (x, z))).filter(|&(x, z)| state::is(l.block(BlockPos::new(x, 100, z)), d::OAK_PLANKS)).count()
        };
        let mut spread = false;
        for _ in 0..400 {
            round(&mut level, -3, -3);
            spread |= (-3..=3).any(|x| (-3..=3).any(|z| (x, z) != (0, 0) && state::is(level.block(BlockPos::new(x, 101, z)), d::FIRE)));
        }
        assert!(spread, "fire spread over the planks");
        assert!(planks(&level) < 49, "planks burnt away");
    }

    #[test]
    fn lava_sets_planks_alight() {
        let mut level = floor_level();
        for x in -2..=2 {
            for z in -2..=2 {
                put(&mut level, BlockPos::new(x, 99, z), d::OAK_PLANKS);
            }
        }
        put(&mut level, BlockPos::new(0, 99, 0), d::LAVA);
        let fire_near = |l: &TestLevel| (-2..=2).any(|x| (-2..=2).any(|z| (99..=101).any(|y| state::is(l.block(BlockPos::new(x, y, z)), d::FIRE))));
        let mut lit = false;
        for _ in 0..200 {
            lava_random_tick(&mut level, BlockPos::new(0, 99, 0));
            if fire_near(&level) {
                lit = true;
                break;
            }
        }
        assert!(lit, "lava lit the planks");
    }

    #[test]
    fn fire_faces_and_survival() {
        let mut level = floor_level();
        put(&mut level, BlockPos::new(1, 100, 0), d::OAK_PLANKS);
        // Over the void beside planks: a face toward them.
        let s = fire_state(&level, BlockPos::new(0, 100, 0));
        assert!(state::get_bool(s, "east") && !state::get_bool(s, "west"));
        assert!(fire_can_survive(&level, BlockPos::new(0, 100, 0)));
        assert!(!fire_can_survive(&level, BlockPos::new(-2, 100, 0)));
        // Netherrack: soul fire on soul soil, plain fire elsewhere.
        put(&mut level, BlockPos::new(5, 99, 5), d::SOUL_SOIL);
        assert!(state::is(fire_state(&level, BlockPos::new(5, 100, 5)), d::SOUL_FIRE));
    }
}
