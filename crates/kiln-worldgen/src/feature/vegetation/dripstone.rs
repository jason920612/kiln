//! Pointed dripstone and its kin (sulfur spikes): `SpeleothemFeature` (one stalactite or
//! stalagmite), `SpeleothemClusterFeature` (a cave area full of them) and
//! `LargeDripstoneFeature` (a thick stalactite and stalagmite pair), with `SpeleothemUtils` and
//! `Column.scan`.

use super::field;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{block_state, is_air, is_block, is_lava, is_water, same_block, with_prop};
use crate::carver::{cos, sin};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::providers::{FloatProvider, IntProvider, between_inclusive, float, int, normal};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use kiln_javamath::math::clamp;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

/// `SpeleothemUtils.isEmptyOrWater`.
fn empty_or_water(s: u16) -> bool {
    is_air(s) || is_water(s)
}

/// `SpeleothemUtils.isEmptyOrWaterOrLava`.
fn empty_or_water_or_lava(s: u16) -> bool {
    is_air(s) || is_water(s) || is_lava(s)
}

/// The blocks of one speleothem kind: base block, pointed block, and what the base may replace.
#[derive(Debug)]
struct Kind {
    base: u16,
    pointed: u16,
    replaceable: Arc<BlockSet>,
}

impl Kind {
    fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            base: block_state(field(json, "base_block")?)?,
            pointed: block_state(field(json, "pointed_block")?)?,
            replaceable: l.blocks(field(json, "replaceable_blocks")?)?,
        })
    }

    /// `SpeleothemUtils.isBase`.
    fn is_base(&self, s: u16) -> bool {
        same_block(s, self.base) || self.replaceable.contains(s)
    }

    /// `SpeleothemUtils.placeBaseBlockIfPossible`.
    fn place_base(&self, r: &mut Region, p: BlockPos) -> bool {
        if self.replaceable.contains(r.get(p)) {
            r.set(p, self.base, 2);
            true
        } else {
            false
        }
    }

    /// `SpeleothemUtils.growSpeleothem`: a column of pointed blocks from `p` toward `dir`,
    /// base first, if the block behind `p` is a base.
    fn grow(&self, r: &mut Region, p: BlockPos, dir: Dir, height: i32, merge: bool) {
        if !self.is_base(r.get(p.relative(dir.opposite()))) {
            return;
        }
        let mut thicknesses = Vec::new();
        if height >= 3 {
            thicknesses.push("base");
            thicknesses.extend(std::iter::repeat_n("middle", (height - 3) as usize));
        }
        if height >= 2 {
            thicknesses.push("frustum");
        }
        if height >= 1 {
            thicknesses.push(if merge { "tip_merge" } else { "tip" });
        }
        let pointed = with_prop(self.pointed, "vertical_direction", dir.name());
        let mut q = p;
        for t in thicknesses {
            let s = with_prop(pointed, "thickness", t);
            let water = r.fluid(q).is_water();
            r.set(q, with_prop(s, "waterlogged", if water { "true" } else { "false" }), 2);
            q = q.relative(dir);
        }
    }
}

/// `Column`: the air (or water) gap around a position, with the solid edges found.
#[derive(Clone, Copy, Debug)]
struct Column {
    floor: Option<i32>,
    ceiling: Option<i32>,
}

impl Column {
    /// `Column.scan`.
    fn scan(r: &mut Region, p: BlockPos, range: i32, inside: impl Fn(u16) -> bool, edge: impl Fn(u16) -> bool) -> Option<Column> {
        if !inside(r.get(p)) {
            return None;
        }
        let mut scan_dir = |d: Dir| {
            let mut q = p;
            let mut i = 1;
            while i < range && inside(r.get(q)) {
                q = q.relative(d);
                i += 1;
            }
            edge(r.get(q)).then_some(q.y)
        };
        let ceiling = scan_dir(Dir::Up);
        let floor = scan_dir(Dir::Down);
        Some(Column { floor, ceiling })
    }

    /// `Column.getHeight`: only a range has one.
    fn height(self) -> Option<i32> {
        match (self.floor, self.ceiling) {
            (Some(f), Some(c)) => Some(c - f - 1),
            _ => None,
        }
    }
}

