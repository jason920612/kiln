//! Number providers: `ContextIntProvider` and `ContextFloatProvider` (26.3 merged the old loot
//! number providers into these two dispatch registries, with entries also loadable from
//! `context_int_provider/` and `context_float_provider/`).
//!
//! Evaluation mirrors vanilla's two layers: `get*Unsafe` may fail with an arithmetic error
//! ([`Arith`]) that composite providers propagate, and `getInt` / `getFloat` turn a failure
//! (or a non-finite float) into 0.

use crate::condition::Condition;
use crate::data::Kind;
use crate::eval::Eval;
use crate::json::Json;
use crate::parse::{PResult, Parser, Ref, fail, float, ident, int, list, obj, opt_or, req, string};
use crate::random::RngExt;
use kiln_javamath::random::RandomSource;
use crate::context::EntityTarget;
use kiln_command::nbt_path::NbtPath;
use kiln_item::Identifier;
use kiln_javamath::math;

/// `ArithmeticException` inside a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arith;

pub type ArithResult<T> = Result<T, Arith>;

/// `ScoreboardNameProvider`: a fixed name, or an entity of the context.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreTarget {
    Fixed(String),
    Context(EntityTarget),
}

/// `StoredNumberAccess`: a numeric tag in command storage.
#[derive(Debug, Clone)]
pub struct StoredNumber {
    pub storage: Identifier,
    pub path: NbtPath,
    /// The path as written.
    pub path_text: String,
}

#[derive(Debug, Clone)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: Ref<IntProvider>, max: Ref<IntProvider> },
    Binomial { n: Ref<IntProvider>, p: Ref<FloatProvider> },
    Score { target: ScoreTarget, score: String, fallback: Ref<IntProvider> },
    Storage { access: StoredNumber, fallback: Ref<IntProvider> },
    EnvironmentAttribute(Identifier),
    Abs(Ref<IntProvider>),
    Negate(Ref<IntProvider>),
    FromFloat(Ref<FloatProvider>),
    Avg(Vec<Ref<IntProvider>>),
    Max(Vec<Ref<IntProvider>>),
    Min(Vec<Ref<IntProvider>>),
    Mul(Vec<Ref<IntProvider>>),
    Add(Vec<Ref<IntProvider>>),
    Sub(Ref<IntProvider>, Ref<IntProvider>),
    Div(Ref<IntProvider>, Ref<IntProvider>),
    Mod(Ref<IntProvider>, Ref<IntProvider>),
    FloorDiv(Ref<IntProvider>, Ref<IntProvider>),
    FloorMod(Ref<IntProvider>, Ref<IntProvider>),
    Pow { base: Ref<IntProvider>, exponent: Ref<IntProvider> },
    Conditional { condition: Ref<Condition>, on_true: Ref<IntProvider>, on_false: Ref<IntProvider> },
    Dispatcher { cases: Vec<(Ref<Condition>, Ref<IntProvider>)>, default: Ref<IntProvider> },
    WeightedList(Vec<(Ref<IntProvider>, i32)>),
}

#[derive(Debug, Clone)]
pub enum FloatProvider {
    Constant(f32),
    Uniform { min: Ref<FloatProvider>, max: Ref<FloatProvider> },
    EnchantmentLevel(LevelBasedValue),
    Storage { access: StoredNumber, fallback: Ref<FloatProvider> },
    EnvironmentAttribute(Identifier),
    Abs(Ref<FloatProvider>),
    Ceil(Ref<FloatProvider>),
    Floor(Ref<FloatProvider>),
    Round(Ref<FloatProvider>),
    Truncate(Ref<FloatProvider>),
    Negate(Ref<FloatProvider>),
    Sin(Ref<FloatProvider>),
    Cos(Ref<FloatProvider>),
    Sqrt(Ref<FloatProvider>),
    FromInt(Ref<IntProvider>),
    Avg(Vec<Ref<FloatProvider>>),
    Length(Vec<Ref<FloatProvider>>),
    Max(Vec<Ref<FloatProvider>>),
    Min(Vec<Ref<FloatProvider>>),
    Mul(Vec<Ref<FloatProvider>>),
    Add(Vec<Ref<FloatProvider>>),
    Sub(Ref<FloatProvider>, Ref<FloatProvider>),
    Div(Ref<FloatProvider>, Ref<FloatProvider>),
    Mod(Ref<FloatProvider>, Ref<FloatProvider>),
    Pow { base: Ref<FloatProvider>, exponent: Ref<FloatProvider> },
    Conditional { condition: Ref<Condition>, on_true: Ref<FloatProvider>, on_false: Ref<FloatProvider> },
    Dispatcher { cases: Vec<(Ref<Condition>, Ref<FloatProvider>)>, default: Ref<FloatProvider> },
    WeightedList(Vec<(Ref<FloatProvider>, i32)>),
}

