//! `canSurvive` and the blocks that pop off when their support goes: vegetation, torches,
//! carpets, ladders, levers and buttons, diodes, redstone wire, doors and tall plants.

use super::sturdy;
use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::state;
use crate::tags;
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind, Support};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;

/// `Block.canSupportCenter`.
pub fn can_support_center(s: u16, dir: Direction) -> bool {
    if dir == Direction::Down && tags::is(s, "minecraft:unstable_bottom_center") {
        return false;
    }
    sturdy(s, dir, Support::Center)
}

/// `Block.canSupportRigidBlock` (the block below).
pub fn can_support_rigid(below: u16) -> bool {
    sturdy(below, Direction::Up, Support::Rigid)
}

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the way the block points
/// away from its support.
pub fn attached_direction(s: u16) -> Direction {
    match state::get(s, "face") {
        Some("ceiling") => Direction::Down,
        Some("floor") => Direction::Up,
        _ => state::get_dir(s, "facing").unwrap_or(Direction::North),
    }
}

/// `VegetationBlock.mayPlaceOn` and its overrides, for a plant whose ground is `below`.
fn may_place_on<L: Level + ?Sized>(level: &L, plant: u16, below: u16, below_pos: BlockPos) -> bool {
    use BlockClass as C;
    let tag = |t| tags::is(below, t);
    for &class in logic::class_info(logic::block_index(plant)).classes {
        return match class {
            C::MushroomBlock => block_props::solid_render(below),
            C::AzaleaBlock => tag("minecraft:supports_azalea"),
            C::DryVegetationBlock => tag("minecraft:supports_dry_vegetation"),
            C::WitherRoseBlock => tag("minecraft:supports_wither_rose"),
            C::CropBlock | C::PitcherCropBlock => tag("minecraft:supports_crops"),
            C::NetherWartBlock => tag("minecraft:supports_nether_wart"),
            C::NetherSproutsBlock => tag("minecraft:supports_nether_sprouts"),
            C::MangrovePropaguleBlock => tag("minecraft:supports_mangrove_propagule"),
            C::SeagrassBlock => sturdy(below, Direction::Up, Support::Full) && !tag("minecraft:cannot_support_seagrass"),
            C::CactusFlowerBlock => {
                tag("minecraft:support_override_cactus_flower") || sturdy(below, Direction::Up, Support::Center)
            }
            C::LilyPadBlock => {
                let f = logic::fluid(below);
                let above = logic::fluid(level.block(below_pos.above()));
                (f.kind == FluidKind::Water && f.source || tag("minecraft:supports_lily_pad")) && above.is_empty()
            }
            C::SeaPickleBlock => {
                block_props::collision(below).iter().all(|b| b[4] < 1.0) || sturdy(below, Direction::Up, Support::Full)
            }
            C::SmallDripleafBlock => {
                let water_above = {
                    let f = logic::fluid(level.block(below_pos.above()));
                    f.kind == FluidKind::Water && f.source
                };
                tag("minecraft:supports_small_dripleaf") || water_above && tag("minecraft:supports_vegetation")
            }
            C::AttachedStemBlock | C::StemBlock | C::NetherFungusBlock | C::NetherRootsBlock => {
                super::params_tag(plant).is_some_and(|t| tags::is(below, t))
            }
            C::VegetationBlock => tag("minecraft:supports_vegetation"),
            _ => continue,
        };
    }
    tag("minecraft:supports_vegetation")
}

