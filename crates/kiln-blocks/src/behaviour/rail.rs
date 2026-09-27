//! Rails (`BaseRailBlock`, `RailState`, `RailBlock`, `PoweredRailBlock` for powered and
//! activator rails, `DetectorRailBlock`).

use super::support::can_support_rigid;
use crate::level::{EntityKind, Level, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{neighbor_changed_with, remove_block, set_block_and_update, update_neighbors_at, update_neighbour_for_output_signal};
use kiln_data::block_logic::{self as logic, BlockClass};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    NorthSouth,
    EastWest,
    AscEast,
    AscWest,
    AscNorth,
    AscSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}

impl Shape {
    const ALL: [Shape; 10] = [
        Shape::NorthSouth,
        Shape::EastWest,
        Shape::AscEast,
        Shape::AscWest,
        Shape::AscNorth,
        Shape::AscSouth,
        Shape::SouthEast,
        Shape::SouthWest,
        Shape::NorthWest,
        Shape::NorthEast,
    ];

    fn name(self) -> &'static str {
        match self {
            Shape::NorthSouth => "north_south",
            Shape::EastWest => "east_west",
            Shape::AscEast => "ascending_east",
            Shape::AscWest => "ascending_west",
            Shape::AscNorth => "ascending_north",
            Shape::AscSouth => "ascending_south",
            Shape::SouthEast => "south_east",
            Shape::SouthWest => "south_west",
            Shape::NorthWest => "north_west",
            Shape::NorthEast => "north_east",
        }
    }

    fn of(s: u16) -> Shape {
        let v = state::get(s, "shape").unwrap_or("north_south");
        Shape::ALL.into_iter().find(|x| x.name() == v).unwrap_or(Shape::NorthSouth)
    }

    fn is_slope(self) -> bool {
        matches!(self, Shape::AscEast | Shape::AscWest | Shape::AscNorth | Shape::AscSouth)
    }
}

/// `BaseRailBlock.isRail`.
pub fn is_rail(s: u16) -> bool {
    tags::is(s, "minecraft:rails") && logic::is_instance(s, BlockClass::BaseRailBlock)
}

fn straight(s: u16) -> bool {
    logic::params(s).straight_rail
}

/// A rail and the positions it connects to (`RailState`).
struct Rail {
    pos: BlockPos,
    state: u16,
    straight: bool,
    connections: Vec<BlockPos>,
}

impl Rail {
    fn new(pos: BlockPos, state: u16) -> Self {
        let mut r = Rail { pos, state, straight: straight(state), connections: Vec::new() };
        r.update_connections(Shape::of(state));
        r
    }

    fn update_connections(&mut self, shape: Shape) {
        let p = self.pos;
        let (n, s, w, e) = (p.relative(Direction::North), p.relative(Direction::South), p.relative(Direction::West), p.relative(Direction::East));
        self.connections = match shape {
            Shape::NorthSouth => vec![n, s],
            Shape::EastWest => vec![w, e],
            Shape::AscEast => vec![w, e.above()],
            Shape::AscWest => vec![w.above(), e],
            Shape::AscNorth => vec![n.above(), s],
            Shape::AscSouth => vec![n, s.above()],
            Shape::SouthEast => vec![e, s],
            Shape::SouthWest => vec![w, s],
            Shape::NorthWest => vec![w, n],
            Shape::NorthEast => vec![e, n],
        };
    }

    fn get<L: Level + ?Sized>(level: &L, p: BlockPos) -> Option<Rail> {
        [p, p.above(), p.below()].into_iter().find_map(|q| {
            let s = level.block(q);
            is_rail(s).then(|| Rail::new(q, s))
        })
    }

    fn remove_soft_connections<L: Level + ?Sized>(&mut self, level: &L) {
        let mut i = 0;
        while i < self.connections.len() {
            match Rail::get(level, self.connections[i]) {
                Some(r) if r.has_connection(self.pos) => {
                    self.connections[i] = r.pos;
                    i += 1;
                }
                _ => {
                    self.connections.remove(i);
                }
            }
        }
    }

    fn has_rail<L: Level + ?Sized>(level: &L, p: BlockPos) -> bool {
        is_rail(level.block(p)) || is_rail(level.block(p.above())) || is_rail(level.block(p.below()))
    }

