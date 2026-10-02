//! `Math.atan2` (fdlibm's `e_atan2.c` and `s_atan.c`, which the JDK's `StrictMath.atan2` is a
//! port of, and `Math.atan2` calls it): bit-exact, unlike the platform C library's.

const ATAN_HI: [f64; 4] = [
    f64::from_bits(0x3FDD_AC67_0561_BB4F),
    f64::from_bits(0x3FE9_21FB_5444_2D18),
    f64::from_bits(0x3FEF_730B_D281_F69B),
    f64::from_bits(0x3FF9_21FB_5444_2D18),
];

const ATAN_LO: [f64; 4] = [
    f64::from_bits(0x3C7A_2B7F_222F_65E2),
    f64::from_bits(0x3C81_A626_3314_5C07),
    f64::from_bits(0x3C70_0788_7AF0_CBBD),
    f64::from_bits(0x3C91_A626_3314_5C07),
];

const A_T: [f64; 11] = [
    f64::from_bits(0x3FD5_5555_5555_550D),
    f64::from_bits(0xBFC9_9999_9998_EBC4),
    f64::from_bits(0x3FC2_4924_9200_83FF),
    f64::from_bits(0xBFBC_71C6_FE23_1671),
    f64::from_bits(0x3FB7_45CD_C54C_206E),
    f64::from_bits(0xBFB3_B0F2_AF74_9A6D),
    f64::from_bits(0x3FB1_0D66_A0D0_3D51),
    f64::from_bits(0xBFAD_DE2D_52DE_FD9A),
    f64::from_bits(0x3FA9_7B4B_2476_0DEB),
    f64::from_bits(0xBFA2_B444_2C6A_6C2F),
    f64::from_bits(0x3F90_AD3A_E322_DA11),
];

fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn lo(x: f64) -> u32 {
    x.to_bits() as u32
}

/// fdlibm `atan`.
fn atan(x: f64) -> f64 {
    let hx = hi(x);
    let ix = hx & 0x7fff_ffff;
    let id: i32;
    let mut x = x;
    if ix >= 0x4410_0000 {
        // |x| >= 2^66
        if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && lo(x) != 0) {
            return x + x;
        }
        return if hx > 0 { ATAN_HI[3] + ATAN_LO[3] } else { -ATAN_HI[3] - ATAN_LO[3] };
    }
    if ix < 0x3fdc_0000 {
        // |x| < 0.4375
        if ix < 0x3e20_0000 {
            // |x| < 2^-29
            return x;
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3ff3_0000 {
            // |x| < 1.1875
            if ix < 0x3fe6_0000 {
                id = 0;
                x = (2.0 * x - 1.0) / (2.0 + x);
            } else {
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x4003_8000 {
            // |x| < 2.4375
            id = 2;
            x = (x - 1.5) / (1.0 + 1.5 * x);
        } else {
            id = 3;
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    let s1 = z * (A_T[0] + w * (A_T[2] + w * (A_T[4] + w * (A_T[6] + w * (A_T[8] + w * A_T[10])))));
    let s2 = w * (A_T[1] + w * (A_T[3] + w * (A_T[5] + w * (A_T[7] + w * A_T[9]))));
    if id < 0 {
        return x - x * (s1 + s2);
    }
    let z = ATAN_HI[id as usize] - ((x * (s1 + s2) - ATAN_LO[id as usize]) - x);
    if hx < 0 { -z } else { z }
}

const TINY: f64 = 1.0e-300;
const PI_O_4: f64 = f64::from_bits(0x3FE9_21FB_5444_2D18);
const PI_O_2: f64 = f64::from_bits(0x3FF9_21FB_5444_2D18);
const PI: f64 = f64::from_bits(0x4009_21FB_5444_2D18);
const PI_LO: f64 = f64::from_bits(0x3CA1_A626_3314_5C07);

/// `Math.atan2(y, x)`.
pub fn atan2(y: f64, x: f64) -> f64 {
    let (hx, lx) = (hi(x), lo(x));
    let ix = hx & 0x7fff_ffff;
    let (hy, ly) = (hi(y), lo(y));
    let iy = hy & 0x7fff_ffff;
    if x.is_nan() || y.is_nan() {
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
        1 => f64::from_bits(z.to_bits() ^ 0x8000_0000_0000_0000),
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `KILN_TRIG_DUMP`: lines of `y x angle atan2 sin cos` as raw `long` bits printed by a JDK
    /// (`Math.atan2(y, x)`, `Math.sin(angle)`, `Math.cos(angle)`); skipped when unset.
    #[test]
    fn matches_a_java_dump() {
        let Some(path) = std::env::var_os("KILN_TRIG_DUMP") else { return };
        let text = std::fs::read_to_string(path).unwrap();
        let (mut n, mut bad_atan, mut bad_trig) = (0, 0, 0);
        for l in text.lines() {
            let v: Vec<f64> = l.split_whitespace().map(|t| f64::from_bits(t.parse::<i64>().unwrap() as u64)).collect();
            n += 1;
            bad_atan += (atan2(v[0], v[1]).to_bits() != v[3].to_bits()) as i32;
            bad_trig += (crate::trig::sin(v[2]).to_bits() != v[4].to_bits()) as i32 + (crate::trig::cos(v[2]).to_bits() != v[5].to_bits()) as i32;
        }
        assert_eq!(bad_atan, 0, "atan2 differs from the JDK in {bad_atan} of {n}");
        assert!(bad_trig * 100 < n, "sin and cos differ in {bad_trig} of {}", 2 * n);
    }

    #[test]
    fn special_values() {
        assert_eq!(atan2(0.0, -1.0), std::f64::consts::PI);
        assert_eq!(atan2(1.0, 0.0), std::f64::consts::FRAC_PI_2);
        assert!(atan2(f64::NAN, 1.0).is_nan());
        assert_eq!(atan2(-0.0, 1.0).to_bits(), (-0.0f64).to_bits());
    }
}
