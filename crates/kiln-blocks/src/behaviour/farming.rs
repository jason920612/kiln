//! Farming blocks: farmland, crops (wheat, carrots, potatoes, beetroots, torchflower, pitcher),
//! stems and their fruit, nether wart, cocoa, sweet berries, sugar cane, cactus and bamboo.
//! Their random ticks, scheduled ticks and the shape updates that schedule them, each
//! following the vanilla method call by call (the order and number of random draws is part
//! of the behaviour).

use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{destroy_block, set_block, set_block_and_update};
use kiln_data::block_logic::{self as logic, BlockClass, FluidKind};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

/// `BlockState.is(Block)` for a block given by name.
fn is_named(s: u16, name: &str) -> bool {
    BlockId::of(s).name() == name
}

fn water(s: u16) -> bool {
    logic::fluid(s).kind == FluidKind::Water
}

fn lava(s: u16) -> bool {
    logic::fluid(s).kind == FluidKind::Lava
}

/// `Level.getRawBrightness(pos, 0)`.
fn light<L: Level + ?Sized>(level: &L, pos: BlockPos) -> i32 {
    level.raw_brightness(pos, 0)
}

// ---------------------------------------------------------------------------------- dispatch

/// `randomTick` of the farming blocks; false for other blocks.
pub fn random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) -> bool {
    use BlockClass as C;
    match logic::block_class(s) {
        C::FarmlandBlock => farmland_random_tick(level, s, pos),
        C::BeetrootBlock | C::TorchflowerCropBlock => {
            // `BeetrootBlock` / `TorchflowerCropBlock.randomTick`: a third of the rolls grow.
            if level.random().next_int_bounded(3) != 0 {
                crop_random_tick(level, s, pos);
            }
        }
        C::CropBlock | C::CarrotBlock | C::PotatoBlock => crop_random_tick(level, s, pos),
        C::PitcherCropBlock => pitcher_random_tick(level, s, pos),
        C::NetherWartBlock => nether_wart_random_tick(level, s, pos),
        C::StemBlock => stem_random_tick(level, s, pos),
        C::CocoaBlock => cocoa_random_tick(level, s, pos),
        C::SweetBerryBushBlock => sweet_berry_random_tick(level, s, pos),
        C::SugarCaneBlock => sugar_cane_random_tick(level, s, pos),
        C::CactusBlock => cactus_random_tick(level, s, pos),
        C::BambooStalkBlock => bamboo_random_tick(level, s, pos),
        C::BambooSaplingBlock => bamboo_sapling_random_tick(level, pos),
        _ => return false,
    }
    true
}

/// A scheduled `tick` of the farming blocks; false for other blocks.
pub fn tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) -> bool {
    use BlockClass as C;
    match logic::block_class(s) {
        C::FarmlandBlock => {
            if !farmland_can_survive(level, pos) {
                turn_to_dirt(level, s, pos);
            }
        }
        // `SugarCaneBlock.tick` / `CactusBlock.tick` / `BambooStalkBlock.tick`: without support
        // the block breaks and drops.
        C::SugarCaneBlock | C::CactusBlock | C::BambooStalkBlock => {
            if !can_survive(level, s, pos).unwrap_or(true) {
                destroy_block(level, pos, true, flags::LIMIT);
            }
        }
        _ => return false,
    }
    true
}

