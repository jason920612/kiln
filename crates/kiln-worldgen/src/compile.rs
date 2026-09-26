//! Density function compilation, reproducing `DensityFunctionCompiler`.
//!
//! Vanilla rewrites the tree before compiling it: references are inlined, every `cache` is
//! replaced by a shared prepared cache whose input is compiled as a root, and wherever a
//! child depends on fewer axes than its parent it is wrapped in `slice` nodes pinning the
//! missing axes to 0 (so a volume evaluates it on a flattened sub-volume). Sampler selection
//! then specializes on constant operands and on declared value ranges. All of this changes
//! floating-point results in edge cases, so it is mirrored step by step.

use crate::Error;
use crate::function::{ALL_AXES, AXIS_X, AXIS_Y, AXIS_Z, Axis, BinaryOp, Graph, Node, NodeId, Tiling};
use crate::noise::{NoiseStack, blended_fbm};
use crate::sampler::{Gradient, GradientKind, Sampler, SamplerRef};
use crate::spline::{CompiledSpline, SplineDef};
use kiln_javamath::random::{LegacyRandom, PositionalRandomFactory, WorldgenRandom};
use std::collections::HashMap;
use std::sync::Arc;

const BLENDED_NOISE_SEED: &str = "minecraft:terrain";

/// Seed-dependent inputs of compilation (vanilla's `CompileContext`).
pub trait NoiseSource {
    fn noise(&mut self, id: &str) -> Result<Arc<NoiseStack>, Error>;
    /// `CompileContext.createRandom(id)`.
    fn random(&mut self, id: &str) -> WorldgenRandom;
}

/// A node after the rewrite passes: inlined, possibly a prepared cache, possibly wrapped in
/// axis slices.
enum Rewritten {
    Node(NodeId),
    Cache(NodeId),
    Slice { axis: Axis, coordinate: i32, input: Box<Rewritten> },
}

pub struct Compiler<'a, S: NoiseSource> {
    graph: &'a Graph,
    source: &'a mut S,
    built: HashMap<NodeId, SamplerRef>,
    caches: HashMap<NodeId, SamplerRef>,
}

impl<'a, S: NoiseSource> Compiler<'a, S> {
    pub fn new(graph: &'a Graph, source: &'a mut S) -> Self {
        Self { graph, source, built: HashMap::new(), caches: HashMap::new() }
    }

    /// Compiles a top-level function (`DensityFunctionCompiler.getSampler`).
    pub fn compile(&mut self, root: NodeId) -> Result<SamplerRef, Error> {
        let rw = self.rewrite(root, ALL_AXES)?;
        self.compile_rewritten(&rw)
    }

    fn rewrite(&self, id: NodeId, parent_axes: u8) -> Result<Rewritten, Error> {
        let id = self.graph.resolve(id)?;
        let node = self.graph.node(id);
        if matches!(node, Node::Const(_) | Node::Gradient { .. }) {
            return Ok(Rewritten::Node(id));
        }
        let axes = self.graph.domain(id)?;
        let base = match node {
            Node::Cache(input) => Rewritten::Cache(*input),
            Node::Slice { axis, coordinate, input } => {
                Rewritten::Slice { axis: *axis, coordinate: *coordinate, input: Box::new(self.rewrite(*input, axes)?) }
            }
            _ => Rewritten::Node(id),
        };
        if axes == parent_axes {
            return Ok(base);
        }
        let mut missing = parent_axes & !axes;
        let mut f = &base;
        while let Rewritten::Slice { axis, input, .. } = f {
            missing &= !axis.bit();
            f = input;
        }
        let mut out = base;
        for (bit, axis) in [(AXIS_X, Axis::X), (AXIS_Z, Axis::Z), (AXIS_Y, Axis::Y)] {
            if missing & bit != 0 {
                out = Rewritten::Slice { axis, coordinate: 0, input: Box::new(out) };
            }
        }
        Ok(out)
    }

