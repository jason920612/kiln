//! Redstone wire (`RedStoneWireBlock` with `DefaultRedstoneWireEvaluator`).

use super::best_neighbor_signal;
use crate::behaviour::support::wire_can_survive_on;
use crate::level::{Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::update::{drop_and_remove, neighbor_shape_changed, set_block, update_neighbors_at};
use kiln_data::block_logic::{self as logic, BlockClass, Support};
use kiln_data::blocks::default_state as d;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Up,
    Side,
    None,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Up => "up",
            Side::Side => "side",
            Side::None => "none",
        }
    }

    fn connected(self) -> bool {
        self != Side::None
    }
}

fn is_wire(s: u16) -> bool {
    state::is(s, d::REDSTONE_WIRE)
}

fn wire_block() -> BlockId {
    BlockId::of(d::REDSTONE_WIRE)
}

fn side(s: u16, dir: Direction) -> Side {
    match state::get(s, dir.name()) {
        Some("up") => Side::Up,
        Some("side") => Side::Side,
        _ => Side::None,
    }
}

fn with_side(s: u16, dir: Direction, side: Side) -> u16 {
    state::set(s, dir.name(), side.name())
}

fn power(s: u16) -> i32 {
    if is_wire(s) { state::get_int(s, "power") } else { 0 }
}

fn is_cross(s: u16) -> bool {
    Direction::HORIZONTAL.iter().all(|&d| side(s, d).connected())
}

fn is_dot(s: u16) -> bool {
    !Direction::HORIZONTAL.iter().any(|&d| side(s, d).connected())
}

/// `shouldRedstoneWireConnectTo` of the block `s` for a wire toward `dir` (`None` for the
/// diagonal checks above and below).
fn connects_to(s: u16, dir: Option<Direction>) -> bool {
    match logic::block_class(s) {
        BlockClass::RedstoneWireBlock => true,
        BlockClass::RepeaterBlock => {
            let f = state::get_dir(s, "facing");
            dir.is_some() && (f == dir || f.map(Direction::opposite) == dir)
        }
        BlockClass::ObserverBlock => dir.is_some() && state::get_dir(s, "facing") == dir,
        _ => logic::is_signal_source(s) && dir.is_some(),
    }
}

/// `getConnectingSide(level, pos, dir, canClimb)`.
fn connecting_side_with<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction, climb: bool) -> Side {
    let n = pos.relative(dir);
    let ns = level.block(n);
    if climb {
        let supports = logic::is_instance(ns, BlockClass::TrapDoorBlock) || wire_can_survive_on(ns);
        if supports && connects_to(level.block(n.above()), None) {
            return if logic::face_sturdy(ns, dir.opposite() as u8, Support::Full) { Side::Up } else { Side::Side };
        }
    }
    if connects_to(ns, Some(dir)) || !logic::is_redstone_conductor(ns) && connects_to(level.block(n.below()), None) {
        Side::Side
    } else {
        Side::None
    }
}

fn connecting_side<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> Side {
    let climb = !logic::is_redstone_conductor(level.block(pos.above()));
    connecting_side_with(level, pos, dir, climb)
}

/// `getMissingConnections`.
fn missing_connections<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> u16 {
    let climb = !logic::is_redstone_conductor(level.block(pos.above()));
    let mut s = s;
    for dir in Direction::HORIZONTAL {
        if !side(s, dir).connected() {
            s = with_side(s, dir, connecting_side_with(level, pos, dir, climb));
        }
    }
    s
}

/// `getConnectionState`: connections from the world; a lone line extends to both ends.
pub fn connection_state<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> u16 {
    let dot = is_dot(s);
    let mut st = missing_connections(level, state::set_int(d::REDSTONE_WIRE, "power", power(s)), pos);
    if dot && is_dot(st) {
        return st;
    }
    let [n, e, so, w] = [Direction::North, Direction::East, Direction::South, Direction::West].map(|d| side(st, d).connected());
    let no_ns = !n && !so;
    let no_ew = !e && !w;
    if !w && no_ns {
        st = with_side(st, Direction::West, Side::Side);
    }
    if !e && no_ns {
        st = with_side(st, Direction::East, Side::Side);
    }
    if !n && no_ew {
        st = with_side(st, Direction::North, Side::Side);
    }
    if !so && no_ew {
        st = with_side(st, Direction::South, Side::Side);
    }
    st
}

/// `RedStoneWireBlock.updateShape`.
pub fn update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    match dir {
        Direction::Down => {
            if wire_can_survive_on(neighbor) { s } else { d::AIR }
        }
        Direction::Up => connection_state(level, s, pos),
        _ => {
            let new = connecting_side(level, pos, dir);
            if new.connected() == side(s, dir).connected() && !is_cross(s) {
                return with_side(s, dir, new);
            }
            let cross = [Direction::North, Direction::East, Direction::South, Direction::West]
                .into_iter()
                .fold(state::set_int(d::REDSTONE_WIRE, "power", power(s)), |acc, d| with_side(acc, d, Side::Side));
            connection_state(level, with_side(cross, dir, new), pos)
        }
    }
}

