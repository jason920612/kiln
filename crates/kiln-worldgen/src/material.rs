//! Material rules (`worldgen/material_rule`, `worldgen/material_condition`): 26.3's successor
//! of surface rules, which also places ore veins. This module parses the datapack JSON; the
//! surface pass that evaluates them is [`crate::surface`].

use crate::Error;
use crate::blocks::{BlockSpec, parse_block_state};
use crate::function::{Graph, NodeId, field, qualify};
use crate::json::Json;

/// `VerticalAnchor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
    RelativeToSeaLevel(i32),
}

impl Anchor {
    pub fn parse(json: &Json) -> Result<Anchor, Error> {
        let int = |k: &str| json.get(k).map(|v| v.as_i32().ok_or_else(|| Error::Invalid(format!("bad anchor {k}"))));
        if let Some(v) = int("absolute") {
            return Ok(Anchor::Absolute(v?));
        }
        if let Some(v) = int("above_bottom") {
            return Ok(Anchor::AboveBottom(v?));
        }
        if let Some(v) = int("below_top") {
            return Ok(Anchor::BelowTop(v?));
        }
        if let Some(v) = int("relative_to_sea_level") {
            return Ok(Anchor::RelativeToSeaLevel(v?));
        }
        Err(Error::Invalid(format!("bad vertical anchor {json:?}")))
    }

    /// `VerticalAnchor.resolveY` against a `WorldGenerationContext`.
    pub fn resolve(self, min_y: i32, height: i32, sea_level: i32) -> i32 {
        match self {
            Anchor::Absolute(y) => y,
            Anchor::AboveBottom(o) => min_y + o,
            Anchor::BelowTop(o) => min_y + height - 1 - o,
            Anchor::RelativeToSeaLevel(o) => sea_level + o,
        }
    }
}

/// A rule or condition either inline or named in its registry.
#[derive(Clone, Debug)]
pub enum Ref<T> {
    Inline(Box<T>),
    Named(String),
}

#[derive(Clone, Debug)]
pub enum RuleDef {
    Block(BlockSpec),
    Bandlands,
    Sequence(Vec<Ref<RuleDef>>),
    Condition { if_true: Ref<CondDef>, then_run: Ref<RuleDef> },
    OreVein {
        ore_block: BlockSpec,
        raw_ore_block: BlockSpec,
        filler_block: BlockSpec,
        raw_ore_chance: f32,
        density: NodeId,
        richness: NodeId,
        filler_gap: NodeId,
    },
}

#[derive(Clone, Debug)]
pub enum CondDef {
    Biome(Vec<String>),
    NoiseThreshold { noise: String, min: f64, max: f64, is_3d: bool },
    VerticalGradient { random_name: String, true_at_and_below: Anchor, false_at_and_above: Anchor },
    YAbove { anchor: Anchor, surface_depth_multiplier: i32, add_stone_depth: bool },
    Water { offset: i32, surface_depth_multiplier: i32, add_stone_depth: bool },
    Temperature,
    Steep,
    Not(Ref<CondDef>),
    Hole,
    AbovePreliminarySurface,
    StoneDepth { offset: i32, add_surface_depth: bool, secondary_depth_range: i32, ceiling: bool },
}

fn type_name(json: &Json) -> Result<String, Error> {
    let t = json.get("type").and_then(Json::as_str).ok_or_else(|| Error::Invalid("missing type".into()))?;
    let t = qualify(t);
    Ok(t.strip_prefix("minecraft:").map(str::to_string).unwrap_or(t))
}

fn bool_field(json: &Json, key: &str, default: bool) -> Result<bool, Error> {
    match json.get(key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| Error::Invalid(format!("{key} must be a boolean"))),
    }
}

fn int_field(json: &Json, key: &str) -> Result<i32, Error> {
    field(json, key)?.as_i32().ok_or_else(|| Error::Invalid(format!("{key} must be an integer")))
}

fn str_field(json: &Json, key: &str) -> Result<String, Error> {
    field(json, key)?.as_str().map(qualify).ok_or_else(|| Error::Invalid(format!("{key} must be a string")))
}

