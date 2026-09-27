//! Structure processors (`StructureProcessor`) and processor lists
//! (`worldgen/processor_list`): per-block rewrites applied while a template is placed.

use super::template::{BlockInfo, PlaceSettings};
use crate::Error;
use crate::blocks::{block, block_state, is_block};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::RuleTest;
use crate::proto::Heightmap;
use crate::providers::IntProvider;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use kiln_javamath::math::get_seed;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// `PosRuleTest`.
#[derive(Clone, Debug)]
pub enum PosRuleTest {
    AlwaysTrue,
    Linear { min_chance: f32, max_chance: f32, min_dist: i32, max_dist: i32 },
    AxisAlignedLinear { min_chance: f32, max_chance: f32, min_dist: i32, max_dist: i32, axis: usize },
}

/// `Mth.clampedLerp(float, float, float)`.
fn clamped_lerp(t: f32, a: f32, b: f32) -> f32 {
    if t < 0.0 {
        a
    } else if t > 1.0 {
        b
    } else {
        a + t * (b - a)
    }
}

fn linear_chance(dist: i32, min_chance: f32, max_chance: f32, min_dist: i32, max_dist: i32) -> f32 {
    let t = (dist as f32 - min_dist as f32) / (max_dist as f32 - min_dist as f32);
    clamped_lerp(t, min_chance, max_chance)
}

impl PosRuleTest {
    fn parse(json: Option<&Json>) -> Result<PosRuleTest, Error> {
        let Some(json) = json else { return Ok(PosRuleTest::AlwaysTrue) };
        let f = |k: &str| json.get(k).and_then(Json::as_f32).unwrap_or(0.0);
        let i = |k: &str| json.get(k).and_then(Json::as_i32).unwrap_or(0);
        let ty = json.get("predicate_type").and_then(Json::as_str).unwrap_or("");
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "always_true" => PosRuleTest::AlwaysTrue,
            "linear_pos" => PosRuleTest::Linear {
                min_chance: f("min_chance"),
                max_chance: f("max_chance"),
                min_dist: i("min_dist"),
                max_dist: i("max_dist"),
            },
            "axis_aligned_linear_pos" => PosRuleTest::AxisAlignedLinear {
                min_chance: f("min_chance"),
                max_chance: f("max_chance"),
                min_dist: i("min_dist"),
                max_dist: i("max_dist"),
                axis: match json.get("axis").and_then(Json::as_str).unwrap_or("y") {
                    "x" => 0,
                    "z" => 2,
                    _ => 1,
                },
            },
            t => return Err(Error::Invalid(format!("unsupported position rule test {t}"))),
        })
    }

    /// `PosRuleTest.test(templatePos, worldPos, pivot, random)`.
    fn test(&self, world: BlockPos, pivot: BlockPos, random: &mut impl RandomSource) -> bool {
        match *self {
            PosRuleTest::AlwaysTrue => true,
            PosRuleTest::Linear { min_chance, max_chance, min_dist, max_dist } => {
                let d = world.dist_manhattan(pivot);
                random.next_float() <= linear_chance(d, min_chance, max_chance, min_dist, max_dist)
            }
            PosRuleTest::AxisAlignedLinear { min_chance, max_chance, min_dist, max_dist, axis } => {
                let d = [world.x - pivot.x, world.y - pivot.y, world.z - pivot.z][axis].abs();
                random.next_float() <= linear_chance(d, min_chance, max_chance, min_dist, max_dist)
            }
        }
    }
}

/// `RuleBlockEntityModifier`.
#[derive(Clone, Debug)]
pub enum BlockEntityModifier {
    Passthrough,
    Clear,
    AppendStatic(Vec<(String, Tag)>),
    AppendLoot(String),
}

impl BlockEntityModifier {
    fn apply(&self, random: &mut impl RandomSource, nbt: Option<&Tag>) -> Option<Tag> {
        match self {
            BlockEntityModifier::Passthrough => nbt.cloned(),
            BlockEntityModifier::Clear => None,
            BlockEntityModifier::AppendStatic(fields) => {
                let mut out = match nbt {
                    Some(Tag::Compound(f)) => f.clone(),
                    _ => Vec::new(),
                };
                for (k, v) in fields {
                    put(&mut out, k, v.clone());
                }
                Some(Tag::Compound(out))
            }
            BlockEntityModifier::AppendLoot(table) => {
                let mut out = match nbt {
                    Some(Tag::Compound(f)) => f.clone(),
                    _ => Vec::new(),
                };
                put(&mut out, "LootTable", Tag::String(table.clone()));
                put(&mut out, "LootTableSeed", Tag::Long(random.next_long()));
                Some(Tag::Compound(out))
            }
        }
    }
}

