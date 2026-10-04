//! Stray: a skeleton of the cold whose arrows slow.

use super::skeleton;
use super::zombie;
use crate::entity::{Entity, EntityKind};
use crate::math::BlockPos;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, SpawnView};
use crate::mob::{GroupData, MobData, SpawnContext};
use kiln_javamath::random::{LegacyRandom, RandomSource};

pub struct Stray;

pub static KIND: Stray = Stray;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:stray", &[(MovementSpeed, 0.25)])
};

impl Kind for Stray {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn register_goals(&self, m: &mut MobData) {
        skeleton::register_goals(m);
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        skeleton::finalize(e, m, r, ctx);
    }

    /// `Stray.getArrow`: a plain arrow carries slowness (30 seconds; an eighth of it on a hit).
    fn ranged_arrow(&self, _e: &mut Entity, _m: &mut MobData, arrow: &mut Entity) {
        if arrow.type_name == "minecraft:arrow"
            && let EntityKind::Arrow(a) = &mut arrow.kind
        {
            a.effects.push(("minecraft:slowness", 600, 0));
        }
    }

    /// `Stray.checkStraySpawnRules`: the monster rules, and the sky visible above any powder
    /// snow the position is in.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        let mut p = pos.above();
        while crate::blocks::block_name(view.block(p)) == "minecraft:powder_snow" {
            p = p.above();
        }
        Some(zombie::monster_rules(view, pos, r) && (view.spawner() || view.sky_light(p.below()) >= 15))
    }
}
