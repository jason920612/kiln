//! Gradient noise in 26.3's f32 formulation: `GradientNoise`, `PerlinNoise`,
//! `SmearedPerlinNoise`, `NoiseStack` and the `NormalNoise` octave setup.
//!
//! Coordinates stay `f64` up to the lattice cell; the fractional offsets and everything after
//! are `f32`. Point queries (`get3`) and volume fills (`add_to_volume`) round differently
//! (dot-product term order, where the amplitude is applied, how scales combine), so both are
//! reproduced as vanilla writes them.

use crate::interval::Interval;
use crate::volume::Volume;
use kiln_javamath::math::{floor, lerp3, smoothstep};
use kiln_javamath::random::{PositionalRandomFactory, RandomSource};

const GRADIENTS: [[i32; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

/// Coordinates at least this far from the origin are wrapped by `2^25` before sampling.
const HALF_ROUND_OFF: f64 = 16_777_215.999_999_998;
const ROUND_OFF: f64 = 33_554_432.0;

/// `GradientNoise.wrap`: keeps lattice coordinates small without moving the pattern.
#[inline]
pub fn wrap(x: f64) -> f64 {
    if (-HALF_ROUND_OFF..HALF_ROUND_OFF).contains(&x) { x } else { x - (x / ROUND_OFF + 0.5).floor() * ROUND_OFF }
}

#[inline]
fn grad(hash: i32) -> [f32; 3] {
    let g = GRADIENTS[(hash & 15) as usize];
    [g[0] as f32, g[1] as f32, g[2] as f32]
}

/// `Gradient.dot(float, float, float)`: `(gx*x + gy*y) + gz*z`.
#[inline]
fn dot(hash: i32, x: f32, y: f32, z: f32) -> f32 {
    let g = grad(hash);
    g[0] * x + g[1] * y + g[2] * z
}

/// A single-octave lattice: permutation table plus random origin offset.
#[derive(Clone, Debug)]
pub struct Lattice {
    perms: [u8; 256],
    offset: [f64; 3],
}

impl Lattice {
    pub fn new(random: &mut impl RandomSource) -> Self {
        let offset = [random.next_double() * 256.0, random.next_double() * 256.0, random.next_double() * 256.0];
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = random.next_int_bounded(256 - i as i32) as usize;
            perms.swap(i, i + j);
        }
        Self { perms, offset }
    }

    pub fn offset(&self) -> [f64; 3] {
        self.offset
    }

    #[inline]
    fn permute(&self, i: i32) -> i32 {
        self.perms[(i & 255) as usize] as i32
    }

    /// `PerlinNoise.sampleAndLerp`: `fy` feeds the gradients, `fy_smooth` the fade curve.
    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp(&self, x: i32, y: i32, z: i32, fx: f32, fy: f32, fz: f32, fy_smooth: f32) -> f32 {
        let a = self.permute(x);
        let b = self.permute(x.wrapping_add(1));
        let aa = self.permute(a.wrapping_add(y));
        let ab = self.permute(a.wrapping_add(y).wrapping_add(1));
        let ba = self.permute(b.wrapping_add(y));
        let bb = self.permute(b.wrapping_add(y).wrapping_add(1));
        let z1 = z.wrapping_add(1);
        let d000 = dot(self.permute(aa.wrapping_add(z)), fx, fy, fz);
        let d100 = dot(self.permute(ba.wrapping_add(z)), fx - 1.0, fy, fz);
        let d010 = dot(self.permute(ab.wrapping_add(z)), fx, fy - 1.0, fz);
        let d110 = dot(self.permute(bb.wrapping_add(z)), fx - 1.0, fy - 1.0, fz);
        let d001 = dot(self.permute(aa.wrapping_add(z1)), fx, fy, fz - 1.0);
        let d101 = dot(self.permute(ba.wrapping_add(z1)), fx - 1.0, fy, fz - 1.0);
        let d011 = dot(self.permute(ab.wrapping_add(z1)), fx, fy - 1.0, fz - 1.0);
        let d111 = dot(self.permute(bb.wrapping_add(z1)), fx - 1.0, fy - 1.0, fz - 1.0);
        lerp3(
            smoothstep(fx),
            smoothstep(fy_smooth),
            smoothstep(fz),
            [d000, d100, d010, d110, d001, d101, d011, d111],
        )
    }
}

/// One layer's noise: plain Perlin, or the "smeared" variant old blended noise uses.
#[derive(Clone, Debug)]
pub enum LayerNoise {
    Perlin(Lattice),
    Smeared { lattice: Lattice, fudge_y_scale: f64 },
}

impl LayerNoise {
    fn lattice(&self) -> &Lattice {
        match self {
            LayerNoise::Perlin(l) | LayerNoise::Smeared { lattice: l, .. } => l,
        }
    }

    /// `SmearedPerlinNoise.computeFudgeY`.
    #[inline]
    fn fudge_y(scale: f64, y: f64, fy: f64) -> f64 {
        let t = if y >= 0.0 && y < fy { y } else { fy };
        floor(t / scale + 1.000_000_011_686_097_4e-7) as f64 * scale
    }

    pub fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        let l = self.lattice();
        let xw = wrap(x) + l.offset[0];
        let yw = wrap(y) + l.offset[1];
        let zw = wrap(z) + l.offset[2];
        let (ix, iy, iz) = (floor(xw), floor(yw), floor(zw));
        let fx = (xw - ix as f64) as f32;
        let fz = (zw - iz as f64) as f32;
        match self {
            LayerNoise::Perlin(l) => {
                let fy = (yw - iy as f64) as f32;
                l.sample_and_lerp(ix, iy, iz, fx, fy, fz, fy)
            }
            LayerNoise::Smeared { lattice, fudge_y_scale } => {
                let fyd = yw - iy as f64;
                let fy = (fyd - Self::fudge_y(*fudge_y_scale, y, fyd)) as f32;
                lattice.sample_and_lerp(ix, iy, iz, fx, fy, fz, fyd as f32)
            }
        }
    }

    /// `PerlinNoise.get(double, double)`: a y = 0 slice.
    pub fn get2(&self, x: f64, z: f64) -> f32 {
        self.get3(wrap(x), 0.0, wrap(z))
    }

    /// `PerlinNoise.addToVolume` / `SmearedPerlinNoise.addToVolume`: adds `amp * noise` at
    /// every volume position, with coordinates `block * scale`.
    pub fn add_to_volume(&self, buf: &mut [f32], vol: &Volume, xz_scale: f64, y_scale: f64, amp: f32) {
        let l = self.lattice();
        let smear = match self {
            LayerNoise::Smeared { fudge_y_scale, .. } => Some(*fudge_y_scale),
            LayerNoise::Perlin(_) => None,
        };
        let mut i = 0;
        for zi in 0..vol.size[2] {
            let zw = wrap(vol.block_z(zi) as f64 * xz_scale) + l.offset[2];
            let iz = floor(zw);
            let fz = (zw - iz as f64) as f32;
            let sz = smoothstep(fz);
            let iz1 = iz.wrapping_add(1);
            for xi in 0..vol.size[0] {
                let xw = wrap(vol.block_x(xi) as f64 * xz_scale) + l.offset[0];
                let ix = floor(xw);
                let fx = (xw - ix as f64) as f32;
                let a = l.permute(ix);
                let b = l.permute(ix.wrapping_add(1));
                let sx = smoothstep(fx);
                let mut last_y = i32::MIN;
                let mut dxz = [0f32; 8];
                let mut gy = [0f32; 8];
                for yi in 0..vol.size[1] {
                    let y_raw = vol.block_y(yi) as f64 * y_scale;
                    let yw = wrap(y_raw) + l.offset[1];
                    let iy = floor(yw);
                    let fyd = yw - iy as f64;
                    let sy = smoothstep(fyd as f32);
                    if last_y != iy {
                        let aa = l.permute(a.wrapping_add(iy));
                        let ab = l.permute(a.wrapping_add(iy).wrapping_add(1));
                        let ba = l.permute(b.wrapping_add(iy));
                        let bb = l.permute(b.wrapping_add(iy).wrapping_add(1));
                        let corners = [
                            (aa, iz, fx, fz),
                            (ba, iz, fx - 1.0, fz),
                            (ab, iz, fx, fz),
                            (bb, iz, fx - 1.0, fz),
                            (aa, iz1, fx, fz - 1.0),
                            (ba, iz1, fx - 1.0, fz - 1.0),
                            (ab, iz1, fx, fz - 1.0),
                            (bb, iz1, fx - 1.0, fz - 1.0),
                        ];
                        for (k, (h, zc, cx, cz)) in corners.into_iter().enumerate() {
                            let g = grad(l.permute(h.wrapping_add(zc)));
                            dxz[k] = g[0] * cx + g[2] * cz;
                            gy[k] = g[1];
                        }
                        last_y = iy;
                    }
                    let fy = match smear {
                        Some(scale) => (fyd - Self::fudge_y(scale, y_raw, fyd)) as f32,
                        None => fyd as f32,
                    };
                    let fy1 = fy - 1.0;
                    let v = lerp3(
                        sx,
                        sy,
                        sz,
                        [
                            dxz[0] + gy[0] * fy,
                            dxz[1] + gy[1] * fy,
                            dxz[2] + gy[2] * fy1,
                            dxz[3] + gy[3] * fy1,
                            dxz[4] + gy[4] * fy,
                            dxz[5] + gy[5] * fy,
                            dxz[6] + gy[6] * fy1,
                            dxz[7] + gy[7] * fy1,
                        ],
                    );
                    buf[i] += amp * v;
                    i += 1;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub noise: LayerNoise,
    pub frequency: f64,
    pub amplitude: f32,
}

/// `NoiseStack`: a sum of scaled noise layers.
#[derive(Clone, Debug, Default)]
pub struct NoiseStack {
    pub layers: Vec<Layer>,
}

impl NoiseStack {
    pub fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut sum = 0f32;
        for l in &self.layers {
            let f = l.frequency;
            sum += l.amplitude * l.noise.get3(x * f, y * f, z * f);
        }
        sum
    }

    pub fn get2(&self, x: f64, z: f64) -> f32 {
        let mut sum = 0f32;
        for l in &self.layers {
            let f = l.frequency;
            sum += l.amplitude * l.noise.get2(x * f, z * f);
        }
        sum
    }

    pub fn add_to_volume(&self, buf: &mut [f32], vol: &Volume, xz_scale: f64, y_scale: f64, amp: f32) {
        for l in &self.layers {
            let f = l.frequency;
            l.noise.add_to_volume(buf, vol, xz_scale * f, y_scale * f, amp * l.amplitude);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    Disabled,
    Enabled,
    Legacy,
}

/// `NormalNoise.Parameters`: the `worldgen/noise` registry entry.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalNoiseParams {
    pub base_amplitude: f64,
    pub base_octave: i32,
    pub octave_count: i32,
    pub normalize: Normalization,
    pub amplitude_modifiers: Vec<f64>,
}

#[derive(Clone, Copy, Debug)]
struct OctaveInfo {
    index: i32,
    frequency: f64,
    amplitude: f64,
}

/// `2^n` for the small exponents octave setup uses (exact, like `Math.pow(2.0, n)`).
fn pow2(n: i32) -> f64 {
    assert!((-1022..=1023).contains(&n));
    f64::from_bits(((n + 1023) as u64) << 52)
}

/// `DoubleStream.sum()`: Kahan summation with the JDK's final correction.
fn java_stream_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut sum, mut comp, mut simple) = (0.0f64, 0.0f64, 0.0f64);
    for v in values {
        let tmp = v - comp;
        let velvel = sum + tmp;
        comp = (velvel - sum) - tmp;
        sum = velvel;
        simple += v;
    }
    let tmp = sum - comp;
    if tmp.is_nan() && simple.is_infinite() { simple } else { tmp }
}

impl NormalNoiseParams {
    fn modifier(&self, i: i32) -> f64 {
        if self.amplitude_modifiers.is_empty() { 1.0 } else { self.amplitude_modifiers[i as usize] }
    }

    fn octaves(&self) -> Vec<OctaveInfo> {
        let n = self.octave_count;
        let mut frequency = pow2(self.base_octave);
        let mut amplitude = self.base_amplitude;
        if self.normalize != Normalization::Disabled {
            amplitude *= pow2(n - 1) / (pow2(n) - 1.0);
        }
        let mut out = Vec::new();
        for i in 0..n {
            let m = self.modifier(i);
            if m != 0.0 {
                out.push(OctaveInfo { index: self.base_octave + i, frequency, amplitude: amplitude * m });
            }
            frequency *= 2.0;
            amplitude *= 0.5;
        }
        out
    }

    fn normalization_factor(sum: f64, octaves: &[OctaveInfo]) -> f64 {
        let mut dev = 0.0f64;
        for o in octaves {
            let d = 0.270_224_783_124_521_1 * o.amplitude.abs();
            dev += d * d;
        }
        let dev = dev.sqrt();
        if dev == 0.0 { 0.0 } else { (sum * 0.333_333_333_333_333_3) / (dev * 2f64.sqrt()) }
    }

    fn parity_normalization_factor(&self, amplitude: f64) -> f64 {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for i in 0..self.octave_count {
            if self.modifier(i) != 0.0 {
                lo = lo.min(i);
                hi = hi.max(i);
            }
        }
        let expected = 0.1 * (1.0 + 1.0 / (hi.wrapping_sub(lo) + 1) as f64);
        amplitude * 0.5 * 0.333_333_333_333_333_3 / expected
    }

    /// Octave list, amplitude normalization and declared range (`NormalNoise`'s constructor).
    fn setup(&self) -> (Vec<OctaveInfo>, f64, Interval) {
        let octaves = self.octaves();
        let mut sum = java_stream_sum(octaves.iter().map(|o| o.amplitude.abs()));
        let mut norm = Self::normalization_factor(sum, &octaves);
        if self.normalize == Normalization::Legacy && norm != 0.0 {
            let parity = self.parity_normalization_factor(self.base_amplitude);
            sum *= parity / norm;
            norm = parity;
        }
        let range = Interval::symmetric((sum * 0.333_333_333_333_333_3 * 6.0) as f32);
        (octaves, norm, range)
    }

    /// The value range `NoiseFunction` and shift functions declare for this noise.
    pub fn range(&self) -> Interval {
        self.setup().2
    }

    /// `NormalNoise.create`: two Perlin stacks forked from `random`, interleaved per octave,
    /// the second offset in frequency.
    pub fn create(&self, random: &mut impl RandomSource) -> NoiseStack {
        const INPUT_FACTOR: f64 = 1.018_126_888_217_522_7;
        let (octaves, norm, _) = self.setup();
        let first = random.fork_positional();
        let second = random.fork_positional();
        let mut stack = NoiseStack::default();
        for o in octaves {
            let seed = format!("octave_{}", o.index);
            let a = Lattice::new(&mut first.from_hash_of(&seed));
            let b = Lattice::new(&mut second.from_hash_of(&seed));
            let amplitude = (norm * o.amplitude) as f32;
            stack.layers.push(Layer { noise: LayerNoise::Perlin(a), frequency: o.frequency, amplitude });
            stack.layers.push(Layer { noise: LayerNoise::Perlin(b), frequency: o.frequency * INPUT_FACTOR, amplitude });
        }
        stack
    }
}

/// `Noises.instantiate`: the noise registered as `key`, seeded from the world's positional
/// factory by the key's name.
pub fn instantiate(params: &NormalNoiseParams, factory: &PositionalRandomFactory, key: &str) -> NoiseStack {
    params.create(&mut factory.from_hash_of(key))
}

/// `BlendedNoise.createFbm`: `-first_octave + 1` smeared octaves, highest frequency first,
/// all drawn from one random source in sequence.
pub fn blended_fbm(random: &mut impl RandomSource, first_octave: i32, smear: f64, amplitude_factor: f64) -> NoiseStack {
    assert!(first_octave <= 0, "firstOctave>0");
    let count = -first_octave + 1;
    let mut frequency = 1.0f64;
    let mut amplitude = amplitude_factor / (pow2(count) - 1.0);
    let mut stack = NoiseStack::default();
    for _ in 0..count {
        let lattice = Lattice::new(random);
        stack.layers.push(Layer {
            noise: LayerNoise::Smeared { lattice, fudge_y_scale: smear * frequency },
            frequency,
            amplitude: amplitude as f32,
        });
        frequency /= 2.0;
        amplitude *= 2.0;
    }
    stack
}

/// `BlendedNoise.computeFbmRange`: the range vanilla declares for `old_blended_noise`.
pub fn blended_fbm_range(first_octave: i32, smear: f64, amplitude_factor: f64) -> Interval {
    let count = -first_octave + 1;
    let mut frequency = 1.0f64;
    let mut amplitude = amplitude_factor / (pow2(count) - 1.0);
    let mut range = Interval::exact(0.0);
    for _ in 0..count {
        let layer = Interval::symmetric(((smear * frequency).abs() + 2.0) as f32);
        range = Interval::add(range, Interval::mul(layer, Interval::exact(amplitude as f32)));
        frequency /= 2.0;
        amplitude *= 2.0;
    }
    range
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_small_and_moves_large() {
        assert_eq!(wrap(1234.5), 1234.5);
        assert_eq!(wrap(-16_777_215.0), -16_777_215.0);
        assert_eq!(wrap(16_777_216.0), -16_777_216.0);
        assert_eq!(wrap(33_554_432.0 + 7.25), 7.25);
    }

    #[test]
    fn stream_sum_compensates() {
        let plain: f64 = [1e16, 1.0, 1.0].iter().sum();
        assert_eq!(plain, 1e16);
        assert_eq!(java_stream_sum([1e16, 1.0, 1.0]), 1e16 + 2.0);
    }
}
