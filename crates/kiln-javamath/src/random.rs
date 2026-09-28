//! Minecraft's random sources: the 48-bit LCG of `java.util.Random` (`LegacyRandomSource`),
//! Xoroshiro128++ (`XoroshiroRandomSource`) and the positional factories forked from them.

use md5::{Digest, Md5};

pub const GOLDEN_RATIO_64: i64 = -7_046_029_254_386_353_131;
pub const SILVER_RATIO_64: i64 = 7_640_891_576_956_012_809;

const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;
const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;

/// Stafford's "mix13" finalizer (`RandomSupport.mixStafford13`).
pub fn mix_stafford13(mut z: i64) -> i64 {
    z = (z ^ ((z as u64) >> 30) as i64).wrapping_mul(-4_658_895_280_553_007_687);
    z = (z ^ ((z as u64) >> 27) as i64).wrapping_mul(-7_723_592_293_110_705_685);
    z ^ ((z as u64) >> 31) as i64
}

/// A 128-bit Xoroshiro seed (`RandomSupport.Seed128bit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seed128 {
    pub lo: i64,
    pub hi: i64,
}

impl Seed128 {
    pub fn upgrade_unmixed(seed: i64) -> Self {
        let lo = seed ^ SILVER_RATIO_64;
        Self { lo, hi: lo.wrapping_add(GOLDEN_RATIO_64) }
    }

    /// `RandomSupport.upgradeSeedTo128bit`: how a world seed becomes a Xoroshiro state.
    pub fn upgrade(seed: i64) -> Self {
        let s = Self::upgrade_unmixed(seed);
        Self { lo: mix_stafford13(s.lo), hi: mix_stafford13(s.hi) }
    }

    /// `RandomSupport.seedFromHashOf`: the big-endian halves of the name's MD5.
    pub fn from_hash_of(name: &str) -> Self {
        let digest = Md5::digest(name.as_bytes());
        let lo = i64::from_be_bytes(digest[0..8].try_into().unwrap());
        let hi = i64::from_be_bytes(digest[8..16].try_into().unwrap());
        Self { lo, hi }
    }

    pub fn xor(self, lo: i64, hi: i64) -> Self {
        Self { lo: self.lo ^ lo, hi: self.hi ^ hi }
    }

    /// `Seed128bit.mixed`: both halves through [`mix_stafford13`].
    pub fn mixed(self) -> Self {
        Self { lo: mix_stafford13(self.lo), hi: mix_stafford13(self.hi) }
    }
}

#[derive(Clone, Debug)]
pub struct Xoroshiro128PlusPlus {
    lo: i64,
    hi: i64,
}

impl Xoroshiro128PlusPlus {
    pub fn new(lo: i64, hi: i64) -> Self {
        if lo | hi == 0 { Self { lo: GOLDEN_RATIO_64, hi: SILVER_RATIO_64 } } else { Self { lo, hi } }
    }

    pub fn next_long(&mut self) -> i64 {
        let (lo, mut hi) = (self.lo, self.hi);
        let out = lo.wrapping_add(hi).rotate_left(17).wrapping_add(lo);
        hi ^= lo;
        self.lo = lo.rotate_left(49) ^ hi ^ (hi << 21);
        self.hi = hi.rotate_left(28);
        out
    }
}

/// The `RandomSource` methods worldgen uses, with Minecraft's exact algorithms.
pub trait RandomSource {
    fn next_long(&mut self) -> i64;
    fn next_int(&mut self) -> i32;
    /// Uniform in `[0, bound)`; `bound` must be positive.
    fn next_int_bounded(&mut self, bound: i32) -> i32;
    fn next_bool(&mut self) -> bool;
    fn next_float(&mut self) -> f32;
    fn next_double(&mut self) -> f64;
    fn fork_positional(&mut self) -> PositionalRandomFactory;

    fn consume_count(&mut self, count: usize) {
        for _ in 0..count {
            self.next_int();
        }
    }
}

