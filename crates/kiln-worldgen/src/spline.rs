//! `CubicSpline`: nested Hermite splines over density function coordinates, in f32.

use crate::Error;
use crate::function::{Graph, NodeId, f32_field, field};
use crate::interval::Interval;
use crate::json::Json;
use kiln_javamath::math as jm;

#[derive(Clone, Debug)]
pub enum SplineDef {
    Constant(f32),
    Multipoint { coordinate: NodeId, points: Vec<Point> },
}

#[derive(Clone, Debug)]
pub struct Point {
    pub location: f32,
    pub value: SplineDef,
    pub derivative: f32,
}

impl SplineDef {
    pub fn parse(graph: &mut Graph, json: &Json) -> Result<SplineDef, Error> {
        if let Some(v) = json.as_f32() {
            return Ok(SplineDef::Constant(v));
        }
        let coordinate = graph.parse(field(json, "coordinate")?)?;
        let points = field(json, "points")?
            .as_array()
            .ok_or_else(|| Error::Invalid("spline points must be a list".into()))?
            .iter()
            .map(|p| {
                Ok(Point {
                    location: f32_field(p, "location")?,
                    value: SplineDef::parse(graph, field(p, "value")?)?,
                    derivative: f32_field(p, "derivative")?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        if points.is_empty() {
            return Err(Error::Invalid("Cannot create a multipoint spline with no points".into()));
        }
        Ok(SplineDef::Multipoint { coordinate, points })
    }

    /// Every coordinate function, nested splines included.
    pub fn coordinates(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_coordinates(&mut out);
        out
    }

    fn collect_coordinates(&self, out: &mut Vec<NodeId>) {
        if let SplineDef::Multipoint { coordinate, points } = self {
            out.push(*coordinate);
            for p in points {
                p.value.collect_coordinates(out);
            }
        }
    }

    /// `CubicSpline.range()`.
    pub fn range(&self, graph: &Graph) -> Result<Interval, Error> {
        let (coordinate, points) = match self {
            SplineDef::Constant(v) => return Ok(Interval::exact(*v)),
            SplineDef::Multipoint { coordinate, points } => (*coordinate, points),
        };
        let locations: Vec<f32> = points.iter().map(|p| p.location).collect();
        let derivatives: Vec<f32> = points.iter().map(|p| p.derivative).collect();
        let last = points.len() - 1;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let input = graph.range(coordinate)?;
        if input.is_nai() {
            return Ok(input);
        }
        let mut widen = |a: f32, b: f32| {
            lo = jm::min(lo, jm::min(a, b));
            hi = jm::max(hi, jm::max(a, b));
        };
        if input.min() < locations[0] {
            let v = points[0].value.range(graph)?;
            widen(
                linear_extend(input.min(), &locations, v.min(), &derivatives, 0),
                linear_extend(input.min(), &locations, v.max(), &derivatives, 0),
            );
        }
        if input.max() > locations[last] {
            let v = points[last].value.range(graph)?;
            widen(
                linear_extend(input.max(), &locations, v.min(), &derivatives, last),
                linear_extend(input.max(), &locations, v.max(), &derivatives, last),
            );
        }
        let ranges = points.iter().map(|p| p.value.range(graph)).collect::<Result<Vec<_>, _>>()?;
        for r in &ranges {
            lo = jm::min(lo, r.min());
            hi = jm::max(hi, r.max());
        }
        for i in 0..last {
            let dl = locations[i + 1] - locations[i];
            let (min0, max0, min1, max1) = (ranges[i].min(), ranges[i].max(), ranges[i + 1].min(), ranges[i + 1].max());
            let (d0, d1) = (derivatives[i], derivatives[i + 1]);
            if d0 != 0.0 || d1 != 0.0 {
                let p0 = d0 * dl;
                let p1 = d1 * dl;
                let lo_v = jm::min(min0, min1);
                let hi_v = jm::max(max0, max1);
                let a1 = p0 - max1 + min0;
                let a2 = p0 - min1 + max0;
                let b1 = -p1 + min1 - max0;
                let b2 = -p1 + max1 - min0;
                lo = jm::min(lo, lo_v + 0.25 * jm::min(a1, b1));
                hi = jm::max(hi, hi_v + 0.25 * jm::max(a2, b2));
            }
        }
        Ok(Interval::of(lo, hi))
    }
}

fn linear_extend(x: f32, locations: &[f32], value: f32, derivatives: &[f32], i: usize) -> f32 {
    let d = derivatives[i];
    if d == 0.0 { value } else { value + d * (x - locations[i]) }
}

/// A spline whose coordinates are indices into the owner's coordinate samplers.
#[derive(Clone, Debug)]
pub enum CompiledSpline {
    Constant(f32),
    Multipoint { coordinate: usize, locations: Vec<f32>, values: Vec<CompiledSpline>, derivatives: Vec<f32> },
}

impl CompiledSpline {
    /// `CubicSpline.Multipoint.sample`; `coordinate(i)` yields the i-th coordinate's value.
    pub fn sample(&self, coordinate: &mut impl FnMut(usize) -> f32) -> f32 {
        let (c, locations, values, derivatives) = match self {
            CompiledSpline::Constant(v) => return *v,
            CompiledSpline::Multipoint { coordinate, locations, values, derivatives } => {
                (*coordinate, locations, values, derivatives)
            }
        };
        let x = coordinate(c);
        let start = interval_start(locations, x);
        let last = locations.len() as i32 - 1;
        if start < 0 {
            return linear_extend(x, locations, values[0].sample(coordinate), derivatives, 0);
        }
        if start == last {
            let last = last as usize;
            return linear_extend(x, locations, values[last].sample(coordinate), derivatives, last);
        }
        let i = start as usize;
        let (l0, l1) = (locations[i], locations[i + 1]);
        let t = (x - l0) / (l1 - l0);
        let v0 = values[i].sample(coordinate);
        let v1 = values[i + 1].sample(coordinate);
        let a = derivatives[i] * (l1 - l0) - (v1 - v0);
        let b = -derivatives[i + 1] * (l1 - l0) + (v1 - v0);
        jm::lerp(t, v0, v1) + t * (1.0 - t) * jm::lerp(t, a, b)
    }
}

/// `binarySearch(0, n, i -> x < locations[i]) - 1`.
fn interval_start(locations: &[f32], x: f32) -> i32 {
    let (mut lo, mut len) = (0i32, locations.len() as i32);
    while len > 0 {
        let half = len / 2;
        let mid = lo + half;
        if x < locations[mid as usize] {
            len = half;
        } else {
            lo = mid + 1;
            len -= half + 1;
        }
    }
    lo - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_start_brackets() {
        let l = [-1.0, 0.0, 1.0];
        assert_eq!(interval_start(&l, -2.0), -1);
        assert_eq!(interval_start(&l, -1.0), 0);
        assert_eq!(interval_start(&l, 0.5), 1);
        assert_eq!(interval_start(&l, 1.0), 2);
        assert_eq!(interval_start(&l, f32::NAN), 2);
    }

    #[test]
    fn hermite_hits_points() {
        let s = CompiledSpline::Multipoint {
            coordinate: 0,
            locations: vec![0.0, 1.0],
            values: vec![CompiledSpline::Constant(2.0), CompiledSpline::Constant(4.0)],
            derivatives: vec![0.0, 0.0],
        };
        assert_eq!(s.sample(&mut |_| 0.0), 2.0);
        assert_eq!(s.sample(&mut |_| 1.0), 4.0);
        assert_eq!(s.sample(&mut |_| 0.5), 3.0);
        assert_eq!(s.sample(&mut |_| 7.0), 4.0);
    }
}
