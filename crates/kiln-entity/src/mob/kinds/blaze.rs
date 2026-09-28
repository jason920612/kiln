//! Blaze: hovers and shoots small fireballs.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Blaze;

pub static KIND: Blaze = Blaze;

static INFO: Info = Info {
    fire_immune: true,
    ..Info::monster("minecraft:blaze", &[(AttackDamage, 6.0), (MovementSpeed, 0.23000000417232513), (FollowRange, 48.0)])
};

impl Kind for Blaze {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
