//! `Mth` helpers the mob code needs, Java-exact.

use kiln_javamath::random::RandomSource;

pub use crate::projectile::mth_atan2 as atan2;

fn sin_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..65536).map(|i| kiln_javamath::trig::sin(i as f64 * std::f64::consts::PI * 2.0 / 65536.0) as f32).collect())
}

/// `Mth.sin`: the 65536-entry table.
pub fn sin(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453) as i64 & 0xffff) as usize]
}

/// `Mth.cos`.
pub fn cos(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453 + 16384.0) as i64 & 0xffff) as usize]
}

/// `Mth.wrapDegrees(float)`: Java's `%` on floats is `fmod`, like Rust's.
pub fn wrap_degrees(v: f32) -> f32 {
    let mut f = v % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

pub fn wrap_degrees_d(v: f64) -> f64 {
    let mut f = v % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

/// `Mth.degreesDifference`.
pub fn degrees_difference(from: f32, to: f32) -> f32 {
    wrap_degrees(to - from)
}

/// `Mth.clamp(float)`: `Math.min(Math.max(v, lo), hi)` (NaN goes to... NaN, as in Java).
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    kiln_javamath::math::min(kiln_javamath::math::max(v, lo), hi)
}

pub fn clamp_d(v: f64, lo: f64, hi: f64) -> f64 {
    crate::math::jmin(crate::math::jmax(v, lo), hi)
}

/// `Mth.rotateIfNecessary`.
pub fn rotate_if_necessary(current: f32, target: f32, max: f32) -> f32 {
    let d = degrees_difference(current, target);
    let c = clamp(d, -max, max);
    target - c
}

/// `LookControl.rotateTowards`.
pub fn rotate_towards(from: f32, to: f32, max: f32) -> f32 {
    let f = degrees_difference(from, to);
    let g = clamp(f, -max, max);
    from + g
}

/// `Mth.positiveCeilDiv`.
pub fn positive_ceil_div(a: i32, b: i32) -> i32 {
    -(-a).div_euclid(b)
}

/// `Goal.reducedTickDelay`.
pub fn reduced_tick_delay(t: i32) -> i32 {
    positive_ceil_div(t, 2)
}

/// `Mth.sign(double)`.
pub fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// `Mth.sqrt(float)`.
pub fn sqrt_f(v: f32) -> f32 {
    (v as f64).sqrt() as f32
}

/// `Mth.lerp(float)`.
pub fn lerp_f(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// `Mth.ceil(double)`.
pub fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > i as f64 { i + 1 } else { i }
}

/// `RandomSource.triangle(mode, deviation)`.
pub fn triangle(r: &mut dyn RandomSource, mode: f64, deviation: f64) -> f64 {
    mode + deviation * (r.next_double() - r.next_double())
}

/// `RandomSource.nextIntBetweenInclusive`.
pub fn next_int_between(r: &mut dyn RandomSource, lo: i32, hi: i32) -> i32 {
    r.next_int_bounded(hi - lo + 1) + lo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_div() {
        assert_eq!(reduced_tick_delay(10), 5);
        assert_eq!(reduced_tick_delay(11), 6);
        assert_eq!(reduced_tick_delay(1), 1);
        assert_eq!(reduced_tick_delay(0), 0);
        assert_eq!(positive_ceil_div(120, 2), 60);
        assert_eq!(positive_ceil_div(7, 2), 4);
    }

    #[test]
    fn wrap() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees(180.0), -180.0);
    }
}
