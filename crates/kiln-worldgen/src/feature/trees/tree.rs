//! `TreeFeature`: roots, trunk and foliage, then decorators, then leaf distances and shape
//! updates along the tree's outline.

use super::decorator::{DecoCtx, Decorator};
use super::foliage::FoliagePlacer;
use super::jset::JHashSet;
use super::root::RootPlacer;
use super::shape;
use super::trunk::TrunkPlacer;
use super::{field, int_or};
use crate::Error;
use crate::blocks::{is_air, is_block, prop, with_prop};
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use crate::vtags;

/// `FeatureSize`.
#[derive(Debug)]
pub enum FeatureSize {
    TwoLayers { limit: i32, lower: i32, upper: i32, min_clipped: Option<i32> },
    ThreeLayers { limit: i32, upper_limit: i32, lower: i32, middle: i32, upper: i32, min_clipped: Option<i32> },
}

impl FeatureSize {
    fn parse(json: &Json) -> Result<FeatureSize, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        let min_clipped = json.get("min_clipped_height").and_then(Json::as_i32);
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "two_layers_feature_size" => FeatureSize::TwoLayers {
                limit: int_or(json, "limit", 1),
                lower: int_or(json, "lower_size", 0),
                upper: int_or(json, "upper_size", 1),
                min_clipped,
            },
            "three_layers_feature_size" => FeatureSize::ThreeLayers {
                limit: int_or(json, "limit", 1),
                upper_limit: int_or(json, "upper_limit", 1),
                lower: int_or(json, "lower_size", 0),
                middle: int_or(json, "middle_size", 1),
                upper: int_or(json, "upper_size", 1),
                min_clipped,
            },
            t => return Err(Error::Invalid(format!("unknown feature size {t}"))),
        })
    }

    /// `FeatureSize.getSizeAtHeight`.
    fn size_at(&self, height: i32, y: i32) -> i32 {
        match *self {
            FeatureSize::TwoLayers { limit, lower, upper, .. } => {
                if y < limit {
                    lower
                } else {
                    upper
                }
            }
            FeatureSize::ThreeLayers { limit, upper_limit, lower, middle, upper, .. } => {
                if y < limit {
                    lower
                } else if y >= height - upper_limit {
                    upper
                } else {
                    middle
                }
            }
        }
    }

    fn min_clipped(&self) -> Option<i32> {
        match *self {
            FeatureSize::TwoLayers { min_clipped, .. } | FeatureSize::ThreeLayers { min_clipped, .. } => min_clipped,
        }
    }
}

#[derive(Debug)]
pub struct Tree {
    pub trunk_provider: StateProvider,
    trunk_placer: TrunkPlacer,
    pub foliage_provider: StateProvider,
    foliage_placer: FoliagePlacer,
    root_placer: Option<RootPlacer>,
    minimum_size: FeatureSize,
    pub decorators: Vec<Decorator>,
    ignore_vines: bool,
    pub below_trunk_provider: StateProvider,
}

/// Which of `TreeFeature.place`'s position sets a setter records into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Roots,
    Trunk,
    Foliage,
}

/// A tree being placed: the level, the feature random and the positions placed so far.
pub struct Ctx<'c, 'r> {
    pub tree: &'c Tree,
    pub r: &'c mut Region<'r>,
    pub random: &'c mut WorldgenRandom,
    pub roots: JHashSet,
    pub logs: JHashSet,
    pub leaves: JHashSet,
}

impl Ctx<'_, '_> {
    /// The setters of `TreeFeature.place`: record the position, then `setBlock(pos, state, 19)`.
    pub fn put(&mut self, part: Part, p: BlockPos, s: u16) {
        match part {
            Part::Roots => self.roots.insert(p),
            Part::Trunk => self.logs.insert(p),
            Part::Foliage => self.leaves.insert(p),
        };
        self.r.set(p, s, 19);
    }
}

/// `TreeFeature.validTreePos`.
pub fn valid_tree_pos(r: &mut Region, p: BlockPos) -> bool {
    let s = r.get(p);
    is_air(s) || vtags::is(s, "replaceable_by_trees")
}

/// `TreeFeature.isAirOrLeaves`.
pub fn is_air_or_leaves(r: &mut Region, p: BlockPos) -> bool {
    let s = r.get(p);
    is_air(s) || vtags::is(s, "leaves")
}

