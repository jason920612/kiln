//! `Math.sin` and `Math.cos` as close to the JVM as practical.
//!
//! On x86-64, HotSpot computes `Math.sin`/`Math.cos` with Intel's libm stubs (in the
//! interpreter too), which are correctly rounded except within about 1% of an ulp of a
//! rounding midpoint. These functions return the correctly rounded result (double-double
//! reduction and series), which agrees with the JVM on about 99.75% of arguments (measured on
//! 600,000 values against JDK 25), against about 94% for the platform C library.

use crate::dd::{Dd, add, div_f64, mul};

/// π/2 as a sum of three doubles (about 160 bits).
const PIO2: [f64; 3] = [
    f64::from_bits(0x3FF9_21FB_5444_2D18),
    f64::from_bits(0x3C91_A626_3314_5C07),
    f64::from_bits(0xB91F_1976_B7ED_8FBC),
];

/// `x = n·π/2 + r` with `|r| <= π/4` (to within rounding), `r` in double-double.
fn reduce(x: f64) -> (i64, Dd) {
    let n = (x * std::f64::consts::FRAC_2_PI).round();
    let mut r = Dd(x, 0.0);
    for p in PIO2 {
        let hi = n * p;
        r = add(r, Dd(hi, n.mul_add(p, -hi)).neg());
    }
    (n as i64, r)
}

/// Terms of the series beyond the first.
const TERMS: usize = 16;

/// The series' coefficients in double-double: `(-1)^k / (2k+1)!` (sine) and `(-1)^k / (2k)!`
/// (cosine) for `k` in `0..=TERMS`, made once by the same division steps the term-by-term sum
/// takes.
fn coefficients() -> &'static [[Dd; TERMS + 1]; 2] {
    static C: std::sync::OnceLock<[[Dd; TERMS + 1]; 2]> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let mut c = [[Dd(0.0, 0.0); TERMS + 1]; 2];
        for (even, row) in c.iter_mut().enumerate() {
            let (mut term, mut k) = if even == 1 { (Dd(1.0, 0.0), 0.0) } else { (Dd(1.0, 0.0), 1.0) };
            row[0] = term;
            for slot in row.iter_mut().skip(1) {
                term = div_f64(div_f64(term.neg(), k + 1.0), k + 2.0);
                k += 2.0;
                *slot = term;
            }
        }
        c
    })
}

/// Taylor series of `sin` (odd) or `cos` (even) on the reduced argument, by Horner's rule in
/// `r²` (the precision is double-double either way, which leaves the rounded result as the
/// term-by-term sum gives it).
fn series(r: Dd, even: bool) -> Dd {
    let r2 = mul(r, r);
    let c = &coefficients()[usize::from(even)];
    let mut sum = c[TERMS];
    for k in (0..TERMS).rev() {
        sum = add(mul(sum, r2), c[k]);
    }
    if even { sum } else { mul(sum, r) }
}

/// The term-by-term sum (the reference [`series`] is checked against).
#[cfg(test)]
fn series_by_terms(r: Dd, even: bool) -> Dd {
    let r2 = mul(r, r);
    let (mut term, mut k) = if even { (Dd(1.0, 0.0), 0.0) } else { (r, 1.0) };
    let mut sum = term;
    for _ in 0..TERMS {
        term = div_f64(div_f64(mul(term, r2).neg(), k + 1.0), k + 2.0);
        k += 2.0;
        sum = add(sum, term);
    }
    sum
}

/// `sin(x)` for `quadrant_shift = 0`, `cos(x)` for 1.
fn eval(x: f64, quadrant_shift: i64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() > 1e9 {
        // The three-part π/2 no longer leaves enough bits after cancellation; no game code
        // gets here (angles stay within a few thousand radians), so the platform libm answers.
        return if quadrant_shift == 0 { x.sin() } else { x.cos() };
    }
    if x == 0.0 && quadrant_shift == 0 {
        return x;
    }
    let (n, r) = reduce(x);
    let v = match (n + quadrant_shift).rem_euclid(4) {
        0 => series(r, false),
        1 => series(r, true),
        2 => series(r, false).neg(),
        _ => series(r, true).neg(),
    };
    v.0
}

/// `Math.sin(double)`.
pub fn sin(x: f64) -> f64 {
    eval(x, 0)
}

/// `Math.cos(double)`.
pub fn cos(x: f64) -> f64 {
    eval(x, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits from `Math.sin`/`Math.cos` on JDK 25; the first cosine is one ulp off in the
    /// Windows C library.
    #[test]
    #[allow(clippy::excessive_precision)] // inputs as Java printed them
    fn matches_java() {
        let cases: [(f64, u64, u64); 4] = [
            (2.702_454_318_956_087_2, 0x3fdb35d116a86f8c, 0xbfecf6babf20251a),
            (5.525_250_120_739_501, 0xbfe5ff5f3426e4a9, 0x3fe73d7f3be1a2d2),
            (3.676_306_928_595_664, 0xbfe04e9aa1a353bc, 0xbfeb88838104f926),
            (4.272_454_318_956_087_2, 0xbfecf3f41ecb99a2, 0xbfdb419fa6b70578),
        ];
        for (x, s, c) in cases {
            assert_eq!(sin(x).to_bits(), s, "sin({x})");
            assert_eq!(cos(x).to_bits(), c, "cos({x})");
        }
        assert_eq!(sin(0.0), 0.0);
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(sin(-0.0).to_bits(), (-0.0f64).to_bits());
    }

    /// Horner's rule rounds as the term-by-term sum on a million arguments across the reduced
    /// range (and a spread of whole arguments).
    #[test]
    fn horner_rounds_as_the_terms() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for i in 0..1_000_000u32 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let unit = (x >> 11) as f64 / (1u64 << 53) as f64;
            let r = if i % 2 == 0 { (unit - 0.5) * std::f64::consts::FRAC_PI_2 } else { (unit - 0.5) * 2000.0 };
            let (_, red) = reduce(r);
            for even in [false, true] {
                assert_eq!(series(red, even).0.to_bits(), series_by_terms(red, even).0.to_bits(), "series({r}, {even})");
            }
        }
    }
}
