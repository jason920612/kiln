//! Worldgen features placed into a live level: what saplings, propagules, huge mushrooms,
//! azaleas and bone meal on grass do (`Feature.place(level, generator, random, pos)` of the
//! level they grow in).
//!
//! The features live in kiln-worldgen, which depends on this crate, so a [`FeatureHost`]
//! (implemented there) is handed to the level instead: the host runs the feature on a copy of
//! the blocks around the origin (read through a closure) with the level's random and returns
//! what vanilla's `ServerLevel` would have been asked to do, in order: every `setBlock` with
//! its flags, every scheduled tick, the block entity fields a feature filled in. [`apply`]
//! replays that on the level through the same [`crate::set_block`] the level uses for
//! everything else, so block behaviour (`onPlace`, neighbour and shape updates) runs exactly
//! as it does for vanilla's direct writes.

use crate::level::Level;
use crate::pos::BlockPos;
use crate::state::BlockId;
use crate::ticks::TickPriority;
use kiln_javamath::random::LegacyRandom;
use kiln_proto::nbt::Tag;

/// A feature to place: a configured feature (`Feature.place`) or a placed one
/// (`PlacedFeature.place`, with its placement modifiers), by registry name.
#[derive(Clone, Copy, Debug)]
pub enum FeatureRef<'a> {
    Configured(&'a str),
    Placed(&'a str),
}

/// One thing a feature asked of the level.
#[derive(Clone, Debug)]
pub enum FeatureOp {
    /// `level.setBlock(pos, state, flags)`.
    Set { pos: BlockPos, state: u16, flags: u32 },
    /// `level.scheduleTick(pos, block, delay)`.
    BlockTick { pos: BlockPos, block: &'static str, delay: i32 },
    /// `level.scheduleTick(pos, fluid, delay)`.
    FluidTick { pos: BlockPos, fluid: &'static str, delay: i32 },
    /// Fields of the block entity at `pos` after the feature (bees in a bee nest).
    BlockEntity { pos: BlockPos, data: Tag },
}

/// The result of a placement.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    /// What `Feature.place` returned.
    pub ok: bool,
    pub ops: Vec<FeatureOp>,
}

/// Worldgen as a level sees it when something grows.
pub trait FeatureHost: Send + Sync {
    /// Places `feature` at `origin` with `random` (the level's random, advanced by what the
    /// feature draws). `read` is the level's `getBlockState`; `biome` the registry name of
    /// the biome at `origin`.
    fn place(
        &self,
        feature: FeatureRef<'_>,
        origin: BlockPos,
        biome: Option<&str>,
        read: &mut dyn FnMut(BlockPos) -> u16,
        random: &mut LegacyRandom,
    ) -> Placed;

    /// `Biome.getGenerationSettings().getBoneMealFeatures()`: the configured features of the
    /// biome tagged `#minecraft:can_spawn_from_bone_meal` (grass bone meal picks one), in order.
    fn bone_meal_features(&self, biome: &str) -> Vec<String>;

    /// `TreeFeature.trunkPlacer().getBaseHeight()` of a configured feature, if it is a tree
    /// (`TreeGrower.getMinimumHeight`).
    fn tree_base_height(&self, feature: &str) -> Option<i32>;

    /// `AbstractHugeMushroomFeature.foliageRadius()` of a configured feature, if it is a huge
    /// mushroom (`MushroomBlock.isValidBonemealTarget`).
    fn huge_mushroom_radius(&self, feature: &str) -> Option<i32>;
}

/// Places a feature into the level with the level's random: `Feature.place(level, generator,
/// level.getRandom(), pos)`. False when the feature fails and when the level has no worldgen.
pub fn place<L: Level>(level: &mut L, feature: FeatureRef<'_>, pos: BlockPos) -> bool {
    let Some(host) = level.feature_host() else { return false };
    let biome = level.biome_name(pos);
    let Some(random) = level.legacy_random() else { return false };
    let mut random = std::mem::replace(random, LegacyRandom::new(0));
    let placed = {
        let reader: &L = level;
        host.place(feature, pos, biome.as_deref(), &mut |p| reader.block(p), &mut random)
    };
    if let Some(slot) = level.legacy_random() {
        *slot = random;
    }
    apply(level, placed.ops);
    placed.ok
}

/// Runs what a feature asked of the level, in order.
pub fn apply<L: Level>(level: &mut L, ops: Vec<FeatureOp>) {
    for op in ops {
        match op {
            FeatureOp::Set { pos, state, flags } => {
                crate::set_block(level, pos, state, flags);
            }
            FeatureOp::BlockTick { pos, block, delay } => {
                if let Some(id) = BlockId::by_name(block) {
                    crate::schedule_block_tick(level, pos, id, delay, TickPriority::Normal);
                }
            }
            FeatureOp::FluidTick { pos, fluid, delay } => {
                if let Some(f) = crate::FluidType::from_name(fluid) {
                    crate::schedule_fluid_tick(level, pos, f, delay);
                }
            }
            FeatureOp::BlockEntity { pos, data } => level.set_block_entity_data(pos, &data),
        }
    }
}
