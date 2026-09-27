//! `Math.sin` and `Math.cos` as close to the JVM as practical.
//!
//! On x86-64, HotSpot computes `Math.sin`/`Math.cos` with Intel's libm stubs (in the
//! interpreter too), which are correctly rounded except within about 1% of an ulp of a
//! rounding midpoint. These functions return the correctly rounded result (double-double
//! reduction and series), which agrees with the JVM on about 99.75% of arguments (measured on
//! 600,000 values against JDK 25), against about 94% for the platform C library.

#[derive(Clone, Copy)]
struct Dd(f64, f64);

impl Dd {
    fn neg(self) -> Dd {
        Dd(-self.0, -self.1)
    }
}

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd(s, (a - (s - bb)) + (b - bb))
}

fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd(s, b - (s - a))
}

fn add(a: Dd, b: Dd) -> Dd {
    let s = two_sum(a.0, b.0);
    let t = two_sum(a.1, b.1);
    let s = quick_two_sum(s.0, s.1 + t.0);
    quick_two_sum(s.0, s.1 + t.1)
}

fn mul(a: Dd, b: Dd) -> Dd {
    let p = a.0 * b.0;
    quick_two_sum(p, a.0.mul_add(b.0, -p) + (a.0 * b.1 + a.1 * b.0))
}

fn mul_f64(a: Dd, b: f64) -> Dd {
    let p = a.0 * b;
    quick_two_sum(p, a.0.mul_add(b, -p) + a.1 * b)
}

fn div_f64(a: Dd, b: f64) -> Dd {
    let q = a.0 / b;
    let r = add(a, mul_f64(Dd(q, 0.0), -b));
    quick_two_sum(q, r.0 / b)
}

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

/// Taylor series of `sin` (odd) or `cos` (even) on the reduced argument.
fn series(r: Dd, even: bool) -> Dd {
    let r2 = mul(r, r);
    let (mut term, mut k) = if even { (Dd(1.0, 0.0), 0.0) } else { (r, 1.0) };
    let mut sum = term;
    for _ in 0..16 {
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
        // The three-part π/2 no longer leaves enough bits after cancellation.
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
}
