//! `Aquifer.NoiseBasedAquifer`: which fluid (or barrier stone) fills each open position.
//!
//! Aquifer centers sit on a jittered 16×12×16 grid; a position takes the fluid status of its
//! nearest center, blended against the next ones with a pressure term that builds barriers
//! between different fluid levels. A center's fluid level comes from the preliminary surface
//! height around it and the floodedness/spread noises; lava replaces water deep down.

use crate::blocks::{is_air, state};
use crate::sampler::{SamplerRef, Scratch};
use crate::volume::Volume;
use kiln_javamath::math as jm;
use kiln_javamath::random::{PositionalRandomFactory, RandomSource};
use std::collections::HashMap;

/// `DimensionType.WAY_BELOW_MIN_Y`.
pub const WAY_BELOW_MIN_Y: i32 = -2032 << 4;

/// `Aquifer.FluidStatus`: `fluid` below `level`, air at and above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidStatus {
    pub level: i32,
    pub fluid: u16,
}

impl FluidStatus {
    #[inline]
    pub fn at(self, y: i32) -> u16 {
        if y < self.level { self.fluid } else { state::AIR }
    }
}

/// `NoiseBasedChunkGenerator.createFluidPicker`: lava up to y = -54 below
/// `min(-54, sea level)`, else the default fluid up to sea level.
#[derive(Clone, Copy, Debug)]
pub struct FluidPicker {
    pub lava: FluidStatus,
    pub sea: FluidStatus,
}

impl FluidPicker {
    pub fn new(sea_level: i32, default_fluid: u16) -> Self {
        Self { lava: FluidStatus { level: -54, fluid: state::LAVA }, sea: FluidStatus { level: sea_level, fluid: default_fluid } }
    }

    #[inline]
    pub fn compute(&self, _x: i32, y: i32, _z: i32) -> FluidStatus {
        if y < (-54).min(self.sea.level) { self.lava } else { self.sea }
    }
}

/// The density functions of `noise_settings.aquifers` plus the aquifer random factory.
pub struct AquiferFunctions {
    pub barrier: SamplerRef,
    pub floodedness: SamplerRef,
    pub spread: SamplerRef,
    pub lava: SamplerRef,
    pub exclusion: SamplerRef,
    pub surface_level: SamplerRef,
    /// `RandomState.getOrCreateRandomFactory("minecraft:aquifer")`.
    pub random: PositionalRandomFactory,
}

const SURFACE_SAMPLING_OFFSETS: [[i32; 2]; 13] =
    [[0, 0], [-2, -1], [-1, -1], [0, -1], [1, -1], [-3, 0], [-2, 0], [-1, 0], [1, 0], [-2, 1], [-1, 1], [0, 1], [1, 1]];

#[inline]
fn grid_x(x: i32) -> i32 {
    x >> 4
}

#[inline]
fn from_grid_x(g: i32, offset: i32) -> i32 {
    (g << 4) + offset
}

#[inline]
fn grid_y(y: i32) -> i32 {
    jm::floor_div(y, 12)
}

#[inline]
fn from_grid_y(g: i32, offset: i32) -> i32 {
    g * 12 + offset
}

/// `FLOWING_UPDATE_SIMULARITY` = `similarity(10², 12²)`.
const FLOWING_UPDATE_SIMILARITY: f64 = 1.0 - (144 - 100) as f64 / 25.0;

#[inline]
fn similarity(a: i32, b: i32) -> f64 {
    1.0 - (b - a) as f64 / 25.0
}

#[inline]
fn quart_block(v: i32) -> i32 {
    (v >> 2) << 2
}

/// The aquifer of one chunk (one per `NoiseChunk`).
pub enum Aquifer<'a> {
    Disabled(FluidPicker),
    Noise(Box<NoiseAquifer<'a>>),
}

impl Aquifer<'_> {
    /// `Aquifer.computeSubstance`: the fluid or air at an open position, or `None` for the
    /// default block (positive density, or an aquifer barrier).
    pub fn compute_substance(&mut self, s: &mut Scratch, x: i32, y: i32, z: i32, density: f64) -> Option<u16> {
        match self {
            Aquifer::Disabled(p) => {
                if density > 0.0 {
                    None
                } else {
                    Some(p.compute(x, y, z).at(y))
                }
            }
            Aquifer::Noise(a) => a.compute_substance(s, x, y, z, density),
        }
    }

    /// `Aquifer.shouldScheduleFluidUpdate`: whether the last computed fluid borders a
    /// different aquifer and must flow once the chunk is loaded.
    pub fn should_schedule_fluid_update(&self) -> bool {
        match self {
            Aquifer::Disabled(_) => false,
            Aquifer::Noise(a) => a.schedule,
        }
    }
}

