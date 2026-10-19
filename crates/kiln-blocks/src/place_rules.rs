//! `getStateForPlacement` and `canSurvive` of the blocks whose rules are more than the
//! property pattern of [`crate::placement`]: signs, banners, skulls, hanging blocks, plants
//! that need water or a given ground, multiface blocks, speleothems, and the rest of the
//! blocks vanilla refuses to place in some surroundings.

use crate::behaviour::{growth, speleothem, sturdy, sturdy_at, support, wet};
use crate::level::Level;
use crate::placement::Ctx;
use crate::pos::{Axis, BlockPos, Direction};
use crate::redstone::has_neighbor_signal;
use crate::state;
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind, Support};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;

/// `RotationSegment.convertToSegment(float)`.
pub(crate) fn rotation_segment(angle: f32) -> i32 {
    ((angle * 16.0 / 360.0) as f64 + 0.5).floor() as i32 & 15
}

/// `RotationSegment.convertToSegment(Direction)`.
fn direction_segment(dir: Direction) -> i32 {
    rotation_segment((dir.to_2d() & 3) as f32 * 90.0)
}

/// `RotationSegment.convertToDirection`: the cardinal direction of a segment.
fn segment_direction(segment: i32) -> Option<Direction> {
    (segment & 3 == 0).then(|| Direction::from_2d(segment / 4))
}

/// `FluidState.is(FluidTags.WATER)` at `pos`.
pub(crate) fn water_at<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    logic::fluid(level.block(pos)).kind == FluidKind::Water
}

/// `FluidState.isSourceOfType(WATER)` of a state.
fn water_source(s: u16) -> bool {
    let f = logic::fluid(s);
    f.kind == FluidKind::Water && f.source
}

/// `MultifaceBlock.canAttachTo` toward `dir` from `pos`.
fn multiface_attach<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> bool {
    growth::acceptable_neighbour(level, pos.relative(dir), dir)
}

fn is_air(s: u16) -> bool {
    kiln_data::blocks_types::is_air(s)
}

// ---------------------------------------------------------------- canSurvive

/// `canSurvive` of the classes below; `None` for the ones this module has no rule for.
pub(crate) fn can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> Option<bool> {
    use BlockClass as C;
    let below = || level.block(pos.below());
    let above = || level.block(pos.above());
    let facing = || state::get_dir(s, "facing").unwrap_or(Direction::North);
    for &class in logic::class_info(logic::block_index(s)).classes {
        return Some(match class {
        C::StandingSignBlock | C::BannerBlock | C::CakeBlock => logic::is_solid(below()),
        C::WallSignBlock | C::WallBannerBlock => logic::is_solid(level.block(pos.relative(facing().opposite()))),
        C::CeilingHangingSignBlock => sturdy_at(level, pos.above(), Direction::Down, Support::Center),
        C::AmethystClusterBlock => sturdy_at(level, pos.relative(facing().opposite()), facing(), Support::Full),
        C::BigDripleafBlock => crate::behaviour::misc3::dripleaf_can_survive(level, s, pos),
        C::BambooStalkBlock | C::BambooSaplingBlock => crate::tags::is(below(), "minecraft:supports_bamboo"),
        C::CandleBlock => support::can_support_center_at(level, pos.below(), Direction::Up),
        C::LanternBlock | C::WeatheringLanternBlock => {
            let dir = if state::get_bool(s, "hanging") { Direction::Down } else { Direction::Up };
            support::can_support_center_at(level, pos.relative(dir.opposite()), dir)
        }
        C::SporeBlossomBlock => support::can_support_center_at(level, pos.above(), Direction::Down) && !water_at(level, pos),
        C::HangingRootsBlock => sturdy_at(level, pos.above(), Direction::Down, Support::Full),
        C::TripWireHookBlock => {
            let f = facing();
            f.is_horizontal() && sturdy_at(level, pos.relative(f.opposite()), f, Support::Full)
        }
        C::ShelfMushroomBlock => sturdy_at(level, pos.relative(facing().opposite()), facing(), Support::Full),
        C::MossyCarpetBlock => {
            let b = below();
            if state::get_bool(s, "bottom") { !is_air(b) } else { state::same_block(b, s) && state::get_bool(b, "bottom") }
        }
        C::HangingMossBlock => {
            let a = above();
            growth::acceptable_neighbour(level, pos.above(), Direction::Up) || state::same_block(a, s)
        }
        C::MultifaceBlock => {
            let mut any = false;
            for dir in Direction::ALL {
                if state::get_bool(s, dir.name()) {
                    if !multiface_attach(level, pos, dir) {
                        return Some(false);
                    }
                    any = true;
                }
            }
            any
        }
        C::SpeleothemBlock => speleothem::valid_placement(level, s, pos, state::get_dir(s, "vertical_direction").unwrap_or(Direction::Up)),
        C::ScaffoldingBlock => wet::scaffolding_distance(level, pos) < 7,
        C::SeaPickleBlock => sea_pickle_may_place_on(below()),
        C::PathBlock => !logic::is_solid(above()) || logic::is_instance(above(), C::FenceGateBlock),
        _ => continue,
        });
    }
    None
}

