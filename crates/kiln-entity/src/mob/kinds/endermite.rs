//! Endermite: a small monster that lives two minutes (unless persistent).

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::Vec3;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{Goal, MeleeKind, Wanted, JUMP};
use crate::mob::{MobData, mth};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

pub struct Endermite;

pub static KIND: Endermite = Endermite;

static INFO: Info = Info::monster("minecraft:endermite", &[(MaxHealth, 8.0), (MovementSpeed, 0.25), (AttackDamage, 2.0)]);

/// `MAX_LIFE`.
const MAX_LIFE: i32 = 2400;

#[derive(Clone, Debug)]
pub struct EndermiteState {
    pub life: i32,
}

impl Kind for Endermite {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(EndermiteState { life: 0 }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(1, Goal::Custom(Box::new(ClimbOnTopOfPowderSnow)));
        g.add(2, Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 });
        g.add(3, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::NearestAttackable { wanted: Wanted::Player, interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false });
    }

    /// `Endermite.tick`: the body turns with the yaw.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.y_body_rot = e.y_rot;
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let persistent = m.persistence_required;
        let s = ext::state_mut::<EndermiteState>(m).expect("endermite state");
        if !persistent {
            s.life += 1;
        }
        if s.life >= MAX_LIFE {
            e.discard();
        }
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(3)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let life = r.int_or("Lifetime", 0);
        ext::state_mut::<EndermiteState>(m).expect("endermite state").life = life;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("Lifetime", Tag::Int(ext::state::<EndermiteState>(m).map_or(0, |s| s.life)));
    }
}

/// `ClimbOnTopOfPowderSnowGoal`: jumps out of powder snow when the block above is free.
#[derive(Clone, Debug)]
pub struct ClimbOnTopOfPowderSnow;

impl CustomGoal for ClimbOnTopOfPowderSnow {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ClimbOnTopOfPowderSnowGoal"
    }
    fn flags(&self) -> u8 {
        JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !(e.was_in_powder_snow || e.is_in_powder_snow) {
            return false;
        }
        let above = e.block_position().above();
        let s = level.block(above);
        if crate::blocks::kind(s) == crate::blocks::Kind::PowderSnow {
            return true;
        }
        let (shape, _) = crate::collision::collision_shape(s, above, &crate::collision::CollisionContext::EMPTY);
        shape.is_empty()
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.jump.jump = true;
    }
}