/// `SpeleothemFeature`.
#[derive(Debug)]
pub struct Speleothem {
    kind: Kind,
    chance_of_taller_generation: f32,
    chance_of_directional_spread: f32,
    chance_of_spread_radius2: f32,
    chance_of_spread_radius3: f32,
}

fn opt_float(json: &Json, key: &str, default: f32) -> f32 {
    json.get(key).and_then(Json::as_f32).unwrap_or(default)
}

impl Speleothem {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            kind: Kind::parse(json, l)?,
            chance_of_taller_generation: opt_float(json, "chance_of_taller_generation", 0.2),
            chance_of_directional_spread: opt_float(json, "chance_of_directional_spread", 0.7),
            chance_of_spread_radius2: opt_float(json, "chance_of_spread_radius2", 0.5),
            chance_of_spread_radius3: opt_float(json, "chance_of_spread_radius3", 0.5),
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let above = self.kind.is_base(r.get(origin.above()));
        let below = self.kind.is_base(r.get(origin.below()));
        let tip = match (above, below) {
            (true, true) => {
                if random.next_bool() {
                    Dir::Down
                } else {
                    Dir::Up
                }
            }
            (true, false) => Dir::Down,
            (false, true) => Dir::Up,
            (false, false) => return false,
        };
        self.base_patch(r, random, origin.relative(tip.opposite()));
        let height = if random.next_float() < self.chance_of_taller_generation && empty_or_water(r.get(origin.relative(tip))) {
            2
        } else {
            1
        };
        self.kind.grow(r, origin, tip, height, false);
        true
    }

    /// `createPatchOfBaseBlocks`.
    fn base_patch(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) {
        self.kind.place_base(r, p);
        for d in Dir::HORIZONTAL {
            if random.next_float() > self.chance_of_directional_spread {
                continue;
            }
            let p1 = p.relative(d);
            self.kind.place_base(r, p1);
            if random.next_float() > self.chance_of_spread_radius2 {
                continue;
            }
            let p2 = p1.relative(Dir::from_index(random.next_int_bounded(6) as usize));
            self.kind.place_base(r, p2);
            if random.next_float() > self.chance_of_spread_radius3 {
                continue;
            }
            let p3 = p2.relative(Dir::from_index(random.next_int_bounded(6) as usize));
            self.kind.place_base(r, p3);
        }
    }
}

/// `SpeleothemClusterFeature`.
#[derive(Debug)]
pub struct Cluster {
    kind: Kind,
    search_range: i32,
    height: IntProvider,
    radius: IntProvider,
    max_height_diff: i32,
    height_deviation: i32,
    layer_thickness: IntProvider,
    density: FloatProvider,
    wetness: FloatProvider,
    chance_at_max_distance: f32,
    max_distance_from_edge: i32,
    max_distance_from_center: i32,
}