/// `MaterialRule.CODEC`: a registry reference or a typed rule.
pub fn parse_rule(graph: &mut Graph, json: &Json) -> Result<Ref<RuleDef>, Error> {
    if let Some(name) = json.as_str() {
        return Ok(Ref::Named(qualify(name)));
    }
    let rule = match type_name(json)?.as_str() {
        "block" => RuleDef::Block(parse_block_state(field(json, "result_state")?)?),
        "bandlands" => RuleDef::Bandlands,
        "sequence" => {
            let list = field(json, "sequence")?
                .as_array()
                .ok_or_else(|| Error::Invalid("sequence must be a list".into()))?
                .iter()
                .map(|r| parse_rule(graph, r))
                .collect::<Result<Vec<_>, _>>()?;
            if list.is_empty() {
                return Err(Error::Invalid("Need at least 1 rule for a sequence".into()));
            }
            RuleDef::Sequence(list)
        }
        "condition" => RuleDef::Condition {
            if_true: parse_condition(field(json, "if_true")?)?,
            then_run: parse_rule(graph, field(json, "then_run")?)?,
        },
        "ore_vein" => RuleDef::OreVein {
            ore_block: parse_block_state(field(json, "ore_block")?)?,
            raw_ore_block: parse_block_state(field(json, "raw_ore_block")?)?,
            filler_block: parse_block_state(field(json, "filler_block")?)?,
            raw_ore_chance: field(json, "raw_ore_chance")?
                .as_f32()
                .filter(|c| (0.0..=1.0).contains(c))
                .ok_or_else(|| Error::Invalid("bad raw_ore_chance".into()))?,
            density: graph.parse(field(json, "density")?)?,
            richness: graph.parse(field(json, "richness")?)?,
            filler_gap: graph.parse(field(json, "filler_gap")?)?,
        },
        other => return Err(Error::Invalid(format!("unsupported material rule type {other}"))),
    };
    Ok(Ref::Inline(Box::new(rule)))
}

/// `MaterialCondition.CODEC`.
pub fn parse_condition(json: &Json) -> Result<Ref<CondDef>, Error> {
    if let Some(name) = json.as_str() {
        return Ok(Ref::Named(qualify(name)));
    }
    let cond = match type_name(json)?.as_str() {
        "biome" => {
            let v = field(json, "biome_is")?;
            let ids = match v {
                Json::String(s) if s.starts_with('#') => {
                    return Err(Error::Invalid(format!("biome tags are not supported: {s}")));
                }
                Json::String(s) => vec![qualify(s)],
                Json::Array(a) => a
                    .iter()
                    .map(|b| b.as_str().map(qualify).ok_or_else(|| Error::Invalid("bad biome id".into())))
                    .collect::<Result<_, _>>()?,
                _ => return Err(Error::Invalid("bad biome_is".into())),
            };
            CondDef::Biome(ids)
        }
        "noise_threshold" => CondDef::NoiseThreshold {
            noise: str_field(json, "noise")?,
            min: field(json, "min_threshold")?.as_f64().ok_or_else(|| Error::Invalid("bad min_threshold".into()))?,
            max: match json.get("max_threshold") {
                Some(v) => v.as_f64().ok_or_else(|| Error::Invalid("bad max_threshold".into()))?,
                None => f64::MAX,
            },
            is_3d: bool_field(json, "is_3d", false)?,
        },
        "vertical_gradient" => CondDef::VerticalGradient {
            random_name: str_field(json, "random_name")?,
            true_at_and_below: Anchor::parse(field(json, "true_at_and_below")?)?,
            false_at_and_above: Anchor::parse(field(json, "false_at_and_above")?)?,
        },
        "y_above" => CondDef::YAbove {
            anchor: Anchor::parse(field(json, "anchor")?)?,
            surface_depth_multiplier: int_field(json, "surface_depth_multiplier")?,
            add_stone_depth: bool_field(json, "add_stone_depth", false)?,
        },
        "water" => CondDef::Water {
            offset: int_field(json, "offset")?,
            surface_depth_multiplier: int_field(json, "surface_depth_multiplier")?,
            add_stone_depth: bool_field(json, "add_stone_depth", false)?,
        },
        "temperature" => CondDef::Temperature,
        "steep" => CondDef::Steep,
        "not" => CondDef::Not(parse_condition(field(json, "invert")?)?),
        "hole" => CondDef::Hole,
        "above_preliminary_surface" => CondDef::AbovePreliminarySurface,
        "stone_depth" => CondDef::StoneDepth {
            offset: int_field(json, "offset")?,
            add_surface_depth: bool_field(json, "add_surface_depth", false)?,
            secondary_depth_range: int_field(json, "secondary_depth_range")?,
            ceiling: match field(json, "surface_type")?.as_str() {
                Some("ceiling") => true,
                Some("floor") => false,
                _ => return Err(Error::Invalid("bad surface_type".into())),
            },
        },
        other => return Err(Error::Invalid(format!("unsupported material condition type {other}"))),
    };
    Ok(Ref::Inline(Box::new(cond)))
}
