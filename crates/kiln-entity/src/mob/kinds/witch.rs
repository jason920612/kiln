//! Witch: throws potions, drinks its own.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Witch;

pub static KIND: Witch = Witch;

static INFO: Info = Info::monster("minecraft:witch", &[(MaxHealth, 26.0), (MovementSpeed, 0.25)]);

impl Kind for Witch {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
