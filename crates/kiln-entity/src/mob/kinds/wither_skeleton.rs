//! Wither skeleton: a nether fortress skeleton with a stone sword whose hits wither.

use super::skeleton;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, SpawnView};
use crate::mob::goals::Living;
use crate::mob::{GroupData, MobData, SpawnContext};
use crate::math::BlockPos;
use kiln_javamath::random::{LegacyRandom, RandomSource};

pub struct WitherSkeleton;

pub static KIND: WitherSkeleton = WitherSkeleton;

static INFO: Info = Info {
    burns_in_daylight: false,
    breathes_under_water: true,
    fire_immune: true,
    ..Info::monster("minecraft:wither_skeleton", &[(MovementSpeed, 0.25)])
};

impl Kind for WitherSkeleton {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// `WitherSkeleton.canBeAffected`: never withered.
    fn can_be_affected(&self, _m: &MobData, effect: &crate::effect::Effect, base: bool) -> bool {
        base && effect.id != crate::effect::ids::wither()
    }

    fn register_goals(&self, m: &mut MobData) {
        skeleton::register_goals(m);
    }

    /// `WitherSkeleton.finalizeSpawn`: a stone sword (no armor, no enchantments) and attack
    /// damage 4.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        skeleton::finalize(e, m, r, ctx);
    }

    /// `WitherSkeleton.doHurtTarget`: ten seconds of wither.
    fn after_hurt_target(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
        level.add_effect(t.id, "minecraft:wither", 200, 0, Some(e.id));
    }

    /// `WitherSkeleton.getArrow`: its arrows burn (with a bow).
    fn ranged_arrow(&self, _e: &mut Entity, _m: &mut MobData, arrow: &mut Entity) {
        arrow.ignite_for_seconds(100.0);
    }

    /// Wither skeletons come from nether fortress spawns (`SpawnPlacements`: the monster rules,
    /// `Monster.checkMonsterSpawnRules`).
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        Some(super::zombie::monster_rules(view, pos, r))
    }

    fn dimensions(&self, _m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        base
    }
}
