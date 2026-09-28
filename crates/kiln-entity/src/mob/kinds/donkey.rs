//! Donkey.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Donkey;

pub static KIND: Donkey = Donkey;

static INFO: Info = Info::animal("minecraft:donkey", &[(MaxHealth, 53.0), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5), (MovementSpeed, 0.17499999701976776), (JumpStrength, 0.5)]);

impl Kind for Donkey {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