/// `LevelBasedValue`: a value computed from an enchantment level.
#[derive(Debug, Clone, PartialEq)]
pub enum LevelBasedValue {
    Constant(f32),
    Linear { base: f32, per_level_above_first: f32 },
    LevelsSquared { added: f32 },
    Clamped { value: Box<LevelBasedValue>, min: f32, max: f32 },
    Fraction { numerator: Box<LevelBasedValue>, denominator: Box<LevelBasedValue> },
    Exponent { base: Box<LevelBasedValue>, power: Box<LevelBasedValue> },
    Lookup { values: Vec<f32>, fallback: Box<LevelBasedValue> },
}

impl LevelBasedValue {
    pub fn calculate(&self, level: i32) -> f32 {
        match self {
            LevelBasedValue::Constant(v) => *v,
            LevelBasedValue::Linear { base, per_level_above_first } => {
                base + per_level_above_first * level.wrapping_sub(1) as f32
            }
            LevelBasedValue::LevelsSquared { added } => level.wrapping_mul(level) as f32 + added,
            LevelBasedValue::Clamped { value, min, max } => math::clamp(value.calculate(level), *min, *max),
            LevelBasedValue::Fraction { numerator, denominator } => {
                let d = denominator.calculate(level);
                if d == 0.0 { 0.0 } else { numerator.calculate(level) / d }
            }
            LevelBasedValue::Exponent { base, power } => {
                (base.calculate(level) as f64).powf(power.calculate(level) as f64) as f32
            }
            LevelBasedValue::Lookup { values, fallback } => {
                if level <= values.len() as i32 && level >= 1 {
                    values[(level - 1) as usize]
                } else if level <= values.len() as i32 {
                    // `values.get(level - 1)` with level < 1 throws in vanilla; there is no
                    // sensible value, so behave like the fallback.
                    fallback.calculate(level)
                } else {
                    fallback.calculate(level)
                }
            }
        }
    }

    pub fn parse(j: &Json) -> PResult<LevelBasedValue> {
        if j.is_number() {
            return Ok(LevelBasedValue::Constant(float(j)?));
        }
        let ty = req(j, "type", ident)?;
        let boxed = |key: &str| req(j, key, |v| LevelBasedValue::parse(v).map(Box::new));
        Ok(match ty.as_str() {
            "minecraft:linear" => LevelBasedValue::Linear {
                base: req(j, "base", float)?,
                per_level_above_first: req(j, "per_level_above_first", float)?,
            },
            "minecraft:levels_squared" => LevelBasedValue::LevelsSquared { added: req(j, "added", float)? },
            "minecraft:clamped" => {
                let (min, max) = (req(j, "min", float)?, req(j, "max", float)?);
                if max <= min {
                    return fail(format!("max must be larger than min, min: {min}, max: {max}"));
                }
                LevelBasedValue::Clamped { value: boxed("value")?, min, max }
            }
            "minecraft:fraction" => {
                LevelBasedValue::Fraction { numerator: boxed("numerator")?, denominator: boxed("denominator")? }
            }
            "minecraft:exponent" => LevelBasedValue::Exponent { base: boxed("base")?, power: boxed("power")? },
            "minecraft:lookup" => {
                LevelBasedValue::Lookup { values: req(j, "values", |v| list(v, float))?, fallback: boxed("fallback")? }
            }
            other => return fail(format!("unknown level based value type {other}")),
        })
    }
}

// ---- decoding -----------------------------------------------------------------------------

