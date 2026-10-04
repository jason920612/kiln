//! Growing trees and the plants that grow with bone meal: saplings and mangrove propagules
//! (`SaplingBlock`, `MangrovePropaguleBlock`, `TreeGrower`), azaleas (`AzaleaBlock`), brown and
//! red mushrooms into huge ones (`MushroomBlock.growMushroom`), grass (`GrassBlock`: short
//! grass, tall grass and the biome's flowers) and short grass into tall grass (`TallGrassBlock`).
//!
//! The features themselves (the tree, the huge mushroom, the flower patches) are worldgen's:
//! the level's [`crate::feature_host::FeatureHost`] places them with the level random and the
//! level replays the result through `set_block`, so nothing here knows what a tree looks like,
//! only which feature to ask for and how the saplings are taken out and put back.
//!
//! Random draws are vanilla's, in vanilla's order (bone meal's validity check draws too:
//! `TreeGrower.canGrow` rolls the feature lists).

use crate::feature_host::{self, FeatureRef};
use crate::fluid;
use crate::level::{Level, flags};
use crate::pos::BlockPos;
use crate::state;
use kiln_data::block_logic::{self as logic, BlockClass as C, FluidKind};
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

/// `TreeGrower`: the weighted feature lists (`trees`, `megaTrees`, `flowerTrees`) and the
/// shortest tree type (what `getMinimumHeight` measures).
struct Grower {
    trees: &'static [(&'static str, i32)],
    mega: &'static [(&'static str, i32)],
    flower: &'static [(&'static str, i32)],
    shortest: Option<&'static str>,
}

const OAK: Grower = Grower {
    trees: &[("minecraft:oak", 9), ("minecraft:fancy_oak", 1)],
    mega: &[],
    flower: &[("minecraft:oak_bees_005", 9), ("minecraft:fancy_oak_bees_005", 1)],
    shortest: Some("minecraft:oak"),
};
const SPRUCE: Grower = Grower {
    trees: &[("minecraft:spruce", 1)],
    mega: &[("minecraft:mega_spruce", 1), ("minecraft:mega_pine", 1)],
    flower: &[],
    shortest: Some("minecraft:spruce"),
};
const MANGROVE: Grower = Grower {
    trees: &[("minecraft:mangrove", 15), ("minecraft:tall_mangrove", 85)],
    mega: &[],
    flower: &[],
    shortest: Some("minecraft:mangrove"),
};
const AZALEA: Grower = Grower { trees: &[("minecraft:azalea_tree", 1)], mega: &[], flower: &[], shortest: Some("minecraft:azalea_tree") };
const BIRCH: Grower = Grower {
    trees: &[("minecraft:birch", 1)],
    mega: &[],
    flower: &[("minecraft:birch_bees_005", 1)],
    shortest: Some("minecraft:birch"),
};
const JUNGLE: Grower = Grower {
    trees: &[("minecraft:jungle_tree_no_vine", 1)],
    mega: &[("minecraft:mega_jungle_tree", 1)],
    flower: &[],
    shortest: Some("minecraft:jungle_tree_no_vine"),
};
const ACACIA: Grower = Grower { trees: &[("minecraft:acacia", 1)], mega: &[], flower: &[], shortest: Some("minecraft:acacia") };
const CHERRY: Grower = Grower {
    trees: &[("minecraft:cherry", 1)],
    mega: &[],
    flower: &[("minecraft:cherry_bees_005", 1)],
    shortest: Some("minecraft:cherry"),
};
const DARK_OAK: Grower = Grower { trees: &[], mega: &[("minecraft:dark_oak", 1)], flower: &[], shortest: None };
const PALE_OAK: Grower = Grower { trees: &[], mega: &[("minecraft:pale_oak_bonemeal", 1)], flower: &[], shortest: None };
const POPLAR: Grower = Grower {
    trees: &[("minecraft:red_poplar", 1), ("minecraft:orange_poplar", 1), ("minecraft:yellow_poplar", 1)],
    mega: &[],
    flower: &[],
    shortest: Some("minecraft:red_poplar"),
};

