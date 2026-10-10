//! Landing: `Block.fallOn` overrides and `Entity.causeFallDamage`.

use crate::blocks::{Kind, kind};
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityLevel, Event};
use crate::math::BlockPos;
use crate::physics;
use kiln_javamath::random::RandomSource;

/// `Block.fallOn` for the block the entity landed on.
pub fn fall_on(e: &mut Entity, level: &mut dyn EntityLevel, state: u16, _pos: BlockPos) {
    let distance = e.fall_distance;
    match kind(state) {
        Kind::Farmland => {
            // Only living entities trample, but every landing draws from the level random.
            let _ = (level.random().next_float() as f64) < distance - 0.5;
            default_fall_on(e, level, state, distance);
        }
        Kind::HoneyBlock => {
            e.play_sound(level, "minecraft:block.honey_block.slide", 1.0, 1.0);
            level.emit(Event::EntityEvent { entity: e.id, event: 54 });
            e.honey_fall = true;
            cause_fall_damage(e, level, distance, 0.2);
        }
        Kind::Slime if !e.is_suppressing_bounce() => {
            cause_fall_damage(e, level, distance, 0.0);
        }
        _ => {
            if crate::blocks::block_name(state) == "minecraft:hay_block" {
                cause_fall_damage(e, level, distance, 0.2);
            } else {
                default_fall_on(e, level, state, distance);
            }
        }
    }
}

fn default_fall_on(e: &mut Entity, level: &mut dyn EntityLevel, state: u16, distance: f64) {
    let reduction = physics::block_factors(state).fall_reduction;
    cause_fall_damage(e, level, distance * (1.0 - reduction) as f64, 1.0);
}

/// `causeFallDamage(distance, multiplier, fall)`.
pub fn cause_fall_damage(e: &mut Entity, level: &mut dyn EntityLevel, distance: f64, multiplier: f32) -> bool {
    match e.kind {
        EntityKind::FallingBlock(_) => crate::falling_block::cause_fall_damage(e, level, distance, multiplier),
        EntityKind::MobTicking { .. } => {
            e.pending_fall = Some((distance, multiplier));
            false
        }
        // `MinecartTNT.causeFallDamage`: a hard landing sets it off, which removes the cart at
        // once (its move ends there); the cart works out the blast right after its move.
        EntityKind::Other { type_name: "minecraft:tnt_minecart" } => {
            e.pending_fall = Some((distance, multiplier));
            if distance >= 3.0 && level.tnt_explodes() {
                e.discard();
            }
            false
        }
        _ => false,
    }
}
