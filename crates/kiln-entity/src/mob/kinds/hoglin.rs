//! Hoglin.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Hoglin;

pub static KIND: Hoglin = Hoglin;

static INFO: Info = Info {
    animal: true,
    ageable: true,
    ..Info::monster("minecraft:hoglin", &[(MaxHealth, 40.0), (MovementSpeed, 0.30000001192092896), (KnockbackResistance, 0.6000000238418579), (AttackKnockback, 1.0), (AttackDamage, 6.0)])
};

impl Kind for Hoglin {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
