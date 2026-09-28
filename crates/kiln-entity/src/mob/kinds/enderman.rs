//! Enderman: teleports, carries blocks, angers when stared at.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Enderman;

pub static KIND: Enderman = Enderman;

static INFO: Info = Info::monster("minecraft:enderman", &[(MaxHealth, 40.0), (MovementSpeed, 0.30000001192092896), (AttackDamage, 7.0), (FollowRange, 64.0), (StepHeight, 1.0)]);

impl Kind for Enderman {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
