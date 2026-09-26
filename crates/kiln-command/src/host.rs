//! What the simulation implements for commands to run: the command source ([`Source`]), the
//! world as seen by selectors ([`SelectorWorld`]) and the effects of the built-in commands
//! ([`Host`]). Commands parse arguments, resolve selectors and coordinates, validate, call the
//! host for the effect and send vanilla's feedback; the host only mutates game state.

use crate::error::CommandError;
use crate::selector::SelectorWorld;
use crate::text::{Arg, Text};
use crate::tr;
use crate::types::{Difficulty, GameMode, Identifier, ItemInput};
use uuid::Uuid;

/// The executor of a command.
pub trait Source {
    /// Permission level 0-4: all, moderators, gamemasters, admins, owners.
    fn permission_level(&self) -> u8;
    /// Online player names, for suggestions.
    fn player_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Dimension ids, for suggestions.
    fn dimensions(&self) -> Vec<String> {
        ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"].map(String::from).to_vec()
    }
}

/// A player identity (`NameAndId`), as `op` and `deop` take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub uuid: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weather {
    Clear,
    Rain,
    Thunder,
}

impl Weather {
    pub fn name(self) -> &'static str {
        match self {
            Weather::Clear => "clear",
            Weather::Rain => "rain",
            Weather::Thunder => "thunder",
        }
    }
}

/// A resolved teleport (`TeleportCommand.performTeleport`). `pos` is absolute; `relative`
/// marks axes given with `~`/`^`, which vanilla sends as relative movement when the entity
/// stays in its dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct Teleport {
    pub dimension: String,
    pub pos: [f64; 3],
    pub relative: [bool; 3],
    /// New `[yaw, pitch]` (already wrapped), or `None` to keep the entity's rotation.
    pub rotation: Option<[f32; 2]>,
    /// Whether yaw and pitch were given relative (`~`).
    pub relative_rotation: [bool; 2],
    /// A point to face after moving (`facing <pos>` / `facing entity <target> [anchor]`);
    /// see [`look_at`].
    pub facing: Option<[f64; 3]>,
}

/// `CommandSourceStack.facing` / `Entity.lookAt`: the `[yaw, pitch]` that faces `to` from
/// `from`, with `Mth.atan2` for bit-exact angles.
pub fn look_at(from: [f64; 3], to: [f64; 3]) -> [f32; 2] {
    use crate::coords::{mth_atan2, wrap_degrees};
    const RAD_TO_DEG: f64 = 57.2957763671875;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    let pitch = wrap_degrees((-(mth_atan2(dy, horizontal) * RAD_TO_DEG)) as f32);
    let yaw = wrap_degrees((mth_atan2(dz, dx) * RAD_TO_DEG) as f32 - 90.0);
    [yaw, pitch]
}

/// A respawn point (`LevelData.RespawnData`); yaw is wrapped and pitch clamped to ±90.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPoint {
    pub dimension: String,
    pub pos: [i32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// What `/time` asks of a world clock; the host applies it and sends the `commands.time.*`
/// feedback, since the result depends on clock state.
#[derive(Debug, Clone, PartialEq)]
pub enum TimeAction {
    Add(i32),
    Set(i32),
    SetMarker(Identifier),
    QueryGameTime,
    QueryTime,
    QueryTimeline { timeline: Identifier, repetitions: bool },
    Pause,
    Resume,
    Rate(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameRuleValue {
    Bool(bool),
    Int(i32),
}

impl GameRuleValue {
    /// `GameRule.serialize`.
    pub fn serialize(self) -> String {
        match self {
            GameRuleValue::Bool(b) => b.to_string(),
            GameRuleValue::Int(v) => v.to_string(),
        }
    }

    /// `GameRule.getCommandResult`.
    pub fn command_result(self) -> i32 {
        match self {
            GameRuleValue::Bool(b) => b as i32,
            GameRuleValue::Int(v) => v,
        }
    }
}

/// The chat types of `say`, `me` and `msg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    Say,
    Emote,
    MsgIncoming,
    MsgOutgoing,
}

impl ChatKind {
    /// Id in the `minecraft:chat_type` registry, for `disguised_chat`.
    pub fn id(self) -> &'static str {
        match self {
            ChatKind::Say => "minecraft:say_command",
            ChatKind::Emote => "minecraft:emote_command",
            ChatKind::MsgIncoming => "minecraft:msg_command_incoming",
            ChatKind::MsgOutgoing => "minecraft:msg_command_outgoing",
        }
    }
}

/// A chat message sent through a chat type: `sender` is the source's name and `target` the
/// recipient's name for outgoing whispers.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub kind: ChatKind,
    pub sender: Text,
    pub target: Option<Text>,
    pub content: Text,
}

impl ChatMessage {
    /// The chat type's decoration as a system message, for hosts that do not use `disguised_chat`.
    pub fn to_text(&self) -> Text {
        let content = Arg::Text(self.content.clone());
        match self.kind {
            ChatKind::Say => tr!("chat.type.announcement", self.sender.clone(), content),
            ChatKind::Emote => tr!("chat.type.emote", self.sender.clone(), content),
            ChatKind::MsgIncoming => {
                tr!("commands.message.display.incoming", self.sender.clone(), content).color("gray").italic()
            }
            ChatKind::MsgOutgoing => {
                let target = self.target.clone().unwrap_or_default();
                tr!("commands.message.display.outgoing", target, content).color("gray").italic()
            }
        }
    }
}