impl IntProvider {
    /// `ContextIntProviders.CODEC`: a reference to `context_int_provider/`, a bare int, or a
    /// typed object.
    pub fn parse_ref(p: &Parser, j: &Json) -> PResult<Ref<IntProvider>> {
        p.holder(j, Kind::IntProvider, IntProvider::parse)
    }

    /// `ContextIntProviders.DIRECT_CODEC`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<IntProvider> {
        if j.is_number() {
            return Ok(IntProvider::Constant(int(j)?));
        }
        let ty = req(j, "type", ident)?;
        let r = |key: &str| req(j, key, |v| IntProvider::parse_ref(p, v));
        let inputs = || req(j, "inputs", |v| p.holder_list(v, Kind::IntProvider, IntProvider::parse));
        Ok(match ty.as_str() {
            "minecraft:constant" => IntProvider::Constant(req(j, "value", int)?),
            "minecraft:uniform" => IntProvider::Uniform { min: r("min")?, max: r("max")? },
            "minecraft:binomial" => IntProvider::Binomial { n: r("n")?, p: req(j, "p", |v| FloatProvider::parse_ref(p, v))? },
            "minecraft:score" => IntProvider::Score {
                target: req(j, "target", score_target)?,
                score: req(j, "score", string)?,
                fallback: opt_or(j, "fallback", Ref::direct(IntProvider::Constant(0)), |v| IntProvider::parse_ref(p, v))?,
            },
            "minecraft:storage" => IntProvider::Storage {
                access: stored_number(j)?,
                fallback: opt_or(j, "fallback", Ref::direct(IntProvider::Constant(0)), |v| IntProvider::parse_ref(p, v))?,
            },
            "minecraft:environment_attribute" => IntProvider::EnvironmentAttribute(req(j, "attribute", ident)?),
            "minecraft:abs" => IntProvider::Abs(r("input")?),
            "minecraft:negate" => IntProvider::Negate(r("input")?),
            "minecraft:from_float" => IntProvider::FromFloat(req(j, "input", |v| FloatProvider::parse_ref(p, v))?),
            "minecraft:avg" => IntProvider::Avg(inputs()?),
            "minecraft:max" => IntProvider::Max(inputs()?),
            "minecraft:min" => IntProvider::Min(inputs()?),
            "minecraft:mul" => IntProvider::Mul(inputs()?),
            "minecraft:add" => IntProvider::Add(inputs()?),
            "minecraft:sub" => IntProvider::Sub(r("left")?, r("right")?),
            "minecraft:div" => IntProvider::Div(r("left")?, r("right")?),
            "minecraft:mod" => IntProvider::Mod(r("left")?, r("right")?),
            "minecraft:floor_div" => IntProvider::FloorDiv(r("left")?, r("right")?),
            "minecraft:floor_mod" => IntProvider::FloorMod(r("left")?, r("right")?),
            "minecraft:pow" => IntProvider::Pow { base: r("base")?, exponent: r("exponent")? },
            "minecraft:conditional" => IntProvider::Conditional {
                condition: req(j, "condition", |v| Condition::parse_ref(p, v))?,
                on_true: r("on_true")?,
                on_false: opt_or(j, "on_false", Ref::direct(IntProvider::Constant(0)), |v| IntProvider::parse_ref(p, v))?,
            },
            "minecraft:number_dispatcher" => IntProvider::Dispatcher {
                cases: req(j, "cases", |v| {
                    list(v, |c| {
                        Ok((req(c, "condition", |v| Condition::parse_ref(p, v))?, req(c, "value", |v| IntProvider::parse_ref(p, v))?))
                    })
                })?,
                default: opt_or(j, "default", Ref::direct(IntProvider::Constant(0)), |v| IntProvider::parse_ref(p, v))?,
            },
            "minecraft:weighted_list" => {
                IntProvider::WeightedList(req(j, "distribution", |v| weighted(v, |e| IntProvider::parse_ref(p, e)))?)
            }
            other => return fail(format!("unknown context int provider type {other}")),
        })
    }
}

impl FloatProvider {
    pub fn parse_ref(p: &Parser, j: &Json) -> PResult<Ref<FloatProvider>> {
        p.holder(j, Kind::FloatProvider, FloatProvider::parse)
    }