/// The `treeGrower` of a sapling-like block.
fn grower(s: u16) -> Option<&'static Grower> {
    Some(match state::BlockId::of(s).name() {
        "minecraft:oak_sapling" => &OAK,
        "minecraft:spruce_sapling" => &SPRUCE,
        "minecraft:mangrove_propagule" => &MANGROVE,
        "minecraft:azalea" | "minecraft:flowering_azalea" => &AZALEA,
        "minecraft:birch_sapling" => &BIRCH,
        "minecraft:jungle_sapling" => &JUNGLE,
        "minecraft:acacia_sapling" => &ACACIA,
        "minecraft:cherry_sapling" => &CHERRY,
        "minecraft:dark_oak_sapling" => &DARK_OAK,
        "minecraft:pale_oak_sapling" => &PALE_OAK,
        "minecraft:poplar_sapling" => &POPLAR,
        _ => return None,
    })
}

/// `WeightedList.getRandom`: no draw from an empty list, else `nextInt(totalWeight)`.
fn pick(random: &mut impl RandomSource, list: &'static [(&'static str, i32)]) -> Option<&'static str> {
    if list.is_empty() {
        return None;
    }
    let mut i = random.next_int_bounded(list.iter().map(|e| e.1).sum());
    for &(name, weight) in list {
        i -= weight;
        if i < 0 {
            return Some(name);
        }
    }
    None
}

/// `TreeGrower.getConfiguredFeature`.
fn configured_feature<L: Level>(level: &mut L, g: &Grower, flowers: bool) -> Option<&'static str> {
    let list = if flowers && !g.flower.is_empty() { g.flower } else { g.trees };
    pick(level.random(), list)
}

/// `TreeGrower.hasFlowers`: a flower within two blocks sideways and one up or down.
fn has_flowers<L: Level>(level: &L, pos: BlockPos) -> bool {
    for dx in -2..=2 {
        for dy in -1..=1 {
            for dz in -2..=2 {
                if crate::tags::is(level.block(pos.offset(dx, dy, dz)), "minecraft:flowers") {
                    return true;
                }
            }
        }
    }
    false
}

/// The 2x2 of saplings `TreeGrower.findTwoByTwoSaplingPos` found: the offset of the feature's
/// origin from the ticking sapling and the four saplings (state and position).
struct TwoByTwo {
    offset_x: i32,
    offset_z: i32,
    ring: [(u16, BlockPos); 4],
}

fn find_two_by_two<L: Level>(level: &L, s: u16, pos: BlockPos) -> Option<TwoByTwo> {
    for x in [0, -1] {
        for z in [0, -1] {
            let ring = [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)].map(|(dx, dz)| {
                let p = pos.offset(dx, 0, dz);
                (level.block(p), p)
            });
            if ring.iter().all(|&(b, _)| state::same_block(b, s)) {
                return Some(TwoByTwo { offset_x: x, offset_z: z, ring });
            }
        }
    }
    None
}

/// `TreeGrower.removeSapling`: the sapling's spot becomes its fluid (or air) without updates.
fn remove_sapling<L: Level>(level: &mut L, pos: BlockPos) {
    let legacy = fluid::legacy_block(logic::fluid(level.block(pos)));
    crate::set_block(level, pos, legacy, 818);
}

/// `TreeGrower.resetSaplings`.
fn reset_saplings<L: Level>(level: &mut L, saplings: &[(u16, BlockPos)]) {
    for &(s, p) in saplings {
        crate::set_block(level, p, s, flags::NONE);
    }
}

/// `TreeGrower.growTree`: a mega tree from a 2x2 of saplings if the grower has one, else the
/// tree (with bees if flowers are near). The saplings are taken out before the feature is
/// placed and put back if it fails.
fn grow_tree<L: Level>(level: &mut L, g: &Grower, pos: BlockPos, s: u16) -> bool {
    if level.feature_host().is_none() {
        return false;
    }
    if let Some(feature) = pick(level.random(), g.mega)
        && let Some(group) = find_two_by_two(level, s, pos)
    {
        for &(_, p) in &group.ring {
            remove_sapling(level, p);
        }
        if feature_host::place(level, FeatureRef::Configured(feature), pos.offset(group.offset_x, 0, group.offset_z)) {
            return true;
        }
        reset_saplings(level, &group.ring);
        return false;
    }
    let flowers = has_flowers(level, pos);
    let Some(feature) = configured_feature(level, g, flowers) else { return false };
    remove_sapling(level, pos);
    if feature_host::place(level, FeatureRef::Configured(feature), pos) {
        return true;
    }
    reset_saplings(level, &[(s, pos)]);
    false
}

