//! What the skeleton family shares (vanilla's `AbstractSkeleton`: skeletons, strays, wither
//! skeletons): the goals, `finalizeSpawn`, and the skeleton's powder snow conversion into a
//! stray. The plain skeleton lives in the shared mob code and calls in here.

use super::common_a::{Avoid, AvoidEntityGoal};
use super::zombie::{self, IRON_GOLEM, PIGLINS, hurt_by, nearest, stroll};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::attributes::Attr;
use crate::mob::goals::{Goal, MeleeKind, Wanted};
use crate::mob::{self, MobData, MobKind, SpawnContext, Species};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

/// `AbstractSkeleton.registerGoals` (wither skeletons first target piglins) and the melee goal
/// `reassessWeaponGoal` adds in the constructor (empty hands).
pub fn register_goals(m: &mut MobData) {
    if m.kind == MobKind::WitherSkeleton {
        m.targets.add(3, nearest(Wanted::Types(PIGLINS), true));
    }
    let g = &mut m.goals;
    g.add(2, Goal::RestrictSun);
    g.add(3, Goal::FleeSun { speed: 1.0, wanted: Vec3::ZERO });
    // `AvoidEntityGoal<Wolf>(this, Wolf.class, 6.0F, 1.0, 1.2)`.
    g.add(3, Goal::Custom(Box::new(AvoidEntityGoal::new("AvoidEntityGoal", Avoid::Types(&["minecraft:wolf"]), 6.0, 1.0, 1.2))));
    g.add(5, stroll(1.0, true));
    g.add(6, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
    g.add(6, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    let t = &mut m.targets;
    t.add(1, hurt_by(false));
    t.add(2, nearest(Wanted::Player, true));
    t.add(3, nearest(Wanted::Types(IRON_GOLEM), true));
    // Baby turtles on land.
    t.add(3, nearest(Wanted::BabyTurtlesOnLand, true));
    m.goals.add(
        4,
        Goal::Melee { kind: MeleeKind::Plain, speed: 1.2, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 },
    );
    if m.kind == MobKind::WitherSkeleton {
        m.maluses.push((mob::path::PathType::Lava, 8.0));
    }
}

/// `AbstractSkeleton.finalizeSpawn` (and `WitherSkeleton`'s), from `r` (the level's random).
pub fn finalize(e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext) {
    let _ = e;
    super::super::ext::mob_finalize(m, r);
    if m.kind == MobKind::WitherSkeleton {
        // No armor roll, no enchantments.
        m.equipment[mob::MAINHAND] = ItemStack::of("minecraft:stone_sword", 1).unwrap_or_else(ItemStack::empty);
    } else {
        zombie::populate_armor(m, r, ctx);
        m.equipment[mob::MAINHAND] = ItemStack::of("minecraft:bow", 1).unwrap_or_else(ItemStack::empty);
        zombie::populate_enchantments(m, r, ctx);
    }
    mob::reassess_weapon_goal(m, ctx.hard);
    m.can_pick_up_loot = r.next_float() < 0.55 * ctx.special_multiplier;
    if ctx.halloween && m.equipment[mob::HEAD].is_empty() && r.next_float() < 0.25 {
        let name = if r.next_float() < 0.1 { "minecraft:jack_o_lantern" } else { "minecraft:carved_pumpkin" };
        if let Some(s) = ItemStack::of(name, 1) {
            m.equipment[mob::HEAD] = s;
            m.drop_chances[mob::HEAD] = 0.0;
        }
    }
    if m.kind == MobKind::WitherSkeleton {
        if let Some(i) = m.attrs.get_mut(Attr::AttackDamage) {
            i.base = 4.0;
        }
        mob::reassess_weapon_goal(m, ctx.hard);
    }
}

/// `Skeleton.tick`'s freezing tracker (before `Mob.tick`): 140 ticks in powder snow start a
/// 300 tick conversion into a stray.
pub fn tick_freezing(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !mob::is_alive(e, m) || m.no_ai {
        return;
    }
    let snow = e.is_in_powder_snow;
    let Species::Skeleton { freezing } = &mut m.species else { return };
    if freezing.tick(snow, 140, 300) {
        mob::convert::convert_to(e, m, level, MobKind::Stray, true, true, |ne, _, level| {
            if !ne.silent {
                level.emit(Event::LevelEvent { event: 1048, pos: ne.block_position(), data: 0 });
            }
        });
    }
}