/// `SeaPickleBlock.mayPlaceOn`: a top face with something on it, or a sturdy one.
fn sea_pickle_may_place_on(below: u16) -> bool {
    block_props::collision(below).iter().any(|b| b[4] >= 1.0) || sturdy(below, Direction::Up, Support::Full)
}

/// `SeaPickleBlock.mayPlaceOn` for the plant rules in [`support`].
pub(crate) fn sea_pickle_ground(below: u16) -> bool {
    sea_pickle_may_place_on(below)
}

// ---------------------------------------------------------------- updateShape

/// The pop-off of `updateShape` of these classes: `Some(new state)`, `None` for the others.
pub(crate) fn update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> Option<u16> {
    use BlockClass as C;
    let breaks = |cond: bool| if cond && !support::can_survive(level, s, pos) { d::AIR } else { s };
    let facing = || state::get_dir(s, "facing").unwrap_or(Direction::North);
    for &class in logic::class_info(logic::block_index(s)).classes {
        return Some(match class {
        C::StandingSignBlock | C::BannerBlock | C::CakeBlock => breaks(dir == Direction::Down),
        C::WallSignBlock | C::ShelfMushroomBlock => breaks(dir.opposite() == facing()),
        C::WallBannerBlock | C::AmethystClusterBlock => breaks(dir == facing().opposite()),
        C::CeilingHangingSignBlock | C::SporeBlossomBlock | C::HangingRootsBlock => breaks(dir == Direction::Up),
        C::LanternBlock | C::WeatheringLanternBlock => {
            let connected = if state::get_bool(s, "hanging") { Direction::Down } else { Direction::Up };
            breaks(connected.opposite() == dir)
        }
        C::SeaPickleBlock => breaks(true),
        C::HugeMushroomBlock => {
            if state::same_block(neighbor, s) { state::set_bool(s, dir.name(), false) } else { s }
        }
        C::BambooSaplingBlock => {
            if !support::can_survive(level, s, pos) {
                d::AIR
            } else if dir == Direction::Up && state::is(neighbor, d::BAMBOO) {
                d::BAMBOO
            } else {
                s
            }
        }
        C::MultifaceBlock => {
            if !(Direction::ALL.iter().any(|&f| state::get_bool(s, f.name()))) {
                d::AIR
            } else if state::get_bool(s, dir.name()) && !multiface_attach(level, pos, dir) {
                // `MultifaceBlock.removeFace`: the last face takes the block with it.
                let n = state::set_bool(s, dir.name(), false);
                if Direction::ALL.iter().any(|&f| state::get_bool(n, f.name())) { n } else { d::AIR }
            } else {
                s
            }
        }
        C::MossyCarpetBlock => {
            if !support::can_survive(level, s, pos) {
                d::AIR
            } else {
                let n = mossy_carpet_state(level, s, pos, false);
                if mossy_carpet_faces(n) { n } else { d::AIR }
            }
        }
        _ => continue,
        });
    }
    None
}