    /// `ContextFloatProviders.DIRECT_CODEC`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<FloatProvider> {
        if j.is_number() {
            return Ok(FloatProvider::Constant(float(j)?));
        }
        let ty = req(j, "type", ident)?;
        let r = |key: &str| req(j, key, |v| FloatProvider::parse_ref(p, v));
        let inputs = || req(j, "inputs", |v| p.holder_list(v, Kind::FloatProvider, FloatProvider::parse));
        Ok(match ty.as_str() {
            "minecraft:constant" => FloatProvider::Constant(req(j, "value", float)?),
            "minecraft:uniform" => FloatProvider::Uniform { min: r("min")?, max: r("max")? },
            "minecraft:enchantment_level" => FloatProvider::EnchantmentLevel(req(j, "amount", LevelBasedValue::parse)?),
            "minecraft:storage" => FloatProvider::Storage {
                access: stored_number(j)?,
                fallback: opt_or(j, "fallback", Ref::direct(FloatProvider::Constant(0.0)), |v| FloatProvider::parse_ref(p, v))?,
            },
            "minecraft:environment_attribute" => FloatProvider::EnvironmentAttribute(req(j, "attribute", ident)?),
            "minecraft:abs" => FloatProvider::Abs(r("input")?),
            "minecraft:ceil" => FloatProvider::Ceil(r("input")?),
            "minecraft:floor" => FloatProvider::Floor(r("input")?),
            "minecraft:round" => FloatProvider::Round(r("input")?),
            "minecraft:truncate" => FloatProvider::Truncate(r("input")?),
            "minecraft:negate" => FloatProvider::Negate(r("input")?),
            "minecraft:sin" => FloatProvider::Sin(r("input")?),
            "minecraft:cos" => FloatProvider::Cos(r("input")?),
            "minecraft:sqrt" => FloatProvider::Sqrt(r("input")?),
            "minecraft:from_int" => FloatProvider::FromInt(req(j, "input", |v| IntProvider::parse_ref(p, v))?),
            "minecraft:avg" => FloatProvider::Avg(inputs()?),
            "minecraft:length" => FloatProvider::Length(inputs()?),
            "minecraft:max" => FloatProvider::Max(inputs()?),
            "minecraft:min" => FloatProvider::Min(inputs()?),
            "minecraft:mul" => FloatProvider::Mul(inputs()?),
            "minecraft:add" => FloatProvider::Add(inputs()?),
            "minecraft:sub" => FloatProvider::Sub(r("left")?, r("right")?),
            "minecraft:div" => FloatProvider::Div(r("left")?, r("right")?),
            "minecraft:mod" => FloatProvider::Mod(r("left")?, r("right")?),
            "minecraft:pow" => FloatProvider::Pow { base: r("base")?, exponent: r("exponent")? },
            "minecraft:conditional" => FloatProvider::Conditional {
                condition: req(j, "condition", |v| Condition::parse_ref(p, v))?,
                on_true: r("on_true")?,
                on_false: opt_or(j, "on_false", Ref::direct(FloatProvider::Constant(0.0)), |v| FloatProvider::parse_ref(p, v))?,
            },
            "minecraft:number_dispatcher" => FloatProvider::Dispatcher {
                cases: req(j, "cases", |v| {
                    list(v, |c| {
                        Ok((req(c, "condition", |v| Condition::parse_ref(p, v))?, req(c, "value", |v| FloatProvider::parse_ref(p, v))?))
                    })
                })?,
                default: opt_or(j, "default", Ref::direct(FloatProvider::Constant(0.0)), |v| FloatProvider::parse_ref(p, v))?,
            },
            "minecraft:weighted_list" => {
                FloatProvider::WeightedList(req(j, "distribution", |v| weighted(v, |e| FloatProvider::parse_ref(p, e)))?)
            }
            other => return fail(format!("unknown context float provider type {other}")),
        })
    }
}

/// `ScoreboardNameProviders.CODEC`: an entity target name, or `{type: fixed|context, ...}`.
pub fn score_target(j: &Json) -> PResult<ScoreTarget> {
    if let Json::Str(_) = j {
        return entity_target(j).map(ScoreTarget::Context);
    }
    let ty = req(j, "type", ident)?;
    match ty.as_str() {
        "minecraft:fixed" => Ok(ScoreTarget::Fixed(req(j, "name", string)?)),
        "minecraft:context" => Ok(ScoreTarget::Context(req(j, "target", entity_target)?)),
        other => fail(format!("unknown loot score provider type {other}")),
    }
}

