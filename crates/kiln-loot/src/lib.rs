//! Loot tables, predicates and item modifiers, loaded at run time from a datapack and evaluated
//! the way vanilla 26.3 does.
//!
//! - [`LootData::load`]: every `loot_table/`, `predicate/`, `item_modifier/`, `slot_source/`,
//!   `context_int_provider/` and `context_float_provider/` file, with the enchantment
//!   definitions and tags (in vanilla's order) they refer to. Unknown types fail by name.
//! - [`LootContext`]: the parameters and world queries an evaluation reads (entities, block,
//!   tool, explosion, weather...). Entity, damage source and location predicates are decoded
//!   here and evaluated by the context implementation.
//! - [`LootData::random_items`] / [`LootData::fill`] / [`LootData::random_items_raw`]: vanilla's
//!   `getRandomItems`, `fill` and `getRandomItemsRaw`; [`LootTable::random`] picks the random
//!   source (explicit seed, [`RandomSequences`], or the level's random) the way
//!   `LootContext.Builder` does.
//!
//! Randomness goes through `kiln-javamath`'s Java-exact sources, so a given seed and context
//! give vanilla's items.

pub mod condition;
pub mod context;
pub mod data;
pub mod effects;
pub mod enchant;
pub mod entry;
pub mod eval;
pub mod function;
pub mod json;
pub mod number;
pub mod parse;
pub mod predicate;
pub mod provider;
pub mod random;
pub mod slot;
pub mod stack;
pub mod table;
pub mod tags;
pub mod text;
pub mod trade;

pub use condition::Condition;
pub use context::{EmptyContext, EntityTarget, LootContext, Source};
pub use data::{JukeboxSong, Kind, LoadError, LootData};
pub use eval::Eval;
pub use function::Function;
pub use json::Json;
pub use parse::ParseError;
pub use random::{RandomSequences, Xoroshiro};
pub use table::{LootPool, LootRandom, LootTable, TableRef};
