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
pub mod block_sounds;

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

pub mod live;

/// Index of `entry` in a synchronized registry, i.e. its network id (the registry as the
/// running server has it, see [`live`]).
pub fn synced_id(registry: &str, entry: &str) -> Option<i32> {
    live::synced_id(registry, entry)
}

/// Protocol id of `entry` in a built-in registry such as `minecraft:entity_type`.
pub fn builtin_id(registry: &str, entry: &str) -> Option<i32> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    // An index per registry (the first entry of a name wins, as the scan it replaces).
    static INDEX: OnceLock<HashMap<&'static str, HashMap<&'static str, i32>>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        registries::BUILTIN
            .iter()
            .map(|(r, entries)| {
                let mut ids = HashMap::with_capacity(entries.len());
                for (i, e) in entries.iter().enumerate() {
                    ids.entry(*e).or_insert(i as i32);
                }
                (*r, ids)
            })
            .collect()
    });
    index.get(registry)?.get(entry).copied()
}

/// Entries of a built-in registry, indexed by protocol id.
pub fn builtin_entries(registry: &str) -> Option<&'static [&'static str]> {
    registries::BUILTIN.iter().find(|(r, _)| *r == registry).map(|(_, e)| *e)
}

#[path = "gen/dimension_types.rs"]
pub mod dimension_types;

/// When a `bed_rule` lets a player sleep or set the respawn point (`BedRule.Rule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedWhen {
    Always,
    /// The sky is dark outside (`Level.isDarkOutside`).
    WhenDark,
    Never,
}

impl BedWhen {
    /// The name in the dimension type JSON (`always`, `when_dark`, `never`).
    pub fn parse(name: &str) -> Option<BedWhen> {
        match name {
            "always" => Some(BedWhen::Always),
            "when_dark" => Some(BedWhen::WhenDark),
            "never" => Some(BedWhen::Never),
            _ => None,
        }
    }
}

/// A dimension type (`minecraft:dimension_type`): what the server simulates with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimensionType {
    pub name: &'static str,
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub coordinate_scale: f64,
    pub has_skylight: bool,
    pub has_ceiling: bool,
    pub has_fixed_time: bool,
    pub has_ender_dragon_fight: bool,
    pub ambient_light: f32,
    /// The world clock the dimension's time follows, if any.
    pub default_clock: Option<&'static str>,
    /// Block tag fire burns forever on.
    pub infiniburn: &'static str,
    /// `minecraft:gameplay/fast_lava` (the Nether's "ultrawarm" lava).
    pub fast_lava: bool,
    /// `minecraft:gameplay/water_evaporates`.
    pub water_evaporates: bool,
    pub respawn_anchor_works: bool,
    /// Beds set the respawn point (`bed_rule.can_set_spawn` is not `never`).
    pub bed_sets_spawn: bool,
    /// Using a bed destroys it (`bed_rule.destroy_on_use`: it explodes).
    pub bed_explodes: bool,
    pub monster_spawn_block_light_limit: i64,
    /// `bed_rule.can_sleep` and `bed_rule.can_set_spawn`.
    pub bed_can_sleep: BedWhen,
    pub bed_can_set_spawn: BedWhen,
    /// `bed_rule.error_message`'s translation key, shown when sleeping is not possible.
    pub bed_error: Option<&'static str>,
    /// `minecraft:gameplay/piglins_zombify` (true unless a type says false).
    pub piglins_zombify: bool,
    /// `minecraft:gameplay/can_start_raid` (true unless a type says false).
    pub can_start_raid: bool,
    /// `minecraft:gameplay/nether_portal_spawns_piglin`.
    pub nether_portal_spawns_piglin: bool,
}

/// A dimension type by name, e.g. `minecraft:the_nether`: the loaded data packs' (see [`levels`]), else vanilla's.
pub fn dimension_type(name: &str) -> Option<&'static DimensionType> {
    levels::find_type(name)
}

pub mod levels;

#[path = "gen/map_colors.rs"]
#[allow(dead_code, reason = "generated: the count of colours is part of the table")]
mod map_colors;

/// `BlockState.getMapColor`: the `MapColor` id (0 is none) of block state `state`.
pub fn map_color(state: u16) -> u8 {
    let runs = map_colors::MAP_COLOR_RUNS;
    let i = runs.partition_point(|(first, _)| *first <= state);
    runs[i.saturating_sub(1)].1
}

/// `MapColor.col` (RGB) of map color `id`.
pub fn map_color_rgb(id: u8) -> u32 {
    map_colors::MAP_COLOR_RGB.get(id as usize).map_or(0, |(_, c)| *c)
}
