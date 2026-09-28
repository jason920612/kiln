//! Ghast: floats and shoots fireballs.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Ghast;

pub static KIND: Ghast = Ghast;

static INFO: Info = Info {
    fire_immune: true,
    ..Info::monster("minecraft:ghast", &[(MaxHealth, 10.0), (FollowRange, 100.0), (CameraDistance, 8.0), (FlyingSpeed, 0.06)])
};

impl Kind for Ghast {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
