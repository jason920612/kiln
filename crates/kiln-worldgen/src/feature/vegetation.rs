//! Plant and cave features: block columns, vegetation patches, vines, multiface growth, root systems, dripstone, sculk, block piles, bamboo, huge mushrooms and fungi, coral.

pub mod column;
pub mod jhash;
pub mod multiface;
pub mod patch;
pub mod shape;
pub mod vines;

use crate::Error;
use crate::block_facts::Dir;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;

/// The feature types of this family.
#[derive(Debug)]
pub enum Kind {
    BlockColumn(column::BlockColumn),
    SingleBlockPillar(column::Pillar),
    ProjectedRandomPatchySquare(column::PatchySquare),
    RandomNeighborSpread(column::NeighborSpread),
    VegetationPatch(patch::VegetationPatch),
    WaterloggedVegetationPatch(patch::VegetationPatch),
    Vines,
    MultifaceGrowth(vines::MultifaceGrowth),
}

/// Parses a feature of this family (`ty` without the `minecraft:` prefix); `None` if the
/// type is not one of them.
pub fn parse(ty: &str, json: &Json, f: &mut Features, l: &Loader) -> Option<Result<Kind, Error>> {
    Some(match ty {
        "block_column" => column::BlockColumn::parse(json, l).map(Kind::BlockColumn),
        "single_block_pillar" => column::Pillar::parse(json, f, l).map(Kind::SingleBlockPillar),
        "projected_random_patchy_square" => column::PatchySquare::parse(json, l).map(Kind::ProjectedRandomPatchySquare),
        "random_neighbor_spread" => column::NeighborSpread::parse(json, l).map(Kind::RandomNeighborSpread),
        "vegetation_patch" => patch::VegetationPatch::parse(json, f, l, false).map(Kind::VegetationPatch),
        "waterlogged_vegetation_patch" => patch::VegetationPatch::parse(json, f, l, true).map(Kind::WaterloggedVegetationPatch),
        "vines" => Ok(Kind::Vines),
        "multiface_growth" => vines::MultifaceGrowth::parse(json, l).map(Kind::MultifaceGrowth),
        _ => return None,
    })
}

impl Kind {
    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        match self {
            Kind::BlockColumn(k) => k.place(r, random, p),
            Kind::SingleBlockPillar(k) => k.place(f, r, random, p),
            Kind::ProjectedRandomPatchySquare(k) => k.place(r, random, p),
            Kind::RandomNeighborSpread(k) => k.place(r, random, p),
            Kind::VegetationPatch(k) | Kind::WaterloggedVegetationPatch(k) => k.place(f, r, random, p),
            Kind::Vines => vines::place_vine(r, p),
            Kind::MultifaceGrowth(k) => k.place(r, random, p),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Kind::BlockColumn(_) => "minecraft:block_column",
            Kind::SingleBlockPillar(_) => "minecraft:single_block_pillar",
            Kind::ProjectedRandomPatchySquare(_) => "minecraft:projected_random_patchy_square",
            Kind::RandomNeighborSpread(_) => "minecraft:random_neighbor_spread",
            Kind::VegetationPatch(_) => "minecraft:vegetation_patch",
            Kind::WaterloggedVegetationPatch(_) => "minecraft:waterlogged_vegetation_patch",
            Kind::Vines => "minecraft:vines",
            Kind::MultifaceGrowth(_) => "minecraft:multiface_growth",
        }
    }

    /// Placed features this feature places (for [`Features::is_supported`]).
    pub fn nested(&self) -> Vec<usize> {
        match self {
            Kind::SingleBlockPillar(k) => k.nested(),
            Kind::VegetationPatch(k) | Kind::WaterloggedVegetationPatch(k) => k.nested(),
            _ => Vec::new(),
        }
    }
}

/// A required field.
fn field<'a>(json: &'a Json, key: &str) -> Result<&'a Json, Error> {
    json.get(key).ok_or_else(|| Error::Invalid(format!("feature without {key}")))
}

/// An optional boolean field (default false).
fn flag(json: &Json, key: &str) -> bool {
    json.get(key).and_then(Json::as_bool).unwrap_or(false)
}

/// A required `Direction` field.
fn dir(json: &Json, key: &str) -> Result<Dir, Error> {
    let name = field(json, key)?.as_str().unwrap_or("");
    Dir::by_name(name).ok_or_else(|| Error::Invalid(format!("bad direction {name}")))
}

fn state_provider(json: &Json, key: &str, l: &Loader) -> Result<StateProvider, Error> {
    StateProvider::parse(field(json, key)?, l)
}

/// A required `Holder<PlacedFeature>` field.
fn placed(json: &Json, key: &str, f: &mut Features, l: &Loader) -> Result<usize, Error> {
    f.placed_ref(field(json, key)?, l)
}

/// An optional `Holder<PlacedFeature>` field.
fn placed_opt(json: &Json, key: &str, f: &mut Features, l: &Loader) -> Result<Option<usize>, Error> {
    json.get(key).map(|j| f.placed_ref(j, l)).transpose()
}

/// An optional `BlockPredicate` field defaulting to `alwaysTrue`.
fn predicate_or_true(json: &Json, key: &str, l: &Loader) -> Result<BlockPredicate, Error> {
    json.get(key).map_or(Ok(BlockPredicate::True), |p| BlockPredicate::parse(p, l))
}
