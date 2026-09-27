//! `OreFeature` (a blob along a random segment) and `ScatteredOreFeature` (single blocks
//! around the origin).

use crate::Error;
use crate::blocks::{block_state, is_air};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::RuleTest;
use crate::proto::Heightmap;
use crate::providers::{float, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct Ore {
    /// `BlockReplacement`s: target test and the state replacing a match.
    targets: Vec<(RuleTest, u16)>,
    size: i32,
    discard_chance_on_air_exposure: f32,
}

/// `Mth.ceil(float)`.
fn ceil(f: f32) -> i32 {
    let i = f as i32;
    if f > i as f32 { i + 1 } else { i }
}

/// `Mth.floor(double)`.
fn floor(d: f64) -> i32 {
    let i = d as i32;
    if d < i as f64 { i - 1 } else { i }
}

/// `Math.round(float)`.
pub fn round(f: f32) -> i32 {
    if f.is_nan() { 0 } else { (f as f64 + 0.5).floor() as i32 }
}

impl Ore {
    pub fn parse(json: &Json, l: &Loader) -> Result<Ore, Error> {
        let targets = json
            .get("targets")
            .and_then(Json::as_array)
            .ok_or_else(|| Error::Invalid("ore without targets".into()))?
            .iter()
            .map(|t| {
                let target = RuleTest::parse(t.get("target").ok_or_else(|| Error::Invalid("target without rule".into()))?, l)?;
                Ok((target, block_state(t.get("state").ok_or_else(|| Error::Invalid("target without state".into()))?)?))
            })
            .collect::<Result<_, Error>>()?;
        Ok(Ore { targets, size: int(json, "size")?, discard_chance_on_air_exposure: float(json, "discard_chance_on_air_exposure")? })
    }

    /// `AbstractOreFeature.canPlaceOre`.
    fn can_place(&self, r: &mut Region, s: u16, target: &RuleTest, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        if !target.test(s, p, random) {
            return false;
        }
        let discard = self.discard_chance_on_air_exposure;
        let skip_air_check = if !(discard > 0.0) {
            true
        } else if discard >= 1.0 {
            false
        } else {
            random.next_float() >= discard
        };
        if skip_air_check {
            return true;
        }
        !crate::block_facts::Dir::ALL.iter().any(|d| is_air(r.get(p.relative(*d))))
    }

    /// `OreFeature.place`.
    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        let angle = random.next_float() * std::f32::consts::PI;
        let spread = self.size as f32 / 8.0;
        let pad = ceil((self.size as f32 / 16.0 * 2.0 + 1.0) / 2.0);
        let x0 = p.x as f64 + (angle as f64).sin() * spread as f64;
        let x1 = p.x as f64 - (angle as f64).sin() * spread as f64;
        let z0 = p.z as f64 + (angle as f64).cos() * spread as f64;
        let z1 = p.z as f64 - (angle as f64).cos() * spread as f64;
        let y0 = (p.y + random.next_int_bounded(3) - 2) as f64;
        let y1 = (p.y + random.next_int_bounded(3) - 2) as f64;
        let min_x = p.x - ceil(spread) - pad;
        let min_y = p.y - 2 - pad;
        let min_z = p.z - ceil(spread) - pad;
        let width = 2 * (ceil(spread) + pad);
        let height = 2 * (2 + pad);
        for x in min_x..=min_x + width {
            for z in min_z..=min_z + width {
                if min_y <= r.height_at(Heightmap::OceanFloorWg, x, z) {
                    return self.do_place(r, random, [x0, x1, z0, z1, y0, y1], [min_x, min_y, min_z], width, height);
                }
            }
        }
        false
    }

    fn do_place(&self, r: &mut Region, random: &mut WorldgenRandom, seg: [f64; 6], min: [i32; 3], width: i32, height: i32) -> bool {
        let [x0, x1, z0, z1, y0, y1] = seg;
        let [min_x, min_y, min_z] = min;
        let size = self.size as usize;
        let mut placed = 0;
        let mut done = vec![false; (width * height * width) as usize];
        let mut balls = vec![0f64; size * 4];
        let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
        for i in 0..size {
            let t = i as f32 / size as f32;
            let x = lerp(t as f64, x0, x1);
            let y = lerp(t as f64, y0, y1);
            let z = lerp(t as f64, z0, z1);
            let s = random.next_double() * size as f64 / 16.0;
            let radius = ((crate::carver::sin((std::f32::consts::PI * t) as f64) + 1.0) as f64 * s + 1.0) / 2.0;
            balls[i * 4] = x;
            balls[i * 4 + 1] = y;
            balls[i * 4 + 2] = z;
            balls[i * 4 + 3] = radius;
        }
        for i in 0..size.saturating_sub(1) {
            if !(balls[i * 4 + 3] > 0.0) {
                continue;
            }
            for j in i + 1..size {
                if !(balls[j * 4 + 3] > 0.0) {
                    continue;
                }
                let dx = balls[i * 4] - balls[j * 4];
                let dy = balls[i * 4 + 1] - balls[j * 4 + 1];
                let dz = balls[i * 4 + 2] - balls[j * 4 + 2];
                let dr = balls[i * 4 + 3] - balls[j * 4 + 3];
                if dr * dr > dx * dx + dy * dy + dz * dz {
                    if dr > 0.0 {
                        balls[j * 4 + 3] = -1.0;
                    } else {
                        balls[i * 4 + 3] = -1.0;
                    }
                }
            }
        }
        for i in 0..size {
            let radius = balls[i * 4 + 3];
            if radius < 0.0 {
                continue;
            }
            let (bx, by, bz) = (balls[i * 4], balls[i * 4 + 1], balls[i * 4 + 2]);
            let x_lo = floor(bx - radius).max(min_x);
            let y_lo = floor(by - radius).max(min_y);
            let z_lo = floor(bz - radius).max(min_z);
            let x_hi = floor(bx + radius).max(x_lo);
            let y_hi = floor(by + radius).max(y_lo);
            let z_hi = floor(bz + radius).max(z_lo);
            for x in x_lo..=x_hi {
                let nx = (x as f64 + 0.5 - bx) / radius;
                if !(nx * nx < 1.0) {
                    continue;
                }
                for y in y_lo..=y_hi {
                    let ny = (y as f64 + 0.5 - by) / radius;
                    if !(nx * nx + ny * ny < 1.0) {
                        continue;
                    }
                    for z in z_lo..=z_hi {
                        let nz = (z as f64 + 0.5 - bz) / radius;
                        if !(nx * nx + ny * ny + nz * nz < 1.0) {
                            continue;
                        }
                        if r.is_outside_build_height(y) {
                            continue;
                        }
                        let k = ((x - min_x) + (y - min_y) * width + (z - min_z) * width * height) as usize;
                        if done[k] {
                            continue;
                        }
                        done[k] = true;
                        let q = BlockPos::new(x, y, z);
                        if !r.can_write(q) {
                            continue;
                        }
                        let s = r.get(q);
                        for (target, state) in &self.targets {
                            if self.can_place(r, s, target, random, q) {
                                r.set_raw(q, *state);
                                placed += 1;
                                break;
                            }
                        }
                    }
                }
            }
        }
        placed > 0
    }

    /// `ScatteredOreFeature.place`.
    pub fn place_scattered(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        let n = random.next_int_bounded(self.size + 1);
        for i in 0..n {
            let range = i.min(7) as f32;
            let dx = round((random.next_float() - random.next_float()) * range);
            let dy = round((random.next_float() - random.next_float()) * range);
            let dz = round((random.next_float() - random.next_float()) * range);
            let q = p.offset(dx, dy, dz);
            let s = r.get(q);
            for (target, state) in &self.targets {
                if self.can_place(r, s, target, random, q) {
                    r.set(q, *state, 2);
                    break;
                }
            }
        }
        true
    }
}