/// `CompoundTag.put`: replaces an existing key in place.
pub fn put(fields: &mut Vec<(String, Tag)>, key: &str, value: Tag) {
    match fields.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value,
        None => fields.push((key.to_string(), value)),
    }
}

/// `ProcessorRule`.
#[derive(Clone, Debug)]
pub struct ProcessorRule {
    input: RuleTest,
    location: RuleTest,
    position: PosRuleTest,
    output: u16,
    modifier: BlockEntityModifier,
}

/// `StructureProcessor`.
#[derive(Clone, Debug)]
pub enum Processor {
    /// `BlockIgnoreProcessor`: drops blocks of the listed blocks (first state ids).
    BlockIgnore(Vec<u16>),
    BlockRot { rottable: Option<Arc<BlockSet>>, integrity: f32 },
    Gravity { heightmap: Heightmap, offset: i32 },
    Rule(Vec<ProcessorRule>),
    JigsawReplacement,
    ProtectedBlocks(Arc<BlockSet>),
    Capped { delegate: Box<Processor>, limit: IntProvider },
    Nop,
}

fn first_state(name: &str) -> u16 {
    block(name).expect("vanilla block").first
}

/// `BlockIgnoreProcessor.STRUCTURE_BLOCK`.
pub static IGNORE_STRUCTURE_BLOCK: LazyLock<Processor> =
    LazyLock::new(|| Processor::BlockIgnore(vec![first_state("minecraft:structure_block")]));
/// `BlockIgnoreProcessor.STRUCTURE_AND_AIR`.
pub static IGNORE_STRUCTURE_AND_AIR: LazyLock<Processor> =
    LazyLock::new(|| Processor::BlockIgnore(vec![first_state("minecraft:air"), first_state("minecraft:structure_block")]));
/// `BlockIgnoreProcessor.AIR`.
pub static IGNORE_AIR: LazyLock<Processor> = LazyLock::new(|| Processor::BlockIgnore(vec![first_state("minecraft:air")]));
/// `JigsawReplacementProcessor.INSTANCE`.
pub static JIGSAW_REPLACEMENT: Processor = Processor::JigsawReplacement;
/// The processor of the `terrain_matching` projection: `GravityProcessor(WORLD_SURFACE_WG, -1)`.
pub static TERRAIN_MATCHING_GRAVITY: Processor = Processor::Gravity { heightmap: Heightmap::WorldSurfaceWg, offset: -1 };

fn heightmap(name: &str) -> Result<Heightmap, Error> {
    Heightmap::parse(name).ok_or_else(|| Error::Invalid(format!("unknown heightmap {name}")))
}

