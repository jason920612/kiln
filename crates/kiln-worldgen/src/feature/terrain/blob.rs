//! `BlockBlobFeature`: three overlapping small balls of one block, sunk onto a valid floor.

use super::{between_closed, field};
use crate::Error;
use crate::blocks::block_state;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use kiln_javamath::random::RandomSource;

#[derive(Debug)]
pub struct BlockBlob {
    state: u16,
    can_place_on: BlockPredicate,
}

impl BlockBlob {
    pub fn parse(json: &Json, l: &Loader) -> Result<BlockBlob, Error> {
        Ok(BlockBlob { state: block_state(field(json, "state")?)?, can_place_on: BlockPredicate::parse(field(json, "can_place_on")?, l)? })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut p = origin;
        while p.y > r.min_y() + 3 && !self.can_place_on.test(r, p.below()) {
            p = p.below();
        }
        if p.y <= r.min_y() + 3 {
            return false;
        }
        for _ in 0..3 {
            let a = random.next_int_bounded(2);
            let b = random.next_int_bounded(2);
            let c = random.next_int_bounded(2);
            let radius = (a + b + c) as f32 * 0.333 + 0.5;
            for q in between_closed(p.offset(-a, -b, -c), p.offset(a, b, c)) {
                if q.dist_sqr(p) <= (radius * radius) as f64 {
                    r.set_block(q, self.state);
                }
            }
            p = p.offset(-1 + random.next_int_bounded(2), -random.next_int_bounded(2), -1 + random.next_int_bounded(2));
        }
        true
    }
}
