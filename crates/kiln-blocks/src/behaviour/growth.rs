//! Plants that grow and spread on their own: kelp, weeping, twisting and cave vines
//! (`GrowingPlantBlock`), vines, mushrooms, nylium and chorus. Each function follows the vanilla
//! method call by call: the order and number of random draws is part of the behaviour.

use super::spread::light_dampening_into;
use super::sturdy;
use crate::fluid::FluidType;
use crate::level::{Effect, Level, flags, schedule_block_tick, schedule_fluid_tick};
use crate::pos::{Axis, BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use kiln_data::block_logic::{self as logic, BlockClass, Support};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

fn empty<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    is_air(level.block(pos))
}

/// `Direction.getRandom`: `Util.getRandom(values(), random)`.
fn random_direction<R: RandomSource + ?Sized>(random: &mut R) -> Direction {
    Direction::ALL[random.next_int_bounded(6) as usize]
}

/// `Direction.Plane.HORIZONTAL.getRandomDirection`.
fn random_horizontal<R: RandomSource + ?Sized>(random: &mut R) -> Direction {
    Direction::HORIZONTAL[random.next_int_bounded(4) as usize]
}

/// `Block.isFaceFull(state.getCollisionShape(..), dir)`: some collision box fills the whole face.
fn collision_face_full(s: u16, dir: Direction) -> bool {
    let axis = dir.axis() as usize;
    let at_max = dir.is_positive();
    block_props::collision(s).iter().any(|b| {
        let touches = if at_max { b[3 + axis] >= 1.0 } else { b[axis] <= 0.0 };
        touches && (0..3).filter(|&a| a != axis).all(|a| b[a] <= 0.0 && b[3 + a] >= 1.0)
    })
}

/// `MultifaceBlock.canAttachTo(level, dir, pos, state)` (what `VineBlock.isAcceptableNeighbour`
/// reads): the neighbour at `pos` (toward `dir`) offers a full face back.
fn acceptable_neighbour<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> bool {
    let n = level.block(pos);
    sturdy(n, dir.opposite(), Support::Full) || collision_face_full(n, dir.opposite())
}

// ---------------------------------------------------------------- growing plants

/// What a `GrowingPlantBlock` class is: its growth direction, head and body blocks, whether it
/// keeps water flowing, and (heads) the chance to grow per random tick.
struct Plant {
    growth: Direction,
    head: u16,
    body: u16,
    fluid_ticks: bool,
    chance: f64,
    is_head: bool,
    cave: bool,
}

fn plant(s: u16) -> Option<Plant> {
    use BlockClass as C;
    let (growth, head, body, fluid_ticks, chance, is_head, cave) = match logic::block_class(s) {
        C::KelpBlock => (Direction::Up, d::KELP, d::KELP_PLANT, true, 0.14, true, false),
        C::KelpPlantBlock => (Direction::Up, d::KELP, d::KELP_PLANT, true, 0.14, false, false),
        C::WeepingVinesBlock => (Direction::Down, d::WEEPING_VINES, d::WEEPING_VINES_PLANT, false, 0.1, true, false),
        C::WeepingVinesPlantBlock => (Direction::Down, d::WEEPING_VINES, d::WEEPING_VINES_PLANT, false, 0.1, false, false),
        C::TwistingVinesBlock => (Direction::Up, d::TWISTING_VINES, d::TWISTING_VINES_PLANT, false, 0.1, true, false),
        C::TwistingVinesPlantBlock => (Direction::Up, d::TWISTING_VINES, d::TWISTING_VINES_PLANT, false, 0.1, false, false),
        C::CaveVinesBlock => (Direction::Down, d::CAVE_VINES, d::CAVE_VINES_PLANT, false, 0.1, true, true),
        C::CaveVinesPlantBlock => (Direction::Down, d::CAVE_VINES, d::CAVE_VINES_PLANT, false, 0.1, false, true),
        _ => return None,
    };
    Some(Plant { growth, head, body, fluid_ticks, chance, is_head, cave })
}

/// Whether `s` is a growing plant (`GrowingPlantBlock`).
pub fn is_growing_plant(s: u16) -> bool {
    plant(s).is_some()
}

/// `GrowingPlantBlock.canAttachTo` (kelp refuses `#cannot_support_kelp`).
fn plant_can_attach_to(p: &Plant, below: u16) -> bool {
    if state::same_block(p.head, d::KELP) { !tags::is(below, "minecraft:cannot_support_kelp") } else { true }
}

