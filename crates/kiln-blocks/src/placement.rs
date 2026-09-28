//! Placing block items (`BlockItem.place`, `BlockPlaceContext`, `getStateForPlacement`,
//! `StandingAndWallBlockItem`, `setPlacedBy`).
//!
//! Blocks without a specific rule get the common vanilla pattern from their properties:
//! `axis` from the clicked face, horizontal `facing` against the player, six-way `facing`
//! toward the player, `waterlogged` in source water.

use crate::behaviour::{connect, container, misc, support};
use crate::level::{Level, flags};
use crate::pos::{Axis, BlockPos, Direction};
use crate::redstone::{diode, has_neighbor_signal, wire};
use crate::state::{self, BlockId};
use crate::update::{set_block, set_block_and_update};
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind};
use kiln_data::block_props;
use std::sync::OnceLock;

/// Where and how the player clicked.
#[derive(Clone, Copy, Debug)]
pub struct PlaceContext {
    /// The block the player's ray hit.
    pub hit: BlockPos,
    pub face: Direction,
    /// The exact hit location (world coordinates).
    pub click: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub sneaking: bool,
}

/// What a block item places: its block, and for standing-and-wall items the wall block and
/// the direction the standing block attaches toward (down for torches, up for hanging signs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockItem {
    pub block: BlockId,
    pub wall: Option<(BlockId, Direction)>,
}

impl BlockItem {
    /// By item id (`minecraft:torch`), from the extracted item table.
    pub fn of_item(item: &str) -> Option<Self> {
        let (block, wall) = logic::block_item(item)?;
        let wall = wall.and_then(|(w, d)| Some((BlockId::by_name(w)?, Direction::from_name(d)?)));
        Some(Self { block: BlockId::by_name(block)?, wall })
    }
}

struct Ctx<'a, L: ?Sized> {
    level: &'a L,
    pos: BlockPos,
    replace_clicked: bool,
    face: Direction,
    click: [f64; 3],
    yaw: f32,
    pitch: f32,
    sneaking: bool,
}

/// `Mth.sin` / `Mth.cos` (the 65536-entry table, double-argument versions).
fn mth_table(x: f32, offset: f64) -> f32 {
    static SIN: OnceLock<Vec<f32>> = OnceLock::new();
    let t = SIN.get_or_init(|| (0..65536).map(|i| (i as f64 * std::f64::consts::PI * 2.0 / 65536.0).sin() as f32).collect());
    t[((x as f64 * 10430.378350470453 + offset) as i64 & 65535) as usize]
}

fn mth_sin(x: f32) -> f32 {
    mth_table(x, 0.0)
}

fn mth_cos(x: f32) -> f32 {
    mth_table(x, 16384.0)
}

/// `Direction.orderedByNearest`: the six directions by how directly the player looks along them.
fn ordered_by_nearest(yaw: f32, pitch: f32) -> [Direction; 6] {
    let f = pitch * 0.017453292;
    let g = -yaw * 0.017453292;
    let (h, i, j, k) = (mth_sin(f), mth_cos(f), mth_sin(g), mth_cos(g));
    let (east, up, south) = (j > 0.0, h < 0.0, k > 0.0);
    let l = if east { j } else { -j };
    let m = if up { -h } else { h };
    let n = if south { k } else { -k };
    let o = l * i;
    let p = n * i;
    let d1 = if east { Direction::East } else { Direction::West };
    let d2 = if up { Direction::Up } else { Direction::Down };
    let d3 = if south { Direction::South } else { Direction::North };
    let make = |a: Direction, b: Direction, c: Direction| [a, b, c, c.opposite(), b.opposite(), a.opposite()];
    if l > n {
        if m > o {
            make(d2, d1, d3)
        } else if p > m {
            make(d1, d3, d2)
        } else {
            make(d1, d2, d3)
        }
    } else if m > p {
        make(d2, d3, d1)
    } else if o > m {
        make(d3, d1, d2)
    } else {
        make(d3, d2, d1)
    }
}

