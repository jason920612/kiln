//! Placed features: a feature plus the placement modifiers that turn one origin into the
//! positions it is placed at (`PlacedFeature`, `PlacementModifier`, `FeaturePlacer`).

use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{is_lava, is_water};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::proto::Heightmap;
use crate::providers::{GenContext, HeightProvider, IntProvider, float, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::simplex::biome_noises;
use kiln_javamath::random::RandomSource;

/// `PlacementModifier`.
#[derive(Clone, Debug)]
pub enum Modifier {
    Count(IntProvider),
    CountOnEveryLayer(IntProvider),
    NoiseBasedCount { ratio: i32, factor: f64, offset: f64 },
    NoiseThresholdCount { level: f64, below: i32, above: i32 },
    InSquare,
    Offset { x: IntProvider, y: IntProvider, z: IntProvider },
    HeightRange(HeightProvider),
    Heightmap(Heightmap),
    Rarity(i32),
    RandomChance(f32),
    RandomlySelected(Vec<Modifier>),
    Biome,
    BlockPredicate(BlockPredicate),
    SurfaceWaterDepth(i32),
    SurfaceRelativeThreshold { heightmap: Heightmap, min: i32, max: i32 },
    EnvironmentScan { direction: Dir, target: BlockPredicate, allowed: BlockPredicate, max_steps: i32 },
    Fixed(Vec<BlockPos>),
    /// `CuboidPlacement`: every position of a box around the origin.
    Cuboid(crate::feature::cuboid::Cuboid),
}

impl Modifier {
    pub fn parse(json: &Json, l: &Loader) -> Result<Modifier, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("placement modifier {ty} without {k}")));
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "count" => Modifier::Count(IntProvider::parse(field("count")?)?),
            "count_on_every_layer" => Modifier::CountOnEveryLayer(IntProvider::parse(field("count")?)?),
            "noise_based_count" => Modifier::NoiseBasedCount {
                ratio: int(json, "noise_to_count_ratio")?,
                factor: field("noise_factor")?.as_f64().ok_or_else(|| Error::Invalid("bad noise_factor".into()))?,
                offset: json.get("noise_offset").and_then(Json::as_f64).unwrap_or(0.0),
            },
            "noise_threshold_count" => Modifier::NoiseThresholdCount {
                level: field("noise_level")?.as_f64().ok_or_else(|| Error::Invalid("bad noise_level".into()))?,
                below: int(json, "below_noise")?,
                above: int(json, "above_noise")?,
            },
            "in_square" => Modifier::InSquare,
            "offset" => Modifier::Offset {
                x: json.get("x").map_or(Ok(IntProvider::Constant(0)), IntProvider::parse)?,
                y: json.get("y").map_or(Ok(IntProvider::Constant(0)), IntProvider::parse)?,
                z: json.get("z").map_or(Ok(IntProvider::Constant(0)), IntProvider::parse)?,
            },
            "height_range" => Modifier::HeightRange(HeightProvider::parse(field("height")?)?),
            "heightmap" => Modifier::Heightmap(heightmap(field("heightmap")?)?),
            "rarity_filter" => Modifier::Rarity(int(json, "chance")?),
            "random_chance" => Modifier::RandomChance(float(json, "chance")?),
            "randomly_selected" => Modifier::RandomlySelected(
                field("placements")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("placements must be a list".into()))?
                    .iter()
                    .map(|m| Modifier::parse(m, l))
                    .collect::<Result<_, _>>()?,
            ),
            "biome" => Modifier::Biome,
            "block_predicate_filter" => Modifier::BlockPredicate(BlockPredicate::parse(field("predicate")?, l)?),
            "surface_water_depth_filter" => Modifier::SurfaceWaterDepth(int(json, "max_water_depth")?),
            "surface_relative_threshold_filter" => Modifier::SurfaceRelativeThreshold {
                heightmap: heightmap(field("heightmap")?)?,
                min: json.get("min_inclusive").and_then(Json::as_i32).unwrap_or(i32::MIN),
                max: json.get("max_inclusive").and_then(Json::as_i32).unwrap_or(i32::MAX),
            },
            "environment_scan" => Modifier::EnvironmentScan {
                direction: Dir::by_name(field("direction_of_search")?.as_str().unwrap_or(""))
                    .ok_or_else(|| Error::Invalid("bad direction_of_search".into()))?,
                target: BlockPredicate::parse(field("target_condition")?, l)?,
                allowed: match json.get("allowed_search_condition") {
                    Some(p) => BlockPredicate::parse(p, l)?,
                    None => BlockPredicate::True,
                },
                max_steps: int(json, "max_steps")?,
            },
            "fixed_placement" => Modifier::Fixed(
                field("positions")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("positions must be a list".into()))?
                    .iter()
                    .map(|p| match p.as_array() {
                        Some([x, y, z]) => Ok(BlockPos::new(
                            x.as_i32().unwrap_or(0),
                            y.as_i32().unwrap_or(0),
                            z.as_i32().unwrap_or(0),
                        )),
                        _ => Err(Error::Invalid(format!("bad position {p:?}"))),
                    })
                    .collect::<Result<_, _>>()?,
            ),
            "cuboid" => Modifier::Cuboid(crate::feature::cuboid::Cuboid::parse(json)?),
            t => return Err(Error::Invalid(format!("unsupported placement modifier {t}"))),
        })
    }

    /// `modify(context, random, pos, out)`.
    pub fn modify(&self, cx: &mut PlaceCtx, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, out: &mut Vec<BlockPos>) {
        match self {
            Modifier::Count(c) => {
                for _ in 0..c.sample(random) {
                    out.push(p);
                }
            }
            Modifier::NoiseBasedCount { ratio, factor, offset } => {
                let n = biome_noises().biome_info.get2(p.x as f64 / factor, p.z as f64 / factor) as f64;
                for _ in 0..((n + offset) * *ratio as f64).ceil() as i32 {
                    out.push(p);
                }
            }
            Modifier::NoiseThresholdCount { level, below, above } => {
                let n = biome_noises().biome_info.get2(p.x as f64 / 200.0, p.z as f64 / 200.0) as f64;
                for _ in 0..if n < *level { *below } else { *above } {
                    out.push(p);
                }
            }
            Modifier::CountOnEveryLayer(c) => {
                let mut layer = 0;
                loop {
                    let mut found = false;
                    for _ in 0..c.sample(random) {
                        let x = random.next_int_bounded(16) + p.x;
                        let z = random.next_int_bounded(16) + p.z;
                        let top = r.height_at(Heightmap::MotionBlocking, x, z);
                        let y = on_ground_y(r, x, top, z, layer);
                        if y != i32::MAX {
                            out.push(BlockPos::new(x, y, z));
                            found = true;
                        }
                    }
                    layer += 1;
                    if !found {
                        break;
                    }
                }
            }
            Modifier::InSquare => {
                let x = random.next_int_bounded(16) + p.x;
                let z = random.next_int_bounded(16) + p.z;
                out.push(BlockPos::new(x, p.y, z));
            }
            Modifier::Offset { x, y, z } => {
                let dx = x.sample(random);
                let dy = y.sample(random);
                let dz = z.sample(random);
                out.push(p.offset(dx, dy, dz));
            }
            Modifier::HeightRange(h) => {
                let g = GenContext { min_y: r.min_y(), height: r.height(), sea_level: r.sea_level() };
                out.push(p.at_y(h.sample(random, g)));
            }
            Modifier::Heightmap(map) => {
                let y = r.height_at(*map, p.x, p.z);
                if y > r.min_y() {
                    out.push(p.at_y(y));
                }
            }
            Modifier::RandomlySelected(list) => {
                let m = &list[random.next_int_bounded(list.len() as i32) as usize];
                m.modify(cx, r, random, p, out);
            }
            Modifier::Fixed(list) => {
                for q in list {
                    if q.x >> 4 == p.x >> 4 && q.z >> 4 == p.z >> 4 {
                        out.push(*q);
                    }
                }
            }
            Modifier::Cuboid(c) => c.modify(random, p, out),
            filter => {
                if filter.should_place(cx, r, random, p) {
                    out.push(p);
                }
            }
        }
    }

    /// `PlacementFilter.shouldPlace`.
    fn should_place(&self, cx: &mut PlaceCtx, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        match self {
            Modifier::Rarity(chance) => random.next_float() < 1.0 / *chance as f32,
            Modifier::RandomChance(chance) => random.next_float() < *chance,
            Modifier::Biome => {
                let top = cx.top.expect("biome check of an unregistered feature, or one that should not restrict the biome");
                let biome = r.biome(p);
                cx.features.biome_has(biome, top)
            }
            Modifier::BlockPredicate(pred) => pred.test(r, p),
            Modifier::SurfaceWaterDepth(max) => {
                let floor = r.height_at(Heightmap::OceanFloor, p.x, p.z);
                let surface = r.height_at(Heightmap::WorldSurface, p.x, p.z);
                surface - floor <= *max
            }
            Modifier::SurfaceRelativeThreshold { heightmap, min, max } => {
                let h = r.height_at(*heightmap, p.x, p.z) as i64;
                h + *min as i64 <= p.y as i64 && p.y as i64 <= h + *max as i64
            }
            _ => unreachable!("not a filter"),
        }
    }
}

