//! Density function definitions as the datapack declares them, plus the static analyses
//! vanilla's compiler runs on them: which axes a function depends on and its value range.

use crate::interval::Interval;
use crate::json::Json;
use crate::noise::{NormalNoiseParams, blended_fbm_range};
use crate::spline::SplineDef;
use crate::Error;
use kiln_javamath::math as jm;
use std::cell::RefCell;
use std::collections::HashMap;

pub type NodeId = u32;

pub const AXIS_X: u8 = 1;
pub const AXIS_Y: u8 = 2;
pub const AXIS_Z: u8 = 4;
pub const ALL_AXES: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn bit(self) -> u8 {
        match self {
            Axis::X => AXIS_X,
            Axis::Y => AXIS_Y,
            Axis::Z => AXIS_Z,
        }
    }

    pub fn choose(self, x: i32, y: i32, z: i32) -> i32 {
        match self {
            Axis::X => x,
            Axis::Y => y,
            Axis::Z => z,
        }
    }

    fn parse(s: &str) -> Option<Axis> {
        match s {
            "x" => Some(Axis::X),
            "y" => Some(Axis::Y),
            "z" => Some(Axis::Z),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Abs,
    Square,
    Cube,
    Sqrt,
    HalfNegative,
    QuarterNegative,
    Reciprocal,
    Negate,
    Squeeze,
    Log,
    Sign,
}

impl UnaryOp {
    /// `UnaryFunction.SqueezeSampler.apply`.
    pub fn squeeze(x: f32) -> f32 {
        let c = jm::clamp(x, -1.0, 1.0);
        c / 2.0 - jm::cube(c) / 24.0
    }

    /// `UnaryFunction.LeakyReLUSampler.apply`.
    pub fn leaky(negative_factor: f32, x: f32) -> f32 {
        if x > 0.0 { x } else { x * negative_factor }
    }

    pub fn apply(self, x: f32) -> f32 {
        match self {
            UnaryOp::Abs => x.abs(),
            UnaryOp::Square => jm::square(x),
            UnaryOp::Cube => jm::cube(x),
            UnaryOp::Sqrt => (x as f64).sqrt() as f32,
            UnaryOp::HalfNegative => Self::leaky(0.5, x),
            UnaryOp::QuarterNegative => Self::leaky(0.25, x),
            UnaryOp::Reciprocal => 1.0 / x,
            UnaryOp::Negate => -x,
            UnaryOp::Squeeze => Self::squeeze(x),
            UnaryOp::Log => (x as f64).ln() as f32,
            UnaryOp::Sign => jm::signum(x),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceMetric {
    Euclidean,
    EuclideanSquared,
    Manhattan,
    Chebyshev,
}

impl DistanceMetric {
    pub fn compute(self, x: f32, y: f32, z: f32) -> f32 {
        match self {
            DistanceMetric::Euclidean => ((x * x + y * y + z * z) as f64).sqrt() as f32,
            DistanceMetric::EuclideanSquared => x * x + y * y + z * z,
            DistanceMetric::Manhattan => x.abs() + y.abs() + z.abs(),
            DistanceMetric::Chebyshev => jm::max(jm::max(x.abs(), y.abs()), z.abs()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiling {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

/// A density function node. References to registry entries stay symbolic until compiled.
#[derive(Clone, Debug)]
pub enum Node {
    Const(f32),
    Ref(String),
    Unary(UnaryOp, NodeId),
    Binary(BinaryOp, NodeId, NodeId),
    Clamp { input: NodeId, min: f32, max: f32 },
    Lerp { alpha: NodeId, first: NodeId, second: NodeId },
    RangeChoice { input: NodeId, min_inclusive: f32, max_exclusive: f32, when_in_range: NodeId, when_out_of_range: NodeId },
    IntervalSelect { input: NodeId, thresholds: Vec<f32>, functions: Vec<NodeId> },
    Noise { noise: String, xz_scale: f64, y_scale: f64, shift: [NodeId; 3] },
    ShiftA(String),
    ShiftB(String),
    Shift(String),
    Gradient { axis: Axis, tiling: Tiling, from_coordinate: i32, to_coordinate: i32, from_value: f32, to_value: f32 },
    Spline(SplineDef),
    Cache(NodeId),
    Interpolated { input: NodeId, cell_size_xz: i32, cell_size_y: i32 },
    Slice { axis: Axis, coordinate: i32, input: NodeId },
    FindTopSurface { density: NodeId, upper_bound: NodeId, lower_bound: i32, cell_height: i32 },
    BlendedNoise { xz_scale: f64, y_scale: f64, xz_factor: f64, y_factor: f64, smear_scale_multiplier: f64 },
    BlendAlpha,
    BlendOffset,
    Beardifier,
    BlendDensity(NodeId),
    DistanceToPoint { point: [i32; 3], metric: DistanceMetric },
    /// `minecraft:end_outer_islands` (`EndIslandFunction`).
    EndIslands,
    /// A type this implementation does not support; compiling it fails with its name.
    Unsupported(String),
}

/// All density functions and noise parameters of a datapack, as one node arena.
#[derive(Default)]
pub struct Graph {
    nodes: Vec<Node>,
    functions: HashMap<String, NodeId>,
    noises: HashMap<String, NormalNoiseParams>,
    zero: Option<NodeId>,
    domain_cache: RefCell<HashMap<NodeId, u8>>,
    range_cache: RefCell<HashMap<NodeId, Interval>>,
}

/// `minecraft:` is implied for unqualified identifiers.
pub fn qualify(id: &str) -> String {
    if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }
}

impl Graph {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    fn push(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        (self.nodes.len() - 1) as NodeId
    }

    fn zero(&mut self) -> NodeId {
        match self.zero {
            Some(z) => z,
            None => {
                let z = self.push(Node::Const(0.0));
                self.zero = Some(z);
                z
            }
        }
    }

    pub fn add_noise(&mut self, id: &str, params: NormalNoiseParams) {
        self.noises.insert(qualify(id), params);
    }

    pub fn noise(&self, id: &str) -> Result<&NormalNoiseParams, Error> {
        self.noises.get(id).ok_or_else(|| Error::UnknownNoise(id.to_string()))
    }

    pub fn noise_ids(&self) -> impl Iterator<Item = &str> {
        self.noises.keys().map(String::as_str)
    }

    /// Registers the `worldgen/density_function` entry `id`.
    pub fn add_function(&mut self, id: &str, json: &Json) -> Result<NodeId, Error> {
        let node = self.parse(json).map_err(|e| e.context(id))?;
        self.functions.insert(qualify(id), node);
        Ok(node)
    }

    pub fn function(&self, id: &str) -> Option<NodeId> {
        self.functions.get(id).copied()
    }

    /// Follows references to the node they name.
    pub fn resolve(&self, mut id: NodeId) -> Result<NodeId, Error> {
        for _ in 0..64 {
            match self.node(id) {
                Node::Ref(name) => {
                    id = self.function(name).ok_or_else(|| Error::UnknownFunction(name.clone()))?;
                }
                _ => return Ok(id),
            }
        }
        Err(Error::Invalid("density function reference cycle".into()))
    }

    /// Parses an inline density function (`DensityFunction.CODEC`): a number is a constant,
    /// a string a registry reference, an object a typed function.
    pub fn parse(&mut self, json: &Json) -> Result<NodeId, Error> {
        match json {
            Json::Number(_) => {
                let v = json.as_f32().ok_or_else(|| Error::Invalid(format!("bad number {json:?}")))?;
                if !(-1_000_000.0..=1_000_000.0).contains(&v) {
                    return Err(Error::Invalid(format!("constant {v} out of range")));
                }
                Ok(self.push(Node::Const(v)))
            }
            Json::String(s) => Ok(self.push(Node::Ref(qualify(s)))),
            Json::Object(_) => self.parse_typed(json),
            _ => Err(Error::Invalid(format!("not a density function: {json:?}"))),
        }
    }

    fn parse_typed(&mut self, json: &Json) -> Result<NodeId, Error> {
        let ty = json.get("type").and_then(Json::as_str).ok_or_else(|| Error::Invalid("missing type".into()))?;
        let ty = qualify(ty);
        let name = ty.strip_prefix("minecraft:").unwrap_or(&ty);
        if let Some(op) = unary_op(name) {
            let input = self.child(json, "input")?;
            return Ok(self.push(Node::Unary(op, input)));
        }
        let node = match name {
            "constant" => Node::Const(f32_field(json, "value")?),
            "add" | "sub" | "mul" | "div" | "min" | "max" => {
                let op = match name {
                    "add" => BinaryOp::Add,
                    "sub" => BinaryOp::Sub,
                    "mul" => BinaryOp::Mul,
                    "div" => BinaryOp::Div,
                    "min" => BinaryOp::Min,
                    _ => BinaryOp::Max,
                };
                let left = self.child(json, "left")?;
                let right = self.child(json, "right")?;
                Node::Binary(op, left, right)
            }
            "clamp" => {
                let (min, max) = (f32_field(json, "min")?, f32_field(json, "max")?);
                if min > max {
                    return Err(Error::Invalid(format!("clamp min {min} > max {max}")));
                }
                Node::Clamp { input: self.child(json, "input")?, min, max }
            }
            "lerp" => Node::Lerp {
                alpha: self.child(json, "alpha")?,
                first: self.child(json, "first")?,
                second: self.child(json, "second")?,
            },
            "range_choice" => Node::RangeChoice {
                input: self.child(json, "input")?,
                min_inclusive: f32_field(json, "min_inclusive")?,
                max_exclusive: f32_field(json, "max_exclusive")?,
                when_in_range: self.child(json, "when_in_range")?,
                when_out_of_range: self.child(json, "when_out_of_range")?,
            },
            "interval_select" => {
                let thresholds = field(json, "thresholds")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("thresholds must be a list".into()))?
                    .iter()
                    .map(|t| t.as_f32().ok_or_else(|| Error::Invalid("bad threshold".into())))
                    .collect::<Result<Vec<_>, _>>()?;
                let functions = field(json, "functions")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("functions must be a list".into()))?
                    .iter()
                    .map(|f| self.parse(f))
                    .collect::<Result<Vec<_>, _>>()?;
                if functions.len() < 2 || thresholds.len() != functions.len() - 1 {
                    return Err(Error::Invalid("interval_select needs one threshold per gap".into()));
                }
                if thresholds.windows(2).any(|w| w[0] > w[1]) {
                    return Err(Error::Invalid("thresholds must be ordered".into()));
                }
                Node::IntervalSelect { input: self.child(json, "input")?, thresholds, functions }
            }
            "noise" => {
                let mut shift = [0; 3];
                for (s, key) in shift.iter_mut().zip(["shift_x", "shift_y", "shift_z"]) {
                    *s = match json.get(key) {
                        Some(v) => self.parse(v)?,
                        None => self.zero(),
                    };
                }
                Node::Noise {
                    noise: noise_ref(json, "noise")?,
                    xz_scale: f64_field(json, "xz_scale")?,
                    y_scale: f64_field(json, "y_scale")?,
                    shift,
                }
            }
            "shift_a" => Node::ShiftA(noise_ref(json, "noise")?),
            "shift_b" => Node::ShiftB(noise_ref(json, "noise")?),
            "shift" => Node::Shift(noise_ref(json, "noise")?),
            "gradient" => {
                let axis = field(json, "axis")?
                    .as_str()
                    .and_then(Axis::parse)
                    .ok_or_else(|| Error::Invalid("bad gradient axis".into()))?;
                let tiling = match json.get("tiling").and_then(Json::as_str) {
                    None | Some("clamp_to_edge") => Tiling::ClampToEdge,
                    Some("repeat") => Tiling::Repeat,
                    Some("mirrored_repeat") => Tiling::MirroredRepeat,
                    Some(t) => return Err(Error::Invalid(format!("unknown tiling {t}"))),
                };
                let (from_coordinate, to_coordinate) = (i32_field(json, "from_coordinate")?, i32_field(json, "to_coordinate")?);
                if from_coordinate == to_coordinate {
                    return Err(Error::Invalid("from_coordinate cannot be equal to to_coordinate".into()));
                }
                Node::Gradient {
                    axis,
                    tiling,
                    from_coordinate,
                    to_coordinate,
                    from_value: f32_field(json, "from_value")?,
                    to_value: f32_field(json, "to_value")?,
                }
            }
            "spline" => Node::Spline(SplineDef::parse(self, field(json, "spline")?)?),
            "cache" => Node::Cache(self.child(json, "input")?),
            "interpolated" => Node::Interpolated {
                input: self.child(json, "input")?,
                cell_size_xz: positive(i32_field(json, "cell_size_xz")?)?,
                cell_size_y: positive(i32_field(json, "cell_size_y")?)?,
            },
            "slice" => Node::Slice {
                axis: field(json, "axis")?.as_str().and_then(Axis::parse).ok_or_else(|| Error::Invalid("bad slice axis".into()))?,
                coordinate: i32_field(json, "coordinate")?,
                input: self.child(json, "input")?,
            },
            "find_top_surface" => Node::FindTopSurface {
                density: self.child(json, "density")?,
                upper_bound: self.child(json, "upper_bound")?,
                lower_bound: i32_field(json, "lower_bound")?,
                cell_height: positive(i32_field(json, "cell_height")?)?,
            },
            "old_blended_noise" => Node::BlendedNoise {
                xz_scale: f64_field(json, "xz_scale")?,
                y_scale: f64_field(json, "y_scale")?,
                xz_factor: f64_field(json, "xz_factor")?,
                y_factor: f64_field(json, "y_factor")?,
                smear_scale_multiplier: f64_field(json, "smear_scale_multiplier")?,
            },
            "blend_alpha" => Node::BlendAlpha,
            "blend_offset" => Node::BlendOffset,
            "beardifier" => Node::Beardifier,
            "end_outer_islands" => Node::EndIslands,
            "blend_density" => Node::BlendDensity(self.child(json, "input")?),
            "distance_to_point" => {
                let point = field(json, "point")?
                    .as_array()
                    .filter(|p| p.len() == 3)
                    .and_then(|p| Some([p[0].as_i32()?, p[1].as_i32()?, p[2].as_i32()?]))
                    .ok_or_else(|| Error::Invalid("point must be three integers".into()))?;
                let metric = match field(json, "metric")?.as_str() {
                    Some("euclidean") => DistanceMetric::Euclidean,
                    Some("euclidean_squared") => DistanceMetric::EuclideanSquared,
                    Some("manhattan") => DistanceMetric::Manhattan,
                    Some("chebyshev") => DistanceMetric::Chebyshev,
                    m => return Err(Error::Invalid(format!("unknown distance metric {m:?}"))),
                };
                Node::DistanceToPoint { point, metric }
            }
            _ => Node::Unsupported(ty),
        };
        Ok(self.push(node))
    }

    fn child(&mut self, json: &Json, key: &str) -> Result<NodeId, Error> {
        let v = field(json, key)?;
        self.parse(v)
    }

    /// `domainAxes()`: the axes the function's value can depend on.
    pub fn domain(&self, id: NodeId) -> Result<u8, Error> {
        if let Some(&d) = self.domain_cache.borrow().get(&id) {
            return Ok(d);
        }
        let d = match self.node(id) {
            Node::Const(_) => 0,
            Node::Ref(_) => self.domain(self.resolve(id)?)?,
            Node::Unary(_, i) | Node::Clamp { input: i, .. } | Node::Cache(i) | Node::BlendDensity(i) => self.domain(*i)?,
            Node::Interpolated { input, .. } => self.domain(*input)?,
            Node::Binary(_, l, r) => self.domain(*l)? | self.domain(*r)?,
            Node::Lerp { alpha, first, second } => self.domain(*alpha)? | self.domain(*first)? | self.domain(*second)?,
            Node::RangeChoice { input, when_in_range, when_out_of_range, .. } => {
                self.domain(*input)? | self.domain(*when_in_range)? | self.domain(*when_out_of_range)?
            }
            Node::IntervalSelect { input, functions, .. } => {
                let mut d = self.domain(*input)?;
                for f in functions {
                    d |= self.domain(*f)?;
                }
                d
            }
            Node::Noise { xz_scale, y_scale, shift, .. } => {
                let mut d = ALL_AXES;
                if *y_scale == 0.0 {
                    d &= !AXIS_Y;
                }
                if *xz_scale == 0.0 {
                    d &= !(AXIS_X | AXIS_Z);
                }
                d | self.domain(shift[0])? | self.domain(shift[1])? | self.domain(shift[2])?
            }
            Node::ShiftA(_) | Node::ShiftB(_) | Node::BlendAlpha | Node::BlendOffset | Node::EndIslands => AXIS_X | AXIS_Z,
            Node::Shift(_) | Node::BlendedNoise { .. } | Node::Beardifier | Node::DistanceToPoint { .. } => ALL_AXES,
            Node::Unsupported(ty) => return Err(Error::UnsupportedFunction(ty.clone())),
            Node::Gradient { axis, .. } => axis.bit(),
            Node::Spline(s) => {
                let mut d = 0;
                for c in s.coordinates() {
                    d |= self.domain(c)?;
                }
                d
            }
            Node::Slice { axis, input, .. } => self.domain(*input)? & !axis.bit(),
            Node::FindTopSurface { density, upper_bound, .. } => (self.domain(*density)? | self.domain(*upper_bound)?) & !AXIS_Y,
        };
        self.domain_cache.borrow_mut().insert(id, d);
        Ok(d)
    }

    /// `range()`: the value interval vanilla declares for the function.
    pub fn range(&self, id: NodeId) -> Result<Interval, Error> {
        if let Some(&r) = self.range_cache.borrow().get(&id) {
            return Ok(r);
        }
        let r = match self.node(id) {
            Node::Const(v) => Interval::exact(*v),
            Node::Ref(_) => self.range(self.resolve(id)?)?,
            Node::Unary(op, input) => {
                let i = self.range(*input)?;
                match op {
                    UnaryOp::Abs => Interval::abs(i),
                    UnaryOp::Square => Interval::square(i),
                    UnaryOp::Cube => Interval::map_monotonic(i, jm::cube),
                    UnaryOp::Sqrt => Interval::pow_exact(i, 0.5),
                    UnaryOp::HalfNegative => Interval::map_monotonic(i, |x| UnaryOp::leaky(0.5, x)),
                    UnaryOp::QuarterNegative => Interval::map_monotonic(i, |x| UnaryOp::leaky(0.25, x)),
                    UnaryOp::Reciprocal => Interval::reciprocal(i),
                    UnaryOp::Negate => Interval::sub(Interval::exact(0.0), i),
                    UnaryOp::Squeeze => Interval::map_monotonic(i, UnaryOp::squeeze),
                    UnaryOp::Log => Interval::log(i),
                    UnaryOp::Sign => Interval::sign(i),
                }
            }
            Node::Binary(op, l, r) => {
                let (a, b) = (self.range(*l)?, self.range(*r)?);
                match op {
                    BinaryOp::Add => Interval::add(a, b),
                    BinaryOp::Sub => Interval::sub(a, b),
                    BinaryOp::Mul => Interval::mul(a, b),
                    BinaryOp::Div => Interval::div(a, b),
                    BinaryOp::Min => Interval::min_of(a, b),
                    BinaryOp::Max => Interval::max_of(a, b),
                }
            }
            Node::Clamp { input, min, max } => Interval::clamp(self.range(*input)?, *min, *max),
            Node::Lerp { alpha, first, second } => {
                Interval::lerp(self.range(*alpha)?, self.range(*first)?, self.range(*second)?)
            }
            Node::RangeChoice { when_in_range, when_out_of_range, .. } => {
                Interval::encapsulating([self.range(*when_in_range)?, self.range(*when_out_of_range)?])
            }
            Node::IntervalSelect { functions, .. } => {
                Interval::encapsulating(functions.iter().map(|f| self.range(*f)).collect::<Result<Vec<_>, _>>()?)
            }
            Node::Noise { noise, .. } => self.noise(noise)?.range(),
            Node::ShiftA(n) | Node::ShiftB(n) | Node::Shift(n) => {
                Interval::mul(self.noise(n)?.range(), Interval::exact(4.0))
            }
            Node::Gradient { from_value, to_value, .. } => Interval::encapsulating_values(*from_value, *to_value),
            Node::Spline(s) => s.range(self)?,
            Node::Cache(i) | Node::BlendDensity(i) => self.range(*i)?,
            Node::Interpolated { input, .. } | Node::Slice { input, .. } => self.range(*input)?,
            Node::FindTopSurface { upper_bound, lower_bound, .. } => {
                let lower = *lower_bound as f32;
                Interval::of(lower, jm::max(lower, self.range(*upper_bound)?.max()))
            }
            Node::BlendedNoise { y_scale, smear_scale_multiplier, .. } => {
                blended_fbm_range(-15, 684.412 * y_scale * smear_scale_multiplier, 0.999_984_741_210_937_5)
            }
            Node::BlendAlpha => Interval::of(0.0, 1.0),
            Node::BlendOffset | Node::Beardifier => Interval::INFINITE,
            Node::DistanceToPoint { .. } => Interval::of(0.0, f32::INFINITY),
            Node::EndIslands => Interval::of(-0.84375, 0.5625),
            Node::Unsupported(ty) => return Err(Error::UnsupportedFunction(ty.clone())),
        };
        self.range_cache.borrow_mut().insert(id, r);
        Ok(r)
    }
}

pub(crate) fn field<'a>(json: &'a Json, key: &str) -> Result<&'a Json, Error> {
    json.get(key).ok_or_else(|| Error::Invalid(format!("missing field {key}")))
}

pub(crate) fn f32_field(json: &Json, key: &str) -> Result<f32, Error> {
    field(json, key)?.as_f32().ok_or_else(|| Error::Invalid(format!("{key} must be a number")))
}

pub(crate) fn f64_field(json: &Json, key: &str) -> Result<f64, Error> {
    field(json, key)?.as_f64().ok_or_else(|| Error::Invalid(format!("{key} must be a number")))
}

pub(crate) fn i32_field(json: &Json, key: &str) -> Result<i32, Error> {
    field(json, key)?.as_i32().ok_or_else(|| Error::Invalid(format!("{key} must be an integer")))
}

fn unary_op(name: &str) -> Option<UnaryOp> {
    Some(match name {
        "abs" => UnaryOp::Abs,
        "square" => UnaryOp::Square,
        "cube" => UnaryOp::Cube,
        "sqrt" => UnaryOp::Sqrt,
        "half_negative" => UnaryOp::HalfNegative,
        "quarter_negative" => UnaryOp::QuarterNegative,
        "reciprocal" => UnaryOp::Reciprocal,
        "negate" => UnaryOp::Negate,
        "squeeze" => UnaryOp::Squeeze,
        "log" => UnaryOp::Log,
        "sign" => UnaryOp::Sign,
        _ => return None,
    })
}

fn positive(v: i32) -> Result<i32, Error> {
    if v > 0 { Ok(v) } else { Err(Error::Invalid(format!("{v} must be positive"))) }
}

fn noise_ref(json: &Json, key: &str) -> Result<String, Error> {
    match field(json, key)? {
        Json::String(s) => Ok(qualify(s)),
        _ => Err(Error::UnsupportedFunction("inline noise parameters".into())),
    }
}
