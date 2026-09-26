//! Version-pinned game data generated from the vanilla server jar (`cargo xtask codegen`).

#[path = "gen/version.rs"]
pub mod version;

#[path = "gen/packets.rs"]
pub mod packets;

#[path = "gen/blocks.rs"]
pub mod blocks;
pub mod blocks_types;

#[path = "gen/registries.rs"]
pub mod registries;

/// Index of `entry` in a synchronized registry, i.e. its network id.
pub fn synced_id(registry: &str, entry: &str) -> Option<i32> {
    let (_, entries) = registries::SYNCHRONIZED.iter().find(|(r, _)| *r == registry)?;
    entries.iter().position(|e| *e == entry).map(|i| i as i32)
}

/// Protocol id of `entry` in a built-in registry such as `minecraft:entity_type`.
pub fn builtin_id(registry: &str, entry: &str) -> Option<i32> {
    let (_, entries) = registries::BUILTIN.iter().find(|(r, _)| *r == registry)?;
    entries.iter().position(|e| *e == entry).map(|i| i as i32)
}

/// Entries of a built-in registry, indexed by protocol id.
pub fn builtin_entries(registry: &str) -> Option<&'static [&'static str]> {
    registries::BUILTIN.iter().find(|(r, _)| *r == registry).map(|(_, e)| *e)
}