pub fn entity_target(j: &Json) -> PResult<EntityTarget> {
    let s = j.as_str().ok_or_else(|| crate::parse::ParseError::new("expected an entity target"))?;
    EntityTarget::by_name(s).ok_or_else(|| crate::parse::ParseError::new(format!("unknown entity target {s:?}")))
}

fn stored_number(j: &Json) -> PResult<StoredNumber> {
    let storage = req(j, "storage", ident)?;
    let path_text = req(j, "path", string)?;
    let path = parse_nbt_path(&path_text)?;
    Ok(StoredNumber { storage, path, path_text })
}

pub fn parse_nbt_path(text: &str) -> PResult<NbtPath> {
    let mut reader = kiln_command::reader::StringReader::new(text);
    let path = NbtPath::parse(&mut reader).map_err(|e| crate::parse::ParseError::new(format!("invalid NBT path {text:?}: {e:?}")))?;
    if reader.can_read() {
        return fail(format!("invalid NBT path {text:?}: trailing characters"));
    }
    Ok(path)
}

/// `WeightedList.nonEmptyCodec`: `[{data, weight}]`, weights non-negative, total positive.
fn weighted<T>(j: &Json, mut f: impl FnMut(&Json) -> PResult<T>) -> PResult<Vec<(T, i32)>> {
    let entries = list(j, |e| {
        obj(e)?;
        let weight = req(e, "weight", int)?;
        if weight < 0 {
            return fail(format!("weight should be >= 0: {weight}"));
        }
        Ok((req(e, "data", &mut f)?, weight))
    })?;
    if entries.is_empty() {
        return fail("weighted list must not be empty");
    }
    let total: i64 = entries.iter().map(|(_, w)| *w as i64).sum();
    if total > i32::MAX as i64 {
        return fail("sum of weights must be <= 2147483647");
    }
    Ok(entries)
}

// ---- evaluation ---------------------------------------------------------------------------

/// `ContextIntProvider.longToIntSafe`.
fn long_to_int(v: i64) -> ArithResult<i32> {
    i32::try_from(v).map_err(|_| Arith)
}

/// `ContextIntProvider.floatToIntSafe`: non-finite fails; the float truncates to a long first.
fn float_to_int(v: f32) -> ArithResult<i32> {
    if !v.is_finite() {
        return Err(Arith);
    }
    long_to_int(v as i64)
}

/// `Math.powExact(int, int)`.
fn pow_exact(base: i32, exponent: i32) -> ArithResult<i32> {
    if exponent < 0 {
        return Err(Arith);
    }
    match base {
        0 => Ok(if exponent == 0 { 1 } else { 0 }),
        1 => Ok(1),
        -1 => Ok(if exponent % 2 == 0 { 1 } else { -1 }),
        _ => {
            // |base| >= 2 overflows within 31 multiplications.
            let mut result: i32 = 1;
            for _ in 0..exponent {
                result = result.checked_mul(base).ok_or(Arith)?;
            }
            Ok(result)
        }
    }
}

/// A pick from a `WeightedList` (`getRandomOrThrow`): `nextInt(total)` then the entry whose
/// cumulative weight passes it.
fn pick_weighted<'a, T>(entries: &'a [(T, i32)], rng: &mut dyn RandomSource) -> &'a T {
    let total: i32 = entries.iter().map(|(_, w)| *w).sum();
    let mut r = rng.bounded(total);
    for (v, w) in entries {
        r -= w;
        if r < 0 {
            return v;
        }
    }
    &entries[entries.len() - 1].0
}

