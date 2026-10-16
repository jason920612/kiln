//! Shelves (`ShelfBlock`) and the chains powered shelves make (`SideChainPartBlock`): a shelf with a redstone signal links to
//! the powered shelves beside it that face the same way, up to three in a row, and a click on any of them swaps the player's
//! hotbar with the whole row. The items are the block entity's (the simulation's); this is the block's part: power, the
//! chain state (`side_chain`) of each shelf, and what a neighbour's change does to it.

use crate::level::{Effect, Level};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state;
use crate::update::{set_block_and_update, update_neighbour_for_output_signal};
use crate::BlockId;
use kiln_data::block_logic::{self as logic, BlockClass};

/// `ShelfBlock.getMaxChainLength`.
pub const MAX_CHAIN: usize = 3;

/// `SideChainPart`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Unconnected,
    Right,
    Center,
    Left,
}

impl Part {
    fn of(s: u16) -> Part {
        match state::get(s, "side_chain") {
            Some("right") => Part::Right,
            Some("center") => Part::Center,
            Some("left") => Part::Left,
            _ => Part::Unconnected,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Part::Unconnected => "unconnected",
            Part::Right => "right",
            Part::Center => "center",
            Part::Left => "left",
        }
    }

    pub fn is_connected(self) -> bool {
        self != Part::Unconnected
    }

    fn is_connection_towards(self, part: Part) -> bool {
        self == Part::Center || self == part
    }

    fn is_chain_end(self) -> bool {
        self != Part::Center
    }

    fn when_connected_to_the_right(self) -> Part {
        match self {
            Part::Unconnected | Part::Left => Part::Left,
            Part::Right | Part::Center => Part::Center,
        }
    }

    fn when_connected_to_the_left(self) -> Part {
        match self {
            Part::Unconnected | Part::Right => Part::Right,
            Part::Center | Part::Left => Part::Center,
        }
    }

    fn when_disconnected_from_the_right(self) -> Part {
        match self {
            Part::Unconnected | Part::Left => Part::Unconnected,
            Part::Right | Part::Center => Part::Right,
        }
    }

    fn when_disconnected_from_the_left(self) -> Part {
        match self {
            Part::Unconnected | Part::Right => Part::Unconnected,
            Part::Center | Part::Left => Part::Left,
        }
    }
}

fn is_shelf(s: u16) -> bool {
    logic::block_class(s) == BlockClass::ShelfBlock
}

fn facing(s: u16) -> Direction {
    state::get_dir(s, "facing").unwrap_or(Direction::North)
}

/// `ShelfBlock.isConnectable`: a powered wooden shelf.
fn is_connectable(s: u16) -> bool {
    is_shelf(s) && state::get_bool(s, "powered")
}

/// `SideChainPartBlock.Neighbors`: the shelf `n` blocks to one side of `center`, if it can be part of a chain facing `facing`
/// (`None` is an `EmptyNeighbor`).
struct Neighbor {
    pos: BlockPos,
    part: Option<Part>,
}

/// The side a chain's left is on (looking at the fronts): `facing.getClockWise()`.
fn left_side(facing: Direction) -> Direction {
    facing.clockwise()
}

fn right_side(facing: Direction) -> Direction {
    facing.counter_clockwise()
}

fn neighbor<L: Level + ?Sized>(level: &L, center: BlockPos, facing_: Direction, side: Direction, n: i32) -> Neighbor {
    let pos = center.relative_by(side, n);
    let s = level.block(pos);
    let part = (is_connectable(s) && facing(s) == facing_).then(|| Part::of(s));
    Neighbor { pos, part }
}

impl Neighbor {
    fn is_connectable(&self) -> bool {
        self.part.is_some()
    }

    fn is_unconnectable_or_chain_end(&self) -> bool {
        self.part.is_none_or(Part::is_chain_end)
    }

    fn connects_towards(&self, part: Part) -> bool {
        self.part.is_some_and(|p| p.is_connection_towards(part))
    }
}

/// `SideChainPartBlock.setPart`.
fn set_part<L: Level>(level: &mut L, pos: BlockPos, part: Part) {
    let s = level.block(pos);
    if Part::of(s) != part {
        set_block_and_update(level, pos, state::set(s, "side_chain", part.name()));
    }
}

/// `SideChainPartBlock.addBlocksConnectingTowards`.
fn add_connecting_towards<L: Level + ?Sized>(level: &L, center: BlockPos, facing_: Direction, side: Direction, part: Part, out: &mut Vec<BlockPos>) {
    for i in 1..MAX_CHAIN as i32 {
        let nb = neighbor(level, center, facing_, side, i);
        if nb.connects_towards(part) {
            out.push(nb.pos);
        }
        if nb.is_unconnectable_or_chain_end() {
            break;
        }
    }
}

