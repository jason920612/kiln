//! Float interval arithmetic (`net.minecraft.util.Interval`).
//!
//! Density function compilation uses declared value ranges to drop `min`/`max` branches and to
//! short-circuit them per position, so the ranges must match vanilla's exactly (including
//! float rounding of the bounds), not merely be conservative.

use kiln_javamath::math as jm;

#[derive(Clone, Copy, Debug)]
pub struct Interval {
    min: f32,
    max: f32,
}

impl PartialEq for Interval {
    fn eq(&self, other: &Self) -> bool {
        self.min.to_bits() == other.min.to_bits() && self.max.to_bits() == other.max.to_bits()
    }
}

#[allow(clippy::should_implement_trait)]
impl Interval {
    /// "Not an interval": the result of undefined operations. Its NaN bounds fail every
    /// comparison, which is how vanilla's pruning checks treat it.
    pub const NAI: Interval = Interval { min: f32::NAN, max: f32::NAN };
    pub const INFINITE: Interval = Interval { min: f32::NEG_INFINITY, max: f32::INFINITY };

    pub fn of(min: f32, max: f32) -> Interval {
        assert!(!(max < min) && !min.is_nan() && !max.is_nan(), "invalid interval [{min}, {max}]");
        Interval { min, max }
    }

    pub fn symmetric(v: f32) -> Interval {
        Self::of(-v, v)
    }

    pub fn exact(v: f32) -> Interval {
        Self::of(v, v)
    }

    pub fn min(&self) -> f32 {
        self.min
    }

    pub fn max(&self) -> f32 {
        self.max
    }

    pub fn is_nai(&self) -> bool {
        self.min.is_nan()
    }

    pub fn contains(&self, v: f32) -> bool {
        v >= self.min && v <= self.max
    }

    pub fn encapsulating(items: impl IntoIterator<Item = Interval>) -> Interval {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for i in items {
            if !i.is_nai() {
                lo = jm::min(i.min, lo);
                hi = jm::max(i.max, hi);
            }
        }
        if hi < lo { Self::NAI } else { Self::of(lo, hi) }
    }

    pub fn encapsulating_values(a: f32, b: f32) -> Interval {
        match (a.is_nan(), b.is_nan()) {
            (true, true) => Self::NAI,
            (true, false) => Self::exact(b),
            (false, true) => Self::exact(a),
            (false, false) => Self::of(jm::min(a, b), jm::max(a, b)),
        }
    }

    pub fn add(a: Interval, b: Interval) -> Interval {
        let (lo, hi) = (a.min + b.min, a.max + b.max);
        if lo.is_nan() || hi.is_nan() { Self::NAI } else { Self::of(lo, hi) }
    }

    pub fn sub(a: Interval, b: Interval) -> Interval {
        let (lo, hi) = (a.min - b.max, a.max - b.min);
        if lo.is_nan() || hi.is_nan() { Self::NAI } else { Self::of(lo, hi) }
    }

    fn mul_bound(a: f32, b: f32) -> f32 {
        if a == 0.0 || b == 0.0 { 0.0 } else { a * b }
    }

    pub fn mul(a: Interval, b: Interval) -> Interval {
        if a.is_nai() || b.is_nai() {
            return Self::NAI;
        }
        let p1 = Self::mul_bound(a.min, b.min);
        let p2 = Self::mul_bound(a.min, b.max);
        let p3 = Self::mul_bound(a.max, b.min);
        let p4 = Self::mul_bound(a.max, b.max);
        Self::of(jm::min(jm::min(p1, p2), jm::min(p3, p4)), jm::max(jm::max(p1, p2), jm::max(p3, p4)))
    }

    pub fn reciprocal(a: Interval) -> Interval {
        if a.is_nai() || (a.min == 0.0 && a.max == 0.0) {
            Self::NAI
        } else if !a.contains(0.0) {
            Self::of(1.0 / a.max, 1.0 / a.min)
        } else if a.max == 0.0 {
            Self::of(f32::NEG_INFINITY, 1.0 / a.min)
        } else if a.min == 0.0 {
            Self::of(1.0 / a.max, f32::INFINITY)
        } else {
            Self::INFINITE
        }
    }

    pub fn div(a: Interval, b: Interval) -> Interval {
        Self::mul(a, Self::reciprocal(b))
    }

    pub fn min_of(a: Interval, b: Interval) -> Interval {
        if a.is_nai() || b.is_nai() {
            return Self::NAI;
        }
        Self::of(jm::min(a.min, b.min), jm::min(a.max, b.max))
    }

    pub fn max_of(a: Interval, b: Interval) -> Interval {
        if a.is_nai() || b.is_nai() {
            return Self::NAI;
        }
        Self::of(jm::max(a.min, b.min), jm::max(a.max, b.max))
    }

    pub fn clamp(i: Interval, lo: f32, hi: f32) -> Interval {
        assert!(!(lo > hi), "clamp bounds out of order");
        if i.is_nai() {
            Self::NAI
        } else if !(i.min < hi) {
            Self::exact(hi)
        } else if !(i.max > lo) {
            Self::exact(lo)
        } else {
            Self::of(jm::max(i.min, lo), jm::min(i.max, hi))
        }
    }

    pub fn abs(i: Interval) -> Interval {
        Self::even(i, f32::abs)
    }

    pub fn square(i: Interval) -> Interval {
        Self::even(i, jm::square)
    }

