//! `BlockState.canSurvive`: whether a block can stay where it is, per block class (vanilla
//! dispatches on the block's Java class; the chain comes from [`crate::block_facts`]).
//!
//! Light is 0 everywhere during generation (chunks are lit after FEATURES), which is what
//! vanilla sees in an unlit proto-chunk.

use crate::block_facts::{Dir, Support, class_chain, collision_top_full, fluid, is_face_sturdy, is_solid};
use crate::blocks::{is_block, prop, same_block};
use crate::pos::BlockPos;
use crate::region::Region;
use crate::vtags;
use kiln_data::block_props::solid_render;

/// `BlockState.canSurvive(level, pos)`.
pub fn can_survive(state: u16, r: &mut Region, p: BlockPos) -> bool {
    for class in class_chain(state).split('<') {
        if let Some(v) = class_can_survive(class, state, r, p) {
            return v;
        }
    }
    true
}

/// The `canSurvive` override of one class, if it has one.
fn class_can_survive(class: &str, state: u16, r: &mut Region, p: BlockPos) -> Option<bool> {
    Some(match class {
        "VegetationBlock" => {
            let below = r.get(p.below());
            may_place_on(state, below, r, p.below())
        }
        "DoublePlantBlock" => {
            if prop(state, "half") == Some("upper") {
                let below = r.get(p.below());
                same_block(below, state) && prop(below, "half") == Some("lower")
            } else {
                let below = r.get(p.below());
                may_place_on(state, below, r, p.below())
            }
        }
        "SmallDripleafBlock" if prop(state, "half") != Some("upper") => {
            let below = r.get(p.below());
            may_place_on(state, below, r, p.below())
        }
        "TallSeagrassBlock" => {
            if prop(state, "half") == Some("upper") {
                let below = r.get(p.below());
                same_block(below, state) && prop(below, "half") == Some("lower")
            } else {
                let below = r.get(p.below());
                let f = r.fluid(p);
                may_place_on(state, below, r, p.below()) && f.is_water() && f.amount == 8
            }
        }
        "MushroomBlock" => {
            let below = r.get(p.below());
            if vtags::is(below, "overrides_mushroom_light_requirement") {
                return Some(true);
            }
            // getRawBrightness is 0 in an unlit proto-chunk.
            may_place_on(state, below, r, p.below())
        }
        "CactusBlock" => {
            for d in Dir::HORIZONTAL {
                let n = r.get(p.relative(d));
                if is_solid(n) || fluid(n).is_lava() {
                    return Some(false);
                }
            }
            let below = r.get(p.below());
            (same_block(below, state) || vtags::is(below, "supports_cactus")) && !is_liquid(r.get(p.above()))
        }
        "SugarCaneBlock" => {
            let below = r.get(p.below());
            if same_block(below, state) {
                return Some(true);
            }
            if vtags::is(below, "supports_sugar_cane") {
                for d in Dir::HORIZONTAL {
                    let n = r.get(p.below().relative(d));
                    if vtags::fluid_is(fluid(n), "supports_sugar_cane_adjacently") || vtags::is(n, "supports_sugar_cane_adjacently") {
                        return Some(true);
                    }
                }
            }
            false
        }
        "SnowLayerBlock" => {
            let below = r.get(p.below());
            if vtags::is(below, "cannot_support_snow_layer") {
                return Some(false);
            }
            if vtags::is(below, "support_override_snow_layer") {
                return Some(true);
            }
            collision_top_full(below) || (same_block(below, state) && prop(below, "layers") == Some("8"))
        }
        "CocoaBlock" => {
            let facing = prop(state, "facing").and_then(Dir::by_name).unwrap_or(Dir::North);
            vtags::is(r.get(p.relative(facing)), "supports_cocoa")
        }
        "BigDripleafBlock" => {
            let below = r.get(p.below());
            same_block(below, state) || is_block(below, "minecraft:big_dripleaf_stem") || vtags::is(below, "supports_big_dripleaf")
        }
        "BigDripleafStemBlock" => {
            let below = r.get(p.below());
            let above = r.get(p.above());
            (same_block(below, state) || vtags::is(below, "supports_big_dripleaf"))
                && (same_block(above, state) || is_block(above, "minecraft:big_dripleaf"))
        }
        "GrowingPlantBlock" => {
            let name = kiln_data::blocks_types::block_of(state).name;
            let head = name.strip_suffix("_plant").unwrap_or(name);
            let grows = if matches!(head, "minecraft:kelp" | "minecraft:twisting_vines") { Dir::Up } else { Dir::Down };
            let q = p.relative(grows.opposite());
            let support = r.get(q);
            if head == "minecraft:kelp" && vtags::is(support, "cannot_support_kelp") {
                return Some(false);
            }
            let support_name = kiln_data::blocks_types::block_of(support).name;
            support_name == head
                || support_name.strip_suffix("_plant") == Some(head)
                || is_face_sturdy(support, grows, Support::Full)
        }
        "LeafLitterBlock" => is_face_sturdy(r.get(p.below()), Dir::Up, Support::Full),
        "MangrovePropaguleBlock" if prop(state, "hanging") == Some("true") => {
            vtags::is(r.get(p.above()), "supports_hanging_mangrove_propagule")
        }
        "SporeBlossomBlock" => is_face_sturdy(r.get(p.above()), Dir::Down, Support::Center) && !r.fluid(p).is_water(),
        "CarpetBlock" => !crate::blocks::is_air(r.get(p.below())),
        "MossyCarpetBlock" => {
            let below = r.get(p.below());
            if prop(state, "base") == Some("true") {
                !crate::blocks::is_air(below)
            } else {
                same_block(below, state) && prop(below, "base") == Some("true")
            }
        }
        "HangingRootsBlock" => is_face_sturdy(r.get(p.above()), Dir::Down, Support::Full),
        "BambooStalkBlock" | "BambooSaplingBlock" => vtags::is(r.get(p.below()), "supports_bamboo"),
        "BaseCoralPlantTypeBlock" => is_face_sturdy(r.get(p.below()), Dir::Up, Support::Full),
        "BaseCoralWallFanBlock" => {
            let facing = prop(state, "facing").and_then(Dir::by_name).unwrap_or(Dir::North);
            is_face_sturdy(r.get(p.relative(facing.opposite())), facing, Support::Full)
        }
        _ => return None,
    })
}

