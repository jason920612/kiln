//! Value providers of worldgen data: `IntProvider`, `FloatProvider`, `HeightProvider` and
//! `VerticalAnchor`, plus the weighted lists they (and features) draw from.

use crate::Error;
use crate::json::Json;
use crate::random::WorldgenRandom;
use kiln_javamath::random::RandomSource;

fn bad(what: &str, json: &Json) -> Error {
    Error::Invalid(format!("bad {what}: {json:?}"))
}

fn ty(json: &Json) -> &str {
    json.get("type").and_then(Json::as_str).map_or("", |t| t.strip_prefix("minecraft:").unwrap_or(t))
}

pub(crate) fn int(json: &Json, key: &str) -> Result<i32, Error> {
    json.get(key).and_then(Json::as_i32).ok_or_else(|| bad(key, json))
}

pub(crate) fn float(json: &Json, key: &str) -> Result<f32, Error> {
    json.get(key).and_then(Json::as_f32).ok_or_else(|| bad(key, json))
}

/// `Mth.randomBetweenInclusive`.
#[inline]
pub fn between_inclusive(r: &mut WorldgenRandom, min: i32, max: i32) -> i32 {
    r.next_int_bounded(max - min + 1) + min
}

/// `Mth.nextInt(random, min, max)`: `min` when the range is empty.
#[inline]
pub fn next_int(r: &mut WorldgenRandom, min: i32, max: i32) -> i32 {
    if min >= max { min } else { r.next_int_bounded(max - min + 1) + min }
}

/// `Mth.normal`.
#[inline]
pub fn normal(r: &mut WorldgenRandom, mean: f32, deviation: f32) -> f32 {
    mean + r.next_gaussian() as f32 * deviation
}

/// `WeightedList`: entries with positive weights, drawn by `nextInt(totalWeight)`.
#[derive(Clone, Debug)]
pub struct Weighted<T> {
    pub entries: Vec<(T, i32)>,
    pub total: i32,
}

impl<T> Weighted<T> {
    pub fn new(entries: Vec<(T, i32)>) -> Self {
        let total = entries.iter().map(|(_, w)| *w).sum();
        Self { entries, total }
    }

    /// `WeightedList.CODEC`: `[{"data": .., "weight": n}, ...]`.
    pub fn parse(json: &Json, data: impl Fn(&Json) -> Result<T, Error>) -> Result<Self, Error> {
        let list = json.as_array().ok_or_else(|| bad("weighted list", json))?;
        let entries = list
            .iter()
            .map(|e| Ok((data(e.get("data").ok_or_else(|| bad("weighted entry", e))?)?, int(e, "weight")?)))
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self::new(entries))
    }

    /// `WeightedList.getRandom`: `None` when the total weight is zero.
    pub fn pick(&self, r: &mut WorldgenRandom) -> Option<&T> {
        if self.total <= 0 {
            return None;
        }
        let mut i = r.next_int_bounded(self.total);
        for (v, w) in &self.entries {
            if i < *w {
                return Some(v);
            }
            i -= w;
        }
        None
    }
}

/// `IntProvider`.
#[derive(Clone, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: i32, max: i32 },
    BiasedToBottom { min: i32, max: i32 },
    VeryBiasedToBottom { min: i32, max: i32 },
    Clamped { source: Box<IntProvider>, min: i32, max: i32 },
    ClampedNormal { mean: f32, deviation: f32, min: i32, max: i32 },
    Trapezoid { min: i32, max: i32, plateau: i32 },
    WeightedList(Weighted<IntProvider>),
}

impl IntProvider {
    pub fn parse(json: &Json) -> Result<IntProvider, Error> {
        if let Some(v) = json.as_i32() {
            return Ok(IntProvider::Constant(v));
        }
        Ok(match ty(json) {
            "constant" => IntProvider::Constant(int(json, "value")?),
            "uniform" => IntProvider::Uniform { min: int(json, "min_inclusive")?, max: int(json, "max_inclusive")? },
            "biased_to_bottom" => IntProvider::BiasedToBottom { min: int(json, "min_inclusive")?, max: int(json, "max_inclusive")? },
            "very_biased_to_bottom" => {
                IntProvider::VeryBiasedToBottom { min: int(json, "min_inclusive")?, max: int(json, "max_inclusive")? }
            }
            "clamped" => IntProvider::Clamped {
                source: Box::new(IntProvider::parse(json.get("source").ok_or_else(|| bad("clamped", json))?)?),
                min: int(json, "min_inclusive")?,
                max: int(json, "max_inclusive")?,
            },
            "clamped_normal" => IntProvider::ClampedNormal {
                mean: float(json, "mean")?,
                deviation: float(json, "deviation")?,
                min: int(json, "min_inclusive")?,
                max: int(json, "max_inclusive")?,
            },
            "trapezoid" => IntProvider::Trapezoid { min: int(json, "min")?, max: int(json, "max")?, plateau: int(json, "plateau")? },
            "weighted_list" => IntProvider::WeightedList(Weighted::parse(
                json.get("distribution").ok_or_else(|| bad("weighted_list", json))?,
                IntProvider::parse,
            )?),
            _ => return Err(bad("int provider", json)),
        })
    }

