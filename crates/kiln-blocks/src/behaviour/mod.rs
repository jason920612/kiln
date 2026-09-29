//! Per-block behaviour (`BlockBehaviour` overrides), dispatched on the vanilla block class.
//!
//! Blocks whose class has no rule here behave like `Block`: no reaction to neighbours,
//! unchanged by shape updates, always surviving. Waterlogged blocks of any class re-check
//! their water on shape updates, as almost every `SimpleWaterloggedBlock` does.

pub mod connect;
pub mod container;
pub mod misc;
pub mod piston;
pub mod portal;
pub mod rail;
pub mod sculk;
pub mod support;

use crate::fluid;
use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::redstone::{components, devices, diode, torch, wire};
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
pub fn neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId, moved_by_piston: bool) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::LiquidBlock => fluid::liquid_block_changed(level, s, pos),
        C::RedstoneWireBlock => wire::neighbor_changed(level, s, pos),
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::neighbor_changed(level, s, pos),
        C::RepeaterBlock | C::ComparatorBlock => diode::neighbor_changed(level, s, pos),
        C::RedstoneLampBlock => components::lamp_neighbor_changed(level, s, pos),
        C::NoteBlock => devices::note_neighbor_changed(level, s, pos),
        C::TntBlock => devices::tnt_neighbor_changed(level, pos),
        C::FenceGateBlock => misc::powered_open_neighbor_changed(level, s, pos),
        C::PistonBaseBlock => piston::check_if_extend(level, s, pos),
        C::PistonHeadBlock => piston::head_neighbor_changed(level, s, pos, source),
        C::HopperBlock => container::hopper_check_powered(level, s, pos),
        C::DispenserBlock | C::DropperBlock => container::dispenser_neighbor_changed(level, s, pos),
        _ if logic::is_instance(s, C::TrapDoorBlock) => misc::powered_open_neighbor_changed(level, s, pos),
        _ if logic::is_instance(s, C::DoorBlock) => components::door_neighbor_changed(level, s, pos, source),
        _ if logic::is_instance(s, C::BaseRailBlock) => rail::neighbor_changed(level, s, pos, source, moved_by_piston),
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
    if container::is_chest(s) {
        return container::chest_update_shape(s, dir, neighbor_state);
    }
    match class {
        C::RedstoneWireBlock => return wire::update_shape(level, s, pos, dir, neighbor_state),
        C::RepeaterBlock | C::ComparatorBlock => return diode::update_shape(level, s, pos, dir, neighbor_state),
        C::StairBlock | C::WeatheringCopperStairBlock if dir.is_horizontal() => {
            return state::set(s, "shape", connect::stairs_shape(level, s, pos));
        }
        C::WallBlock if dir != Direction::Down => return connect::wall_update(level, s, pos, dir, neighbor_state),
        C::FenceGateBlock => return misc::gate_update_shape(level, s, pos, dir, neighbor_state),
        C::ObserverBlock => return devices::observer_update_shape(level, s, pos, dir),
        C::NoteBlock => return devices::note_update_shape(level, s, pos, dir),
        C::PistonHeadBlock => return piston::head_update_shape(level, s, pos, dir),
        C::NetherPortalBlock => return portal::portal_update_shape(level, s, pos, dir, neighbor_state),
        C::FireBlock | C::SoulFireBlock => return crate::fire::update_shape(level, s, pos),
        _ => {}
    }
    if logic::is_instance(s, C::LeavesBlock) {
        return misc::leaves_update_shape(level, s, pos, neighbor_state);
    }
    if logic::is_instance(s, C::SpeleothemBlock) {
        return speleothem_update_shape(level, s, pos, dir);
    }
    if class == C::BrushableBlock {
        // `BrushableBlock.updateShape`: re-check the fall in 2 ticks.
        crate::level::schedule_block_tick(level, pos, BlockId::of(s), 2, crate::ticks::TickPriority::Normal);
        return s;
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
    if matches!(class, C::KelpBlock | C::KelpPlantBlock) {
        return kelp_update_shape(level, s, pos, dir, neighbor_state);
    }
    if class == C::SeagrassBlock {
        // `SeagrassBlock.updateShape`: the water around it flows again while it stays.
        let new = support::pop_off(level, s, pos, dir, neighbor_state).unwrap_or(s);
        if !kiln_data::blocks_types::is_air(new) {
            crate::level::schedule_fluid_tick(level, pos, crate::FluidType::Water, 5);
        }
        return new;
    }
    if let Some(new) = support::pop_off(level, s, pos, dir, neighbor_state) {
        return new;
    }
    s
}

