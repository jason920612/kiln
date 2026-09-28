//! Vindicator: an axe-wielding illager (`MeleeAttackGoal`), opening doors in raided villages.
//! One named Johnny attacks every living thing that is not an illager friend.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityFilter, EntityLevel};
use crate::math::Vec3;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, MeleeKind, Wanted, MOVE, TARGET};
use crate::mob::kinds::raider::{self, IllagerState};
use crate::mob::kinds::zombie::{IRON_GOLEM, VILLAGERS, nearest};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Vindicator;

pub static KIND: Vindicator = Vindicator;

static INFO: Info = Info {
    sounds: Some("vindicator"),
    ..Info::monster(
        "minecraft:vindicator",
        &[(MovementSpeed, 0.3499999940395355), (FollowRange, 12.0), (MaxHealth, 24.0), (AttackDamage, 5.0)],
    )
};

fn st(m: &MobData) -> &IllagerState {
    raider::illager(m).expect("vindicator state")
}

impl Kind for Vindicator {
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
        g.add(1, raider::never());
        g.add(2, Goal::Custom(Box::new(BreakDoorGoal)));
        g.add(3, Goal::Custom(Box::new(RaiderOpenDoorGoal)));
        g.add(4, Goal::Custom(Box::new(raider::HoldGroundAttackGoal { hostile_radius_sqr: 100.0 })));
        g.add(
            5,
            Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 },
        );
        let t = &mut m.targets;
        t.add(1, raider::hurt_by_ignoring_raiders());
        t.add(2, nearest(Wanted::Player, true));
        t.add(3, nearest(Wanted::Types(VILLAGERS), true));
        t.add(3, nearest(Wanted::Types(IRON_GOLEM), true));
        t.add(4, Goal::Custom(Box::new(JohnnyAttackGoal { target: None, unseen: 0 })));
        let g = &mut m.goals;
        g.add(8, Goal::RandomStroll { speed: 0.6, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(9, Goal::LookAtPlayer { dist: 3.0, probability: 1.0, look_at: None, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(raider::LookAtMobGoal::new(8.0))));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        raider::ai_step_before(e, m, level);
    }

    /// `customServerAiStep`: doors open for it where a raid is on.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !m.no_ai {
            m.nav.can_open_doors = level.raid_at(e.block_position()).is_some();
        }
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

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        raider::finalize_spawn(m, r, group);
        ext::mob_finalize(m, r);
        m.nav.can_open_doors = true;
        // `populateDefaultEquipmentSlots`: an iron axe outside raids.
        if raider::raider(m).is_none_or(|st| st.raid.is_none()) {
            m.equipment[MAINHAND] = ItemStack::of("minecraft:iron_axe", 1).unwrap_or_else(ItemStack::empty);
        }
        super::zombie::populate_enchantments(m, r, ctx);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        raider::load(m, r);
        let johnny = r.bool_or("Johnny", false) || custom_name(e) == Some("Johnny");
        if let Some(s) = raider::illager_mut(m) {
            s.johnny = johnny;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        raider::save(m, o);
        if st(m).johnny {
            o.put("Johnny", Tag::Byte(1));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::raider::IS_CELEBRATING, &DataValue::Boolean(st(m).raider.celebrating));
    }
}

/// The plain text of an entity's saved `CustomName`.
pub fn custom_name(e: &Entity) -> Option<&str> {
    let t = e.extra.iter().find(|(k, _)| k == "CustomName").map(|(_, t)| t)?;
    match t {
        Tag::String(s) => Some(s.as_str()),
        c @ Tag::Compound(_) => c.get("text").and_then(Tag::as_str),
        _ => None,
    }
}

/// `Vindicator.applyRaidBuffs`: a fresh iron axe (enchanted with the raid's odds by vanilla's
/// providers; the enchanting is not applied, its roll is).
pub fn apply_raid_buffs(e: &mut Entity, m: &mut MobData, enchant_odds: f32) {
    let _ = e.random.next_float() <= enchant_odds;
    m.equipment[MAINHAND] = ItemStack::of("minecraft:iron_axe", 1).unwrap_or_else(ItemStack::empty);
}

/// `VindicatorBreakDoorGoal`: only in an active raid, one in ten tries. Kiln's doors cannot be
/// broken yet: the roll is made, the goal never starts.
#[derive(Clone, Debug)]
struct BreakDoorGoal;

impl CustomGoal for BreakDoorGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "VindicatorBreakDoorGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !raider::has_active_raid(m, level) {
            return false;
        }
        let _ = e.random.next_int_bounded(reduced_tick_delay(10));
        false
    }
}

/// `AbstractIllager.RaiderOpenDoorGoal` (doors open through the navigation's door flag; the
/// goal itself is not simulated).
#[derive(Clone, Debug)]
struct RaiderOpenDoorGoal;

impl CustomGoal for RaiderOpenDoorGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RaiderOpenDoorGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
}

/// `VindicatorJohnnyAttackGoal`: `NearestAttackableTargetGoal<LivingEntity>` without a random
/// interval, for Johnny only.
#[derive(Clone, Debug)]
struct JohnnyAttackGoal {
    target: Option<i32>,
    unseen: i32,
}

impl CustomGoal for JohnnyAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "VindicatorJohnnyAttackGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !st(m).johnny {
            return false;
        }
        let range = m.attrs.value(FollowRange);
        let area = e.bounding_box().inflate(range, 4.0, range);
        let eye = Vec3::new(e.x(), e.eye_y(), e.z());
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            let Some(t) = goals::living(level, id) else { continue };
            if !t.player && raider::illager_ally(t.type_name) {
                continue;
            }
            if !goals::targeting_ok(e, m, level, &t, true, range, true) {
                continue;
            }
            let d = t.pos.distance_to_sqr(eye);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.target = best.map(|(_, id)| id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, self.target, true, &mut self.unseen, 60)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        mob::set_target(e, m, self.target);
        self.unseen = 0;
        m.no_action_time = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        mob::set_target(e, m, None);
        self.target = None;
    }
}