// ---------------------------------------------------------------- mossy carpet

/// `MossyCarpetBlock.hasFaces`: a side that is not none.
pub(crate) fn mossy_carpet_faces(s: u16) -> bool {
    state::get_bool(s, "bottom") || Direction::HORIZONTAL.iter().any(|d| state::get(s, d.name()) != Some("none"))
}

/// `MossyCarpetBlock.canSupportAtFace`.
fn mossy_carpet_support<L: Level + ?Sized>(level: &L, pos: BlockPos, dir: Direction) -> bool {
    dir != Direction::Up && multiface_attach(level, pos, dir)
}

/// `MossyCarpetBlock.getUpdatedState(state, level, pos, flag)`.
pub(crate) fn mossy_carpet_state<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, flag: bool) -> u16 {
    let flag = flag | state::get_bool(s, "bottom");
    let (mut above, mut below): (Option<u16>, Option<u16>) = (None, None);
    let mut s = s;
    for dir in Direction::HORIZONTAL {
        let mut side = if mossy_carpet_support(level, pos, dir) {
            if flag { "low" } else { state::get(s, dir.name()).unwrap_or("none") }
        } else {
            "none"
        };
        if side == "low" {
            let a = *above.get_or_insert_with(|| level.block(pos.above()));
            if state::is(a, d::PALE_MOSS_CARPET) && state::get(a, dir.name()) != Some("none") && !state::get_bool(a, "bottom") {
                side = "tall";
            }
            if !state::get_bool(s, "bottom") {
                let b = *below.get_or_insert_with(|| level.block(pos.below()));
                if state::is(b, d::PALE_MOSS_CARPET) && state::get(b, dir.name()) == Some("none") {
                    side = "none";
                }
            }
        }
        s = state::set(s, dir.name(), side);
    }
    s
}

// ---------------------------------------------------------------- getStateForPlacement