/// `SaplingBlock.advanceTree`: the first time only the stage changes; the second time the tree grows.
fn advance_tree<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    if state::get_int(s, "stage") == 0 {
        crate::set_block(level, pos, state::set_int(s, "stage", 1), flags::NONE);
    } else if let Some(g) = grower(s) {
        grow_tree(level, g, pos, s);
    }
}

fn is_propagule(s: u16) -> bool {
    logic::block_class(s) == C::MangrovePropaguleBlock
}

fn hanging_age(s: u16) -> Option<i32> {
    (is_propagule(s) && state::get_bool(s, "hanging")).then(|| state::get_int(s, "age"))
}

/// Whether a block's random tick is one of this module's (saplings and propagules).
pub fn ticks_randomly(s: u16) -> bool {
    logic::is_instance(s, C::SaplingBlock)
}

/// `SaplingBlock.randomTick` (and `MangrovePropaguleBlock.randomTick`): with enough light
/// above, one tick in seven advances the tree; a hanging propagule ripens instead.
pub fn random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if is_propagule(s) {
        match hanging_age(s) {
            None => {
                if level.random().next_int_bounded(7) == 0 {
                    advance_tree(level, pos, s);
                }
            }
            Some(age) if age < 4 => {
                crate::set_block(level, pos, state::set_int(s, "age", age + 1), flags::CLIENTS);
            }
            Some(_) => {}
        }
        return;
    }
    let darken = level.sky_darken();
    if level.raw_brightness(pos.above(), darken) >= 9 && level.random().next_int_bounded(7) == 0 {
        advance_tree(level, pos, s);
    }
}

/// `LevelReader.isInsideBuildHeight`.
fn inside_build_height<L: Level>(level: &L, pos: BlockPos) -> bool {
    pos.y >= level.min_y() && pos.y < level.min_y() + level.height()
}

/// `TreeGrower.getMinimumHeight`: the base height of the shortest tree's trunk (0 if none).
fn minimum_height<L: Level>(level: &L, g: &Grower) -> i32 {
    let (Some(host), Some(name)) = (level.feature_host(), g.shortest) else { return 0 };
    host.tree_base_height(name).unwrap_or(0)
}

/// `TreeGrower.canGrow`: rolls the lists with the level random (those draws count), and a
/// grower with only mega trees needs a 2x2.
fn can_grow<L: Level>(level: &mut L, g: &Grower, pos: BlockPos, s: u16) -> bool {
    let flowers = has_flowers(level, pos);
    let feature = configured_feature(level, g, flowers);
    let mega = pick(level.random(), g.mega);
    if feature.is_none() && mega.is_some() {
        return find_two_by_two(level, s, pos).is_some();
    }
    true
}

/// `DoublePlantBlock.placeAt`: both halves, without waterlogging (grass has none).
fn place_double<L: Level>(level: &mut L, pos: BlockPos, plant: u16, f: u32) {
    crate::set_block(level, pos, state::set(plant, "half", "lower"), f);
    crate::set_block(level, pos.above(), state::set(plant, "half", "upper"), f);
}

/// The block `TallGrassBlock.getGrownBlock` makes of a short plant.
fn grown(s: u16) -> u16 {
    if state::same_block(s, d::FERN) { d::LARGE_FERN } else { d::TALL_GRASS }
}

/// `TallGrassBlock.isValidBonemealTarget`.
fn tall_grass_valid<L: Level>(level: &L, pos: BlockPos, s: u16) -> bool {
    crate::behaviour::can_survive(level, grown(s), pos) && is_air(level.block(pos.above())) && inside_build_height(level, pos.above())
}

/// `GrassBlock.stopBonemealSpread`.
fn stop_spread<L: Level>(level: &L, pos: BlockPos) -> bool {
    !state::same_block(level.block(pos.below()), d::GRASS_BLOCK) || kiln_data::block_props::full_collision(level.block(pos))
}

/// `GrassBlock.placeBonemealEffect` at one end of a walk.
fn grass_effect<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    if state::same_block(s, d::SHORT_GRASS) && level.random().next_float() < 0.1 && tall_grass_valid(level, pos, s) {
        place_double(level, pos, grown(s), flags::CLIENTS);
    }
    if !is_air(s) || !inside_build_height(level, pos) {
        return;
    }
    if level.random().next_float() < 0.125 {
        let features = level.feature_host().zip(level.biome_name(pos)).map(|(h, b)| h.bone_meal_features(&b)).unwrap_or_default();
        if features.is_empty() {
            return;
        }
        let i = level.random().next_int_bounded(features.len() as i32) as usize;
        feature_host::place(level, FeatureRef::Configured(&features[i]), pos);
    } else {
        feature_host::place(level, FeatureRef::Placed("minecraft:grass_bonemeal"), pos);
    }
}

