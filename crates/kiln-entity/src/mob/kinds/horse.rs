//! Horse: tamed by riding, saddled, ridden.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Horse;

pub static KIND: Horse = Horse;

static INFO: Info = Info::animal("minecraft:horse", &[(JumpStrength, 0.7), (MaxHealth, 53.0), (MovementSpeed, 0.22499999403953552), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5)]);

impl Kind for Horse {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
