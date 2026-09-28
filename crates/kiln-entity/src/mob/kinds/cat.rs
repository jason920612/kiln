//! Cat.

use super::tame::Tame;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Cat;

pub static KIND: Cat = Cat;

static INFO: Info = Info::animal("minecraft:cat", &[(MaxHealth, 10.0), (MovementSpeed, 0.30000001192092896), (AttackDamage, 3.0)]);

#[derive(Clone, Debug)]
pub struct State {
    pub tame: Tame,
}

impl Kind for Cat {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
