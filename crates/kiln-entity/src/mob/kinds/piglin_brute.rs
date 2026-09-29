//! PiglinBrute: a stub until its work package fills it in (attributes and behaviour still to port from
//! vanilla; see `crates/kiln-entity/src/mob/brain` for the brain framework).

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct PiglinBrute;

pub static KIND: PiglinBrute = PiglinBrute;

static INFO: Info = Info::monster("minecraft:piglin_brute", &[(MaxHealth, 10.0), (MovementSpeed, 0.25)]);

impl Kind for PiglinBrute {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