/// `updateShape` of the farming blocks that has a rule of its own: `Some(new state)`.
pub fn update_shape<L: Level>(level: &mut L, s: u16, pos: BlockPos, dir: Direction, neighbor: u16) -> Option<u16> {
    use BlockClass as C;
    match logic::block_class(s) {
        // `FarmlandBlock.updateShape`: a block put on top (or the water going) re-checks next tick.
        C::FarmlandBlock => {
            if dir == Direction::Up && !farmland_can_survive(level, pos) {
                schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
            }
            Some(s)
        }
        // `SugarCaneBlock` / `CactusBlock` / `BambooStalkBlock.updateShape`: re-check next tick.
        C::SugarCaneBlock | C::CactusBlock => {
            if !can_survive(level, s, pos).unwrap_or(true) {
                schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
            }
            Some(s)
        }
        C::BambooStalkBlock => {
            if !can_survive(level, s, pos).unwrap_or(true) {
                schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
            }
            // A taller bamboo above takes over the age of the one below.
            if dir == Direction::Up && is_named(neighbor, "minecraft:bamboo") && state::get_int(neighbor, "age") > state::get_int(s, "age") {
                return Some(state::set_int(s, "age", 1 - state::get_int(s, "age")));
            }
            Some(s)
        }
        C::BambooSaplingBlock => {
            if !can_survive(level, s, pos).unwrap_or(true) {
                return Some(d::AIR);
            }
            if dir == Direction::Up && is_named(neighbor, "minecraft:bamboo") {
                return Some(d::BAMBOO);
            }
            Some(s)
        }
        C::CocoaBlock => {
            if Some(dir) == state::get_dir(s, "facing") && !can_survive(level, s, pos).unwrap_or(true) {
                return Some(d::AIR);
            }
            Some(s)
        }
        C::AttachedStemBlock => {
            // The fruit gone: the stem is a full-grown stem again.
            if Some(dir) == state::get_dir(s, "facing") && !is_named(neighbor, attached_fruit(s)) {
                let stem = BlockId::by_name(attached_stem_of(s)).map(|b| b.default_state()).unwrap_or(d::AIR);
                return Some(state::set(stem, "age", "7"));
            }
            None
        }
        C::PitcherCropBlock => {
            // A young pitcher plant is one block: it only needs its ground.
            if state::get_int(s, "age") < 3 {
                return Some(if super::support::can_survive(level, s, pos) { s } else { d::AIR });
            }
            None
        }
        _ => None,
    }
}

/// `canSurvive` of the farming blocks that have a rule beyond `VegetationBlock`'s.
pub fn can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> Option<bool> {
    use BlockClass as C;
    Some(match logic::block_class(s) {
        C::FarmlandBlock => farmland_can_survive(level, pos),
        C::SugarCaneBlock => sugar_cane_can_survive(level, pos),
        C::CactusBlock => cactus_can_survive(level, pos),
        C::BambooStalkBlock | C::BambooSaplingBlock => tags::is(level.block(pos.below()), "minecraft:supports_bamboo"),
        C::CocoaBlock => {
            let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
            tags::is(level.block(pos.relative(facing)), "minecraft:supports_cocoa")
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------- farmland

/// `FarmlandBlock.canSurvive`: nothing solid on top unless it keeps farmland.
fn farmland_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let above = level.block(pos.above());
    !logic::is_solid(above) || tags::is(above, "minecraft:maintains_farmland")
}

/// `FarmlandBlock.isNearWater`: water within 4 blocks horizontally, from this level to one above.
fn near_water<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    for y in 0..=1 {
        for z in -4..=4 {
            for x in -4..=4 {
                if water(level.block(pos.offset(x, y, z))) {
                    return true;
                }
            }
        }
    }
    false
}

/// `FarmlandBlock.turnToBaseBlock` (no entity): dirt, pushing entities up, and the game event.
fn turn_to_dirt<L: Level>(level: &mut L, _s: u16, pos: BlockPos) {
    set_block_and_update(level, pos, d::DIRT);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: d::DIRT });
}

fn farmland_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let moisture = state::get_int(s, "moisture");
    if near_water(level, pos) || level.is_raining_at(pos.above()) {
        if moisture < 7 {
            set_block(level, pos, state::set_int(s, "moisture", 7), flags::CLIENTS);
        }
    } else if moisture > 0 {
        set_block(level, pos, state::set_int(s, "moisture", moisture - 1), flags::CLIENTS);
    } else if !tags::is(level.block(pos.above()), "minecraft:maintains_farmland") {
        turn_to_dirt(level, s, pos);
    }
}

// ---------------------------------------------------------------------------------- crops