    fn has_connection(&self, p: BlockPos) -> bool {
        self.connections.iter().any(|c| c.x == p.x && c.z == p.z)
    }

    fn count_potential_connections<L: Level + ?Sized>(&self, level: &L) -> usize {
        Direction::HORIZONTAL.iter().filter(|&&d| Rail::has_rail(level, self.pos.relative(d))).count()
    }

    fn can_connect_to(&self, other: &Rail) -> bool {
        self.has_connection(other.pos) || self.connections.len() != 2
    }

    fn connect_to<L: Level>(&mut self, level: &mut L, other: &Rail) {
        self.connections.push(other.pos);
        let p = self.pos;
        let (n, s, w, e) = (p.relative(Direction::North), p.relative(Direction::South), p.relative(Direction::West), p.relative(Direction::East));
        let (hn, hs, hw, he) = (self.has_connection(n), self.has_connection(s), self.has_connection(w), self.has_connection(e));
        let mut shape = None;
        if hn || hs {
            shape = Some(Shape::NorthSouth);
        }
        if hw || he {
            shape = Some(Shape::EastWest);
        }
        if !self.straight {
            if hs && he && !hn && !hw {
                shape = Some(Shape::SouthEast);
            }
            if hs && hw && !hn && !he {
                shape = Some(Shape::SouthWest);
            }
            if hn && hw && !hs && !he {
                shape = Some(Shape::NorthWest);
            }
            if hn && he && !hs && !hw {
                shape = Some(Shape::NorthEast);
            }
        }
        if shape == Some(Shape::NorthSouth) {
            if is_rail(level.block(n.above())) {
                shape = Some(Shape::AscNorth);
            }
            if is_rail(level.block(s.above())) {
                shape = Some(Shape::AscSouth);
            }
        }
        if shape == Some(Shape::EastWest) {
            if is_rail(level.block(e.above())) {
                shape = Some(Shape::AscEast);
            }
            if is_rail(level.block(w.above())) {
                shape = Some(Shape::AscWest);
            }
        }
        let shape = shape.unwrap_or(Shape::NorthSouth);
        self.state = state::set(self.state, "shape", shape.name());
        set_block_and_update(level, self.pos, self.state);
    }

    fn has_neighbor_rail<L: Level + ?Sized>(&self, level: &L, p: BlockPos) -> bool {
        let Some(mut r) = Rail::get(level, p) else { return false };
        r.remove_soft_connections(level);
        r.can_connect_to(self)
    }

    /// `RailState.place`: picks the shape joining neighbouring rails, then lets them join back.
    fn place<L: Level>(mut self, level: &mut L, powered: bool, always: bool, current: Shape) -> Rail {
        let p = self.pos;
        let (n, s, w, e) = (p.relative(Direction::North), p.relative(Direction::South), p.relative(Direction::West), p.relative(Direction::East));
        let (hn, hs, hw, he) = (self.has_neighbor_rail(level, n), self.has_neighbor_rail(level, s), self.has_neighbor_rail(level, w), self.has_neighbor_rail(level, e));
        let ns = hn || hs;
        let ew = hw || he;
        let (se, sw, ne, nw) = (hs && he, hs && hw, hn && he, hn && hw);
        let mut shape = None;
        if ns && !ew {
            shape = Some(Shape::NorthSouth);
        }
        if ew && !ns {
            shape = Some(Shape::EastWest);
        }
        if !self.straight {
            if se && !hn && !hw {
                shape = Some(Shape::SouthEast);
            }
            if sw && !hn && !he {
                shape = Some(Shape::SouthWest);
            }
            if nw && !hs && !he {
                shape = Some(Shape::NorthWest);
            }
            if ne && !hs && !hw {
                shape = Some(Shape::NorthEast);
            }
        }
        if shape.is_none() {
            if ns && ew {
                shape = Some(current);
            } else if ns {
                shape = Some(Shape::NorthSouth);
            } else if ew {
                shape = Some(Shape::EastWest);
            }
            if !self.straight {
                let order = if powered {
                    [(se, Shape::SouthEast), (sw, Shape::SouthWest), (ne, Shape::NorthEast), (nw, Shape::NorthWest)]
                } else {
                    [(nw, Shape::NorthWest), (ne, Shape::NorthEast), (sw, Shape::SouthWest), (se, Shape::SouthEast)]
                };
                for (cond, sh) in order {
                    if cond {
                        shape = Some(sh);
                    }
                }
            }
        }
        if shape == Some(Shape::NorthSouth) {
            if is_rail(level.block(n.above())) {
                shape = Some(Shape::AscNorth);
            }
            if is_rail(level.block(s.above())) {
                shape = Some(Shape::AscSouth);
            }
        }
        if shape == Some(Shape::EastWest) {
            if is_rail(level.block(e.above())) {
                shape = Some(Shape::AscEast);
            }
            if is_rail(level.block(w.above())) {
                shape = Some(Shape::AscWest);
            }
        }
        let shape = shape.unwrap_or(current);
        self.update_connections(shape);
        self.state = state::set(self.state, "shape", shape.name());
        if always || level.block(self.pos) != self.state {
            set_block_and_update(level, self.pos, self.state);
            let mut i = 0;
            while i < self.connections.len() {
                if let Some(mut r) = Rail::get(level, self.connections[i]) {
                    r.remove_soft_connections(level);
                    if r.can_connect_to(&self) {
                        r.connect_to(level, &self);
                    }
                }
                i += 1;
            }
        }
        self
    }
}

