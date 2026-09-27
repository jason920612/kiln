//! Predicates used by loot conditions: item predicates (evaluated here), block predicates
//! (state here, block entity through the context) and the world predicates the context
//! evaluates.

pub mod block;
pub mod item;
pub mod world;

pub use block::{BlockEntityPredicate, BlockPredicate};
pub use item::{Components, components_match, item_matches};
pub use world::{DamageSourcePredicate, EntityPredicate, EntitySubPredicate, LocationPredicate};

use crate::json::Json;
use crate::parse::{PResult, ParseError};
use kiln_item::component::ItemPredicate;

/// `ItemPredicate.CODEC`.
pub fn item_predicate(j: &Json) -> PResult<ItemPredicate> {
    crate::parse::obj(j)?;
    ItemPredicate::from_value(&j.to_value()).map_err(|e| ParseError::new(e.0))
}
