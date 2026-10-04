//! `SpeleothemBlock` (pointed dripstone and sulfur spikes): the falling of unsupported
//! stalactites, the slow growth of stalactites and stalagmites on random ticks, and what a
//! pointed dripstone does with the fluid above its root (mud turns to clay, cauldrons below fill up).

use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{destroy_block, set_block_and_update};
use kiln_data::block_logic::{self as logic, BlockClass as C, FluidKind, Support};
use kiln_data::block_props;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

/// `GROWTH_PROBABILITY_PER_RANDOM_TICK`.
const GROWTH_PROBABILITY: f32 = 0.011377778;
const WATER_TRANSFER_PROBABILITY: f32 = 0.17578125;
const LAVA_TRANSFER_PROBABILITY: f32 = 0.05859375;

fn is_pointed_dripstone(s: u16) -> bool {
    logic::block_class(s) == C::PointedDripstoneBlock
}

/// `blockToGrowOn` of the block (the constructor's argument).
fn block_to_grow_on(s: u16) -> BlockId {
    BlockId::by_name(if is_pointed_dripstone(s) { "minecraft:dripstone_block" } else { "minecraft:sulfur" }).expect("grow-on block")
}

/// `getMaxGrowthLength`.
fn max_growth_length(s: u16) -> i32 {
    if is_pointed_dripstone(s) { 7 } else { 2 }
}

fn tip_direction(s: u16) -> Direction {
    state::get_dir(s, "vertical_direction").unwrap_or(Direction::Up)
}

/// `isSpeleothemWithDirection`.
fn with_direction(s: u16, dir: Direction) -> bool {
    tags::is(s, "minecraft:speleothems") && state::get_dir(s, "vertical_direction") == Some(dir)
}

fn is_stalagmite(s: u16) -> bool {
    with_direction(s, Direction::Up)
}

fn is_stalactite(s: u16) -> bool {
    with_direction(s, Direction::Down)
}

/// `isTip`.
fn is_tip(s: u16, include_merged: bool) -> bool {
    if !tags::is(s, "minecraft:speleothems") {
        return false;
    }
    let t = state::get(s, "thickness");
    t == Some("tip") || (include_merged && t == Some("tip_merge"))
}

/// `isFreeHangingStalactite`: a dry tip pointing down.
fn is_free_hanging_stalactite(s: u16) -> bool {
    is_stalactite(s) && state::get(s, "thickness") == Some("tip") && !state::get_bool(s, "waterlogged")
}

/// `isUnmergedTipWithDirection` of `this` block.
fn is_unmerged_tip_with_direction(this: u16, s: u16, dir: Direction) -> bool {
    is_tip(s, false) && state::get_dir(s, "vertical_direction") == Some(dir) && state::same_block(s, this)
}

fn is_stalactite_start_pos<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    is_stalactite(s) && !state::same_block(level.block(pos.above()), s)
}

/// `isValidSpeleothemPlacement` of `this` block at `pos` for a tip pointing `dir`.
fn valid_placement<L: Level + ?Sized>(level: &L, this: u16, pos: BlockPos, dir: Direction) -> bool {
    let b = level.block(pos.relative(dir.opposite()));
    logic::face_sturdy(b, dir as u8, Support::Full) || (with_direction(b, dir) && state::same_block(b, this))
}

/// `findBlockVertical`: steps up or down from `pos` (at most `max - 1` of them); the first block
/// `stop` accepts is the answer, a block outside the build height or one `keep_going` rejects ends
/// the search without one.
fn find_block_vertical<L: Level + ?Sized>(
    level: &L,
    pos: BlockPos,
    dir: Direction,
    keep_going: impl Fn(BlockPos, u16) -> bool,
    stop: impl Fn(u16) -> bool,
    max: i32,
) -> Option<BlockPos> {
    let mut p = pos;
    for _ in 1..max {
        p = p.relative(dir);
        let s = level.block(p);
        if stop(s) {
            return Some(p);
        }
        let outside = p.y < level.min_y() || p.y >= level.min_y() + level.height();
        if outside || !keep_going(p, s) {
            return None;
        }
    }
    None
}

