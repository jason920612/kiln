//! Cave and canyon carvers (`CaveWorldCarver`, `CanyonWorldCarver`). In 26.3 carvers only
//! mark positions in a [`CarvingMask`]; the generator then fills the marked positions from
//! the aquifer (`NoiseBasedChunkGenerator.applyCarvingMask`).

use crate::Error;
use crate::function::field;
use crate::json::Json;
use crate::material::Anchor;
use kiln_javamath::math as jm;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::sync::OnceLock;

/// `Mth.SIN`: `sin(i / 10430.378350470453)` as `float` for every 16-bit angle.
fn sin_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| (0..65536).map(|i| (i as f64 / 10430.378350470453).sin() as f32).collect())
}

/// `Mth.sin(double)`.
#[inline]
pub fn sin(x: f64) -> f32 {
    sin_table()[((x * 10430.378350470453) as i64 & 65535) as usize]
}

/// `Mth.cos(double)`.
#[inline]
pub fn cos(x: f64) -> f32 {
    sin_table()[((x * 10430.378350470453 + 16384.0) as i64 & 65535) as usize]
}

/// `FloatProvider`.
#[derive(Clone, Debug)]
pub enum FloatProvider {
    Constant(f32),
    Uniform { min: f32, max: f32 },
    Trapezoid { min: f32, max: f32, plateau: f32 },
}

impl FloatProvider {
    pub fn parse(json: &Json) -> Result<FloatProvider, Error> {
        if let Some(v) = json.as_f32() {
            return Ok(FloatProvider::Constant(v));
        }
        let f = |k: &str| field(json, k)?.as_f32().ok_or_else(|| Error::Invalid(format!("bad {k}")));
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        Ok(match ty.trim_start_matches("minecraft:") {
            "constant" => FloatProvider::Constant(f("value")?),
            "uniform" => FloatProvider::Uniform { min: f("min_inclusive")?, max: f("max_exclusive")? },
            "trapezoid" => FloatProvider::Trapezoid { min: f("min")?, max: f("max")?, plateau: f("plateau")? },
            other => return Err(Error::Invalid(format!("unsupported float provider {other}"))),
        })
    }

    pub fn sample(&self, r: &mut impl RandomSource) -> f32 {
        match *self {
            FloatProvider::Constant(v) => v,
            FloatProvider::Uniform { min, max } => r.next_float() * (max - min) + min,
            FloatProvider::Trapezoid { min, max, plateau } => {
                let range = max - min;
                let slope = (range - plateau) / 2.0;
                let top = range - slope;
                min + r.next_float() * top + r.next_float() * slope
            }
        }
    }
}

/// `IntProvider`.
#[derive(Clone, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: i32, max: i32 },
    BiasedToBottom { min: i32, max: i32 },
    VeryBiasedToBottom { min: i32, max: i32 },
}

impl IntProvider {
    pub fn parse(json: &Json) -> Result<IntProvider, Error> {
        if let Some(v) = json.as_i32() {
            return Ok(IntProvider::Constant(v));
        }
        let i = |k: &str| field(json, k)?.as_i32().ok_or_else(|| Error::Invalid(format!("bad {k}")));
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        Ok(match ty.trim_start_matches("minecraft:") {
            "constant" => IntProvider::Constant(i("value")?),
            "uniform" => IntProvider::Uniform { min: i("min_inclusive")?, max: i("max_inclusive")? },
            "biased_to_bottom" => IntProvider::BiasedToBottom { min: i("min_inclusive")?, max: i("max_inclusive")? },
            "very_biased_to_bottom" => IntProvider::VeryBiasedToBottom { min: i("min_inclusive")?, max: i("max_inclusive")? },
            other => return Err(Error::Invalid(format!("unsupported int provider {other}"))),
        })
    }

    pub fn sample(&self, r: &mut impl RandomSource) -> i32 {
        match *self {
            IntProvider::Constant(v) => v,
            IntProvider::Uniform { min, max } => r.next_int_bounded(max - min + 1) + min,
            IntProvider::BiasedToBottom { min, max } => {
                let inner = r.next_int_bounded(max - min + 1) + 1;
                min + r.next_int_bounded(inner)
            }
            IntProvider::VeryBiasedToBottom { min, max } => {
                let a = r.next_int_bounded(max - min + 1) + 1;
                let b = r.next_int_bounded(a) + 1;
                min + r.next_int_bounded(b)
            }
        }
    }
}

/// `HeightProvider` (the kinds carvers use).
#[derive(Clone, Debug)]
pub enum HeightProvider {
    Constant(Anchor),
    Uniform { min: Anchor, max: Anchor },
}