/// `BaseRailBlock.updateDir`.
fn update_dir<L: Level>(level: &mut L, pos: BlockPos, s: u16, always: bool) -> u16 {
    let powered = has_neighbor_signal(level, pos);
    Rail::new(pos, s).place(level, powered, always, Shape::of(s)).state
}

/// `BaseRailBlock.shouldBeRemoved`.
fn should_be_removed<L: Level + ?Sized>(level: &L, pos: BlockPos, shape: Shape) -> bool {
    let rigid = |p: BlockPos| can_support_rigid(level.block(p));
    if !rigid(pos.below()) {
        return true;
    }
    match shape {
        Shape::AscEast => !rigid(pos.relative(Direction::East)),
        Shape::AscWest => !rigid(pos.relative(Direction::West)),
        Shape::AscNorth => !rigid(pos.relative(Direction::North)),
        Shape::AscSouth => !rigid(pos.relative(Direction::South)),
        _ => false,
    }
}

pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16, moved_by_piston: bool) {
    if state::same_block(old, s) {
        return;
    }
    let new = update_dir(level, pos, s, true);
    if straight(s) {
        neighbor_changed_with(level, new, pos, BlockId::of(s), moved_by_piston);
    }
    if logic::block_class(s) == BlockClass::DetectorRailBlock {
        detector_check(level, pos, new);
    }
}

pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId, moved_by_piston: bool) {
    if !state::same_block(level.block(pos), s) {
        return;
    }
    if should_be_removed(level, pos, Shape::of(s)) {
        level.effect(crate::level::Effect::Drop { pos, state: s });
        remove_block(level, pos, moved_by_piston);
        return;
    }
    match logic::block_class(s) {
        BlockClass::RailBlock => {
            if logic::is_signal_source(source.default_state()) && Rail::new(pos, s).count_potential_connections(level) == 3 {
                update_dir(level, pos, s, false);
            }
        }
        BlockClass::PoweredRailBlock => powered_update(level, pos, s),
        _ => {}
    }
}

pub fn affect_neighbors_after_removal<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if moved_by_piston {
        return;
    }
    let block = BlockId::of(s);
    if Shape::of(s).is_slope() {
        update_neighbors_at(level, pos.above(), block);
    }
    if straight(s) {
        update_neighbors_at(level, pos, block);
        update_neighbors_at(level, pos.below(), block);
    }
}

/// `PoweredRailBlock.updateState`.
fn powered_update<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let powered = state::get_bool(s, "powered");
    let should = has_neighbor_signal(level, pos) || find_power(level, pos, s, true, 0) || find_power(level, pos, s, false, 0);
    if should == powered {
        return;
    }
    set_block_and_update(level, pos, state::set_bool(s, "powered", should));
    let block = BlockId::of(s);
    update_neighbors_at(level, pos.below(), block);
    if Shape::of(s).is_slope() {
        update_neighbors_at(level, pos.above(), block);
    }
}

