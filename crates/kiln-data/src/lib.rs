//! Version-pinned game data generated from the vanilla server jar (`cargo xtask codegen`).

#[path = "gen/version.rs"]
pub mod version;

#[path = "gen/packets.rs"]
pub mod packets;

#[path = "gen/blocks.rs"]
pub mod blocks;

#[path = "gen/registries.rs"]
pub mod registries;

/// Index of `entry` in a synchronized registry, i.e. its network id.
pub fn synced_id(registry: &str, entry: &str) -> Option<i32> {
    let (_, entries) = registries::SYNCHRONIZED.iter().find(|(r, _)| *r == registry)?;
    entries.iter().position(|e| *e == entry).map(|i| i as i32)
}
