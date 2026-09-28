//! Zombie villager.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct ZombieVillager;

pub static KIND: ZombieVillager = ZombieVillager;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:zombie_villager", &[(FollowRange, 35.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 3.0), (Armor, 2.0), (SpawnReinforcements, 0.0)])
};

impl Kind for ZombieVillager {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