/// `PoweredRailBlock.findPoweredRailSignal`: power along up to 8 rails of the same kind.
fn find_power<L: Level + ?Sized>(level: &L, pos: BlockPos, s: u16, forward: bool, depth: i32) -> bool {
    if depth >= 8 {
        return false;
    }
    let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
    let mut check_below = true;
    let mut shape = Shape::of(s);
    match shape {
        Shape::NorthSouth => z += if forward { 1 } else { -1 },
        Shape::EastWest => x += if forward { -1 } else { 1 },
        Shape::AscEast => {
            if forward {
                x -= 1;
            } else {
                x += 1;
                y += 1;
                check_below = false;
            }
            shape = Shape::EastWest;
        }
        Shape::AscWest => {
            if forward {
                x -= 1;
                y += 1;
                check_below = false;
            } else {
                x += 1;
            }
            shape = Shape::EastWest;
        }
        Shape::AscNorth => {
            if forward {
                z += 1;
            } else {
                z -= 1;
                y += 1;
                check_below = false;
            }
            shape = Shape::NorthSouth;
        }
        Shape::AscSouth => {
            if forward {
                z += 1;
                y += 1;
                check_below = false;
            } else {
                z -= 1;
            }
            shape = Shape::NorthSouth;
        }
        _ => {}
    }
    same_rail_with_power(level, BlockPos::new(x, y, z), s, forward, depth, shape)
        || check_below && same_rail_with_power(level, BlockPos::new(x, y - 1, z), s, forward, depth, shape)
}

/// `PoweredRailBlock.isSameRailWithPower`.
fn same_rail_with_power<L: Level + ?Sized>(level: &L, pos: BlockPos, rail: u16, forward: bool, depth: i32, shape: Shape) -> bool {
    let s = level.block(pos);
    if !state::same_block(s, rail) {
        return false;
    }
    let other = Shape::of(s);
    if shape == Shape::EastWest && matches!(other, Shape::NorthSouth | Shape::AscNorth | Shape::AscSouth) {
        return false;
    }
    if shape == Shape::NorthSouth && matches!(other, Shape::EastWest | Shape::AscEast | Shape::AscWest) {
        return false;
    }
    if !state::get_bool(s, "powered") {
        return false;
    }
    has_neighbor_signal(level, pos) || find_power(level, pos, s, forward, depth + 1)
}

/// `DetectorRailBlock.checkPressed`: minecarts on the rail power it.
fn detector_check<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    if !can_support_rigid(level.block(pos.below())) || !state::same_block(level.block(pos), s) {
        return;
    }
    let powered = state::get_bool(s, "powered");
    let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
    let occupied = level.count_entities([x + 0.2, y, z + 0.2], [x + 0.8, y + 0.8, z + 0.8], EntityKind::Minecart) > 0;
    let block = BlockId::of(s);
    if occupied != powered {
        let new = state::set_bool(s, "powered", occupied);
        set_block_and_update(level, pos, new);
        for c in Rail::new(pos, new).connections {
            let cs = level.block(c);
            neighbor_changed_with(level, cs, c, BlockId::of(cs), false);
        }
        update_neighbors_at(level, pos, block);
        update_neighbors_at(level, pos.below(), block);
    }
    if occupied {
        schedule_block_tick(level, pos, block, 20, TickPriority::Normal);
    }
    update_neighbour_for_output_signal(level, pos, block);
}

pub fn detector_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "powered") {
        detector_check(level, pos, s);
    }
}

/// `DetectorRailBlock.entityInside` for a minecart on the rail.
pub fn detector_entity_inside<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    if logic::block_class(s) == BlockClass::DetectorRailBlock && !state::get_bool(s, "powered") {
        detector_check(level, pos, s);
    }
}

/// `BaseRailBlock.getStateForPlacement`.
pub fn placement(d: u16, horizontal: Direction, water: bool) -> u16 {
    let shape = if horizontal.axis() == crate::pos::Axis::X { Shape::EastWest } else { Shape::NorthSouth };
    state::set_bool(state::set(d, "shape", shape.name()), "waterlogged", water)
}