impl Cluster {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            kind: Kind::parse(json, l)?,
            search_range: int(json, "floor_to_ceiling_search_range")?,
            height: IntProvider::parse(field(json, "height")?)?,
            radius: IntProvider::parse(field(json, "radius")?)?,
            max_height_diff: int(json, "max_stalagmite_stalactite_height_diff")?,
            height_deviation: int(json, "height_deviation")?,
            layer_thickness: IntProvider::parse(field(json, "speleothem_block_layer_thickness")?)?,
            density: FloatProvider::parse(field(json, "density")?)?,
            wetness: FloatProvider::parse(field(json, "wetness")?)?,
            chance_at_max_distance: float(json, "chance_of_speleothem_at_max_distance_from_center")?,
            max_distance_from_edge: int(json, "max_distance_from_edge_affecting_chance_of_speleothem")?,
            max_distance_from_center: int(json, "max_distance_from_center_affecting_height_bias")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !empty_or_water(r.get(origin)) {
            return false;
        }
        let height = self.height.sample(random);
        let wetness = self.wetness.sample(random);
        let density = self.density.sample(random);
        let rx = self.radius.sample(random);
        let rz = self.radius.sample(random);
        for dx in -rx..=rx {
            for dz in -rz..=rz {
                let chance = self.chance(rx, rz, dx, dz);
                self.place_column(r, random, origin.offset(dx, 0, dz), dx, dz, wetness, chance, height, density);
            }
        }
        true
    }

    /// `getChanceOfStalagmiteOrStalactite`.
    fn chance(&self, rx: i32, rz: i32, dx: i32, dz: i32) -> f64 {
        let edge = (rx - dx.abs()).min(rz - dz.abs());
        clamped_map_f32(edge as f32, 0.0, self.max_distance_from_edge as f32, self.chance_at_max_distance, 1.0) as f64
    }

    /// `getSpeleothemHeight`.
    fn speleothem_height(&self, random: &mut WorldgenRandom, dx: i32, dz: i32, density: f32, max: i32) -> i32 {
        if random.next_float() > density {
            return 0;
        }
        let d = dx.abs() + dz.abs();
        let mean = crate::state_provider::clamped_map(d as f64, 0.0, self.max_distance_from_center as f64, max as f64 / 2.0, 0.0) as f32;
        clamp(normal(random, mean, self.height_deviation as f32), 0.0, max as f32) as i32
    }

    #[allow(clippy::too_many_arguments)]
    fn place_column(
        &self,
        r: &mut Region,
        random: &mut WorldgenRandom,
        p: BlockPos,
        dx: i32,
        dz: i32,
        wetness: f32,
        chance: f64,
        height: i32,
        density: f32,
    ) {
        let Some(column) = Column::scan(r, p, self.search_range, empty_or_water, |s| !empty_or_water(s)) else { return };
        let ceiling = column.ceiling;
        if ceiling.is_none() && column.floor.is_none() {
            return;
        }
        let wet = random.next_float() < wetness;
        let column = match column.floor {
            Some(f) if wet && self.can_place_pool(r, p.at_y(f)) => {
                r.set(p.at_y(f), crate::blocks::state::WATER, 2);
                Column { floor: Some(f - 1), ..column }
            }
            _ => column,
        };
        let floor = column.floor;
        let place_stalactite = random.next_double() < chance;
        let stalactite = match ceiling {
            Some(c) if place_stalactite && !is_lava(r.get(p.at_y(c))) => {
                let thickness = self.layer_thickness.sample(random);
                self.replace_with_base(r, p.at_y(c), thickness, Dir::Up);
                let max = match floor {
                    Some(f) => height.min(c - f),
                    None => height,
                };
                self.speleothem_height(random, dx, dz, density, max)
            }
            _ => 0,
        };
        let place_stalagmite = random.next_double() < chance;
        let stalagmite = match floor {
            Some(f) if place_stalagmite && !is_lava(r.get(p.at_y(f))) => {
                let thickness = self.layer_thickness.sample(random);
                self.replace_with_base(r, p.at_y(f), thickness, Dir::Down);
                if ceiling.is_some() {
                    0.max(stalactite + between_inclusive(random, -self.max_height_diff, self.max_height_diff))
                } else {
                    self.speleothem_height(random, dx, dz, density, height)
                }
            }
            _ => 0,
        };
        let (down, up) = match (ceiling, floor) {
            (Some(c), Some(f)) if c - stalactite <= f + stalagmite => {
                let lo = (c - stalactite).max(f + 1);
                let hi = (f + stalagmite).min(c - 1);
                let k = between_inclusive(random, lo, hi + 1);
                (c - k, k - 1 - f)
            }
            _ => (stalactite, stalagmite),
        };
        let merge = random.next_bool() && down > 0 && up > 0 && column.height() == Some(down + up);
        if let Some(c) = ceiling {
            self.kind.grow(r, p.at_y(c - 1), Dir::Down, down, merge);
        }
        if let Some(f) = floor {
            self.kind.grow(r, p.at_y(f + 1), Dir::Up, up, merge);
        }
    }

    /// `canPlacePool`.
    fn can_place_pool(&self, r: &mut Region, p: BlockPos) -> bool {
        let s = r.get(p);
        if is_water(s) || same_block(s, self.kind.base) || same_block(s, self.kind.pointed) {
            return false;
        }
        if r.fluid(p.above()).is_water() {
            return false;
        }
        let adjacent_ok = |r: &mut Region, q: BlockPos| {
            let s = r.get(q);
            crate::vtags::is(s, "base_stone_overworld") || crate::block_facts::fluid(s).is_water()
        };
        for d in Dir::HORIZONTAL {
            if !adjacent_ok(r, p.relative(d)) {
                return false;
            }
        }
        adjacent_ok(r, p.below())
    }

    /// `replaceBlocksWithBaseBlocks`.
    fn replace_with_base(&self, r: &mut Region, p: BlockPos, n: i32, dir: Dir) {
        let mut q = p;
        for _ in 0..n {
            if !self.kind.place_base(r, q) {
                return;
            }
            q = q.relative(dir);
        }
    }
}