impl Eval<'_> {
    /// `ContextIntProvider.getInt`.
    pub fn int(&mut self, p: &Ref<IntProvider>) -> i32 {
        self.int_unsafe(p).unwrap_or(0)
    }

    /// `ContextFloatProvider.getFloat`.
    pub fn float(&mut self, p: &Ref<FloatProvider>) -> f32 {
        match self.float_unsafe(p) {
            Ok(v) if v.is_finite() => v,
            _ => 0.0,
        }
    }

    /// `ContextFloatProvider.getFloatOrThrow`.
    pub fn float_or_throw(&mut self, p: &Ref<FloatProvider>) -> ArithResult<f32> {
        let v = self.float_unsafe(p)?;
        if v.is_finite() { Ok(v) } else { Err(Arith) }
    }

    pub fn int_unsafe(&mut self, p: &Ref<IntProvider>) -> ArithResult<i32> {
        let data = self.data;
        let p: &IntProvider = match p {
            Ref::Direct(v) => v,
            Ref::Named(i) => match data.int_providers.get(*i) {
                Some(v) => v,
                None => return Ok(0),
            },
        };
        match p {
            IntProvider::Constant(v) => Ok(*v),
            IntProvider::Uniform { min, max } => {
                let min = self.int_unsafe(min)?;
                let max = self.int_unsafe(max)?;
                Ok(self.rng.next_int_between(min, max))
            }
            IntProvider::Binomial { n, p } => {
                let n = self.int_unsafe(n)?;
                let p = self.float_or_throw(p)?;
                let mut hits = 0;
                for _ in 0..n {
                    if self.rng.next_float() < p {
                        hits += 1;
                    }
                }
                Ok(hits)
            }
            IntProvider::Score { target, score, fallback } => match self.score(target, score) {
                Some(v) => Ok(v),
                None => self.int_unsafe(fallback),
            },
            IntProvider::Storage { access, fallback } => match self.ctx.storage_number(&access.storage, &access.path) {
                Some(n) => Ok(n.int_value()),
                None => self.int_unsafe(fallback),
            },
            IntProvider::EnvironmentAttribute(attr) => self.ctx.environment_attribute_int(attr).ok_or(Arith),
            IntProvider::Abs(v) => {
                let v = self.int_unsafe(v)?;
                v.checked_abs().ok_or(Arith)
            }
            IntProvider::Negate(v) => {
                let v = self.int_unsafe(v)?;
                v.checked_neg().ok_or(Arith)
            }
            IntProvider::FromFloat(v) => {
                let v = self.float_unsafe(v)?;
                float_to_int(v)
            }
            IntProvider::Avg(inputs) => {
                let (mut sum, mut count) = (0i64, 0i64);
                for v in inputs {
                    sum = sum.wrapping_add(self.int_unsafe(v)? as i64);
                    count += 1;
                }
                if count == 0 {
                    return Err(Arith);
                }
                long_to_int(sum.wrapping_div(count))
            }
            IntProvider::Max(inputs) => {
                let mut acc = i32::MIN;
                for v in inputs {
                    acc = acc.max(self.int_unsafe(v)?);
                }
                Ok(acc)
            }
            IntProvider::Min(inputs) => {
                let mut acc = i32::MAX;
                for v in inputs {
                    acc = acc.min(self.int_unsafe(v)?);
                }
                Ok(acc)
            }
            IntProvider::Mul(inputs) => {
                let mut acc = 1i64;
                for v in inputs {
                    acc = acc.wrapping_mul(self.int_unsafe(v)? as i64);
                }
                long_to_int(acc)
            }
            IntProvider::Add(inputs) => {
                let mut acc = 0i64;
                for v in inputs {
                    acc = acc.wrapping_add(self.int_unsafe(v)? as i64);
                }
                long_to_int(acc)
            }
            IntProvider::Sub(l, r) => {
                let (l, r) = (self.int_unsafe(l)?, self.int_unsafe(r)?);
                l.checked_sub(r).ok_or(Arith)
            }
            IntProvider::Div(l, r) => {
                let (l, r) = (self.int_unsafe(l)?, self.int_unsafe(r)?);
                if r == 0 { Err(Arith) } else { Ok(l.wrapping_div(r)) }
            }
            IntProvider::Mod(l, r) => {
                let (l, r) = (self.int_unsafe(l)?, self.int_unsafe(r)?);
                if r == 0 { Err(Arith) } else { Ok(l.wrapping_rem(r)) }
            }
            IntProvider::FloorDiv(l, r) => {
                let (l, r) = (self.int_unsafe(l)?, self.int_unsafe(r)?);
                if r == 0 || (l == i32::MIN && r == -1) { Err(Arith) } else { Ok(math::floor_div(l, r)) }
            }
            IntProvider::FloorMod(l, r) => {
                let (l, r) = (self.int_unsafe(l)?, self.int_unsafe(r)?);
                if r == 0 { Err(Arith) } else { Ok(math::floor_mod(l, r)) }
            }
            IntProvider::Pow { base, exponent } => {
                let (b, e) = (self.int_unsafe(base)?, self.int_unsafe(exponent)?);
                if b == 0 && e == 0 {
                    return Err(Arith);
                }
                pow_exact(b, e)
            }
            IntProvider::Conditional { condition, on_true, on_false } => {
                let branch = if self.test(condition) { on_true } else { on_false };
                self.int_unsafe(branch)
            }
            IntProvider::Dispatcher { cases, default } => {
                for (condition, value) in cases {
                    if self.test(condition) {
                        return self.int_unsafe(value);
                    }
                }
                self.int_unsafe(default)
            }
            IntProvider::WeightedList(entries) => {
                let v = pick_weighted(entries, self.rng).clone();
                self.int_unsafe(&v)
            }
        }
    }

    pub fn float_unsafe(&mut self, p: &Ref<FloatProvider>) -> ArithResult<f32> {
        let data = self.data;
        let p: &FloatProvider = match p {
            Ref::Direct(v) => v,
            Ref::Named(i) => match data.float_providers.get(*i) {
                Some(v) => v,
                None => return Ok(0.0),
            },
        };
        match p {
            FloatProvider::Constant(v) => Ok(*v),
            FloatProvider::Uniform { min, max } => {
                let min = self.float_unsafe(min)?;
                let max = self.float_unsafe(max)?;
                Ok(self.rng.next_float_between(min, max))
            }
            FloatProvider::EnchantmentLevel(v) => Ok(v.calculate(self.ctx.enchantment_level().unwrap_or(0))),
            FloatProvider::Storage { access, fallback } => match self.ctx.storage_number(&access.storage, &access.path) {
                Some(n) => Ok(n.float_value()),
                None => self.float_unsafe(fallback),
            },
            FloatProvider::EnvironmentAttribute(attr) => self.ctx.environment_attribute_float(attr).ok_or(Arith),
            FloatProvider::Abs(v) => Ok(self.float_unsafe(v)?.abs()),
            FloatProvider::Ceil(v) => Ok((self.float_unsafe(v)? as f64).ceil() as i32 as f32),
            FloatProvider::Floor(v) => Ok((self.float_unsafe(v)? as f64).floor() as f32),
            FloatProvider::Round(v) => Ok(java_round(self.float_unsafe(v)?) as f32),
            FloatProvider::Truncate(v) => {
                let v = self.float_unsafe(v)?;
                Ok(if v > 0.0 { (v as f64).floor() as f32 } else { (v as f64).ceil() as f32 })
            }
            FloatProvider::Negate(v) => Ok(-self.float_unsafe(v)?),
            FloatProvider::Sin(v) => Ok(mth_sin(self.float_unsafe(v)? as f64)),
            FloatProvider::Cos(v) => Ok(mth_cos(self.float_unsafe(v)? as f64)),
            FloatProvider::Sqrt(v) => Ok((self.float_unsafe(v)? as f64).sqrt() as f32),
            FloatProvider::FromInt(v) => Ok(self.int_unsafe(v)? as f32),
            FloatProvider::Avg(inputs) => {
                let (mut sum, mut count) = (0.0f32, 0i32);
                for v in inputs {
                    sum += self.float_unsafe(v)?;
                    count += 1;
                }
                Ok(sum / count as f32)
            }
            FloatProvider::Length(inputs) => {
                let mut sum = 0.0f32;
                for v in inputs {
                    let v = self.float_unsafe(v)?;
                    sum += v * v;
                }
                Ok((sum as f64).sqrt() as f32)
            }
            FloatProvider::Max(inputs) => {
                let mut acc = -f32::MAX;
                for v in inputs {
                    acc = math::max(acc, self.float_unsafe(v)?);
                }
                Ok(acc)
            }
            FloatProvider::Min(inputs) => {
                let mut acc = f32::MAX;
                for v in inputs {
                    acc = math::min(acc, self.float_unsafe(v)?);
                }
                Ok(acc)
            }
            FloatProvider::Mul(inputs) => {
                let mut acc = 1.0f32;
                for v in inputs {
                    acc *= self.float_unsafe(v)?;
                }
                Ok(acc)
            }
            FloatProvider::Add(inputs) => {
                let mut acc = 0.0f32;
                for v in inputs {
                    acc += self.float_unsafe(v)?;
                }
                Ok(acc)
            }
            FloatProvider::Sub(l, r) => {
                let l = self.float_unsafe(l)?;
                Ok(l - self.float_unsafe(r)?)
            }
            FloatProvider::Div(l, r) => {
                let l = self.float_unsafe(l)?;
                Ok(l / self.float_unsafe(r)?)
            }
            FloatProvider::Mod(l, r) => {
                // Vanilla evaluates the divisor first and skips the dividend when it is zero.
                let r = self.float_unsafe(r)?;
                if r == 0.0 {
                    return Ok(f32::NAN);
                }
                Ok(self.float_unsafe(l)? % r)
            }
            FloatProvider::Pow { base, exponent } => {
                let b = self.float_unsafe(base)?;
                let e = self.float_unsafe(exponent)?;
                if b == 0.0 && e == 0.0 {
                    return Ok(f32::NAN);
                }
                Ok((b as f64).powf(e as f64) as f32)
            }
            FloatProvider::Conditional { condition, on_true, on_false } => {
                let branch = if self.test(condition) { on_true } else { on_false };
                self.float_unsafe(branch)
            }
            FloatProvider::Dispatcher { cases, default } => {
                for (condition, value) in cases {
                    if self.test(condition) {
                        return self.float_unsafe(value);
                    }
                }
                self.float_unsafe(default)
            }
            FloatProvider::WeightedList(entries) => {
                let v = pick_weighted(entries, self.rng).clone();
                self.float_unsafe(&v)
            }
        }
    }

    fn score(&mut self, target: &ScoreTarget, objective: &str) -> Option<i32> {
        match target {
            ScoreTarget::Fixed(name) => self.ctx.score(&crate::context::ScoreHolder::Name(name), objective),
            ScoreTarget::Context(t) => {
                if !self.ctx.has_entity(*t) {
                    return None;
                }
                self.ctx.score(&crate::context::ScoreHolder::Entity(*t), objective)
            }
        }
    }
}