/// `TreeFeature.isVine`.
fn is_vine(r: &mut Region, p: BlockPos) -> bool {
    is_block(r.get(p), "minecraft:vine")
}

impl Tree {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader) -> Result<Tree, Error> {
        Ok(Tree {
            trunk_provider: StateProvider::parse(field(json, "trunk_provider")?, l)?,
            trunk_placer: TrunkPlacer::parse(field(json, "trunk_placer")?, l)?,
            foliage_provider: StateProvider::parse(field(json, "foliage_provider")?, l)?,
            foliage_placer: FoliagePlacer::parse(field(json, "foliage_placer")?)?,
            root_placer: json.get("root_placer").map(|j| RootPlacer::parse(j, l)).transpose()?,
            minimum_size: FeatureSize::parse(field(json, "minimum_size")?)?,
            decorators: match json.get("decorators").and_then(Json::as_array) {
                Some(list) => list.iter().map(|d| Decorator::parse(d, f, l)).collect::<Result<_, _>>()?,
                None => Vec::new(),
            },
            ignore_vines: json.get("ignore_vines").and_then(Json::as_bool).unwrap_or(false),
            below_trunk_provider: StateProvider::parse(field(json, "below_trunk_provider")?, l)?,
        })
    }

    /// `TreeFeature.trunkPlacer().getBaseHeight()`.
    pub fn base_height(&self) -> i32 {
        self.trunk_placer.base_height()
    }

    /// `TreeFeature.place`.
    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut cx = Ctx { tree: self, r, random, roots: JHashSet::new(), logs: JHashSet::new(), leaves: JHashSet::new() };
        let placed = self.do_place(&mut cx, origin);
        let Ctx { r, random, roots, logs, leaves, .. } = cx;
        if !placed || (logs.is_empty() && leaves.is_empty()) {
            return false;
        }
        let mut decor = JHashSet::new();
        if !self.decorators.is_empty() {
            let mut d = DecoCtx::new(f, r, random, &logs, &leaves, &roots, Some(&mut decor));
            for decorator in &self.decorators {
                decorator.place(&mut d);
            }
        }
        let all = roots.iter().chain(logs.iter()).chain(leaves.iter()).chain(decor.iter());
        let Some(bbox) = BBox::encapsulating(all) else { return false };
        let voxels = update_leaves(r, &bbox, &logs, &decor, &roots);
        shape::update_shape_at_edge(r, 3, &voxels, bbox.min);
        true
    }

    /// `TreeFeature.doPlace`.
    fn do_place(&self, cx: &mut Ctx, origin: BlockPos) -> bool {
        let height = self.trunk_placer.tree_height(cx.random);
        let foliage_height = self.foliage_placer.foliage_height(cx.random, height);
        let trunk_height = height - foliage_height;
        let foliage_radius = self.foliage_placer.foliage_radius(cx.random, trunk_height);
        let trunk_origin = match &self.root_placer {
            Some(rp) => rp.trunk_origin(origin, cx.random),
            None => origin,
        };
        let min_y = origin.y.min(trunk_origin.y);
        let max_y = origin.y.max(trunk_origin.y) + height + 1;
        if min_y < cx.r.min_y() + 1 || max_y > cx.r.max_y() + 1 {
            return false;
        }
        let free = self.max_free_tree_height(cx.r, height, trunk_origin);
        if free < height && self.minimum_size.min_clipped().is_none_or(|m| free < m) {
            return false;
        }
        if let Some(rp) = &self.root_placer
            && !rp.place_roots(cx, origin, trunk_origin)
        {
            return false;
        }
        let attachments = self.trunk_placer.place_trunk(cx, free, trunk_origin);
        for a in &attachments {
            self.foliage_placer.create_foliage(cx, a, foliage_height, foliage_radius);
        }
        true
    }

    /// `TreeFeature.getMaxFreeTreeHeight`.
    fn max_free_tree_height(&self, r: &mut Region, height: i32, origin: BlockPos) -> i32 {
        for y in 0..=height + 1 {
            let size = self.minimum_size.size_at(height, y);
            for dx in -size..=size {
                for dz in -size..=size {
                    let p = origin.offset(dx, y, dz);
                    if !self.trunk_placer.is_free(r, p) || (!self.ignore_vines && is_vine(r, p)) {
                        return y - 2;
                    }
                }
            }
        }
        height
    }
}