    pub fn sample(&self, r: &mut WorldgenRandom) -> i32 {
        match self {
            IntProvider::Constant(v) => *v,
            IntProvider::Uniform { min, max } => between_inclusive(r, *min, *max),
            IntProvider::BiasedToBottom { min, max } => {
                let n = r.next_int_bounded(max - min + 1);
                min + r.next_int_bounded(n + 1)
            }
            IntProvider::VeryBiasedToBottom { min, max } => {
                let a = r.next_int_bounded(max - min + 1);
                let b = r.next_int_bounded(a + 1);
                min + r.next_int_bounded(b + 1)
            }
            IntProvider::Clamped { source, min, max } => source.sample(r).clamp(*min, *max),
            IntProvider::ClampedNormal { mean, deviation, min, max } => {
                kiln_javamath::math::clamp(normal(r, *mean, *deviation), *min as f32, *max as f32) as i32
            }
            IntProvider::Trapezoid { min, max, plateau } => {
                if *plateau == 0 && *max == -*min {
                    let a = r.next_int_bounded(max + 1);
                    return a - r.next_int_bounded(max + 1);
                }
                let range = max - min;
                if *plateau == range {
                    return between_inclusive(r, *min, *max);
                }
                let slope = (range - plateau) / 2;
                let high = range - slope;
                min + between_inclusive(r, 0, high) + between_inclusive(r, 0, slope)
            }
            IntProvider::WeightedList(w) => w.pick(r).expect("weighted list is empty").sample(r),
        }
    }

    /// `IntProvider.minInclusive` (used by some features to size buffers).
    pub fn min_value(&self) -> i32 {
        match self {
            IntProvider::Constant(v) => *v,
            IntProvider::Uniform { min, .. }
            | IntProvider::BiasedToBottom { min, .. }
            | IntProvider::VeryBiasedToBottom { min, .. }
            | IntProvider::Trapezoid { min, .. } => *min,
            IntProvider::Clamped { source, min, max } => source.min_value().clamp(*min, *max),
            IntProvider::ClampedNormal { min, .. } => *min,
            IntProvider::WeightedList(w) => w.entries.iter().map(|(p, _)| p.min_value()).min().unwrap_or(0),
        }
    }

    /// `IntProvider.maxInclusive`.
    pub fn max_value(&self) -> i32 {
        match self {
            IntProvider::Constant(v) => *v,
            IntProvider::Uniform { max, .. }
            | IntProvider::BiasedToBottom { max, .. }
            | IntProvider::VeryBiasedToBottom { max, .. }
            | IntProvider::Trapezoid { max, .. } => *max,
            IntProvider::Clamped { source, min, max } => source.max_value().clamp(*min, *max),
            IntProvider::ClampedNormal { max, .. } => *max,
            IntProvider::WeightedList(w) => w.entries.iter().map(|(p, _)| p.max_value()).max().unwrap_or(0),
        }
    }
}

/// `FloatProvider`.
#[derive(Clone, Debug)]
pub enum FloatProvider {
    Constant(f32),
    Uniform { min: f32, max: f32 },
    ClampedNormal { mean: f32, deviation: f32, min: f32, max: f32 },
    Trapezoid { min: f32, max: f32, plateau: f32 },
}

impl FloatProvider {
    pub fn parse(json: &Json) -> Result<FloatProvider, Error> {
        if let Some(v) = json.as_f32() {
            return Ok(FloatProvider::Constant(v));
        }
        Ok(match ty(json) {
            "constant" => FloatProvider::Constant(float(json, "value")?),
            "uniform" => FloatProvider::Uniform { min: float(json, "min_inclusive")?, max: float(json, "max_exclusive")? },
            "clamped_normal" => FloatProvider::ClampedNormal {
                mean: float(json, "mean")?,
                deviation: float(json, "deviation")?,
                min: float(json, "min")?,
                max: float(json, "max")?,
            },
            "trapezoid" => FloatProvider::Trapezoid { min: float(json, "min")?, max: float(json, "max")?, plateau: float(json, "plateau")? },
            _ => return Err(bad("float provider", json)),
        })
    }

    pub fn sample(&self, r: &mut WorldgenRandom) -> f32 {
        match *self {
            FloatProvider::Constant(v) => v,
            FloatProvider::Uniform { min, max } => r.next_float() * (max - min) + min,
            FloatProvider::ClampedNormal { mean, deviation, min, max } => {
                kiln_javamath::math::clamp(normal(r, mean, deviation), min, max)
            }
            FloatProvider::Trapezoid { min, max, plateau } => {
                let range = max - min;
                let slope = (range - plateau) / 2.0;
                let high = range - slope;
                min + r.next_float() * high + r.next_float() * slope
            }
        }
    }
}

