//! `Math.asin`, `Math.acos` and `Math.log1p`: HotSpot has no intrinsic for them, so they are the
//! JDK's `StrictMath`, the fdlibm algorithms (`e_asin.c`, `e_acos.c`, `s_log1p.c`) bit for bit.
//! Checked against JDK 25 in `tests/jvm_dump.rs`.

const PIO2_HI: f64 = 1.570_796_326_794_896_6;
const PIO2_LO: f64 = 6.123_233_995_736_766e-17;
const PIO4_HI: f64 = 0.785_398_163_397_448_3;
const P_S0: f64 = 0.166_666_666_666_666_66;
const P_S1: f64 = -0.325_565_818_622_400_9;
const P_S2: f64 = 0.201_212_532_134_862_93;
const P_S3: f64 = -0.040_055_534_500_679_41;
const P_S4: f64 = 7.915_349_942_898_145e-4;
const P_S5: f64 = 3.479_331_075_960_212e-5;
const Q_S1: f64 = -2.403_394_911_734_414;
const Q_S2: f64 = 2.020_945_760_233_505_7;
const Q_S3: f64 = -0.688_283_971_605_453_3;
const Q_S4: f64 = 0.077_038_150_555_901_94;

fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn lo(x: f64) -> u32 {
    x.to_bits() as u32
}

/// `x` with its low word cleared.
fn clear_lo(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & 0xffff_ffff_0000_0000)
}

fn rational_p(t: f64) -> f64 {
    t * (P_S0 + t * (P_S1 + t * (P_S2 + t * (P_S3 + t * (P_S4 + t * P_S5)))))
}

fn rational_q(t: f64) -> f64 {
    1.0 + t * (Q_S1 + t * (Q_S2 + t * (Q_S3 + t * Q_S4)))
}

/// `Math.asin(double)`.
pub fn asin(x: f64) -> f64 {
    let hx = hi(x);
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        // |x| >= 1
        if ((ix - 0x3ff0_0000) as u32 | lo(x)) == 0 {
            return x * PIO2_HI + x * PIO2_LO;
        }
        return f64::NAN;
    } else if ix < 0x3fe0_0000 {
        // |x| < 0.5
        if ix < 0x3e40_0000 {
            // |x| < 2^-27
            return x;
        }
        let t = x * x;
        let p = rational_p(t);
        let q = rational_q(t);
        let w = p / q;
        return x + x * w;
    }
    // 1 > |x| >= 0.5
    let w = 1.0 - x.abs();
    let t = w * 0.5;
    let p = rational_p(t);
    let q = rational_q(t);
    let s = t.sqrt();
    let t = if ix >= 0x3fef_3333 {
        // |x| > 0.975
        let w = p / q;
        PIO2_HI - (2.0 * (s + s * w) - PIO2_LO)
    } else {
        let w = clear_lo(s);
        let c = (t - w * w) / (s + w);
        let r = p / q;
        let p = 2.0 * s * r - (PIO2_LO - 2.0 * c);
        let q = PIO4_HI - 2.0 * w;
        PIO4_HI - (p - q)
    };
    if hx > 0 { t } else { -t }
}

/// `Math.acos(double)`.
pub fn acos(x: f64) -> f64 {
    const PI: f64 = 3.141_592_653_589_793;
    let hx = hi(x);
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        if ((ix - 0x3ff0_0000) as u32 | lo(x)) == 0 {
            // |x| == 1
            return if hx > 0 { 0.0 } else { PI + 2.0 * PIO2_LO };
        }
        return f64::NAN;
    }
    if ix < 0x3fe0_0000 {
        // |x| < 0.5
        if ix <= 0x3c60_0000 {
            return PIO2_HI + PIO2_LO;
        }
        let z = x * x;
        let p = rational_p(z);
        let q = rational_q(z);
        let r = p / q;
        PIO2_HI - (x - (PIO2_LO - x * r))
    } else if hx < 0 {
        // x < -0.5
        let z = (1.0 + x) * 0.5;
        let p = rational_p(z);
        let q = rational_q(z);
        let s = z.sqrt();
        let r = p / q;
        let w = r * s - PIO2_LO;
        PI - 2.0 * (s + w)
    } else {
        // x > 0.5
        let z = (1.0 - x) * 0.5;
        let s = z.sqrt();
        let df = clear_lo(s);
        let c = (z - df * df) / (s + df);
        let p = rational_p(z);
        let q = rational_q(z);
        let r = p / q;
        let w = r * s + c;
        2.0 * (df + w)
    }
}

