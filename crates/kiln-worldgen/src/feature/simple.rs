//! `SimpleBlockFeature`: one block from a state provider, if it can survive there.

use crate::Error;
use crate::block_facts::{fluid, is_instance};
use crate::blocks::{has_prop, is_air, is_block, prop, with_prop};
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
            place_mossy_carpet(r, p, 2);
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

/// `MossyCarpetBlock.placeAt` with the level's random (`WorldGenLevel.getRandom`, as
/// `SimpleBlockFeature` passes): a pale moss carpet with wall sides where it can attach, and
/// sometimes a side-only carpet above it.
pub fn place_mossy_carpet(r: &mut Region, p: BlockPos, flags: i32) {
    let carpet = updated_carpet(r, crate::blocks::state::PALE_MOSS_CARPET, p, true);
    r.set(p, carpet, flags);
    let topper = carpet_topper(r, p);
    if !is_air(topper) {
        r.set(p.above(), topper, flags);
        let carpet = updated_carpet(r, carpet, p, true);
        r.set(p, carpet, flags);
    }
}

/// `MossyCarpetBlock.getUpdatedState`.
fn updated_carpet(r: &mut Region, mut s: u16, p: BlockPos, tall_sides: bool) -> u16 {
    use crate::block_facts::Dir;
    let base = prop(s, "bottom") == Some("true");
    let low_sides = tall_sides || base;
    let mut above = None;
    let mut below = None;
    for d in Dir::HORIZONTAL {
        let supported = crate::feature::vegetation::shape::can_attach_to(r.get(p.relative(d)), d);
        let mut side = if supported {
            if low_sides { "low" } else { prop(s, d.name()).unwrap_or("none") }
        } else {
            "none"
        };
        if side == "low" {
            let a = *above.get_or_insert_with(|| r.get(p.above()));
            if is_block(a, "minecraft:pale_moss_carpet") && prop(a, d.name()) != Some("none") && prop(a, "bottom") != Some("true") {
                side = "tall";
            }
            if !base {
                let b = *below.get_or_insert_with(|| r.get(p.below()));
                if is_block(b, "minecraft:pale_moss_carpet") && prop(b, d.name()) == Some("none") {
                    side = "none";
                }
            }
        }
        s = with_prop(s, d.name(), side);
    }
    s
}

/// `MossyCarpetBlock.createTopperWithSideChance`.
fn carpet_topper(r: &mut Region, p: BlockPos) -> u16 {
    use crate::block_facts::Dir;
    use kiln_javamath::random::RandomSource;
    let above = r.get(p.above());
    let is_carpet = is_block(above, "minecraft:pale_moss_carpet");
    if (is_carpet && prop(above, "bottom") == Some("true")) || (!is_carpet && !replaceable(above)) {
        return crate::blocks::state::AIR;
    }
    let top = with_prop(crate::blocks::state::PALE_MOSS_CARPET, "bottom", "false");
    let mut top = updated_carpet(r, top, p.above(), true);
    for d in Dir::HORIZONTAL {
        if prop(top, d.name()) != Some("none") && !r.level_random().next_bool() {
            top = with_prop(top, d.name(), "none");
        }
    }
    let has_faces = prop(top, "bottom") == Some("true") || Dir::HORIZONTAL.iter().any(|d| prop(top, d.name()) != Some("none"));
    if has_faces && top != above { top } else { crate::blocks::state::AIR }
}
