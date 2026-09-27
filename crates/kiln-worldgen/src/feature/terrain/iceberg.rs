//! `IcebergFeature`: a round or elliptic iceberg at sea level with an underwater part,
//! smoothed, optionally snow-capped and with a cut-out.

use super::ceil;
use crate::block_facts::Dir;
use crate::blocks::{is_air, is_block, is_water, state};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_javamath::math::clamp;
use kiln_javamath::trig::{cos, sin};
use kiln_javamath::random::RandomSource;
use std::f64::consts::PI;

/// The per-iceberg shape parameters.
struct Shape {
    ellipse: bool,
    /// `getEllipseC` base (3..=5).
    ellipse_c: i32,
    angle: f64,
    snow_on_top: bool,
    state: u16,
}

/// `IcebergFeature.place`.
pub fn place(ice: u16, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    let origin = BlockPos::new(origin.x, r.sea_level(), origin.z);
    let snow_on_top = random.next_double() > 0.7;
    let angle = random.next_double() * 2.0 * PI;
    let shape_a = 11 - random.next_int_bounded(5);
    let ellipse_c = 3 + random.next_int_bounded(3);
    let ellipse = random.next_double() > 0.7;
    let mut height = if ellipse { random.next_int_bounded(6) + 6 } else { random.next_int_bounded(15) + 3 };
    if !ellipse && random.next_double() > 0.9 {
        height += random.next_int_bounded(19) + 7;
    }
    let depth = (height + random.next_int_bounded(11)).min(18);
    let width = (height + random.next_int_bounded(7) - random.next_int_bounded(5)).min(11);
    let extent = if ellipse { shape_a } else { 11 };
    let shape = Shape { ellipse, ellipse_c, angle, snow_on_top, state: ice };
    for x in -extent..extent {
        for z in -extent..extent {
            for y in 0..height {
                let radius = if ellipse { radius_ellipse(y, height, width) } else { radius_round(random, y, height, width) };
                if ellipse || x < radius {
                    generate_block(r, random, origin, &shape, height, (x, y, z), radius, extent);
                }
            }
        }
    }
    smooth(r, origin, width, height, ellipse, shape_a);
    for x in -extent..extent {
        for z in -extent..extent {
            let mut y = -1;
            while y > -depth {
                let outer = if ellipse {
                    ceil(extent as f32 * (1.0 - (y * y) as f32 / (depth as f32 * 8.0)))
                } else {
                    extent
                };
                let radius = radius_steep(random, -y, depth, width);
                if x < radius {
                    generate_block(r, random, origin, &shape, depth, (x, y, z), radius, outer);
                }
                y -= 1;
            }
        }
    }
    let cut = if ellipse { random.next_double() > 0.1 } else { random.next_double() > 0.7 };
    if cut {
        generate_cut_out(r, random, width, height, origin, &shape, shape_a);
    }
    true
}