impl<L: Level + ?Sized> Ctx<'_, L> {
    /// `getHorizontalDirection`.
    fn horizontal(&self) -> Direction {
        Direction::from_yaw(self.yaw as f64)
    }

    /// `getNearestLookingDirections`: when placing against a face, the face's opposite first.
    fn nearest(&self) -> [Direction; 6] {
        let mut dirs = ordered_by_nearest(self.yaw, self.pitch);
        if !self.replace_clicked {
            let want = self.face.opposite();
            let i = dirs.iter().position(|&d| d == want).unwrap_or(0);
            dirs.copy_within(0..i, 1);
            dirs[0] = want;
        }
        dirs
    }

    fn in_water_source(&self) -> bool {
        let f = logic::fluid(self.level.block(self.pos));
        f.kind == FluidKind::Water && f.source
    }

    fn click_upper_half(&self) -> bool {
        self.click[1] - self.pos.y as f64 > 0.5
    }

    fn waterlogged(&self, s: u16) -> u16 {
        state::set_bool(s, "waterlogged", self.in_water_source())
    }
}

/// `BlockState.canBeReplaced(BlockPlaceContext)` for the block at the placing position.
fn can_be_replaced<L: Level + ?Sized>(level: &L, s: u16, item: BlockId, face: Direction, click_y: f64, pos: BlockPos, replace_clicked: bool) -> bool {
    if logic::block_class(s) == BlockClass::SlabBlock {
        let ty = state::get(s, "type");
        if ty == Some("double") || BlockId::of(s) != item {
            return false;
        }
        if !replace_clicked {
            return true;
        }
        let upper = click_y - pos.y as f64 > 0.5;
        return if ty == Some("bottom") {
            face == Direction::Up || upper && face.is_horizontal()
        } else {
            face == Direction::Down || !upper && face.is_horizontal()
        };
    }
    let _ = level;
    block_props::replaceable(s) && BlockId::of(s) != item
}