/// `findTip`.
fn find_tip<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, max: i32, include_merged: bool) -> Option<BlockPos> {
    if is_tip(s, include_merged) {
        return Some(pos);
    }
    let dir = tip_direction(s);
    find_block_vertical(level, pos, dir, |_, b| state::same_block(b, s) && state::get_dir(b, "vertical_direction") == Some(dir), |b| is_tip(b, include_merged), max)
}

// ---------------------------------------------------------------------- falling

/// `SpeleothemBlock.tick` (scheduled when a stalactite lost its support or a stalagmite its
/// base): a stalagmite that cannot stand breaks and drops; a stalactite falls with everything
/// hanging below it.
pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if is_stalagmite(s) && !valid_placement(level, s, pos, tip_direction(s)) {
        destroy_block(level, pos, true, 512);
    } else {
        spawn_falling_stalactite(level, s, pos);
    }
}

/// `spawnFallingStalactite`: each part of the column falls on its own, from the ticked block down
/// to the tip, which hurts what it lands on.
fn spawn_falling_stalactite<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let mut at = pos;
    let mut cur = s;
    while is_stalactite(cur) {
        // `FallingBlockEntity.fall`: the block turns into its fluid (or air) first.
        let spawned = state::set_bool(cur, "waterlogged", false);
        crate::update::set_block(level, at, crate::fluid::legacy_block(logic::fluid(cur)), flags::ALL);
        if is_tip(cur, true) {
            let i = (1 + pos.y - at.y).max(6);
            level.effect(Effect::FallingStalactite { pos: at, state: spawned, per_distance: 1.0 * i as f32 });
            break;
        }
        level.effect(Effect::FallingBlock { pos: at, state: spawned });
        at = at.below();
        cur = level.block(at);
    }
}

// ---------------------------------------------------------------------- growth

/// `SpeleothemBlock.randomTick` (the fluid transfer of pointed dripstone comes first).
pub fn random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if is_pointed_dripstone(s) {
        let f = level.random().next_float();
        maybe_transfer_fluid(level, s, pos, f);
    }
    if level.random().next_float() < GROWTH_PROBABILITY && is_stalactite_start_pos(level, s, pos) {
        grow_stalactite_or_stalagmite_if_possible(level, s, pos);
    }
}

/// `canGrow`: the block two above is the base block (pointed dripstone also needs a water
/// source one block above that).
fn can_grow<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let base = state::same_block(level.block(pos.above()), block_to_grow_on(s).default_state());
    if !is_pointed_dripstone(s) {
        return base;
    }
    let f = logic::fluid(level.block(pos.above().above()));
    base && f.kind == FluidKind::Water && f.source
}

fn grow_stalactite_or_stalagmite_if_possible<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !can_grow(level, s, pos) {
        return;
    }
    let Some(tip) = find_tip(level, s, pos, max_growth_length(s), false) else { return };
    let tip_state = level.block(tip);
    if !(is_free_hanging_stalactite(tip_state) && can_tip_grow(level, s, tip_state, tip)) {
        return;
    }
    if level.random().next_bool() {
        grow(level, s, tip, Direction::Down);
    } else {
        grow_stalagmite_below(level, s, tip);
    }
}

fn can_tip_grow<L: Level + ?Sized>(level: &L, this: u16, tip_state: u16, pos: BlockPos) -> bool {
    let dir = tip_direction(tip_state);
    let grow = level.block(pos.relative(dir));
    if !logic::fluid(grow).is_empty() {
        return false;
    }
    if is_air(grow) {
        return true;
    }
    is_unmerged_tip_with_direction(this, grow, dir.opposite())
}

/// `grow`: a tip meeting the opposite tip merges both; air or water gets a new tip.
fn grow<L: Level>(level: &mut L, this: u16, pos: BlockPos, dir: Direction) {
    let at = pos.relative(dir);
    let s = level.block(at);
    if is_unmerged_tip_with_direction(this, s, dir.opposite()) {
        create_merged_tips(level, this, s, at);
    } else if is_air(s) || state::is(s, kiln_data::blocks::default_state::WATER) {
        create_speleothem(level, this, at, dir, "tip");
    }
}

