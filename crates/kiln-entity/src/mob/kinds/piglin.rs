//! Piglin: admires gold and barters.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Piglin;

pub static KIND: Piglin = Piglin;

static INFO: Info = Info::monster("minecraft:piglin", &[(MaxHealth, 16.0), (MovementSpeed, 0.3499999940395355), (AttackDamage, 5.0)]);

impl Kind for Piglin {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
