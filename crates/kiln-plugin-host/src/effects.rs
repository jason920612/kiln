//! Effects: what plugins ask the game to do (messages, HUD, teleports, items, menus,
//! entities, blocks).
//!
//! A call that returns normally commits its effects to the host's outbox; the embedder takes
//! them at a serial point ([`PluginRuntime::take_effects`](crate::PluginRuntime::take_effects)),
//! applies them in the order handed out (tick, acting player, call order: the same for any
//! thread count and region layout) and reports each outcome with
//! [`PluginRuntime::effect_done`](crate::PluginRuntime::effect_done), which turns it into an
//! `op-result` for the plugin the next tick.

use crate::Span;
use std::sync::Arc;

/// What the host knows of a player when an event happens (`event.info`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInfo {
    /// Level id (index into the registries' levels).
    pub level: u32,
    pub pos: [f64; 3],
    /// Yaw, pitch.
    pub rot: [f32; 2],
    pub health: f32,
    pub food: u32,
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: u8,
    pub on_ground: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    pub flying: bool,
    /// The main hand's item id, none when empty.
    pub held: Option<u32>,
    pub held_count: u32,
}

/// A player online at the start of the tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnlinePlayer {
    pub uuid: u128,
    pub name: String,
    pub level: u32,
}

/// An item to create.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemSpec {
    /// Resource key (`minecraft:diamond`).
    pub item: String,
    pub count: u32,
    pub name: Option<Vec<Span>>,
    pub lore: Vec<Vec<Span>>,
    /// The tag as stored: `<plugin id>:<tag>`.
    pub tag: Option<String>,
    pub model: Option<String>,
    pub glint: bool,
}

/// A chest-style menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuSpec {
    /// The id as stored: `<plugin id>:<id>`.
    pub id: String,
    pub title: Vec<Span>,
    pub rows: u8,
    pub items: Vec<(u8, ItemSpec)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnSpec {
    /// Entity type key.
    pub kind: String,
    pub level: u32,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub name: Option<Vec<Span>>,
    pub no_ai: bool,
    pub invulnerable: bool,
    pub silent: bool,
    pub no_gravity: bool,
    /// Derived from the server seed, the plugin and the call.
    pub uuid: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockChange {
    pub pos: [i32; 3],
    /// A block state string (`minecraft:oak_stairs[facing=north]`).
    pub state: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EffectKind {
    /// Chat to one player or everyone.
    Message { to: Option<u128>, text: Vec<Span> },
    Title { to: u128, title: Vec<Span>, subtitle: Vec<Span>, fade_in: u32, stay: u32, fade_out: u32 },
    ActionBar { to: u128, text: Vec<Span> },
    Sidebar { to: u128, title: Vec<Span>, lines: Vec<Vec<Span>> },
    ClearSidebar { to: u128 },
    /// `id` is `<plugin id>:<id>`; `color` and `style` are the vanilla ordinals.
    Bossbar { to: u128, id: String, text: Vec<Span>, progress: f32, color: u8, style: u8 },
    ClearBossbar { to: u128, id: String },
    Teleport { who: u128, level: u32, pos: [f64; 3], rot: [f32; 2] },
    GameMode { who: u128, mode: u8 },
    Heal { who: u128 },
    Kick { who: u128, reason: Vec<Span> },
    Give { who: u128, item: ItemSpec },
    Take { who: u128, item: String, count: u32 },
    Clear { who: u128 },
    OpenMenu { who: u128, menu: MenuSpec },
    /// `menu` is `<plugin id>:<id>`.
    SetSlot { who: u128, menu: String, slot: u8, item: Option<ItemSpec> },
    CloseMenu { who: u128 },
    Spawn(SpawnSpec),
    Remove { level: u32, uuid: u128 },
    SetBlocks { level: u32, changes: Vec<BlockChange> },
}

/// A committed effect with the plugin it came from and the ticket its outcome is reported
/// under.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub plugin: usize,
    pub plugin_id: Arc<str>,
    pub generation: u32,
    /// The acting player (0: none).
    pub source: u128,
    /// 0 for effects that have no outcome (messages).
    pub ticket: u64,
    pub kind: EffectKind,
}