impl HeightProvider {
    pub fn parse(json: &Json) -> Result<HeightProvider, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        Ok(match ty.trim_start_matches("minecraft:") {
            "constant" => HeightProvider::Constant(Anchor::parse(field(json, "value")?)?),
            "uniform" => HeightProvider::Uniform {
                min: Anchor::parse(field(json, "min_inclusive")?)?,
                max: Anchor::parse(field(json, "max_inclusive")?)?,
            },
            "" => HeightProvider::Constant(Anchor::parse(json)?),
            other => return Err(Error::Invalid(format!("unsupported height provider {other}"))),
        })
    }

    pub fn sample(&self, r: &mut impl RandomSource, g: &GenContext) -> i32 {
        match self {
            HeightProvider::Constant(a) => g.resolve(*a),
            HeightProvider::Uniform { min, max } => {
                let (lo, hi) = (g.resolve(*min), g.resolve(*max));
                if lo > hi { lo } else { r.next_int_bounded(hi - lo + 1) + lo }
            }
        }
    }
}

/// `WorldGenerationContext`.
#[derive(Clone, Copy, Debug)]
pub struct GenContext {
    pub min_y: i32,
    pub height: i32,
    pub sea_level: i32,
}

impl GenContext {
    pub fn resolve(&self, a: Anchor) -> i32 {
        a.resolve(self.min_y, self.height, self.sea_level)
    }
}

/// `CarvingMask`: carved positions of one chunk, local x/z, absolute y in `min_y..=max_y`.
pub struct CarvingMask {
    pub min_y: i32,
    pub max_y: i32,
    height: i32,
    bits: Vec<u64>,
}

impl CarvingMask {
    pub fn new(min_y: i32, max_y: i32) -> Self {
        let height = max_y - min_y + 1;
        Self { min_y, max_y, height, bits: vec![0; (256 * height as usize).div_ceil(64)] }
    }

    #[inline]
    pub fn carve(&mut self, x: i32, y: i32, z: i32) {
        let i = (y - self.min_y + (z + (x << 4)) * self.height) as usize;
        self.bits[i >> 6] |= 1 << (i & 63);
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&b| b == 0)
    }

    #[inline]
    fn get(&self, i: usize) -> bool {
        self.bits[i >> 6] & (1 << (i & 63)) != 0
    }

    /// `CarvingMask.visit`: runs of set bits in index order, split into columns; calls
    /// `f(x, z, min_y, max_y)`.
    pub fn visit(&self, f: &mut dyn FnMut(i32, i32, i32, i32)) {
        let n = 256 * self.height as usize;
        let mut i = 0;
        while i < n {
            if !self.get(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && self.get(i) {
                i += 1;
            }
            let end = i - 1;
            let (h, s, e) = (self.height, start as i32, end as i32);
            for column in s / h..=e / h {
                let base = column * h;
                f((column >> 4) & 15, column & 15, (s - base).max(0) + self.min_y, (e - base).min(h - 1) + self.min_y);
            }
        }
    }
}

/// A configured carver.
#[derive(Clone, Debug)]
pub enum Carver {
    Cave(Cave),
    Canyon(Canyon),
}

#[derive(Clone, Debug)]
pub struct Cave {
    probability: f32,
    y: HeightProvider,
    count: IntProvider,
    thickness: FloatProvider,
    weird_thickness_bias: bool,
    room_vertical_radius_multiplier: FloatProvider,
    horizontal_radius_multiplier: FloatProvider,
    vertical_radius_multiplier: FloatProvider,
    start_vertical_radius_multiplier: FloatProvider,
    floor_level: FloatProvider,
}

#[derive(Clone, Debug)]
pub struct Canyon {
    probability: f32,
    y: HeightProvider,
    vertical_rotation: FloatProvider,
    distance_factor: FloatProvider,
    thickness: FloatProvider,
    width_smoothness: i32,
    horizontal_radius_factor: FloatProvider,
    vertical_radius_default_factor: f32,
    vertical_radius_center_factor: f32,
    y_scale: FloatProvider,
}

/// `WorldCarver.getRange()` chunks, times two minus one, in blocks.
const RANGE_BLOCKS: i32 = (4 * 2 - 1) * 16;