/// Effects of the built-in commands. `Self::Entity` handles come from selectors.
pub trait Host: SelectorWorld {
    /// The source's display name (the player's name, `Server` for the console).
    fn source_name(&self) -> Text;
    /// The source's `[yaw, pitch]`, for `~` rotations and `^` coordinates.
    fn source_rotation(&self) -> [f32; 2];
    /// `sendSuccess`: feedback to the source; `broadcast` also informs operators and the log.
    fn send_success(&mut self, text: Text, broadcast: bool);
    /// A system message to one player.
    fn send_system(&mut self, player: &Self::Entity, text: Text);
    /// `say` and `me`: to every player.
    fn broadcast_chat(&mut self, message: ChatMessage);
    /// An incoming whisper, to one player.
    fn send_chat(&mut self, player: &Self::Entity, message: ChatMessage);
    /// An outgoing whisper, echoed to the source.
    fn send_chat_to_source(&mut self, message: ChatMessage);
    /// The `send_command_feedback` game rule.
    fn send_command_feedback(&self) -> bool {
        true
    }

    /// Moves `entity` (bounds already checked); may refuse with an error.
    fn teleport(&mut self, entity: &Self::Entity, to: &Teleport) -> Result<(), CommandError>;
    /// Returns whether the mode changed.
    fn set_game_mode(&mut self, player: &Self::Entity, mode: GameMode) -> bool;
    fn kill(&mut self, entity: &Self::Entity);
    /// Gives `count` items (already checked against 100 stacks), dropping what does not fit.
    fn give(&mut self, player: &Self::Entity, item: &ItemInput, count: i32);
    fn max_stack_size(&self, _item: &ItemInput) -> i32 {
        64
    }
    fn kick(&mut self, player: &Self::Entity, reason: Text);
    fn max_players(&self) -> usize;
    /// Profile lookup by name (online players, then the profile cache).
    fn find_profile(&mut self, name: &str) -> Option<Profile>;
    fn is_operator(&self, profile: &Profile) -> bool;
    fn operator_names(&self) -> Vec<String>;
    fn set_operator(&mut self, profile: &Profile, op: bool);
    fn difficulty(&self) -> Difficulty;
    fn set_difficulty(&mut self, difficulty: Difficulty);
    /// Sets the weather for `duration` ticks, or a random vanilla duration; returns the duration.
    fn set_weather(&mut self, weather: Weather, duration: Option<i32>) -> i32;
    /// Applies a `/time` action to `clock` (`None`: the source dimension's default clock).
    fn time(&mut self, clock: Option<&Identifier>, action: &TimeAction) -> Result<i32, CommandError>;
    /// Time marker ids of `clock`, for suggestions.
    fn time_markers(&self, _clock: Option<&Identifier>) -> Vec<String> {
        Vec::new()
    }
    /// Timeline ids valid for `clock`, for suggestions.
    fn timelines(&self, _clock: Option<&Identifier>) -> Vec<String> {
        Vec::new()
    }
    /// Current value of `rule`, a full `minecraft:game_rule` id such as `minecraft:keep_inventory`.
    fn game_rule(&self, rule: &str) -> GameRuleValue;
    /// Sets `rule` (a full id); the value was checked against the rule's type and bounds.
    fn set_game_rule(&mut self, rule: &str, value: GameRuleValue);
    fn seed(&self) -> i64;
    fn stop(&mut self);
    /// `Level.isInSpawnableBounds`.
    fn is_in_spawnable_bounds(&self, pos: [i32; 3]) -> bool {
        let [x, y, z] = pos;
        (-30_000_000..30_000_000).contains(&x)
            && (-30_000_000..30_000_000).contains(&z)
            && (-20_000_000..20_000_000).contains(&y)
    }
    fn set_spawn_point(&mut self, player: &Self::Entity, spawn: &SpawnPoint);
    fn set_world_spawn(&mut self, spawn: &SpawnPoint) -> Result<(), CommandError>;
    /// `/kiln tick`: tick timing report lines.
    fn kiln_tick(&mut self) -> Vec<Text>;
    /// `/kiln regions`: region report lines.
    fn kiln_regions(&mut self) -> Vec<Text>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_at_directions() {
        // Vanilla multiplies by 57.2957763671875, slightly below 180/pi: due south is not 0.
        assert_eq!(look_at([0.0; 3], [0.0, 0.0, 5.0]), [-7.6293945e-6, -0.0]);
        let [yaw, pitch] = look_at([0.0; 3], [-5.0, 0.0, 0.0]);
        assert!((yaw - 90.0).abs() < 1e-4 && pitch == 0.0);
        let [yaw, pitch] = look_at([0.0; 3], [1.0, 1.0, 0.0]);
        assert!((yaw + 90.0).abs() < 1e-4);
        assert!((pitch + 45.0).abs() < 1e-4);
    }

    #[test]
    fn chat_decorations() {
        let m = ChatMessage {
            kind: ChatKind::MsgOutgoing,
            sender: Text::literal("A"),
            target: Some(Text::literal("B")),
            content: Text::literal("hi"),
        };
        assert_eq!(m.to_text().to_plain(), "commands.message.display.outgoing[B, hi]");
        assert_eq!(m.to_text().style.italic, Some(true));
        assert_eq!(ChatKind::Emote.id(), "minecraft:emote_command");
    }
}
