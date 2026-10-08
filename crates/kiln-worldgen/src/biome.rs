//! Multi-noise biome lookup: vanilla's `Climate` parameter space, its R-tree search and the
//! `multi_noise` biome source.
//!
//! Climate values are quantized to `i64` (`(long) (v * 10000f)`), each biome occupies a box in
//! the 7-dimensional parameter space (the seventh axis is the fixed `offset`), and a lookup
//! returns the box with the smallest squared distance. The R-tree is built exactly like
//! `Climate.RTree.create`, because with equal distances the first leaf found wins, and a
//! search starts from the previous result (`RTree.lastResult`), which is threaded explicitly
//! here: see [`LastResult`].

use crate::Error;
use crate::json::Json;
use std::cmp::Ordering;

pub const PARAMETERS: usize = 7;
const CHILDREN_PER_NODE: usize = 19;

/// `Climate.quantizeCoord`.
#[inline]
pub fn quantize(v: f32) -> i64 {
    (v * 10000.0) as i64
}

/// `Climate.unquantizeCoord`.
#[inline]
pub fn unquantize(v: i64) -> f32 {
    v as f32 / 10000.0
}

/// `Climate.Parameter`: a closed interval of quantized values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub min: i64,
    pub max: i64,
}

impl Parameter {
    pub fn span(min: f32, max: f32) -> Parameter {
        Parameter { min: quantize(min), max: quantize(max) }
    }

    /// Distance from `v` to the interval (0 inside).
    #[inline]
    pub fn distance(self, v: i64) -> i64 {
        let above = v - self.max;
        let below = self.min - v;
        if above > 0 { above } else { below.max(0) }
    }

    fn union(self, other: Option<Parameter>) -> Parameter {
        match other {
            None => self,
            Some(o) => Parameter { min: self.min.min(o.min), max: self.max.max(o.max) },
        }
    }

    /// The sort key `RTree.comparator` uses: the midpoint, optionally absolute.
    fn midpoint(self, absolute: bool) -> i64 {
        let m = (self.min + self.max) / 2;
        if absolute { m.abs() } else { m }
    }
}

/// `Climate.ParameterPoint.parameterSpace()`: temperature, humidity, continentalness,
/// erosion, depth, weirdness, then the offset as a degenerate interval.
pub type ParameterSpace = [Parameter; PARAMETERS];

/// `Climate.TargetPoint.toParameterArray()`: the six sampled values and a zero offset.
pub type Target = [i64; PARAMETERS];

/// `Climate.target`.
pub fn target(temperature: f32, humidity: f32, continentalness: f32, erosion: f32, depth: f32, weirdness: f32) -> Target {
    [
        quantize(temperature),
        quantize(humidity),
        quantize(continentalness),
        quantize(erosion),
        quantize(depth),
        quantize(weirdness),
        0,
    ]
}

/// `RTree.Node.distance`: squared distance from the target to a node's bounding box.
#[inline]
fn distance(space: &ParameterSpace, target: &Target) -> i64 {
    let mut d = 0i64;
    for i in 0..PARAMETERS {
        let p = space[i].distance(target[i]);
        d = d.wrapping_add(p.wrapping_mul(p));
    }
    d
}

struct Node {
    space: ParameterSpace,
    /// Child node ids; empty for a leaf.
    children: Vec<u32>,
}

/// The leaf a previous search returned (`RTree.lastResult`, a thread-local in vanilla): a
/// search starts from it, so it decides between leaves at equal distance.
pub type LastResult = Option<u32>;

/// `Climate.ParameterList` with its search tree; leaf `i` is value `i`.
pub struct ParameterList<T> {
    values: Vec<(ParameterSpace, T)>,
    nodes: Vec<Node>,
    root: u32,
}

impl<T> ParameterList<T> {
    pub fn new(values: Vec<(ParameterSpace, T)>) -> Result<Self, Error> {
        if values.is_empty() {
            return Err(Error::Invalid("Need at least one value to build the search tree.".into()));
        }
        let mut nodes: Vec<Node> = values.iter().map(|(s, _)| Node { space: *s, children: Vec::new() }).collect();
        let leaves: Vec<u32> = (0..values.len() as u32).collect();
        let root = build(&mut nodes, leaves);
        Ok(Self { values, nodes, root })
    }

    pub fn values(&self) -> &[(ParameterSpace, T)] {
        &self.values
    }

    /// `ParameterList.findValue` / `RTree.search`: the closest value, starting from and
    /// updating `last`.
    pub fn find(&self, target: &Target, last: &mut LastResult) -> &T {
        let leaf = self.search(self.root, target, *last).expect("a search always finds a leaf");
        *last = Some(leaf);
        &self.values[leaf as usize].1
    }

