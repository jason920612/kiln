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
        C::KelpBlock | C::WeepingVinesBlock | C::TwistingVinesBlock | C::CaveVinesBlock | C::KelpPlantBlock | C::WeepingVinesPlantBlock | C::TwistingVinesPlantBlock | C::CaveVinesPlantBlock => {
            growing_plant(level, pos, s)
        }
        C::SmallDripleafBlock => {
            small_dripleaf(level, pos, s);
            Some(true)
        }
        C::BigDripleafBlock => {
            let valid = dripleaf_can_grow_into(level, pos.above());
            if valid {
                big_dripleaf_grow(level, pos, s);
            }
            Some(valid)
        }
        C::BigDripleafStemBlock => {
            let top = connected_top(level, pos, s, 1, "minecraft:big_dripleaf");
            let valid = top.is_some_and(|t| dripleaf_can_grow_into(level, t.above()));
            if let (true, Some(t)) = (valid, top) {
                let above = t.above();
                let facing = state::get(s, "facing").unwrap_or("north").to_owned();
                dripleaf_place(level, t, "minecraft:big_dripleaf_stem", &facing);
                dripleaf_place(level, above, "minecraft:big_dripleaf", &facing);
            }
            Some(valid)
        }
        // A hanging mangrove propagule under the leaves.
        C::MangroveLeavesBlock => {
            let valid = is_air(level.block(pos.below()));
            if valid {
                let propagule = state::set_int(state::set(named("minecraft:mangrove_propagule"), "hanging", "true"), "age", 0);
                crate::update::set_block(level, pos.below(), propagule, flags::CLIENTS);
            }
            Some(valid)
        }
        // Nylium grows the nether's vegetation on top; netherrack beside nylium turns into nylium (both kinds around: either at random).
        // A pale moss carpet on the ground puts another on top of itself: `createTopperWithSideChance` (every side that can hold one takes it).
        C::MossyCarpetBlock => {
            let valid = state::get_bool(s, "bottom") && !is_air(mossy_topper(level, pos));
            if valid {
                let topper = mossy_topper(level, pos);
                crate::update::set_block_and_update(level, pos.above(), topper);
            }
            Some(valid)
        }
        // A fungus on its own nylium grows into a huge fungus four times in ten.
        C::NetherFungusBlock => {
            let (required, feature) = if is_named(s, "minecraft:warped_fungus") {
                ("minecraft:warped_nylium", "minecraft:warped_fungus_planted")
            } else {
                ("minecraft:crimson_nylium", "minecraft:crimson_fungus_planted")
            };
            let valid = is_named(level.block(pos.below()), required) && inside_build_height(level, pos.above());
            if valid && f64::from(level.random().next_float()) < 0.4 {
                feature_host::place(level, FeatureRef::Configured(feature), pos);
            }
            Some(valid)
        }
        C::NyliumBlock => {
            let valid = is_air(level.block(pos.above())) && inside_build_height(level, pos.above());
            if valid {
                feature_host::place(level, FeatureRef::Configured("minecraft:nylium_bonemeal"), pos.above());
            }
            Some(valid)
        }
        C::NetherrackBlock => {
            let nylium = |level: &L| -> (bool, bool) {
                let (mut warped, mut crimson) = (false, false);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let b = level.block(pos.offset(dx, dy, dz));
                            warped |= is_named(b, "minecraft:warped_nylium");
                            crimson |= is_named(b, "minecraft:crimson_nylium");
                        }
                    }
                }
                (warped, crimson)
            };
            let skylight = kiln_data::block_props::propagates_skylight_down(level.block(pos.above()));
            let (warped, crimson) = nylium(level);
            let valid = skylight && (warped || crimson);
            if valid {
                let to = if warped && crimson {
                    if level.random().next_bool() { "minecraft:warped_nylium" } else { "minecraft:crimson_nylium" }
                } else if warped {
                    "minecraft:warped_nylium"
                } else {
                    "minecraft:crimson_nylium"
                };
                crate::update::set_block_and_update(level, pos, named(to));
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

/// `BlockUtil.getTopConnectedBlock`: from `pos` along `dir` (+1 up, -1 down) past the blocks of `s`'s kind to the first other one, if that is the `head` block.
fn connected_top<L: Level>(level: &L, pos: BlockPos, s: u16, dir: i32, head: &str) -> Option<BlockPos> {
    let body = BlockId::of(s);
    let mut p = pos;
    loop {
        p = p.offset(0, dir, 0);
        if BlockId::of(level.block(p)) != body {
            break;
        }
    }
    is_named(level.block(p), head).then_some(p)
}

/// `GrowingPlantHeadBlock` / `GrowingPlantBodyBlock` bone meal: kelp and the vines grow their head along the growth direction (`getBlocksToGrowWhenBonemealed`
/// blocks, the age going up to 25 as it goes); cave vines only ripen their berries.
fn growing_plant<L: Level>(level: &mut L, pos: BlockPos, s: u16) -> Option<bool> {
    let class = logic::block_class(s);
    let (head_name, dir, is_body) = match class {
        C::KelpBlock => ("minecraft:kelp", 1, false),
        C::KelpPlantBlock => ("minecraft:kelp", 1, true),
        C::TwistingVinesBlock => ("minecraft:twisting_vines", 1, false),
        C::TwistingVinesPlantBlock => ("minecraft:twisting_vines", 1, true),
        C::WeepingVinesBlock => ("minecraft:weeping_vines", -1, false),
        C::WeepingVinesPlantBlock => ("minecraft:weeping_vines", -1, true),
        C::CaveVinesBlock => ("minecraft:cave_vines", -1, false),
        _ => ("minecraft:cave_vines", -1, true),
    };
    if head_name == "minecraft:cave_vines" {
        // `CaveVinesBlock` / `CaveVinesPlantBlock`: a bare vine ripens.
        let valid = state::get(s, "berries") != Some("true");
        if valid {
            crate::update::set_block(level, pos, state::set(s, "berries", "true"), flags::CLIENTS);
        }
        return Some(valid);
    }
    let (head_pos, head_state) = if is_body {
        let Some(h) = connected_top(level, pos, s, dir, head_name) else { return Some(false) };
        (h, level.block(h))
    } else {
        (pos, s)
    };
    let can_grow_into = |level: &L, p: BlockPos| {
        let b = level.block(p);
        if head_name == "minecraft:kelp" { is_named(b, "minecraft:water") } else { is_air(b) }
    };
    let next = head_pos.offset(0, dir, 0);
    let valid = can_grow_into(level, next) && inside_build_height(level, next);
    if !valid {
        return Some(false);
    }
    let mut age = (state::get_int(head_state, "age") + 1).min(25);
    let blocks = if head_name == "minecraft:kelp" {
        1
    } else {
        // `NetherVines.getBlocksToGrowWhenBonemealed`.
        let (mut d, mut n) = (1.0f64, 0);
        while level.random().next_double() < d {
            d *= 0.826;
            n += 1;
        }
        n
    };
    let mut at = next;
    for _ in 0..blocks {
        if !can_grow_into(level, at) || !inside_build_height(level, at) {
            break;
        }
        crate::update::set_block_and_update(level, at, state::set_int(head_state, "age", age));
        at = at.offset(0, dir, 0);
        age = (age + 1).min(25);
    }
    Some(true)
}

/// `BigDripleafBlock.canGrowInto`: empty, water or a small dripleaf, inside the build height.
fn dripleaf_can_grow_into<L: Level>(level: &L, pos: BlockPos) -> bool {
    let b = level.block(pos);
    inside_build_height(level, pos) && (is_air(b) || is_named(b, "minecraft:water") || is_named(b, "minecraft:small_dripleaf"))
}

/// `BigDripleafBlock.place` / `BigDripleafStemBlock.place`: the block waterlogged when a water source was there.
fn dripleaf_place<L: Level>(level: &mut L, pos: BlockPos, block: &str, facing: &str) {
    let f = logic::fluid(level.block(pos));
    let waterlogged = f.kind == logic::FluidKind::Water && f.source;
    let mut s = named(block);
    s = state::set(s, "waterlogged", if waterlogged { "true" } else { "false" });
    s = state::set(s, "facing", facing);
    crate::update::set_block_and_update(level, pos, s);
}

/// `BigDripleafBlock.performBonemeal`: the leaf moves up one and a stem takes its place.
fn big_dripleaf_grow<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    let above = pos.above();
    if dripleaf_can_grow_into(level, above) {
        let facing = state::get(s, "facing").unwrap_or("north");
        dripleaf_place(level, pos, "minecraft:big_dripleaf_stem", facing);
        dripleaf_place(level, above, "minecraft:big_dripleaf", facing);
    }
}