impl Processor {
    pub fn parse(json: &Json, l: &Loader) -> Result<Processor, Error> {
        let ty = json.get("processor_type").and_then(Json::as_str).ok_or_else(|| Error::Invalid("processor without type".into()))?;
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("processor {ty} without {k}")));
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "block_ignore" => Processor::BlockIgnore(
                field("blocks")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("blocks must be a list".into()))?
                    .iter()
                    .map(|b| block_state(b).map(|s| kiln_data::blocks_types::block_of(s).first))
                    .collect::<Result<_, _>>()?,
            ),
            "block_rot" => Processor::BlockRot {
                rottable: json.get("rottable_blocks").map(|b| l.blocks(b)).transpose()?,
                integrity: field("integrity")?.as_f32().ok_or_else(|| Error::Invalid("bad integrity".into()))?,
            },
            "gravity" => Processor::Gravity {
                heightmap: heightmap(json.get("heightmap").and_then(Json::as_str).unwrap_or("WORLD_SURFACE_WG"))?,
                offset: json.get("offset").and_then(Json::as_i32).unwrap_or(0),
            },
            "rule" => Processor::Rule(
                field("rules")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("rules must be a list".into()))?
                    .iter()
                    .map(|r| {
                        let f = |k: &str| r.get(k).ok_or_else(|| Error::Invalid(format!("processor rule without {k}")));
                        Ok(ProcessorRule {
                            input: RuleTest::parse(f("input_predicate")?, l)?,
                            location: RuleTest::parse(f("location_predicate")?, l)?,
                            position: PosRuleTest::parse(r.get("position_predicate"))?,
                            output: block_state(f("output_state")?)?,
                            modifier: match r.get("block_entity_modifier") {
                                None => BlockEntityModifier::Passthrough,
                                Some(m) => parse_modifier(m)?,
                            },
                        })
                    })
                    .collect::<Result<_, Error>>()?,
            ),
            "jigsaw_replacement" => Processor::JigsawReplacement,
            "protected_blocks" => Processor::ProtectedBlocks(l.blocks(field("value")?)?),
            "capped" => Processor::Capped {
                delegate: Box::new(Processor::parse(field("delegate")?, l)?),
                limit: IntProvider::parse(field("limit")?)?,
            },
            "nop" => Processor::Nop,
            t => return Err(Error::Invalid(format!("unsupported structure processor {t}"))),
        })
    }

    /// `evaluatesEntirePieceState`.
    pub fn evaluates_entire_piece(&self) -> bool {
        matches!(self, Processor::Capped { .. })
    }

    /// `processBlock(level, pos, pivot, templatePos, info, settings)`.
    pub fn process(
        &self,
        r: &mut Region,
        pos: BlockPos,
        pivot: BlockPos,
        template_pos: BlockPos,
        info: BlockInfo,
        settings: &mut PlaceSettings,
    ) -> Option<BlockInfo> {
        let _ = (pos, template_pos);
        match self {
            Processor::BlockIgnore(blocks) => {
                let first = kiln_data::blocks_types::block_of(info.state).first;
                (!blocks.contains(&first)).then_some(info)
            }
            Processor::BlockRot { rottable, integrity } => {
                let keep = if rottable.as_ref().is_some_and(|b| !b.contains(info.state)) {
                    true
                } else {
                    settings.random_at(info.pos).next_float() <= *integrity
                };
                keep.then_some(info)
            }
            Processor::Gravity { heightmap, offset } => {
                let h = r.height_at(*heightmap, info.pos.x, info.pos.z) + offset;
                Some(BlockInfo { pos: BlockPos::new(info.pos.x, h + template_pos.y, info.pos.z), ..info })
            }
            Processor::Rule(rules) => {
                let mut random = LegacyRandom::new(get_seed(info.pos.x, info.pos.y, info.pos.z));
                for rule in rules {
                    if rule.input.test(info.state, info.pos, &mut random)
                        && rule.location.test(r.get(info.pos), info.pos, &mut random)
                        && rule.position.test(info.pos, pivot, &mut random)
                    {
                        let nbt = rule.modifier.apply(&mut random, info.nbt.as_deref()).map(Arc::new);
                        return Some(BlockInfo { pos: info.pos, state: rule.output, nbt });
                    }
                }
                Some(info)
            }
            Processor::JigsawReplacement => {
                if !is_block(info.state, "minecraft:jigsaw") {
                    return Some(info);
                }
                let Some(nbt) = &info.nbt else { return Some(info) };
                let text = nbt.get("final_state").and_then(Tag::as_str).unwrap_or("minecraft:air");
                let s = parse_state_string(text)?;
                if is_block(s, "minecraft:structure_void") {
                    return None;
                }
                Some(BlockInfo { pos: info.pos, state: s, nbt: None })
            }
            Processor::ProtectedBlocks(set) => (!set.contains(r.get(info.pos))).then_some(info),
            Processor::Capped { .. } | Processor::Nop => Some(info),
        }
    }

    /// `finalizeProcessing`: whole-piece processing after every block was processed.
    #[allow(clippy::too_many_arguments)]
    pub fn finalize(
        &self,
        r: &mut Region,
        pos: BlockPos,
        pivot: BlockPos,
        originals: &[&BlockInfo],
        mut processed: Vec<BlockInfo>,
        settings: &mut PlaceSettings,
    ) -> Vec<BlockInfo> {
        let Processor::Capped { delegate, limit } = self else { return processed };
        if limit.max_value() == 0 || processed.is_empty() || originals.len() != processed.len() {
            return processed;
        }
        // `createThreadLocalInstance(seed).forkPositional().at(pos)`: a legacy LCG.
        let fork = LegacyRandom::new(r.seed()).next_long();
        let mut random = WorldgenRandom::legacy(get_seed(pos.x, pos.y, pos.z) ^ fork);
        let count = limit.sample(&mut random).min(processed.len() as i32);
        if count < 1 {
            return processed;
        }
        let mut order: Vec<usize> = (0..processed.len()).collect();
        for i in (2..=order.len()).rev() {
            let j = random.next_int_bounded(i as i32) as usize;
            order.swap(i - 1, j);
        }
        let mut changed = 0;
        for i in order {
            if changed >= count {
                break;
            }
            let before = processed[i].clone();
            if let Some(out) = delegate.process(r, pos, pivot, originals[i].pos, before.clone(), settings)
                && out != before
            {
                changed += 1;
                processed[i] = out;
            }
        }
        processed
    }
}