/// `TreeFeature.getLowestTrunkOrRootOfTree`.
pub fn lowest_trunk_or_root(logs: &[BlockPos], roots: &[BlockPos]) -> Vec<BlockPos> {
    let mut out = Vec::new();
    if roots.is_empty() {
        out.extend_from_slice(logs);
    } else if !logs.is_empty() && roots[0].y == logs[0].y {
        out.extend_from_slice(logs);
        out.extend_from_slice(roots);
    } else {
        out.extend_from_slice(roots);
    }
    out
}

/// `BoundingBox`.
#[derive(Clone, Copy, Debug)]
pub struct BBox {
    pub min: BlockPos,
    pub max: BlockPos,
}

impl BBox {
    /// `BoundingBox.encapsulatingPositions`.
    fn encapsulating(mut it: impl Iterator<Item = BlockPos>) -> Option<BBox> {
        let first = it.next()?;
        let mut b = BBox { min: first, max: first };
        for p in it {
            b.min = BlockPos::new(b.min.x.min(p.x), b.min.y.min(p.y), b.min.z.min(p.z));
            b.max = BlockPos::new(b.max.x.max(p.x), b.max.y.max(p.y), b.max.z.max(p.z));
        }
        Some(b)
    }

    fn inside(&self, p: BlockPos) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y && p.z >= self.min.z && p.z <= self.max.z
    }

    fn span(&self) -> (i32, i32, i32) {
        (self.max.x - self.min.x + 1, self.max.y - self.min.y + 1, self.max.z - self.min.z + 1)
    }
}

/// `BitSetDiscreteVoxelShape`: filled cells of a box.
pub struct Voxels {
    pub size: (i32, i32, i32),
    bits: Vec<bool>,
}

impl Voxels {
    fn new(size: (i32, i32, i32)) -> Voxels {
        Voxels { size, bits: vec![false; (size.0 * size.1 * size.2) as usize] }
    }

    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        ((x * self.size.1 + y) * self.size.2 + z) as usize
    }

    fn fill(&mut self, x: i32, y: i32, z: i32) {
        let i = self.index(x, y, z);
        self.bits[i] = true;
    }

    pub fn is_full(&self, x: i32, y: i32, z: i32) -> bool {
        self.bits[self.index(x, y, z)]
    }
}

/// `LeavesBlock.getOptionalDistanceAt`.
fn optional_distance(s: u16) -> Option<i32> {
    if vtags::is(s, "prevents_nearby_leaf_decay") {
        return Some(0);
    }
    prop(s, "distance").and_then(|d| d.parse().ok())
}

/// `TreeFeature.updateLeaves`: leaf distances from the trunk, breadth first in the order of
/// Java's hash sets; returns the cells of the tree (decorations, roots and everything reached).
fn update_leaves(r: &mut Region, bbox: &BBox, logs: &JHashSet, decor: &JHashSet, roots: &JHashSet) -> Voxels {
    let mut shape = Voxels::new(bbox.span());
    let rel = |p: BlockPos| (p.x - bbox.min.x, p.y - bbox.min.y, p.z - bbox.min.z);
    for p in decor.iter().chain(roots.iter().filter(|p| !decor.contains(*p))) {
        if bbox.inside(p) {
            let (x, y, z) = rel(p);
            shape.fill(x, y, z);
        }
    }
    let mut layers: Vec<JHashSet> = (0..7).map(|_| JHashSet::new()).collect();
    for p in logs.iter() {
        layers[0].insert(p);
    }
    let mut i = 0usize;
    loop {
        while i < 7 && layers[i].is_empty() {
            i += 1;
        }
        if i >= 7 {
            break;
        }
        let p = layers[i].pop_first().expect("non-empty layer");
        if !bbox.inside(p) {
            continue;
        }
        if i != 0 {
            let s = r.get(p);
            r.set(p, with_prop(s, "distance", &i.to_string()), 19);
        }
        let (x, y, z) = rel(p);
        shape.fill(x, y, z);
        for d in crate::block_facts::Dir::ALL {
            let q = p.relative(d);
            if !bbox.inside(q) {
                continue;
            }
            let (qx, qy, qz) = rel(q);
            if shape.is_full(qx, qy, qz) {
                continue;
            }
            let Some(dist) = optional_distance(r.get(q)) else { continue };
            let k = dist.min(i as i32 + 1) as usize;
            if k < 7 {
                layers[k].insert(q);
                i = i.min(k);
            }
        }
    }
    shape
}
