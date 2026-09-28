//! Item stacks with data components, vanilla-exact for Minecraft 26.3.
//!
//! - [`ItemStack`] / [`DataComponentPatch`]: the three network codecs (`OPTIONAL_STREAM_CODEC`,
//!   `STREAM_CODEC`, `OPTIONAL_UNTRUSTED_STREAM_CODEC`) and the persistent `ItemStack.CODEC`
//!   (NBT in playerdata and chunks).
//! - [`Component`]: one typed value per data component type, from vanilla's own codecs.
//! - [`HashedStack`] and [`hash`]: the per-component hashes clients send in `container_click`.
//! - [`default_components`]: every item's prototype components, generated from the jar.
//!
//! Registry references are network ids (`minecraft:item` ids for items); synchronized
//! registries use the vanilla datapack's ids.

#[path = "gen/mod.rs"]
mod generated;

mod enums;

pub mod component;
pub mod defaults;
pub mod hash;
pub mod hashed;
pub mod holder;
pub mod ident;
mod javamap;
pub mod patch;
pub mod registry;
pub mod stack;
pub mod text;
pub mod trading;
pub mod value;
pub mod wire;

pub use component::{Component, ComponentId, ComponentValue, Key, keys};
pub use defaults::{ComponentMap, default_components};
pub use hash::hash;
pub use hashed::HashedStack;
pub use holder::{Holder, HolderSet};
pub use ident::Identifier;
pub use patch::DataComponentPatch;
pub use stack::{ItemStack, ItemStackTemplate};
pub use text::Text;
pub use value::{DataError, Value};
