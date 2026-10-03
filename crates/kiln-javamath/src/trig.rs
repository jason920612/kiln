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

#[allow(clippy::excessive_precision)] // fdlibm's constants as printed
const ATAN_HI: [f64; 4] = [4.63647609000806093515e-01, 7.85398163397448278999e-01, 9.82793723247329054082e-01, 1.57079632679489655800e+00];
#[allow(clippy::excessive_precision)]
const ATAN_LO: [f64; 4] = [2.26987774529616870924e-17, 3.06161699786838301793e-17, 1.39033110312309984516e-17, 6.12323399573676603587e-17];
#[allow(clippy::excessive_precision)]
const AT: [f64; 11] = [
    3.33333333333329318027e-01,
    -1.99999999998764832476e-01,
    1.42857142725034663711e-01,
    -1.11111104054623557880e-01,
    9.09088713343650656196e-02,
    -7.69187620504482999495e-02,
    6.66107313738753120669e-02,
    -5.83357013379057348645e-02,
    4.97687799461593236017e-02,
    -3.65315727442169155270e-02,
    1.62858201153657823623e-02,
];

/// `StrictMath.atan` (fdlibm `s_atan.c`).
fn atan(x: f64) -> f64 {
    let hx = (x.to_bits() >> 32) as u32 as i32;
    let ix = hx & 0x7fff_ffff;
    let id: i32;
    let mut x = x;
    if ix >= 0x4410_0000 {
        // |x| >= 2^66
        if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && (x.to_bits() as u32) != 0) {
            return x + x;
        }
        return if hx > 0 { ATAN_HI[3] + ATAN_LO[3] } else { -ATAN_HI[3] - ATAN_LO[3] };
    }
    if ix < 0x3fdc_0000 {
        // |x| < 0.4375
        if ix < 0x3e20_0000 && 1.0e300 + x > 1.0 {
            return x;
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3ff3_0000 {
            if ix < 0x3fe6_0000 {
                id = 0;
                x = (2.0 * x - 1.0) / (2.0 + x);
            } else {
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x4003_8000 {
            id = 2;
            x = (x - 1.5) / (1.0 + 1.5 * x);
        } else {
            id = 3;
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id < 0 {
        return x - x * (s1 + s2);
    }
    let z = ATAN_HI[id as usize] - ((x * (s1 + s2) - ATAN_LO[id as usize]) - x);
    if hx < 0 { -z } else { z }
}

/// `Math.atan2(y, x)` (`StrictMath`, fdlibm `e_atan2.c`).
pub fn atan2(y: f64, x: f64) -> f64 {
    const TINY: f64 = 1.0e-300;
    #[allow(clippy::excessive_precision)]
    const PI_O_4: f64 = 7.8539816339744827900E-01;
    #[allow(clippy::excessive_precision)]
    const PI_O_2: f64 = 1.5707963267948965580E+00;
    #[allow(clippy::excessive_precision)]
    const PI: f64 = 3.1415926535897931160E+00;
    #[allow(clippy::excessive_precision)]
    const PI_LO: f64 = 1.2246467991473531772E-16;
    let (hx, lx) = ((x.to_bits() >> 32) as u32 as i32, x.to_bits() as u32);
    let (hy, ly) = ((y.to_bits() >> 32) as u32 as i32, y.to_bits() as u32);
    let (ix, iy) = (hx & 0x7fff_ffff, hy & 0x7fff_ffff);
    let nan = |i: i32, l: u32| ((i as u32) | ((l | l.wrapping_neg()) >> 31)) > 0x7ff0_0000;
    if nan(ix, lx) || nan(iy, ly) {
        return x + y;
    }
    if (hx.wrapping_sub(0x3ff0_0000) as u32 | lx) == 0 {
        return atan(y);
    }
    let m = ((hy >> 31) & 1) | ((hx >> 30) & 2);
    if (iy as u32 | ly) == 0 {
        return match m {
            0 | 1 => y,
            2 => PI + TINY,
            _ => -PI - TINY,
        };
    }
    if (ix as u32 | lx) == 0 {
        return if hy < 0 { -PI_O_2 - TINY } else { PI_O_2 + TINY };
    }
    if ix == 0x7ff0_0000 {
        if iy == 0x7ff0_0000 {
            return match m {
                0 => PI_O_4 + TINY,
                1 => -PI_O_4 - TINY,
                2 => 3.0 * PI_O_4 + TINY,
                _ => -3.0 * PI_O_4 - TINY,
            };
        }
        return match m {
            0 => 0.0,
            1 => -0.0,
            2 => PI + TINY,
            _ => -PI - TINY,
        };
    }
    if iy == 0x7ff0_0000 {
        return if hy < 0 { -PI_O_2 - TINY } else { PI_O_2 + TINY };
    }
    let k = (iy - ix) >> 20;
    let z = if k > 60 {
        PI_O_2 + 0.5 * PI_LO
    } else if hx < 0 && k < -60 {
        0.0
    } else {
        atan((y / x).abs())
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atan2_quadrants() {
        assert_eq!(atan2(1.0, 1.0), std::f64::consts::FRAC_PI_4);
        assert!((atan2(1.0, -1.0) - 3.0 * std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert!((atan2(-1.0, -1.0) + 3.0 * std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert_eq!(atan2(0.0, 1.0), 0.0);
        assert!((atan2(2.0, 0.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-15);
        assert!((atan2(0.3, 0.7) - 0.3f64.atan2(0.7)).abs() < 1e-15);
    }

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
