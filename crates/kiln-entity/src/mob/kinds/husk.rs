//! Husk: a desert zombie that does not burn in daylight, makes its target hungry and turns into
//! a zombie after drowning for a while.

use super::zombie::{self, ZombieState};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::Living;
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, SpawnContext};
use crate::math::BlockPos;
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::entity::EntityData;

pub struct Husk;

pub static KIND: Husk = Husk;

static INFO: Info = Info {
    burns_in_daylight: false,
    breathes_under_water: true,
    ..Info::monster("minecraft:husk", &[(FollowRange, 35.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 3.0), (Armor, 2.0), (SpawnReinforcements, 0.0)])
};

impl Kind for Husk {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(ZombieState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        zombie::register_goals(m);
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        zombie::tick_drowning(e, m, level);
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt {
            zombie::reinforcements(e, m, level, source);
        }
    }

    /// `Husk.doHurtTarget`: an empty-handed husk's hit makes the target hungry.
    fn after_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
        if m.equipment[mob::MAINHAND].is_empty() {
            let eff = level.effective_difficulty(e.block_position());
            level.add_effect(t.id, "minecraft:hunger", 140 * eff as i32, 0, Some(e.id));
        }
    }

    /// `Husk.finalizeSpawn`: the zombie's, a second loot pickup roll, and for natural spawns,
    /// where the camel's box is free (`group.camel_space`, looked at by the caller), one husk in
    /// ten rides a camel husk with an iron spear, a parched sitting behind it.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        zombie::finalize(e, m, r, ctx, group, false);
        m.can_pick_up_loot = r.next_float() < 0.55 * ctx.special_multiplier;
        if group.natural && group.camel_space && r.next_float() < 0.1 {
            if let Some(s) = kiln_item::ItemStack::of("minecraft:iron_spear", 1) {
                m.equipment[mob::MAINHAND] = s;
            }
            // `camelHusk.setPos(x, y, z)`, finalized with no group data, ridden by this husk.
            let mut camel = mob::new_jockey_at(e, MobKind::CamelHusk, false);
            mob::finalize_spawn(&mut camel, r, ctx, &mut GroupData::default(), true);
            group.companions.push(mob::Companion { entity: camel, seat: mob::Seat::UnderMob });
            // The parched: `snapTo` the husk's place and yaw, finalized, riding the camel husk.
            let mut parched = mob::new_jockey(e, MobKind::Parched);
            mob::finalize_spawn(&mut parched, r, ctx, &mut GroupData::default(), true);
            let camel_index = group.companions.len() - 1;
            group.companions.push(mob::Companion { entity: parched, seat: mob::Seat::OnCompanion(camel_index) });
        }
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        zombie::load(e, m, r);
    }

    fn save(&self, e: &Entity, m: &MobData, o: &mut Output) {
        zombie::save(e, m, o);
    }

    fn entity_data(&self, e: &Entity, m: &MobData, d: &mut EntityData) {
        zombie::entity_data(e, m, d);
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { zombie::baby_dimensions(m.kind) } else { base }
    }

    /// `Monster.checkSurfaceMonstersSpawnRules`: dark enough, and under the open sky.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        Some(zombie::monster_rules(view, pos, r) && (view.spawner() || view.sky_light(pos) >= 15))
    }

    fn placement(&self) -> Placement {
        Placement::OnGround
    }
}
