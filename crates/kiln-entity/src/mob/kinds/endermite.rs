//! Endermite.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Endermite;

pub static KIND: Endermite = Endermite;

static INFO: Info = Info::monster("minecraft:endermite", &[(MaxHealth, 8.0), (MovementSpeed, 0.25), (AttackDamage, 2.0)]);

impl Kind for Endermite {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