    fn compile_rewritten(&mut self, rw: &Rewritten) -> Result<SamplerRef, Error> {
        match rw {
            Rewritten::Node(id) => self.build(*id),
            Rewritten::Cache(input) => {
                let input = self.graph.resolve(*input)?;
                if let Some(s) = self.caches.get(&input) {
                    return Ok(s.clone());
                }
                let inner = self.rewrite(input, ALL_AXES)?;
                let s = self.compile_rewritten(&inner)?;
                self.caches.insert(input, s.clone());
                Ok(s)
            }
            Rewritten::Slice { axis, coordinate, input } => {
                if let Rewritten::Slice { axis: inner_axis, coordinate: inner_coordinate, input: inner } = input.as_ref() {
                    let pair = match (axis, inner_axis) {
                        (Axis::X, Axis::Z) => Some((*coordinate, *inner_coordinate)),
                        (Axis::Z, Axis::X) => Some((*inner_coordinate, *coordinate)),
                        _ => None,
                    };
                    if let Some((x, z)) = pair {
                        let s = self.compile_rewritten(inner)?;
                        return Ok(Arc::new(Sampler::SliceXz(s, x, z)));
                    }
                }
                let s = self.compile_rewritten(input)?;
                Ok(Arc::new(match axis {
                    Axis::X => Sampler::SliceX(s, *coordinate),
                    Axis::Y => Sampler::SliceY(s, *coordinate),
                    Axis::Z => Sampler::SliceZ(s, *coordinate),
                }))
            }
        }
    }

    /// A child compiled under its parent's axes.
    fn child(&mut self, parent: NodeId, child: NodeId) -> Result<SamplerRef, Error> {
        let axes = self.graph.domain(parent)?;
        let rw = self.rewrite(child, axes)?;
        self.compile_rewritten(&rw)
    }

    fn constant(&self, id: NodeId) -> Result<Option<f32>, Error> {
        Ok(match self.graph.node(self.graph.resolve(id)?) {
            Node::Const(v) => Some(*v),
            _ => None,
        })
    }

    fn build(&mut self, id: NodeId) -> Result<SamplerRef, Error> {
        if let Some(s) = self.built.get(&id) {
            return Ok(s.clone());
        }
        let s = self.build_node(id)?;
        self.built.insert(id, s.clone());
        Ok(s)
    }

