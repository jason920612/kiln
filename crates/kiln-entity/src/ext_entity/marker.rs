//! Markers (`Marker`): a point in the world that does nothing and is never sent to a client; datapacks keep
//! data on it (`data`, which every entity keeps in its saved compound) and find it with selectors.

use crate::entity::Entity;
use crate::ext_entity::EntityExt;
use crate::level::EntityLevel;
use crate::persist::Input;

#[derive(Clone, Debug)]
pub struct Marker;

pub fn load(_r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(Marker))
}

impl EntityExt for Marker {
    crate::entity_ext_boilerplate!();

    /// `Marker.tick` does nothing.
    fn tick(&mut self, _e: &mut Entity, _level: &mut dyn EntityLevel) {}
}