/// The result for the classes handled here: `Some(state or none)`; `None` for the others.
pub(crate) fn state_for_placement<L: Level + ?Sized>(c: &Ctx<L>, d0: u16) -> Option<Option<u16>> {
    use BlockClass as C;
    let level = c.level;
    let pos = c.pos;
    // the horizontal directions the player looks along first (wall-mounted things)
    let first_wall = |f: &dyn Fn(Direction) -> Option<u16>| c.nearest().into_iter().filter(|dir| dir.is_horizontal()).find_map(f);
    for &class in logic::class_info(logic::block_index(d0)).classes {
        return Some(match class {
        C::StandingSignBlock => Some(c.waterlogged(state::set_int(d0, "rotation", rotation_segment(c.yaw + 180.0)))),
        C::BannerBlock => Some(state::set_int(d0, "rotation", rotation_segment(c.yaw + 180.0))),
        C::WallSignBlock => first_wall(&|dir| {
            let s = state::set_dir(d0, "facing", dir.opposite());
            support::can_survive(level, s, pos).then(|| c.waterlogged(s))
        }),
        C::WallBannerBlock => first_wall(&|dir| {
            let s = state::set_dir(d0, "facing", dir.opposite());
            support::can_survive(level, s, pos).then_some(s)
        }),
        C::ShelfMushroomBlock => first_wall(&|dir| {
            let s = state::set_dir(d0, "facing", dir.opposite());
            support::can_survive(level, s, pos).then_some(s)
        }),
        C::BaseCoralWallFanBlock => first_wall(&|dir| {
            let s = c.waterlogged(state::set_dir(d0, "facing", dir.opposite()));
            support::can_survive(level, s, pos).then_some(s)
        }),
        C::TripWireHookBlock => first_wall(&|dir| {
            let s = state::set_bool(state::set_bool(state::set_dir(d0, "facing", dir.opposite()), "powered", false), "attached", false);
            support::can_survive(level, s, pos).then_some(s)
        }),
        C::WallSkullBlock => {
            let powered = has_neighbor_signal(level, pos);
            first_wall(&|dir| {
                let s = state::set_dir(state::set_bool(d0, "powered", powered), "facing", dir.opposite());
                (!block_props::replaceable(level.block(pos.relative(dir)))).then_some(s)
            })
        }
        C::SkullBlock => {
            let s = state::set_bool(d0, "powered", has_neighbor_signal(level, pos));
            Some(state::set_int(s, "rotation", rotation_segment(c.yaw)))
        }
        C::CeilingHangingSignBlock => Some(ceiling_hanging_sign(c, d0)),
        C::AmethystClusterBlock => Some(c.waterlogged(state::set_dir(d0, "facing", c.face))),
        C::BigDripleafBlock => {
            let below = level.block(pos.below());
            let on_leaf = state::same_block(below, d0) || state::is(below, d::BIG_DRIPLEAF_STEM);
            let facing = if on_leaf { state::get_dir(below, "facing").unwrap_or(Direction::North) } else { c.horizontal().opposite() };
            let s = state::set_bool(d0, "waterlogged", water_source(level.block(pos)));
            Some(state::set_dir(s, "facing", facing))
        }
        C::BambooStalkBlock => {
            if !logic::fluid(level.block(pos)).is_empty() {
                return Some(None);
            }
            let below = level.block(pos.below());
            if !crate::tags::is(below, "minecraft:supports_bamboo") {
                return Some(None);
            }
            Some(if state::is(below, d::BAMBOO_SAPLING) {
                state::set_int(d0, "age", 0)
            } else if state::is(below, d::BAMBOO) {
                state::set_int(d0, "age", (state::get_int(below, "age") > 0) as i32)
            } else {
                let a = level.block(pos.above());
                if state::is(a, d::BAMBOO) { state::set_int(d0, "age", state::get_int(a, "age")) } else { d::BAMBOO_SAPLING }
            })
        }
        C::CandleBlock => {
            let here = level.block(pos);
            Some(if state::same_block(here, d0) {
                let n = state::get_int(here, "candles");
                state::set_int(here, "candles", if n >= 4 { 1 } else { n + 1 })
            } else {
                c.waterlogged(d0)
            })
        }
        C::ChainBlock | C::WeatheringCopperChainBlock => Some(c.waterlogged(state::set(d0, "axis", axis_name(c.face.axis())))),
        C::LanternBlock | C::WeatheringLanternBlock => c.nearest().into_iter().filter(|dir| dir.axis() == Axis::Y).find_map(|dir| {
            let s = state::set_bool(d0, "hanging", dir == Direction::Up);
            support::can_survive(level, s, pos).then(|| c.waterlogged(s))
        }),
        C::ChorusPlantBlock => Some(growth::chorus_plant_connections(level, pos, d0)),
        C::LightBlock => Some(d0),
        C::LightningRodBlock | C::WeatheringLightningRodBlock => Some(c.waterlogged(state::set_dir(d0, "facing", c.face))),
        C::EndRodBlock => {
            let behind = level.block(pos.relative(c.face.opposite()));
            let facing = if state::same_block(behind, d0) && state::get_dir(behind, "facing") == Some(c.face) { c.face.opposite() } else { c.face };
            Some(state::set_dir(d0, "facing", facing))
        }
        C::CampfireBlock => {
            let wet = water_source(level.block(pos));
            let s = state::set_bool(d0, "waterlogged", wet);
            let s = state::set_bool(s, "signal_fire", state::is(level.block(pos.below()), d::HAY_BLOCK));
            let s = state::set_bool(s, "lit", !wet);
            Some(state::set_dir(s, "facing", c.horizontal()))
        }
        C::AnvilBlock => Some(state::set_dir(d0, "facing", c.horizontal().clockwise())),
        C::CalibratedSculkSensorBlock => Some(state::set_dir(c.waterlogged(d0), "facing", c.horizontal())),
        C::ConcretePowderBlock => Some(if concrete_should_solidify(level, pos) { concrete_of(d0) } else { d0 }),
        C::FarmlandBlock | C::PathBlock => Some(if support::can_survive(level, d0, pos) { d0 } else { d::DIRT }),
        C::KelpBlock => {
            let f = logic::fluid(level.block(pos));
            if f.kind != FluidKind::Water || f.amount != 8 {
                return Some(None);
            }
            Some(d0)
        }
        C::SeagrassBlock => {
            let f = logic::fluid(level.block(pos));
            if f.kind != FluidKind::Water || f.amount != 8 {
                return Some(None);
            }
            Some(d0)
        }
        C::SeaPickleBlock => {
            let here = level.block(pos);
            Some(if state::same_block(here, d0) { state::set_int(here, "pickles", (state::get_int(here, "pickles") + 1).min(4)) } else { c.waterlogged(d0) })
        }
        C::ScaffoldingBlock => {
            let distance = wet::scaffolding_distance(level, pos);
            let s = c.waterlogged(state::set_int(d0, "distance", distance));
            Some(state::set_bool(s, "bottom", wet::scaffolding_is_bottom(level, pos, distance)))
        }
        C::HangingRootsBlock => Some(c.waterlogged(d0)),
        C::VineBlock => {
            let here = level.block(pos);
            let same = state::same_block(here, d0);
            let s = if same { here } else { d0 };
            let found = c.nearest().into_iter().filter(|&dir| dir != Direction::Down).find_map(|dir| {
                let has = same && state::get_bool(s, dir.name());
                (!has && growth::vine_can_support_at_face(level, pos, dir)).then(|| state::set_bool(s, dir.name(), true))
            });
            found.or(if same { Some(s) } else { None })
        }
        C::MultifaceBlock => c.nearest().into_iter().find_map(|dir| {
            let here = level.block(pos);
            let valid = !(state::same_block(here, d0) && state::get_bool(here, dir.name())) && multiface_attach(level, pos, dir);
            valid.then(|| {
                let s = if state::same_block(here, d0) {
                    here
                } else if water_source(here) {
                    state::set_bool(d0, "waterlogged", true)
                } else {
                    d0
                };
                state::set_bool(s, dir.name(), true)
            })
        }),
        C::MossyCarpetBlock => Some(mossy_carpet_state(level, d0, pos, true)),
        // `HugeMushroomBlock.getStateForPlacement`: a face is shown unless the same block is beside it.
        C::HugeMushroomBlock => {
            let mut s = d0;
            for dir in Direction::ALL {
                s = state::set_bool(s, dir.name(), !state::same_block(level.block(pos.relative(dir)), d0));
            }
            Some(s)
        }
        C::TurtleEggBlock => {
            let here = level.block(pos);
            Some(if state::same_block(here, d0) { state::set_int(here, "eggs", (state::get_int(here, "eggs") + 1).min(4)) } else { d0 })
        }
        C::SnowLayerBlock => {
            let here = level.block(pos);
            Some(if state::same_block(here, d0) { state::set_int(here, "layers", (state::get_int(here, "layers") + 1).min(8)) } else { d0 })
        }
        // `SmallDripleafBlock.getStateForPlacement`: a double plant that faces the player and takes the water that is there.
        C::SmallDripleafBlock => {
            if !level.in_bounds(pos.above()) || !block_props::replaceable(level.block(pos.above())) {
                return Some(None);
            }
            let s = state::set_dir(d0, "facing", c.horizontal().opposite());
            Some(state::set_bool(s, "waterlogged", water_at(level, pos)))
        }
        // `SegmentableBlock.getStateForPlacement`: petals and litter pile up on themselves.
        _ if logic::implements(d0, kiln_data::block_logic::interface::SEGMENTABLE_BLOCK) => {
            let amount = if state::has(d0, "flower_amount") { "flower_amount" } else { "segment_amount" };
            let here = level.block(pos);
            Some(if state::same_block(here, d0) {
                state::set_int(here, amount, (state::get_int(here, amount) + 1).min(4))
            } else {
                state::set_dir(d0, "facing", c.horizontal().opposite())
            })
        }
        C::PitcherCropBlock => Some(d0),
        C::MangrovePropaguleBlock => Some(state::set_int(c.waterlogged(d0), "age", 4)),
        C::PotentSulfurBlock => Some(potent_sulfur_state(level, d0, pos)),
        C::SpeleothemBlock => {
            let want = if c.pitch < 0.0 { Direction::Down } else { Direction::Up };
            let tip = if speleothem::valid_placement(level, d0, pos, want) {
                Some(want)
            } else if speleothem::valid_placement(level, d0, pos, want.opposite()) {
                Some(want.opposite())
            } else {
                None
            };
            tip.map(|tip| {
                let thickness = crate::behaviour::speleothem_thickness(level, d0, pos, tip, !c.sneaking);
                let s = state::set(state::set_dir(d0, "vertical_direction", tip), "thickness", thickness);
                c.waterlogged(s)
            })
        }
        _ => continue,
        });
    }
    None
}

