//! Compiled density functions and their two evaluation modes.
//!
//! `point` answers one block position (vanilla's `sampleValue`); `fill` evaluates a whole
//! [`Volume`] at once (`sampleVolume`), each node processing the full batch before its parent,
//! so the work vectorizes along positions. The modes deliberately differ where vanilla's do:
//! short-circuits, operand order and noise accumulation round differently, so each is ported
//! separately and both are bit-exact against their Java counterpart.

use crate::function::{Axis, DistanceMetric, UnaryOp};
use crate::noise::NoiseStack;
use crate::spline::CompiledSpline;
use crate::volume::Volume;
use kiln_javamath::math as jm;
use std::sync::Arc;

pub type SamplerRef = Arc<Sampler>;

#[derive(Debug)]
pub enum GradientKind {
    Clamped { min: i32, max: i32 },
    Repeat { range: i32 },
    MirroredRepeat { range: i32 },
}

#[derive(Debug)]
pub struct Gradient {
    pub axis: Axis,
    pub kind: GradientKind,
    pub from_coordinate: i32,
    pub from_value: f32,
    pub factor: f32,
}

impl Gradient {
    fn compute(&self, c: i32) -> f32 {
        let from = self.from_coordinate;
        match self.kind {
            GradientKind::Clamped { min, max } => {
                self.from_value + (c.max(min).min(max).wrapping_sub(from)) as f32 * self.factor
            }
            GradientKind::Repeat { range } => {
                self.from_value + jm::floor_mod(c.wrapping_sub(from), range) as f32 * self.factor
            }
            GradientKind::MirroredRepeat { range } => {
                let d = c.wrapping_sub(from);
                let q = jm::floor_div(d, range);
                let r = d.wrapping_sub(q.wrapping_mul(range));
                if q & 1 == 0 {
                    self.from_value + r as f32 * self.factor
                } else {
                    self.from_value + range.wrapping_sub(r) as f32 * self.factor
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum Sampler {
    Const(f32),
    /// A prepared `cache`, shared by every use of the same function (`id` is unique within
    /// one compilation). A volume evaluates it once and reuses the buffer, as vanilla's
    /// caching contexts do; functions are pure, so this never changes a value.
    Cache { id: u32, input: SamplerRef },
    Add(SamplerRef, SamplerRef),
    ConstAdd(SamplerRef, f32),
    Sub(SamplerRef, SamplerRef),
    ConstSub(f32, SamplerRef),
    Mul(SamplerRef, SamplerRef),
    ConstMul(SamplerRef, f32),
    Div(SamplerRef, SamplerRef),
    ConstDiv(f32, SamplerRef),
    /// `min(left, right)`; per position, `left <= right_min` skips `right`.
    Min(SamplerRef, SamplerRef, f32),
    ConstMin(SamplerRef, f32),
    /// `max(left, right)`; per position, `left >= right_max` skips `right`.
    Max(SamplerRef, SamplerRef, f32),
    ConstMax(SamplerRef, f32),
    Unary(UnaryOp, SamplerRef),
    Clamp(SamplerRef, f32, f32),
    Lerp(SamplerRef, SamplerRef, SamplerRef),
    LerpConstFirst(SamplerRef, f32, SamplerRef),
    LerpConstSecond(SamplerRef, SamplerRef, f32),
    RangeChoice { input: SamplerRef, min: f32, max: f32, in_range: SamplerRef, out_of_range: SamplerRef },
    RangeChoiceConst { input: SamplerRef, min: f32, max: f32, in_range: f32, out_of_range: f32 },
    SelectOne { input: SamplerRef, threshold: f32, below: SamplerRef, above: SamplerRef },
    Select { input: SamplerRef, thresholds: Vec<f32>, functions: Vec<SamplerRef> },
    Spline { spline: CompiledSpline, coordinates: Vec<SamplerRef> },
    Interpolated { input: SamplerRef, cell_xz: i32, cell_y: i32, inv_xz: f32, inv_y: f32 },
    SliceX(SamplerRef, i32),
    SliceY(SamplerRef, i32),
    SliceZ(SamplerRef, i32),
    SliceXz(SamplerRef, i32, i32),
    FindTopSurface { density: SamplerRef, upper_bound: SamplerRef, lower_bound: i32, cell_height: i32 },
    Noise { noise: Arc<NoiseStack>, xz_scale: f64, y_scale: f64 },
    ShiftedXz { shift_x: SamplerRef, shift_z: SamplerRef, noise: Arc<NoiseStack>, xz_scale: f64, y_scale: f64 },
    ShiftedXyz {
        shift_x: SamplerRef,
        shift_y: SamplerRef,
        shift_z: SamplerRef,
        noise: Arc<NoiseStack>,
        xz_scale: f64,
        y_scale: f64,
    },
    ShiftB(Arc<NoiseStack>),
    Gradient(Gradient),
    DistanceToPoint { point: [i32; 3], metric: DistanceMetric },
}

/// Recycled buffers for volume evaluation, plus the cache buffers of the evaluation in
/// progress (released when the outermost `fill` returns).
#[derive(Default)]
pub struct Scratch {
    free: Vec<Vec<f32>>,
    depth: u32,
    cached: Vec<(u32, Volume, Vec<f32>)>,
}

impl Scratch {
    fn take(&mut self, len: usize) -> Vec<f32> {
        let mut v = self.free.pop().unwrap_or_default();
        v.clear();
        v.resize(len, 0.0);
        v
    }

    fn give(&mut self, v: Vec<f32>) {
        self.free.push(v);
    }
}

impl Sampler {
    /// Value at one block position.
    pub fn point(&self, s: &mut Scratch, x: i32, y: i32, z: i32) -> f32 {
        use Sampler::*;
        match self {
            Const(v) => *v,
            Cache { input, .. } => input.point(s, x, y, z),
            Add(l, r) => l.point(s, x, y, z) + r.point(s, x, y, z),
            ConstAdd(i, c) => i.point(s, x, y, z) + c,
            Sub(l, r) => l.point(s, x, y, z) - r.point(s, x, y, z),
            ConstSub(c, i) => c - i.point(s, x, y, z),
            Mul(l, r) => {
                let a = l.point(s, x, y, z);
                if a == 0.0 { 0.0 } else { a * r.point(s, x, y, z) }
            }
            ConstMul(i, c) => i.point(s, x, y, z) * c,
            Div(l, r) => {
                let a = l.point(s, x, y, z);
                if a == 0.0 { 0.0 } else { a / r.point(s, x, y, z) }
            }
            ConstDiv(c, i) => c / i.point(s, x, y, z),
            Min(l, r, right_min) => {
                let a = l.point(s, x, y, z);
                if a <= *right_min { a } else { jm::min(a, r.point(s, x, y, z)) }
            }
            ConstMin(i, c) => jm::min(i.point(s, x, y, z), *c),
            Max(l, r, right_max) => {
                let a = l.point(s, x, y, z);
                if a >= *right_max { a } else { jm::max(a, r.point(s, x, y, z)) }
            }
            ConstMax(i, c) => jm::max(i.point(s, x, y, z), *c),
            Unary(op, i) => op.apply(i.point(s, x, y, z)),
            Clamp(i, lo, hi) => jm::clamp(i.point(s, x, y, z), *lo, *hi),
            Lerp(a, f, g) => {
                let t = a.point(s, x, y, z);
                if t == 0.0 {
                    f.point(s, x, y, z)
                } else if t == 1.0 {
                    g.point(s, x, y, z)
                } else {
                    jm::lerp(t, f.point(s, x, y, z), g.point(s, x, y, z))
                }
            }
            LerpConstFirst(a, c, g) => {
                let t = a.point(s, x, y, z);
                if t == 0.0 {
                    *c
                } else if t == 1.0 {
                    g.point(s, x, y, z)
                } else {
                    jm::lerp(t, *c, g.point(s, x, y, z))
                }
            }
            LerpConstSecond(a, f, c) => {
                let t = a.point(s, x, y, z);
                if t == 0.0 {
                    f.point(s, x, y, z)
                } else if t == 1.0 {
                    *c
                } else {
                    jm::lerp(t, f.point(s, x, y, z), *c)
                }
            }
            RangeChoice { input, min, max, in_range, out_of_range } => {
                let v = input.point(s, x, y, z);
                if v >= *min && v < *max { in_range.point(s, x, y, z) } else { out_of_range.point(s, x, y, z) }
            }
            RangeChoiceConst { input, min, max, in_range, out_of_range } => {
                let v = input.point(s, x, y, z);
                if v >= *min && v < *max { *in_range } else { *out_of_range }
            }
            SelectOne { input, threshold, below, above } => {
                if input.point(s, x, y, z) < *threshold { below.point(s, x, y, z) } else { above.point(s, x, y, z) }
            }
            Select { input, thresholds, functions } => {
                let v = input.point(s, x, y, z);
                functions[select_index(thresholds, v)].point(s, x, y, z)
            }
            Spline { spline, coordinates } => {
                let mut cached = vec![f32::NAN; coordinates.len()];
                spline.sample(&mut |c| {
                    if !cached[c].is_nan() {
                        return cached[c];
                    }
                    let v = coordinates[c].point(s, x, y, z);
                    cached[c] = v;
                    v
                })
            }
            Interpolated { input, cell_xz, cell_y, .. } => interpolated_point(input, *cell_xz, *cell_y, s, x, y, z),
            SliceX(i, c) => i.point(s, *c, y, z),
            SliceY(i, c) => i.point(s, x, *c, z),
            SliceZ(i, c) => i.point(s, x, y, *c),
            SliceXz(i, cx, cz) => i.point(s, *cx, y, *cz),
            FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                let upper = upper_bound.point(s, x, y, z);
                find_surface(density, *lower_bound, *cell_height, s, x, z, upper) as f32
            }
            Noise { noise, xz_scale, y_scale } => {
                noise.get3(x as f64 * xz_scale, y as f64 * y_scale, z as f64 * xz_scale)
            }
            ShiftedXz { shift_x, shift_z, noise, xz_scale, y_scale } => {
                let nx = x as f64 * xz_scale + shift_x.point(s, x, y, z) as f64;
                let ny = y as f64 * y_scale;
                let nz = z as f64 * xz_scale + shift_z.point(s, x, y, z) as f64;
                noise.get3(nx, ny, nz)
            }
            ShiftedXyz { shift_x, shift_y, shift_z, noise, xz_scale, y_scale } => {
                let nx = x as f64 * xz_scale + shift_x.point(s, x, y, z) as f64;
                let ny = y as f64 * y_scale + shift_y.point(s, x, y, z) as f64;
                let nz = z as f64 * xz_scale + shift_z.point(s, x, y, z) as f64;
                noise.get3(nx, ny, nz)
            }
            ShiftB(noise) => noise.get3(z as f64 * 0.25, x as f64 * 0.25, 0.0) * 4.0,
            Gradient(g) => g.compute(g.axis.choose(x, y, z)),
            DistanceToPoint { point, metric } => metric.compute(
                point[0].wrapping_sub(x) as f32,
                point[1].wrapping_sub(y) as f32,
                point[2].wrapping_sub(z) as f32,
            ),
        }
    }

    /// Values for every position of `vol`, written to `out` in the volume's buffer order.
    pub fn fill(&self, s: &mut Scratch, vol: &Volume, out: &mut [f32]) {
        debug_assert_eq!(out.len(), vol.len());
        s.depth += 1;
        self.fill_node(s, vol, out);
        s.depth -= 1;
        if s.depth == 0 {
            let cached = std::mem::take(&mut s.cached);
            s.free.extend(cached.into_iter().map(|(_, _, buf)| buf));
        }
    }

    fn fill_node(&self, s: &mut Scratch, vol: &Volume, out: &mut [f32]) {
        use Sampler::*;
        match self {
            Const(v) => out.fill(*v),
            Cache { id, input } => {
                if let Some((_, _, buf)) = s.cached.iter().find(|(i, v, _)| i == id && v == vol) {
                    out.copy_from_slice(buf);
                    return;
                }
                input.fill(s, vol, out);
                let mut buf = s.take(0);
                buf.extend_from_slice(out);
                s.cached.push((*id, *vol, buf));
            }
            Add(l, r) => binary(s, vol, out, l, r, |a, b| a + b),
            ConstAdd(i, c) => map(s, vol, out, i, |a| a + c),
            Sub(l, r) => binary(s, vol, out, l, r, |a, b| a + -b),
            ConstSub(c, i) => map(s, vol, out, i, |a| c - a),
            Mul(l, r) => binary(s, vol, out, l, r, |a, b| a * b),
            ConstMul(i, c) => map(s, vol, out, i, |a| a * c),
            Div(l, r) => binary(s, vol, out, l, r, |a, b| a / b),
            ConstDiv(c, i) => map(s, vol, out, i, |a| c / a),
            Min(l, r, _) => binary(s, vol, out, l, r, |a, b| if b < a { b } else { a }),
            ConstMin(i, c) => map(s, vol, out, i, |a| if *c < a { *c } else { a }),
            Max(l, r, _) => binary(s, vol, out, l, r, |a, b| if b > a { b } else { a }),
            ConstMax(i, c) => map(s, vol, out, i, |a| if *c > a { *c } else { a }),
            Unary(op, i) => map(s, vol, out, i, |a| op.apply(a)),
            Clamp(i, lo, hi) => map(s, vol, out, i, |a| jm::clamp(a, *lo, *hi)),
            Lerp(a, f, g) => {
                a.fill(s, vol, out);
                let first = eval(s, vol, f);
                let second = eval(s, vol, g);
                for ((o, &p), &q) in out.iter_mut().zip(&first).zip(&second) {
                    *o = if *o == 0.0 {
                        p
                    } else if *o == 1.0 {
                        q
                    } else {
                        jm::lerp(*o, p, q)
                    };
                }
                s.give(first);
                s.give(second);
            }
            LerpConstFirst(a, c, g) => {
                a.fill(s, vol, out);
                let second = eval(s, vol, g);
                for (o, &q) in out.iter_mut().zip(&second) {
                    *o = if *o == 0.0 {
                        *c
                    } else if *o == 1.0 {
                        q
                    } else {
                        jm::lerp(*o, *c, q)
                    };
                }
                s.give(second);
            }
            LerpConstSecond(a, f, c) => {
                a.fill(s, vol, out);
                let first = eval(s, vol, f);
                for (o, &p) in out.iter_mut().zip(&first) {
                    *o = if *o == 0.0 {
                        p
                    } else if *o == 1.0 {
                        *c
                    } else {
                        jm::lerp(*o, p, *c)
                    };
                }
                s.give(first);
            }
            RangeChoice { input, min, max, in_range, out_of_range } => {
                in_range.fill(s, vol, out);
                let v = eval(s, vol, input);
                let other = eval(s, vol, out_of_range);
                for ((o, &v), &w) in out.iter_mut().zip(&v).zip(&other) {
                    if !(v >= *min && v < *max) {
                        *o = w;
                    }
                }
                s.give(v);
                s.give(other);
            }
            RangeChoiceConst { input, min, max, in_range, out_of_range } => {
                map(s, vol, out, input, |v| if v >= *min && v < *max { *in_range } else { *out_of_range })
            }
            SelectOne { input, threshold, below, above } => {
                input.fill(s, vol, out);
                let b = eval(s, vol, below);
                let a = eval(s, vol, above);
                for ((o, &b), &a) in out.iter_mut().zip(&b).zip(&a) {
                    *o = if *o < *threshold { b } else { a };
                }
                s.give(b);
                s.give(a);
            }
            Select { input, thresholds, functions } => {
                input.fill(s, vol, out);
                let bufs: Vec<Vec<f32>> = functions.iter().map(|f| eval(s, vol, f)).collect();
                for (i, o) in out.iter_mut().enumerate() {
                    *o = bufs[select_index(thresholds, *o)][i];
                }
                for b in bufs {
                    s.give(b);
                }
            }
            Spline { spline, coordinates } => {
                let mut bufs: Vec<Option<Vec<f32>>> = vec![None; coordinates.len()];
                for (i, o) in out.iter_mut().enumerate() {
                    *o = spline.sample(&mut |c| {
                        bufs[c].get_or_insert_with(|| eval(s, vol, &coordinates[c]))[i]
                    });
                }
                for b in bufs.into_iter().flatten() {
                    s.give(b);
                }
            }
            Interpolated { input, cell_xz, cell_y, inv_xz, inv_y } => {
                let cell = Cell { xz: *cell_xz, y: *cell_y, inv_xz: *inv_xz, inv_y: *inv_y };
                cell.fill(input, s, vol, out)
            }
            SliceX(i, x) => {
                if vol.size[0] == 1 && vol.min[0] == *x {
                    return i.fill(s, vol, out);
                }
                let sub = Volume::new([1, vol.size[1], vol.size[2]], [*x, vol.min[1], vol.min[2]], vol.step);
                let tmp = eval(s, &sub, i);
                let mut k = 0;
                for zi in 0..vol.size[2] {
                    for _ in 0..vol.size[0] {
                        for yi in 0..vol.size[1] {
                            out[k] = tmp[sub.index(0, yi, zi)];
                            k += 1;
                        }
                    }
                }
                s.give(tmp);
            }
            SliceY(i, y) => {
                if vol.size[1] == 1 && vol.min[1] == *y {
                    return i.fill(s, vol, out);
                }
                let sub = Volume::new([vol.size[0], 1, vol.size[2]], [vol.min[0], *y, vol.min[2]], vol.step);
                let tmp = eval(s, &sub, i);
                let sy = vol.size[1] as usize;
                for zi in 0..vol.size[2] {
                    for xi in 0..vol.size[0] {
                        let start = vol.index(xi, 0, zi);
                        out[start..start + sy].fill(tmp[sub.index(xi, 0, zi)]);
                    }
                }
                s.give(tmp);
            }
            SliceZ(i, z) => {
                if vol.size[2] == 1 && vol.min[2] == *z {
                    return i.fill(s, vol, out);
                }
                let sub = Volume::new([vol.size[0], vol.size[1], 1], [vol.min[0], vol.min[1], *z], vol.step);
                let tmp = eval(s, &sub, i);
                let mut k = 0;
                for _ in 0..vol.size[2] {
                    for xi in 0..vol.size[0] {
                        for yi in 0..vol.size[1] {
                            out[k] = tmp[sub.index(xi, yi, 0)];
                            k += 1;
                        }
                    }
                }
                s.give(tmp);
            }
            SliceXz(i, x, z) => {
                if vol.size[0] == 1 && vol.size[2] == 1 && vol.min[0] == *x && vol.min[2] == *z {
                    return i.fill(s, vol, out);
                }
                let sub = Volume::new([1, vol.size[1], 1], [*x, vol.min[1], *z], vol.step);
                let tmp = eval(s, &sub, i);
                let sy = vol.size[1] as usize;
                for yi in 0..vol.size[1] {
                    let v = tmp[sub.index(0, yi, 0)];
                    let mut k = vol.index(0, yi, 0);
                    for _ in 0..vol.size[2] {
                        for _ in 0..vol.size[0] {
                            out[k] = v;
                            k += sy;
                        }
                    }
                }
                s.give(tmp);
            }
            FindTopSurface { density, upper_bound, lower_bound, cell_height } => {
                assert_eq!(vol.size[1], 1, "find_top_surface needs a single-layer volume");
                upper_bound.fill(s, vol, out);
                let mut k = 0;
                for zi in 0..vol.size[2] {
                    let bz = vol.block_z(zi);
                    for xi in 0..vol.size[0] {
                        let bx = vol.block_x(xi);
                        out[k] = find_surface(density, *lower_bound, *cell_height, s, bx, bz, out[k]) as f32;
                        k += 1;
                    }
                }
            }
            Noise { noise, xz_scale, y_scale } => {
                out.fill(0.0);
                noise.add_to_volume(out, vol, *xz_scale, *y_scale, 1.0);
            }
            ShiftedXz { shift_x, shift_z, noise, xz_scale, y_scale } => {
                shift_x.fill(s, vol, out);
                let sz = eval(s, vol, shift_z);
                let mut k = 0;
                for zi in 0..vol.size[2] {
                    let bz = vol.block_z(zi) as f64 * xz_scale;
                    for xi in 0..vol.size[0] {
                        let bx = vol.block_x(xi) as f64 * xz_scale;
                        for yi in 0..vol.size[1] {
                            let by = vol.block_y(yi) as f64 * y_scale;
                            out[k] = noise.get3(bx + out[k] as f64, by, bz + sz[k] as f64);
                            k += 1;
                        }
                    }
                }
                s.give(sz);
            }
            ShiftedXyz { shift_x, shift_y, shift_z, noise, xz_scale, y_scale } => {
                shift_x.fill(s, vol, out);
                let sy = eval(s, vol, shift_y);
                let sz = eval(s, vol, shift_z);
                let mut k = 0;
                for zi in 0..vol.size[2] {
                    let bz = vol.block_z(zi) as f64 * xz_scale;
                    for xi in 0..vol.size[0] {
                        let bx = vol.block_x(xi) as f64 * xz_scale;
                        for yi in 0..vol.size[1] {
                            let by = vol.block_y(yi) as f64 * y_scale;
                            out[k] = noise.get3(bx + out[k] as f64, by + sy[k] as f64, bz + sz[k] as f64);
                            k += 1;
                        }
                    }
                }
                s.give(sy);
                s.give(sz);
            }
            ShiftB(noise) => {
                let swapped = Volume::new(
                    [vol.size[2], vol.size[0], 1],
                    [vol.min[2], vol.min[0], 0],
                    [vol.step[2], vol.step[0], 1],
                );
                let mut tmp = s.take(swapped.len());
                noise.add_to_volume(&mut tmp, &swapped, 0.25, 0.25, 4.0);
                let sy = vol.size[1] as usize;
                for zi in 0..vol.size[2] {
                    for xi in 0..vol.size[0] {
                        let start = vol.index(xi, 0, zi);
                        out[start..start + sy].fill(tmp[swapped.index(zi, xi, 0)]);
                    }
                }
                s.give(tmp);
            }
            Gradient(g) => match g.axis {
                Axis::X => {
                    let sy = vol.size[1] as usize;
                    for xi in 0..vol.size[0] {
                        let v = g.compute(vol.block_x(xi));
                        for zi in 0..vol.size[2] {
                            let start = vol.index(xi, 0, zi);
                            out[start..start + sy].fill(v);
                        }
                    }
                }
                Axis::Y => {
                    for yi in 0..vol.size[1] {
                        let v = g.compute(vol.block_y(yi));
                        for zi in 0..vol.size[2] {
                            for xi in 0..vol.size[0] {
                                out[vol.index(xi, yi, zi)] = v;
                            }
                        }
                    }
                }
                Axis::Z => {
                    let plane = (vol.size[0] * vol.size[1]) as usize;
                    for zi in 0..vol.size[2] {
                        let start = vol.index(0, 0, zi);
                        out[start..start + plane].fill(g.compute(vol.block_z(zi)));
                    }
                }
            },
            DistanceToPoint { .. } => {
                for (o, [x, y, z]) in out.iter_mut().zip(vol.positions()) {
                    *o = self.point(s, x, y, z);
                }
            }
        }
    }
}

fn eval(s: &mut Scratch, vol: &Volume, f: &Sampler) -> Vec<f32> {
    let mut buf = s.take(vol.len());
    f.fill(s, vol, &mut buf);
    buf
}

fn map(s: &mut Scratch, vol: &Volume, out: &mut [f32], input: &Sampler, f: impl Fn(f32) -> f32) {
    input.fill(s, vol, out);
    for o in out.iter_mut() {
        *o = f(*o);
    }
}

fn binary(s: &mut Scratch, vol: &Volume, out: &mut [f32], l: &Sampler, r: &Sampler, f: impl Fn(f32, f32) -> f32) {
    l.fill(s, vol, out);
    let right = eval(s, vol, r);
    for (o, &b) in out.iter_mut().zip(&right) {
        *o = f(*o, b);
    }
    s.give(right);
}

/// `IntervalSelectFunction`: the first function whose threshold exceeds `v`, else the last.
fn select_index(thresholds: &[f32], v: f32) -> usize {
    thresholds.iter().position(|&t| v < t).unwrap_or(thresholds.len())
}

/// `FindTopSurfaceFunction.findSurfaceFrom`: scans down in `cell_height` steps from the upper
/// bound for the first positive density.
fn find_surface(density: &Sampler, lower: i32, cell_height: i32, s: &mut Scratch, x: i32, z: i32, upper: f32) -> i32 {
    let start = jm::floor_f32(upper / cell_height as f32).wrapping_mul(cell_height);
    if start <= lower {
        return lower;
    }
    let mut y = start;
    while y >= lower {
        if density.point(s, x, y, z) > 0.0 {
            return y;
        }
        y -= cell_height;
    }
    lower
}

/// `InterpolatedFunction` point path: exact at cell corners, otherwise trilinear over a
/// 2×2×2 corner volume sampled in volume mode.
fn interpolated_point(input: &Sampler, cxz: i32, cy: i32, s: &mut Scratch, x: i32, y: i32, z: i32) -> f32 {
    let (fx, fy, fz) = (jm::floor_mod(x, cxz), jm::floor_mod(y, cy), jm::floor_mod(z, cxz));
    if fx == 0 && fy == 0 && fz == 0 {
        return input.point(s, x, y, z);
    }
    let v = Volume::new([2, 2, 2], [x - fx, y - fy, z - fz], [cxz, cy, cxz]);
    let c = eval(s, &v, input);
    let corners = [
        c[v.index(0, 0, 0)],
        c[v.index(1, 0, 0)],
        c[v.index(0, 1, 0)],
        c[v.index(1, 1, 0)],
        c[v.index(0, 0, 1)],
        c[v.index(1, 0, 1)],
        c[v.index(0, 1, 1)],
        c[v.index(1, 1, 1)],
    ];
    s.give(c);
    jm::lerp3(fx as f32 / cxz as f32, fy as f32 / cy as f32, fz as f32 / cxz as f32, corners)
}

/// `InterpolatedFunction` volume paths.
struct Cell {
    xz: i32,
    y: i32,
    inv_xz: f32,
    inv_y: f32,
}

impl Cell {
    fn fill(&self, input: &Sampler, s: &mut Scratch, vol: &Volume, out: &mut [f32]) {
        let aligned_step = (vol.step[0] == self.xz || vol.size[0] == 1)
            && (vol.step[1] == self.y || vol.size[1] == 1)
            && (vol.step[2] == self.xz || vol.size[2] == 1);
        let aligned_min = jm::floor_mod(vol.min[0], self.xz) == 0
            && jm::floor_mod(vol.min[1], self.y) == 0
            && jm::floor_mod(vol.min[2], self.xz) == 0;
        if aligned_step && aligned_min {
            return input.fill(s, vol, out);
        }
        if vol.step == [1, 1, 1] {
            return self.fill_blocks(input, s, vol, out);
        }
        let full = Volume::blocks(
            [vol.size[0] * vol.step[0], vol.size[1] * vol.step[1], vol.size[2] * vol.step[2]],
            vol.min,
        );
        let tmp = {
            let mut t = s.take(full.len());
            self.fill_blocks(input, s, &full, &mut t);
            t
        };
        for zi in 0..vol.size[2] {
            for xi in 0..vol.size[0] {
                for yi in 0..vol.size[1] {
                    out[vol.index(xi, yi, zi)] = tmp[full.index(xi * vol.step[0], yi * vol.step[1], zi * vol.step[2])];
                }
            }
        }
        s.give(tmp);
    }

    fn fill_blocks(&self, input: &Sampler, s: &mut Scratch, vol: &Volume, out: &mut [f32]) {
        let cell = [self.xz, self.y, self.xz];
        let lo: [i32; 3] = std::array::from_fn(|a| jm::floor_div(vol.min[a], cell[a]));
        let hi: [i32; 3] = std::array::from_fn(|a| jm::floor_div(vol.max_block(a), cell[a]));
        let n: [i32; 3] = std::array::from_fn(|a| hi[a] - lo[a] + 1);
        let corner_size: [i32; 3] =
            std::array::from_fn(|a| if jm::floor_mod(vol.max_block(a), cell[a]) == 0 { n[a] } else { n[a] + 1 });
        let corners = Volume::new(corner_size, std::array::from_fn(|a| lo[a] * cell[a]), cell);
        let c = eval(s, &corners, input);
        for zc in 0..n[2] {
            let zc1 = (zc + 1).min(corners.size[2] - 1);
            for xc in 0..n[0] {
                let xc1 = (xc + 1).min(corners.size[0] - 1);
                let mut c000 = c[corners.index(xc, 0, zc)];
                let mut c100 = c[corners.index(xc1, 0, zc)];
                let mut c001 = c[corners.index(xc, 0, zc1)];
                let mut c101 = c[corners.index(xc1, 0, zc1)];
                for yc in 0..n[1] {
                    let yc1 = (yc + 1).min(corners.size[1] - 1);
                    let c010 = c[corners.index(xc, yc1, zc)];
                    let c110 = c[corners.index(xc1, yc1, zc)];
                    let c011 = c[corners.index(xc, yc1, zc1)];
                    let c111 = c[corners.index(xc1, yc1, zc1)];
                    self.fill_cell(out, vol, &corners, [xc, yc, zc], [c000, c100, c010, c110, c001, c101, c011, c111]);
                    (c000, c100, c001, c101) = (c010, c110, c011, c111);
                }
            }
        }
        s.give(c);
    }

    /// `fillCell`: lerps along z, then x, then steps y incrementally.
    fn fill_cell(&self, out: &mut [f32], vol: &Volume, corners: &Volume, cell: [i32; 3], v: [f32; 8]) {
        let [c000, c100, c010, c110, c001, c101, c011, c111] = v;
        let rx = corners.block_x(cell[0]) - vol.min[0];
        let ry = corners.block_y(cell[1]) - vol.min[1];
        let rz = corners.block_z(cell[2]) - vol.min[2];
        let (x0, y0, z0) = (0.max(-rx), 0.max(-ry), 0.max(-rz));
        let x1 = self.xz.min(vol.size[0] - rx) - 1;
        let y1 = self.y.min(vol.size[1] - ry) - 1;
        let z1 = self.xz.min(vol.size[2] - rz) - 1;
        for zo in z0..=z1 {
            let tz = zo as f32 * self.inv_xz;
            let a = jm::lerp(tz, c000, c001);
            let b = jm::lerp(tz, c010, c011);
            let c = jm::lerp(tz, c100, c101);
            let d = jm::lerp(tz, c110, c111);
            for xo in x0..=x1 {
                let tx = xo as f32 * self.inv_xz;
                let lo = jm::lerp(tx, a, c);
                let hi = jm::lerp(tx, b, d);
                let dy = (hi - lo) * self.inv_y;
                let mut value = lo + dy * y0 as f32;
                let start = vol.index(rx + xo, ry + y0, rz + zo);
                for o in &mut out[start..=start + (y1 - y0) as usize] {
                    *o = value;
                    value += dy;
                }
            }
        }
    }
}