/// `Mth.clampedMap(float...)`.
fn clamped_map_f32(v: f32, from_lo: f32, from_hi: f32, to_lo: f32, to_hi: f32) -> f32 {
    let t = (v - from_lo) / (from_hi - from_lo);
    if t < 0.0 {
        to_lo
    } else if t > 1.0 {
        to_hi
    } else {
        to_lo + t * (to_hi - to_lo)
    }
}

/// `LargeDripstoneFeature`.
#[derive(Debug)]
pub struct Large {
    replaceable: Arc<BlockSet>,
    search_range: i32,
    column_radius: IntProvider,
    height_scale: FloatProvider,
    max_radius_ratio: f32,
    stalactite_bluntness: FloatProvider,
    stalagmite_bluntness: FloatProvider,
    wind_speed: FloatProvider,
    min_radius_for_wind: i32,
    min_bluntness_for_wind: f32,
}

/// `LargeDripstoneFeature.WindOffsetter`.
#[derive(Clone, Copy, Debug)]
struct Wind {
    origin_y: i32,
    speed: Option<(f64, f64)>,
    max_offset: i32,
}

impl Wind {
    fn offset(&self, p: BlockPos) -> BlockPos {
        let Some((sx, sz)) = self.speed else { return p };
        let dy = (self.origin_y - p.y) as f64;
        let x = kiln_javamath::math::floor(sx * dy).max(-self.max_offset).min(self.max_offset);
        let z = kiln_javamath::math::floor(sz * dy).max(-self.max_offset).min(self.max_offset);
        p.offset(x, 0, z)
    }
}

/// `LargeDripstoneFeature.LargeDripstone`.
struct LargeDripstone {
    root: BlockPos,
    pointing_up: bool,
    radius: i32,
    bluntness: f64,
    scale: f64,
}

/// `SpeleothemUtils.getSpeleothemHeight`.
fn speleothem_height(mut at: f64, radius: f64, scale: f64, bluntness: f64) -> f64 {
    if at < bluntness {
        at = bluntness;
    }
    let t = at / radius * 0.384;
    let a = 0.75 * kiln_javamath::pow::pow(t, 1.3333333333333333);
    let b = kiln_javamath::pow::pow(t, 0.6666666666666666);
    let c = 0.3333333333333333 * kiln_javamath::pow::log(t);
    let h = (scale * (a - b - c)).max(0.0);
    h / 0.384 * radius
}

/// `SpeleothemUtils.isCircleMostlyEmbeddedInStone`.
fn circle_embedded(r: &mut Region, p: BlockPos, radius: i32) -> bool {
    if empty_or_water_or_lava(r.get(p)) {
        return false;
    }
    let step = 6.0f32 / radius as f32;
    let mut a = 0.0f32;
    while a < 6.2831855f32 {
        let x = (cos(a as f64) * radius as f32) as i32;
        let z = (sin(a as f64) * radius as f32) as i32;
        if empty_or_water_or_lava(r.get(p.offset(x, 0, z))) {
            return false;
        }
        a += step;
    }
    true
}

impl LargeDripstone {
    fn height_at(&self, d: f32) -> i32 {
        speleothem_height(d as f64, self.radius as f64, self.scale, self.bluntness) as i32
    }

    fn suitable_for_wind(&self, min_radius: i32, min_bluntness: f32) -> bool {
        self.radius >= min_radius && self.bluntness >= min_bluntness as f64
    }

    /// `moveBackUntilBaseIsInsideStoneAndShrinkRadiusIfNecessary`.
    fn settle(&mut self, r: &mut Region, wind: &Wind) -> bool {
        while self.radius > 1 {
            let mut p = self.root;
            let n = 10.min(self.height_at(0.0));
            for _ in 0..n {
                if is_lava(r.get(p)) {
                    return false;
                }
                if circle_embedded(r, wind.offset(p), self.radius) {
                    self.root = p;
                    return true;
                }
                p = p.relative(if self.pointing_up { Dir::Down } else { Dir::Up });
            }
            self.radius /= 2;
        }
        false
    }

