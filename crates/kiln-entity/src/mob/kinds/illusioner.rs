//! Illusioner: a spellcasting illager with a bow (`RangedBowAttackGoal`) that turns invisible
//! and blinds its targets on hard difficulty. Its mirror images are client side.

use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::Vec3;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::{Goal, Living, Wanted};
use crate::mob::kinds::evoker::{self, CastingSpellGoal, Spell, UseSpellGoal};
use crate::mob::kinds::raider::{self, IllagerState};
use crate::mob::kinds::zombie::{IRON_GOLEM, VILLAGERS};
use crate::mob::{DamageSource, GroupData, MAINHAND, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::EntityData;

pub struct Illusioner;

pub static KIND: Illusioner = Illusioner;

static INFO: Info = Info {
    sounds: Some("illusioner"),
    ..Info::monster("minecraft:illusioner", &[(MovementSpeed, 0.5), (FollowRange, 18.0), (MaxHealth, 32.0)])
};

impl Kind for Illusioner {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(IllagerState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        raider::register_raider_goals(m);
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Custom(Box::new(CastingSpellGoal { name: "SpellcasterCastingSpellGoal" })));
        g.add(3, raider::never());
        g.add(4, Goal::Custom(Box::new(UseSpellGoal::new(Spell::Disappear))));
        g.add(5, Goal::Custom(Box::new(UseSpellGoal::new(Spell::Blindness))));
        g.add(
            6,
            Goal::RangedBow {
                speed: 0.5,
                interval_min: 20,
                radius_sqr: 15.0 * 15.0,
                attack_time: -1,
                see_time: 0,
                strafing_clockwise: false,
                strafing_backwards: false,
                strafing_time: -1,
            },
        );
        g.add(8, Goal::RandomStroll { speed: 0.6, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(9, Goal::LookAtPlayer { dist: 3.0, probability: 1.0, look_at: None, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(raider::LookAtMobGoal::new(8.0))));
        let t = &mut m.targets;
        t.add(1, raider::hurt_by_ignoring_raiders());
        t.add(2, raider::nearest_with_memory(Wanted::Player, true, 300));
        t.add(3, raider::nearest_with_memory(Wanted::Types(VILLAGERS), false, 300));
        t.add(3, raider::nearest_with_memory(Wanted::Types(IRON_GOLEM), false, 300));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        raider::ai_step_before(e, m, level);
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        evoker::KIND.custom_server_ai_step(e, m, level);
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        raider::die(e, m, level, source);
    }

    fn remove_when_far_away_at(&self, m: &MobData, dist_sqr: f64) -> Option<bool> {
        Some(raider::remove_when_far_away(m, dist_sqr))
    }

    fn can_attack(&self, _m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        raider::illager_can_attack(level, t)
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        m.equipment[MAINHAND] = ItemStack::of("minecraft:bow", 1).unwrap_or_else(ItemStack::empty);
        raider::finalize_spawn(m, r, group);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        raider::load(m, r);
        if let Some(s) = raider::illager_mut(m) {
            s.spell_ticks = r.int_or("SpellTicks", 0);
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        raider::save(m, o);
        o.put("SpellTicks", Tag::Int(raider::illager(m).map_or(0, |s| s.spell_ticks)));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        evoker::spellcaster_data(m, d);
    }
}