/// `SideChainPartBlock.getAllBlocksConnectedTo`: the shelf itself, then the ones connected to its left, then to its right.
pub fn all_connected_to<L: Level + ?Sized>(level: &L, pos: BlockPos) -> Vec<BlockPos> {
    let s = level.block(pos);
    if !is_connectable(s) {
        return Vec::new();
    }
    let f = facing(s);
    let mut out = vec![pos];
    add_connecting_towards(level, pos, f, left_side(f), Part::Left, &mut out);
    add_connecting_towards(level, pos, f, right_side(f), Part::Right, &mut out);
    out
}

/// `SideChainPartBlock.updateNeighborsAfterPoweringDown`.
fn update_neighbors_after_powering_down<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let f = facing(s);
    let left = neighbor(level, pos, f, left_side(f), 1);
    if let Some(p) = left.part {
        set_part(level, left.pos, p.when_disconnected_from_the_right());
    }
    let right = neighbor(level, pos, f, right_side(f), 1);
    if let Some(p) = right.part {
        set_part(level, right.pos, p.when_disconnected_from_the_left());
    }
}

/// `SideChainPartBlock.canConnect`.
fn can_connect(n: usize, total: usize) -> bool {
    n > 0 && total + n <= MAX_CHAIN
}

/// `SideChainPartBlock.updateSelfAndNeighborsOnPoweringUp`.
fn update_self_and_neighbors_on_powering_up<L: Level>(level: &mut L, pos: BlockPos, new: u16, old: u16) {
    if !is_connectable(new) {
        return;
    }
    // `isBeingUpdatedByNeighbor`: a part that was already connected is only being rewritten by the shelf next to it.
    if Part::of(new).is_connected() || (is_connectable(old) && Part::of(old).is_connected()) {
        return;
    }
    let f = facing(new);
    let left = neighbor(level, pos, f, left_side(f), 1);
    let right = neighbor(level, pos, f, right_side(f), 1);
    let left_len = if left.is_connectable() { all_connected_to(level, left.pos).len() } else { 0 };
    let right_len = if right.is_connectable() { all_connected_to(level, right.pos).len() } else { 0 };
    let mut part = Part::Unconnected;
    let mut total = 1;
    if can_connect(left_len, total) {
        part = part.when_connected_to_the_left();
        if let Some(p) = left.part {
            set_part(level, left.pos, p.when_connected_to_the_right());
        }
        total += left_len;
    }
    if can_connect(right_len, total) {
        part = part.when_connected_to_the_right();
        if let Some(p) = right.part {
            set_part(level, right.pos, p.when_connected_to_the_left());
        }
    }
    set_part(level, pos, part);
}

/// `ShelfBlock.onPlace`.
pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if state::get_bool(s, "powered") {
        update_self_and_neighbors_on_powering_up(level, pos, s, old);
    } else {
        update_neighbors_after_powering_down(level, pos, s);
    }
}

/// `ShelfBlock.neighborChanged`: a signal powers the shelf (and clears its chain when it goes).
pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let powered = has_neighbor_signal(level, pos);
    if state::get_bool(s, "powered") == powered {
        return;
    }
    let mut new = state::set_bool(s, "powered", powered);
    if !powered {
        new = state::set(new, "side_chain", Part::Unconnected.name());
    }
    set_block_and_update(level, pos, new);
    let sound = if powered { "minecraft:block.shelf.activate" } else { "minecraft:block.shelf.deactivate" };
    level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
    level.effect(Effect::BlockGameEvent { pos, event: if powered { "minecraft:block_activate" } else { "minecraft:block_deactivate" }, state: new });
}

/// `ShelfBlock.affectNeighborsAfterRemoval`: comparators are told, the chain beside it is cut.
pub fn removed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    update_neighbour_for_output_signal(level, pos, BlockId::of(s));
    update_neighbors_after_powering_down(level, pos, s);
}

/// `ShelfBlock.getStateForPlacement` (without the water).
pub fn placement<L: Level + ?Sized>(level: &L, default: u16, pos: BlockPos, horizontal: Direction) -> u16 {
    state::set_bool(state::set_dir(default, "facing", horizontal.opposite()), "powered", has_neighbor_signal(level, pos))
}

/// `SelectableSlotContainer.getHitSlot` of a shelf (one row of three): the slot a click on `face` at `cursor` (relative to the clicked block)
/// lands in; none unless the click is on the front.
pub fn hit_slot(s: u16, face: Direction, cursor: [f32; 3]) -> Option<usize> {
    if facing(s) != face {
        return None;
    }
    let (x, z) = (cursor[0] as f64, cursor[2] as f64);
    let u = match face {
        Direction::North => (1.0 - x) as f32,
        Direction::South => x as f32,
        Direction::West => z as f32,
        Direction::East => (1.0 - z) as f32,
        _ => return None,
    };
    // `getSection(u, 3)`.
    Some((((u * 16.0) / (16.0 / 3.0)).floor() as i32).clamp(0, 2) as usize)
}

/// `ShelfBlock.getAnalogOutputSignal` from the slots that hold something, read from the shelf's back.
pub fn analog(s: u16, from: Direction, filled: [bool; 3]) -> i32 {
    if from != facing(s).opposite() {
        return 0;
    }
    filled[0] as i32 | (filled[1] as i32) << 1 | (filled[2] as i32) << 2
}