fn generate_cut_out(r: &mut Region, random: &mut WorldgenRandom, width: i32, height: i32, origin: BlockPos, shape: &Shape, shape_a: i32) {
    let sx = if random.next_bool() { -1 } else { 1 };
    let sz = if random.next_bool() { -1 } else { 1 };
    let mut a = random.next_int_bounded((width / 2 - 2).max(1));
    if random.next_bool() {
        a = width / 2 + 1 - random.next_int_bounded((width - width / 2 - 1).max(1));
    }
    let mut b = random.next_int_bounded((width / 2 - 2).max(1));
    if random.next_bool() {
        b = width / 2 + 1 - random.next_int_bounded((width - width / 2 - 1).max(1));
    }
    if shape.ellipse {
        a = random.next_int_bounded((shape_a - 5).max(1));
        b = a;
    }
    let center = BlockPos::new(sx * a, 0, sz * b);
    let angle = if shape.ellipse { shape.angle + PI / 2.0 } else { random.next_double() * 2.0 * PI };
    for y in 0..height - 3 {
        let radius = radius_round(random, y, height, width);
        carve(r, radius, y, origin, false, angle, center, shape_a, shape.ellipse_c);
    }
    let mut y = -1;
    while y > -height + random.next_int_bounded(5) {
        let radius = radius_steep(random, -y, height, width);
        carve(r, radius, y, origin, true, angle, center, shape_a, shape.ellipse_c);
        y -= 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn carve(r: &mut Region, radius: i32, y: i32, origin: BlockPos, underwater: bool, angle: f64, center: BlockPos, shape_a: i32, ellipse_c: i32) {
    let a = radius + 1 + shape_a / 3;
    let c = (radius - 3).min(3) + ellipse_c / 2 - 1;
    for x in -a..a {
        for z in -a..a {
            // NaN (a zero semi-axis) counts as outside, like the bytecode's `dcmpg`.
            if !(signed_distance_ellipse(x, z, center, a, c, angle) < 0.0) {
                continue;
            }
            let p = origin.offset(x, y, z);
            let s = r.get(p);
            if is_iceberg_state(s) || is_block(s, "minecraft:snow_block") {
                if underwater {
                    r.set_block(p, state::WATER);
                } else {
                    r.set_block(p, state::AIR);
                    if is_block(r.get(p.above()), "minecraft:snow") {
                        r.set_block(p.above(), state::AIR);
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_block(r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos, shape: &Shape, height: i32, (x, y, z): (i32, i32, i32), radius: i32, outer: i32) {
    let dist = if shape.ellipse {
        signed_distance_ellipse(x, z, BlockPos::default(), outer, ellipse_c(y, height, shape.ellipse_c), shape.angle)
    } else {
        signed_distance_circle(x, z, BlockPos::default(), radius, random)
    };
    if !(dist < 0.0) {
        return;
    }
    let p = origin.offset(x, y, z);
    let threshold = if shape.ellipse { -0.5 } else { (-6 - random.next_int_bounded(3)) as f64 };
    if dist > threshold && random.next_double() > 0.9 {
        return;
    }
    set_iceberg_block(r, random, p, height - y, height, shape);
}

fn set_iceberg_block(r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, height_diff: i32, height: i32, shape: &Shape) {
    let s = r.get(p);
    let water = is_water(s);
    if !(is_air(s) || is_block(s, "minecraft:snow_block") || is_block(s, "minecraft:ice") || water) {
        return;
    }
    let chance = !shape.ellipse || random.next_double() > 0.05;
    let divisor = if shape.ellipse { 3 } else { 2 };
    if shape.snow_on_top
        && !water
        && height_diff as f64 <= random.next_int_bounded((height / divisor).max(1)) as f64 + height as f64 * 0.6
        && chance
    {
        r.set_block(p, state::SNOW_BLOCK);
    } else {
        r.set_block(p, shape.state);
    }
}

fn ellipse_c(y: i32, height: i32, c: i32) -> i32 {
    if y > 0 && height - y <= 3 { c - (4 - (height - y)) } else { c }
}

fn signed_distance_circle(x: i32, z: i32, center: BlockPos, radius: i32, random: &mut WorldgenRandom) -> f64 {
    let f = 10.0 * clamp(random.next_float(), 0.2, 0.8) / radius as f32;
    let (dx, dz, r) = ((x - center.x) as f64, (z - center.z) as f64, radius as f64);
    f as f64 + dx * dx + dz * dz - r * r
}

fn signed_distance_ellipse(x: i32, z: i32, center: BlockPos, a: i32, c: i32, angle: f64) -> f64 {
    let (dx, dz) = ((x - center.x) as f64, (z - center.z) as f64);
    let u = (dx * cos(angle) - dz * sin(angle)) / a as f64;
    let v = (dx * sin(angle) + dz * cos(angle)) / c as f64;
    u * u + v * v - 1.0
}

fn radius_round(random: &mut WorldgenRandom, y: i32, height: i32, width: i32) -> i32 {
    let k = 3.5 - random.next_float();
    let mut f = (1.0 - (y * y) as f32 / (height as f32 * k)) * width as f32;
    if height > 15 + random.next_int_bounded(5) {
        let yy = if y < 3 + random.next_int_bounded(6) { y / 2 } else { y };
        f = (1.0 - yy as f32 / (height as f32 * k * 0.4)) * width as f32;
    }
    ceil(f / 2.0)
}

fn radius_ellipse(y: i32, height: i32, width: i32) -> i32 {
    let f = (1.0 - (y * y) as f32 / (height as f32 * 1.0)) * width as f32;
    ceil(f / 2.0)
}

fn radius_steep(random: &mut WorldgenRandom, y: i32, height: i32, width: i32) -> i32 {
    let k = 1.0 + random.next_float() / 2.0;
    let f = (1.0 - y as f32 / (height as f32 * k)) * width as f32;
    ceil(f / 2.0)
}

fn is_iceberg_state(s: u16) -> bool {
    is_block(s, "minecraft:packed_ice") || is_block(s, "minecraft:snow_block") || is_block(s, "minecraft:blue_ice")
}

fn smooth(r: &mut Region, origin: BlockPos, width: i32, height: i32, ellipse: bool, shape_a: i32) {
    let n = if ellipse { shape_a } else { width / 2 };
    for x in -n..=n {
        for z in -n..=n {
            for y in 0..=height {
                let p = origin.offset(x, y, z);
                let s = r.get(p);
                if !is_iceberg_state(s) && !is_block(s, "minecraft:snow") {
                    continue;
                }
                if r.is_air(p.below()) {
                    r.set_block(p, state::AIR);
                    r.set_block(p.above(), state::AIR);
                } else if is_iceberg_state(s) {
                    let open = [Dir::West, Dir::East, Dir::North, Dir::South]
                        .into_iter()
                        .filter(|&d| !is_iceberg_state(r.get(p.relative(d))))
                        .count();
                    if open >= 3 {
                        r.set_block(p, state::AIR);
                    }
                }
            }
        }
    }
}
