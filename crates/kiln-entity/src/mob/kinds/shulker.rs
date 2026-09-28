//! Shulker: peeks and shoots homing bullets.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Shulker;

pub static KIND: Shulker = Shulker;

static INFO: Info = Info::monster("minecraft:shulker", &[(MaxHealth, 30.0)]);

impl Kind for Shulker {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