/// `GrowingPlantBlock.canSurvive`.
pub fn plant_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let Some(p) = plant(s) else { return true };
    let behind = pos.relative(p.growth.opposite());
    let b = level.block(behind);
    if !plant_can_attach_to(&p, b) {
        return false;
    }
    state::same_block(b, p.head) || state::same_block(b, p.body) || sturdy(b, p.growth, Support::Full)
}

/// `GrowingPlantBlock.tick`: a plant that lost its support is destroyed.
pub fn plant_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !plant_can_survive(level, s, pos) {
        crate::destroy_block(level, pos, true, flags::LIMIT);
    }
}

/// `updateBodyAfterConvertedFromHead` / `updateHeadAfterConvertedFromBody`: cave vines keep
/// their berries across the conversion.
fn convert(p: &Plant, from: u16, to: u16) -> u16 {
    if p.cave { state::set_bool(to, "berries", state::get_bool(from, "berries")) } else { to }
}

/// `GrowingPlantHeadBlock.updateShape` / `GrowingPlantBodyBlock.updateShape`.
pub fn plant_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    let Some(p) = plant(s) else { return s };
    let id = BlockId::of(s);
    let body_default = BlockId::of(p.body).default_state();
    if p.is_head {
        if dir == p.growth.opposite() {
            if !plant_can_survive(level, s, pos) {
                schedule_block_tick(level, pos, id, 1, TickPriority::Normal);
            } else {
                let forward = level.block(pos.relative(p.growth));
                if state::same_block(forward, s) || state::same_block(forward, p.body) {
                    return convert(&p, s, body_default);
                }
            }
        }
        if dir == p.growth && (state::same_block(neighbor, s) || state::same_block(neighbor, p.body)) {
            return convert(&p, s, body_default);
        }
    } else {
        if dir == p.growth.opposite() && !plant_can_survive(level, s, pos) {
            schedule_block_tick(level, pos, id, 1, TickPriority::Normal);
        }
        if dir == p.growth && !state::same_block(neighbor, s) && !state::same_block(neighbor, p.head) {
            // `getHeadBlock().getStateForPlacement(random)`: a random age.
            let age = level.random().next_int_bounded(25);
            let head = state::set_int(BlockId::of(p.head).default_state(), "age", age);
            return convert(&p, s, head);
        }
    }
    if p.fluid_ticks {
        schedule_fluid_tick(level, pos, FluidType::Water, 5);
    }
    s
}

/// `canGrowInto` of the head block.
fn can_grow_into(p: &Plant, s: u16) -> bool {
    if state::same_block(p.head, d::KELP) { state::is(s, d::WATER) } else { is_air(s) }
}

/// `GrowingPlantHeadBlock.randomTick`.
pub fn plant_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let Some(p) = plant(s) else { return };
    if !p.is_head || state::get_int(s, "age") >= 25 {
        return;
    }
    if level.random().next_double() < p.chance {
        let target = pos.relative(p.growth);
        if can_grow_into(&p, level.block(target)) {
            let mut grown = state::set_int(s, "age", state::get_int(s, "age") + 1);
            if p.cave {
                grown = state::set_bool(grown, "berries", level.random().next_float() < 0.11);
            }
            crate::set_block_and_update(level, target, grown);
        }
    }
}

// ---------------------------------------------------------------- vines

fn vine_face(s: u16, dir: Direction) -> bool {
    state::get_bool(s, dir.name())
}

fn set_vine_face(s: u16, dir: Direction, on: bool) -> u16 {
    state::set_bool(s, dir.name(), on)
}

fn vine_faces(s: u16) -> bool {
    [Direction::Up, Direction::North, Direction::East, Direction::South, Direction::West].iter().any(|&d| vine_face(s, d))
}

/// `VineBlock.canSupportAtFace`.
fn vine_can_support_at_face<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> bool {
    if dir == Direction::Down {
        return false;
    }
    if acceptable_neighbour(level, pos.relative(dir), dir) {
        return true;
    }
    if dir.axis() != Axis::Y {
        let above = level.block(pos.above());
        return state::is(above, d::VINE) && vine_face(above, dir);
    }
    false
}