impl Carver {
    pub fn parse(json: &Json) -> Result<Carver, Error> {
        let fp = |j: &Json, k: &str| FloatProvider::parse(field(j, k)?).map_err(|e| e.context(k));
        let probability = field(json, "probability")?.as_f32().ok_or_else(|| Error::Invalid("bad probability".into()))?;
        let y = HeightProvider::parse(field(json, "y")?)?;
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        Ok(match ty.trim_start_matches("minecraft:") {
            "cave" => Carver::Cave(Cave {
                probability,
                y,
                count: IntProvider::parse(field(json, "count")?)?,
                thickness: fp(json, "thickness")?,
                weird_thickness_bias: json.get("weird_thickness_bias").and_then(Json::as_bool).unwrap_or(false),
                room_vertical_radius_multiplier: fp(json, "room_vertical_radius_multiplier")?,
                horizontal_radius_multiplier: fp(json, "horizontal_radius_multiplier")?,
                vertical_radius_multiplier: fp(json, "vertical_radius_multiplier")?,
                start_vertical_radius_multiplier: match json.get("start_vertical_radius_multiplier") {
                    Some(v) => FloatProvider::parse(v)?,
                    None => FloatProvider::Constant(1.0),
                },
                floor_level: fp(json, "floor_level")?,
            }),
            "canyon" => {
                let shape = field(json, "shape")?;
                let f = |k: &str| field(shape, k)?.as_f32().ok_or_else(|| Error::Invalid(format!("bad {k}")));
                Carver::Canyon(Canyon {
                    probability,
                    y,
                    vertical_rotation: fp(json, "vertical_rotation")?,
                    distance_factor: fp(shape, "distance_factor")?,
                    thickness: fp(shape, "thickness")?,
                    width_smoothness: field(shape, "width_smoothness")?
                        .as_i32()
                        .filter(|&w| w > 0)
                        .ok_or_else(|| Error::Invalid("bad width_smoothness".into()))?,
                    horizontal_radius_factor: fp(shape, "horizontal_radius_factor")?,
                    vertical_radius_default_factor: f("vertical_radius_default_factor")?,
                    vertical_radius_center_factor: f("vertical_radius_center_factor")?,
                    y_scale: fp(shape, "y_scale")?,
                })
            }
            other => return Err(Error::Invalid(format!("unsupported carver type {other}"))),
        })
    }

    /// `isStartChunk`.
    pub fn is_start_chunk(&self, r: &mut impl RandomSource) -> bool {
        let p = match self {
            Carver::Cave(c) => c.probability,
            Carver::Canyon(c) => c.probability,
        };
        r.next_float() <= p
    }

    /// `carve`: marks what the carver started in chunk `(sx, sz)` cuts out of chunk `(cx, cz)`.
    #[allow(clippy::too_many_arguments)]
    pub fn carve(&self, g: &GenContext, r: &mut impl RandomSource, cx: i32, cz: i32, sx: i32, sz: i32, mask: &mut CarvingMask) {
        let target = Target { x: cx, z: cz };
        match self {
            Carver::Cave(c) => c.carve(g, r, target, sx, sz, mask),
            Carver::Canyon(c) => c.carve(g, r, target, sx, sz, mask),
        }
    }
}

/// The chunk being carved.
#[derive(Clone, Copy)]
struct Target {
    x: i32,
    z: i32,
}

impl Target {
    fn middle_x(self) -> f64 {
        ((self.x << 4) + 8) as f64
    }

    fn middle_z(self) -> f64 {
        ((self.z << 4) + 8) as f64
    }
}

/// `WorldCarver.canReach`.
fn can_reach(t: Target, x: f64, z: f64, step: i32, end: i32, thickness: f32) -> bool {
    let dx = x - t.middle_x();
    let dz = z - t.middle_z();
    let remaining = (end - step) as f64;
    let reach = (thickness + 2.0 + 16.0) as f64;
    dx * dx + dz * dz - remaining * remaining <= reach * reach
}

/// `WorldCarver.carveEllipsoid`.
#[allow(clippy::too_many_arguments)]
fn carve_ellipsoid(
    t: Target,
    x: f64,
    y: f64,
    z: f64,
    hr: f64,
    vr: f64,
    mask: &mut CarvingMask,
    skip: &dyn Fn(f64, f64, f64, i32) -> bool,
) {
    let limit = 16.0 + hr * 2.0;
    if (x - t.middle_x()).abs() > limit || (z - t.middle_z()).abs() > limit {
        return;
    }
    let (min_x, min_z) = (t.x << 4, t.z << 4);
    let x0 = (jm::floor(x - hr) - min_x - 1).max(0);
    let x1 = (jm::floor(x + hr) - min_x).min(15);
    let y0 = (jm::floor(y - vr) - 1).max(mask.min_y);
    let y1 = (jm::floor(y + vr) + 1).min(mask.max_y);
    let z0 = (jm::floor(z - hr) - min_z - 1).max(0);
    let z1 = (jm::floor(z + hr) - min_z).min(15);
    for lx in x0..=x1 {
        let dx = ((min_x + lx) as f64 + 0.5 - x) / hr;
        for lz in z0..=z1 {
            let dz = ((min_z + lz) as f64 + 0.5 - z) / hr;
            if !(dx * dx + dz * dz < 1.0) {
                continue;
            }
            let mut by = y1;
            while by > y0 {
                let dy = (by as f64 - 0.5 - y) / vr;
                if !skip(dx, dy, dz, by) {
                    mask.carve(lx, by, lz);
                }
                by -= 1;
            }
        }
    }
}

