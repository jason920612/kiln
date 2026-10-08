//! Giant (`Giant`): a huge monster that vanilla never spawns by itself (`/summon` only). It has 100 health, hits
//! for 50 and has no goals at all: it stands where it is put.

use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};
use crate::mob::MobData;

pub struct Giant;

pub static KIND: Giant = Giant;

static INFO: Info = Info::monster("minecraft:giant", &[(MaxHealth, 100.0), (MovementSpeed, 0.5), (AttackDamage, 50.0)]);

impl Kind for Giant {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// `Giant` registers no goals.
    fn register_goals(&self, _m: &mut MobData) {}
}
