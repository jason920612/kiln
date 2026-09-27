//! `BlockPredicate` (feature placement conditions) and `RuleTest` (block matching for ores and
//! structure processors).

use crate::Error;
use crate::block_facts::{Dir, Support, is_face_sturdy, is_solid};
use crate::blocks::block_state;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::{Anchor, GenContext, float, int};
use crate::region::Region;
use crate::sets::{BiomeSet, BlockSet, FluidSet, Loader};
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

fn ty(json: &Json, key: &str) -> String {
    let t = json.get(key).and_then(Json::as_str).unwrap_or("");
    t.strip_prefix("minecraft:").unwrap_or(t).to_string()
}

fn vec3(json: &Json, key: &str) -> Result<(i32, i32, i32), Error> {
    match json.get(key) {
        None => Ok((0, 0, 0)),
        Some(v) => match v.as_array() {
            Some([x, y, z]) => Ok((
                x.as_i32().ok_or_else(|| Error::Invalid("bad offset".into()))?,
                y.as_i32().ok_or_else(|| Error::Invalid("bad offset".into()))?,
                z.as_i32().ok_or_else(|| Error::Invalid("bad offset".into()))?,
            )),
            _ => Err(Error::Invalid(format!("bad vector {v:?}"))),
        },
    }
}

/// `BlockPredicate`.
#[derive(Clone, Debug)]
pub enum BlockPredicate {
    MatchingBlocks { offset: (i32, i32, i32), blocks: Arc<BlockSet> },
    MatchingFluids { offset: (i32, i32, i32), fluids: FluidSet },
    HasSturdyFace { offset: (i32, i32, i32), direction: Dir },
    Solid { offset: (i32, i32, i32) },
    Replaceable { offset: (i32, i32, i32) },
    WouldSurvive { offset: (i32, i32, i32), state: u16 },
    InsideWorldBounds { offset: (i32, i32, i32) },
    HeightRange { min: Anchor, max: Anchor },
    MatchingBiomes(BiomeSet),
    VolumeMatch { min: (i32, i32, i32), max: (i32, i32, i32), predicate: Box<BlockPredicate> },
    /// No entities exist during generation, so nothing obstructs.
    Unobstructed,
    AllOf(Vec<BlockPredicate>),
    AnyOf(Vec<BlockPredicate>),
    Not(Box<BlockPredicate>),
    True,
}

