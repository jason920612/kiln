//! `SpikeFeature` (ice spikes): a tapering spike, mirrored downwards, over a thin root column.

use super::{ceil, field};
use crate::Error;
use crate::blocks::{block_state, is_air};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct Spike {
    state: u16,
    can_place_on: BlockPredicate,
    can_replace: BlockPredicate,
}

impl Spike {
    pub fn parse(json: &Json, l: &Loader) -> Result<Spike, Error> {
        Ok(Spike {
            state: block_state(field(json, "state")?)?,
            can_place_on: BlockPredicate::parse(field(json, "can_place_on")?, l)?,
            can_replace: BlockPredicate::parse(field(json, "can_replace")?, l)?,
        })
    }

    fn try_set(&self, r: &mut Region, p: BlockPos) {
        if r.is_air(p) || self.can_replace.test(r, p) {
            r.set_block(p, self.state);
        }
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut p = origin;
        while r.is_air(p) && p.y > r.min_y() + 2 {
            p = p.below();
        }
        if !self.can_place_on.test(r, p) {
            return false;
        }
        p = p.above_n(random.next_int_bounded(4));
        let height = random.next_int_bounded(4) + 7;
        let width = height / 4 + random.next_int_bounded(2);
        if width > 1 && random.next_int_bounded(60) == 0 {
            p = p.above_n(10 + random.next_int_bounded(30));
        }
        for y in 0..height {
            let f = (1.0 - y as f32 / height as f32) * width as f32;
            let radius = ceil(f);
            for dx in -radius..=radius {
                let fx = dx.abs() as f32 - 0.25;
                for dz in -radius..=radius {
                    let fz = dz.abs() as f32 - 0.25;
                    if (dx != 0 || dz != 0) && fx * fx + fz * fz > f * f {
                        continue;
                    }
                    if (dx == -radius || dx == radius || dz == -radius || dz == radius) && random.next_float() > 0.75 {
                        continue;
                    }
                    self.try_set(r, p.offset(dx, y, dz));
                    if y != 0 && radius > 1 {
                        self.try_set(r, p.offset(dx, -y, dz));
                    }
                }
            }
        }
        let n = (width - 1).clamp(0, 1);
        for dx in -n..=n {
            for dz in -n..=n {
                let mut q = p.offset(dx, -1, dz);
                let mut run = if dx.abs() == 1 && dz.abs() == 1 { random.next_int_bounded(5) } else { 50 };
                while q.y > 50 {
                    let s = r.get(q);
                    if !(is_air(s) || self.can_replace.test(r, q) || s == self.state) {
                        break;
                    }
                    r.set_block(q, self.state);
                    q = q.below();
                    run -= 1;
                    if run <= 0 {
                        q = q.below_n(random.next_int_bounded(5) + 1);
                        run = random.next_int_bounded(5);
                    }
                }
            }
        }
        true
    }
}