/// `GrowingPlantHeadBlock.updateShape` / `GrowingPlantBodyBlock.updateShape` of kelp
/// (grows up, keeps its water flowing).
fn kelp_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> u16 {
    use kiln_data::blocks::default_state as d;
    let is_kelp = |b: u16| state::is(b, d::KELP) || state::is(b, d::KELP_PLANT);
    let head = state::is(s, d::KELP);
    if dir == Direction::Down {
        let below = level.block(pos.below());
        let survives = !state::is(below, d::MAGMA_BLOCK) && (is_kelp(below) || sturdy(below, Direction::Up, Support::Full));
        if !survives {
            crate::level::schedule_block_tick(level, pos, BlockId::of(s), 1, crate::ticks::TickPriority::Normal);
        }
        if head && is_kelp(level.block(pos.above())) {
            return d::KELP_PLANT;
        }
    }
    if dir == Direction::Up {
        if head && is_kelp(neighbor) {
            return d::KELP_PLANT;
        }
        if !head && !is_kelp(neighbor) {
            // `getHeadBlock().getStateForPlacement(random)`: a random age from the level random.
            use kiln_javamath::random::RandomSource;
            let age = level.random().next_int_bounded(25);
            return state::set(d::KELP, "age", &age.to_string());
        }
    }
    crate::level::schedule_fluid_tick(level, pos, crate::FluidType::Water, 5);
    s
}

/// `isSpeleothemWithDirection`: in `#speleothems` with this tip direction.
fn speleothem_toward(s: u16, dir: Direction) -> bool {
    crate::tags::is(s, "minecraft:speleothems") && state::get_dir(s, "vertical_direction") == Some(dir)
}

/// `SpeleothemBlock.updateShape` (pointed dripstone, sulfur spikes): thickness follows the
/// column; a tip that lost its support schedules its fall.
fn speleothem_update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if dir != Direction::Up && dir != Direction::Down {
        return s;
    }
    let tip = state::get_dir(s, "vertical_direction").unwrap_or(Direction::Up);
    let id = BlockId::of(s);
    if tip == Direction::Down && level.block_ticks().has_scheduled_tick(pos, id) {
        return s;
    }
    if dir == tip.opposite() {
        let behind = pos.relative(tip.opposite());
        let b = level.block(behind);
        let valid = sturdy(b, tip, Support::Full) || (speleothem_toward(b, tip) && state::same_block(b, s));
        if !valid {
            let delay = if tip == Direction::Down { 2 } else { 1 };
            crate::level::schedule_block_tick(level, pos, id, delay, crate::ticks::TickPriority::Normal);
            return s;
        }
    }
    let merge = state::get(s, "thickness") == Some("tip_merge");
    let ahead = level.block(pos.relative(tip));
    let thickness = if speleothem_toward(ahead, tip.opposite()) && state::same_block(ahead, s) {
        if merge || state::get(ahead, "thickness") == Some("tip_merge") { "tip_merge" } else { "tip" }
    } else if !speleothem_toward(ahead, tip) {
        "tip"
    } else if matches!(state::get(ahead, "thickness"), Some("tip" | "tip_merge")) {
        "frustum"
    } else if !speleothem_toward(level.block(pos.relative(tip.opposite())), tip) {
        "base"
    } else {
        "middle"
    };
    state::set(s, "thickness", thickness)
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
pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16, moved_by_piston: bool) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::LiquidBlock => fluid::liquid_block_changed(level, s, pos),
        C::RedstoneWireBlock => wire::on_place(level, s, pos, old),
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::on_place(level, s, pos),
        C::RepeaterBlock | C::ComparatorBlock => diode::on_place(level, s, pos),
        C::ObserverBlock => devices::observer_on_place(level, s, pos, old),
        C::TntBlock => devices::tnt_on_place(level, s, pos, old),
        C::PistonBaseBlock => piston::on_place(level, s, pos, old),
        C::HopperBlock => container::hopper_on_place(level, s, pos, old),
        C::SculkSensorBlock | C::CalibratedSculkSensorBlock => sculk::sensor_on_place(level, s, pos, old),
        C::SnifferEggBlock if !state::same_block(old, s) => misc::sniffer_egg_on_place(level, s, pos),
        C::FrogspawnBlock => misc::frogspawn_on_place(level, s, pos),
        // `BaseFireBlock.onPlace`: a new fire in an empty frame lights it; one that cannot
        // survive goes out.
        C::FireBlock | C::SoulFireBlock => {
            if !state::same_block(old, s) && !portal::fire_on_place(level, pos) && !crate::fire::can_survive(level, s, pos) {
                crate::remove_block(level, pos, false);
            }
            // `FireBlock.onPlace`: on every change of the state, the next tick.
            if logic::block_class(s) == C::FireBlock {
                crate::fire::schedule_fire_tick(level, pos);
            }
        }
        _ if logic::is_instance(s, C::FallingBlock) => misc::falling_schedule(level, s, pos),
        _ if logic::is_instance(s, C::BaseRailBlock) => rail::on_place(level, s, pos, old, moved_by_piston),
        _ => {}
    }
}