/// `RedStoneWireBlock.updateIndirectNeighbourShapes`: wires one step up or down a slope.
pub fn update_indirect_neighbour_shapes<L: Level>(level: &mut L, s: u16, pos: BlockPos, flags: u32, limit: i32) {
    for dir in Direction::HORIZONTAL {
        if side(s, dir) == Side::None || is_wire(level.block(pos.relative(dir))) {
            continue;
        }
        for dy in [Direction::Down, Direction::Up] {
            let m = pos.relative(dir).relative(dy);
            if is_wire(level.block(m)) {
                let np = m.relative(dir.opposite());
                let ns = level.block(np);
                neighbor_shape_changed(level, dir.opposite(), m, np, ns, flags, limit);
            }
        }
    }
}

/// `RedStoneWireBlock.getSignal`.
pub fn signal<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> i32 {
    if dir == Direction::Down {
        return 0;
    }
    let own = power(s);
    if own == 0 {
        return 0;
    }
    if dir == Direction::Up || side(connection_state(level, s, pos), dir.opposite()).connected() { own } else { 0 }
}

/// `calculateTargetStrength`.
fn target_strength<L: Level + ?Sized>(level: &L, pos: BlockPos) -> i32 {
    let block = best_neighbor_signal(level, pos, false);
    if block == 15 {
        return 15;
    }
    let mut wire = 0;
    for dir in Direction::HORIZONTAL {
        let n = pos.relative(dir);
        let ns = level.block(n);
        wire = wire.max(power(ns));
        if logic::is_redstone_conductor(ns) {
            if !logic::is_redstone_conductor(level.block(pos.above())) {
                wire = wire.max(power(level.block(n.above())));
            }
        } else {
            wire = wire.max(power(level.block(n.below())));
        }
    }
    block.max(wire - 1)
}

/// Iteration order of a `java.util.HashSet<BlockPos>` of default capacity holding `positions`
/// inserted in order (buckets by spread hash, insertion order within a bucket).
fn java_hash_set_order(positions: &[BlockPos]) -> Vec<BlockPos> {
    let bucket = |p: &BlockPos| {
        let h = p.java_hash();
        ((h ^ ((h as u32) >> 16) as i32) & 15) as usize
    };
    let mut out: Vec<BlockPos> = positions.to_vec();
    out.sort_by_key(bucket);
    out
}

/// `DefaultRedstoneWireEvaluator.updatePowerStrength`.
fn update_power<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let target = target_strength(level, pos);
    if power(s) == target {
        return;
    }
    if level.block(pos) == s {
        set_block(level, pos, state::set_int(s, "power", target), flags::CLIENTS);
    }
    let mut set = vec![pos];
    set.extend(Direction::ALL.iter().map(|&d| pos.relative(d)));
    for p in java_hash_set_order(&set) {
        update_neighbors_at(level, p, wire_block());
    }
}

fn check_corner<L: Level>(level: &mut L, pos: BlockPos) {
    if !is_wire(level.block(pos)) {
        return;
    }
    update_neighbors_at(level, pos, wire_block());
    for dir in Direction::ALL {
        update_neighbors_at(level, pos.relative(dir), wire_block());
    }
}

/// `updateNeighborsOfNeighboringWires`.
fn update_neighboring_wires<L: Level>(level: &mut L, pos: BlockPos) {
    for dir in Direction::HORIZONTAL {
        check_corner(level, pos.relative(dir));
    }
    for dir in Direction::HORIZONTAL {
        let n = pos.relative(dir);
        if logic::is_redstone_conductor(level.block(n)) {
            check_corner(level, n.above());
        } else {
            check_corner(level, n.below());
        }
    }
}

pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if is_wire(old) {
        return;
    }
    update_power(level, pos, s);
    for dir in Direction::VERTICAL {
        update_neighbors_at(level, pos.relative(dir), wire_block());
    }
    update_neighboring_wires(level, pos);
}

pub fn affect_neighbors_after_removal<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if moved_by_piston {
        return;
    }
    for dir in Direction::ALL {
        update_neighbors_at(level, pos.relative(dir), wire_block());
    }
    update_power(level, pos, s);
    update_neighboring_wires(level, pos);
}

pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if wire_can_survive_on(level.block(pos.below())) {
        update_power(level, pos, s);
    } else {
        drop_and_remove(level, pos, s);
    }
}

/// `getStateForPlacement`: connections of a fresh cross.
pub fn placement<L: Level + ?Sized>(level: &L, pos: BlockPos) -> u16 {
    let cross = [Direction::North, Direction::East, Direction::South, Direction::West]
        .into_iter()
        .fold(d::REDSTONE_WIRE, |acc, d| with_side(acc, d, Side::Side));
    connection_state(level, cross, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_set_order_matches_java() {
        // java.util.HashSet of pos and its six neighbours for (0, 64, 0), inserted as
        // pos, down, up, north, south, west, east; buckets from Vec3i.hashCode.
        let p = BlockPos::new(0, 64, 0);
        let mut set = vec![p];
        set.extend(Direction::ALL.iter().map(|&d| p.relative(d)));
        let order = java_hash_set_order(&set);
        let buckets: Vec<usize> = order
            .iter()
            .map(|p| {
                let h = p.java_hash();
                ((h ^ ((h as u32) >> 16) as i32) & 15) as usize
            })
            .collect();
        assert!(buckets.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(order.len(), 7);
    }
}