/// `SmallDripleafBlock.performBonemeal`: the lower half is replaced by a big dripleaf two to five high.
fn small_dripleaf<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    if state::get(s, "half") == Some("lower") {
        let above = pos.above();
        let legacy = crate::fluid::legacy_block(logic::fluid(level.block(above)));
        crate::update::set_block(level, above, legacy, 18);
        let facing = state::get(s, "facing").unwrap_or("north").to_owned();
        // `BigDripleafBlock.placeWithRandomHeight`.
        let height = 2 + level.random().next_int_bounded(4);
        let mut j = 0;
        let mut p = pos;
        while j < height && dripleaf_can_grow_into(level, p) {
            j += 1;
            p = p.above();
        }
        let top = pos.y + j - 1;
        let mut p = pos;
        while p.y < top {
            dripleaf_place(level, p, "minecraft:big_dripleaf_stem", &facing);
            p = p.above();
        }
        dripleaf_place(level, p, "minecraft:big_dripleaf", &facing);
    } else {
        let below = pos.below();
        let b = level.block(below);
        small_dripleaf(level, below, b);
    }
}

/// `MossyCarpetBlock.createTopperWithSideChance(level, pos, () -> true)`: the carpet that goes on top of the one at `pos` (air when none can).
fn mossy_topper<L: Level>(level: &L, pos: BlockPos) -> u16 {
    let above_pos = pos.above();
    let above = level.block(above_pos);
    let is_carpet = is_named(above, "minecraft:pale_moss_carpet");
    if (is_carpet && state::get_bool(above, "bottom")) || (!is_carpet && !kiln_data::block_props::replaceable(above)) {
        return kiln_data::blocks::default_state::AIR;
    }
    let fresh = state::set_bool(named("minecraft:pale_moss_carpet"), "bottom", false);
    let updated = crate::place_rules::mossy_carpet_state(level, fresh, above_pos, true);
    if crate::place_rules::mossy_carpet_faces(updated) && updated != above { updated } else { kiln_data::blocks::default_state::AIR }
}

