//! Stray: a skeleton of the cold whose arrows slow.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Stray;

pub static KIND: Stray = Stray;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:stray", &[(MovementSpeed, 0.25)])
};

impl Kind for Stray {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
