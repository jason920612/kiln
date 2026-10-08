//! Vanilla's `WorldgenRandom`: a `LegacyRandomSource`-style bit generator whose bits come from
//! another source (Xoroshiro for decoration, the legacy LCG for structures), with the
//! decoration, feature and structure seeding helpers.

use kiln_javamath::random::{LegacyRandom, PositionalRandomFactory, RandomSource, XoroshiroRandom};

const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;
const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;

#[derive(Clone, Debug)]
enum Bits {
    Xoroshiro(XoroshiroRandom),
    Legacy(LegacyRandom),
}

/// `WorldgenRandom`. Every draw goes through `next(bits)` with `BitRandomSource`'s algorithms,
/// so a Xoroshiro-backed one does not draw like a plain `XoroshiroRandomSource`.
#[derive(Clone, Debug)]
pub struct WorldgenRandom {
    bits: Bits,
    /// `MarsagliaPolarGaussian` state; `setSeed` does not clear it (vanilla's override skips
    /// `LegacyRandomSource.setSeed`), so a cached value survives reseeding.
    next_gaussian: Option<f64>,
}

impl WorldgenRandom {
    /// `new WorldgenRandom(new XoroshiroRandomSource(seed))`.
    pub fn xoroshiro(seed: i64) -> Self {
        Self { bits: Bits::Xoroshiro(XoroshiroRandom::new(seed)), next_gaussian: None }
    }

    /// `new WorldgenRandom(new LegacyRandomSource(seed))`.
    pub fn legacy(seed: i64) -> Self {
        Self { bits: Bits::Legacy(LegacyRandom::new(seed)), next_gaussian: None }
    }

    /// Draws from the state of `r` (a feature placed with the level's own random, as saplings
    /// and bone meal do); [`WorldgenRandom::into_legacy`] gives the advanced state back.
    pub fn from_legacy(r: LegacyRandom) -> Self {
        Self { bits: Bits::Legacy(r), next_gaussian: None }
    }

    /// The legacy source of a [`WorldgenRandom::legacy`] or [`WorldgenRandom::from_legacy`].
    pub fn into_legacy(self) -> Option<LegacyRandom> {
        match self.bits {
            Bits::Legacy(r) => Some(r),
            Bits::Xoroshiro(_) => None,
        }
    }

    /// `setSeed`: reseeds the underlying source.
    pub fn set_seed(&mut self, seed: i64) {
        match &mut self.bits {
            Bits::Xoroshiro(r) => *r = XoroshiroRandom::new(seed),
            Bits::Legacy(r) => *r = LegacyRandom::new(seed),
        }
    }

    /// `next(bits)`.
    #[inline]
    pub fn next(&mut self, bits: u32) -> i32 {
        match &mut self.bits {
            Bits::Xoroshiro(r) => ((r.next_long() as u64) >> (64 - bits)) as i32,
            Bits::Legacy(r) => r.next(bits),
        }
    }

    /// `setDecorationSeed`: seeds from the level seed and a block position, returns the seed.
    pub fn set_decoration_seed(&mut self, level_seed: i64, x: i32, z: i32) -> i64 {
        self.set_seed(level_seed);
        let a = self.next_long() | 1;
        let b = self.next_long() | 1;
        let seed = (x as i64).wrapping_mul(a).wrapping_add((z as i64).wrapping_mul(b)) ^ level_seed;
        self.set_seed(seed);
        seed
    }

    /// `setFeatureSeed`.
    pub fn set_feature_seed(&mut self, decoration_seed: i64, index: i32, step: i32) {
        self.set_seed(decoration_seed.wrapping_add(index as i64).wrapping_add(10_000i32.wrapping_mul(step) as i64));
    }

    /// `setLargeFeatureSeed`.
    pub fn set_large_feature_seed(&mut self, seed: i64, x: i32, z: i32) {
        self.set_seed(seed);
        let a = self.next_long();
        let b = self.next_long();
        self.set_seed((x as i64).wrapping_mul(a) ^ (z as i64).wrapping_mul(b) ^ seed);
    }

    /// `setLargeFeatureWithSalt`.
    pub fn set_large_feature_with_salt(&mut self, seed: i64, x: i32, z: i32, salt: i32) {
        let s = (x as i64)
            .wrapping_mul(341_873_128_712)
            .wrapping_add((z as i64).wrapping_mul(132_897_987_541))
            .wrapping_add(seed)
            .wrapping_add(salt as i64);
        self.set_seed(s);
    }

    /// `nextGaussian` (`MarsagliaPolarGaussian`).
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(g) = self.next_gaussian.take() {
            return g;
        }
        loop {
            let a = 2.0 * self.next_double() - 1.0;
            let b = 2.0 * self.next_double() - 1.0;
            let s = a * a + b * b;
            if s < 1.0 && s != 0.0 {
                let m = (-2.0 * s.ln() / s).sqrt();
                self.next_gaussian = Some(b * m);
                return a * m;
            }
        }
    }

    /// `RandomSource.nextInt(origin, bound)`.
    pub fn next_int_range(&mut self, origin: i32, bound: i32) -> i32 {
        origin + self.next_int_bounded(bound - origin)
    }

    /// `RandomSource.nextIntBetweenInclusive`.
    pub fn next_int_between(&mut self, min: i32, max: i32) -> i32 {
        self.next_int_bounded(max - min + 1) + min
    }

    /// `RandomSource.triangle(mode, deviation)`.
    pub fn triangle(&mut self, mode: f64, deviation: f64) -> f64 {
        mode + deviation * (self.next_double() - self.next_double())
    }

    /// `RandomSource.triangle(float mode, float deviation)`.
    pub fn triangle_f32(&mut self, mode: f32, deviation: f32) -> f32 {
        mode + deviation * (self.next_float() - self.next_float())
    }
}

impl RandomSource for WorldgenRandom {
    fn next_long(&mut self) -> i64 {
        let hi = self.next(32) as i64;
        let lo = self.next(32) as i64;
        (hi << 32).wrapping_add(lo)
    }

    fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if bound & (bound - 1) == 0 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    fn next_bool(&mut self) -> bool {
        self.next(1) != 0
    }

    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * FLOAT_UNIT
    }

    fn next_double(&mut self) -> f64 {
        let hi = self.next(26) as i64;
        let lo = self.next(27) as i64;
        ((hi << 27) + lo) as f64 * DOUBLE_UNIT
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        match &mut self.bits {
            Bits::Xoroshiro(r) => r.fork_positional(),
            Bits::Legacy(r) => r.fork_positional(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_backed_draws_like_java_util_random() {
        let mut w = WorldgenRandom::legacy(42);
        let mut l = LegacyRandom::new(42);
        for _ in 0..10 {
            assert_eq!(w.next_int_bounded(100), l.next_int_bounded(100));
            assert_eq!(w.next_long(), l.next_long());
        }
    }

    #[test]
    fn xoroshiro_backed_uses_high_bits() {
        let mut w = WorldgenRandom::xoroshiro(7);
        let mut x = XoroshiroRandom::new(7);
        assert_eq!(w.next(32), ((x.next_long() as u64) >> 32) as i32);
    }
}
