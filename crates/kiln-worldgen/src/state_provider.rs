//! `BlockStateProvider`: how features choose the block states they place.

use crate::Error;
use crate::blocks::{block_state, has_prop, prop, with_prop};
use crate::json::Json;
use crate::noise::NoiseStack;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::providers::{IntProvider, Weighted, float, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use kiln_data::blocks_types::{block_by_name, block_of};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::sync::Arc;

/// `BlockStateProvider`.
#[derive(Clone, Debug)]
pub enum StateProvider {
    Simple(u16),
    Weighted(Weighted<u16>),
    Rotated { source: Box<StateProvider>, direction: Option<crate::block_facts::Dir> },
    RandomizedInt { source: Box<StateProvider>, property: String, values: IntProvider },
    /// `RandomBlockProvider`: a uniformly chosen block (default state) of an ordered list.
    RandomBlock(Vec<u16>),
    RuleBased { fallback: Option<Box<StateProvider>>, rules: Vec<(BlockPredicate, StateProvider)> },
    CopyProperties(Box<StateProvider>),
    Noise { noise: Arc<NoiseStack>, scale: f32, states: Vec<u16> },
    NoiseThreshold {
        noise: Arc<NoiseStack>,
        scale: f32,
        threshold: f32,
        high_chance: f32,
        default: u16,
        low: Vec<u16>,
        high: Vec<u16>,
    },
    DualNoise {
        noise: Arc<NoiseStack>,
        scale: f32,
        states: Vec<u16>,
        variety: (i32, i32),
        slow: Arc<NoiseStack>,
        slow_scale: f32,
    },
}

fn states(json: Option<&Json>) -> Result<Vec<u16>, Error> {
    json.and_then(Json::as_array)
        .ok_or_else(|| Error::Invalid("missing state list".into()))?
        .iter()
        .map(block_state)
        .collect()
}

fn noise(json: &Json, key: &str, seed: i64) -> Result<Arc<NoiseStack>, Error> {
    let params = crate::datapack::parse_noise(json.get(key).ok_or_else(|| Error::Invalid(format!("missing {key}")))?)?;
    Ok(Arc::new(params.create(&mut LegacyRandom::new(seed))))
}

impl StateProvider {
    pub fn parse(json: &Json, l: &Loader) -> Result<StateProvider, Error> {
        if let Some(id) = json.as_str() {
            if let Some(reg) = l.pack.state_providers.get(&crate::function::qualify(id)) {
                return StateProvider::parse(reg, l).map_err(|e| e.context(id));
            }
            return Ok(StateProvider::Simple(block_state(json)?));
        }
        let Some(ty) = json.get("type").and_then(Json::as_str) else {
            return Ok(StateProvider::Simple(block_state(json)?));
        };
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("state provider without {k}")));
        let seed = || json.get("seed").and_then(|s| match s {
            Json::Number(n) => n.parse::<i64>().ok(),
            _ => None,
        });
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "simple" => StateProvider::Simple(block_state(field("state")?)?),
            "weighted" => StateProvider::Weighted(Weighted::parse(field("entries")?, block_state)?),
            "rotated" => StateProvider::Rotated {
                source: Box::new(StateProvider::parse(field("state")?, l)?),
                direction: json.get("direction").and_then(Json::as_str).and_then(crate::block_facts::Dir::by_name),
            },
            "randomized_int" => StateProvider::RandomizedInt {
                source: Box::new(StateProvider::parse(field("source")?, l)?),
                property: field("property")?.as_str().unwrap_or("").to_string(),
                values: IntProvider::parse(field("values")?)?,
            },
            "random_block" => {
                let names: Vec<String> = match field("blocks")? {
                    Json::String(s) if s.starts_with('#') => l.pack.block_tag_ordered(s)?,
                    Json::String(s) => vec![s.clone()],
                    Json::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                    j => return Err(Error::Invalid(format!("bad blocks {j:?}"))),
                };
                StateProvider::RandomBlock(
                    names
                        .iter()
                        .map(|n| {
                            block_by_name(&crate::function::qualify(n))
                                .map(|b| b.default)
                                .ok_or_else(|| Error::Invalid(format!("unknown block {n}")))
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            "rule_based" => StateProvider::RuleBased {
                fallback: match json.get("fallback") {
                    Some(f) => Some(Box::new(StateProvider::parse(f, l)?)),
                    None => None,
                },
                rules: field("rules")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("rules must be a list".into()))?
                    .iter()
                    .map(|r| {
                        let get = |k: &str| r.get(k).ok_or_else(|| Error::Invalid(format!("rule without {k}")));
                        Ok((BlockPredicate::parse(get("if_true")?, l)?, StateProvider::parse(get("then")?, l)?))
                    })
                    .collect::<Result<_, Error>>()?,
            },
            "copy_properties" => StateProvider::CopyProperties(Box::new(StateProvider::parse(field("source")?, l)?)),
            "noise" => {
                let s = seed().ok_or_else(|| Error::Invalid("bad seed".into()))?;
                StateProvider::Noise { noise: noise(json, "noise", s)?, scale: float(json, "scale")?, states: states(json.get("states"))? }
            }
            "noise_threshold" => {
                let s = seed().ok_or_else(|| Error::Invalid("bad seed".into()))?;
                StateProvider::NoiseThreshold {
                    noise: noise(json, "noise", s)?,
                    scale: float(json, "scale")?,
                    threshold: float(json, "threshold")?,
                    high_chance: float(json, "high_chance")?,
                    default: block_state(field("default_state")?)?,
                    low: states(json.get("low_states"))?,
                    high: states(json.get("high_states"))?,
                }
            }
            "dual_noise" => {
                let s = seed().ok_or_else(|| Error::Invalid("bad seed".into()))?;
                let variety = match field("variety")? {
                    Json::Array(a) if a.len() == 2 => (a[0].as_i32().unwrap_or(1), a[1].as_i32().unwrap_or(1)),
                    v => (int(v, "min_inclusive")?, int(v, "max_inclusive")?),
                };
                StateProvider::DualNoise {
                    noise: noise(json, "noise", s)?,
                    scale: float(json, "scale")?,
                    states: states(json.get("states"))?,
                    variety,
                    slow: noise(json, "slow_noise", s)?,
                    slow_scale: float(json, "slow_scale")?,
                }
            }
            t => return Err(Error::Invalid(format!("unsupported block state provider {t}"))),
        })
    }

    /// `getState`: always a state (rule-based and random-block providers fall back to the
    /// block already there).
    pub fn state(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> u16 {
        match self.optional_state(r, random, p) {
            Some(s) => s,
            None => r.get(p),
        }
    }

    /// `getOptionalState`.
    pub fn optional_state(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> Option<u16> {
        Some(match self {
            StateProvider::Simple(s) => *s,
            StateProvider::Weighted(w) => *w.pick(random).expect("weighted state provider is empty"),
            StateProvider::Rotated { source, direction } => {
                let d = match direction {
                    Some(d) => *d,
                    None => crate::block_facts::Dir::from_index(random.next_int_bounded(6) as usize),
                };
                rotate(source.state(r, random, p), d)
            }
            StateProvider::RandomizedInt { source, property, values } => {
                let s = source.state(r, random, p);
                let info = block_of(s);
                let is_int = info
                    .properties
                    .iter()
                    .find(|q| q.name == property)
                    .is_some_and(|q| q.values.iter().all(|v| v.parse::<i32>().is_ok()));
                if !is_int {
                    return Some(s);
                }
                let v = values.sample(random);
                info.with_property(s, property, &v.to_string()).expect("randomized int value out of range")
            }
            StateProvider::RandomBlock(blocks) => {
                if blocks.is_empty() {
                    return None;
                }
                blocks[random.next_int_bounded(blocks.len() as i32) as usize]
            }
            StateProvider::RuleBased { fallback, rules } => {
                for (pred, then) in rules {
                    if pred.test(r, p)
                        && let Some(s) = then.optional_state(r, random, p)
                    {
                        return Some(s);
                    }
                }
                return fallback.as_ref().and_then(|f| f.optional_state(r, random, p));
            }
            StateProvider::CopyProperties(source) => {
                let s = source.state(r, random, p);
                with_properties_of(s, r.get(p))
            }
            StateProvider::Noise { noise, scale, states } => pick_by_noise(states, noise_value(noise, p, *scale as f64)),
            StateProvider::NoiseThreshold { noise, scale, threshold, high_chance, default, low, high } => {
                let v = noise_value(noise, p, *scale as f64) as f64;
                if v < *threshold as f64 {
                    low[random.next_int_bounded(low.len() as i32) as usize]
                } else if random.next_float() < *high_chance {
                    high[random.next_int_bounded(high.len() as i32) as usize]
                } else {
                    *default
                }
            }
            StateProvider::DualNoise { noise, scale, states, variety, slow, slow_scale } => {
                let slow_at = |q: BlockPos| {
                    slow.get3((q.x as f32 * slow_scale) as f64, (q.y as f32 * slow_scale) as f64, (q.z as f32 * slow_scale) as f64)
                };
                let v = slow_at(p) as f64;
                let n = clamped_map(v, -1.0, 1.0, variety.0 as f64, (variety.1 + 1) as f64) as i32;
                let mut chosen = Vec::with_capacity(n.max(0) as usize);
                for i in 0..n {
                    chosen.push(pick_by_noise(states, slow_at(p.offset(i * 54545, 0, i * 34234))));
                }
                pick_by_noise(&chosen, noise_value(noise, p, *scale as f64))
            }
        })
    }
}

/// `NoiseBasedStateProvider.getNoiseValue`.
fn noise_value(noise: &NoiseStack, p: BlockPos, scale: f64) -> f32 {
    noise.get3(p.x as f64 * scale, p.y as f64 * scale, p.z as f64 * scale)
}

/// `NoiseProvider.getRandomState(states, value)`.
fn pick_by_noise(states: &[u16], v: f32) -> u16 {
    let t = kiln_javamath::math::clamp((1.0 + v) / 2.0, 0.0, 0.9999);
    states[(t * states.len() as f32) as usize]
}

/// `Mth.clampedMap` (double).
pub fn clamped_map(v: f64, from_lo: f64, from_hi: f64, to_lo: f64, to_hi: f64) -> f64 {
    let t = (v - from_lo) / (from_hi - from_lo);
    if t < 0.0 {
        to_lo
    } else if t > 1.0 {
        to_hi
    } else {
        to_lo + t * (to_hi - to_lo)
    }
}

/// `RotatedBlockProvider`: axis and facing (six-way, then horizontal) set where the block has
/// them.
fn rotate(s: u16, d: crate::block_facts::Dir) -> u16 {
    use crate::block_facts::Dir;
    let info = block_of(s);
    let values = |name: &str| info.properties.iter().find(|p| p.name == name).map(|p| p.values);
    let mut s = s;
    let axis = match d {
        Dir::Down | Dir::Up => "y",
        Dir::North | Dir::South => "z",
        Dir::West | Dir::East => "x",
    };
    if values("axis").is_some_and(|v| v.len() == 3) {
        s = with_prop(s, "axis", axis);
    }
    match values("facing") {
        Some(v) if v.len() == 6 => s = with_prop(s, "facing", d.name()),
        Some(v) if v.len() == 4 && d.is_horizontal() => s = with_prop(s, "facing", d.name()),
        _ => {}
    }
    s
}

/// `BlockState.withPropertiesOf(other)`: copies every property the two blocks share.
pub fn with_properties_of(s: u16, other: u16) -> u16 {
    let mut s = s;
    for p in block_of(other).properties {
        if has_prop(s, p.name)
            && let Some(v) = prop(other, p.name)
        {
            s = with_prop(s, p.name, v);
        }
    }
    s
}
