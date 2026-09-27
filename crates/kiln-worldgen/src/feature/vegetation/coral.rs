//! `CoralTreeFeature` and `CoralClawFeature`: branching shapes built from repeated placements
//! of a placed feature (a coral block with its decorations).

use super::multiface::shuffle;
use super::placed;
use crate::Error;
use crate::block_facts::Dir;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct Coral {
    feature: usize,
    claw: bool,
}

impl Coral {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader, claw: bool) -> Result<Self, Error> {
        Ok(Self { feature: placed(json, "feature", f, l)?, claw })
    }

    pub fn nested(&self) -> Vec<usize> {
        vec![self.feature]
    }

    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if self.claw { self.place_claw(f, r, random, origin) } else { self.place_tree(f, r, random, origin) }
    }

    fn block(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        f.place_placed(self.feature, r, random, p, false)
    }

    /// `CoralTreeFeature.place`: a trunk with two to four branches.
    fn place_tree(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut p = origin;
        let trunk = random.next_int_bounded(3) + 1;
        for _ in 0..trunk {
            if !self.block(f, r, random, p) {
                return true;
            }
            p = p.above();
        }
        let top = p;
        let branches = random.next_int_bounded(3) + 2;
        let mut dirs = Dir::HORIZONTAL;
        shuffle(&mut dirs, random);
        for &d in &dirs[..branches as usize] {
            let mut p = top.relative(d);
            let length = random.next_int_bounded(5) + 2;
            let mut straight = 0;
            for j in 0..length {
                if !self.block(f, r, random, p) {
                    break;
                }
                straight += 1;
                p = p.above();
                if j == 0 || (straight >= 2 && random.next_float() < 0.25) {
                    p = p.relative(d);
                    straight = 0;
                }
            }
        }
        true
    }

    /// `CoralClawFeature.place`: fingers reaching out and up from a base.
    fn place_claw(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !self.block(f, r, random, origin) {
            return false;
        }
        let main = Dir::HORIZONTAL[random.next_int_bounded(4) as usize];
        let fingers = random.next_int_bounded(2) + 2;
        let mut dirs = [main, main.clockwise(), main.counter_clockwise()];
        shuffle(&mut dirs, random);
        for &d in &dirs[..fingers as usize] {
            let mut p = origin;
            let base = random.next_int_bounded(2) + 1;
            p = p.relative(d);
            let (step, reach) = if d == main {
                (main, random.next_int_bounded(3) + 2)
            } else {
                p = p.above();
                let step = [d, Dir::Up][random.next_int_bounded(2) as usize];
                (step, random.next_int_bounded(3) + 3)
            };
            for _ in 0..base {
                if !self.block(f, r, random, p) {
                    break;
                }
                p = p.relative(step);
            }
            p = p.relative(step.opposite()).above();
            for _ in 0..reach {
                p = p.relative(main);
                if !self.block(f, r, random, p) {
                    break;
                }
                if random.next_float() < 0.25 {
                    p = p.above();
                }
            }
        }
        true
    }
}