fn create_speleothem<L: Level>(level: &mut L, this: u16, pos: BlockPos, dir: Direction, thickness: &str) {
    let f = logic::fluid(level.block(pos));
    let base = BlockId::of(this).default_state();
    let s = state::set_bool(state::set(state::set_dir(base, "vertical_direction", dir), "thickness", thickness), "waterlogged", f.kind == FluidKind::Water && f.source);
    set_block_and_update(level, pos, s);
}

fn create_merged_tips<L: Level>(level: &mut L, this: u16, tip_state: u16, pos: BlockPos) {
    let (stalactite, stalagmite) = if tip_direction(tip_state) == Direction::Up { (pos.above(), pos) } else { (pos, pos.below()) };
    create_speleothem(level, this, stalactite, Direction::Down, "tip_merge");
    create_speleothem(level, this, stalagmite, Direction::Up, "tip_merge");
}

/// `growStalagmiteBelow`: looks up to ten blocks down for a tip to extend or a floor to start on.
fn grow_stalagmite_below<L: Level>(level: &mut L, this: u16, pos: BlockPos) {
    let mut at = pos;
    for _ in 0..10 {
        at = at.below();
        let s = level.block(at);
        if !logic::fluid(s).is_empty() {
            return;
        }
        if is_unmerged_tip_with_direction(this, s, Direction::Up) && can_tip_grow(level, this, s, at) {
            grow(level, this, at, Direction::Up);
            return;
        }
        if valid_placement(level, this, at, Direction::Up) && logic::fluid(level.block(at.below())).kind != FluidKind::Water {
            grow(level, this, at.below(), Direction::Up);
            return;
        }
        if blocks_stalagmite_scan(level, this, at, s) {
            return;
        }
    }
}

fn blocks_stalagmite_scan<L: Level + ?Sized>(level: &L, this: u16, pos: BlockPos, s: u16) -> bool {
    is_pointed_dripstone(this) && !can_drip_through(level, pos, s)
}

/// `PointedDripstoneBlock.canDripThrough`: air, or a fluid-free block that is not solid and
/// leaves the 4x4 column in the middle of the block free.
fn can_drip_through<L: Level + ?Sized>(_level: &L, _pos: BlockPos, s: u16) -> bool {
    if is_air(s) {
        return true;
    }
    if block_props::solid_render(s) {
        return false;
    }
    if !logic::fluid(s).is_empty() {
        return false;
    }
    const COLUMN: [f32; 6] = [0.375, 0.0, 0.375, 0.625, 1.0, 0.625];
    !block_props::collision(s).iter().any(|b| (0..3).all(|a| b[a + 3].min(COLUMN[a + 3]) - b[a].max(COLUMN[a]) > 1.0e-7))
}

// ---------------------------------------------------------------------- the fluid above

/// The fluid type a pointed dripstone finds above its root: source water or lava, `None` for
/// anything else (flowing, none).
type DripFluid = Option<FluidKind>;

/// `FluidInfo`: where the fluid was found (the block above the root), what it is and the block there.
struct FluidInfo {
    pos: BlockPos,
    fluid: DripFluid,
    source_state: u16,
}

/// `findRootBlock` then `getFluidAboveStalactite`.
fn fluid_above_stalactite<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16) -> Option<FluidInfo> {
    if !is_stalactite(s) {
        return None;
    }
    let dir = tip_direction(s);
    let root = find_block_vertical(
        level,
        pos,
        dir.opposite(),
        |_, b| state::same_block(b, s) && state::get_dir(b, "vertical_direction") == Some(dir),
        |b| !state::same_block(b, s),
        11,
    )?;
    let above = root.above();
    let st = level.block(above);
    let fluid = if state::is(st, kiln_data::blocks::default_state::MUD) && !level.rules().water_evaporates {
        Some(FluidKind::Water)
    } else {
        let f = logic::fluid(st);
        (f.source && !f.is_empty()).then_some(f.kind)
    };
    Some(FluidInfo { pos: above, fluid, source_state: st })
}