/// `CropBlock.getGrowthSpeed(block, level, pos)`: the soil under and around, halved by the
/// same crop in rows and by the same crop diagonally.
pub fn growth_speed<L: Level + ?Sized>(level: &L, crop: u16, pos: BlockPos) -> f32 {
    let mut speed = 1.0f32;
    let below = pos.below();
    for i in -1..=1 {
        for j in -1..=1 {
            let mut g = 0.0f32;
            let soil = level.block(below.offset(i, 0, j));
            if tags::is(soil, "minecraft:grows_crops") {
                g = 1.0;
                if state::get_int(soil, "moisture") > 0 {
                    g = 3.0;
                }
            }
            if i != 0 || j != 0 {
                g /= 4.0;
            }
            speed += g;
        }
    }
    let is_crop = |p: BlockPos| state::same_block(level.block(p), crop);
    let (north, south, west, east) = (pos.relative(Direction::North), pos.relative(Direction::South), pos.relative(Direction::West), pos.relative(Direction::East));
    let row_x = is_crop(west) || is_crop(east);
    let row_z = is_crop(north) || is_crop(south);
    if row_x && row_z {
        speed /= 2.0;
    } else {
        let diagonal = is_crop(west.relative(Direction::North))
            || is_crop(east.relative(Direction::North))
            || is_crop(east.relative(Direction::South))
            || is_crop(west.relative(Direction::South));
        if diagonal {
            speed /= 2.0;
        }
    }
    speed
}

/// `25 / speed + 1` as `nextInt`'s bound.
fn growth_bound(speed: f32) -> i32 {
    (25.0f32 / speed) as i32 + 1
}

fn crop_max_age(s: u16) -> i32 {
    match logic::block_class(s) {
        BlockClass::BeetrootBlock => 3,
        BlockClass::TorchflowerCropBlock => 2,
        _ => 7,
    }
}

/// `CropBlock.getStateForAge` (a grown torchflower crop is the flower).
fn crop_state_for_age(s: u16, age: i32) -> u16 {
    if logic::block_class(s) == BlockClass::TorchflowerCropBlock && age == 2 {
        return d::TORCHFLOWER;
    }
    state::set_int(BlockId::of(s).default_state(), "age", age)
}

/// `CropBlock.randomTick`.
fn crop_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if light(level, pos) >= 9 {
        let age = state::get_int(s, "age");
        if age < crop_max_age(s) {
            let speed = growth_speed(level, s, pos);
            if level.random().next_int_bounded(growth_bound(speed)) == 0 {
                set_block(level, pos, crop_state_for_age(s, age + 1), flags::CLIENTS);
            }
        }
    }
}

/// `PitcherCropBlock.randomTick` (the lower half; the upper never ticks).
fn pitcher_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let speed = growth_speed(level, s, pos);
    if level.random().next_int_bounded(growth_bound(speed)) == 0 {
        pitcher_grow(level, s, pos, 1);
    }
}

/// `PitcherCropBlock.grow`: from age 3 on the plant is two blocks tall.
fn pitcher_grow<L: Level>(level: &mut L, s: u16, pos: BlockPos, by: i32) {
    let age = (state::get_int(s, "age") + by).min(4);
    let can_grow = state::get_int(s, "age") < 4
        && light(level, pos) >= 8
        && pos.above().y >= level.min_y()
        && pos.above().y < level.min_y() + level.height()
        && (age < 3 || {
            let above = level.block(pos.above());
            is_air(above) || is_named(above, "minecraft:pitcher_crop")
        });
    if !can_grow {
        return;
    }
    let grown = state::set_int(s, "age", age);
    set_block(level, pos, grown, flags::CLIENTS);
    if age >= 3 {
        set_block_and_update(level, pos.above(), state::set(grown, "half", "upper"));
    }
}

// ---------------------------------------------------------------------------------- stems

fn fruit_of(stem: u16) -> (&'static str, &'static str, &'static str) {
    if is_named(stem, "minecraft:pumpkin_stem") {
        ("minecraft:pumpkin", "minecraft:attached_pumpkin_stem", "minecraft:supports_pumpkin_stem_fruit")
    } else {
        ("minecraft:melon", "minecraft:attached_melon_stem", "minecraft:supports_melon_stem_fruit")
    }
}