/// `canSurvive` of the block in `s` at `pos`; blocks without a rule always survive.
pub fn can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    use BlockClass as C;
    let below = || level.block(pos.below());
    match logic::block_class(s) {
        C::WallTorchBlock | C::RedstoneWallTorchBlock => {
            let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
            sturdy(level.block(pos.relative(facing.opposite())), facing, Support::Full)
        }
        C::TorchBlock | C::RedstoneTorchBlock => can_support_center(below(), Direction::Up),
        C::LadderBlock => {
            let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
            sturdy(level.block(pos.relative(facing.opposite())), facing, Support::Full)
        }
        C::CarpetBlock | C::WoolCarpetBlock => !is_air(below()),
        C::RedstoneWireBlock => wire_can_survive_on(below()),
        C::RepeaterBlock | C::ComparatorBlock => can_support_rigid(below()),
        C::LeverBlock | C::ButtonBlock => {
            let dir = attached_direction(s).opposite();
            sturdy(level.block(pos.relative(dir)), dir.opposite(), Support::Full)
        }
        C::DoorBlock | C::WeatheringCopperDoorBlock => {
            if state::get(s, "half") == Some("lower") {
                sturdy(below(), Direction::Up, Support::Full)
            } else {
                state::same_block(below(), s)
            }
        }
        C::LeafLitterBlock => sturdy(below(), Direction::Up, Support::Full),
        C::MushroomBlock => {
            let b = below();
            tags::is(b, "minecraft:overrides_mushroom_light_requirement")
                || level.raw_brightness(pos, 0) < 13 && may_place_on(level, s, b, pos.below())
        }
        C::CropBlock | C::CarrotBlock | C::PotatoBlock | C::BeetrootBlock | C::TorchflowerCropBlock => {
            level.raw_brightness(pos, 0) >= 8 && may_place_on(level, s, below(), pos.below())
        }
        C::MangrovePropaguleBlock if state::get_bool(s, "hanging") => {
            tags::is(level.block(pos.above()), "minecraft:supports_hanging_mangrove_propagule")
        }
        C::SmallDripleafBlock if state::get(s, "half") == Some("lower") => may_place_on(level, s, below(), pos.below()),
        C::PitcherCropBlock if state::get(s, "half") == Some("lower") && level.raw_brightness(pos, 0) < 8 => false,
        _ if logic::is_instance(s, C::DoublePlantBlock) && state::get(s, "half") == Some("upper") => {
            let b = below();
            state::same_block(b, s) && state::get(b, "half") == Some("lower")
        }
        _ if logic::is_instance(s, C::VegetationBlock) => may_place_on(level, s, below(), pos.below()),
        _ => true,
    }
}

/// `RedstoneWireBlock.canSurviveOn`.
pub fn wire_can_survive_on(below: u16) -> bool {
    sturdy(below, Direction::Up, Support::Full) || state::is(below, d::HOPPER)
}

/// The pop-off rules of `updateShape`: the new state (air when the block breaks), or `None`
/// when the block has no such rule for this neighbour.
pub fn pop_off<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> Option<u16> {
    use BlockClass as C;
    let breaks = |cond: bool| cond && !can_survive(level, s, pos);
    let class = logic::block_class(s);
    let broken = match class {
        C::WallTorchBlock | C::RedstoneWallTorchBlock | C::LadderBlock => {
            breaks(state::get_dir(s, "facing").is_some_and(|f| dir.opposite() == f))
        }
        C::TorchBlock | C::RedstoneTorchBlock => breaks(dir == Direction::Down),
        C::CarpetBlock | C::WoolCarpetBlock => breaks(true),
        C::LeverBlock | C::ButtonBlock => breaks(attached_direction(s).opposite() == dir),
        C::RepeaterBlock => dir == Direction::Down && !can_support_rigid(neighbor),
        C::RedstoneWireBlock => dir == Direction::Down && !wire_can_survive_on(neighbor),
        _ if logic::is_instance(s, C::DoorBlock) => return door_update(level, s, pos, dir, neighbor),
        _ if logic::is_instance(s, C::DoublePlantBlock) => {
            let half = state::get(s, "half");
            let toward_other = (half == Some("lower")) == (dir == Direction::Up);
            let other_half_gone = dir.axis() == crate::pos::Axis::Y
                && toward_other
                && !(state::same_block(neighbor, s) && state::get(neighbor, "half") != half);
            other_half_gone || breaks(true)
        }
        _ if logic::is_instance(s, C::VegetationBlock) => breaks(true),
        _ => return None,
    };
    Some(if broken { d::AIR } else { s })
}

/// `DoorBlock.updateShape`: the halves copy each other; the lower half needs its floor.
fn door_update<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> Option<u16> {
    let half = state::get(s, "half");
    if dir.axis() == crate::pos::Axis::Y {
        if (half == Some("lower")) == (dir == Direction::Up) {
            return Some(if logic::is_instance(neighbor, BlockClass::DoorBlock) && state::get(neighbor, "half") != half {
                state::set(neighbor, "half", half.unwrap_or("lower"))
            } else {
                d::AIR
            });
        }
    }
    if half == Some("lower") && dir == Direction::Down && !can_survive(level, s, pos) {
        return Some(d::AIR);
    }
    Some(s)
}
