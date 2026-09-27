//! Plant and cave features: block columns, vegetation patches, vines, multiface growth, root systems, dripstone, sculk, block piles, bamboo, huge mushrooms and fungi, coral.

pub mod column;
pub mod coral;
pub mod dripstone;
pub mod fungus;
pub mod jhash;
pub mod multiface;
pub mod mushroom;
pub mod nether;
pub mod patch;
pub mod pile;
pub mod roots;
pub mod sculk;
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
    BlockColumn(Box<column::BlockColumn>),
    SingleBlockPillar(Box<column::Pillar>),
    ProjectedRandomPatchySquare(Box<column::PatchySquare>),
    RandomNeighborSpread(Box<column::NeighborSpread>),
    VegetationPatch(Box<patch::VegetationPatch>),
    WaterloggedVegetationPatch(Box<patch::VegetationPatch>),
    Vines,
    MultifaceGrowth(Box<vines::MultifaceGrowth>),
    RootSystem(Box<roots::RootSystem>),
    Speleothem(Box<dripstone::Speleothem>),
    SpeleothemCluster(Box<dripstone::Cluster>),
    LargeDripstone(Box<dripstone::Large>),
    SculkPatch(Box<sculk::SculkPatch>),
    Bamboo(Box<pile::Bamboo>),
    BlockPile(Box<pile::BlockPile>),
    HugeBrownMushroom(Box<mushroom::HugeMushroom>),
    HugeRedMushroom(Box<mushroom::HugeMushroom>),
    CoralTree(Box<coral::Coral>),
    CoralClaw(Box<coral::Coral>),
    HugeFungus(Box<fungus::HugeFungus>),
    SteppedColumnCluster(Box<nether::SteppedColumns>),
    ChorusPlant,
}

/// Parses a feature of this family (`ty` without the `minecraft:` prefix); `None` if the
/// type is not one of them.
pub fn parse(ty: &str, json: &Json, f: &mut Features, l: &Loader) -> Option<Result<Kind, Error>> {
    Some(match ty {
        "block_column" => column::BlockColumn::parse(json, l).map(|k| Kind::BlockColumn(Box::new(k))),
        "single_block_pillar" => column::Pillar::parse(json, f, l).map(|k| Kind::SingleBlockPillar(Box::new(k))),
        "projected_random_patchy_square" => column::PatchySquare::parse(json, l).map(|k| Kind::ProjectedRandomPatchySquare(Box::new(k))),
        "random_neighbor_spread" => column::NeighborSpread::parse(json, l).map(|k| Kind::RandomNeighborSpread(Box::new(k))),
        "vegetation_patch" => patch::VegetationPatch::parse(json, f, l, false).map(|k| Kind::VegetationPatch(Box::new(k))),
        "waterlogged_vegetation_patch" => patch::VegetationPatch::parse(json, f, l, true).map(|k| Kind::WaterloggedVegetationPatch(Box::new(k))),
        "vines" => Ok(Kind::Vines),
        "multiface_growth" => vines::MultifaceGrowth::parse(json, l).map(|k| Kind::MultifaceGrowth(Box::new(k))),
        "root_system" => roots::RootSystem::parse(json, f, l).map(|k| Kind::RootSystem(Box::new(k))),
        "speleothem" => dripstone::Speleothem::parse(json, l).map(|k| Kind::Speleothem(Box::new(k))),
        "speleothem_cluster" => dripstone::Cluster::parse(json, l).map(|k| Kind::SpeleothemCluster(Box::new(k))),
        "large_dripstone" => dripstone::Large::parse(json, l).map(|k| Kind::LargeDripstone(Box::new(k))),
        "sculk_patch" => sculk::SculkPatch::parse(json).map(|k| Kind::SculkPatch(Box::new(k))),
        "bamboo" => pile::Bamboo::parse(json).map(|k| Kind::Bamboo(Box::new(k))),
        "block_pile" => pile::BlockPile::parse(json, l).map(|k| Kind::BlockPile(Box::new(k))),
        "huge_brown_mushroom" => mushroom::HugeMushroom::parse(json, l, false).map(|k| Kind::HugeBrownMushroom(Box::new(k))),
        "huge_red_mushroom" => mushroom::HugeMushroom::parse(json, l, true).map(|k| Kind::HugeRedMushroom(Box::new(k))),
        "coral_tree" => coral::Coral::parse(json, f, l, false).map(|k| Kind::CoralTree(Box::new(k))),
        "coral_claw" => coral::Coral::parse(json, f, l, true).map(|k| Kind::CoralClaw(Box::new(k))),
        "huge_fungus" => fungus::HugeFungus::parse(json, l).map(|k| Kind::HugeFungus(Box::new(k))),
        "stepped_column_cluster" => nether::SteppedColumns::parse(json, l).map(|k| Kind::SteppedColumnCluster(Box::new(k))),
        "chorus_plant" => Ok(Kind::ChorusPlant),
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
            Kind::RootSystem(k) => k.place(f, r, random, p),
            Kind::Speleothem(k) => k.place(r, random, p),
            Kind::SpeleothemCluster(k) => k.place(r, random, p),
            Kind::LargeDripstone(k) => k.place(r, random, p),
            Kind::SculkPatch(k) => k.place(r, random, p),
            Kind::Bamboo(k) => k.place(r, random, p),
            Kind::BlockPile(k) => k.place(r, random, p),
            Kind::HugeBrownMushroom(k) | Kind::HugeRedMushroom(k) => k.place(r, random, p),
            Kind::CoralTree(k) | Kind::CoralClaw(k) => k.place(f, r, random, p),
            Kind::HugeFungus(k) => k.place(r, random, p),
            Kind::SteppedColumnCluster(k) => k.place(r, random, p),
            Kind::ChorusPlant => nether::place_chorus(r, random, p),
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
            Kind::RootSystem(_) => "minecraft:root_system",
            Kind::Speleothem(_) => "minecraft:speleothem",
            Kind::SpeleothemCluster(_) => "minecraft:speleothem_cluster",
            Kind::LargeDripstone(_) => "minecraft:large_dripstone",
            Kind::SculkPatch(_) => "minecraft:sculk_patch",
            Kind::Bamboo(_) => "minecraft:bamboo",
            Kind::BlockPile(_) => "minecraft:block_pile",
            Kind::HugeBrownMushroom(_) => "minecraft:huge_brown_mushroom",
            Kind::HugeRedMushroom(_) => "minecraft:huge_red_mushroom",
            Kind::CoralTree(_) => "minecraft:coral_tree",
            Kind::CoralClaw(_) => "minecraft:coral_claw",
            Kind::HugeFungus(_) => "minecraft:huge_fungus",
            Kind::SteppedColumnCluster(_) => "minecraft:stepped_column_cluster",
            Kind::ChorusPlant => "minecraft:chorus_plant",
        }
    }

    /// Placed features this feature places (for [`Features::is_supported`]).
    pub fn nested(&self) -> Vec<usize> {
        match self {
            Kind::SingleBlockPillar(k) => k.nested(),
            Kind::RootSystem(k) => k.nested(),
            Kind::CoralTree(k) | Kind::CoralClaw(k) => k.nested(),
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