/// `XoroshiroRandomSource`.
#[derive(Clone, Debug)]
pub struct XoroshiroRandom {
    rng: Xoroshiro128PlusPlus,
}

impl XoroshiroRandom {
    /// Seeds from a 64-bit seed, mixing it first (`new XoroshiroRandomSource(long)`).
    pub fn new(seed: i64) -> Self {
        Self::from_seed128(Seed128::upgrade(seed))
    }

    /// Uses the 128-bit state as is.
    pub fn from_seed128(seed: Seed128) -> Self {
        Self { rng: Xoroshiro128PlusPlus::new(seed.lo, seed.hi) }
    }

    /// The current state (`XoroshiroRandomSource.CODEC`: `[lo, hi]`).
    pub fn state(&self) -> Seed128 {
        Seed128 { lo: self.rng.lo, hi: self.rng.hi }
    }
}

impl RandomSource for XoroshiroRandom {
    fn next_long(&mut self) -> i64 {
        self.rng.next_long()
    }

    fn next_int(&mut self) -> i32 {
        self.rng.next_long() as i32
    }

    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let bound64 = bound as i64;
        let mut product = (self.next_int() as u32 as i64) * bound64;
        let mut low = product & 0xFFFF_FFFF;
        if low < bound64 {
            let threshold = (bound.wrapping_neg() as u32 % bound as u32) as i64;
            while low < threshold {
                product = (self.next_int() as u32 as i64) * bound64;
                low = product & 0xFFFF_FFFF;
            }
        }
        (product >> 32) as i32
    }

    fn next_bool(&mut self) -> bool {
        self.rng.next_long() & 1 != 0
    }

    fn next_float(&mut self) -> f32 {
        ((self.rng.next_long() as u64) >> 40) as f32 * FLOAT_UNIT
    }

    fn next_double(&mut self) -> f64 {
        ((self.rng.next_long() as u64) >> 11) as f64 * DOUBLE_UNIT
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        let lo = self.rng.next_long();
        let hi = self.rng.next_long();
        PositionalRandomFactory::Xoroshiro { lo, hi }
    }

    fn consume_count(&mut self, count: usize) {
        for _ in 0..count {
            self.rng.next_long();
        }
    }
}

/// `LegacyRandomSource`: `java.util.Random`'s 48-bit LCG.
#[derive(Clone, Debug)]
pub struct LegacyRandom {
    seed: i64,
    /// `MarsagliaPolarGaussian`'s second value of the last pair, until drawn.
    next_gaussian: Option<f64>,
}

impl LegacyRandom {
    const MULTIPLIER: i64 = 0x5_DEEC_E66D;
    const MASK: i64 = (1 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        Self { seed: (seed ^ Self::MULTIPLIER) & Self::MASK, next_gaussian: None }
    }

    /// `nextGaussian` (`MarsagliaPolarGaussian`): pairs of normal values from pairs of doubles
    /// inside the unit circle; the second of a pair is kept for the next call. The values use
    /// the platform's `ln` (vanilla: `StrictMath.log`), so their last bits may differ; the
    /// draws do not.
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

    /// The raw 48-bit state (`seed` of `LegacyRandomSource`), for tests.
    pub fn state(&self) -> i64 {
        self.seed
    }

    pub fn next(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        (self.seed >> (48 - bits)) as i32
    }
}

impl RandomSource for LegacyRandom {
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
        PositionalRandomFactory::Legacy { seed: self.next_long() }
    }
}

/// Either kind of random source, as returned by positional factories.
#[derive(Clone, Debug)]
pub enum WorldgenRandom {
    Xoroshiro(XoroshiroRandom),
    Legacy(LegacyRandom),
}

macro_rules! delegate {
    ($self:ident, $r:ident => $e:expr) => {
        match $self {
            WorldgenRandom::Xoroshiro($r) => $e,
            WorldgenRandom::Legacy($r) => $e,
        }
    };
}