/// `WorldGenerationContext`: the generator's lowest y and depth.
#[derive(Clone, Copy, Debug)]
pub struct GenContext {
    pub min_y: i32,
    pub height: i32,
    pub sea_level: i32,
}

/// `VerticalAnchor`.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
    RelativeToSeaLevel(i32),
}

impl Anchor {
    pub fn parse(json: &Json) -> Result<Anchor, Error> {
        if let Some(v) = json.get("absolute").and_then(Json::as_i32) {
            Ok(Anchor::Absolute(v))
        } else if let Some(v) = json.get("above_bottom").and_then(Json::as_i32) {
            Ok(Anchor::AboveBottom(v))
        } else if let Some(v) = json.get("below_top").and_then(Json::as_i32) {
            Ok(Anchor::BelowTop(v))
        } else if let Some(v) = json.get("relative_to_sea_level").and_then(Json::as_i32) {
            Ok(Anchor::RelativeToSeaLevel(v))
        } else {
            Err(bad("vertical anchor", json))
        }
    }

    pub fn resolve(self, g: GenContext) -> i32 {
        match self {
            Anchor::Absolute(y) => y,
            Anchor::AboveBottom(o) => g.min_y + o,
            Anchor::BelowTop(o) => g.height - 1 + g.min_y - o,
            Anchor::RelativeToSeaLevel(o) => g.sea_level + o,
        }
    }
}

/// `HeightProvider`.
#[derive(Clone, Debug)]
pub enum HeightProvider {
    Constant(Anchor),
    Uniform { min: Anchor, max: Anchor },
    BiasedToBottom { min: Anchor, max: Anchor, inner: i32 },
    VeryBiasedToBottom { min: Anchor, max: Anchor, inner: i32 },
    Trapezoid { min: Anchor, max: Anchor, plateau: i32 },
    WeightedList(Weighted<HeightProvider>),
}

impl HeightProvider {
    pub fn parse(json: &Json) -> Result<HeightProvider, Error> {
        let anchor = |k: &str| Anchor::parse(json.get(k).ok_or_else(|| bad(k, json))?);
        if json.get("type").is_none() {
            return Ok(HeightProvider::Constant(Anchor::parse(json)?));
        }
        Ok(match ty(json) {
            "constant" => HeightProvider::Constant(anchor("value")?),
            "uniform" => HeightProvider::Uniform { min: anchor("min_inclusive")?, max: anchor("max_inclusive")? },
            "biased_to_bottom" => HeightProvider::BiasedToBottom {
                min: anchor("min_inclusive")?,
                max: anchor("max_inclusive")?,
                inner: json.get("inner").and_then(Json::as_i32).unwrap_or(1),
            },
            "very_biased_to_bottom" => HeightProvider::VeryBiasedToBottom {
                min: anchor("min_inclusive")?,
                max: anchor("max_inclusive")?,
                inner: json.get("inner").and_then(Json::as_i32).unwrap_or(1),
            },
            "trapezoid" => HeightProvider::Trapezoid {
                min: anchor("min_inclusive")?,
                max: anchor("max_inclusive")?,
                plateau: json.get("plateau").and_then(Json::as_i32).unwrap_or(0),
            },
            "weighted_list" => HeightProvider::WeightedList(Weighted::parse(
                json.get("distribution").ok_or_else(|| bad("weighted_list", json))?,
                HeightProvider::parse,
            )?),
            _ => return Err(bad("height provider", json)),
        })
    }

    pub fn sample(&self, r: &mut WorldgenRandom, g: GenContext) -> i32 {
        match self {
            HeightProvider::Constant(a) => a.resolve(g),
            HeightProvider::Uniform { min, max } => {
                let (lo, hi) = (min.resolve(g), max.resolve(g));
                if lo > hi { lo } else { between_inclusive(r, lo, hi) }
            }
            HeightProvider::BiasedToBottom { min, max, inner } => {
                let (lo, hi) = (min.resolve(g), max.resolve(g));
                if hi - lo - inner < 0 {
                    return lo;
                }
                let n = r.next_int_bounded(hi - lo - inner + 1);
                r.next_int_bounded(n + inner) + lo
            }
            HeightProvider::VeryBiasedToBottom { min, max, inner } => {
                let (lo, hi) = (min.resolve(g), max.resolve(g));
                if hi - lo - inner < 0 {
                    return lo;
                }
                let a = next_int(r, lo + inner, hi);
                let b = next_int(r, lo, a - 1);
                next_int(r, lo, b - 1 + inner)
            }
            HeightProvider::Trapezoid { min, max, plateau } => {
                let (lo, hi) = (min.resolve(g), max.resolve(g));
                if lo > hi {
                    return lo;
                }
                let range = hi - lo;
                if *plateau >= range {
                    return between_inclusive(r, lo, hi);
                }
                let slope = (range - plateau) / 2;
                let high = range - slope;
                lo + between_inclusive(r, 0, high) + between_inclusive(r, 0, slope)
            }
            HeightProvider::WeightedList(w) => w.pick(r).expect("weighted list is empty").sample(r, g),
        }
    }
}
