//! Mule.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Mule;

pub static KIND: Mule = Mule;

static INFO: Info = Info::animal("minecraft:mule", &[(MaxHealth, 53.0), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5), (MovementSpeed, 0.17499999701976776), (JumpStrength, 0.5)]);

impl Kind for Mule {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