/// `VegetationBlock.mayPlaceOn` and its overrides.
fn may_place_on(state: u16, below: u16, r: &mut Region, below_pos: BlockPos) -> bool {
    for class in class_chain(state).split('<') {
        let v = match class {
            "VegetationBlock" => vtags::is(below, "supports_vegetation"),
            "MushroomBlock" => solid_render(below),
            "SmallDripleafBlock" => {
                vtags::is(below, "supports_small_dripleaf")
                    || (fluid(r.get(below_pos.above())).is_water_source() && vtags::is(below, "supports_vegetation"))
            }
            "MangrovePropaguleBlock" => vtags::is(below, "supports_mangrove_propagule"),
            "NetherFungusBlock" | "NetherRootsBlock" => {
                let name = kiln_data::blocks_types::block_of(state).name.trim_start_matches("minecraft:");
                vtags::is(below, &format!("supports_{name}"))
            }
            "NetherSproutsBlock" => vtags::is(below, "supports_nether_sprouts"),
            "WitherRoseBlock" => vtags::is(below, "supports_wither_rose"),
            "DryVegetationBlock" => vtags::is(below, "supports_dry_vegetation"),
            "AzaleaBlock" => vtags::is(below, "supports_azalea"),
            "CactusFlowerBlock" => {
                vtags::is(below, "support_override_cactus_flower") || is_face_sturdy(below, Dir::Up, Support::Center)
            }
            "SeagrassBlock" | "TallSeagrassBlock" => {
                is_face_sturdy(below, Dir::Up, Support::Full) && !vtags::is(below, "cannot_support_seagrass")
            }
            "SeaPickleBlock" => !collision_top_empty(below) || is_face_sturdy(below, Dir::Up, Support::Full),
            "LilyPadBlock" => {
                let above = fluid(r.get(below_pos.above()));
                (vtags::fluid_is(fluid(below), "supports_lily_pad") || vtags::is(below, "supports_lily_pad")) && above.is_empty()
            }
            _ => continue,
        };
        return v;
    }
    true
}

/// `BlockState.liquid()` (the block is a liquid block: water or lava).
fn is_liquid(state: u16) -> bool {
    matches!(kiln_data::blocks_types::block_of(state).name, "minecraft:water" | "minecraft:lava")
}

/// Whether the top face of the collision shape is empty (`getFaceShape(UP).isEmpty()`: the
/// slice just below y = 1).
fn collision_top_empty(state: u16) -> bool {
    const Y: f32 = 0.9999999;
    !kiln_data::block_props::collision(state).iter().any(|b| b[1] <= Y && b[4] > Y)
}
