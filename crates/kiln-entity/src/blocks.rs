//! Per-state classification of the blocks entity behaviour special-cases, and block tags.

use kiln_data::blocks::{BLOCKS, STATE_COUNT};
use std::sync::OnceLock;

/// Blocks whose identity entity code checks (`state.is(Blocks.X)` / `instanceof`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Other,
    Air,
    Water,
    Lava,
    BubbleColumn,
    Scaffolding,
    PowderSnow,
    MovingPiston,
    Cobweb,
    SweetBerryBush,
    HoneyBlock,
    Fire,
    SoulFire,
    Campfire,
    Cactus,
    MagmaBlock,
    Ice,
    FenceGate,
    Hopper,
    LavaCauldron,
    WaterCauldron,
    PowderSnowCauldron,
    Farmland,
    Tnt,
    PointedDripstone,
    Bamboo,
    EndPortal,
    NetherPortal,
    WitherRose,
    Slime,
    Bed,
    Anvil,
    ConcretePowder,
}

fn classify(name: &str) -> Kind {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    match n {
        "air" | "cave_air" | "void_air" => Kind::Air,
        "water" => Kind::Water,
        "lava" => Kind::Lava,
        "bubble_column" => Kind::BubbleColumn,
        "scaffolding" => Kind::Scaffolding,
        "powder_snow" => Kind::PowderSnow,
        "moving_piston" => Kind::MovingPiston,
        "cobweb" => Kind::Cobweb,
        "sweet_berry_bush" => Kind::SweetBerryBush,
        "honey_block" => Kind::HoneyBlock,
        "fire" => Kind::Fire,
        "soul_fire" => Kind::SoulFire,
        "campfire" | "soul_campfire" => Kind::Campfire,
        "cactus" => Kind::Cactus,
        "magma_block" => Kind::MagmaBlock,
        "ice" | "frosted_ice" => Kind::Ice,
        "hopper" => Kind::Hopper,
        "lava_cauldron" => Kind::LavaCauldron,
        "water_cauldron" => Kind::WaterCauldron,
        "powder_snow_cauldron" => Kind::PowderSnowCauldron,
        "farmland" => Kind::Farmland,
        "tnt" => Kind::Tnt,
        "pointed_dripstone" => Kind::PointedDripstone,
        "bamboo" => Kind::Bamboo,
        "end_portal" => Kind::EndPortal,
        "nether_portal" => Kind::NetherPortal,
        "wither_rose" => Kind::WitherRose,
        "slime_block" => Kind::Slime,
        "anvil" | "chipped_anvil" | "damaged_anvil" => Kind::Anvil,
        _ if n.ends_with("_fence_gate") => Kind::FenceGate,
        _ if n.ends_with("_bed") => Kind::Bed,
        _ if n.ends_with("_concrete_powder") => Kind::ConcretePowder,
        _ => Kind::Other,
    }
}

pub fn kind(state: u16) -> Kind {
    static KINDS: OnceLock<Vec<Kind>> = OnceLock::new();
    KINDS.get_or_init(|| {
        let mut out = vec![Kind::Other; STATE_COUNT as usize];
        for b in BLOCKS {
            out[b.first as usize..=b.last as usize].fill(classify(b.name));
        }
        out
    })[state as usize]
}

/// The state's block name (`minecraft:stone`).
pub fn block_name(state: u16) -> &'static str {
    kiln_data::blocks_types::block_of(state).name
}

/// Block tags entity code consults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Tag {
    Fences = 1 << 0,
    Walls = 1 << 1,
    Climbable = 1 << 2,
    SuppressesBounce = 1 << 3,
    FallDamageResetting = 1 << 4,
    BlocksFluidFlow = 1 << 5,
    Fire = 1 << 6,
    Anvil = 1 << 7,
    Doors = 1 << 8,
    EdibleForSheep = 1 << 9,
    Cauldrons = 1 << 10,
    AnimalsSpawnableOn = 1 << 11,
    BlocksDolphinJump = 1 << 12,
    HappyGhastAvoids = 1 << 13,
    InsideStepSoundBlocks = 1 << 14,
    CombinationStepSoundBlocks = 1 << 15,
    CrystalSoundBlocks = 1 << 16,
    CamelSandStepSoundBlocks = 1 << 17,
}

const TAGS: &[(Tag, &str)] = &[
    (Tag::Fences, "minecraft:fences"),
    (Tag::Walls, "minecraft:walls"),
    (Tag::Climbable, "minecraft:climbable"),
    (Tag::SuppressesBounce, "minecraft:suppresses_bounce"),
    (Tag::FallDamageResetting, "minecraft:fall_damage_resetting"),
    (Tag::BlocksFluidFlow, "minecraft:blocks_fluid_flow"),
    (Tag::Fire, "minecraft:fire"),
    (Tag::Anvil, "minecraft:anvil"),
    (Tag::Doors, "minecraft:doors"),
    (Tag::EdibleForSheep, "minecraft:edible_for_sheep"),
    (Tag::Cauldrons, "minecraft:cauldrons"),
    (Tag::AnimalsSpawnableOn, "minecraft:animals_spawnable_on"),
    (Tag::BlocksDolphinJump, "minecraft:blocks_dolphin_jump"),
    (Tag::HappyGhastAvoids, "minecraft:happy_ghast_avoids"),
    (Tag::InsideStepSoundBlocks, "minecraft:inside_step_sound_blocks"),
    (Tag::CombinationStepSoundBlocks, "minecraft:combination_step_sound_blocks"),
    (Tag::CrystalSoundBlocks, "minecraft:crystal_sound_blocks"),
    (Tag::CamelSandStepSoundBlocks, "minecraft:camel_sand_step_sound_blocks"),
];

pub fn has_tag(state: u16, tag: Tag) -> bool {
    static FLAGS: OnceLock<Vec<u32>> = OnceLock::new();
    FLAGS.get_or_init(|| {
        let mut out = vec![0u32; STATE_COUNT as usize];
        let block_tags = kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:block")
            .map_or(&[][..], |(_, tags)| *tags);
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        for &(flag, tag_name) in TAGS {
            let ids = block_tags.iter().find(|(t, _)| *t == tag_name).map_or(&[][..], |(_, ids)| *ids);
            for &id in ids {
                if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                    for s in &mut out[info.first as usize..=info.last as usize] {
                        *s |= flag as u32;
                    }
                }
            }
        }
        out
    })[state as usize]
        & tag as u32
        != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn kinds_and_tags() {
        assert_eq!(kind(d::WATER), Kind::Water);
        assert_eq!(kind(d::OAK_FENCE_GATE), Kind::FenceGate);
        assert_eq!(kind(d::STONE), Kind::Other);
        assert!(has_tag(d::OAK_FENCE, Tag::Fences) && !has_tag(d::STONE, Tag::Fences));
        assert!(has_tag(d::COBBLESTONE_WALL, Tag::Walls));
        assert!(has_tag(d::LADDER, Tag::Climbable));
        assert!(has_tag(d::FIRE, Tag::Fire));
        assert!(has_tag(d::WATER, Tag::FallDamageResetting) || !has_tag(d::STONE, Tag::FallDamageResetting));
    }
}

/// `DiodeBlock.isDiode`: a repeater or a comparator.
pub fn is_diode(state: u16) -> bool {
    matches!(kiln_data::block_logic::block_class(state), kiln_data::block_logic::BlockClass::RepeaterBlock | kiln_data::block_logic::BlockClass::ComparatorBlock)
}
