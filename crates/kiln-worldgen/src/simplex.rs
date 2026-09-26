//! 2D simplex noise (`SimplexNoise.get(double, double)`) and the fixed-seed biome
//! temperature noises (`Biome.TEMPERATURE_NOISE`, `FROZEN_TEMPERATURE_NOISE`,
//! `BIOME_INFO_NOISE`). These stay in `f64` until the final `(float)` cast.

use kiln_javamath::math::floor;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::sync::OnceLock;

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

pub struct Simplex {
    perms: [u8; 256],
    offset: [f64; 2],
}

impl Simplex {
    /// `new SimplexNoise(random, originAtZero)`: `GradientNoise(random, 0.0 or 256.0)`.
    pub fn new(random: &mut impl RandomSource, origin_at_zero: bool) -> Self {
        let factor = if origin_at_zero { 0.0 } else { 256.0 };
        let ox = random.next_double() * factor;
        let oy = random.next_double() * factor;
        let _oz = random.next_double() * factor;
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = random.next_int_bounded(256 - i as i32) as usize;
            perms.swap(i, i + j);
        }
        Self { perms, offset: [ox, oy] }
    }

    #[inline]
    fn permute(&self, i: i32) -> i32 {
        self.perms[(i & 255) as usize] as i32
    }

    #[inline]
    fn corner(index: i32, x: f64, y: f64) -> f64 {
        let t = 0.5 - x * x - y * y - 0.0 * 0.0;
        if t < 0.0 {
            0.0
        } else {
            let t = t * t;
            let g = GRADIENTS[index as usize];
            t * t * (g[0] as f64 * x + g[1] as f64 * y + g[2] as f64 * 0.0)
        }
    }

    /// `SimplexNoise.get(double, double)`.
    pub fn get2(&self, x: f64, y: f64) -> f32 {
        let f2 = 0.5 * (3f64.sqrt() - 1.0);
        let g2 = (3.0 - 3f64.sqrt()) / 6.0;
        let (x, y) = (x + self.offset[0], y + self.offset[1]);
        let s = (x + y) * f2;
        let i = floor(x + s);
        let j = floor(y + s);
        let t = (i.wrapping_add(j)) as f64 * g2;
        let x0 = x - (i as f64 - t);
        let y0 = y - (j as f64 - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - i1 as f64 + g2;
        let y1 = y0 - j1 as f64 + g2;
        let x2 = x0 - 1.0 + 2.0 * g2;
        let y2 = y0 - 1.0 + 2.0 * g2;
        let (ii, jj) = (i & 255, j & 255);
        let gi0 = self.permute(ii + self.permute(jj)) % 12;
        let gi1 = self.permute(ii + i1 + self.permute(jj + j1)) % 12;
        let gi2 = self.permute(ii + 1 + self.permute(jj + 1)) % 12;
        let n0 = Self::corner(gi0, x0, y0);
        let n1 = Self::corner(gi1, x1, y1);
        let n2 = Self::corner(gi2, x2, y2);
        (70.0 * (n0 + n1 + n2)) as f32
    }
}

/// A `NoiseStack` of simplex layers, 2D queries only.
pub struct SimplexStack {
    layers: Vec<(Simplex, f64, f32)>,
}

impl SimplexStack {
    /// `NoiseStack.get(double, double)`: `sum += amplitude * noise.get(x * f, z * f)`.
    pub fn get2(&self, x: f64, z: f64) -> f32 {
        let mut sum = 0f32;
        for (n, f, a) in &self.layers {
            sum += a * n.get2(x * f, z * f);
        }
        sum
    }
}

/// The biome climate noises, seeded with fixed values.
pub struct BiomeNoises {
    pub temperature: Simplex,
    pub frozen_temperature: SimplexStack,
    pub biome_info: Simplex,
}

pub fn biome_noises() -> &'static BiomeNoises {
    static NOISES: OnceLock<BiomeNoises> = OnceLock::new();
    NOISES.get_or_init(|| {
        let mut frozen = LegacyRandom::new(3456);
        let layers = vec![
            (Simplex::new(&mut frozen, true), 1.0, 0.142_857_15),
            (Simplex::new(&mut frozen, true), 0.5, 0.285_714_3),
            (Simplex::new(&mut frozen, true), 0.25, 0.571_428_6),
        ];
        BiomeNoises {
            temperature: Simplex::new(&mut LegacyRandom::new(1234), true),
            frozen_temperature: SimplexStack { layers },
            biome_info: Simplex::new(&mut LegacyRandom::new(2345), true),
        }
    })
}

/// `Biome.getHeightAdjustedTemperature` (with its temperature modifier); vanilla memoizes it
/// per position, which cannot change the value.
pub fn biome_temperature(base: f32, frozen: bool, sea_level: i32, x: i32, y: i32, z: i32) -> f32 {
    let n = biome_noises();
    let mut t = base;
    if frozen {
        let f = (n.frozen_temperature.get2(x as f64 * 0.05, z as f64 * 0.05) * 7.0) as f64;
        let info = n.biome_info.get2(x as f64 * 0.2, z as f64 * 0.2) as f64;
        if f + info < 0.3 && (n.biome_info.get2(x as f64 * 0.09, z as f64 * 0.09) as f64) < 0.8 {
            t = 0.2;
        }
    }
    let snow_line = sea_level + 17;
    if y > snow_line {
        let noise = n.temperature.get2((x as f32 / 8.0) as f64, (z as f32 / 8.0) as f64) * 8.0;
        t - (noise + y as f32 - snow_line as f32) * 0.05 / 40.0
    } else {
        t
    }
}

/// `Biome.coldEnoughToSnow`: temperature below 0.15.
pub fn cold_enough_to_snow(base: f32, frozen: bool, sea_level: i32, x: i32, y: i32, z: i32) -> bool {
    !(biome_temperature(base, frozen, sea_level, x, y, z) >= 0.15)
}

/// `Biome.shouldMeltFrozenOceanIcebergSlightly`: temperature above 0.1.
pub fn melts_icebergs_slightly(base: f32, frozen: bool, sea_level: i32, x: i32, y: i32, z: i32) -> bool {
    biome_temperature(base, frozen, sea_level, x, y, z) > 0.1
}
