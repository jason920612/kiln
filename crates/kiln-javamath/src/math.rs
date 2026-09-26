//! `java.lang.Math` and `net.minecraft.util.Mth` operations with Java semantics.

/// `Math.min(float, float)`: NaN if either argument is NaN, and `-0.0 < +0.0`.
#[inline]
pub fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b { a } else { b }
}

/// `Math.max(float, float)`: NaN if either argument is NaN, and `-0.0 < +0.0`.
#[inline]
pub fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Math.signum(float)`: zeros and NaN map to themselves.
#[inline]
pub fn signum(a: f32) -> f32 {
    if a == 0.0 || a.is_nan() { a } else { 1.0f32.copysign(a) }
}

/// `Mth.floor(double)`: `(int) Math.floor(x)`, saturating, NaN to 0 (as Rust `as` does).
#[inline]
pub fn floor(x: f64) -> i32 {
    x.floor() as i32
}

/// `Mth.floor(float)`.
#[inline]
pub fn floor_f32(x: f32) -> i32 {
    (x as f64).floor() as i32
}

/// `Mth.lfloor(double)`.
#[inline]
pub fn lfloor(x: f64) -> i64 {
    x.floor() as i64
}

/// `Math.floorDiv(int, int)`.
#[inline]
pub fn floor_div(x: i32, y: i32) -> i32 {
    let q = x.wrapping_div(y);
    if (x ^ y) < 0 && q.wrapping_mul(y) != x { q - 1 } else { q }
}

/// `Math.floorMod(int, int)`.
#[inline]
pub fn floor_mod(x: i32, y: i32) -> i32 {
    let r = x.wrapping_rem(y);
    if (x ^ y) < 0 && r != 0 { r + y } else { r }
}

/// `Mth.clamp(float, float, float)`: `v < lo ? lo : Math.min(v, hi)`.
#[inline]
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else { min(v, hi) }
}

/// `Mth.lerp(float, float, float)`: `a + t * (b - a)`.
#[inline]
pub fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// `Mth.lerp2`: bilinear, x first.
#[inline]
pub fn lerp2(tx: f32, ty: f32, v00: f32, v10: f32, v01: f32, v11: f32) -> f32 {
    lerp(ty, lerp(tx, v00, v10), lerp(tx, v01, v11))
}

/// `Mth.lerp3`: trilinear, x then y then z; corners in (x, y, z) bit order.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn lerp3(tx: f32, ty: f32, tz: f32, v: [f32; 8]) -> f32 {
    lerp(tz, lerp2(tx, ty, v[0], v[1], v[2], v[3]), lerp2(tx, ty, v[4], v[5], v[6], v[7]))
}

/// `Mth.smoothstep(float)`: `6t⁵ - 15t⁴ + 10t³` in Java's evaluation order.
#[inline]
pub fn smoothstep(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// `Mth.square(float)`.
#[inline]
pub fn square(x: f32) -> f32 {
    x * x
}

/// `Mth.cube(float)`.
#[inline]
pub fn cube(x: f32) -> f32 {
    x * x * x
}

/// `Mth.getSeed(int, int, int)`: the per-position seed used by positional random factories.
pub fn get_seed(x: i32, y: i32, z: i32) -> i64 {
    let mut seed = (x.wrapping_mul(3_129_871) as i64) ^ (z as i64).wrapping_mul(116_129_781) ^ y as i64;
    seed = seed.wrapping_mul(seed).wrapping_mul(42_317_861).wrapping_add(seed.wrapping_mul(11));
    seed >> 16
}

/// `String.hashCode()` over UTF-16 code units.
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_max_follow_java() {
        assert!(min(f32::NAN, 1.0).is_nan());
        assert!(min(1.0, f32::NAN).is_nan());
        assert!(min(0.0, -0.0).is_sign_negative());
        assert!(min(-0.0, 0.0).is_sign_negative());
        assert!(max(-0.0, 0.0).is_sign_positive());
        assert!(max(0.0, -0.0).is_sign_positive());
        assert_eq!(max(-3.0, 2.0), 2.0);
    }

    #[test]
    fn signum_keeps_zero_sign() {
        assert!(signum(-0.0).is_sign_negative() && signum(-0.0) == 0.0);
        assert_eq!(signum(0.0).to_bits(), 0);
        assert_eq!(signum(-7.5), -1.0);
    }

    #[test]
    fn floor_div_mod() {
        assert_eq!(floor_div(-7, 4), -2);
        assert_eq!(floor_mod(-7, 4), 1);
        assert_eq!(floor_div(7, -4), -2);
        assert_eq!(floor_mod(7, -4), -1);
        assert_eq!(floor_div(i32::MIN, -1), i32::MIN);
        assert_eq!(floor(-0.5), -1);
        assert_eq!(floor(f64::NAN), 0);
        assert_eq!(floor(1e300), i32::MAX);
    }

    #[test]
    fn string_hash_matches_java() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("octave_-9"), 440_898_203);
        assert_eq!(string_hash("minecraft:terrain"), 1_657_813_608);
    }
}