pub struct NoiseAquifer<'a> {
    f: &'a AquiferFunctions,
    picker: FluidPicker,
    min_grid: [i32; 3],
    size_x: i32,
    size_z: i32,
    status: Vec<Option<FluidStatus>>,
    location: Vec<Option<[i32; 3]>>,
    skip_sampling_above_y: i32,
    surface_levels: HashMap<(i32, i32), i32>,
    schedule: bool,
}

impl<'a> NoiseAquifer<'a> {
    /// The constructor: grid bounds from the chunk volume, then the highest preliminary
    /// surface around the chunk, above which no aquifer is sampled.
    pub fn new(f: &'a AquiferFunctions, picker: FluidPicker, s: &mut Scratch, vol: &Volume) -> Self {
        let min_x = grid_x(vol.min[0] - 5);
        let max_x = grid_x(vol.max_block(0) - 5) + 1;
        let min_y = grid_y(vol.min[1] + 1) - 1;
        let max_y = grid_y(vol.max_block(1) + 1) + 1;
        let min_z = grid_x(vol.min[2] - 5);
        let max_z = grid_x(vol.max_block(2) - 5) + 1;
        let size_x = max_x - min_x + 1;
        let size_y = max_y - min_y + 1;
        let size_z = max_z - min_z + 1;
        let n = (size_x * size_y * size_z) as usize;
        let mut a = NoiseAquifer {
            f,
            picker,
            min_grid: [min_x, min_y, min_z],
            size_x,
            size_z,
            status: vec![None; n],
            location: vec![None; n],
            skip_sampling_above_y: 0,
            surface_levels: HashMap::new(),
            schedule: false,
        };
        let max_surface = a.max_surface_level(
            s,
            from_grid_x(min_x, 0),
            from_grid_x(min_z, 0),
            from_grid_x(max_x, 9),
            from_grid_x(max_z, 9),
        ) + 8;
        let top_grid = grid_y(max_surface + 12) + 1;
        a.skip_sampling_above_y = from_grid_y(top_grid, 11) - 1;
        a
    }

    /// `maxSurfaceLevel`: samples the surface level on the quart grid of the box in volume
    /// mode, caching every column.
    fn max_surface_level(&mut self, s: &mut Scratch, x0: i32, z0: i32, x1: i32, z1: i32) -> i32 {
        let (qx0, qx1, qz0, qz1) = (x0 >> 2, x1 >> 2, z0 >> 2, z1 >> 2);
        let vol = Volume::new([qx1 - qx0 + 1, 1, qz1 - qz0 + 1], [qx0 << 2, 0, qz0 << 2], [4, 1, 4]);
        let mut buf = vec![0f32; vol.len()];
        self.f.surface_level.fill(s, &vol, &mut buf);
        let mut max = i32::MIN;
        for zi in 0..vol.size[2] {
            for xi in 0..vol.size[0] {
                let v = jm::floor_f32(buf[vol.index(xi, 0, zi)]);
                self.surface_levels.insert((vol.block_x(xi), vol.block_z(zi)), v);
                max = max.max(v);
            }
        }
        max
    }

    /// `surfaceLevel`: the floored preliminary surface at the quart column, point-sampled.
    fn surface_level(&mut self, s: &mut Scratch, x: i32, z: i32) -> i32 {
        let key = (quart_block(x), quart_block(z));
        if let Some(&v) = self.surface_levels.get(&key) {
            return v;
        }
        let v = jm::floor_f32(self.f.surface_level.point(s, key.0, 0, key.1));
        self.surface_levels.insert(key, v);
        v
    }

    #[inline]
    fn index(&self, gx: i32, gy: i32, gz: i32) -> usize {
        let (x, y, z) = (gx - self.min_grid[0], gy - self.min_grid[1], gz - self.min_grid[2]);
        ((y * self.size_z + z) * self.size_x + x) as usize
    }

    fn compute_substance(&mut self, s: &mut Scratch, x: i32, y: i32, z: i32, density: f64) -> Option<u16> {
        let (block, schedule) = self.substance(s, x, y, z, density);
        self.schedule = schedule;
        block
    }

