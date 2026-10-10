//! The eye of ender in players' hands (`EnderEyeItem`).
//!
//! Used on an empty end portal frame (`useOn`), the eye goes in: the level part (the frame shows
//! the eye, comparators read 15, a complete ring opens its portal) is
//! [`kiln_blocks::behaviour::end_portal`]; here the item is used up and counted. Used in the
//! air (`use`), it asks the level for the nearest `#eye_of_ender_located` structure (a
//! stronghold) within 100 chunks and flies toward it ([`EyeOfEnder`]); with nothing found
//! nothing happens.
//!
//! [`EyeOfEnder`]: kiln_entity::ext_entity::eye_of_ender::EyeOfEnder

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use crate::player_stats::{self, Stat};
use kiln_blocks::behaviour::end_portal;
use kiln_blocks::{BlockPos, Level};
use kiln_entity::math::Vec3;
use kiln_inventory::Container;
use kiln_javamath::random::RandomSource;

pub(crate) const ITEM: &str = "minecraft:ender_eye";

fn consume(p: &mut Player, off_hand: bool) {
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        p.inv.item_mut(i).shrink(1);
        p.inv.times_changed += 1;
    }
}

/// `EnderEyeItem.useOn`: `false` (pass) unless the clicked block is an empty frame.
pub(crate) fn use_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, off_hand: bool) -> bool {
    let used = p.in_hand(off_hand).clone();
    // (`Block.pushEntitiesUp` is not simulated: an entity standing on top of the frame is not
    // lifted by the eye's extra height.)
    if !end_portal::insert_eye(level, pos) {
        return false;
    }
    consume(p, off_hand);
    p.award_stat(Stat::item(player_stats::USED, used.item()), 1);
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    p.used_on_block("minecraft:item_used_on_block", [pos.x, pos.y, pos.z], level.block(pos), &used, &probe);
    true
}

/// The structures `StructureTags.EYE_OF_ENDER_LOCATED` lists.
fn located_structures() -> Vec<String> {
    // (Vanilla's tag lists the stronghold alone; used when the tag file cannot be read.)
    crate::world_state::worldgen_tag("worldgen/structure", "minecraft:eye_of_ender_located").unwrap_or_else(|| vec!["minecraft:stronghold".to_owned()])
}

/// `EnderEyeItem.use`.
pub(crate) fn use_item(p: &mut Player, level: &mut RegionLevel, off_hand: bool, spawns: &mut Vec<Spawn>) {
    // Aimed at a frame, the item passes (the click on the block places the eye).
    if let Some(hit) = crate::use_item::pov_hit(p, &|pos| level.block(pos), crate::use_item::FluidMode::None)
        && end_portal::is_frame(level.block(hit.pos))
    {
        return;
    }
    let Some(pipeline) = level.env.pipeline.clone() else { return };
    let at = [p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32];
    let mut gs = kiln_worldgen::generator::GenScratch::default();
    let Some((target, _)) = pipeline.find_nearest_structure(&mut gs, &located_structures(), at, 100) else { return };
    let stack = p.in_hand(off_hand).clone();
    let seed = crate::ranged::projectile_seed(level, p, spawns.len() as u64);
    // `getY(0.5)`: half the player's height up.
    let height = p.dimensions().1 as f64;
    let mut e = kiln_entity::ext_entity::eye_of_ender::new(Vec3::new(p.pos[0], p.pos[1] + height * 0.5, p.pos[2]), &stack, seed);
    kiln_entity::ext_entity::eye_of_ender::signal_to(&mut e, Vec3::new(target[0] as f64, target[1] as f64, target[2] as f64));
    crate::ranged::push_spawn(spawns, e);
    p.used_ender_eye(target);
    // `Mth.lerp(random.nextFloat(), 0.33F, 0.5F)`.
    let t = level.random().next_float();
    let pitch = 0.33 + t * (0.5 - 0.33);
    p.sound_for_all("minecraft:entity.ender_eye.launch", kiln_proto::packets::world_fx::SoundSource::Neutral, 1.0, pitch);
    consume(p, off_hand);
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
}
