//! Zoglin: a stub until its work package fills it in (attributes and behaviour still to port from
//! vanilla; see `crates/kiln-entity/src/mob/brain` for the brain framework).

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Zoglin;

pub static KIND: Zoglin = Zoglin;

static INFO: Info = Info::monster("minecraft:zoglin", &[(MaxHealth, 10.0), (MovementSpeed, 0.25)]);

impl Kind for Zoglin {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