/// `VineBlock.getUpdatedState`.
fn vine_updated_state<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> u16 {
    let above = pos.above();
    let mut s = s;
    if vine_face(s, Direction::Up) {
        s = set_vine_face(s, Direction::Up, acceptable_neighbour(level, above, Direction::Down));
    }
    let mut above_state = None;
    for dir in Direction::HORIZONTAL {
        if vine_face(s, dir) {
            let mut supported = vine_can_support_at_face(level, pos, dir);
            if !supported {
                let a = *above_state.get_or_insert_with(|| level.block(above));
                supported = state::is(a, d::VINE) && vine_face(a, dir);
            }
            s = set_vine_face(s, dir, supported);
        }
    }
    s
}

/// `VineBlock.canSurvive`.
pub fn vine_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    vine_faces(vine_updated_state(level, s, pos))
}

/// `VineBlock.updateShape`.
pub fn vine_update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if dir == Direction::Down {
        return s;
    }
    let updated = vine_updated_state(level, s, pos);
    if vine_faces(updated) { updated } else { d::AIR }
}

fn has_horizontal_connection(s: u16) -> bool {
    Direction::HORIZONTAL.iter().any(|&d| vine_face(s, d))
}

/// `VineBlock.canSpread`: at most four vines in the 9x3x9 box around it.
fn vine_can_spread<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    at_most_matched(level, pos, 4, |s| state::is(s, d::VINE))
}