impl BlockPredicate {
    pub fn parse(json: &Json, l: &Loader) -> Result<BlockPredicate, Error> {
        let offset = || vec3(json, "offset");
        let list = |k: &str| -> Result<Vec<BlockPredicate>, Error> {
            json.get(k)
                .and_then(Json::as_array)
                .ok_or_else(|| Error::Invalid(format!("missing {k}")))?
                .iter()
                .map(|p| BlockPredicate::parse(p, l))
                .collect()
        };
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("block predicate without {k}: {json:?}")));
        Ok(match ty(json, "type").as_str() {
            "matching_blocks" => BlockPredicate::MatchingBlocks { offset: offset()?, blocks: l.blocks(field("blocks")?)? },
            "matching_block_tag" => BlockPredicate::MatchingBlocks {
                offset: offset()?,
                blocks: l.block_tag(field("tag")?.as_str().ok_or_else(|| Error::Invalid("bad tag".into()))?)?,
            },
            "matching_fluids" => BlockPredicate::MatchingFluids { offset: offset()?, fluids: l.fluids(field("fluids")?)? },
            "has_sturdy_face" => BlockPredicate::HasSturdyFace {
                offset: offset()?,
                direction: Dir::by_name(field("direction")?.as_str().unwrap_or(""))
                    .ok_or_else(|| Error::Invalid("bad direction".into()))?,
            },
            "solid" => BlockPredicate::Solid { offset: offset()? },
            "replaceable" => BlockPredicate::Replaceable { offset: offset()? },
            "would_survive" => BlockPredicate::WouldSurvive { offset: offset()?, state: block_state(field("state")?)? },
            "inside_world_bounds" => BlockPredicate::InsideWorldBounds { offset: offset()? },
            "height_range" => BlockPredicate::HeightRange {
                min: Anchor::parse(field("min_inclusive")?)?,
                max: Anchor::parse(field("max_inclusive")?)?,
            },
            "matching_biomes" => BlockPredicate::MatchingBiomes(l.biomes(field("biomes")?)?),
            "volume_match" => BlockPredicate::VolumeMatch {
                min: vec3(json, "min")?,
                max: vec3(json, "max")?,
                predicate: Box::new(BlockPredicate::parse(field("match")?, l)?),
            },
            "unobstructed" => BlockPredicate::Unobstructed,
            "all_of" => BlockPredicate::AllOf(list("predicates")?),
            "any_of" => BlockPredicate::AnyOf(list("predicates")?),
            "not" => BlockPredicate::Not(Box::new(BlockPredicate::parse(field("predicate")?, l)?)),
            "true" => BlockPredicate::True,
            t => return Err(Error::Invalid(format!("unsupported block predicate {t}"))),
        })
    }

    pub fn test(&self, r: &mut Region, p: BlockPos) -> bool {
        let at = |o: (i32, i32, i32)| p.offset(o.0, o.1, o.2);
        match self {
            BlockPredicate::MatchingBlocks { offset, blocks } => blocks.contains(r.get(at(*offset))),
            BlockPredicate::MatchingFluids { offset, fluids } => fluids.contains(r.fluid(at(*offset))),
            BlockPredicate::HasSturdyFace { offset, direction } => is_face_sturdy(r.get(at(*offset)), *direction, Support::Full),
            BlockPredicate::Solid { offset } => is_solid(r.get(at(*offset))),
            BlockPredicate::Replaceable { offset } => kiln_data::block_props::replaceable(r.get(at(*offset))),
            BlockPredicate::WouldSurvive { offset, state } => crate::survive::can_survive(*state, r, at(*offset)),
            BlockPredicate::InsideWorldBounds { offset } => !r.is_outside_build_height(at(*offset).y),
            BlockPredicate::HeightRange { min, max } => {
                let g = GenContext { min_y: r.min_y(), height: r.height() };
                p.y >= min.resolve(g) && p.y <= max.resolve(g)
            }
            BlockPredicate::MatchingBiomes(set) => set.contains(r.biome(p)),
            BlockPredicate::VolumeMatch { min, max, predicate } => {
                for x in min.0..=max.0 {
                    for z in min.2..=max.2 {
                        for y in min.1..=max.1 {
                            if !predicate.test(r, p.offset(x, y, z)) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            BlockPredicate::Unobstructed | BlockPredicate::True => true,
            BlockPredicate::AllOf(ps) => ps.iter().all(|q| q.test(r, p)),
            BlockPredicate::AnyOf(ps) => ps.iter().any(|q| q.test(r, p)),
            BlockPredicate::Not(q) => !q.test(r, p),
        }
    }
}

/// `RuleTest`.
#[derive(Clone, Debug)]
pub enum RuleTest {
    AlwaysTrue,
    BlockMatch(Arc<BlockSet>),
    BlockStateMatch(u16),
    TagMatch(Arc<BlockSet>),
    RandomBlockMatch { blocks: Arc<BlockSet>, probability: f32 },
    RandomBlockStateMatch { state: u16, probability: f32 },
    HeightMatch { min: i32, max: i32 },
    AllOf(Vec<RuleTest>),
    AnyOf(Vec<RuleTest>),
    Not(Box<RuleTest>),
}

impl RuleTest {
    pub fn parse(json: &Json, l: &Loader) -> Result<RuleTest, Error> {
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("rule test without {k}: {json:?}")));
        let rules = || -> Result<Vec<RuleTest>, Error> {
            field("rules")?
                .as_array()
                .ok_or_else(|| Error::Invalid("rules must be a list".into()))?
                .iter()
                .map(|r| RuleTest::parse(r, l))
                .collect()
        };
        Ok(match ty(json, "predicate_type").as_str() {
            "always_true" => RuleTest::AlwaysTrue,
            "block_match" => RuleTest::BlockMatch(l.blocks(field("block")?)?),
            "blockstate_match" => RuleTest::BlockStateMatch(block_state(field("block_state")?)?),
            "tag_match" => RuleTest::TagMatch(l.block_tag(field("tag")?.as_str().unwrap_or(""))?),
            "random_block_match" => {
                RuleTest::RandomBlockMatch { blocks: l.blocks(field("block")?)?, probability: float(json, "probability")? }
            }
            "random_blockstate_match" => RuleTest::RandomBlockStateMatch {
                state: block_state(field("block_state")?)?,
                probability: float(json, "probability")?,
            },
            "height_match" => RuleTest::HeightMatch { min: int(json, "min_inclusive")?, max: int(json, "max_inclusive")? },
            "all_of" => RuleTest::AllOf(rules()?),
            "any_of" => RuleTest::AnyOf(rules()?),
            "not" => RuleTest::Not(Box::new(RuleTest::parse(field("rule")?, l)?)),
            t => return Err(Error::Invalid(format!("unsupported rule test {t}"))),
        })
    }

    pub fn test(&self, state: u16, p: BlockPos, random: &mut impl RandomSource) -> bool {
        match self {
            RuleTest::AlwaysTrue => true,
            RuleTest::BlockMatch(b) | RuleTest::TagMatch(b) => b.contains(state),
            RuleTest::BlockStateMatch(s) => state == *s,
            RuleTest::RandomBlockMatch { blocks, probability } => blocks.contains(state) && random.next_float() < *probability,
            RuleTest::RandomBlockStateMatch { state: s, probability } => state == *s && random.next_float() < *probability,
            RuleTest::HeightMatch { min, max } => *min <= p.y && p.y <= *max,
            RuleTest::AllOf(rs) => rs.iter().all(|r| r.test(state, p, random)),
            RuleTest::AnyOf(rs) => rs.iter().any(|r| r.test(state, p, random)),
            RuleTest::Not(r) => !r.test(state, p, random),
        }
    }
}