/// `getStateForPlacement` of `block` in context.
fn state_for_placement<L: Level + ?Sized>(c: &Ctx<L>, block: BlockId) -> Option<u16> {
    use BlockClass as C;
    let d = block.default_state();
    let level = c.level;
    let pos = c.pos;
    let class = logic::block_class(d);
    let s = match class {
        C::StairBlock | C::WeatheringCopperStairBlock => {
            let half = if c.face != Direction::Down && (c.face == Direction::Up || !c.click_upper_half()) { "bottom" } else { "top" };
            let s = c.waterlogged(state::set(state::set_dir(d, "facing", c.horizontal()), "half", half));
            state::set(s, "shape", connect::stairs_shape(level, s, pos))
        }
        C::SlabBlock | C::WeatheringCopperSlabBlock => {
            let here = level.block(pos);
            if state::same_block(here, d) {
                state::set_bool(state::set(here, "type", "double"), "waterlogged", false)
            } else {
                let s = c.waterlogged(state::set(d, "type", "bottom"));
                let top = c.face == Direction::Down || c.face != Direction::Up && c.click_upper_half();
                if top { state::set(s, "type", "top") } else { s }
            }
        }
        C::WallBlock => connect::wall_placement(level, c.waterlogged(d), pos),
        C::FenceGateBlock => {
            let facing = c.horizontal();
            let wall = |p: BlockPos| crate::tags::is(level.block(p), "minecraft:walls");
            let in_wall = match facing.axis() {
                Axis::Z => wall(pos.relative(Direction::West)) || wall(pos.relative(Direction::East)),
                _ => wall(pos.relative(Direction::North)) || wall(pos.relative(Direction::South)),
            };
            let powered = has_neighbor_signal(level, pos);
            let s = state::set_bool(state::set_bool(state::set_dir(d, "facing", facing), "open", powered), "powered", powered);
            state::set_bool(s, "in_wall", in_wall)
        }
        _ if logic::is_instance(d, C::DoorBlock) => {
            if !level.in_bounds(pos.above()) || !block_props::replaceable(level.block(pos.above())) {
                return None;
            }
            let powered = has_neighbor_signal(level, pos) || has_neighbor_signal(level, pos.above());
            let s = state::set_dir(d, "facing", c.horizontal());
            let s = state::set(s, "hinge", door_hinge(c));
            let s = state::set_bool(state::set_bool(s, "powered", powered), "open", powered);
            state::set(s, "half", "lower")
        }
        _ if logic::is_instance(d, C::TrapDoorBlock) => {
            let mut s = if c.replace_clicked || !c.face.is_horizontal() {
                let s = state::set_dir(d, "facing", c.horizontal().opposite());
                state::set(s, "half", if c.face == Direction::Up { "bottom" } else { "top" })
            } else {
                let s = state::set_dir(d, "facing", c.face);
                state::set(s, "half", if c.click_upper_half() { "top" } else { "bottom" })
            };
            if has_neighbor_signal(level, pos) {
                s = state::set_bool(state::set_bool(s, "open", true), "powered", true);
            }
            c.waterlogged(s)
        }
        _ if logic::is_instance(d, C::DoublePlantBlock) => {
            if !level.in_bounds(pos.above()) || !block_props::replaceable(level.block(pos.above())) {
                return None;
            }
            generic(c, d)
        }
        _ if logic::is_instance(d, C::FenceBlock) || logic::is_instance(d, C::IronBarsBlock) => {
            connect::cross_placement(level, c.waterlogged(d), pos)
        }
        C::WallTorchBlock | C::RedstoneWallTorchBlock | C::LadderBlock => {
            let here = level.block(pos.relative(c.face.opposite()));
            if class == C::LadderBlock && !c.replace_clicked && state::same_block(here, d) && state::get_dir(here, "facing") == Some(c.face) {
                return None;
            }
            let found = c.nearest().into_iter().filter(|dir| dir.is_horizontal()).map(|dir| state::set_dir(d, "facing", dir.opposite())).find(|&s| support::can_survive(level, s, pos))?;
            if class == C::LadderBlock { c.waterlogged(found) } else { found }
        }
        C::LeverBlock | C::ButtonBlock => {
            return c.nearest().into_iter().find_map(|dir| {
                let s = if dir.axis() == Axis::Y {
                    let face = if dir == Direction::Up { "ceiling" } else { "floor" };
                    state::set_dir(state::set(d, "face", face), "facing", c.horizontal())
                } else {
                    state::set_dir(state::set(d, "face", "wall"), "facing", dir.opposite())
                };
                support::can_survive(level, s, pos).then_some(s)
            });
        }
        C::RepeaterBlock | C::ComparatorBlock => diode::placement(level, pos, state::set_dir(d, "facing", c.horizontal().opposite())),
        C::RedstoneWireBlock => wire::placement(level, pos),
        C::HopperBlock => container::hopper_placement(d, c.face),
        C::ShulkerBoxBlock => container::shulker_placement(d, c.face),
        _ if container::is_chest(d) => c.waterlogged(container::chest_placement(level, d, pos, c.horizontal(), c.face, c.sneaking)),
        C::ObserverBlock => state::set_dir(d, "facing", c.nearest()[0]),
        C::RedstoneLampBlock => state::set_bool(d, "lit", has_neighbor_signal(level, pos)),
        C::NoteBlock => crate::redstone::devices::note_instrument(level, pos, d),
        C::RailBlock | C::PoweredRailBlock | C::DetectorRailBlock => {
            crate::behaviour::rail::placement(d, c.horizontal(), c.in_water_source())
        }
        C::BedBlock | C::StrawBedBlock => {
            let facing = c.horizontal();
            if !block_props::replaceable(level.block(pos.relative(facing))) {
                return None;
            }
            state::set_dir(d, "facing", facing)
        }
        _ if logic::is_instance(d, C::SnowyBlock) => state::set_bool(d, "snowy", connect::snowy_setting(level.block(pos.above()))),
        _ if logic::is_instance(d, C::LeavesBlock) => misc::leaves_distance(level, c.waterlogged(state::set_bool(d, "persistent", true)), pos),
        _ if logic::is_instance(d, C::RotatedPillarBlock) => state::set(d, "axis", axis_name(c.face.axis())),
        _ => generic(c, d),
    };
    Some(s)
}

fn axis_name(a: Axis) -> &'static str {
    match a {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

/// The property-driven pattern most blocks follow.
fn generic<L: Level + ?Sized>(c: &Ctx<L>, d: u16) -> u16 {
    let mut s = d;
    if state::has(s, "axis") {
        s = state::set(s, "axis", axis_name(c.face.axis()));
    }
    if let Some(values) = property_values(d, "facing") {
        s = if values.len() == 6 { state::set_dir(s, "facing", c.nearest()[0].opposite()) } else { state::set_dir(s, "facing", c.horizontal().opposite()) };
    }
    if state::has(s, "waterlogged") {
        s = c.waterlogged(s);
    }
    s
}

fn property_values(s: u16, name: &str) -> Option<&'static [&'static str]> {
    BlockId::of(s).info().properties.iter().find(|p| p.name == name).map(|p| p.values)
}