/// `EnvironmentScanPlacement.modify` (kept apart: it moves the position).
fn scan(direction: Dir, target: &BlockPredicate, allowed: &BlockPredicate, max_steps: i32, r: &mut Region, p: BlockPos, out: &mut Vec<BlockPos>) {
    let mut q = p;
    if !allowed.test(r, q) {
        return;
    }
    for _ in 0..max_steps {
        if target.test(r, q) {
            out.push(q);
            return;
        }
        q = q.relative(direction);
        if r.is_outside_build_height(q.y) {
            return;
        }
        if !allowed.test(r, q) {
            break;
        }
    }
    if target.test(r, q) {
        out.push(q);
    }
}

fn heightmap(json: &Json) -> Result<Heightmap, Error> {
    json.as_str().and_then(Heightmap::parse).ok_or_else(|| Error::Invalid(format!("bad heightmap {json:?}")))
}

/// `CountOnEveryLayerPlacement.findOnGroundYPosition`.
fn on_ground_y(r: &mut Region, x: i32, top: i32, z: i32, target: i32) -> i32 {
    let empty = |s: u16| crate::blocks::is_air(s) || is_water(s) || is_lava(s);
    let mut layer = 0;
    let mut above = r.get(BlockPos::new(x, top, z));
    let mut y = top;
    while y > r.min_y() {
        let below = r.get(BlockPos::new(x, y - 1, z));
        if !empty(below) && empty(above) && !crate::blocks::is_block(below, "minecraft:bedrock") {
            if layer == target {
                return y;
            }
            layer += 1;
        }
        above = below;
        y -= 1;
    }
    i32::MAX
}