    fn even(i: Interval, f: fn(f32) -> f32) -> Interval {
        if i.is_nai() {
            return Self::NAI;
        }
        let hi = jm::max(f(i.min), f(i.max));
        if i.contains(0.0) { Self::of(0.0, hi) } else { Self::of(jm::min(f(i.min), f(i.max)), hi) }
    }

    pub fn map_monotonic(i: Interval, f: impl Fn(f32) -> f32) -> Interval {
        if i.is_nai() {
            return Self::NAI;
        }
        let (a, b) = (f(i.min), f(i.max));
        assert!(!a.is_nan() && !b.is_nan(), "monotonic operator should not produce NaN");
        Self::of(jm::min(a, b), jm::max(a, b))
    }

    /// `Interval.pow` restricted to an exact exponent, which is all density functions need
    /// (`sqrt` uses 0.5).
    pub fn pow_exact(i: Interval, exponent: f32) -> Interval {
        let base = |b: f32| -> Interval {
            let v = kiln_javamath::pow::pow(b as f64, exponent as f64) as f32;
            if v.is_nan() { Self::NAI } else { Self::exact(v) }
        };
        if i.is_nai() {
            return Self::NAI;
        }
        if i.min == i.max {
            return base(i.min);
        }
        let mut r = Self::encapsulating([base(i.min), base(i.max)]);
        if i.contains(0.0) {
            if i.max > 0.0 {
                r = Self::encapsulating([r, base(0.0)]);
            }
            if i.min < 0.0 {
                r = Self::encapsulating([r, base(-0.0)]);
            }
        }
        r
    }

    pub fn log(i: Interval) -> Interval {
        if i.max < 0.0 {
            return Self::NAI;
        }
        Self::map_monotonic(Self::max_of(i, Self::exact(0.0)), |v| kiln_javamath::pow::log(v as f64) as f32)
    }

    pub fn sign(i: Interval) -> Interval {
        if i.is_nai() {
            Self::NAI
        } else if i.min == i.max {
            Self::exact(jm::signum(i.min))
        } else if i.contains(0.0) {
            if i.min == 0.0 {
                Self::of(0.0, 1.0)
            } else if i.max == 0.0 {
                Self::of(-1.0, 0.0)
            } else {
                Self::of(-1.0, 1.0)
            }
        } else {
            Self::exact(if i.min > 0.0 { 1.0 } else { -1.0 })
        }
    }

    /// Range of `lerp(alpha, first, second)`.
    pub fn lerp(alpha: Interval, first: Interval, second: Interval) -> Interval {
        if alpha.is_nai() || first.is_nai() || second.is_nai() {
            return Self::NAI;
        }
        Self::encapsulating([
            Self::lerp_values(alpha, first.min, second.min),
            Self::lerp_values(alpha, first.max, second.min),
            Self::lerp_values(alpha, first.min, second.max),
            Self::lerp_values(alpha, first.max, second.max),
        ])
    }

    fn lerp_values(alpha: Interval, a: f32, b: f32) -> Interval {
        if alpha.is_nai() || a.is_nan() || b.is_nan() {
            return Self::NAI;
        }
        if a.is_finite() && b.is_finite() {
            let bound = |t: f32| a + Self::mul_bound(t, b - a);
            return Self::encapsulating_values(bound(alpha.min), bound(alpha.max));
        }
        if a == b {
            return Self::exact(a);
        }
        let bound = |t: f32| -> f32 {
            let p = Self::mul_bound(1.0 - t, a);
            let q = Self::mul_bound(t, b);
            if p.is_infinite() && q.is_infinite() {
                if !(t > 0.0) {
                    if b > a { f32::NEG_INFINITY } else { f32::INFINITY }
                } else if !(t < 1.0) {
                    if b > a { f32::INFINITY } else { f32::NEG_INFINITY }
                } else {
                    f32::NAN
                }
            } else {
                p + q
            }
        };
        let (lo, hi) = (bound(alpha.min), bound(alpha.max));
        if lo.is_nan() || hi.is_nan() { Self::NAI } else { Self::encapsulating_values(lo, hi) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = Interval::of(-1.0, 2.0);
        let b = Interval::of(3.0, 4.0);
        assert_eq!(Interval::add(a, b), Interval::of(2.0, 6.0));
        assert_eq!(Interval::sub(a, b), Interval::of(-5.0, -1.0));
        assert_eq!(Interval::mul(a, b), Interval::of(-4.0, 8.0));
        assert_eq!(Interval::abs(a), Interval::of(0.0, 2.0));
        assert_eq!(Interval::reciprocal(b), Interval::of(0.25, 1.0 / 3.0));
        assert!(Interval::reciprocal(Interval::exact(0.0)).is_nai());
        assert_eq!(Interval::clamp(Interval::of(5.0, 9.0), 0.0, 1.0), Interval::exact(1.0));
        // Multiplying by an infinite bound through a zero bound stays 0, not NaN.
        assert_eq!(Interval::mul(Interval::of(0.0, 1.0), Interval::INFINITE), Interval::INFINITE);
    }

    #[test]
    fn nai_propagates_and_fails_comparisons() {
        let n = Interval::NAI;
        assert!(Interval::add(n, Interval::exact(1.0)).is_nai());
        assert!(!(n.max() < 0.0) && !(n.min() > 0.0));
        assert_eq!(Interval::encapsulating([n, Interval::exact(2.0)]), Interval::exact(2.0));
    }
}
