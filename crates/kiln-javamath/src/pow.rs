//! `Math.log` and `Math.pow` as close to the JVM as practical.
//!
//! On x86-64, HotSpot computes both with Intel's libm stubs (interpreter and compiled code
//! alike), which are accurate to well under one ulp but not correctly rounded. These functions
//! return the correctly rounded result (double-double logarithm and exponential), which is what
//! the stubs return on nearly every argument; `tests/jvm_dump.rs` measures the agreement against
//! a JDK. The platform C library disagrees with the JVM far more often, and differently per OS.
//!
//! `Math.pow`'s special cases follow the Java specification, which differs from C's in a few
//! places (`pow(1, ±inf)` and `pow(1, NaN)` are NaN).

use crate::dd::{Dd, add, div, div_f64, mul, mul_f64, scale, sub, two_sum};
use std::sync::OnceLock;

/// ln 2 in double-double.
const LN2: Dd = Dd(f64::from_bits(0x3FE6_2E42_FEFA_39EF), f64::from_bits(0x3C7A_BC9E_3B39_803F));
const SQRT2: f64 = std::f64::consts::SQRT_2;
const TWO54: f64 = 18014398509481984.0;

/// Terms of the `atanh` series beyond the first (`|s| <= 0.1716`, so `s²` gains over 5 bits a term).
const LOG_TERMS: usize = 24;
/// Terms of the exponential series beyond the first (`|r| <= 0.00136` after scaling).
const EXP_TERMS: usize = 13;

/// `1/(2k+1)` for the `atanh` series.
fn log_coefficients() -> &'static [Dd; LOG_TERMS + 1] {
    static C: OnceLock<[Dd; LOG_TERMS + 1]> = OnceLock::new();
    C.get_or_init(|| std::array::from_fn(|k| div_f64(Dd(1.0, 0.0), (2 * k + 1) as f64)))
}

/// `1/i!` for the exponential series.
fn exp_coefficients() -> &'static [Dd; EXP_TERMS + 1] {
    static C: OnceLock<[Dd; EXP_TERMS + 1]> = OnceLock::new();
    C.get_or_init(|| {
        let mut c = [Dd(0.0, 0.0); EXP_TERMS + 1];
        c[0] = Dd(1.0, 0.0);
        for i in 1..=EXP_TERMS {
            c[i] = div_f64(c[i - 1], i as f64);
        }
        c
    })
}

/// `ln(x)` for finite `x > 0`, in double-double: `x = 2^e · m`, `ln m = 2·atanh((m-1)/(m+1))`.
fn log_dd(x: f64) -> Dd {
    let mut bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7ff) as i64;
    if e == 0 {
        // Subnormal: scale into the normal range first.
        bits = (x * TWO54).to_bits();
        e = ((bits >> 52) & 0x7ff) as i64 - 54;
    }
    e -= 1023;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > SQRT2 {
        m *= 0.5;
        e += 1;
    }
    // m is in [0.7071, 1.4143], so m - 1 is exact and m + 1 is held as a pair.
    let s = div(Dd(m - 1.0, 0.0), two_sum(m, 1.0));
    let s2 = mul(s, s);
    let c = log_coefficients();
    let mut sum = c[LOG_TERMS];
    for k in (0..LOG_TERMS).rev() {
        sum = add(mul(sum, s2), c[k]);
    }
    let l = scale(mul(sum, s), 2.0);
    if e == 0 { l } else { add(mul_f64(LN2, e as f64), l) }
}

/// `Math.log(double)`.
pub fn log(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        f64::NAN
    } else if x == 0.0 {
        f64::NEG_INFINITY
    } else if x.is_infinite() {
        x
    } else if x == 1.0 {
        0.0
    } else {
        log_dd(x).0
    }
}