    /// `placeBlocks`.
    fn place(&self, r: &mut Region, random: &mut WorldgenRandom, wind: &Wind) {
        let dripstone = crate::blocks::state::DRIPSTONE_BLOCK;
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                if d > self.radius as f32 {
                    continue;
                }
                let mut h = self.height_at(d);
                if h <= 0 {
                    continue;
                }
                if (random.next_float() as f64) < 0.2 {
                    h = (h as f32 * (random.next_float() * (1.0 - 0.8) + 0.8)) as i32;
                }
                let mut p = self.root.offset(dx, 0, dz);
                let mut placed = false;
                let max_y = if self.pointing_up { r.height_at(Heightmap::WorldSurfaceWg, p.x, p.z) } else { i32::MAX };
                for _ in 0..h {
                    if p.y >= max_y {
                        break;
                    }
                    let q = wind.offset(p);
                    if empty_or_water_or_lava(r.get(q)) {
                        placed = true;
                        r.set(q, dripstone, 2);
                    } else if placed && crate::vtags::is(r.get(q), "base_stone_overworld") {
                        break;
                    }
                    p = p.relative(if self.pointing_up { Dir::Up } else { Dir::Down });
                }
            }
        }
    }
}

impl Large {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self {
            replaceable: l.blocks(field(json, "replaceable_blocks")?)?,
            search_range: json.get("floor_to_ceiling_search_range").and_then(Json::as_i32).unwrap_or(30),
            column_radius: IntProvider::parse(field(json, "column_radius")?)?,
            height_scale: FloatProvider::parse(field(json, "height_scale")?)?,
            max_radius_ratio: float(json, "max_column_radius_to_cave_height_ratio")?,
            stalactite_bluntness: FloatProvider::parse(field(json, "stalactite_bluntness")?)?,
            stalagmite_bluntness: FloatProvider::parse(field(json, "stalagmite_bluntness")?)?,
            wind_speed: FloatProvider::parse(field(json, "wind_speed")?)?,
            min_radius_for_wind: int(json, "min_radius_for_wind")?,
            min_bluntness_for_wind: float(json, "min_bluntness_for_wind")?,
        })
    }

    fn make(&self, root: BlockPos, pointing_up: bool, random: &mut WorldgenRandom, radius: i32, bluntness: &FloatProvider) -> LargeDripstone {
        let bluntness = bluntness.sample(random) as f64;
        let scale = self.height_scale.sample(random) as f64;
        LargeDripstone { root, pointing_up, radius, bluntness, scale }
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !empty_or_water(r.get(origin)) {
            return false;
        }
        let base_or_lava = |s: u16| is_block(s, "minecraft:dripstone_block") || self.replaceable.contains(s) || is_lava(s);
        let Some(Column { floor: Some(floor), ceiling: Some(ceiling) }) = Column::scan(r, origin, self.search_range, empty_or_water, base_or_lava)
        else {
            return false;
        };
        let height = ceiling - floor - 1;
        if height < 4 {
            return false;
        }
        let max_radius = (height as f32 * self.max_radius_ratio) as i32;
        let (lo, hi) = (self.column_radius.min_value(), self.column_radius.max_value());
        let max_radius = max_radius.max(lo).min(hi);
        let radius = between_inclusive(random, lo, max_radius);
        let mut stalactite = self.make(origin.at_y(ceiling - 1), false, random, radius, &self.stalactite_bluntness);
        let mut stalagmite = self.make(origin.at_y(floor + 1), true, random, radius, &self.stalagmite_bluntness);
        let wind = if stalactite.suitable_for_wind(self.min_radius_for_wind, self.min_bluntness_for_wind)
            && stalagmite.suitable_for_wind(self.min_radius_for_wind, self.min_bluntness_for_wind)
        {
            let speed = self.wind_speed.sample(random);
            let angle = random.next_float() * std::f32::consts::PI;
            Wind {
                origin_y: origin.y,
                speed: Some(((cos(angle as f64) * speed) as f64, (sin(angle as f64) * speed) as f64)),
                max_offset: 16 - radius,
            }
        } else {
            Wind { origin_y: 0, speed: None, max_offset: 0 }
        };
        let up = stalactite.settle(r, &wind);
        let down = stalagmite.settle(r, &wind);
        if up {
            stalactite.place(r, random, &wind);
        }
        if down {
            stalagmite.place(r, random, &wind);
        }
        true
    }
}
