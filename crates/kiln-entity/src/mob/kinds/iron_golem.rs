//! Iron golem.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct IronGolem;

pub static KIND: IronGolem = IronGolem;

static INFO: Info = Info::misc("minecraft:iron_golem", &[(MaxHealth, 100.0), (MovementSpeed, 0.25), (KnockbackResistance, 1.0), (AttackDamage, 15.0), (StepHeight, 1.0)]);

impl Kind for IronGolem {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