    /// `RTree.SubTree.search` (a leaf returns itself).
    fn search(&self, node: u32, target: &Target, candidate: LastResult) -> LastResult {
        let n = &self.nodes[node as usize];
        if n.children.is_empty() {
            return Some(node);
        }
        let mut best = match candidate {
            Some(c) => distance(&self.nodes[c as usize].space, target),
            None => i64::MAX,
        };
        let mut result = candidate;
        for &child in &n.children {
            let child_distance = distance(&self.nodes[child as usize].space, target);
            if best > child_distance {
                let leaf = self.search(child, target, result).expect("subtrees are never empty");
                let leaf_distance =
                    if leaf == child { child_distance } else { distance(&self.nodes[leaf as usize].space, target) };
                if best > leaf_distance {
                    best = leaf_distance;
                    result = Some(leaf);
                }
            }
        }
        result
    }
}

fn subtree(nodes: &mut Vec<Node>, children: Vec<u32>) -> u32 {
    let mut space: [Option<Parameter>; PARAMETERS] = [None; PARAMETERS];
    for &c in &children {
        for (i, s) in space.iter_mut().enumerate() {
            *s = Some(nodes[c as usize].space[i].union(*s));
        }
    }
    nodes.push(Node { space: space.map(|s| s.expect("SubTree needs at least one child")), children });
    (nodes.len() - 1) as u32
}

