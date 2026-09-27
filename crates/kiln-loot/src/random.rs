//! Loot randomness: the helpers `Mth` offers over a `RandomSource`, and the named random
//! sequences (`RandomSequences`, server-wide in 26.3) that tables with a `random_sequence` draw
//! from when no explicit seed is given.

use kiln_item::Identifier;
use kiln_javamath::random::{LegacyRandom, PositionalRandomFactory, RandomSource, Seed128, mix_stafford13};
use std::collections::HashMap;

/// The `Mth` random helpers loot uses.
pub trait RngExt: RandomSource {
    /// `RandomSource.nextInt(bound)`.
    fn next_int(&mut self, bound: i32) -> i32 {
        self.next_int_bounded(bound)
    }

    /// `Mth.nextInt(random, min, max)`: inclusive, `min` when the range is empty.
    fn next_int_between(&mut self, min: i32, max: i32) -> i32 {
        if min >= max { min } else { self.next_int_bounded(max.wrapping_sub(min).wrapping_add(1)).wrapping_add(min) }
    }

    /// `Mth.nextFloat(random, min, max)`.
    fn next_float_between(&mut self, min: f32, max: f32) -> f32 {
        if min >= max { min } else { self.next_float() * (max - min) + min }
    }
}

impl<R: RandomSource + ?Sized> RngExt for R {}

/// `RandomSource.create(seed)`: the random of an explicit loot seed.
pub fn seeded(seed: i64) -> LegacyRandom {
    LegacyRandom::new(seed)
}

/// `XoroshiroRandomSource` with its state exposed, so sequences can be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xoroshiro {
    lo: i64,
    hi: i64,
}

const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;
const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;

impl Xoroshiro {
    /// A generator in state `(lo, hi)` (an all-zero state is replaced, as vanilla does).
    pub fn from_state(lo: i64, hi: i64) -> Self {
        if lo | hi == 0 {
            Xoroshiro { lo: kiln_javamath::random::GOLDEN_RATIO_64, hi: kiln_javamath::random::SILVER_RATIO_64 }
        } else {
            Xoroshiro { lo, hi }
        }
    }

    /// The state as `XoroshiroRandomSource.CODEC` saves it (`[lo, hi]`).
    pub fn state(&self) -> (i64, i64) {
        (self.lo, self.hi)
    }
}

impl RandomSource for Xoroshiro {
    fn next_long(&mut self) -> i64 {
        let (lo, mut hi) = (self.lo, self.hi);
        let out = lo.wrapping_add(hi).rotate_left(17).wrapping_add(lo);
        hi ^= lo;
        self.lo = lo.rotate_left(49) ^ hi ^ (hi << 21);
        self.hi = hi.rotate_left(28);
        out
    }

    fn next_int(&mut self) -> i32 {
        self.next_long() as i32
    }

    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let bound64 = bound as i64;
        let mut product = (RandomSource::next_int(self) as u32 as i64) * bound64;
        let mut low = product & 0xFFFF_FFFF;
        if low < bound64 {
            let threshold = (bound.wrapping_neg() as u32 % bound as u32) as i64;
            while low < threshold {
                product = (RandomSource::next_int(self) as u32 as i64) * bound64;
                low = product & 0xFFFF_FFFF;
            }
        }
        (product >> 32) as i32
    }

    fn next_bool(&mut self) -> bool {
        self.next_long() & 1 != 0
    }

    fn next_float(&mut self) -> f32 {
        ((self.next_long() as u64) >> 40) as f32 * FLOAT_UNIT
    }

    fn next_double(&mut self) -> f64 {
        ((self.next_long() as u64) >> 11) as f64 * DOUBLE_UNIT
    }

    fn fork_positional(&mut self) -> PositionalRandomFactory {
        let lo = self.next_long();
        let hi = self.next_long();
        PositionalRandomFactory::Xoroshiro { lo, hi }
    }
}