/// The fruit block an attached stem grew (`AttachedStemBlock.fruit`).
fn attached_fruit(s: u16) -> &'static str {
    if is_named(s, "minecraft:attached_pumpkin_stem") { "minecraft:pumpkin" } else { "minecraft:melon" }
}

/// The stem block an attached stem turns back into (`AttachedStemBlock.stem`).
fn attached_stem_of(s: u16) -> &'static str {
    if is_named(s, "minecraft:attached_pumpkin_stem") { "minecraft:pumpkin_stem" } else { "minecraft:melon_stem" }
}

/// `StemBlock.randomTick`: grows to age 7, then puts its fruit on a random side.
fn stem_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if light(level, pos) < 9 {
        return;
    }
    let speed = growth_speed(level, s, pos);
    if level.random().next_int_bounded(growth_bound(speed)) != 0 {
        return;
    }
    let age = state::get_int(s, "age");
    if age < 7 {
        set_block(level, pos, state::set_int(s, "age", age + 1), flags::CLIENTS);
        return;
    }
    let dir = Direction::HORIZONTAL[level.random().next_int_bounded(4) as usize];
    let target = pos.relative(dir);
    let ground = level.block(target.below());
    let (fruit, attached, support) = fruit_of(s);
    if is_air(level.block(target)) && tags::is(ground, support) {
        let (Some(fruit), Some(attached)) = (BlockId::by_name(fruit), BlockId::by_name(attached)) else { return };
        set_block_and_update(level, target, fruit.default_state());
        set_block_and_update(level, pos, state::set_dir(attached.default_state(), "facing", dir));
    }
}

// ---------------------------------------------------------------------------------- others

/// `NetherWartBlock.randomTick`.
fn nether_wart_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let age = state::get_int(s, "age");
    if age < 3 && level.random().next_int_bounded(10) == 0 {
        set_block(level, pos, state::set_int(s, "age", age + 1), flags::CLIENTS);
    }
}

/// `CocoaBlock.randomTick`.
fn cocoa_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if level.random().next_int_bounded(5) == 0 {
        let age = state::get_int(s, "age");
        if age < 2 {
            set_block(level, pos, state::set_int(s, "age", age + 1), flags::CLIENTS);
        }
    }
}

/// `SweetBerryBushBlock.randomTick`.
fn sweet_berry_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let age = state::get_int(s, "age");
    if age < 3 && level.random().next_int_bounded(5) == 0 && light(level, pos.above()) >= 9 {
        let grown = state::set_int(s, "age", age + 1);
        set_block(level, pos, grown, flags::CLIENTS);
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: grown });
    }
}

// ---------------------------------------------------------------------------------- sugar cane

/// `SugarCaneBlock.canSurvive`: on sugar cane, or on `#supports_sugar_cane` with water (or
/// frosted ice) beside the ground.
fn sugar_cane_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    let below = level.block(pos.below());
    if state::same_block(below, d::SUGAR_CANE) {
        return true;
    }
    if tags::is(below, "minecraft:supports_sugar_cane") {
        let ground = pos.below();
        for dir in Direction::HORIZONTAL {
            let n = level.block(ground.relative(dir));
            if water(n) || tags::is(n, "minecraft:supports_sugar_cane_adjacently") {
                return true;
            }
        }
    }
    false
}

/// `SugarCaneBlock.randomTick`: grows up to 3 high; a full-age cane starts the next block.
fn sugar_cane_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if !is_air(level.block(pos.above())) {
        return;
    }
    let mut height = 1;
    while state::same_block(level.block(pos.offset(0, -height, 0)), s) {
        height += 1;
    }
    if height < 3 {
        let age = state::get_int(s, "age");
        if age == 15 {
            set_block_and_update(level, pos.above(), BlockId::of(s).default_state());
            set_block(level, pos, state::set_int(s, "age", 0), flags::NONE);
        } else {
            set_block(level, pos, state::set_int(s, "age", age + 1), flags::NONE);
        }
    }
}

// ---------------------------------------------------------------------------------- cactus

