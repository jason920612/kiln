//! Water and lava (`FlowingFluid`, `WaterFluid`, `LavaFluid`, `LiquidBlock`, and the
//! `LiquidBlockContainer` side of waterloggable blocks).

use crate::level::{Effect, Level, flags, schedule_block_tick, schedule_fluid_tick};
use crate::pos::{Axis, BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{set_block, set_block_and_update};
use kiln_data::block_logic::{self as logic, BlockClass, Fluid, FluidKind, interface};
use kiln_data::block_props::{self, Aabb};
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

/// A fluid type (`Fluids.*`): the key of fluid ticks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum FluidType {
    Empty,
    FlowingWater,
    Water,
    FlowingLava,
    Lava,
}

impl FluidType {
    pub fn of(f: Fluid) -> Self {
        match (f.kind, f.source) {
            (FluidKind::Empty, _) => Self::Empty,
            (FluidKind::Water, true) => Self::Water,
            (FluidKind::Water, false) => Self::FlowingWater,
            (FluidKind::Lava, true) => Self::Lava,
            (FluidKind::Lava, false) => Self::FlowingLava,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Empty => "minecraft:empty",
            Self::FlowingWater => "minecraft:flowing_water",
            Self::Water => "minecraft:water",
            Self::FlowingLava => "minecraft:flowing_lava",
            Self::Lava => "minecraft:lava",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
            "empty" => Self::Empty,
            "flowing_water" => Self::FlowingWater,
            "water" => Self::Water,
            "flowing_lava" => Self::FlowingLava,
            "lava" => Self::Lava,
            _ => return None,
        })
    }

    pub fn kind(self) -> FluidKind {
        match self {
            Self::Empty => FluidKind::Empty,
            Self::FlowingWater | Self::Water => FluidKind::Water,
            Self::FlowingLava | Self::Lava => FluidKind::Lava,
        }
    }
}

fn source(kind: FluidKind) -> Fluid {
    Fluid { kind, source: true, falling: false, amount: 8 }
}

fn flowing(kind: FluidKind, amount: u8, falling: bool) -> Fluid {
    Fluid { kind, source: false, falling, amount }
}

/// `FluidState.createLegacyBlock`: the water/lava block state holding the fluid (air if
/// empty).
pub fn legacy_block(f: Fluid) -> u16 {
    let block = match f.kind {
        FluidKind::Empty => return d::AIR,
        FluidKind::Water => d::WATER,
        FluidKind::Lava => d::LAVA,
    };
    let level = if f.source { 0 } else { 8 - f.amount.min(8) as i32 + if f.falling { 8 } else { 0 } };
    state::set_int(block, "level", level)
}

pub(crate) fn tick_delay<L: Level + ?Sized>(level: &L, kind: FluidKind) -> i32 {
    match kind {
        FluidKind::Water => 5,
        FluidKind::Lava if level.rules().fast_lava => 10,
        FluidKind::Lava => 30,
        FluidKind::Empty => 0,
    }
}

fn drop_off<L: Level + ?Sized>(level: &L, kind: FluidKind) -> i32 {
    if kind == FluidKind::Lava && !level.rules().fast_lava { 2 } else { 1 }
}

fn slope_find_distance<L: Level + ?Sized>(level: &L, kind: FluidKind) -> i32 {
    if kind == FluidKind::Lava && !level.rules().fast_lava { 2 } else { 4 }
}

fn can_convert_to_source<L: Level + ?Sized>(level: &L, kind: FluidKind) -> bool {
    match kind {
        FluidKind::Water => level.rules().water_source_conversion,
        FluidKind::Lava => level.rules().lava_source_conversion,
        FluidKind::Empty => false,
    }
}

/// `FluidState.getHeight`: 1 under the same fluid, else amount / 9.
fn height<L: Level + ?Sized>(level: &L, f: Fluid, pos: BlockPos) -> f32 {
    if f.kind != FluidKind::Empty && logic::fluid(level.block(pos.above())).kind == f.kind {
        1.0
    } else {
        f.amount as f32 / 9.0
    }
}