/// `DoorBlock.getHinge`.
fn door_hinge<L: Level + ?Sized>(c: &Ctx<L>) -> &'static str {
    let level = c.level;
    let pos = c.pos;
    let facing = c.horizontal();
    let above = pos.above();
    let ccw = facing.counter_clockwise();
    let cw = facing.clockwise();
    let (l, la) = (level.block(pos.relative(ccw)), level.block(above.relative(ccw)));
    let (r, ra) = (level.block(pos.relative(cw)), level.block(above.relative(cw)));
    let full = |s: u16| block_props::full_collision(s) as i32;
    let score = -full(l) - full(la) + full(r) + full(ra);
    let lower_door = |s: u16| logic::is_instance(s, BlockClass::DoorBlock) && state::get(s, "half") == Some("lower");
    let (left_door, right_door) = (lower_door(l), lower_door(r));
    if (left_door && !right_door) || score > 0 {
        return "right";
    }
    if (right_door && !left_door) || score < 0 {
        return "left";
    }
    let [sx, _, sz] = facing.step();
    let dx = c.click[0] - pos.x as f64;
    let dz = c.click[2] - pos.z as f64;
    if (sx < 0 && dz < 0.5) || (sx > 0 && dz > 0.5) || (sz < 0 && dx > 0.5) || (sz > 0 && dx < 0.5) { "right" } else { "left" }
}

/// `BlockItem.place` up to the state: where the item's block goes and in which state.
pub fn placement<L: Level + ?Sized>(level: &L, item: &BlockItem, ctx: &PlaceContext) -> Option<(BlockPos, u16)> {
    let hit_state = level.block(ctx.hit);
    let replace_clicked = can_be_replaced(level, hit_state, item.block, ctx.face, ctx.click[1], ctx.hit, true);
    let pos = if replace_clicked { ctx.hit } else { ctx.hit.relative(ctx.face) };
    if !level.in_bounds(pos) {
        return None;
    }
    if !replace_clicked && !can_be_replaced(level, level.block(pos), item.block, ctx.face, ctx.click[1], pos, false) {
        return None;
    }
    let c = Ctx { level, pos, replace_clicked, face: ctx.face, click: ctx.click, yaw: ctx.yaw, pitch: ctx.pitch, sneaking: ctx.sneaking };
    let state = match item.wall {
        None => state_for_placement(&c, item.block).filter(|&s| support::can_survive(level, s, pos)),
        Some((wall_block, attach)) => {
            let wall = state_for_placement(&c, wall_block);
            c.nearest().into_iter().filter(|&d| d != attach.opposite()).find_map(|d| {
                let s = if d == attach { state_for_placement(&c, item.block) } else { wall };
                s.filter(|&s| support::can_survive(level, s, pos))
            })
        }
    }?;
    Some((pos, state))
}

/// `BlockItem.place`: sets the block (flags 11) and runs `setPlacedBy` (the second half of
/// doors, tall plants and beds; diodes placed onto power).
pub fn place<L: Level>(level: &mut L, item: &BlockItem, ctx: &PlaceContext) -> Option<(BlockPos, u16)> {
    let (pos, s) = placement(level, item, ctx)?;
    if !set_block(level, pos, s, flags::ALL_IMMEDIATE) {
        return None;
    }
    let placed = level.block(pos);
    if state::same_block(placed, s) {
        placed_by(level, pos, placed);
    }
    Some((pos, placed))
}

