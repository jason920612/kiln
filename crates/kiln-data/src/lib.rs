//! Version-pinned game data generated from the vanilla server jar (`cargo xtask codegen`).

#[path = "gen/version.rs"]
pub mod version;

#[path = "gen/packets.rs"]
pub mod packets;

#[path = "gen/blocks.rs"]
pub mod blocks;
pub mod blocks_types;
pub mod block_props;
pub mod block_logic;

#[path = "gen/registries.rs"]
pub mod registries;

#[path = "gen/entities.rs"]
pub mod entities;

#[path = "gen/game_rules.rs"]
pub mod game_rules;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameRuleDefault {
    Bool(bool),
    Int(i32),
}

/// Default value of a game rule such as `minecraft:player_movement_check`.
pub fn game_rule_default(rule: &str) -> Option<GameRuleDefault> {
    game_rules::GAME_RULES.iter().find(|(r, _)| *r == rule).map(|&(_, d)| d)
}

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