/// `RandomSequences`: one Xoroshiro generator per sequence id, created on first use from the
/// world seed, the salt and the id's MD5 (`RandomSequence.createSequence`).
#[derive(Debug, Clone)]
pub struct RandomSequences {
    world_seed: i64,
    salt: i32,
    include_world_seed: bool,
    include_sequence_id: bool,
    sequences: HashMap<Identifier, Xoroshiro>,
}

impl RandomSequences {
    pub fn new(world_seed: i64) -> Self {
        RandomSequences {
            world_seed,
            salt: 0,
            include_world_seed: true,
            include_sequence_id: true,
            sequences: HashMap::new(),
        }
    }

    /// `RandomSequences.setSeedDefaults` (the `/random reset` defaults).
    pub fn set_seed_defaults(&mut self, salt: i32, include_world_seed: bool, include_sequence_id: bool) {
        self.salt = salt;
        self.include_world_seed = include_world_seed;
        self.include_sequence_id = include_sequence_id;
    }

    /// The generator of `id`, created with the current defaults if new.
    pub fn get(&mut self, id: &Identifier) -> &mut Xoroshiro {
        let (seed, salt, ws, si) = (self.world_seed, self.salt, self.include_world_seed, self.include_sequence_id);
        self.sequences.entry(id.clone()).or_insert_with(|| create_sequence(id, seed, salt, ws, si))
    }

    /// `RandomSequences.reset(id, ...)`.
    pub fn reset(&mut self, id: &Identifier, salt: i32, include_world_seed: bool, include_sequence_id: bool) {
        let seq = create_sequence(id, self.world_seed, salt, include_world_seed, include_sequence_id);
        self.sequences.insert(id.clone(), seq);
    }

    /// `RandomSequences.clear()`: forgets every sequence, returning how many there were.
    pub fn clear(&mut self) -> usize {
        let n = self.sequences.len();
        self.sequences.clear();
        n
    }

    /// Restores a saved sequence state.
    pub fn insert(&mut self, id: Identifier, state: Xoroshiro) {
        self.sequences.insert(id, state);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Identifier, &Xoroshiro)> {
        self.sequences.iter()
    }

    pub fn salt(&self) -> i32 {
        self.salt
    }

    pub fn include_world_seed(&self) -> bool {
        self.include_world_seed
    }

    pub fn include_sequence_id(&self) -> bool {
        self.include_sequence_id
    }
}

/// `RandomSequences.createSequence` + `RandomSequence.createSequence`.
pub fn create_sequence(id: &Identifier, world_seed: i64, salt: i32, include_world_seed: bool, include_sequence_id: bool) -> Xoroshiro {
    let seed = if include_world_seed { world_seed } else { 0 } ^ salt as i64;
    let mut s = Seed128::upgrade_unmixed(seed);
    if include_sequence_id {
        let h = Seed128::from_hash_of(id.as_str());
        s = s.xor(h.lo, h.hi);
    }
    Xoroshiro::from_state(mix_stafford13(s.lo), mix_stafford13(s.hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_javamath::random::XoroshiroRandom;

    #[test]
    fn matches_kiln_javamath_xoroshiro() {
        let s = Seed128::upgrade(1234);
        let mut a = XoroshiroRandom::from_seed128(s);
        let mut b = Xoroshiro::from_state(s.lo, s.hi);
        for bound in [1, 7, 16, 100, i32::MAX] {
            assert_eq!(a.next_int_bounded(bound), b.next_int_bounded(bound));
            assert_eq!(a.next_float(), b.next_float());
            assert_eq!(a.next_long(), b.next_long());
        }
    }

    #[test]
    fn mth_helpers() {
        let mut r = seeded(5);
        assert_eq!(r.next_int_between(3, 3), 3);
        assert_eq!(r.next_int_between(4, 2), 4);
        assert_eq!(r.next_float_between(2.0, 1.0), 2.0);
    }
}
