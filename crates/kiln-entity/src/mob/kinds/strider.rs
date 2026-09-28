//! Strider: walks on lava.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Strider;

pub static KIND: Strider = Strider;

static INFO: Info = Info {
    fire_immune: true,
    ..Info::animal("minecraft:strider", &[(MovementSpeed, 0.17499999701976776)])
};

impl Kind for Strider {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
