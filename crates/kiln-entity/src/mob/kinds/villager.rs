//! Villager: professions and trading.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Villager;

pub static KIND: Villager = Villager;

static INFO: Info = Info {
    ageable: true,
    ..Info::misc("minecraft:villager", &[(MovementSpeed, 0.5)])
};

impl Kind for Villager {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
