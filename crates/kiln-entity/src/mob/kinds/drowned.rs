//! Drowned: an underwater zombie that throws tridents.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Drowned;

pub static KIND: Drowned = Drowned;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:drowned", &[(FollowRange, 35.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 3.0), (Armor, 2.0), (SpawnReinforcements, 0.0), (StepHeight, 1.0)])
};

impl Kind for Drowned {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