/// `Fluid.canBeReplacedWith` for the fluid `f` at `pos` and an incoming fluid `by`.
fn can_be_replaced_with<L: Level + ?Sized>(level: &L, f: Fluid, pos: BlockPos, by: FluidType, dir: Direction) -> bool {
    match f.kind {
        FluidKind::Empty => true,
        FluidKind::Water => dir == Direction::Down && by.kind() != FluidKind::Water,
        FluidKind::Lava => height(level, f, pos) >= 0.44444445 && by.kind() == FluidKind::Water,
    }
}

fn is_container(state: u16) -> bool {
    logic::implements(state, interface::LIQUID_BLOCK_CONTAINER)
}

/// `canHoldAnyFluid`.
fn can_hold_any(state: u16) -> bool {
    is_container(state) || tags::washed_away_by_fluids(state)
}

/// `LiquidBlockContainer.canPlaceLiquid` (no placing entity).
pub fn can_place_liquid(state: u16, fluid: FluidType) -> bool {
    match logic::block_class(state) {
        BlockClass::KelpBlock | BlockClass::KelpPlantBlock | BlockClass::SeagrassBlock | BlockClass::TallSeagrassBlock => false,
        BlockClass::BarrierBlock => false,
        BlockClass::SlabBlock if state::get(state, "type") == Some("double") => false,
        _ => logic::implements(state, interface::SIMPLE_WATERLOGGED_BLOCK) && fluid == FluidType::Water,
    }
}

/// `canHoldSpecificFluid`.
fn can_hold_specific(state: u16, fluid: FluidType) -> bool {
    !is_container(state) || can_place_liquid(state, fluid)
}

fn can_hold(state: u16, fluid: FluidType) -> bool {
    can_hold_any(state) && can_hold_specific(state, fluid)
}

fn full_cube(state: u16) -> bool {
    block_props::full_collision(state)
}

/// Faces of `boxes` in the layer touching the face at `max`/`min` of `axis`, projected on the
/// other two axes.
fn face_slice(boxes: &[Aabb], axis: usize, at_max: bool, out: &mut Vec<[f32; 4]>) {
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let edge = if at_max {
        boxes.iter().map(|b| b[3 + axis]).fold(f32::NEG_INFINITY, f32::max)
    } else {
        boxes.iter().map(|b| b[axis]).fold(f32::INFINITY, f32::min)
    };
    let wanted = if at_max { 1.0 } else { 0.0 };
    if (edge - wanted).abs() > 1e-6 {
        return;
    }
    for b in boxes {
        let touches = if at_max { (b[3 + axis] - edge).abs() < 1e-6 } else { (b[axis] - edge).abs() < 1e-6 };
        if touches {
            out.push([b[u], b[v], b[3 + u], b[3 + v]]);
        }
    }
}

fn covers_unit_square(rects: &[[f32; 4]]) -> bool {
    let mut us = vec![0.0f32, 1.0];
    let mut vs = vec![0.0f32, 1.0];
    for r in rects {
        us.extend([r[0].clamp(0.0, 1.0), r[2].clamp(0.0, 1.0)]);
        vs.extend([r[1].clamp(0.0, 1.0), r[3].clamp(0.0, 1.0)]);
    }
    us.sort_by(f32::total_cmp);
    us.dedup();
    vs.sort_by(f32::total_cmp);
    vs.dedup();
    for u in us.windows(2) {
        let cu = (u[0] + u[1]) / 2.0;
        for v in vs.windows(2) {
            let cv = (v[0] + v[1]) / 2.0;
            if !rects.iter().any(|r| r[0] <= cu && cu <= r[2] && r[1] <= cv && cv <= r[3]) {
                return false;
            }
        }
    }
    true
}