fn axis_name(a: Axis) -> &'static str {
    match a {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

/// `CeilingHangingSignBlock.getStateForPlacement`.
fn ceiling_hanging_sign<L: Level + ?Sized>(c: &Ctx<L>, d0: u16) -> u16 {
    let level = c.level;
    let above_pos = c.pos.above();
    let above = level.block(above_pos);
    let facing = Direction::from_yaw(c.yaw as f64);
    let mut attached = !growth::collision_face_full(above, Direction::Down) || c.sneaking;
    if crate::tags::is(above, "minecraft:all_hanging_signs") && !c.sneaking {
        if logic::block_class(above) == BlockClass::WallHangingSignBlock {
            let f = state::get_dir(above, "facing").unwrap_or(Direction::North);
            if f.axis() == facing.axis() {
                attached = false;
            }
        } else if let Some(r) = segment_direction(state::get_int(above, "rotation")).filter(|_| state::has(above, "rotation")) {
            if r.axis() == facing.axis() {
                attached = false;
            }
        }
    }
    let rotation = if !attached { direction_segment(facing.opposite()) } else { rotation_segment(c.yaw + 180.0) };
    let s = state::set_int(state::set_bool(d0, "attached", attached), "rotation", rotation);
    c.waterlogged(s)
}

/// `ConcretePowderBlock.shouldSolidify`.
pub(crate) fn concrete_should_solidify<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let water = |s: u16| logic::fluid(s).kind == FluidKind::Water;
    if water(level.block(pos)) {
        return true;
    }
    // (the block below only counts when the powder itself is in water, which returned above)
    Direction::ALL.iter().filter(|&&dir| dir != Direction::Down).any(|&dir| {
        let n = level.block(pos.relative(dir));
        water(n) && !sturdy_at(level, pos.relative(dir), dir.opposite(), Support::Full)
    })
}

/// The concrete a concrete powder turns into.
pub(crate) fn concrete_of(powder: u16) -> u16 {
    let name = crate::state::BlockId::of(powder).name().replace("_powder", "");
    crate::state::BlockId::by_name(&name).map_or(powder, |b| b.default_state())
}

/// `PotentSulfurBlock.validBlockState`'s wet/dry part (the geyser states need the block entity).
fn potent_sulfur_state<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> u16 {
    if !water_source(level.block(pos.above())) {
        return state::set(s, "potent_sulfur_state", "dry");
    }
    let below = level.block(pos.below());
    let source_if_fluid = || {
        let f = logic::fluid(below);
        f.is_empty() || f.source
    };
    if crate::tags::is(below, "minecraft:causes_continuous_geyser_eruptions") && source_if_fluid() {
        return state::set(s, "potent_sulfur_state", "continuous");
    }
    if crate::tags::is(below, "minecraft:causes_periodic_geyser_eruptions") && source_if_fluid() {
        return if state::get(s, "potent_sulfur_state") == Some("erupting") { s } else { state::set(s, "potent_sulfur_state", "dormant") };
    }
    state::set(s, "potent_sulfur_state", "wet")
}
