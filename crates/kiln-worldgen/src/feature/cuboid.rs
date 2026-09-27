//! `CuboidPlacement`: the positions of a box (faces only, or with edges or interior) whose
//! corner is the origin.

use crate::Error;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::IntProvider;
use crate::random::WorldgenRandom;

#[derive(Clone, Debug)]
pub struct Cuboid {
    xz_size: IntProvider,
    y_size: IntProvider,
    include_edges: bool,
    include_interior: bool,
}

impl Cuboid {
    pub fn parse(json: &Json) -> Result<Cuboid, Error> {
        let get = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("cuboid without {k}")));
        Ok(Cuboid {
            xz_size: IntProvider::parse(get("xz_size")?)?,
            y_size: IntProvider::parse(get("y_size")?)?,
            include_edges: json.get("include_edges").and_then(Json::as_bool).unwrap_or(false),
            include_interior: json.get("include_interior").and_then(Json::as_bool).unwrap_or(false),
        })
    }

    pub fn modify(&self, random: &mut WorldgenRandom, p: BlockPos, out: &mut Vec<BlockPos>) {
        let sy = self.y_size.sample(random);
        let sx = self.xz_size.sample(random);
        let sz = self.xz_size.sample(random);
        let edge = |v: i32, max: i32| v == 0 || v == max;
        for x in 0..=sx {
            for y in 0..=sy {
                for z in 0..=sz {
                    let (ex, ey, ez) = (edge(x, sx), edge(y, sy), edge(z, sz));
                    if !self.include_edges && ((ex && ey) || (ez && ey) || (ex && ez)) {
                        continue;
                    }
                    if !self.include_interior && !ex && !ey && !ez {
                        continue;
                    }
                    out.push(p.offset(x, y, z));
                }
            }
        }
    }
}