/// `maybeTransferFluid`: water or lava above a stalactite's root drips at the tip; mud under water
/// turns to clay, a cauldron below the tip gets its fill scheduled.
fn maybe_transfer_fluid<L: Level>(level: &mut L, s: u16, pos: BlockPos, random: f32) {
    if random > WATER_TRANSFER_PROBABILITY && random > LAVA_TRANSFER_PROBABILITY {
        return;
    }
    if !is_stalactite_start_pos(level, s, pos) {
        return;
    }
    let Some(info) = fluid_above_stalactite(level, pos, s) else { return };
    let chance = match info.fluid {
        Some(FluidKind::Water) => WATER_TRANSFER_PROBABILITY,
        Some(FluidKind::Lava) => LAVA_TRANSFER_PROBABILITY,
        _ => return,
    };
    if random >= chance {
        return;
    }
    let Some(tip) = find_tip(level, s, pos, 11, false) else { return };
    if state::is(info.source_state, kiln_data::blocks::default_state::MUD) && info.fluid == Some(FluidKind::Water) {
        let clay = kiln_data::blocks::default_state::CLAY;
        set_block_and_update(level, info.pos, clay);
        level.effect(Effect::GameEvent { pos: info.pos, event: "minecraft:block_change" });
        level.effect(Effect::LevelEvent { id: 1504, pos: tip, data: 0 });
        return;
    }
    let Some(cauldron) = find_fillable_cauldron_below(level, tip, info.fluid) else { return };
    level.effect(Effect::LevelEvent { id: 1504, pos: tip, data: 0 });
    let delay = 50 + (tip.y - cauldron.y);
    let id = BlockId::of(level.block(cauldron));
    schedule_block_tick(level, cauldron, id, delay, TickPriority::Normal);
}

/// `AbstractCauldronBlock.canReceiveStalactiteDrip`.
fn cauldron_can_receive(s: u16, fluid: DripFluid) -> bool {
    match logic::block_class(s) {
        C::CauldronBlock => true,
        // Only the water cauldron (rain), not powder snow.
        C::LayeredCauldronBlock => fluid == Some(FluidKind::Water) && state::is(s, kiln_data::blocks::default_state::WATER_CAULDRON),
        _ => false,
    }
}

fn find_fillable_cauldron_below<L: Level + ?Sized>(level: &L, tip: BlockPos, fluid: DripFluid) -> Option<BlockPos> {
    find_block_vertical(
        level,
        tip,
        Direction::Down,
        |p, b| can_drip_through(level, p, b),
        |b| logic::is_instance(b, C::AbstractCauldronBlock) && cauldron_can_receive(b, fluid),
        11,
    )
}

/// `AbstractCauldronBlock.tick` (scheduled by a dripping stalactite): the cauldron takes the
/// drip of the stalactite tip above it, if that still drips.
pub fn cauldron_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let Some(tip) = find_block_vertical(level, pos, Direction::Up, |p, b| can_drip_through(level, p, b), is_free_hanging_stalactite, 11) else { return };
    let tip_state = level.block(tip);
    let fluid = fluid_above_stalactite(level, tip, tip_state).and_then(|i| i.fluid);
    if fluid.is_some() && cauldron_can_receive(s, fluid) {
        receive_drip(level, s, pos, fluid);
    }
}

/// `receiveStalactiteDrip`: an empty cauldron becomes a water or lava cauldron, a water cauldron
/// gains a level.
fn receive_drip<L: Level>(level: &mut L, s: u16, pos: BlockPos, fluid: DripFluid) {
    use kiln_data::blocks::default_state as d;
    match logic::block_class(s) {
        C::CauldronBlock => match fluid {
            Some(FluidKind::Water) => {
                set_block_and_update(level, pos, d::WATER_CAULDRON);
                level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
                level.effect(Effect::LevelEvent { id: 1047, pos, data: 0 });
            }
            Some(FluidKind::Lava) => {
                set_block_and_update(level, pos, d::LAVA_CAULDRON);
                level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
                level.effect(Effect::LevelEvent { id: 1046, pos, data: 0 });
            }
            _ => {}
        },
        C::LayeredCauldronBlock => {
            let l = state::get_int(s, "level");
            if l == 3 {
                return;
            }
            set_block_and_update(level, pos, state::set_int(s, "level", l + 1));
            level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
            level.effect(Effect::LevelEvent { id: 1047, pos, data: 0 });
        }
        _ => {}
    }
}