fn parse_modifier(json: &Json) -> Result<BlockEntityModifier, Error> {
    let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
    Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
        "passthrough" => BlockEntityModifier::Passthrough,
        "clear" => BlockEntityModifier::Clear,
        "append_loot" => BlockEntityModifier::AppendLoot(crate::function::qualify(
            json.get("loot_table").and_then(Json::as_str).ok_or_else(|| Error::Invalid("append_loot without loot_table".into()))?,
        )),
        "append_static" => match json.get("data").map(json_to_tag) {
            Some(Tag::Compound(f)) => BlockEntityModifier::AppendStatic(f),
            _ => return Err(Error::Invalid("append_static without data".into())),
        },
        t => return Err(Error::Invalid(format!("unsupported block entity modifier {t}"))),
    })
}

/// JSON as NBT the way `JsonOps` → `NbtOps` converts untyped data (integral numbers become
/// ints, others doubles).
pub fn json_to_tag(json: &Json) -> Tag {
    match json {
        Json::Null => Tag::Compound(Vec::new()),
        Json::Bool(b) => Tag::Byte(*b as i8),
        Json::Number(n) => match n.parse::<i32>() {
            Ok(i) => Tag::Int(i),
            Err(_) => Tag::Double(n.parse().unwrap_or(0.0)),
        },
        Json::String(s) => Tag::String(s.clone()),
        Json::Array(items) => Tag::List(items.iter().map(json_to_tag).collect()),
        Json::Object(fields) => Tag::Compound(fields.iter().map(|(k, v)| (k.clone(), json_to_tag(v))).collect()),
    }
}

/// `BlockStateParser.parseForBlock`: `name[prop=value,...]` (unknown text gives `None`).
pub fn parse_state_string(text: &str) -> Option<u16> {
    let (name, props) = match text.split_once('[') {
        Some((n, rest)) => (n, rest.strip_suffix(']')?),
        None => (text, ""),
    };
    let info = kiln_data::blocks_types::block_by_name(&crate::function::qualify(name.trim()))?;
    let mut s = info.default;
    for kv in props.split(',').filter(|p| !p.trim().is_empty()) {
        let (k, v) = kv.split_once('=')?;
        s = info.with_property(s, k.trim(), v.trim())?;
    }
    Some(s)
}

/// A processor list (`StructureProcessorList`).
pub type ProcessorList = Arc<Vec<Processor>>;

/// Processor lists by id, and the inline ones pool elements define.
pub struct ProcessorLists {
    by_id: HashMap<String, ProcessorList>,
}

impl ProcessorLists {
    pub fn load(l: &Loader) -> Result<ProcessorLists, Error> {
        let empty = Vec::new();
        let mut by_id = HashMap::new();
        for (id, json) in l.pack.registries.get("processor_list").unwrap_or(&empty) {
            by_id.insert(id.clone(), parse_list(json, l).map_err(|e| e.context(id))?);
        }
        Ok(ProcessorLists { by_id })
    }

    /// A `Holder<StructureProcessorList>`: an id, an inline `{"processors": [...]}` or a bare
    /// list.
    pub fn get(&self, json: &Json, l: &Loader) -> Result<ProcessorList, Error> {
        match json {
            Json::String(id) => {
                self.by_id.get(&crate::function::qualify(id)).cloned().ok_or_else(|| Error::Invalid(format!("unknown processor list {id}")))
            }
            _ => parse_list(json, l),
        }
    }
}

fn parse_list(json: &Json, l: &Loader) -> Result<ProcessorList, Error> {
    let items = match json {
        Json::Array(items) => items,
        _ => json.get("processors").and_then(Json::as_array).ok_or_else(|| Error::Invalid("processor list without processors".into()))?,
    };
    Ok(Arc::new(items.iter().map(|p| Processor::parse(p, l)).collect::<Result<_, _>>()?))
}
