//! Husk: a desert zombie that does not burn and makes its target hungry.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

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
}