const LN2_HI: f64 = 0.693_147_180_369_123_8;
const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
const LP1: f64 = 0.666_666_666_666_673_5;
const LP2: f64 = 0.399_999_999_994_094_2;
const LP3: f64 = 0.285_714_287_436_623_9;
const LP4: f64 = 0.222_221_984_321_497_84;
const LP5: f64 = 0.181_835_721_616_180_5;
const LP6: f64 = 0.153_138_376_992_093_73;
const LP7: f64 = 0.147_981_986_051_165_86;

/// `x` with its high word replaced.
fn with_hi(x: f64, h: i32) -> f64 {
    f64::from_bits(((h as u32 as u64) << 32) | (x.to_bits() & 0xffff_ffff))
}

/// `Math.log1p(double)`.
pub fn log1p(x: f64) -> f64 {
    const TWO54: f64 = 1.801_439_850_948_198_4e16;
    let hx = hi(x);
    let ax = hx & 0x7fff_ffff;
    let mut k = 1i32;
    let mut f = 0.0;
    let mut c = 0.0;
    let mut hu = 0i32;
    if hx < 0x3FDA_827A {
        // x < 0.41422
        if ax >= 0x3ff0_0000 {
            // x <= -1
            return if x == -1.0 { f64::NEG_INFINITY } else { f64::NAN };
        }
        if ax < 0x3e20_0000 {
            // |x| < 2^-29
            if TWO54 + x > 0.0 && ax < 0x3c90_0000 {
                return x;
            }
            return x - x * x * 0.5;
        }
        if hx > 0 || hx <= 0xbfd2_bec3u32 as i32 {
            // -0.2929 < x < 0.41422
            k = 0;
            f = x;
            hu = 1;
        }
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    if k != 0 {
        let mut u;
        if hx < 0x4340_0000 {
            u = 1.0 + x;
            hu = hi(u);
            k = (hu >> 20) - 1023;
            c = if k > 0 { 1.0 - (u - x) } else { x - (u - 1.0) };
            c /= u;
        } else {
            u = x;
            hu = hi(u);
            k = (hu >> 20) - 1023;
            c = 0.0;
        }
        hu &= 0x000f_ffff;
        if hu < 0x6a09e {
            u = with_hi(u, hu | 0x3ff0_0000);
        } else {
            k += 1;
            u = with_hi(u, hu | 0x3fe0_0000);
            hu = (0x0010_0000 - hu) >> 2;
        }
        f = u - 1.0;
    }
    let hfsq = 0.5 * f * f;
    let kf = k as f64;
    if hu == 0 {
        // |f| < 2^-20
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            c += kf * LN2_LO;
            return kf * LN2_HI + c;
        }
        let r = hfsq * (1.0 - 0.666_666_666_666_666_66 * f);
        if k == 0 {
            return f - r;
        }
        return kf * LN2_HI - ((r - (kf * LN2_LO + c)) - f);
    }
    let s = f / (2.0 + f);
    let z = s * s;
    let r = z * (LP1 + z * (LP2 + z * (LP3 + z * (LP4 + z * (LP5 + z * (LP6 + z * LP7))))));
    if k == 0 {
        f - (hfsq - s * (hfsq + r))
    } else {
        kf * LN2_HI - ((hfsq - (s * (hfsq + r) + (kf * LN2_LO + c))) - f)
    }
}