/// `affectNeighborsAfterRemoval`: `s` was just replaced at `pos`.
pub fn affect_neighbors_after_removal<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::RedstoneWireBlock => wire::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        C::RepeaterBlock | C::ComparatorBlock => diode::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        C::LeverBlock | C::ButtonBlock => components::attached_removed(level, s, pos, moved_by_piston),
        C::ObserverBlock => devices::observer_removed(level, s, pos),
        C::PistonHeadBlock => piston::head_removed(level, s, pos),
        C::SculkSensorBlock | C::CalibratedSculkSensorBlock => sculk::sensor_removed(level, s, pos),
        _ if logic::is_instance(s, C::BasePressurePlateBlock) => components::plate_removed(level, s, pos, moved_by_piston),
        _ if logic::is_instance(s, C::BaseRailBlock) => rail::affect_neighbors_after_removal(level, s, pos, moved_by_piston),
        _ => {}
    }
}

/// A scheduled block tick (`Block.tick`), when the block at `pos` is still the ticked block.
pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    use BlockClass as C;
    match logic::block_class(s) {
        C::RedstoneTorchBlock | C::RedstoneWallTorchBlock => torch::tick(level, s, pos),
        C::RepeaterBlock | C::ComparatorBlock => diode::tick(level, s, pos),
        C::ButtonBlock => components::button_tick(level, s, pos),
        C::RedstoneLampBlock => components::lamp_tick(level, s, pos),
        C::ObserverBlock => devices::observer_tick(level, s, pos),
        C::FireBlock => crate::fire::fire_tick(level, s, pos),
        C::LightningRodBlock | C::WeatheringLightningRodBlock => crate::weather::rod_tick(level, s, pos),
        C::DetectorRailBlock => rail::detector_tick(level, s, pos),
        // `ComposterBlock.tick`: a full composter's bone meal is ready.
        C::ComposterBlock if state::get_int(s, "level") == 7 => {
            crate::update::set_block(level, pos, state::set_int(s, "level", 8), crate::level::flags::ALL);
            level.effect(crate::level::Effect::Sound { pos, sound: "minecraft:block.composter.ready", volume: 1.0, pitch: 1.0 });
        }
        C::SculkSensorBlock | C::CalibratedSculkSensorBlock => sculk::sensor_tick(level, s, pos),
        C::SculkShriekerBlock => sculk::shrieker_tick(level, s, pos),
        C::SculkCatalystBlock => sculk::catalyst_tick(level, s, pos),
        C::SnifferEggBlock => misc::sniffer_egg_tick(level, s, pos),
        C::FrogspawnBlock => misc::frogspawn_tick(level, pos),
        // `ChestBlock.tick` / `BarrelBlock.tick` / `EnderChestBlock.tick` (recheck the openers)
        // and `DispenserBlock.tick` (dispense): the block entity's.
        C::BarrelBlock | C::EnderChestBlock | C::DispenserBlock | C::DropperBlock => level.block_entity_tick(pos, s),
        _ if container::is_chest(s) => level.block_entity_tick(pos, s),
        _ if logic::is_instance(s, C::BasePressurePlateBlock) => components::plate_tick(level, s, pos),
        _ if logic::is_instance(s, C::LeavesBlock) => misc::leaves_tick(level, s, pos),
        _ if logic::is_instance(s, C::FallingBlock) => misc::falling_tick(level, s, pos),
        _ => {}
    }
}

/// `randomTick`: leaf decay and lava setting fire (`LiquidBlock.randomTick`: the fluid's).
/// Other random-tick behaviour (crop growth, grass spread, ...) is not simulated yet; the
/// positions are still drawn so the random-tick sequence stays aligned.
pub fn random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if logic::is_instance(s, BlockClass::LeavesBlock) {
        misc::leaves_random_tick(level, s, pos);
    } else if logic::block_class(s) == BlockClass::LiquidBlock && logic::fluid(s).kind == kiln_data::block_logic::FluidKind::Lava {
        crate::fire::lava_random_tick(level, pos);
    }
}

/// `triggerEvent` for a block event; true if it should reach clients. Note blocks and
/// pistons are the block-event users implemented (chests, bells, ... are not).
pub fn trigger_event<L: Level>(level: &mut L, s: u16, pos: BlockPos, a: i32, b: i32) -> bool {
    match logic::block_class(s) {
        BlockClass::NoteBlock => devices::note_trigger(level, s, pos),
        BlockClass::PistonBaseBlock => piston::trigger_event(level, s, pos, a, b),
        // `BaseEntityBlock.triggerEvent`: the lids of chests, ender chests and shulker boxes
        // (their block entities answer event 1 with the openers count).
        BlockClass::EnderChestBlock | BlockClass::ShulkerBoxBlock => a == 1,
        _ if container::is_chest(s) => a == 1,
        _ => false,
    }
}
