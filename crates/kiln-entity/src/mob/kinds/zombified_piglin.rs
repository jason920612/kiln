//! Zombified piglin: neutral until hurt, then angry with its group.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct ZombifiedPiglin;

pub static KIND: ZombifiedPiglin = ZombifiedPiglin;

static INFO: Info = Info {
    breathes_under_water: true,
    fire_immune: true,
    ..Info::monster("minecraft:zombified_piglin", &[(FollowRange, 35.0), (Armor, 2.0), (SpawnReinforcements, 0.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 5.0)])
};

impl Kind for ZombifiedPiglin {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
