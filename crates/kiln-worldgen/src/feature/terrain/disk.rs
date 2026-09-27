//! `DiskFeature`: replaces matching blocks in a vertical cylinder.

use super::{field, mark_above_for_post_processing};
use crate::Error;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::BlockPredicate;
use crate::providers::{IntProvider, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;

#[derive(Debug)]
pub struct Disk {
    state_provider: StateProvider,
    target: BlockPredicate,
    radius: IntProvider,
    half_height: i32,
}

impl Disk {
    pub fn parse(json: &Json, l: &Loader) -> Result<Disk, Error> {
        Ok(Disk {
            state_provider: StateProvider::parse(field(json, "state_provider")?, l)?,
            target: BlockPredicate::parse(field(json, "target")?, l)?,
            radius: IntProvider::parse(field(json, "radius")?)?,
            half_height: int(json, "half_height")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let top = origin.y + self.half_height;
        let bottom = origin.y - self.half_height - 1;
        let radius = self.radius.sample(random);
        let mut placed = false;
        for z in -radius..=radius {
            for x in -radius..=radius {
                if x * x + z * z > radius * radius {
                    continue;
                }
                placed |= self.place_column(r, random, top, bottom, origin.offset(x, 0, z));
            }
        }
        placed
    }

    fn place_column(&self, r: &mut Region, random: &mut WorldgenRandom, top: i32, bottom: i32, column: BlockPos) -> bool {
        let mut placed = false;
        let mut previous = false;
        for y in (bottom + 1..=top).rev() {
            let p = column.at_y(y);
            if !self.target.test(r, p) {
                previous = false;
                continue;
            }
            if let Some(s) = self.state_provider.optional_state(r, random, p) {
                r.set(p, s, 2);
                if !previous {
                    mark_above_for_post_processing(r, p);
                }
                placed = true;
                previous = true;
            }
        }
        placed
    }
}