    fn build_node(&mut self, id: NodeId) -> Result<SamplerRef, Error> {
        let graph = self.graph;
        Ok(Arc::new(match graph.node(id) {
            Node::Const(v) => Sampler::Const(*v),
            Node::Ref(_) | Node::Cache(_) | Node::Slice { .. } => unreachable!("handled by the rewrite"),
            Node::Unary(op, input) => Sampler::Unary(*op, self.child(id, *input)?),
            Node::Binary(op, l, r) => return self.binary(id, *op, *l, *r),
            Node::Clamp { input, min, max } => Sampler::Clamp(self.child(id, *input)?, *min, *max),
            Node::Lerp { alpha, first, second } => {
                let a = self.child(id, *alpha)?;
                let f = self.child(id, *first)?;
                let g = self.child(id, *second)?;
                if let Some(c) = self.constant(*first)? {
                    Sampler::LerpConstFirst(a, c, g)
                } else if let Some(c) = self.constant(*second)? {
                    Sampler::LerpConstSecond(a, f, c)
                } else {
                    Sampler::Lerp(a, f, g)
                }
            }
            Node::RangeChoice { input, min_inclusive, max_exclusive, when_in_range, when_out_of_range } => {
                let input = self.child(id, *input)?;
                let (min, max) = (*min_inclusive, *max_exclusive);
                match (self.constant(*when_in_range)?, self.constant(*when_out_of_range)?) {
                    (Some(a), Some(b)) => Sampler::RangeChoiceConst { input, min, max, in_range: a, out_of_range: b },
                    _ => Sampler::RangeChoice {
                        input,
                        min,
                        max,
                        in_range: self.child(id, *when_in_range)?,
                        out_of_range: self.child(id, *when_out_of_range)?,
                    },
                }
            }
            Node::IntervalSelect { input, thresholds, functions } => {
                let input = self.child(id, *input)?;
                if thresholds.len() == 1 {
                    Sampler::SelectOne {
                        input,
                        threshold: thresholds[0],
                        below: self.child(id, functions[0])?,
                        above: self.child(id, *functions.last().unwrap())?,
                    }
                } else {
                    let functions = functions.iter().map(|f| self.child(id, *f)).collect::<Result<Vec<_>, _>>()?;
                    Sampler::Select { input, thresholds: thresholds.clone(), functions }
                }
            }
            Node::Noise { noise, xz_scale, y_scale, shift } => {
                let stack = self.source.noise(noise)?;
                let is_zero = |c: Option<f32>| c.is_some_and(|v| v.to_bits() == 0);
                let zero = [self.constant(shift[0])?, self.constant(shift[1])?, self.constant(shift[2])?].map(is_zero);
                let (xz_scale, y_scale) = (*xz_scale, *y_scale);
                if zero == [true; 3] {
                    Sampler::Noise { noise: stack, xz_scale, y_scale }
                } else {
                    let shift_x = self.child(id, shift[0])?;
                    let shift_z = self.child(id, shift[2])?;
                    if zero[1] {
                        Sampler::ShiftedXz { shift_x, shift_z, noise: stack, xz_scale, y_scale }
                    } else {
                        let shift_y = self.child(id, shift[1])?;
                        Sampler::ShiftedXyz { shift_x, shift_y, shift_z, noise: stack, xz_scale, y_scale }
                    }
                }
            }
            Node::ShiftA(noise) => {
                let noise = Arc::new(Sampler::Noise { noise: self.source.noise(noise)?, xz_scale: 0.25, y_scale: 0.0 });
                Sampler::ConstMul(noise, 4.0)
            }
            Node::ShiftB(noise) => Sampler::ShiftB(self.source.noise(noise)?),
            Node::Shift(noise) => {
                let noise = Arc::new(Sampler::Noise { noise: self.source.noise(noise)?, xz_scale: 0.25, y_scale: 0.25 });
                Sampler::ConstMul(noise, 4.0)
            }
            Node::Gradient { axis, tiling, from_coordinate, to_coordinate, from_value, to_value } => {
                let range = to_coordinate.wrapping_sub(*from_coordinate);
                let kind = match tiling {
                    Tiling::ClampToEdge => GradientKind::Clamped {
                        min: (*from_coordinate).min(*to_coordinate),
                        max: (*from_coordinate).max(*to_coordinate),
                    },
                    Tiling::Repeat => GradientKind::Repeat { range },
                    Tiling::MirroredRepeat => GradientKind::MirroredRepeat { range },
                };
                Sampler::Gradient(Gradient {
                    axis: *axis,
                    kind,
                    from_coordinate: *from_coordinate,
                    from_value: *from_value,
                    factor: (to_value - from_value) / range as f32,
                })
            }
            Node::Spline(def) => {
                let mut coordinates = Vec::new();
                let mut index = HashMap::new();
                let spline = self.spline(id, def, &mut coordinates, &mut index)?;
                Sampler::Spline { spline, coordinates }
            }
            Node::Interpolated { input, cell_size_xz, cell_size_y } => Sampler::Interpolated {
                input: self.child(id, *input)?,
                cell_xz: *cell_size_xz,
                cell_y: *cell_size_y,
                inv_xz: 1.0 / *cell_size_xz as f32,
                inv_y: 1.0 / *cell_size_y as f32,
            },
            Node::FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                let inner = Sampler::FindTopSurface {
                    density: self.child(id, *density)?,
                    upper_bound: self.child(id, *upper_bound)?,
                    lower_bound: *lower_bound,
                    cell_height: *cell_height,
                };
                Sampler::SliceY(Arc::new(inner), 0)
            }
            Node::BlendedNoise { xz_scale, y_scale, xz_factor, y_factor, smear_scale_multiplier } => {
                let xz_mul = 684.412 * xz_scale;
                let y_mul = 684.412 * y_scale;
                let smear = y_mul * smear_scale_multiplier;
                let mut random = self.source.random(BLENDED_NOISE_SEED);
                let min = Arc::new(blended_fbm(&mut random, -15, smear, 0.999_984_741_210_937_5));
                let max = Arc::new(blended_fbm(&mut random, -15, smear, 0.999_984_741_210_937_5));
                let main = Arc::new(blended_fbm(&mut random, -7, smear / y_factor, 12.75));
                let noise = |stack, xz_scale, y_scale| Arc::new(Sampler::Noise { noise: stack, xz_scale, y_scale });
                let main = noise(main, xz_mul / xz_factor, y_mul / y_factor);
                let alpha = Arc::new(Sampler::Clamp(Arc::new(Sampler::ConstAdd(main, 0.5)), 0.0, 1.0));
                Sampler::Lerp(alpha, noise(min, xz_mul, y_mul), noise(max, xz_mul, y_mul))
            }
            Node::BlendAlpha => Sampler::Const(1.0),
            Node::BlendOffset | Node::Beardifier => Sampler::Const(0.0),
            Node::BlendDensity(input) => return self.child(id, *input),
            Node::DistanceToPoint { point, metric } => Sampler::DistanceToPoint { point: *point, metric: *metric },
            Node::Unsupported(ty) => return Err(Error::UnsupportedFunction(ty.clone())),
        }))
    }

    fn binary(&mut self, id: NodeId, op: BinaryOp, l: NodeId, r: NodeId) -> Result<SamplerRef, Error> {
        let left = self.child(id, l)?;
        let right = self.child(id, r)?;
        let (lc, rc) = (self.constant(l)?, self.constant(r)?);
        Ok(Arc::new(match op {
            BinaryOp::Add => match (lc, rc) {
                (Some(c), _) => Sampler::ConstAdd(right, c),
                (_, Some(c)) => Sampler::ConstAdd(left, c),
                _ => Sampler::Add(left, right),
            },
            BinaryOp::Sub => match (lc, rc) {
                (Some(c), _) => Sampler::ConstSub(c, right),
                (_, Some(c)) => Sampler::ConstAdd(left, -c),
                _ => Sampler::Sub(left, right),
            },
            BinaryOp::Mul => match (lc, rc) {
                (Some(c), _) => Sampler::ConstMul(right, c),
                (_, Some(c)) => Sampler::ConstMul(left, c),
                _ => Sampler::Mul(left, right),
            },
            BinaryOp::Div => match (lc, rc) {
                (Some(c), _) => Sampler::ConstDiv(c, right),
                (_, Some(c)) => Sampler::ConstMul(left, 1.0 / c),
                _ => Sampler::Div(left, right),
            },
            BinaryOp::Min => {
                let (a, b) = (self.graph.range(l)?, self.graph.range(r)?);
                if a.max() < b.min() {
                    return Ok(left);
                }
                if b.max() < a.min() {
                    return Ok(right);
                }
                match (lc, rc) {
                    (Some(c), _) => Sampler::ConstMin(right, c),
                    (_, Some(c)) => Sampler::ConstMin(left, c),
                    _ => Sampler::Min(left, right, b.min()),
                }
            }
            BinaryOp::Max => {
                let (a, b) = (self.graph.range(l)?, self.graph.range(r)?);
                if a.min() > b.max() {
                    return Ok(left);
                }
                if b.min() > a.max() {
                    return Ok(right);
                }
                match (lc, rc) {
                    (Some(c), _) => Sampler::ConstMax(right, c),
                    (_, Some(c)) => Sampler::ConstMax(left, c),
                    _ => Sampler::Max(left, right, b.max()),
                }
            }
        }))
    }

    fn spline(
        &mut self,
        owner: NodeId,
        def: &SplineDef,
        coordinates: &mut Vec<SamplerRef>,
        index: &mut HashMap<NodeId, usize>,
    ) -> Result<CompiledSpline, Error> {
        let (coordinate, points) = match def {
            SplineDef::Constant(v) => return Ok(CompiledSpline::Constant(*v)),
            SplineDef::Multipoint { coordinate, points } => (*coordinate, points),
        };
        let key = self.graph.resolve(coordinate)?;
        let c = match index.get(&key) {
            Some(&i) => i,
            None => {
                coordinates.push(self.child(owner, coordinate)?);
                index.insert(key, coordinates.len() - 1);
                coordinates.len() - 1
            }
        };
        let values = points
            .iter()
            .map(|p| self.spline(owner, &p.value, coordinates, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CompiledSpline::Multipoint {
            coordinate: c,
            locations: points.iter().map(|p| p.location).collect(),
            values,
            derivatives: points.iter().map(|p| p.derivative).collect(),
        })
    }
}

/// The random source vanilla hands `old_blended_noise`: named from the world's factory, or
/// for legacy worlds the plain LCG seeded with the world seed.
pub fn compile_random(factory: &PositionalRandomFactory, legacy_seed: Option<i64>, id: &str) -> WorldgenRandom {
    match legacy_seed {
        Some(seed) if id == BLENDED_NOISE_SEED => WorldgenRandom::Legacy(LegacyRandom::new(seed)),
        _ => factory.from_hash_of(id),
    }
}