/// `RTree.sort`: a stable sort by midpoints, axis `start` first, then the following axes.
fn sort(nodes: &[Node], list: &mut [u32], start: usize, absolute: bool) {
    list.sort_by(|&a, &b| {
        let (a, b) = (&nodes[a as usize].space, &nodes[b as usize].space);
        for k in 0..PARAMETERS {
            let i = (start + k) % PARAMETERS;
            match a[i].midpoint(absolute).cmp(&b[i].midpoint(absolute)) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        Ordering::Equal
    });
}

/// `RTree.bucketize`: consecutive runs of `19^floor(log19(n - 0.01))` children.
fn bucketize(nodes: &mut Vec<Node>, children: &[u32]) -> Vec<u32> {
    let size = CHILDREN_PER_NODE as f64;
    let exponent = ((kiln_javamath::pow::log(children.len() as f64 - 0.01) / kiln_javamath::pow::log(size)).floor()) as u32;
    let per_bucket = CHILDREN_PER_NODE.pow(exponent);
    let mut buckets = Vec::new();
    let mut current = Vec::new();
    for &c in children {
        current.push(c);
        if current.len() >= per_bucket {
            buckets.push(subtree(nodes, std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        buckets.push(subtree(nodes, current));
    }
    buckets
}

fn cost(space: &ParameterSpace) -> i64 {
    space.iter().fold(0i64, |acc, p| acc.wrapping_add((p.max - p.min).abs()))
}

/// `RTree.build`.
fn build(nodes: &mut Vec<Node>, mut children: Vec<u32>) -> u32 {
    if children.len() == 1 {
        return children[0];
    }
    if children.len() <= CHILDREN_PER_NODE {
        children.sort_by_key(|&c| {
            nodes[c as usize].space.iter().fold(0i64, |acc, p| acc.wrapping_add(p.midpoint(true)))
        });
        return subtree(nodes, children);
    }
    let mut best_cost = i64::MAX;
    let mut best_axis = 0;
    let mut best_buckets = Vec::new();
    for axis in 0..PARAMETERS {
        sort(nodes, &mut children, axis, false);
        let buckets = bucketize(nodes, &children);
        let total = buckets.iter().fold(0i64, |acc, &b| acc.wrapping_add(cost(&nodes[b as usize].space)));
        if best_cost > total {
            best_cost = total;
            best_axis = axis;
            best_buckets = buckets;
        }
    }
    sort(nodes, &mut best_buckets, best_axis, true);
    let built: Vec<u32> = best_buckets
        .iter()
        .map(|&b| {
            let grandchildren = nodes[b as usize].children.clone();
            build(nodes, grandchildren)
        })
        .collect();
    subtree(nodes, built)
}

/// What generation needs of a `worldgen/biome` entry.
#[derive(Clone, Debug)]
pub struct BiomeInfo {
    pub name: String,
    /// `ClimateSettings.temperature`.
    pub temperature: f32,
    /// `TemperatureModifier.FROZEN` (else `NONE`).
    pub frozen: bool,
    /// `ClimateSettings.hasPrecipitation`.
    pub has_precipitation: bool,
    /// Configured carver ids, in order.
    pub carvers: Vec<String>,
}

impl BiomeInfo {
    pub fn parse(name: &str, json: &Json) -> Result<BiomeInfo, Error> {
        let temperature = json.get("temperature").and_then(Json::as_f32).ok_or_else(|| Error::Invalid("bad temperature".into()))?;
        let frozen = match json.get("temperature_modifier").and_then(Json::as_str) {
            None | Some("none") => false,
            Some("frozen") => true,
            Some(m) => return Err(Error::Invalid(format!("unknown temperature modifier {m}"))),
        };
        let has_precipitation = json.get("has_precipitation").and_then(Json::as_bool).unwrap_or(false);
        let carvers = match json.get("carvers") {
            None => Vec::new(),
            Some(Json::String(s)) => vec![crate::function::qualify(s)],
            Some(Json::Array(a)) => a
                .iter()
                .map(|c| c.as_str().map(crate::function::qualify).ok_or_else(|| Error::Invalid("inline carvers are not supported".into())))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(Error::Invalid("bad carvers".into())),
        };
        Ok(BiomeInfo { name: name.to_string(), temperature, frozen, has_precipitation, carvers })
    }
}

/// `Climate.Parameter.CODEC`: a single number or a `[min, max]` pair, read as `float`.
pub(crate) fn parse_parameter(json: &Json) -> Result<Parameter, Error> {
    let bad = || Error::Invalid(format!("bad climate parameter {json:?}"));
    if let Some(v) = json.as_f32() {
        return Ok(Parameter::span(v, v));
    }
    match json.as_array() {
        Some([a, b]) => {
            let (a, b) = (a.as_f32().ok_or_else(bad)?, b.as_f32().ok_or_else(bad)?);
            if a > b {
                return Err(Error::Invalid(format!("climate parameter min {a} > max {b}")));
            }
            Ok(Parameter::span(a, b))
        }
        _ => Err(bad()),
    }
}

/// A biome parameter list in the format of `reports/biome_parameters/<ns>/<id>.json` (and of
/// a `multi_noise` biome source's inline `biomes`): `(parameter space, biome id)` in order.
pub fn parse_parameter_list(json: &Json) -> Result<Vec<(ParameterSpace, String)>, Error> {
    let entries = json
        .get("biomes")
        .and_then(Json::as_array)
        .ok_or_else(|| Error::Invalid("parameter list without biomes".into()))?;
    entries
        .iter()
        .map(|e| {
            let biome = e.get("biome").and_then(Json::as_str).ok_or_else(|| Error::Invalid("entry without biome".into()))?;
            let p = e.get("parameters").ok_or_else(|| Error::Invalid("entry without parameters".into()))?;
            let axis = |k: &str| p.get(k).ok_or_else(|| Error::Invalid(format!("missing {k}"))).and_then(parse_parameter);
            let offset = p.get("offset").and_then(Json::as_f32).ok_or_else(|| Error::Invalid("missing offset".into()))?;
            let q = quantize(offset);
            Ok((
                [
                    axis("temperature")?,
                    axis("humidity")?,
                    axis("continentalness")?,
                    axis("erosion")?,
                    axis("depth")?,
                    axis("weirdness")?,
                    Parameter { min: q, max: q },
                ],
                crate::function::qualify(biome),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(t: f32, h: f32) -> ParameterSpace {
        let z = Parameter::span(0.0, 0.0);
        [Parameter::span(t, t), Parameter::span(h, h), z, z, z, z, z]
    }

    #[test]
    fn finds_nearest_and_breaks_ties_by_history() {
        let mut values = Vec::new();
        for i in 0..40 {
            values.push((point(i as f32 * 0.05 - 1.0, 0.0), i));
        }
        let list = ParameterList::new(values).unwrap();
        let mut last = None;
        let t = target(-0.49, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(*list.find(&t, &mut last), 10);
        // Exactly between entries 10 (-0.5) and 11 (-0.45): the previous result is kept.
        let mid = target(-0.475, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(mid[0], -4750);
        assert_eq!(*list.find(&mid, &mut last), 10);
        let mut other = Some(11);
        assert_eq!(*list.find(&mid, &mut other), 11);
    }

    #[test]
    fn brute_force_agrees_on_distance() {
        let mut values = Vec::new();
        let mut v = 0x2545_f491u32;
        let mut next = || {
            v ^= v << 13;
            v ^= v >> 17;
            v ^= v << 5;
            (v % 2000) as f32 / 1000.0 - 1.0
        };
        for i in 0..500 {
            let (a, b, c, d) = (next(), next(), next(), next());
            let z = Parameter::span(0.0, 0.0);
            values.push((
                [Parameter::span(a.min(b), a.max(b)), Parameter::span(c.min(d), c.max(d)), z, z, z, z, z],
                i,
            ));
        }
        let list = ParameterList::new(values).unwrap();
        for _ in 0..2000 {
            let t = target(next(), next(), 0.0, 0.0, 0.0, 0.0);
            let found = *list.find(&t, &mut None);
            let best = list.values().iter().map(|(s, _)| distance(s, &t)).min().unwrap();
            assert_eq!(distance(&list.values()[found].0, &t), best);
        }
    }
}