/// `GrassBlock.performBonemeal`: 128 walks from above the block, the longer ones further; each
/// ends on a grass block's top (a walk stops at anything else) where short grass, tall grass or
/// the biome's flowers may grow.
fn grass_bone_meal<L: Level>(level: &mut L, pos: BlockPos) {
    let above = pos.above();
    'walk: for i in 0..128 {
        let mut p = above;
        for _ in 0..i / 16 {
            let r = level.random();
            let dx = r.next_int_bounded(3) - 1;
            let dy = (r.next_int_bounded(3) - 1) * r.next_int_bounded(3) / 2;
            let dz = r.next_int_bounded(3) - 1;
            p = p.offset(dx, dy, dz);
            if stop_spread(level, p) {
                continue 'walk;
            }
        }
        grass_effect(level, p);
    }
}

/// The configured feature of a mushroom block (`MushroomBlock.feature`).
fn mushroom_feature(s: u16) -> &'static str {
    if state::BlockId::of(s).name() == "minecraft:red_mushroom" { "minecraft:huge_red_mushroom" } else { "minecraft:huge_brown_mushroom" }
}

/// `MushroomBlock.growMushroom`.
fn grow_mushroom<L: Level>(level: &mut L, pos: BlockPos, s: u16) -> bool {
    if level.feature_host().is_none() {
        return false;
    }
    crate::remove_block(level, pos, false);
    if feature_host::place(level, FeatureRef::Configured(mushroom_feature(s)), pos) {
        return true;
    }
    crate::set_block_and_update(level, pos, s);
    false
}

/// `BoneMealItem.growCrop` for the blocks of this module: `None` if the block at `pos` is
/// not one of them, else whether it was a valid target (the bone meal is used up even if
/// the growth did not take).
pub fn grow_crop<L: Level>(level: &mut L, pos: BlockPos) -> Option<bool> {
    let s = level.block(pos);
    let class = logic::block_class(s);
    match class {
        C::SaplingBlock => {
            let g = grower(s)?;
            let valid = can_grow(level, g, pos, s) && inside_build_height(level, pos.offset(0, minimum_height(level, g), 0));
            if valid && f64::from(level.random().next_float()) < 0.45 {
                advance_tree(level, pos, s);
            }
            Some(valid)
        }
        C::MangrovePropaguleBlock => {
            // Valid unless hanging and ripe; a hanging one ripens, a planted one grows like a sapling.
            let age = hanging_age(s);
            if age == Some(4) {
                return Some(false);
            }
            match age {
                Some(a) => {
                    crate::set_block(level, pos, state::set_int(s, "age", a + 1), flags::CLIENTS);
                }
                None => {
                    if f64::from(level.random().next_float()) < 0.45 {
                        advance_tree(level, pos, s);
                    }
                }
            }
            Some(true)
        }
        C::AzaleaBlock => {
            let g = &AZALEA;
            let fluid_free = logic::fluid(level.block(pos.above())).kind == FluidKind::Empty;
            let valid = level.feature_host().is_some() && inside_build_height(level, pos.offset(0, minimum_height(level, g) + 2, 0)) && fluid_free;
            if valid && f64::from(level.random().next_float()) < 0.45 {
                grow_tree(level, g, pos, s);
            }
            Some(valid)
        }
        C::MushroomBlock => {
            let valid = level
                .feature_host()
                .and_then(|h| h.huge_mushroom_radius(mushroom_feature(s)))
                .is_some_and(|radius| inside_build_height(level, pos.offset(0, 4 + radius, 0)));
            if valid && f64::from(level.random().next_float()) < 0.4 {
                grow_mushroom(level, pos, s);
            }
            Some(valid)
        }
        C::GrassBlock => {
            let valid = is_air(level.block(pos.above())) && inside_build_height(level, pos.above());
            if valid {
                grass_bone_meal(level, pos);
            }
            Some(valid)
        }
        C::TallGrassBlock => {
            let valid = tall_grass_valid(level, pos, s);
            if valid {
                place_double(level, pos, grown(s), flags::CLIENTS);
            }
            Some(valid)
        }
        _ => None,
    }
}
