//! Magma cube.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct MagmaCube;

pub static KIND: MagmaCube = MagmaCube;

static INFO: Info = Info {
    fire_immune: true,
    ..Info::monster("minecraft:magma_cube", &[(MovementSpeed, 0.20000000298023224)])
};

impl Kind for MagmaCube {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
