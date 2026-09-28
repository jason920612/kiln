//! Wither skeleton.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct WitherSkeleton;

pub static KIND: WitherSkeleton = WitherSkeleton;

static INFO: Info = Info {
    breathes_under_water: true,
    fire_immune: true,
    ..Info::monster("minecraft:wither_skeleton", &[(MovementSpeed, 0.25)])
};

impl Kind for WitherSkeleton {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