/// `findBlocksIn(pos - (4, 1, 4), pos + (4, 1, 4)).filterState(f).atMostMatched(n)`.
fn at_most_matched<L: Level + ?Sized>(level: &L, pos: BlockPos, n: usize, f: impl Fn(u16) -> bool) -> bool {
    let mut count = 0;
    for y in -1..=1 {
        for z in -4..=4 {
            for x in -4..=4 {
                if f(level.block(pos.offset(x, y, z))) {
                    count += 1;
                    if count > n {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// `VineBlock.copyRandomFaces`.
fn copy_random_faces<L: Level>(level: &mut L, from: u16, to: u16) -> u16 {
    let mut to = to;
    for dir in Direction::HORIZONTAL {
        if level.random().next_bool() && vine_face(from, dir) {
            to = set_vine_face(to, dir, true);
        }
    }
    to
}

/// `VineBlock.randomTick`.
pub fn vine_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !level.rules().spread_vines {
        return;
    }
    if level.random().next_int_bounded(4) != 0 {
        return;
    }
    let dir = random_direction(level.random());
    let above = pos.above();
    let set = |level: &mut L, p: BlockPos, st: u16| {
        crate::set_block(level, p, st, flags::CLIENTS);
    };
    let default = d::VINE;
    if dir.is_horizontal() && !vine_face(s, dir) {
        if !vine_can_spread(level, pos) {
            return;
        }
        let target = pos.relative(dir);
        let ts = level.block(target);
        if is_air(ts) {
            let (cw, ccw) = (dir.clockwise(), dir.counter_clockwise());
            let (cw_has, ccw_has) = (vine_face(s, cw), vine_face(s, ccw));
            let (cw_pos, ccw_pos) = (target.relative(cw), target.relative(ccw));
            if cw_has && acceptable_neighbour(level, cw_pos, cw) {
                set(level, target, set_vine_face(default, cw, true));
            } else if ccw_has && acceptable_neighbour(level, ccw_pos, ccw) {
                set(level, target, set_vine_face(default, ccw, true));
            } else {
                let opposite = dir.opposite();
                if cw_has && empty(level, cw_pos) && acceptable_neighbour(level, pos.relative(cw), opposite) {
                    set(level, cw_pos, set_vine_face(default, opposite, true));
                } else if ccw_has && empty(level, ccw_pos) && acceptable_neighbour(level, pos.relative(ccw), opposite) {
                    set(level, ccw_pos, set_vine_face(default, opposite, true));
                } else if (level.random().next_float() as f64) < 0.05 && acceptable_neighbour(level, target.above(), Direction::Up) {
                    set(level, target, set_vine_face(default, Direction::Up, true));
                }
            }
        } else if acceptable_neighbour(level, target, dir) {
            set(level, pos, set_vine_face(s, dir, true));
        }
        return;
    }
    if dir == Direction::Up && pos.y < level.min_y() + level.height() - 1 {
        if vine_can_support_at_face(level, pos, dir) {
            set(level, pos, set_vine_face(s, Direction::Up, true));
            return;
        }
        if empty(level, above) {
            if !vine_can_spread(level, pos) {
                return;
            }
            let mut ns = s;
            for d in Direction::HORIZONTAL {
                if level.random().next_bool() || !acceptable_neighbour(level, above.relative(d), d) {
                    ns = set_vine_face(ns, d, false);
                }
            }
            if has_horizontal_connection(ns) {
                set(level, above, ns);
            }
            return;
        }
    }
    if pos.y > level.min_y() {
        let below = pos.below();
        let bs = level.block(below);
        if is_air(bs) || state::is(bs, d::VINE) {
            let base = if is_air(bs) { default } else { bs };
            let copied = copy_random_faces(level, s, base);
            if base != copied && has_horizontal_connection(copied) {
                set(level, below, copied);
            }
        }
    }
}

// ---------------------------------------------------------------- mushrooms and nylium

/// `MushroomBlock.randomTick`: spreads to a nearby spot in the dark, unless crowded.
pub fn mushroom_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.random().next_int_bounded(25) != 0 {
        return;
    }
    if !at_most_matched(level, pos, 4, |b| state::same_block(b, s)) {
        return;
    }
    let mut pos = pos;
    let offset = |level: &mut L, from: BlockPos| {
        let dx = level.random().next_int_bounded(3) - 1;
        let a = level.random().next_int_bounded(2);
        let b = level.random().next_int_bounded(2);
        let dz = level.random().next_int_bounded(3) - 1;
        from.offset(dx, a - b, dz)
    };
    let mut target = offset(level, pos);
    for _ in 0..4 {
        if empty(level, target) && super::support::can_survive(level, s, target) {
            pos = target;
        }
        target = offset(level, pos);
    }
    if empty(level, target) && super::support::can_survive(level, s, target) {
        crate::set_block(level, target, s, flags::CLIENTS);
    }
}

/// `NyliumBlock.randomTick`: covered nylium turns back into netherrack.
pub fn nylium_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let above = level.block(pos.above());
    if light_dampening_into(s, above, Direction::Up) >= 15 {
        crate::set_block_and_update(level, pos, d::NETHERRACK);
    }
}

// ---------------------------------------------------------------- chorus

fn chorus_supports_flower(s: u16) -> bool {
    tags::is(s, "minecraft:supports_chorus_flower")
}

fn chorus_supports_plant(s: u16) -> bool {
    tags::is(s, "minecraft:supports_chorus_plant")
}

/// `ChorusFlowerBlock.canSurvive`.
pub fn chorus_flower_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let below = level.block(pos.below());
    if state::is(below, d::CHORUS_PLANT) || chorus_supports_flower(below) {
        return true;
    }
    if !is_air(below) {
        return false;
    }
    let mut found = false;
    for dir in Direction::HORIZONTAL {
        let n = level.block(pos.relative(dir));
        if state::is(n, d::CHORUS_PLANT) {
            if found {
                return false;
            }
            found = true;
        } else if !is_air(n) {
            return false;
        }
    }
    found
}

/// `ChorusPlantBlock.canSurvive`.
pub fn chorus_plant_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let below = level.block(pos.below());
    let both = !empty(level, pos.above()) && !is_air(below);
    for dir in Direction::HORIZONTAL {
        let n = pos.relative(dir);
        if state::is(level.block(n), d::CHORUS_PLANT) {
            if both {
                return false;
            }
            let nb = level.block(n.below());
            if state::is(nb, d::CHORUS_PLANT) || chorus_supports_plant(nb) {
                return true;
            }
        }
    }
    state::is(below, d::CHORUS_PLANT) || chorus_supports_plant(below)
}

/// `ChorusPlantBlock.getStateWithConnections`.
pub fn chorus_plant_connections<L: Level + ?Sized>(level: &L, pos: BlockPos, plant: u16) -> u16 {
    let connects = |s: u16| state::is(s, d::CHORUS_PLANT) || state::is(s, d::CHORUS_FLOWER);
    let mut s = plant;
    let below = level.block(pos.below());
    s = state::set_bool(s, "down", connects(below) || chorus_supports_plant(below));
    s = state::set_bool(s, "up", connects(level.block(pos.above())));
    for dir in [Direction::North, Direction::East, Direction::South, Direction::West] {
        s = state::set_bool(s, dir.name(), connects(level.block(pos.relative(dir))));
    }
    s
}

/// `ChorusPlantBlock.updateShape`.
pub fn chorus_plant_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    if !chorus_plant_can_survive(level, pos) {
        schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
        return s;
    }
    let connected = state::is(neighbor, d::CHORUS_PLANT) || state::is(neighbor, d::CHORUS_FLOWER) || (dir == Direction::Down && chorus_supports_plant(neighbor));
    state::set_bool(s, dir.name(), connected)
}

