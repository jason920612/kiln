//! Slime: hops, splits when it dies.

#[allow(unused_imports)]
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind};

pub struct Slime;

pub static KIND: Slime = Slime;

static INFO: Info = Info::monster("minecraft:slime", &[]);

impl Kind for Slime {
    fn info(&self) -> &'static Info {
        &INFO
    }
}
