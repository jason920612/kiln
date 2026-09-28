//! Phantom: circles and swoops at sleepless players.

#[allow(unused_imports)]
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Phantom;

pub static KIND: Phantom = Phantom;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:phantom", &[])
};

impl Kind for Phantom {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
