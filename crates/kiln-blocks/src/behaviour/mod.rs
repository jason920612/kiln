//! Per-block behaviour (`BlockBehaviour` overrides), dispatched on the vanilla block class.
//!
//! Blocks whose class has no rule here behave like `Block`: no reaction to neighbours,
//! unchanged by shape updates, always surviving. Waterlogged blocks of any class re-check
//! their water on shape updates, as almost every `SimpleWaterloggedBlock` does.

pub mod connect;
pub mod misc;
pub mod support;

use crate::fluid;
use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::redstone::{components, diode, torch, wire};
use crate::state::{self, BlockId};
use kiln_data::block_logic::{self as logic, BlockClass, Support, interface};

/// `BlockState.isFaceSturdy` of `s` for its face toward `dir`.
pub fn sturdy(s: u16, dir: Direction, support: Support) -> bool {
    logic::face_sturdy(s, dir as u8, support)
}

/// The support tag of stems, fungi and roots (their `supportBlocks` constructor argument).
pub(crate) fn params_tag(s: u16) -> Option<&'static str> {
    logic::params(s).support_tag
}

/// `neighborChanged`.
pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId, _moved_by_piston: bool) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::LiquidBlock => fluid::liquid_block_changed(level, s, pos),
        C::RedstoneWireBlock => wire::neighbor_changed(level, s, pos),
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::neighbor_changed(level, s, pos),
        C::RepeaterBlock => diode::neighbor_changed(level, s, pos),
        C::RedstoneLampBlock => components::lamp_neighbor_changed(level, s, pos),
        C::FenceGateBlock => misc::powered_open_neighbor_changed(level, s, pos),
        _ if logic::is_instance(s, C::TrapDoorBlock) => misc::powered_open_neighbor_changed(level, s, pos),
        _ if logic::is_instance(s, C::DoorBlock) => components::door_neighbor_changed(level, s, pos, source),
        _ => {}
    }
}

/// `updateShape`: the state `s` at `pos` should take now that the neighbour toward `dir`
/// is `neighbor_state`. May schedule ticks.
pub fn update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor_pos: BlockPos, neighbor_state: u16) -> u16 {
    use BlockClass as C;
    let _ = neighbor_pos;
    let class = logic::block_class(s);
    if class == C::LiquidBlock {
        return fluid::liquid_update_shape(level, s, pos, dir, neighbor_state);
    }
    if logic::implements(s, interface::SIMPLE_WATERLOGGED_BLOCK) {
        fluid::tick_water_if_waterlogged(level, s, pos);
    }
    match class {
        C::RedstoneWireBlock => return wire::update_shape(level, s, pos, dir, neighbor_state),
        C::RepeaterBlock => return diode::update_shape(level, s, pos, dir, neighbor_state),
        C::StairBlock | C::WeatheringCopperStairBlock if dir.is_horizontal() => {
            return state::set(s, "shape", connect::stairs_shape(level, s, pos));
        }
        C::WallBlock if dir != Direction::Down => return connect::wall_update(level, s, pos, dir, neighbor_state),
        C::FenceGateBlock => return misc::gate_update_shape(level, s, pos, dir, neighbor_state),
        _ => {}
    }
    if logic::is_instance(s, C::LeavesBlock) {
        return misc::leaves_update_shape(level, s, pos, neighbor_state);
    }
    if logic::is_instance(s, C::FallingBlock) {
        misc::falling_schedule(level, s, pos);
        return s;
    }
    if (logic::is_instance(s, C::FenceBlock) || logic::is_instance(s, C::IronBarsBlock)) && dir.is_horizontal() {
        return connect::cross_update(s, dir, neighbor_state);
    }
    if logic::is_instance(s, C::SnowyBlock) && dir == Direction::Up {
        return state::set_bool(s, "snowy", connect::snowy_setting(neighbor_state));
    }
    if let Some(new) = support::pop_off(level, s, pos, dir, neighbor_state) {
        return new;
    }
    s
}

/// `updateIndirectNeighbourShapes` (only redstone wire has any).
pub fn update_indirect_neighbour_shapes<L: Level>(level: &mut L, s: u16, pos: BlockPos, flags: u32, limit: i32) {
    if logic::block_class(s) == BlockClass::RedstoneWireBlock {
        wire::update_indirect_neighbour_shapes(level, s, pos, flags, limit);
    }
}

pub fn can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    support::can_survive(level, s, pos)
}

/// `onPlace`: after `s` replaced `old` at `pos`.
pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16, _moved_by_piston: bool) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::LiquidBlock => fluid::liquid_block_changed(level, s, pos),
        C::RedstoneWireBlock => wire::on_place(level, s, pos, old),
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::on_place(level, s, pos),
        C::RepeaterBlock => diode::on_place(level, s, pos),
        _ if logic::is_instance(s, C::FallingBlock) => misc::falling_schedule(level, s, pos),
        _ => {}
    }
}

/// `affectNeighborsAfterRemoval`: `s` was just replaced at `pos`.
pub fn affect_neighbors_after_removal<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::RedstoneWireBlock => wire::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        C::RepeaterBlock => diode::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        C::LeverBlock | C::ButtonBlock => components::attached_removed(level, s, pos, moved_by_piston),
        _ => {}
    }
}

/// A scheduled block tick (`Block.tick`), when the block at `pos` is still the ticked block.
pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::tick(level, s, pos),
        C::RepeaterBlock => diode::tick(level, s, pos),
        C::ButtonBlock => components::button_tick(level, s, pos),
        C::RedstoneLampBlock => components::lamp_tick(level, s, pos),
        _ if logic::is_instance(s, C::LeavesBlock) => misc::leaves_tick(level, s, pos),
        _ if logic::is_instance(s, C::FallingBlock) => misc::falling_tick(level, s, pos),
        _ => {}
    }
}

/// `randomTick`: leaf decay. Other random-tick behaviour (crop growth, grass spread, lava
/// fire, ...) is not simulated yet; the positions are still drawn so the random-tick
/// sequence stays aligned.
pub fn random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if logic::is_instance(s, BlockClass::LeavesBlock) {
        misc::leaves_random_tick(level, s, pos);
    }
}

/// `triggerEvent` for a block event; true if it should reach clients.
pub fn trigger_event<L: Level>(_level: &mut L, _s: u16, _pos: BlockPos, _a: i32, _b: i32) -> bool {
    false
}
