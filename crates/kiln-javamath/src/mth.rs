//! `net.minecraft.util.Mth`'s table-driven trigonometry (26.3): `sin`, `cos` and `atan2`.
//!
//! The tables are filled the way `Mth`'s static initializer does it, with `Math.sin`, `Math.asin`
//! and `Math.cos` (here [`crate::trig`] and [`crate::strict`]); every use of "the sine table" in
//! the server goes through this one copy.

use std::sync::OnceLock;

const SCALE: f64 = 10430.378350470453;

/// `Mth.SIN`: `(float) Math.sin(i / 10430.378350470453)` for every 16-bit angle.
fn sin_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| (0..65536).map(|i| crate::trig::sin(i as f64 / SCALE) as f32).collect())
}

/// `Mth.sin(double)`.
#[inline]
pub fn sin(v: f64) -> f32 {
    sin_table()[((v * SCALE) as i64 & 0xffff) as usize]
}

/// `Mth.cos(double)`.
#[inline]
pub fn cos(v: f64) -> f32 {
    sin_table()[((v * SCALE + 16384.0) as i64 & 0xffff) as usize]
}

/// `Mth.ASIN_TAB` and `Mth.COS_TAB`: `asin(i / 256)` and its cosine.
fn atan_tables() -> &'static ([f64; 257], [f64; 257]) {
    static TABLES: OnceLock<([f64; 257], [f64; 257])> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut asin = [0.0; 257];
        let mut cos = [0.0; 257];
        for i in 0..257 {
            let a = crate::strict::asin(i as f64 / 256.0);
            asin[i] = a;
            cos[i] = crate::trig::cos(a);
        }
        (asin, cos)
    })
}

/// `Mth.fastInvSqrt(double)`.
pub fn fast_inv_sqrt(v: f64) -> f64 {
    let half = 0.5 * v;
    let x = f64::from_bits((6910469410427058090i64 - ((v.to_bits() as i64) >> 1)) as u64);
    x * (1.5 - half * x * x)
}

/// `Mth.atan2(double, double)`: the table-based arc tangent used for facing rotations.
pub fn atan2(mut y: f64, mut x: f64) -> f64 {
    let d = x * x + y * y;
    if d.is_nan() {
        return f64::NAN;
    }
    let neg_y = y < 0.0;
    if neg_y {
        y = -y;
    }
    let neg_x = x < 0.0;
    if neg_x {
        x = -x;
    }
    let swap = y > x;
    if swap {
        std::mem::swap(&mut x, &mut y);
    }
    let inv = fast_inv_sqrt(d);
    x *= inv;
    y *= inv;
    let frac_bias = f64::from_bits(4805340802404319232);
    let f = frac_bias + y;
    let i = f.to_bits() as i32 as usize;
    let (asin, cos) = atan_tables();
    let j = f - frac_bias;
    let k = y * cos[i] - x * j;
    let l = (6.0 + k * k) * k * 0.16666666666666666;
    let mut m = asin[i] + l;
    if swap {
        m = std::f64::consts::FRAC_PI_2 - m;
    }
    if neg_x {
        m = std::f64::consts::PI - m;
    }
    if neg_y {
        m = -m;
    }
    m
}
