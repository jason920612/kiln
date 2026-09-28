//! Drowned: an underwater zombie that throws tridents.

use super::zombie::{self, HasZombie, ZombieState};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, MobExt};
use crate::mob::{DamageSource, GroupData, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::EntityData;

pub struct Drowned;

pub static KIND: Drowned = Drowned;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:drowned", &[(FollowRange, 35.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 3.0), (Armor, 2.0), (SpawnReinforcements, 0.0), (StepHeight, 1.0)])
};

#[derive(Clone, Debug)]
pub struct DrownedState {
    pub zombie: ZombieState,
    pub searching_for_land: bool,
    pub ranged_uncertainty: f32,
}

impl HasZombie for DrownedState {
    fn zombie(&self) -> &ZombieState {
        &self.zombie
    }
    fn zombie_mut(&mut self) -> &mut ZombieState {
        &mut self.zombie
    }
}

impl Kind for Drowned {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(DrownedState { zombie: ZombieState::default(), searching_for_land: false, ranged_uncertainty: -1.0 }))
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt {
            zombie::reinforcements(e, m, level, source);
        }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        zombie::finalize(e, m, r, ctx, group, false);
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
}
