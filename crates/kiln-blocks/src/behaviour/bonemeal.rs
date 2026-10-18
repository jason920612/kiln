//! Bone meal on the `BonemealableBlock`s that grow without a tree or a crop table of their own (wp80): moss and pale moss (a worldgen patch on top), bamboo, the
//! little spreaders (bush, firefly bush, dry grass), hanging moss, rooted dirt, seagrass, sea pickles and pitcher plants.
//!
//! [`grow`] answers `BoneMealItem.growCrop` for these blocks: `None` when the block at `pos` is none of them, else whether it was a valid target (the bone meal
//! is used up either way; `isBonemealSuccess` is always true for all of these, so a valid target always grows).

use crate::feature_host::{self, FeatureRef};
use crate::level::{Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

fn inside_build_height<L: Level>(level: &L, pos: BlockPos) -> bool {
    pos.y >= level.min_y() && pos.y < level.min_y() + level.height()
}

fn named(name: &str) -> u16 {
    BlockId::by_name(name).map_or(0, |b| b.default_state())
}

fn is_named(s: u16, name: &str) -> bool {
    BlockId::of(s).name() == name
}

/// `Direction.Plane.HORIZONTAL.shuffledCopy(random)` (`Util.shuffle`).
fn shuffled_horizontal<L: Level>(level: &mut L) -> [Direction; 4] {
    let mut dirs = Direction::HORIZONTAL;
    let mut i = dirs.len();
    while i > 1 {
        let j = level.random().next_int_bounded(i as i32) as usize;
        dirs.swap(i - 1, j);
        i -= 1;
    }
    dirs
}

/// `BonemealableBlock.getSpreadableNeighbourPos`: the first of `dirs` whose neighbour is empty and where `plant` could stand.
fn spreadable_neighbour<L: Level>(level: &L, dirs: &[Direction], pos: BlockPos, plant: u16) -> Option<BlockPos> {
    dirs.iter().map(|&d| pos.relative(d)).find(|&n| is_air(level.block(n)) && crate::behaviour::support::can_survive(level, plant, n))
}

/// A spreader's bone meal: a copy of `plant` on a random free neighbour.
fn spread<L: Level>(level: &mut L, pos: BlockPos, plant: u16) -> bool {
    if spreadable_neighbour(level, &Direction::HORIZONTAL, pos, plant).is_none() {
        return false;
    }
    let dirs = shuffled_horizontal(level);
    if let Some(n) = spreadable_neighbour(level, &dirs, pos, plant) {
        crate::update::set_block_and_update(level, n, plant);
    }
    true
}

/// `PitcherCropBlock.canGrow`.
fn pitcher_can_grow<L: Level>(level: &L, pos: BlockPos, s: u16, new_age: i32) -> bool {
    state::get_int(s, "age") < 4
        && level.raw_brightness(pos, 0) >= 8
        && inside_build_height(level, pos.above())
        && (new_age < 3 || {
            let above = level.block(pos.above());
            is_air(above) || is_named(above, "minecraft:pitcher_crop")
        })
}

/// `PitcherCropBlock.getLowerHalf`.
fn pitcher_lower<L: Level>(level: &L, pos: BlockPos, s: u16) -> Option<(BlockPos, u16)> {
    let lower = |s: u16| is_named(s, "minecraft:pitcher_crop") && state::get(s, "half") == Some("lower");
    if lower(s) {
        return Some((pos, s));
    }
    let below = level.block(pos.below());
    lower(below).then_some((pos.below(), below))
}

/// `BambooStalkBlock.getHeightAboveUpToMax` / `getHeightBelowUpToMax`.
fn bamboo_height<L: Level>(level: &L, pos: BlockPos, up: bool) -> i32 {
    let mut n = 0;
    while n < 16 && is_named(level.block(pos.offset(0, if up { n + 1 } else { -(n + 1) }, 0)), "minecraft:bamboo") {
        n += 1;
    }
    n
}

/// `BambooStalkBlock.growBamboo`: a new stalk above `pos`, with the leaves the stalks under it need.
fn grow_bamboo<L: Level>(level: &mut L, s: u16, pos: BlockPos, height: i32) {
    let below = level.block(pos.below());
    let below2_pos = pos.offset(0, -2, 0);
    let below2 = level.block(below2_pos);
    let mut leaves = "none";
    if height >= 1 {
        if !is_named(below, "minecraft:bamboo") || state::get(below, "leaves") == Some("none") {
            leaves = "small";
        } else if is_named(below, "minecraft:bamboo") && state::get(below, "leaves") != Some("none") {
            leaves = "large";
            if is_named(below2, "minecraft:bamboo") {
                crate::update::set_block_and_update(level, pos.below(), state::set(below, "leaves", "small"));
                crate::update::set_block_and_update(level, below2_pos, state::set(below2, "leaves", "none"));
            }
        }
    }
    let age = i32::from(state::get_int(s, "age") == 1 || is_named(below2, "minecraft:bamboo"));
    let stage = i32::from((height >= 11 && level.random().next_float() < 0.25) || height == 15);
    let mut new = state::set_int(named("minecraft:bamboo"), "age", age);
    new = state::set(new, "leaves", leaves);
    new = state::set_int(new, "stage", stage);
    crate::update::set_block_and_update(level, pos.above(), new);
}

/// `BoneMealItem.growCrop` for the blocks of this module.
pub fn grow<L: Level>(level: &mut L, pos: BlockPos) -> Option<bool> {
    let s = level.block(pos);
    match logic::block_class(s) {
        // Moss and pale moss: a worldgen patch on the block above.
        C::BonemealableFeaturePlacerBlock => {
            let valid = is_air(level.block(pos.above()));
            if valid {
                let feature = if is_named(s, "minecraft:pale_moss_block") { "minecraft:pale_moss_patch_bonemeal" } else { "minecraft:moss_patch_bonemeal" };
                feature_host::place(level, FeatureRef::Configured(feature), pos.above());
            }
            Some(valid)
        }
        C::BambooSaplingBlock => {
            let valid = is_air(level.block(pos.above())) && inside_build_height(level, pos.above());
            if valid {
                let bamboo = state::set(named("minecraft:bamboo"), "leaves", "small");
                crate::update::set_block_and_update(level, pos.above(), bamboo);
            }
            Some(valid)
        }
        C::BambooStalkBlock => {
            let (mut above, below) = (bamboo_height(level, pos, true), bamboo_height(level, pos, false));
            let top = pos.offset(0, above + 1, 0);
            let valid = above + below + 1 < 16
                && state::get_int(level.block(pos.offset(0, above, 0)), "stage") != 1
                && inside_build_height(level, top)
                && is_air(level.block(top));
            if valid {
                let mut total = above + below + 1;
                let count = 1 + level.random().next_int_bounded(2);
                for _ in 0..count {
                    let tip = pos.offset(0, above, 0);
                    let tip_state = level.block(tip);
                    let over = tip.above();
                    if total >= 16 || state::get_int(tip_state, "stage") == 1 || !is_air(level.block(over)) || !inside_build_height(level, over) {
                        break;
                    }
                    grow_bamboo(level, tip_state, tip, total);
                    above += 1;
                    total += 1;
                }
            }
            Some(valid)
        }
        // `BushBlock` / `FireflyBushBlock` spread a copy of themselves; `TallDryGrassBlock` spreads short dry grass; short dry grass becomes tall.
        C::BushBlock | C::FireflyBushBlock => Some(spread(level, pos, BlockId::of(s).default_state())),
        C::TallDryGrassBlock => Some(spread(level, pos, named("minecraft:short_dry_grass"))),
        C::ShortDryGrassBlock => {
            crate::update::set_block_and_update(level, pos, named("minecraft:tall_dry_grass"));
            Some(true)
        }
        // Hanging moss grows one more tip below the last.
        C::HangingMossBlock => {
            let mut tip = pos;
            while BlockId::of(level.block(tip.below())) == BlockId::of(s) {
                tip = tip.below();
            }
            let target = tip.below();
            let valid = is_air(level.block(target)) && inside_build_height(level, target);
            if valid {
                crate::update::set_block_and_update(level, target, state::set(s, "tip", "true"));
            }
            Some(valid)
        }
        // Rooted dirt puts hanging roots under itself.
        C::RootedDirtBlock => {
            let valid = is_air(level.block(pos.below())) && inside_build_height(level, pos.below());
            if valid {
                crate::update::set_block_and_update(level, pos.below(), named("minecraft:hanging_roots"));
            }
            Some(valid)
        }
        // Seagrass grows tall in water.
        C::SeagrassBlock => {
            let valid = is_named(level.block(pos.above()), "minecraft:water");
            if valid {
                let lower = named("minecraft:tall_seagrass");
                let upper = state::set(lower, "half", "upper");
                crate::update::set_block(level, pos, lower, flags::CLIENTS);
                crate::update::set_block(level, pos.above(), upper, flags::CLIENTS);
            }
            Some(valid)
        }
        // A live sea pickle on a coral block spreads over the coral around it and fills up to four.
        C::SeaPickleBlock => {
            let valid = state::get(s, "waterlogged") == Some("true") && tags::is(level.block(pos.below()), "minecraft:coral_blocks");
            if valid {
                sea_pickle_spread(level, pos, s);
            }
            Some(valid)
        }
        C::PitcherCropBlock => {
            let Some((lower_pos, lower)) = pitcher_lower(level, pos, s) else { return Some(false) };
            let valid = pitcher_can_grow(level, lower_pos, lower, state::get_int(lower, "age") + 1);
            if valid {
                pitcher_grow(level, lower_pos, lower, 1);
            }
            Some(valid)
        }
        _ => None,
    }
}

/// `SeaPickleBlock.performBonemeal`: five columns of a widening and narrowing diamond around the pickle, a sixth of the free water over coral becomes pickles.
fn sea_pickle_spread<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let (mut width, mut shift, mut step) = (1, 0, 0);
    for column in 0..5 {
        for k in 0..width {
            let top = 2 + pos.y - 1;
            for y in (top - 2)..top {
                let p = BlockPos::new(pos.x - 2 + column, y, pos.z - shift + k);
                if p == pos || level.random().next_int_bounded(6) != 0 {
                    continue;
                }
                if is_named(level.block(p), "minecraft:water") && tags::is(level.block(p.below()), "minecraft:coral_blocks") {
                    let pickles = level.random().next_int_bounded(4) + 1;
                    crate::update::set_block_and_update(level, p, state::set_int(named("minecraft:sea_pickle"), "pickles", pickles));
                }
            }
        }
        if step < 2 {
            width += 2;
            shift += 1;
        } else {
            width -= 2;
            shift -= 1;
        }
        step += 1;
    }
    crate::update::set_block(level, pos, state::set_int(s, "pickles", 4), flags::CLIENTS);
}

/// `PitcherCropBlock.grow`: the age goes up (to 4 at most) and from age 3 the plant is two blocks tall.
fn pitcher_grow<L: Level>(level: &mut L, pos: BlockPos, s: u16, by: i32) {
    let age = (state::get_int(s, "age") + by).min(4);
    if !pitcher_can_grow(level, pos, s, age) {
        return;
    }
    let grown = state::set_int(s, "age", age);
    crate::update::set_block(level, pos, grown, flags::CLIENTS);
    if age >= 3 {
        crate::update::set_block_and_update(level, pos.above(), state::set(grown, "half", "upper"));
    }
}