    /// `computeSubstance` proper: the substance plus the new `shouldScheduleFluidUpdate`.
    fn substance(&mut self, s: &mut Scratch, x: i32, y: i32, z: i32, density: f64) -> (Option<u16>, bool) {
        if density > 0.0 {
            return (None, false);
        }
        let global = self.picker.compute(x, y, z);
        if y > self.skip_sampling_above_y {
            return (Some(global.at(y)), false);
        }
        if global.at(y) == state::LAVA {
            return (Some(state::LAVA), false);
        }
        let (gx, gy, gz) = (grid_x(x - 5), grid_y(y + 1), grid_x(z - 5));
        let mut dist = [i32::MAX; 4];
        let mut closest = [0usize; 4];
        for dx in 0..=1 {
            for dy in -1..=1 {
                for dz in 0..=1 {
                    let (cx, cy, cz) = (gx + dx, gy + dy, gz + dz);
                    let i = self.index(cx, cy, cz);
                    let center = match self.location[i] {
                        Some(c) => c,
                        None => {
                            let mut r = self.f.random.at(cx, cy, cz);
                            let c = [
                                from_grid_x(cx, r.next_int_bounded(10)),
                                from_grid_y(cy, r.next_int_bounded(9)),
                                from_grid_x(cz, r.next_int_bounded(10)),
                            ];
                            self.location[i] = Some(c);
                            c
                        }
                    };
                    let (ox, oy, oz) = (center[0] - x, center[1] - y, center[2] - z);
                    let d = ox * ox + oy * oy + oz * oz;
                    if dist[0] >= d {
                        closest = [i, closest[0], closest[1], closest[2]];
                        dist = [d, dist[0], dist[1], dist[2]];
                    } else if dist[1] >= d {
                        closest = [closest[0], i, closest[1], closest[2]];
                        dist = [dist[0], d, dist[1], dist[2]];
                    } else if dist[2] >= d {
                        closest[3] = closest[2];
                        closest[2] = i;
                        dist[3] = dist[2];
                        dist[2] = d;
                    } else if dist[3] >= d {
                        closest[3] = i;
                        dist[3] = d;
                    }
                }
            }
        }
        let status1 = self.status_at(s, closest[0]);
        let sim12 = similarity(dist[0], dist[1]);
        let block = status1.at(y);
        if sim12 <= 0.0 {
            let schedule = sim12 >= FLOWING_UPDATE_SIMILARITY && status1 != self.status_at(s, closest[1]);
            return (Some(block), schedule);
        }
        if block == state::WATER && self.picker.compute(x, y - 1, z).at(y - 1) == state::LAVA {
            return (Some(block), true);
        }
        let mut barrier = f64::NAN;
        let status2 = self.status_at(s, closest[1]);
        let p = sim12 * self.pressure(s, x, y, z, &mut barrier, status1, status2);
        if density + p > 0.0 {
            return (None, false);
        }
        let status3 = self.status_at(s, closest[2]);
        let sim13 = similarity(dist[0], dist[2]);
        if sim13 > 0.0 {
            let p = sim12 * sim13 * self.pressure(s, x, y, z, &mut barrier, status1, status3);
            if density + p > 0.0 {
                return (None, false);
            }
        }
        let sim23 = similarity(dist[1], dist[2]);
        if sim23 > 0.0 {
            let p = sim12 * sim23 * self.pressure(s, x, y, z, &mut barrier, status2, status3);
            if density + p > 0.0 {
                return (None, false);
            }
        }
        let flowing = |sim: f64| sim >= FLOWING_UPDATE_SIMILARITY;
        let schedule = status1 != status2
            || (flowing(sim23) && status2 != status3)
            || (flowing(sim13) && status1 != status3)
            || (flowing(sim13)
                && flowing(similarity(dist[0], dist[3]))
                && status1 != self.status_at(s, closest[3]));
        (Some(block), schedule)
    }

    /// `calculatePressure`.
    #[allow(clippy::too_many_arguments)]
    fn pressure(&self, s: &mut Scratch, x: i32, y: i32, z: i32, barrier: &mut f64, a: FluidStatus, b: FluidStatus) -> f64 {
        let (fa, fb) = (a.at(y), b.at(y));
        if (fa == state::LAVA && fb == state::WATER) || (fa == state::WATER && fb == state::LAVA) {
            return 2.0;
        }
        let diff = (a.level - b.level).abs();
        if diff == 0 {
            return 0.0;
        }
        let mid = 0.5 * (a.level + b.level) as f64;
        let offset = y as f64 + 0.5 - mid;
        let half = diff as f64 / 2.0;
        let q = half - offset.abs();
        let pressure = if offset > 0.0 {
            let t = 0.0 + q;
            if t > 0.0 { t / 1.5 } else { t / 2.5 }
        } else {
            let t = 3.0 + q;
            if t > 0.0 { t / 3.0 } else { t / 10.0 }
        };
        let noise = if !(-2.0..=2.0).contains(&pressure) {
            0.0
        } else if barrier.is_nan() {
            let v = self.f.barrier.point(s, x, y, z) as f64;
            *barrier = v;
            v
        } else {
            *barrier
        };
        2.0 * (noise + pressure)
    }