/// `CactusBlock.canSurvive`: nothing solid or lava beside it, on sand (or cactus), no liquid on top.
fn cactus_can_survive<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    for dir in Direction::HORIZONTAL {
        let n = level.block(pos.relative(dir));
        if logic::is_solid(n) || lava(n) {
            return false;
        }
    }
    let below = level.block(pos.below());
    (state::same_block(below, d::CACTUS) || tags::is(below, "minecraft:supports_cactus")) && !block_props::liquid(level.block(pos.above()))
}

/// `CactusBlock.randomTick`: grows up to 3 high, flowers on top of the column at age 8.
fn cactus_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let above = pos.above();
    if !is_air(level.block(above)) {
        return;
    }
    let mut height = 1;
    let age = state::get_int(s, "age");
    while state::same_block(level.block(pos.offset(0, -height, 0)), s) {
        height += 1;
        if height == 3 && age == 15 {
            return;
        }
    }
    if age == 8 && cactus_can_survive(level, above) {
        let chance = if height >= 3 { 0.25 } else { 0.1 };
        if level.random().next_double() <= chance {
            set_block_and_update(level, above, d::CACTUS_FLOWER);
        }
    } else if age == 15 && height < 3 {
        set_block_and_update(level, above, BlockId::of(s).default_state());
        let reset = state::set_int(s, "age", 0);
        set_block(level, pos, reset, flags::NONE);
        // `Level.neighborChanged(newState, above, cactus)`: cacti do nothing on it.
    }
    if age < 15 {
        set_block(level, pos, state::set_int(s, "age", age + 1), flags::NONE);
    }
}

// ---------------------------------------------------------------------------------- bamboo

fn bamboo_height_below<L: Level + ?Sized>(level: &L, pos: BlockPos) -> i32 {
    let mut n = 0;
    while n < 16 && is_named(level.block(pos.offset(0, -(n + 1), 0)), "minecraft:bamboo") {
        n += 1;
    }
    n
}

/// `BambooStalkBlock.randomTick` (stage 0 only).
fn bamboo_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_int(s, "stage") != 0 {
        return;
    }
    if level.random().next_int_bounded(3) == 0 && is_air(level.block(pos.above())) && light(level, pos.above()) >= 9 {
        let height = bamboo_height_below(level, pos) + 1;
        if height < 16 {
            grow_bamboo(level, s, pos, height);
        }
    }
}

/// `BambooStalkBlock.growBamboo`: a new segment on top; the leaves move up the stalk.
fn grow_bamboo<L: Level>(level: &mut L, s: u16, pos: BlockPos, height: i32) {
    let below_pos = pos.below();
    let below = level.block(below_pos);
    let below2_pos = pos.offset(0, -2, 0);
    let below2 = level.block(below2_pos);
    let mut leaves = "none";
    if height >= 1 {
        let below_bamboo = is_named(below, "minecraft:bamboo");
        if !below_bamboo || state::get(below, "leaves") == Some("none") {
            leaves = "small";
        } else if below_bamboo && state::get(below, "leaves") != Some("none") {
            leaves = "large";
            if is_named(below2, "minecraft:bamboo") {
                set_block_and_update(level, below_pos, state::set(below, "leaves", "small"));
                set_block_and_update(level, below2_pos, state::set(below2, "leaves", "none"));
            }
        }
    }
    let age = if state::get_int(s, "age") == 1 || is_named(below2, "minecraft:bamboo") { 1 } else { 0 };
    let stage = if height >= 11 && level.random().next_float() < 0.25 || height == 15 { 1 } else { 0 };
    let new = state::set_int(state::set(state::set_int(d::BAMBOO, "age", age), "leaves", leaves), "stage", stage);
    set_block_and_update(level, pos.above(), new);
}

/// `BambooSaplingBlock.randomTick`: a third of the rolls turn it into a bamboo stalk.
fn bamboo_sapling_random_tick<L: Level>(level: &mut L, pos: BlockPos) {
    if level.random().next_int_bounded(3) == 0 && is_air(level.block(pos.above())) && light(level, pos.above()) >= 9 {
        let small = state::set(d::BAMBOO, "leaves", "small");
        set_block_and_update(level, pos.above(), small);
    }
}
