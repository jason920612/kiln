//! The FEATURES status (`ChunkGenerator.applyBiomeDecoration`): the global feature order per
//! generation step (`FeatureSorter`) and the per-chunk decoration loop with vanilla's
//! decoration and feature seeds.

use crate::Error;
use crate::feature::Features;
use crate::generator::Generator;
use crate::json::Json;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// `GenerationStep.Decoration` count.
pub const STEPS: usize = 11;

/// What a decoration run reports to an observer (the parity test replays and compares).
pub trait Observer {
    /// Before placing step `step`'s feature `index` (placed feature `placed`); returning false
    /// skips it.
    fn before(&mut self, _step: usize, _index: usize, _placed: usize, _region: &mut Region) -> bool {
        true
    }

    /// After the placement (or the skip).
    fn after(&mut self, _step: usize, _index: usize, _placed: usize, _region: &mut Region) {}
}

impl Observer for () {}

/// Features of every biome, sorted into vanilla's global per-step order.
pub struct Decorator {
    pub features: Features,
    /// Placed features of each step in `FeatureSorter` order; a feature's position here is its
    /// feature seed index.
    pub steps: Vec<Vec<usize>>,
    /// Per biome and step: indices into `steps[step]`.
    biome_steps: Vec<Vec<Vec<usize>>>,
    /// Biomes the biome source can produce (`BiomeSource.possibleBiomes`).
    possible: Vec<bool>,
}

impl Decorator {
    pub fn new(generator: &Generator, l: &Loader) -> Result<Decorator, Error> {
        let mut features = Features::load(l)?;
        // Per biome, the placed features of each step, from the biome JSON.
        let mut lists: Vec<Vec<Vec<usize>>> = Vec::with_capacity(generator.biomes.len());
        for b in &generator.biomes {
            let json = l
                .pack
                .biomes
                .iter()
                .find(|(id, _)| *id == b.name)
                .map(|(_, j)| j)
                .ok_or_else(|| Error::Invalid(format!("no biome {}", b.name)))?;
            let mut steps = Vec::new();
            if let Some(Json::Array(step_lists)) = json.get("features") {
                for s in step_lists {
                    steps.push(features.placed_list(s, l).map_err(|e| e.context(&b.name))?);
                }
            }
            lists.push(steps);
        }
        let sets = lists.iter().map(|steps| steps.iter().flatten().copied().collect::<HashSet<_>>()).collect();
        features.set_biome_features(sets);

        // `BiomeSource.possibleBiomes`: parameter list order, first occurrence.
        let mut order: Vec<u16> = Vec::new();
        for (_, b) in generator.parameters().values() {
            if !order.contains(b) {
                order.push(*b);
            }
        }
        let mut possible = vec![false; generator.biomes.len()];
        for &b in &order {
            possible[b as usize] = true;
        }
        let steps = sort_features(&order, &lists)?;
        let position: Vec<HashMap<usize, usize>> =
            steps.iter().map(|list| list.iter().enumerate().map(|(i, &f)| (f, i)).collect()).collect();
        let biome_steps = lists
            .iter()
            .map(|biome| {
                biome
                    .iter()
                    .enumerate()
                    .map(|(step, fs)| fs.iter().filter_map(|f| position.get(step).and_then(|m| m.get(f)).copied()).collect())
                    .collect()
            })
            .collect();
        Ok(Decorator { features, steps, biome_steps, possible })
    }

    /// `applyBiomeDecoration` for the region's center chunk (structures not yet included).
    pub fn decorate(&self, r: &mut Region, observer: &mut dyn Observer) {
        let (x, z) = (r.cx << 4, r.cz << 4);
        let origin = crate::pos::BlockPos::new(x, r.min_y(), z);
        let mut random = WorldgenRandom::xoroshiro(0);
        let seed = random.set_decoration_seed(r.seed(), x, z);
        let mut present: Vec<u16> = Vec::new();
        for c in r.chunks() {
            c.present_biomes(&mut present);
        }
        present.retain(|&b| self.possible[b as usize]);
        let total = STEPS.max(self.steps.len());
        let mut indices: Vec<usize> = Vec::new();
        for step in 0..total {
            if step >= self.steps.len() {
                continue;
            }
            indices.clear();
            for &b in &present {
                if let Some(list) = self.biome_steps[b as usize].get(step) {
                    indices.extend_from_slice(list);
                }
            }
            indices.sort_unstable();
            indices.dedup();
            for &i in &indices {
                let placed = self.steps[step][i];
                random.set_feature_seed(seed, i as i32, step as i32);
                if observer.before(step, i, placed, r) {
                    self.features.place_placed(placed, r, &mut random, origin, true);
                }
                observer.after(step, i, placed, r);
            }
        }
    }
}

/// `FeatureSorter.buildFeaturesPerStep`: a topological order of all placed features that
/// respects every biome's order, with ties broken by (step, first-seen index).
fn sort_features(biomes: &[u16], lists: &[Vec<Vec<usize>>]) -> Result<Vec<Vec<usize>>, Error> {
    type Node = (usize, u32);
    let mut index: HashMap<usize, u32> = HashMap::new();
    let mut edges: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
    let mut max_steps = 0;
    let mut feature_of: HashMap<u32, usize> = HashMap::new();
    for &b in biomes {
        let steps = &lists[b as usize];
        max_steps = max_steps.max(steps.len());
        let mut flat: Vec<Node> = Vec::new();
        for (step, fs) in steps.iter().enumerate() {
            for &f in fs {
                let next = index.len() as u32;
                let i = *index.entry(f).or_insert(next);
                feature_of.insert(i, f);
                flat.push((step, i));
            }
        }
        for i in 0..flat.len() {
            let set = edges.entry(flat[i]).or_default();
            if i + 1 < flat.len() {
                set.insert(flat[i + 1]);
            }
        }
    }
    fn dfs(
        n: Node,
        edges: &BTreeMap<Node, BTreeSet<Node>>,
        done: &mut BTreeSet<Node>,
        path: &mut BTreeSet<Node>,
        out: &mut Vec<Node>,
    ) -> bool {
        if done.contains(&n) {
            return false;
        }
        if path.contains(&n) {
            return true;
        }
        path.insert(n);
        if let Some(next) = edges.get(&n) {
            for &m in next {
                if dfs(m, edges, done, path, out) {
                    return true;
                }
            }
        }
        path.remove(&n);
        done.insert(n);
        out.push(n);
        false
    }
    let (mut done, mut path, mut out) = (BTreeSet::new(), BTreeSet::new(), Vec::new());
    for &n in edges.keys() {
        if done.contains(&n) {
            continue;
        }
        if dfs(n, &edges, &mut done, &mut path, &mut out) {
            return Err(Error::Invalid("feature order cycle found".into()));
        }
    }
    out.reverse();
    Ok((0..max_steps).map(|s| out.iter().filter(|(step, _)| *step == s).map(|(_, i)| feature_of[i]).collect()).collect())
}
