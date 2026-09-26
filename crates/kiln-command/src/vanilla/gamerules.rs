//! Game rule value types. Names come from the `minecraft:game_rule` registry; rules not listed
//! here take booleans.

use crate::arguments::ArgumentType;

/// Integer rules with their bounds.
const INT_RULES: &[(&str, i32, i32)] = &[
    ("minecraft:fire_spread_radius_around_player", -1, i32::MAX),
    ("minecraft:max_block_modifications", 1, i32::MAX),
    ("minecraft:max_command_forks", 0, i32::MAX),
    ("minecraft:max_command_sequence_length", 0, i32::MAX),
    ("minecraft:max_entity_cramming", 0, i32::MAX),
    ("minecraft:max_minecart_speed", 1, 1000),
    ("minecraft:max_snow_accumulation_height", 0, 8),
    ("minecraft:players_nether_portal_creative_delay", 0, i32::MAX),
    ("minecraft:players_nether_portal_default_delay", 0, i32::MAX),
    ("minecraft:players_sleeping_percentage", 0, i32::MAX),
    ("minecraft:random_tick_speed", 0, i32::MAX),
    ("minecraft:respawn_radius", 0, i32::MAX),
];

/// All game rules in registry order.
pub fn rules() -> &'static [&'static str] {
    kiln_data::builtin_entries("minecraft:game_rule").unwrap_or(&[])
}

/// The argument type of a rule's value.
pub fn value_type(rule: &str) -> ArgumentType {
    match INT_RULES.iter().find(|(name, ..)| *name == rule) {
        Some(&(_, min, max)) => ArgumentType::integer_range(min, max),
        None => ArgumentType::Bool,
    }
}

/// `Identifier.toShortString`: the path for the `minecraft` namespace.
pub fn short_name(rule: &str) -> &str {
    rule.strip_prefix("minecraft:").unwrap_or(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_rules_exist() {
        for (name, ..) in INT_RULES {
            assert!(rules().contains(name), "{name}");
        }
        assert_eq!(value_type("minecraft:keep_inventory"), ArgumentType::Bool);
        assert_eq!(short_name("minecraft:pvp"), "pvp");
    }
}