/// `Block.setPlacedBy`.
pub fn placed_by<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    use BlockClass as C;
    if logic::is_instance(s, C::DoorBlock) {
        set_block_and_update(level, pos.above(), state::set(s, "half", "upper"));
    } else if logic::is_instance(s, C::DoublePlantBlock) {
        let up = pos.above();
        let mut upper = state::set(BlockId::of(s).default_state(), "half", "upper");
        if state::has(upper, "waterlogged") {
            upper = state::set_bool(upper, "waterlogged", logic::fluid(level.block(up)).kind == FluidKind::Water);
        }
        set_block_and_update(level, up, upper);
    } else if logic::is_instance(s, C::AbstractBedBlock) {
        let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
        set_block_and_update(level, pos.relative(facing), state::set(s, "part", "head"));
    } else if logic::is_instance(s, C::DiodeBlock) {
        diode::placed(level, s, pos);
    } else if logic::block_class(s) == C::PistonBaseBlock {
        crate::behaviour::piston::check_if_extend(level, s, pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_level::TestLevel;
    use kiln_data::blocks::default_state as d;

    fn ctx(hit: BlockPos, face: Direction, yaw: f32) -> PlaceContext {
        let c = [hit.x as f64 + 0.5, hit.y as f64 + 1.0, hit.z as f64 + 0.5];
        PlaceContext { hit, face, click: c, yaw, pitch: 30.0, sneaking: false }
    }

    #[test]
    fn logs_stairs_torches_doors() {
        let mut level = TestLevel::flat(-64, 384, &[d::STONE; 4]);
        level.load_chunks((-1, -1), (1, 1));
        let ground = BlockPos::new(0, -61, 0);
        let log = BlockItem::of_item("minecraft:oak_log").unwrap();
        // The side of the ground has stone behind it: nothing to place into.
        assert!(placement(&level, &log, &ctx(ground, Direction::East, 0.0)).is_none());
        crate::update::set_block(&mut level, BlockPos::new(0, -60, -3), d::STONE, flags::ALL);
        let (p, s) = placement(&level, &log, &ctx(BlockPos::new(0, -60, -3), Direction::East, 0.0)).unwrap();
        assert_eq!((p, state::get(s, "axis")), (BlockPos::new(1, -60, -3), Some("x")));
        let stairs = BlockItem::of_item("minecraft:oak_stairs").unwrap();
        let (p, s) = placement(&level, &stairs, &ctx(ground, Direction::Up, 0.0)).unwrap();
        assert_eq!(p, ground.above());
        assert_eq!((state::get(s, "facing"), state::get(s, "half")), (Some("south"), Some("bottom")));
        // A torch placed against a wall's side becomes a wall torch facing away from it.
        let torch = BlockItem::of_item("minecraft:torch").unwrap();
        let (_, s) = placement(&level, &torch, &ctx(ground, Direction::Up, 0.0)).unwrap();
        assert!(state::is(s, d::TORCH));
        crate::update::set_block(&mut level, BlockPos::new(3, -60, 0), d::STONE, flags::ALL);
        let (_, s) = placement(&level, &torch, &PlaceContext { click: [3.0, -59.5, 0.5], ..ctx(BlockPos::new(3, -60, 0), Direction::West, -90.0) }).unwrap();
        assert!(state::is(s, d::WALL_TORCH));
        assert_eq!(state::get(s, "facing"), Some("west"));
        // Doors take two blocks.
        let door = BlockItem::of_item("minecraft:oak_door").unwrap();
        let (p, _) = place(&mut level, &door, &ctx(BlockPos::new(0, -61, 5), Direction::Up, 0.0)).unwrap();
        assert_eq!(state::get(crate::Level::block(&level, p.above()), "half"), Some("upper"));
        // Slabs double up.
        let slab = BlockItem::of_item("minecraft:stone_slab").unwrap();
        let (p, _) = place(&mut level, &slab, &ctx(BlockPos::new(0, -61, 8), Direction::Up, 0.0)).unwrap();
        let (p2, s) = placement(&level, &slab, &PlaceContext { click: [0.5, -60.5, 8.5], ..ctx(p, Direction::Up, 0.0) }).unwrap();
        assert_eq!((p2, state::get(s, "type")), (p, Some("double")));
    }

    #[test]
    fn nearest_directions() {
        assert_eq!(ordered_by_nearest(0.0, 90.0)[0], Direction::Down);
        assert_eq!(ordered_by_nearest(0.0, 0.0)[0], Direction::South);
        assert_eq!(ordered_by_nearest(90.0, 10.0)[0], Direction::West);
        assert_eq!(ordered_by_nearest(180.0, -80.0)[0], Direction::Up);
    }
}
