//! Wolf: tamed with bones, sits and follows its owner.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Wolf;

pub static KIND: Wolf = Wolf;

static INFO: Info = Info::animal("minecraft:wolf", &[(MovementSpeed, 0.30000001192092896), (MaxHealth, 8.0), (AttackDamage, 4.0)]);

impl Kind for Wolf {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