/// `2^k` for `k` in the normal exponent range.
fn pow2(k: i64) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// `exp(t)` rounded to a double, `t` in double-double.
fn exp_dd(t: Dd) -> f64 {
    if t.0 > 710.0 {
        return f64::INFINITY;
    }
    if t.0 < -746.0 {
        return 0.0;
    }
    let k = (t.0 * std::f64::consts::LOG2_E).round();
    // exp(t) = 2^k · exp(r), |r| <= ln2/2; exp(r) = (exp(r/256))^256 through expm1 doubling.
    let r = scale(sub(t, mul_f64(LN2, k)), 1.0 / 256.0);
    let c = exp_coefficients();
    let mut p = c[EXP_TERMS];
    for i in (1..EXP_TERMS).rev() {
        p = add(mul(p, r), c[i]);
    }
    let mut p = mul(p, r);
    for _ in 0..8 {
        p = add(scale(p, 2.0), mul(p, p));
    }
    let e = add(Dd(1.0, 0.0), p).0;
    let k = k as i64;
    e * pow2(k / 2) * pow2(k - k / 2)
}

/// `Math.pow(double, double)`.
pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    if y == 1.0 {
        return x;
    }
    if y == 2.0 {
        return x * x;
    }
    if y.is_infinite() {
        let ax = x.abs();
        if ax == 1.0 {
            return f64::NAN;
        }
        return if (ax > 1.0) == (y > 0.0) { f64::INFINITY } else { 0.0 };
    }
    let y_int = y.fract() == 0.0;
    let y_odd = y_int && y.abs() < 9007199254740992.0 && (y as i64) & 1 == 1;
    if x == 0.0 {
        let neg = x.is_sign_negative() && y_odd;
        return match (y > 0.0, neg) {
            (true, false) => 0.0,
            (true, true) => -0.0,
            (false, false) => f64::INFINITY,
            (false, true) => f64::NEG_INFINITY,
        };
    }
    if x.is_infinite() {
        if x > 0.0 {
            return if y > 0.0 { f64::INFINITY } else { 0.0 };
        }
        return match (y > 0.0, y_odd) {
            (true, true) => f64::NEG_INFINITY,
            (true, false) => f64::INFINITY,
            (false, true) => -0.0,
            (false, false) => 0.0,
        };
    }
    let (ax, neg) = if x < 0.0 {
        if !y_int {
            return f64::NAN;
        }
        (-x, y_odd)
    } else {
        (x, false)
    };
    let r = if ax == 1.0 { 1.0 } else { exp_dd(mul_f64(log_dd(ax), y)) };
    if neg { -r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ln 2 = 2·atanh(1/3), summed in double-double with far more terms than the table uses.
    #[test]
    fn ln2_constant() {
        let s = div_f64(Dd(1.0, 0.0), 3.0);
        let s2 = mul(s, s);
        let mut sum = Dd(0.0, 0.0);
        let mut term = s;
        for k in 0..80 {
            sum = add(sum, div_f64(term, (2 * k + 1) as f64));
            term = mul(term, s2);
        }
        let ln2 = scale(sum, 2.0);
        let d = sub(ln2, LN2);
        assert!(d.0.abs() < 1e-31, "ln2 differs by {}", d.0);
    }

    #[test]
    fn exact_cases() {
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert_eq!(pow(10.0, 3.0), 1000.0);
        assert_eq!(pow(10.0, -3.0), 0.001);
        assert_eq!(pow(2.0, -1074.0), 5e-324);
        assert_eq!(pow(-2.0, 3.0), -8.0);
        assert!(pow(-8.0, 1.0 / 3.0).is_nan());
        assert_eq!(pow(0.0, 0.0), 1.0);
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert!(pow(1.0, f64::NAN).is_nan());
        assert_eq!(pow(0.0, -1.0), f64::INFINITY);
        assert_eq!(pow(-0.0, -3.0), f64::NEG_INFINITY);
        assert_eq!(pow(1e300, 2.0), f64::INFINITY);
        assert_eq!(pow(2.0, 1024.0), f64::INFINITY);
        assert_eq!(log(1.0).to_bits(), 0);
        assert_eq!(log(0.0), f64::NEG_INFINITY);
        assert!(log(-1.0).is_nan());
    }
}