impl Cave {
    fn thickness(&self, r: &mut impl RandomSource) -> f32 {
        let mut t = self.thickness.sample(r);
        if self.weird_thickness_bias && r.next_int_bounded(10) == 0 {
            t *= r.next_float() * r.next_float() * 3.0 + 1.0;
        }
        t
    }

    fn carve(&self, g: &GenContext, r: &mut impl RandomSource, t: Target, sx: i32, sz: i32, mask: &mut CarvingMask) {
        let count = self.count.sample(r);
        for _ in 0..count {
            let x = ((sx << 4) + r.next_int_bounded(16)) as f64;
            let y = self.y.sample(r, g) as f64;
            let z = ((sz << 4) + r.next_int_bounded(16)) as f64;
            let hr = self.horizontal_radius_multiplier.sample(r) as f64;
            let vr = self.vertical_radius_multiplier.sample(r) as f64;
            let start_vr = self.start_vertical_radius_multiplier.sample(r) as f64;
            let floor = self.floor_level.sample(r) as f64;
            let skip = move |dx: f64, dy: f64, dz: f64, _y: i32| dy <= floor || dx * dx + dy * dy + dz * dz >= 1.0;
            let mut tunnels = 1;
            if r.next_int_bounded(4) == 0 {
                let room_vr = self.room_vertical_radius_multiplier.sample(r) as f64;
                let thickness = 1.0 + r.next_float() * 6.0;
                let rhr = 1.5 + (sin(1.570_796_370_506_286_6) * thickness) as f64;
                carve_ellipsoid(t, x + 1.0, y, z, rhr, rhr * room_vr, mask, &skip);
                tunnels += r.next_int_bounded(4);
            }
            for _ in 0..tunnels {
                let yaw = r.next_float() * std::f32::consts::TAU;
                let pitch = (r.next_float() - 0.5) / 4.0;
                let thickness = self.thickness(r);
                let end = RANGE_BLOCKS - r.next_int_bounded(RANGE_BLOCKS / 4);
                let seed = r.next_long();
                let tunnel = Tunnel { t, hr, vr, skip: &skip };
                tunnel.run(mask, seed, [x, y, z], thickness, yaw, pitch, 0, end, start_vr);
            }
        }
    }
}

struct Tunnel<'a> {
    t: Target,
    hr: f64,
    vr: f64,
    skip: &'a dyn Fn(f64, f64, f64, i32) -> bool,
}

impl Tunnel<'_> {
    /// `CaveWorldCarver.createTunnel`.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        mask: &mut CarvingMask,
        seed: i64,
        pos: [f64; 3],
        thickness: f32,
        mut yaw: f32,
        mut pitch: f32,
        start: i32,
        end: i32,
        vr_scale: f64,
    ) {
        let [mut x, mut y, mut z] = pos;
        let mut r = LegacyRandom::new(seed);
        let split = r.next_int_bounded(end / 2) + end / 4;
        let steep = r.next_int_bounded(6) == 0;
        let mut yaw_velocity = 0f32;
        let mut pitch_velocity = 0f32;
        for step in start..end {
            let radius = 1.5 + (sin((std::f32::consts::PI * step as f32 / end as f32) as f64) * thickness) as f64;
            let vertical = radius * vr_scale;
            let cos_pitch = cos(pitch as f64);
            x += (cos(yaw as f64) * cos_pitch) as f64;
            y += sin(pitch as f64) as f64;
            z += (sin(yaw as f64) * cos_pitch) as f64;
            pitch *= if steep { 0.92 } else { 0.7 };
            pitch += pitch_velocity * 0.1;
            yaw += yaw_velocity * 0.1;
            pitch_velocity *= 0.9;
            yaw_velocity *= 0.75;
            pitch_velocity += (r.next_float() - r.next_float()) * r.next_float() * 2.0;
            yaw_velocity += (r.next_float() - r.next_float()) * r.next_float() * 4.0;
            if step == split && thickness > 1.0 {
                let seed = r.next_long();
                let t = r.next_float() * 0.5 + 0.5;
                self.run(mask, seed, [x, y, z], t, yaw - std::f32::consts::FRAC_PI_2, pitch / 3.0, step, end, 1.0);
                let seed = r.next_long();
                let t = r.next_float() * 0.5 + 0.5;
                self.run(mask, seed, [x, y, z], t, yaw + std::f32::consts::FRAC_PI_2, pitch / 3.0, step, end, 1.0);
                return;
            }
            if r.next_int_bounded(4) != 0 {
                if !can_reach(self.t, x, z, step, end, thickness) {
                    return;
                }
                carve_ellipsoid(self.t, x, y, z, radius * self.hr, vertical * self.vr, mask, self.skip);
            }
        }
    }
}