/// `Math.round(float)`: `floor(x + 0.5)` with Java's exact handling of halves and limits.
pub fn java_round(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    // Java 7+: rounds half up using exact arithmetic on the float's bits.
    let r = (v as f64 + 0.5).floor();
    // `v + 0.5` in double is exact for every float, so the floor is exact.
    r as i32
}

/// The SIN lookup table of `Mth`.
fn sin_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..65536).map(|i| (i as f64 / 10430.378350470453).sin() as f32).collect())
}

/// `Mth.sin(double)`.
pub fn mth_sin(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453) as i64 & 65535) as usize]
}

/// `Mth.cos(double)`.
pub fn mth_cos(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453 + 16384.0) as i64 & 65535) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_rounding_and_pow() {
        assert_eq!(java_round(2.5), 3);
        assert_eq!(java_round(-2.5), -2);
        assert_eq!(java_round(0.49999997), 0);
        assert_eq!(java_round(f32::MAX), i32::MAX);
        assert_eq!(pow_exact(2, 30), Ok(1 << 30));
        assert_eq!(pow_exact(2, 31), Err(Arith));
        assert_eq!(pow_exact(-1, i32::MAX), Ok(-1));
        assert_eq!(pow_exact(5, -1), Err(Arith));
    }

    #[test]
    fn level_based_values() {
        let lookup = LevelBasedValue::Lookup { values: vec![1.0, 2.0], fallback: Box::new(LevelBasedValue::Constant(9.0)) };
        assert_eq!(lookup.calculate(2), 2.0);
        assert_eq!(lookup.calculate(3), 9.0);
        let linear = LevelBasedValue::Linear { base: 0.1, per_level_above_first: 0.05 };
        assert_eq!(linear.calculate(3), 0.1 + 0.05 * 2.0);
    }
}
