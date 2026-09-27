//! `SimpleBlockFeature`: one block from a state provider, if it can survive there.

use crate::Error;
use crate::block_facts::{fluid, is_instance};
use crate::blocks::{has_prop, is_air, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use kiln_data::block_props::replaceable;

#[derive(Debug)]
pub struct SimpleBlock {
    to_place: StateProvider,
    schedule_tick: bool,
}

impl SimpleBlock {
    pub fn parse(json: &Json, l: &Loader) -> Result<SimpleBlock, Error> {
        Ok(SimpleBlock {
            to_place: StateProvider::parse(json.get("to_place").ok_or_else(|| Error::Invalid("missing to_place".into()))?, l)?,
            schedule_tick: json.get("schedule_tick").and_then(Json::as_bool).unwrap_or(false),
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        let Some(s) = self.to_place.optional_state(r, random, p) else { return false };
        if !crate::survive::can_survive(s, r, p) {
            return false;
        }
        if is_instance(s, "DoublePlantBlock") {
            let above = r.get(p.above());
            if !(is_air(above) || (fluid(s) == fluid(above) && replaceable(above))) {
                return false;
            }
            place_double_plant(r, s, p, 2);
        } else if is_instance(s, "MossyCarpetBlock") {
            // MossyCarpetBlock.placeAt (pale moss carpets with side toppers) is not
            // implemented; the carpet alone is placed.
            r.set(p, s, 2);
        } else {
            r.set(p, s, 2);
        }
        if self.schedule_tick {
            let name = kiln_data::blocks_types::block_of(r.get(p)).name;
            r.schedule_block_tick(p, name, 1);
        }
        true
    }
}

/// `DoublePlantBlock.placeAt`: lower and upper half, each waterlogged if water is there.
pub fn place_double_plant(r: &mut Region, s: u16, p: BlockPos, flags: i32) {
    let lower = copy_waterlogged(r, p, with_prop(s, "half", "lower"));
    r.set(p, lower, flags);
    let upper = copy_waterlogged(r, p.above(), with_prop(s, "half", "upper"));
    r.set(p.above(), upper, flags);
}

/// `DoublePlantBlock.copyWaterloggedFrom`.
pub fn copy_waterlogged(r: &mut Region, p: BlockPos, s: u16) -> u16 {
    if has_prop(s, "waterlogged") {
        let water = r.fluid(p).is_water();
        with_prop(s, "waterlogged", if water { "true" } else { "false" })
    } else {
        s
    }
}