impl Canyon {
    fn carve(&self, g: &GenContext, r: &mut impl RandomSource, t: Target, sx: i32, sz: i32, mask: &mut CarvingMask) {
        let x = ((sx << 4) + r.next_int_bounded(16)) as f64;
        let y = self.y.sample(r, g);
        let z = ((sz << 4) + r.next_int_bounded(16)) as f64;
        let yaw = r.next_float() * std::f32::consts::TAU;
        let pitch = self.vertical_rotation.sample(r);
        let y_scale = self.y_scale.sample(r) as f64;
        let thickness = self.thickness.sample(r);
        let end = (RANGE_BLOCKS as f32 * self.distance_factor.sample(r)) as i32;
        let seed = r.next_long();
        self.run(g, t, seed, [x, y as f64, z], thickness, yaw, pitch, 0, end, y_scale, mask);
    }

    /// `CanyonWorldCarver.doCarve`.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        g: &GenContext,
        t: Target,
        seed: i64,
        pos: [f64; 3],
        thickness: f32,
        mut yaw: f32,
        mut pitch: f32,
        start: i32,
        end: i32,
        y_scale: f64,
        mask: &mut CarvingMask,
    ) {
        let [mut x, mut y, mut z] = pos;
        let mut r = LegacyRandom::new(seed);
        let widths = self.width_factors(g, &mut r);
        let min_y = g.min_y;
        let skip = |dx: f64, dy: f64, dz: f64, y: i32| {
            (dx * dx + dz * dz) * widths[(y - min_y - 1) as usize] as f64 + dy * dy / 6.0 >= 1.0
        };
        let mut yaw_velocity = 0f32;
        let mut pitch_velocity = 0f32;
        for step in start..end {
            let mut hr = 1.5 + (sin((step as f32 * std::f32::consts::PI / end as f32) as f64) * thickness) as f64;
            let vr = hr * y_scale;
            hr *= self.horizontal_radius_factor.sample(&mut r) as f64;
            let vr = self.vertical_radius(&mut r, vr, end as f32, step as f32);
            let cos_pitch = cos(pitch as f64);
            let sin_pitch = sin(pitch as f64);
            x += (cos(yaw as f64) * cos_pitch) as f64;
            y += sin_pitch as f64;
            z += (sin(yaw as f64) * cos_pitch) as f64;
            pitch *= 0.7;
            pitch += pitch_velocity * 0.05;
            yaw += yaw_velocity * 0.05;
            pitch_velocity *= 0.8;
            yaw_velocity *= 0.5;
            pitch_velocity += (r.next_float() - r.next_float()) * r.next_float() * 2.0;
            yaw_velocity += (r.next_float() - r.next_float()) * r.next_float() * 4.0;
            if r.next_int_bounded(4) != 0 {
                if !can_reach(t, x, z, step, end, thickness) {
                    return;
                }
                carve_ellipsoid(t, x, y, z, hr, vr, mask, &skip);
            }
        }
    }

    /// `initWidthFactors`.
    fn width_factors(&self, g: &GenContext, r: &mut impl RandomSource) -> Vec<f32> {
        let mut f = 1.0f32;
        (0..g.height)
            .map(|i| {
                if i == 0 || r.next_int_bounded(self.width_smoothness) == 0 {
                    f = 1.0 + r.next_float() * r.next_float();
                }
                f * f
            })
            .collect()
    }

    /// `updateVerticalRadius`.
    fn vertical_radius(&self, r: &mut impl RandomSource, vr: f64, end: f32, step: f32) -> f64 {
        let f = 1.0 - (0.5 - step / end).abs() * 2.0;
        let factor = self.vertical_radius_default_factor + self.vertical_radius_center_factor * f;
        factor as f64 * vr * (r.next_float() * (1.0 - 0.75) + 0.75) as f64
    }
}