impl RandomSource for WorldgenRandom {
    fn next_long(&mut self) -> i64 {
        delegate!(self, r => r.next_long())
    }

    fn next_int(&mut self) -> i32 {
        delegate!(self, r => r.next_int())
    }

    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        delegate!(self, r => r.next_int_bounded(bound))
    }

    fn next_bool(&mut self) -> bool {
        delegate!(self, r => r.next_bool())
    }

    fn next_float(&mut self) -> f32 {
        delegate!(self, r => r.next_float())
    }

    fn next_double(&mut self) -> f64 {
        delegate!(self, r => r.next_double())
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        delegate!(self, r => r.fork_positional())
    }

    fn consume_count(&mut self, count: usize) {
        delegate!(self, r => r.consume_count(count))
    }
}

/// `PositionalRandomFactory`: derives independent sources from positions, names and seeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionalRandomFactory {
    Xoroshiro { lo: i64, hi: i64 },
    Legacy { seed: i64 },
}

impl PositionalRandomFactory {
    pub fn at(&self, x: i32, y: i32, z: i32) -> WorldgenRandom {
        let pos = crate::math::get_seed(x, y, z);
        match *self {
            Self::Xoroshiro { lo, hi } => {
                WorldgenRandom::Xoroshiro(XoroshiroRandom::from_seed128(Seed128 { lo: pos ^ lo, hi }))
            }
            Self::Legacy { seed } => WorldgenRandom::Legacy(LegacyRandom::new(pos ^ seed)),
        }
    }

    /// `fromHashOf(name)`: Xoroshiro factories hash the name with MD5, legacy ones use
    /// `String.hashCode`.
    pub fn from_hash_of(&self, name: &str) -> WorldgenRandom {
        match *self {
            Self::Xoroshiro { lo, hi } => {
                WorldgenRandom::Xoroshiro(XoroshiroRandom::from_seed128(Seed128::from_hash_of(name).xor(lo, hi)))
            }
            Self::Legacy { seed } => {
                WorldgenRandom::Legacy(LegacyRandom::new(crate::math::string_hash(name) as i64 ^ seed))
            }
        }
    }

    pub fn from_seed(&self, seed: i64) -> WorldgenRandom {
        match *self {
            Self::Xoroshiro { lo, hi } => {
                WorldgenRandom::Xoroshiro(XoroshiroRandom::from_seed128(Seed128 { lo: seed ^ lo, hi: seed ^ hi }))
            }
            Self::Legacy { .. } => WorldgenRandom::Legacy(LegacyRandom::new(seed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_matches_java_util_random() {
        // new java.util.Random(0): nextInt() = -1155484576, nextInt(10) = 8 after it,
        // nextLong() from a fresh Random(42) = -5025562857975149833.
        let mut r = LegacyRandom::new(0);
        assert_eq!(r.next_int(), -1_155_484_576);
        assert_eq!(r.next_int_bounded(10), 8);
        let mut r = LegacyRandom::new(42);
        assert_eq!(r.next_long(), -5_025_562_857_975_149_833);
        let mut r = LegacyRandom::new(42);
        assert_eq!(r.next_double(), 0.7275636800328681);
    }

    #[test]
    fn md5_seed_uses_big_endian_halves() {
        // MD5("") = d41d8cd98f00b204e9800998ecf8427e
        let s = Seed128::from_hash_of("");
        assert_eq!(s.lo as u64, 0xd41d_8cd9_8f00_b204);
        assert_eq!(s.hi as u64, 0xe980_0998_ecf8_427e);
    }

    #[test]
    fn xoroshiro_zero_state_is_replaced() {
        let mut a = Xoroshiro128PlusPlus::new(0, 0);
        let mut b = Xoroshiro128PlusPlus::new(GOLDEN_RATIO_64, SILVER_RATIO_64);
        assert_eq!(a.next_long(), b.next_long());
    }
}