/// `BoneMealItem.growWaterPlant`: bone meal on the water beside a solid face: 128 tries to scatter seagrass (and in a warm ocean corals and coral fans, a wall coral
/// on the first try when `face` is a side) in a growing cloud around `pos`, ripening seagrass already there one time in ten. False when `pos` is not still water.
pub fn grow_water_plant<L: Level>(level: &mut L, pos: BlockPos, face: Option<Direction>) -> bool {
    let is_full_water = |level: &L, p: BlockPos| {
        let b = level.block(p);
        let f = logic::fluid(b);
        is_named(b, "minecraft:water") && f.kind == logic::FluidKind::Water && f.amount == 8
    };
    if !is_full_water(level, pos) {
        return false;
    }
    let wall_corals = tags::members("minecraft:wall_corals");
    let underwater = tags::members("minecraft:underwater_bonemeals");
    'tries: for i in 0..128 {
        let mut at = pos;
        let mut plant = named("minecraft:seagrass");
        for _ in 0..i / 16 {
            let dx = level.random().next_int_bounded(3) - 1;
            let dy = (level.random().next_int_bounded(3) - 1) * level.random().next_int_bounded(3) / 2;
            let dz = level.random().next_int_bounded(3) - 1;
            at = at.offset(dx, dy, dz);
            let b = level.block(at);
            if (0..6u8).all(|d| logic::face_sturdy(b, d, logic::Support::Full)) {
                continue 'tries;
            }
        }
        if level.biome_name(at).as_deref() == Some("minecraft:warm_ocean") {
            if i == 0 && face.is_some_and(|f| f.is_horizontal()) {
                if !wall_corals.is_empty() {
                    let pick = level.random().next_int_bounded(wall_corals.len() as i32) as usize;
                    plant = wall_corals[pick].default_state();
                }
                if state::has(plant, "facing") {
                    plant = state::set(plant, "facing", face.map_or("north", |f| f.name()));
                }
            } else if level.random().next_int_bounded(4) == 0 && !underwater.is_empty() {
                let pick = level.random().next_int_bounded(underwater.len() as i32) as usize;
                plant = underwater[pick].default_state();
            }
        }
        if state::has(plant, "facing") && wall_corals.contains(&BlockId::of(plant)) {
            let mut k = 0;
            while !crate::behaviour::support::can_survive(level, plant, at) && k < 4 {
                let dir = Direction::HORIZONTAL[level.random().next_int_bounded(4) as usize];
                plant = state::set(plant, "facing", dir.name());
                k += 1;
            }
        }
        if !crate::behaviour::support::can_survive(level, plant, at) {
            continue;
        }
        let existing = level.block(at);
        if is_full_water(level, at) {
            crate::update::set_block_and_update(level, at, plant);
        } else if is_named(existing, "minecraft:seagrass") && is_named(level.block(at.above()), "minecraft:water") && level.random().next_int_bounded(10) == 0 {
            grow(level, at);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_level::TestLevel;
    use kiln_data::blocks::default_state as d;

    fn level() -> TestLevel {
        // bedrock at -64, dirt at -63: the ground is y = -63, plants stand at y = -62.
        let mut l = TestLevel::flat(-64, 384, &[d::BEDROCK, d::DIRT]);
        l.load_chunks((-1, -1), (1, 1));
        l.set_random_seed(7);
        l
    }

    fn put(l: &mut TestLevel, x: i32, y: i32, z: i32, s: u16) {
        l.set_raw(BlockPos::new(x, y, z), s, 0);
    }

    fn at(l: &TestLevel, x: i32, y: i32, z: i32) -> u16 {
        l.block(BlockPos::new(x, y, z))
    }

    #[test]
    fn bone_meal_grows_a_bamboo_stalk_one_or_two_taller_with_small_leaves_on_the_new_one() {
        let mut l = level();
        put(&mut l, 0, -62, 0, named("minecraft:bamboo"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        let n = (-62..-50).take_while(|&y| is_named(at(&l, 0, y, 0), "minecraft:bamboo")).count();
        assert!((2..=3).contains(&n), "grew by one or two: {n}");
        let top = at(&l, 0, -62 + n as i32 - 1, 0);
        assert_eq!(state::get(top, "leaves"), Some("small"), "the new top has small leaves");
    }

    #[test]
    fn bone_meal_on_a_bamboo_sapling_puts_a_bamboo_on_top_of_it() {
        let mut l = level();
        put(&mut l, 0, -62, 0, named("minecraft:bamboo_sapling"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        assert!(is_named(at(&l, 0, -61, 0), "minecraft:bamboo"));
    }

    #[test]
    fn bone_meal_spreads_a_bush_to_a_free_neighbour_that_can_hold_it() {
        let mut l = level();
        put(&mut l, 0, -62, 0, named("minecraft:bush"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        let n = Direction::HORIZONTAL.iter().filter(|&&dir| is_named(l.block(BlockPos::new(0, -62, 0).relative(dir)), "minecraft:bush")).count();
        assert_eq!(n, 1, "one copy beside it");
        // Boxed in by stone, there is nowhere to go: not a valid target.
        let mut l = level();
        put(&mut l, 0, -62, 0, named("minecraft:bush"));
        for dir in Direction::HORIZONTAL {
            let p = BlockPos::new(0, -62, 0).relative(dir);
            put(&mut l, p.x, p.y, p.z, d::STONE);
        }
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(false));
    }

    #[test]
    fn bone_meal_turns_short_dry_grass_tall_and_rooted_dirt_hangs_roots() {
        let mut l = level();
        put(&mut l, 0, -62, 0, named("minecraft:short_dry_grass"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        assert!(is_named(at(&l, 0, -62, 0), "minecraft:tall_dry_grass"));
        put(&mut l, 5, -62, 5, named("minecraft:rooted_dirt"));
        put(&mut l, 5, -63, 5, d::AIR);
        put(&mut l, 5, -61, 5, d::AIR);
        // (Rooted dirt over air: its roots hang below.)
        put(&mut l, 5, -50, 5, named("minecraft:rooted_dirt"));
        assert_eq!(grow(&mut l, BlockPos::new(5, -50, 5)), Some(true));
        assert!(is_named(at(&l, 5, -51, 5), "minecraft:hanging_roots"));
    }

    #[test]
    fn bone_meal_on_pitcher_crops_ages_them_and_builds_the_upper_half_from_age_three() {
        let mut l = level();
        put(&mut l, 0, -63, 0, named("minecraft:farmland"));
        put(&mut l, 0, -62, 0, state::set(named("minecraft:pitcher_crop"), "half", "lower"));
        for expected in 1..=2 {
            assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
            assert_eq!(state::get_int(at(&l, 0, -62, 0), "age"), expected);
            assert!(is_air(at(&l, 0, -61, 0)), "still one block tall at age {expected}");
        }
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        assert_eq!(state::get_int(at(&l, 0, -62, 0), "age"), 3);
        assert_eq!(state::get(at(&l, 0, -61, 0), "half"), Some("upper"));
        // From the upper half it is the same plant.
        assert_eq!(grow(&mut l, BlockPos::new(0, -61, 0)), Some(true));
        assert_eq!(state::get_int(at(&l, 0, -62, 0), "age"), 4);
        assert_eq!(state::get_int(at(&l, 0, -61, 0), "age"), 4);
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(false), "full grown");
    }

    #[test]
    fn bone_meal_lengthens_vines_and_ages_their_heads_and_ripens_cave_vine_berries() {
        let mut l = level();
        // Weeping vines hanging from a ceiling at y = -50.
        put(&mut l, 0, -50, 0, d::STONE);
        put(&mut l, 0, -51, 0, state::set_int(named("minecraft:weeping_vines"), "age", 3));
        assert_eq!(grow(&mut l, BlockPos::new(0, -51, 0)), Some(true));
        let grown = (1..=30).filter(|&i| BlockId::of(at(&l, 0, -51 - i, 0)).name().starts_with("minecraft:weeping_vines")).count() as i32;
        assert!(grown >= 1, "at least one more block below");
        let head = at(&l, 0, -51 - grown, 0);
        assert!(is_named(head, "minecraft:weeping_vines"), "the lowest is the head");
        assert_eq!(state::get_int(head, "age"), 3 + grown, "each new block is one older");
        // Cave vines ripen.
        put(&mut l, 4, -50, 4, d::STONE);
        put(&mut l, 4, -51, 4, named("minecraft:cave_vines"));
        assert_eq!(grow(&mut l, BlockPos::new(4, -51, 4)), Some(true));
        assert_eq!(state::get(at(&l, 4, -51, 4), "berries"), Some("true"));
        assert_eq!(grow(&mut l, BlockPos::new(4, -51, 4)), Some(false), "already ripe");
    }

    #[test]
    fn bone_meal_turns_a_small_dripleaf_into_a_big_one_on_a_stem_and_mangrove_leaves_hang_a_propagule() {
        let mut l = level();
        let mut small = named("minecraft:small_dripleaf");
        small = state::set(small, "half", "lower");
        put(&mut l, 0, -62, 0, small);
        put(&mut l, 0, -61, 0, state::set(named("minecraft:small_dripleaf"), "half", "upper"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        assert!(is_named(at(&l, 0, -62, 0), "minecraft:big_dripleaf_stem"));
        let top = (-62..-40).find(|&y| is_named(at(&l, 0, y, 0), "minecraft:big_dripleaf")).expect("a leaf on top");
        assert!(top >= -61, "two to five high: {top}");
        put(&mut l, 8, -50, 8, named("minecraft:mangrove_leaves"));
        assert_eq!(grow(&mut l, BlockPos::new(8, -50, 8)), Some(true));
        let p = at(&l, 8, -51, 8);
        assert!(is_named(p, "minecraft:mangrove_propagule") && state::get(p, "hanging") == Some("true"));
    }

    #[test]
    fn bone_meal_on_water_scatters_seagrass_over_the_floor_around_it_but_not_on_dry_land() {
        let mut l = level();
        for x in -6..=6 {
            for z in -6..=6 {
                for y in -62..=-52 {
                    put(&mut l, x, y, z, d::WATER);
                }
            }
        }
        assert!(grow_water_plant(&mut l, BlockPos::new(0, -62, 0), Some(Direction::Up)));
        let mut seagrass = 0;
        for x in -6..=6 {
            for z in -6..=6 {
                for y in -62..=-52 {
                    if BlockId::of(at(&l, x, y, z)).name().ends_with("seagrass") {
                        seagrass += 1;
                    }
                }
            }
        }
        assert!(seagrass >= 5, "a cloud of seagrass: {seagrass}");
        let mut dry = level();
        assert!(!grow_water_plant(&mut dry, BlockPos::new(0, -62, 0), None), "air is not water");
    }

    #[test]
    fn bone_meal_turns_netherrack_beside_nylium_into_nylium() {
        let mut l = level();
        put(&mut l, 0, -62, 0, named("minecraft:netherrack"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(false), "no nylium near");
        put(&mut l, 1, -62, 0, named("minecraft:warped_nylium"));
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        assert!(is_named(at(&l, 0, -62, 0), "minecraft:warped_nylium"));
        let mut covered = level();
        put(&mut covered, 0, -62, 0, named("minecraft:netherrack"));
        put(&mut covered, 1, -62, 0, named("minecraft:crimson_nylium"));
        put(&mut covered, 0, -61, 0, d::STONE);
        assert_eq!(grow(&mut covered, BlockPos::new(0, -62, 0)), Some(false), "stone on top blocks the sky");
    }

    #[test]
    fn bone_meal_puts_a_pale_moss_carpet_on_top_of_one_on_the_ground() {
        let mut l = level();
        let carpet = state::set_bool(named("minecraft:pale_moss_carpet"), "bottom", true);
        // (The carpet below has the side too: an upper carpet only climbs where the lower one does.)
        put(&mut l, 0, -62, 0, state::set(carpet, "east", "low"));
        // A stone wall beside it gives the upper carpet a side to climb.
        put(&mut l, 1, -62, 0, d::STONE);
        put(&mut l, 1, -61, 0, d::STONE);
        assert_eq!(grow(&mut l, BlockPos::new(0, -62, 0)), Some(true));
        let top = at(&l, 0, -61, 0);
        assert!(is_named(top, "minecraft:pale_moss_carpet") && !state::get_bool(top, "bottom"), "a carpet on top: {top}");
        assert_eq!(state::get(top, "east"), Some("low"), "it climbs the stone beside it");
        // With nothing to hold on to, no topper: not a valid target.
        let mut bare = level();
        put(&mut bare, 0, -62, 0, carpet);
        assert_eq!(grow(&mut bare, BlockPos::new(0, -62, 0)), Some(false));
    }
}