/// What placement modifiers see besides the level (`PlacementContext`).
pub struct PlaceCtx<'f> {
    pub features: &'f crate::feature::Features,
    /// The placed feature being placed with a biome check (`topFeature`).
    pub top: Option<usize>,
}

/// `PlacedFeature`.
#[derive(Clone, Debug)]
pub struct PlacedFeature {
    pub name: String,
    pub feature: usize,
    pub placement: Vec<Modifier>,
}

impl crate::feature::Features {
    /// `FeaturePlacer.place`: runs the modifiers depth first from `origin` and places the
    /// feature at every resulting position. `biome_check` names the placed feature for biome
    /// filters (`placeWithBiomeCheck`).
    pub fn place_placed(&self, placed: usize, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, biome_check: bool) -> bool {
        let pf = &self.placed[placed];
        if pf.placement.is_empty() {
            return self.place_feature(pf.feature, r, random, origin);
        }
        let mut cx = PlaceCtx { features: self, top: biome_check.then_some(placed) };
        let mut result = false;
        let mut stack: Vec<(BlockPos, usize)> = vec![(origin, 0)];
        let mut modified: Vec<BlockPos> = Vec::new();
        while let Some((p, i)) = stack.pop() {
            match &pf.placement[i] {
                Modifier::EnvironmentScan { direction, target, allowed, max_steps } => {
                    scan(*direction, target, allowed, *max_steps, r, p, &mut modified)
                }
                m => m.modify(&mut cx, r, random, p, &mut modified),
            }
            let next = i + 1;
            if next < pf.placement.len() {
                for q in modified.iter().rev() {
                    stack.push((*q, next));
                }
            } else {
                for q in &modified {
                    result |= self.place_feature(pf.feature, r, random, *q);
                }
            }
            modified.clear();
        }
        result
    }
}