/// `Shapes.mergedFaceOccludes`: together the two collision shapes close the face between
/// them (`a` at the origin, `b` toward `dir`).
fn merged_face_occludes(a: &[Aabb], b: &[Aabb], dir: Direction) -> bool {
    let axis = match dir.axis() {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    let (first, second) = if dir.is_positive() { (a, b) } else { (b, a) };
    let mut rects = Vec::new();
    face_slice(first, axis, true, &mut rects);
    face_slice(second, axis, false, &mut rects);
    covers_unit_square(&rects)
}

/// `FlowingFluid.canPassThroughWall`: fluid can move from `from_state` into the neighbour
/// toward `dir`.
fn can_pass_through_wall(dir: Direction, from_state: u16, to_state: u16) -> bool {
    if full_cube(to_state) || full_cube(from_state) {
        return false;
    }
    let to = block_props::collision(to_state);
    let from = block_props::collision(from_state);
    if to.is_empty() && from.is_empty() {
        return true;
    }
    !merged_face_occludes(from, to, dir)
}

/// The fluid behaviour of one kind of flowing fluid.
struct Flow {
    kind: FluidKind,
}

impl Flow {
    fn is_source_of_this(&self, f: Fluid) -> bool {
        f.kind == self.kind && f.source
    }

    fn flowing_type(&self) -> FluidType {
        FluidType::of(flowing(self.kind, 1, false))
    }

    /// `canMaybePassThrough`.
    fn can_maybe_pass(&self, state: u16, dir: Direction, to_state: u16, to_fluid: Fluid) -> bool {
        !self.is_source_of_this(to_fluid) && can_hold_any(to_state) && can_pass_through_wall(dir, state, to_state)
    }

    /// `canPassThrough`.
    fn can_pass(&self, fluid: FluidType, state: u16, dir: Direction, to_state: u16, to_fluid: Fluid) -> bool {
        self.can_maybe_pass(state, dir, to_state, to_fluid) && can_hold_specific(to_state, fluid)
    }

    /// `isWaterHole`.
    fn is_hole(&self, state: u16, below_state: u16) -> bool {
        if !can_pass_through_wall(Direction::Down, state, below_state) {
            return false;
        }
        logic::fluid(below_state).kind == self.kind || can_hold(below_state, self.flowing_type())
    }

    /// `getNewLiquid`: what the fluid at `pos` (holding `state`) should become.
    fn new_liquid<L: Level + ?Sized>(&self, level: &L, pos: BlockPos, state: u16) -> Fluid {
        let mut amount = 0u8;
        let mut sources = 0;
        for dir in Direction::HORIZONTAL {
            let n = pos.relative(dir);
            let ns = level.block(n);
            let nf = logic::fluid(ns);
            if nf.kind == self.kind && can_pass_through_wall(dir, state, ns) {
                if nf.source {
                    sources += 1;
                }
                amount = amount.max(nf.amount);
            }
        }
        if sources >= 2 && can_convert_to_source(level, self.kind) {
            let below = level.block(pos.below());
            if logic::is_solid(below) || self.is_source_of_this(logic::fluid(below)) {
                return source(self.kind);
            }
        }
        let above = level.block(pos.above());
        let af = logic::fluid(above);
        if af.kind == self.kind && can_pass_through_wall(Direction::Up, state, above) {
            return flowing(self.kind, 8, true);
        }
        let k = amount as i32 - drop_off(level, self.kind);
        if k <= 0 { Fluid::EMPTY } else { flowing(self.kind, k as u8, false) }
    }

    fn spread_delay<L: Level + ?Sized>(&self, level: &mut L, pos: BlockPos, from: Fluid, to: Fluid) -> i32 {
        let delay = tick_delay(level, self.kind);
        if self.kind == FluidKind::Lava
            && !from.is_empty()
            && !to.is_empty()
            && !from.falling
            && !to.falling
            && height(level, to, pos) > height(level, from, pos)
            && level.random().next_int_bounded(4) != 0
        {
            return delay * 4;
        }
        delay
    }

    /// `FlowingFluid.tick`.
    fn tick<L: Level>(&self, level: &mut L, pos: BlockPos, mut state: u16, mut fluid: Fluid) {
        if !fluid.source {
            let new = self.new_liquid(level, pos, level.block(pos));
            let delay = self.spread_delay(level, pos, fluid, new);
            if new.is_empty() {
                fluid = new;
                state = d::AIR;
                set_block_and_update(level, pos, state);
            } else if new != fluid {
                fluid = new;
                state = legacy_block(new);
                set_block_and_update(level, pos, state);
                schedule_fluid_tick(level, pos, FluidType::of(new), delay);
            }
        }
        self.spread(level, pos, state, fluid);
    }

    /// `FlowingFluid.spread`.
    fn spread<L: Level>(&self, level: &mut L, pos: BlockPos, state: u16, fluid: Fluid) {
        if fluid.is_empty() {
            return;
        }
        let below = pos.below();
        let bs = level.block(below);
        let bf = logic::fluid(bs);
        if self.can_maybe_pass(state, Direction::Down, bs, bf) {
            let new = self.new_liquid(level, below, bs);
            let ty = FluidType::of(new);
            if can_be_replaced_with(level, bf, below, ty, Direction::Down) && can_hold_specific(bs, ty) {
                self.spread_to(level, below, bs, Direction::Down, new);
                if self.source_neighbor_count(level, pos) >= 3 {
                    self.spread_to_sides(level, pos, fluid, state);
                }
                return;
            }
        }
        if fluid.source || !self.is_hole(state, bs) {
            self.spread_to_sides(level, pos, fluid, state);
        }
    }

    fn source_neighbor_count<L: Level + ?Sized>(&self, level: &L, pos: BlockPos) -> usize {
        Direction::HORIZONTAL.iter().filter(|&&d| self.is_source_of_this(logic::fluid(level.block(pos.relative(d))))).count()
    }

    /// `spreadToSides`: spreads to the neighbours `getSpread` picks, in `Direction` order.
    fn spread_to_sides<L: Level>(&self, level: &mut L, pos: BlockPos, fluid: Fluid, state: u16) {
        let k = if fluid.falling { 7 } else { fluid.amount as i32 - drop_off(level, self.kind) };
        if k <= 0 {
            return;
        }
        let mut spread = self.get_spread(level, pos, state);
        spread.sort_by_key(|(dir, _)| *dir);
        for (dir, f) in spread {
            let n = pos.relative(dir);
            let ns = level.block(n);
            self.spread_to(level, n, ns, dir, f);
        }
    }

    /// `getSpread`: the horizontal neighbours with the shortest path to a hole.
    fn get_spread<L: Level + ?Sized>(&self, level: &L, pos: BlockPos, state: u16) -> Vec<(Direction, Fluid)> {
        let mut best = 1000;
        let mut out: Vec<(Direction, Fluid)> = Vec::new();
        for dir in Direction::HORIZONTAL {
            let n = pos.relative(dir);
            let ns = level.block(n);
            let nf = logic::fluid(ns);
            if !self.can_maybe_pass(state, dir, ns, nf) {
                continue;
            }
            let new = self.new_liquid(level, n, ns);
            if !can_hold_specific(ns, FluidType::of(new)) {
                continue;
            }
            let dist = if self.is_hole(ns, level.block(n.below())) {
                0
            } else {
                self.slope_distance(level, n, 1, dir.opposite(), ns)
            };
            if dist < best {
                out.clear();
            }
            if dist <= best {
                if can_be_replaced_with(level, nf, n, FluidType::of(new), dir) {
                    out.retain(|(d, _)| *d != dir);
                    out.push((dir, new));
                }
                best = dist;
            }
        }
        out
    }

    /// `getSlopeDistance`.
    fn slope_distance<L: Level + ?Sized>(&self, level: &L, pos: BlockPos, depth: i32, from: Direction, state: u16) -> i32 {
        let mut best = 1000;
        for dir in Direction::HORIZONTAL {
            if dir == from {
                continue;
            }
            let n = pos.relative(dir);
            let ns = level.block(n);
            if self.can_pass(self.flowing_type(), state, dir, ns, logic::fluid(ns)) {
                if self.is_hole(ns, level.block(n.below())) {
                    return depth;
                }
                if depth < slope_find_distance(level, self.kind) {
                    best = best.min(self.slope_distance(level, n, depth + 1, dir.opposite(), ns));
                }
            }
        }
        best
    }

    /// `spreadTo` (with `LavaFluid`'s stone-over-water rule).
    fn spread_to<L: Level>(&self, level: &mut L, pos: BlockPos, state: u16, dir: Direction, fluid: Fluid) {
        if self.kind == FluidKind::Lava && dir == Direction::Down && logic::fluid(level.block(pos)).kind == FluidKind::Water {
            if logic::is_instance(state, BlockClass::LiquidBlock) {
                set_block_and_update(level, pos, d::STONE);
            }
            level.effect(Effect::LevelEvent { id: 1501, pos, data: 0 });
            return;
        }
        if is_container(state) {
            place_liquid(level, pos, state, fluid);
        } else {
            if !kiln_data::blocks_types::is_air(state) {
                match self.kind {
                    FluidKind::Water => level.effect(Effect::Drop { pos, state }),
                    _ => level.effect(Effect::LevelEvent { id: 1501, pos, data: 0 }),
                }
            }
            set_block_and_update(level, pos, legacy_block(fluid));
        }
    }
}

/// `LiquidBlockContainer.placeLiquid`: waterloggable blocks take in source water.
pub fn place_liquid<L: Level>(level: &mut L, pos: BlockPos, state: u16, fluid: Fluid) -> bool {
    let class = logic::block_class(state);
    let takes = match class {
        BlockClass::KelpBlock | BlockClass::KelpPlantBlock | BlockClass::SeagrassBlock | BlockClass::TallSeagrassBlock => false,
        BlockClass::SlabBlock => state::get(state, "type") != Some("double"),
        _ => logic::implements(state, interface::SIMPLE_WATERLOGGED_BLOCK),
    };
    if !takes || state::get_bool(state, "waterlogged") || FluidType::of(fluid) != FluidType::Water {
        return false;
    }
    let s = state::set_bool(state, "waterlogged", true);
    match class {
        BlockClass::CampfireBlock => {
            set_block_and_update(level, pos, state::set_bool(s, "lit", false));
        }
        BlockClass::CandleBlock if state::get_bool(state, "lit") => {
            set_block(level, pos, state::set_bool(s, "lit", false), flags::ALL_IMMEDIATE);
        }
        _ => {
            set_block_and_update(level, pos, s);
        }
    }
    let delay = tick_delay(level, FluidKind::Water);
    schedule_fluid_tick(level, pos, FluidType::Water, delay);
    true
}

/// `FluidState.tick` (a scheduled fluid tick whose type still matches).
pub fn tick<L: Level>(level: &mut L, pos: BlockPos, state: u16) {
    let f = logic::fluid(state);
    if f.is_empty() {
        return;
    }
    Flow { kind: f.kind }.tick(level, pos, state, f);
}

/// `LiquidBlock.shouldSpreadLiquid`: lava next to water hardens (obsidian, cobblestone, or
/// basalt over soul soil next to blue ice) and does not spread.
fn should_spread<L: Level>(level: &mut L, pos: BlockPos, state: u16) -> bool {
    if logic::fluid(state).kind != FluidKind::Lava {
        return true;
    }
    let soul_soil = state::is(level.block(pos.below()), d::SOUL_SOIL);
    for dir in [Direction::Down, Direction::South, Direction::North, Direction::East, Direction::West] {
        let n = pos.relative(dir.opposite());
        if logic::fluid(level.block(n)).kind == FluidKind::Water {
            let hardened = if logic::fluid(level.block(pos)).source { d::OBSIDIAN } else { d::COBBLESTONE };
            set_block_and_update(level, pos, hardened);
            level.effect(Effect::LevelEvent { id: 1501, pos, data: 0 });
            return false;
        }
        if soul_soil && state::is(level.block(n), d::BLUE_ICE) {
            set_block_and_update(level, pos, d::BASALT);
            level.effect(Effect::LevelEvent { id: 1501, pos, data: 0 });
            return false;
        }
    }
    true
}

fn bubble_column_can_occupy(state: u16) -> bool {
    let f = logic::fluid(state);
    f.kind == FluidKind::Water && f.source && f.amount == 8
}

/// `LiquidBlock.tryScheduleBubbleBlockColumn`.
fn schedule_bubble_column<L: Level>(level: &mut L, pos: BlockPos, below: u16) {
    if tags::enables_bubble_column(below) {
        let block = BlockId::of(level.block(pos));
        schedule_block_tick(level, pos, block, 20, TickPriority::Normal);
    }
}

/// `LiquidBlock.onPlace` and `neighborChanged` (same body).
pub fn liquid_block_changed<L: Level>(level: &mut L, state: u16, pos: BlockPos) {
    if should_spread(level, pos, state) {
        let f = logic::fluid(state);
        let delay = tick_delay(level, f.kind);
        schedule_fluid_tick(level, pos, FluidType::of(f), delay);
    }
    if bubble_column_can_occupy(state) {
        let below = level.block(pos.below());
        schedule_bubble_column(level, pos, below);
    }
}

/// `LiquidBlock.updateShape`.
pub fn liquid_update_shape<L: Level>(level: &mut L, state: u16, pos: BlockPos, dir: Direction, neighbor_state: u16) -> u16 {
    let f = logic::fluid(state);
    if f.source || logic::fluid(neighbor_state).source {
        let delay = tick_delay(level, f.kind);
        schedule_fluid_tick(level, pos, FluidType::of(f), delay);
    }
    if dir == Direction::Down && bubble_column_can_occupy(state) {
        schedule_bubble_column(level, pos, neighbor_state);
    }
    state
}

/// Waterlogged blocks re-check their water when a neighbour changes (the first line of most
/// `updateShape` overrides).
pub fn tick_water_if_waterlogged<L: Level>(level: &mut L, state: u16, pos: BlockPos) {
    if state::get_bool(state, "waterlogged") {
        let delay = tick_delay(level, FluidKind::Water);
        schedule_fluid_tick(level, pos, FluidType::Water, delay);
    }
}

/// Replaces a waterlogged block's water with air-equivalent (bucket pickup is the caller's).
pub fn drain_waterlogged<L: Level>(level: &mut L, pos: BlockPos, state: u16) {
    set_block(level, pos, state::set_bool(state, "waterlogged", false), flags::ALL);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_levels() {
        assert_eq!(legacy_block(source(FluidKind::Water)), d::WATER);
        let f = flowing(FluidKind::Water, 5, false);
        assert_eq!(state::get_int(legacy_block(f), "level"), 3);
        assert_eq!(logic::fluid(legacy_block(f)), f);
        let falling = flowing(FluidKind::Lava, 8, true);
        assert_eq!(state::get_int(legacy_block(falling), "level"), 8);
        assert_eq!(logic::fluid(legacy_block(falling)), falling);
        assert_eq!(legacy_block(Fluid::EMPTY), d::AIR);
    }

    #[test]
    fn walls_between_shapes() {
        let slab = state::parse_state("minecraft:stone_slab[type=bottom]").unwrap();
        let top = state::parse_state("minecraft:stone_slab[type=top]").unwrap();
        // Water flows over a bottom slab's top but not through its sides' lower half...
        assert!(can_pass_through_wall(Direction::Up, slab, d::AIR));
        assert!(!can_pass_through_wall(Direction::Down, d::AIR, top));
        assert!(can_pass_through_wall(Direction::North, d::AIR, slab));
        // ... and two bottom slabs side by side still leave the upper half open.
        assert!(can_pass_through_wall(Direction::East, slab, slab));
        assert!(!can_pass_through_wall(Direction::East, d::AIR, d::STONE));
        let fence = state::parse_state("minecraft:oak_fence").unwrap();
        assert!(can_pass_through_wall(Direction::East, fence, d::AIR));
        let stairs = state::parse_state("minecraft:oak_stairs[facing=east,half=bottom,shape=straight]").unwrap();
        // The full back of the stairs (east) closes the face; the front does not.
        assert!(!can_pass_through_wall(Direction::West, d::AIR, stairs));
        assert!(can_pass_through_wall(Direction::East, d::AIR, stairs));
    }
}
