//! `SpringFeature`: a fluid source in a wall with the configured numbers of rock and air
//! neighbours.

use super::{field, opt};
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{block, is_air};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use std::sync::Arc;

#[derive(Debug)]
pub struct Spring {
    /// `FluidState.createLegacyBlock()`.
    block: u16,
    fluid: &'static str,
    requires_block_below: bool,
    rock_count: i32,
    hole_count: i32,
    valid_blocks: Arc<BlockSet>,
}

/// `FluidState.CODEC` read as its legacy block (`FlowingFluid.getLegacyLevel`): sources are
/// level 0, flowing fluids `8 - amount`, plus 8 when falling.
fn fluid_block(json: &Json) -> Result<(u16, &'static str), Error> {
    let id = json
        .get("Name")
        .or_else(|| json.get("id"))
        .and_then(Json::as_str)
        .ok_or_else(|| Error::Invalid("fluid state without id".into()))?;
    let props = json.get("Properties").or_else(|| json.get("properties"));
    let prop = |k: &str| props.and_then(|p| p.get(k)).and_then(Json::as_str);
    let (fluid, block_name, source) = match crate::function::qualify(id).as_str() {
        "minecraft:water" => ("minecraft:water", "minecraft:water", true),
        "minecraft:lava" => ("minecraft:lava", "minecraft:lava", true),
        "minecraft:flowing_water" => ("minecraft:flowing_water", "minecraft:water", false),
        "minecraft:flowing_lava" => ("minecraft:flowing_lava", "minecraft:lava", false),
        other => return Err(Error::Invalid(format!("unsupported spring fluid {other}"))),
    };
    let level = if source {
        0
    } else {
        let amount: i32 = prop("level").and_then(|v| v.parse().ok()).unwrap_or(8);
        8 - amount.min(8) + if prop("falling") == Some("true") { 8 } else { 0 }
    };
    let b = block(block_name)?;
    let s = b.with_property(b.default, "level", &level.to_string()).ok_or_else(|| Error::Invalid(format!("bad fluid level {level}")))?;
    Ok((s, fluid))
}

impl Spring {
    pub fn parse(json: &Json, l: &Loader) -> Result<Spring, Error> {
        let (block, fluid) = fluid_block(field(json, "state")?)?;
        Ok(Spring {
            block,
            fluid,
            requires_block_below: opt(json, "requires_block_below", true, Json::as_bool)?,
            rock_count: opt(json, "rock_count", 4, Json::as_i32)?,
            hole_count: opt(json, "hole_count", 1, Json::as_i32)?,
            valid_blocks: l.blocks(field(json, "valid_blocks")?)?,
        })
    }

    pub fn place(&self, r: &mut Region, p: BlockPos) -> bool {
        let valid = |r: &mut Region, q: BlockPos| self.valid_blocks.contains(r.get(q));
        if !valid(r, p.above()) || (self.requires_block_below && !valid(r, p.below())) {
            return false;
        }
        let here = r.get(p);
        if !is_air(here) && !self.valid_blocks.contains(here) {
            return false;
        }
        let sides = [Dir::West, Dir::East, Dir::North, Dir::South, Dir::Down].map(|d| p.relative(d));
        let rocks = sides.iter().filter(|&&q| valid(r, q)).count() as i32;
        let holes = sides.iter().filter(|&&q| r.is_air(q)).count() as i32;
        if rocks == self.rock_count && holes == self.hole_count {
            r.set(p, self.block, 2);
            r.schedule_fluid_tick(p, self.fluid, 0);
            return true;
        }
        false
    }
}