/// `ChorusFlowerBlock.updateShape`: a flower that lost its support is re-checked next tick.
pub fn chorus_flower_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if dir != Direction::Up && !chorus_flower_can_survive(level, pos) {
        schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
    }
    s
}

/// `ChorusFlowerBlock.tick` / `ChorusPlantBlock.tick`.
pub fn chorus_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let survives = if state::is(s, d::CHORUS_FLOWER) { chorus_flower_can_survive(level, pos) } else { chorus_plant_can_survive(level, pos) };
    if !survives {
        crate::destroy_block(level, pos, true, flags::LIMIT);
    }
}

/// `ChorusFlowerBlock.allNeighborsEmpty`: the horizontal neighbours but `except`.
fn all_neighbours_empty<L: Level + ?Sized>(level: &L, pos: BlockPos, except: Option<Direction>) -> bool {
    Direction::HORIZONTAL.iter().all(|&d| Some(d) == except || empty(level, pos.relative(d)))
}

fn place_grown_flower<L: Level>(level: &mut L, pos: BlockPos, age: i32) {
    crate::set_block(level, pos, state::set_int(d::CHORUS_FLOWER, "age", age), flags::CLIENTS);
    level.effect(Effect::LevelEvent { id: 1033, pos, data: 0 });
}

fn place_dead_flower<L: Level>(level: &mut L, pos: BlockPos) {
    crate::set_block(level, pos, state::set_int(d::CHORUS_FLOWER, "age", 5), flags::CLIENTS);
    level.effect(Effect::LevelEvent { id: 1034, pos, data: 0 });
}

/// `ChorusFlowerBlock.randomTick`.
pub fn chorus_flower_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let above = pos.above();
    if !(empty(level, above) && above.y <= level.min_y() + level.height() - 1) {
        return;
    }
    let age = state::get_int(s, "age");
    if age >= 5 {
        return;
    }
    let (mut can_grow_up, mut stopped_by_end_stone) = (false, false);
    let below = level.block(pos.below());
    if chorus_supports_flower(below) {
        can_grow_up = true;
    } else if state::is(below, d::CHORUS_PLANT) {
        let mut height = 1;
        for _ in 0..4 {
            let b = level.block(pos.relative_by(Direction::Down, height + 1));
            if state::is(b, d::CHORUS_PLANT) {
                height += 1;
            } else {
                if chorus_supports_flower(b) {
                    stopped_by_end_stone = true;
                }
                break;
            }
        }
        if height < 2 || height <= level.random().next_int_bounded(if stopped_by_end_stone { 5 } else { 4 }) {
            can_grow_up = true;
        }
    } else if is_air(below) {
        can_grow_up = true;
    }
    if can_grow_up && all_neighbours_empty(level, above, None) && empty(level, pos.relative_by(Direction::Up, 2)) {
        let plant = chorus_plant_connections(level, pos, d::CHORUS_PLANT);
        crate::set_block(level, pos, plant, flags::CLIENTS);
        place_grown_flower(level, above, age);
    } else if age < 4 {
        let mut n = level.random().next_int_bounded(4);
        if stopped_by_end_stone {
            n += 1;
        }
        let mut placed = false;
        for _ in 0..n {
            let dir = random_horizontal(level.random());
            let t = pos.relative(dir);
            if empty(level, t) && empty(level, t.below()) && all_neighbours_empty(level, t, Some(dir.opposite())) {
                place_grown_flower(level, t, age + 1);
                placed = true;
            }
        }
        if placed {
            let plant = chorus_plant_connections(level, pos, d::CHORUS_PLANT);
            crate::set_block(level, pos, plant, flags::CLIENTS);
        } else {
            place_dead_flower(level, pos);
        }
    } else {
        place_dead_flower(level, pos);
    }
}
