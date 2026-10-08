//! Bubble columns (`BubbleColumnBlock`) and the water that makes them (`LiquidBlock.tick`).
//!
//! A full water source above soul sand or magma (tags `enables_bubble_column_push_up` and
//! `enables_bubble_column_drag_down`) schedules a tick 20 ticks out (`LiquidBlock`, see
//! `fluid.rs`); that tick turns the water column above the block into bubble columns
//! (`updateColumn`), every one of which re-checks its column 5 ticks after its support, the
//! block above it or itself changes. What the columns do to entities is `kiln_entity::inside`.

use crate::FluidType;
use crate::fluid;
use crate::level::{Level, flags, schedule_block_tick, schedule_fluid_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::set_block;
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind};
use kiln_data::blocks::default_state as d;

/// `LiquidBlock.shouldBubbleColumnOccupy`: a full water source.
pub fn should_occupy(s: u16) -> bool {
    let f = logic::fluid(s);
    f.kind == FluidKind::Water && f.source && f.amount == 8
}

/// `BubbleColumnBlock.canOccupy`: a bubble column, or a full source of a liquid block.
fn can_occupy(s: u16) -> bool {
    state::is(s, d::BUBBLE_COLUMN) || (logic::block_class(s) == BlockClass::LiquidBlock && should_occupy(s))
}

/// `getColumnState`.
fn column_state(below: u16, occupy: u16) -> u16 {
    if state::is(below, d::BUBBLE_COLUMN) {
        below
    } else if tags::is(below, "minecraft:enables_bubble_column_push_up") {
        state::set_bool(d::BUBBLE_COLUMN, "drag", false)
    } else if tags::is(below, "minecraft:enables_bubble_column_drag_down") {
        state::set_bool(d::BUBBLE_COLUMN, "drag", true)
    } else if state::is(occupy, d::BUBBLE_COLUMN) {
        d::WATER
    } else {
        occupy
    }
}

/// `BubbleColumnBlock.updateColumn`.
pub fn update_column<L: Level>(level: &mut L, pos: BlockPos, occupy: u16, below: u16) {
    if !can_occupy(occupy) {
        return;
    }
    let column = column_state(below, occupy);
    set_block(level, pos, column, flags::CLIENTS);
    let mut p = pos.above();
    while can_occupy(level.block(p)) {
        if !set_block(level, p, column, flags::CLIENTS) {
            return;
        }
        p = p.above();
    }
}

/// `LiquidBlock.tick`.
pub fn liquid_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if should_occupy(s) {
        let below = level.block(pos.below());
        update_column(level, pos, level.block(pos), below);
    }
}

/// `BubbleColumnBlock.tick`.
pub fn tick<L: Level>(level: &mut L, pos: BlockPos) {
    let below = level.block(pos.below());
    let here = level.block(pos);
    update_column(level, pos, here, below);
}

/// `BubbleColumnBlock.canSurvive`.
pub fn can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let b = level.block(pos.below());
    state::is(b, d::BUBBLE_COLUMN) || tags::is(b, "minecraft:enables_bubble_column_push_up") || tags::is(b, "minecraft:enables_bubble_column_drag_down")
}

/// `BubbleColumnBlock.updateShape`.
pub fn update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor_state: u16) -> u16 {
    schedule_fluid_tick(level, pos, FluidType::Water, fluid::tick_delay(level, FluidKind::Water));
    if !can_survive(level, pos) || dir == Direction::Down || (dir == Direction::Up && !state::is(neighbor_state, d::BUBBLE_COLUMN) && can_occupy(neighbor_state)) {
        schedule_block_tick(level, pos, BlockId::of(s), 5, TickPriority::Normal);
    }
    s
}
