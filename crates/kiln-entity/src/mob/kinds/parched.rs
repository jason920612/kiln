//! Parched: the desert skeleton (16 health), slower to shoot (every 70 ticks, 50 on hard), whose
//! arrows weaken (30 seconds; an eighth of it on a hit) and that cannot be weakened itself. It
//! does not burn in daylight. A husk's camel jockey brings one along.

use super::skeleton;
use super::zombie;
use crate::entity::{Entity, EntityKind};
use crate::math::BlockPos;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, SpawnView};
use crate::mob::{GroupData, MobData, SpawnContext};
use kiln_javamath::random::{LegacyRandom, RandomSource};

pub struct Parched;

pub static KIND: Parched = Parched;

static INFO: Info = Info {
    breathes_under_water: true,
    ..Info::monster("minecraft:parched", &[(MovementSpeed, 0.25), (MaxHealth, 16.0)])
};

impl Kind for Parched {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn register_goals(&self, m: &mut MobData) {
        skeleton::register_goals(m);
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        skeleton::finalize(e, m, r, ctx);
    }

    /// `getAttackInterval` / `getHardAttackInterval`.
    fn bow_interval(&self, hard: bool) -> Option<i32> {
        Some(if hard { 50 } else { 70 })
    }

    /// `Parched.getArrow`: a plain arrow carries weakness (30 seconds).
    fn ranged_arrow(&self, _e: &mut Entity, _m: &mut MobData, arrow: &mut Entity) {
        if arrow.type_name == "minecraft:arrow"
            && let EntityKind::Arrow(a) = &mut arrow.kind
        {
            a.effects.push(("minecraft:weakness", 600, 0));
        }
    }

    /// `Parched.canBeAffected`: never weak.
    fn can_be_affected(&self, _m: &MobData, effect: &crate::effect::Effect, base: bool) -> bool {
        base && effect.id != crate::effect::ids::weakness()
    }

    /// `Monster.checkSurfaceMonstersSpawnRules`: dark enough, and under the open sky.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        Some(zombie::monster_rules(view, pos, r) && (view.spawner() || view.sky_light(pos) >= 15))
    }
}