    fn status_at(&mut self, s: &mut Scratch, i: usize) -> FluidStatus {
        if let Some(st) = self.status[i] {
            return st;
        }
        let c = self.location[i].expect("located before its status is read");
        let st = self.compute_fluid(s, c[0], c[1], c[2]);
        self.status[i] = Some(st);
        st
    }

    /// `computeFluid`: the fluid status of an aquifer center.
    fn compute_fluid(&mut self, s: &mut Scratch, x: i32, y: i32, z: i32) -> FluidStatus {
        let global = self.picker.compute(x, y, z);
        let mut min_surface = i32::MAX;
        let top = y + 12;
        let bottom = y - 12;
        let mut surface_under_fluid = false;
        for off in SURFACE_SAMPLING_OFFSETS {
            let sx = x + (off[0] << 4);
            let sz = z + (off[1] << 4);
            let surface = self.surface_level(s, sx, sz);
            let adjusted = surface + 8;
            let start = off == [0, 0];
            if start && bottom > adjusted {
                return global;
            }
            let top_above = top > adjusted;
            if top_above || start {
                let g = self.picker.compute(sx, adjusted, sz);
                if !is_air(g.at(adjusted)) {
                    if start {
                        surface_under_fluid = true;
                    }
                    if top_above {
                        return g;
                    }
                }
            }
            min_surface = min_surface.min(surface);
        }
        let level = self.compute_surface_level(s, x, y, z, global, min_surface, surface_under_fluid);
        FluidStatus { level, fluid: self.compute_fluid_type(s, x, y, z, global, level) }
    }

    /// `computeSurfaceLevel`.
    #[allow(clippy::too_many_arguments)]
    fn compute_surface_level(
        &mut self,
        s: &mut Scratch,
        x: i32,
        y: i32,
        z: i32,
        global: FluidStatus,
        min_surface: i32,
        surface_under_fluid: bool,
    ) -> i32 {
        let (partially, fully) = if self.f.exclusion.point(s, x, y, z) as f64 > 0.0 {
            (-1.0, -1.0)
        } else {
            let distance_below = min_surface + 8 - y;
            let factor =
                if surface_under_fluid { clamped_map(distance_below as f64, 0.0, 64.0, 1.0, 0.0) } else { 0.0 };
            let floodedness = clamp(self.f.floodedness.point(s, x, y, z) as f64, -1.0, 1.0);
            let fully_threshold = map(factor, 1.0, 0.0, -0.3, 0.8);
            let partially_threshold = map(factor, 1.0, 0.0, -0.8, 0.4);
            (floodedness - partially_threshold, floodedness - fully_threshold)
        };
        if fully > 0.0 {
            global.level
        } else if partially > 0.0 {
            self.randomized_fluid_surface_level(s, x, y, z, min_surface)
        } else {
            WAY_BELOW_MIN_Y
        }
    }

    /// `computeRandomizedFluidSurfaceLevel`: the spread noise is sampled at grid coordinates.
    fn randomized_fluid_surface_level(&self, s: &mut Scratch, x: i32, y: i32, z: i32, min_surface: i32) -> i32 {
        let (gx, gy, gz) = (jm::floor_div(x, 16), jm::floor_div(y, 40), jm::floor_div(z, 16));
        let base = gy * 40 + 20;
        let spread = (self.f.spread.point(s, gx, gy, gz) * 10.0) as f64;
        let q = jm::floor(spread / 3.0) * 3;
        min_surface.min(base + q)
    }

    /// `computeFluidType`: deep aquifers turn to lava where the lava noise is strong.
    #[allow(clippy::too_many_arguments)]
    fn compute_fluid_type(&self, s: &mut Scratch, x: i32, y: i32, z: i32, global: FluidStatus, level: i32) -> u16 {
        if level <= -10 && level != WAY_BELOW_MIN_Y && global.fluid != state::LAVA {
            let v = self.f.lava.point(s, jm::floor_div(x, 64), jm::floor_div(y, 40), jm::floor_div(z, 64)) as f64;
            if v.abs() > 0.3 {
                return state::LAVA;
            }
        }
        global.fluid
    }
}

/// `Mth.clamp(double, double, double)`.
#[inline]
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo { lo } else { v.min(hi) }
}

/// `Mth.map(double, ...)`: `lerp(inverseLerp(v, a, b), c, d)`.
#[inline]
pub(crate) fn map(v: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    let t = (v - a) / (b - a);
    c + t * (d - c)
}

/// `Mth.clampedMap(double, ...)`.
#[inline]
pub(crate) fn clamped_map(v: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    let t = (v - a) / (b - a);
    if t < 0.0 {
        c
    } else if t > 1.0 {
        d
    } else {
        c + t * (d - c)
    }
}
