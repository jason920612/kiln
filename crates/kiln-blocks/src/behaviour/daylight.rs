//! Daylight detectors (`DaylightDetectorBlock`, `DaylightDetectorBlockEntity`): in a level with
//! sky light the detector works out its signal every 20 ticks from the sky light at the block, less
//! what the day, rain and thunder take (`getEffectiveSkyBrightness`), and the sun's angle:
//! shaped by the cosine of the angle, drawn a fifth of the way toward the horizon (noon or
//! midnight). An inverted detector gives `15 - brightness`. A click turns the detector over.

use crate::level::{Level, flags};
use crate::pos::BlockPos;
use crate::state;
use crate::update::set_block;

/// The overworld timeline's `minecraft:visual/sun_angle` at `day_time`, in degrees
/// (`KeyframeTrackSampler.sample`: a track of two keyframes at tick 6000, 360 and 0 degrees, over
/// the 24000 tick period, eased with the cubic bezier (0.362, 0.241, 0.638, 0.759)).
pub fn sun_angle(day_time: i64) -> f32 {
    let t = day_time.rem_euclid(24000);
    // The segment from the last keyframe (0 degrees at 6000 - 24000) to the first (360 at 6000)
    // before noon; from the last keyframe on to the first of the next period (360 at 30000) after.
    let (from_ticks, to_ticks) = if t < 6000 { (-18000i64, 6000i64) } else { (6000, 30000) };
    if t <= from_ticks {
        return 0.0;
    }
    let f = (t - from_ticks) as f32 / (to_ticks - from_ticks) as f32;
    let eased = bezier(f);
    // `Mth.lerp(float, float, float)`: 0 + eased * (360 - 0).
    0.0 + eased * (360.0 - 0.0)
}

/// `EasingType.CubicBezier(0.362, 0.241, 0.638, 0.759).apply`.
fn bezier(x: f32) -> f32 {
    let (x1, y1, x2, y2) = (0.362f32, 0.241f32, 0.638f32, 0.759f32);
    let curve = |a: f32, b: f32| -> (f32, f32, f32) { (3.0 * a - 3.0 * b + 1.0, -6.0 * a + 3.0 * b, 3.0 * a) };
    let sample = |c: (f32, f32, f32), t: f32| ((c.0 * t + c.1) * t + c.2) * t;
    let gradient = |c: (f32, f32, f32), t: f32| (3.0 * c.0 * t + 2.0 * c.1) * t + c.2;
    let xc = curve(x1, x2);
    let yc = curve(y1, y2);
    // `solveT`: four Newton-Raphson steps, then bisection.
    let mut t = x;
    for _ in 0..4 {
        let err = sample(xc, t) - x;
        if err.abs() < 1.0e-5 {
            return sample(yc, t);
        }
        let g = gradient(xc, t);
        if g < 1.0e-5 {
            break;
        }
        t -= (err / g).clamp(-0.25, 0.25);
    }
    let (mut lo, mut hi, mut mid) = (0.0f32, 1.0f32, t);
    while lo < hi {
        let err = sample(xc, mid) - x;
        if err.abs() < 1.0e-5 {
            return sample(yc, mid);
        }
        if err < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
        mid = (hi + lo) / 2.0;
    }
    sample(yc, mid)
}

/// `DaylightDetectorBlock.updateSignalStrength`.
pub fn update_signal<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    // `getEffectiveSkyBrightness`.
    let mut i = (level.sky_light(pos) - level.sky_darken()).max(0);
    let mut f = level.sun_angle() * 0.017453292f32;
    if state::get_bool(s, "inverted") {
        i = 15 - i;
    } else if i > 0 {
        let g = if f < std::f32::consts::PI { 0.0 } else { std::f32::consts::TAU };
        f += (g - f) * 0.2;
        // `Math.round(float)`.
        i = ((i as f32 * kiln_javamath::mth::cos(f as f64)) + 0.5).floor() as i32;
    }
    let i = i.clamp(0, 15);
    if state::get_int(s, "power") != i {
        crate::update::set_block_and_update(level, pos, state::set_int(s, "power", i));
    }
}

/// `DaylightDetectorBlock.useWithoutItem` for a player who may build: the detector is turned
/// over and works its signal out at once.
pub fn toggle<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let turned = state::set_bool(s, "inverted", !state::get_bool(s, "inverted"));
    set_block(level, pos, turned, flags::CLIENTS);
    update_signal(level, level.block(pos), pos);
}
