//! What the simulation implements for commands to run: the command source ([`Source`]) with
//! its [`SourceStack`], the world as seen by selectors ([`SelectorWorld`]) and the effects of
//! the built-in commands ([`Host`]). Commands parse arguments, resolve selectors and
//! coordinates, validate, call the host for the effect and send vanilla's feedback; the host
//! only mutates game state.

use crate::blocks::{BlockInput, UpdateFlags};
use crate::bossbar::BossBars;
use crate::functions::{DataPacks, FunctionLibrary, TimerQueue};
use bytes::Bytes;
use crate::coords::{Coordinates, wrap_degrees};
use crate::error::CommandError;
use crate::nbt_path::CommandStorage;
use crate::scoreboard::Scoreboard;
use crate::selector::{SelectorTarget, SelectorWorld};
use crate::text::{Arg, Text};
use crate::tr;
use crate::types::{Anchor, Difficulty, GameMode, Heightmap, Identifier, ItemInput};
use kiln_proto::nbt::Tag;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// The executor of a command. The dispatcher replaces the [`stack`](Source::stack) for each
/// source a redirect modifier (`execute as`, `at`, ...) produces and restores it afterwards.
pub trait Source {
    /// Entity handles, as selectors find them; [`NoEntity`](crate::selector::NoEntity) for
    /// sources without entities.
    type Entity: SelectorTarget + Clone;
    /// Permission level 0-4: all, moderators, gamemasters, admins, owners.
    fn permission_level(&self) -> u8;
    /// The level commands check: [`permission_level`](Self::permission_level) capped by the
    /// stack (function bodies run with at most level 2).
    fn permission(&self) -> u8
    where
        Self: Sized,
    {
        self.permission_level().min(self.stack().max_permission)
    }
    /// Where, how and as whom the command currently runs.
    fn stack(&self) -> &SourceStack<Self>
    where
        Self: Sized;
    fn stack_mut(&mut self) -> &mut SourceStack<Self>
    where
        Self: Sized;
    /// Online player names, for suggestions.
    fn player_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Ids of a data pack registry (`minecraft:advancement`, `minecraft:recipe`), for
    /// `resource_key` suggestions.
    fn registry_ids(&self, _registry: &str) -> Vec<String> {
        Vec::new()
    }
    /// Dimension ids, for suggestions.
    fn dimensions(&self) -> Vec<String> {
        ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"].map(String::from).to_vec()
    }
    /// The `max_command_forks` game rule: sources one modifier stage may produce.
    fn fork_limit(&self) -> usize {
        65536
    }
    /// The `max_command_sequence_length` game rule: modifier stages plus executions one
    /// command may cost.
    fn command_limit(&self) -> i32 {
        65536
    }

    /// Where the command runs (`CommandSourceStack.getPosition`).
    fn origin(&self) -> [f64; 3]
    where
        Self: Sized,
    {
        self.stack().position
    }
    /// The dimension the command runs in.
    fn dimension(&self) -> &str
    where
        Self: Sized,
    {
        &self.stack().dimension
    }
    /// The executing entity (`@s`), if any.
    fn source_entity(&self) -> Option<Self::Entity>
    where
        Self: Sized,
    {
        self.stack().entity.clone()
    }
    /// The source's `[yaw, pitch]`, for `~` rotations and `^` coordinates.
    fn source_rotation(&self) -> [f32; 2]
    where
        Self: Sized,
    {
        self.stack().rotation
    }
    /// The source's display name (the entity's name, `Server` for the console).
    fn source_name(&self) -> Text
    where
        Self: Sized,
    {
        self.stack().display_name()
    }
}

/// `CommandResultCallback`: told the outcome of each execution (`execute store`).
pub type ResultCallback<S> = Arc<dyn Fn(&mut S, bool, i32) + Send + Sync>;

/// A function call's frame (`Frame`): what `return` reported, and whether the rest of the
/// function was discarded. Depth 0 is a top-level command.
#[derive(Debug, Default)]
pub struct Frame {
    pub depth: u32,
    discarded: AtomicBool,
    /// `(success, value)` per `returnSuccess` / `returnFailure`, in order.
    outcomes: Mutex<Vec<(bool, i32)>>,
}

impl Frame {
    pub fn new(depth: u32) -> Arc<Frame> {
        Arc::new(Frame { depth, ..Frame::default() })
    }

    /// `Frame.discard`: the function's remaining lines do not run.
    pub fn discard(&self) {
        self.discarded.store(true, Ordering::Relaxed);
    }

    pub fn is_discarded(&self) -> bool {
        self.discarded.load(Ordering::Relaxed)
    }

    /// `returnSuccess` (`success`) or `returnFailure`.
    pub fn report(&self, success: bool, value: i32) {
        self.outcomes.lock().expect("frame lock").push((success, value));
    }

    pub fn take_outcomes(&self) -> Vec<(bool, i32)> {
        std::mem::take(&mut *self.outcomes.lock().expect("frame lock"))
    }
}

/// `CommandSourceStack` without the server: the executing entity, position, rotation,
/// dimension, anchor and result callbacks. Feedback always goes to the original source
/// (the host's concern); `execute` derives new stacks with the `with_*` methods.
pub struct SourceStack<S: Source> {
    pub entity: Option<S::Entity>,
    pub position: [f64; 3],
    /// `[yaw, pitch]`.
    pub rotation: [f32; 2],
    pub dimension: String,
    pub anchor: Anchor,
    /// The display name when no entity executes (`Server` for the console).
    pub name: Text,
    /// Run in order after each execution (`CommandResultCallback.chain`).
    pub callbacks: Vec<ResultCallback<S>>,
    /// The function call this runs in.
    pub frame: Arc<Frame>,
    /// Commands left in this execution (`ExecutionContext.commandQuota`), shared by every
    /// source and nested function.
    pub quota: Arc<AtomicI32>,
    /// `withSuppressedOutput`: feedback is dropped (function bodies).
    pub silent: bool,
    /// `withMaximumPermission`: the permission level is capped at this.
    pub max_permission: u8,
    /// Under `return run`: the command's result is the frame's return value.
    pub returning: bool,
    /// The first pass of a [custom executor](crate::Builder::custom).
    pub preparing: bool,
}

impl<S: Source> Clone for SourceStack<S> {
    fn clone(&self) -> Self {
        SourceStack {
            entity: self.entity.clone(),
            position: self.position,
            rotation: self.rotation,
            dimension: self.dimension.clone(),
            anchor: self.anchor,
            name: self.name.clone(),
            callbacks: self.callbacks.clone(),
            frame: self.frame.clone(),
            quota: self.quota.clone(),
            silent: self.silent,
            max_permission: self.max_permission,
            returning: self.returning,
            preparing: self.preparing,
        }
    }
}

impl<S: Source> std::fmt::Debug for SourceStack<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("SourceStack")
            .field("entity", &self.entity.as_ref().map(SelectorTarget::name))
            .field("position", &self.position)
            .field("rotation", &self.rotation)
            .field("dimension", &self.dimension)
            .field("anchor", &self.anchor)
            .field("callbacks", &self.callbacks.len())
            .finish()
    }
}

impl<S: Source> SourceStack<S> {
    /// A stack without an entity, e.g. the server console at the world spawn.
    pub fn new(name: Text, dimension: &str, position: [f64; 3]) -> Self {
        SourceStack {
            entity: None,
            position,
            rotation: [0.0, 0.0],
            dimension: dimension.to_owned(),
            anchor: Anchor::Feet,
            name,
            callbacks: Vec::new(),
            frame: Frame::new(0),
            quota: Arc::new(AtomicI32::new(i32::MAX)),
            silent: false,
            max_permission: 4,
            returning: false,
            preparing: false,
        }
    }

    /// A stack executing as `entity` at its position and rotation (a player's own source).
    pub fn of_entity(entity: S::Entity) -> Self {
        SourceStack {
            position: entity.position(),
            rotation: entity.rotation(),
            dimension: entity.dimension().to_owned(),
            name: entity.display_name(),
            entity: Some(entity),
            anchor: Anchor::Feet,
            callbacks: Vec::new(),
            frame: Frame::new(0),
            quota: Arc::new(AtomicI32::new(i32::MAX)),
            silent: false,
            max_permission: 4,
            returning: false,
            preparing: false,
        }
    }

    /// `FunctionCommand.modifySenderForExecution`: silent, at most permission level 2.
    pub fn for_function_body(mut self) -> Self {
        self.silent = true;
        self.max_permission = self.max_permission.min(2);
        self
    }

    /// `clearCallbacks`.
    pub fn without_callbacks(mut self) -> Self {
        self.callbacks.clear();
        self
    }

    /// `getDisplayName`: the entity's name once one executes.
    pub fn display_name(&self) -> Text {
        self.entity.as_ref().map_or_else(|| self.name.clone(), SelectorTarget::display_name)
    }

    /// `withEntity`: runs as `entity` without moving.
    pub fn with_entity(mut self, entity: S::Entity) -> Self {
        self.entity = Some(entity);
        self
    }

    pub fn with_position(mut self, position: [f64; 3]) -> Self {
        self.position = position;
        self
    }

    /// `[yaw, pitch]`.
    pub fn with_rotation(mut self, rotation: [f32; 2]) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// `withLevel`: x and z scale by the dimensions' coordinate scales (1/8 into the nether).
    pub fn with_dimension(mut self, dimension: &str) -> Self {
        if self.dimension != dimension {
            let scale = coordinate_scale(&self.dimension) / coordinate_scale(dimension);
            self.position = [self.position[0] * scale, self.position[1], self.position[2] * scale];
            self.dimension = dimension.to_owned();
        }
        self
    }

    /// `withCallback(callback, CommandResultCallback::chain)`.
    pub fn with_callback(mut self, callback: ResultCallback<S>) -> Self {
        self.callbacks.push(callback);
        self
    }

    /// `EntityAnchorArgument.Anchor.apply(CommandSourceStack)`: the position the anchor
    /// selects, the eyes of the executing entity or the position itself.
    pub fn anchor_position(&self) -> [f64; 3] {
        match (&self.entity, self.anchor) {
            (Some(e), Anchor::Eyes) => {
                let [x, y, z] = self.position;
                [x, y + e.eye_height(), z]
            }
            _ => self.position,
        }
    }

    /// `facing(Vec3)`: turns toward `target` from the anchor position.
    pub fn facing(self, target: [f64; 3]) -> Self {
        let rotation = look_at(self.anchor_position(), target);
        self.with_rotation(rotation)
    }

    /// World position of `c` (`Coordinates.getPosition`): `^` offsets start at the anchor.
    pub fn resolve(&self, c: &Coordinates) -> [f64; 3] {
        match c {
            Coordinates::World(_) => c.position(self.position, self.rotation),
            Coordinates::Local { .. } => c.position(self.anchor_position(), self.rotation),
        }
    }

    /// `Coordinates.getBlockPos`: the block containing [`resolve`](Self::resolve).
    pub fn resolve_block(&self, c: &Coordinates) -> [i32; 3] {
        self.resolve(c).map(|v| v.floor() as i32)
    }
}

/// `DimensionType.coordinateScale` of the vanilla dimension types.
pub fn coordinate_scale(dimension: &str) -> f64 {
    if dimension == "minecraft:the_nether" { 8.0 } else { 1.0 }
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
    const RAD_TO_DEG: f64 = 57.2957763671875;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    let pitch = wrap_degrees((-(crate::coords::mth_atan2(dy, horizontal) * RAD_TO_DEG)) as f32);
    let yaw = wrap_degrees((crate::coords::mth_atan2(dz, dx) * RAD_TO_DEG) as f32 - 90.0);
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

/// The chat types of `say`, `me`, `msg` and `teammsg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    Say,
    Emote,
    MsgIncoming,
    MsgOutgoing,
    TeamMsgIncoming,
    TeamMsgOutgoing,
}

impl ChatKind {
    /// Id in the `minecraft:chat_type` registry, for `disguised_chat`.
    pub fn id(self) -> &'static str {
        match self {
            ChatKind::Say => "minecraft:say_command",
            ChatKind::Emote => "minecraft:emote_command",
            ChatKind::MsgIncoming => "minecraft:msg_command_incoming",
            ChatKind::MsgOutgoing => "minecraft:msg_command_outgoing",
            ChatKind::TeamMsgIncoming => "minecraft:team_msg_command_incoming",
            ChatKind::TeamMsgOutgoing => "minecraft:team_msg_command_outgoing",
        }
    }
}

/// A chat message sent through a chat type: `sender` is the source's name and `target` the
/// recipient's name for outgoing whispers (the team's name for team messages).
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
            ChatKind::TeamMsgIncoming | ChatKind::TeamMsgOutgoing => {
                let key = if self.kind == ChatKind::TeamMsgIncoming { "chat.type.team.text" } else { "chat.type.team.sent" };
                let target = self.target.clone().unwrap_or_default();
                Text::translate(key, vec![target.into(), self.sender.clone().into(), content])
            }
        }
    }
}

/// A change to a waypoint's icon (`/waypoint modify`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaypointChange {
    /// `0xRRGGBB`, or `None` to reset.
    Color(Option<i32>),
    /// A `waypoint_style` asset id, or `None` for the default.
    Style(Option<String>),
}

/// A level's world border as `/worldborder` reads it (`WorldBorder`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderInfo {
    pub center: [f64; 2],
    /// The current size (`getSize`, mid-move while it moves).
    pub size: f64,
    /// Ticks left of a size change (`getLerpTime`), 0 when still.
    pub lerp_time: i64,
    pub damage_per_block: f64,
    pub safe_zone: f64,
    /// Ticks.
    pub warning_time: i32,
    pub warning_blocks: i32,
}

impl Default for BorderInfo {
    /// `WorldBorder.Settings.DEFAULT`.
    fn default() -> Self {
        BorderInfo {
            center: [0.0, 0.0],
            size: 59_999_968.0,
            lerp_time: 0,
            damage_per_block: 0.2,
            safe_zone: 5.0,
            warning_time: 300,
            warning_blocks: 5,
        }
    }
}

/// A change `/worldborder` makes (the `WorldBorder` setters).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderChange {
    Center(f64, f64),
    Size(f64),
    /// `lerpSizeBetween(from, to, ticks, gameTime)`.
    Lerp { from: f64, to: f64, ticks: i64 },
    DamagePerBlock(f64),
    SafeZone(f64),
    WarningTime(i32),
    WarningBlocks(i32),
}

/// The server's tick rate state (`ServerTickRateManager`), for `/tick query`.
#[derive(Debug, Clone, PartialEq)]
pub struct TickRateInfo {
    pub rate: f32,
    pub nanos_per_tick: i64,
    pub frozen: bool,
    pub sprinting: bool,
    /// `getAverageTickTimeNanos`.
    pub average_tick_nanos: i64,
    /// `getTickTimesNanos`: the last 100 tick times.
    pub tick_times: Vec<i64>,
}

impl Default for TickRateInfo {
    fn default() -> Self {
        TickRateInfo {
            rate: 20.0,
            nanos_per_tick: 50_000_000,
            frozen: false,
            sprinting: false,
            average_tick_nanos: 0,
            tick_times: vec![0; 100],
        }
    }
}

/// What `/tick` asks of the tick rate manager.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickRateAction {
    Rate(f32),
    /// `setFrozen`, after stopping a sprint or steps when freezing.
    Freeze(bool),
    /// `stepGameIfPaused`: false unless frozen.
    Step(i32),
    /// `stopStepping`: whether it was stepping.
    StopStepping,
    /// `requestGameToSprint`: whether a sprint was already running.
    Sprint(i32),
    /// `stopSprinting`: whether it was sprinting.
    StopSprinting,
}

/// A located element (`locate`): its position and registered name.
#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    pub pos: [i32; 3],
    pub id: String,
}

/// What `/place` places.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    /// A configured feature by id, or inline (SNBT) when `inline`.
    Feature { id: Option<Identifier>, inline: Option<Tag> },
    Jigsaw { pool: Identifier, target: Identifier, max_depth: i32 },
    Structure(Identifier),
    Template { id: Identifier, rotation: u8, mirror: u8, integrity: f32, seed: i32, strict: bool },
}

/// Effects of the built-in commands. `Self::Entity` handles come from selectors.
pub trait Host: SelectorWorld {
    /// `sendSuccess`: feedback to the source; `broadcast` also informs operators and the log.
    fn send_success(&mut self, text: Text, broadcast: bool);
    /// `sendFailure` without failing the command: red feedback to the source.
    fn send_failure(&mut self, text: Text) {
        self.send_success(text.color("red"), false);
    }
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
    /// `LivingEntity.addEffect(new MobEffectInstance(effect, duration, amplifier, false,
    /// showParticles), source)`: whether it changed anything; `None` for entities that are not
    /// living.
    fn add_effect(&mut self, _entity: &Self::Entity, _effect: &Identifier, _duration: i32, _amplifier: i32, _show_particles: bool) -> Option<bool> {
        None
    }
    /// `LivingEntity.removeEffect`; `None` for entities that are not living.
    fn remove_effect(&mut self, _entity: &Self::Entity, _effect: &Identifier) -> Option<bool> {
        None
    }
    /// `ExperienceCommand.Type.add`: points (`giveExperiencePoints`) or levels.
    fn add_experience(&mut self, _player: &Self::Entity, _amount: i32, _kind: crate::vanilla::experience::XpKind) {}
    /// `ExperienceCommand.Type.set`: false when points are not below the level's need.
    fn set_experience(&mut self, _player: &Self::Entity, _amount: i32, _kind: crate::vanilla::experience::XpKind) -> bool {
        false
    }
    /// `ExperienceCommand.Type.query`: points into the level, or the level.
    fn query_experience(&mut self, _player: &Self::Entity, _kind: crate::vanilla::experience::XpKind) -> i32 {
        0
    }
    /// `LivingEntity.removeAllEffects`; `None` for entities that are not living.
    fn clear_effects(&mut self, _entity: &Self::Entity) -> Option<bool> {
        None
    }
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
    /// `SummonCommand.createEntity` + `spawnEntity`: `entity` at `pos` from `nbt`, running a
    /// mob's `finalizeSpawn` when `initialize`; returns the entity's display name.
    fn summon(&mut self, _entity: &Identifier, _pos: [f64; 3], _nbt: Option<&kiln_proto::nbt::Tag>, _initialize: bool) -> Result<Text, CommandError> {
        Err(CommandError::new(crate::tr!("commands.summon.failed")))
    }
    fn set_spawn_point(&mut self, player: &Self::Entity, spawn: &SpawnPoint);
    fn set_world_spawn(&mut self, spawn: &SpawnPoint) -> Result<(), CommandError>;
    /// `/kiln tick`: tick timing report lines.
    fn kiln_tick(&mut self) -> Vec<Text>;
    /// `/kiln regions`: region report lines.
    fn kiln_regions(&mut self) -> Vec<Text>;
    /// `/kiln use <player> <pos>`: the player right-clicks the block at `pos` with its main
    /// hand, as its client would (for tools and tests). Whether it was queued.
    fn kiln_use(&mut self, _player: &Self::Entity, _pos: [i32; 3]) -> bool {
        false
    }
    /// `/kiln recipebook <player>`: the player's crafting recipe book is open, as if its
    /// client had toggled it (Recipe Book Settings; for tools and tests).
    fn kiln_open_recipe_book(&mut self, _player: &Self::Entity) -> bool {
        false
    }
    /// `/kiln break <player> <pos>`: the player starts breaking the block at `pos` (a creative
    /// player breaks it at once), as its client would. Whether it was queued.
    fn kiln_break(&mut self, _player: &Self::Entity, _pos: [i32; 3]) -> bool {
        false
    }

    /// Whether chunk `(cx, cz)` of `dimension` is loaded (`ChunkSource.hasChunk`).
    fn is_chunk_loaded(&self, dimension: &str, cx: i32, cz: i32) -> bool;
    /// `execute if loaded` (`isChunkLoaded`): loaded at entity-ticking level with its
    /// entities; hosts that do not track ticket levels answer [`is_chunk_loaded`](Self::is_chunk_loaded).
    fn is_chunk_ticking(&self, dimension: &str, cx: i32, cz: i32) -> bool {
        self.is_chunk_loaded(dimension, cx, cz)
    }
    /// The build height of `dimension` as `[min_y, max_y)`.
    fn build_height(&self, _dimension: &str) -> (i32, i32) {
        (-64, 320)
    }
    /// The block state at `pos` (void air outside the build height), loading or generating
    /// the chunk if needed.
    fn block_state(&mut self, dimension: &str, pos: [i32; 3]) -> u16;
    /// The block entity data at `pos` (`saveWithFullMetadata`) if the block has an entity.
    /// Hosts without block entity storage return `None`.
    fn block_entity(&mut self, _dimension: &str, _pos: [i32; 3]) -> Option<Tag> {
        None
    }
    /// An entity's data (`EntityDataAccessor.getData`: `saveWithoutId`, with a player's held
    /// item as `SelectedItem`). Hosts without entity data return `None`.
    fn entity_data(&mut self, _entity: &Self::Entity) -> Option<Tag> {
        None
    }
    /// `Level.setBlock` (`BlockInput.place` when `nbt` is given): returns whether the state
    /// changed, or (with `nbt`) whether the block entity's saved data changed. Without
    /// [`UpdateFlags::KNOWN_SHAPE`] vanilla first adapts `state` to its neighbours' shapes
    /// (fences, stairs, ...); hosts may place it as given. `nbt` is block entity data to load
    /// into the block's entity; hosts without block entity storage ignore it (so re-applying
    /// data to an unchanged block reports no change).
    fn set_block(&mut self, dimension: &str, pos: [i32; 3], state: u16, nbt: Option<&Tag>, flags: UpdateFlags) -> bool;
    /// `BlockInput.place`: without [`UpdateFlags::KNOWN_SHAPE`] vanilla shapes the state by its
    /// neighbours, then re-applies the properties the input names, and sets it with `flags`.
    /// Hosts without shape updates place the input's state as given.
    fn place_block(&mut self, dimension: &str, pos: [i32; 3], block: &BlockInput, flags: UpdateFlags) -> bool {
        let state = block.overwrite_defined(block.state);
        self.set_block(dimension, pos, state, block.nbt.as_ref(), flags)
    }
    /// `Level.updateNeighboursOnBlockSet`: neighbour reactions to a change made without
    /// `strict`.
    fn update_neighbours(&mut self, _dimension: &str, _pos: [i32; 3], _old: u16) {}
    /// `Level.destroyBlock(pos, drop)`: breaks the block as a player would (particles, drops);
    /// returns whether there was a block (not air).
    fn destroy_block(&mut self, dimension: &str, pos: [i32; 3], drop: bool) -> bool;
    /// `Level.getHeight(heightmap, x, z)` of a loaded column: one above the highest block
    /// the heightmap counts, or the minimum build height.
    fn height(&mut self, dimension: &str, heightmap: Heightmap, x: i32, z: i32) -> i32;
    /// The biome id at `pos` (`Level.getBiome`), if the host knows biomes.
    fn biome(&mut self, _dimension: &str, _pos: [i32; 3]) -> Option<String> {
        None
    }
    /// Whether the level `dimension` exists (`DimensionArgument.getDimension`).
    fn has_dimension(&self, dimension: &str) -> bool {
        self.dimensions().iter().any(|d| d == dimension)
    }
    /// The scoreboard, if the host keeps one (see [`SelectorWorld::scoreboard`]).
    fn scoreboard_mut(&mut self) -> Option<&mut Scoreboard> {
        None
    }
    /// Command storage (`CommandStorage`), if the host keeps it.
    fn storage_mut(&mut self) -> Option<&mut CommandStorage> {
        None
    }
    /// Custom boss bars, if the host keeps them; hosts send what
    /// [`BossBars::take_packets`] queues.
    fn bossbars(&self) -> Option<&BossBars> {
        None
    }
    fn bossbars_mut(&mut self) -> Option<&mut BossBars> {
        None
    }
    /// Sets the value (or `max`) of custom boss bar `id` (`execute store ... bossbar`).
    /// Hosts without boss bars have none, like a fresh vanilla server.
    fn set_bossbar(&mut self, id: &Identifier, max: bool, value: i32) -> Result<(), CommandError> {
        match self.bossbars_mut().filter(|b| b.get(id).is_some()) {
            Some(bars) if max => bars.set_max(id, value),
            Some(bars) => bars.set_value(id, value),
            None => return Err(CommandError::new(tr!("commands.bossbar.unknown", id.to_string()))),
        }
        Ok(())
    }
    /// Whether custom boss bar `id` exists.
    fn has_bossbar(&self, id: &Identifier) -> bool {
        self.bossbars().is_some_and(|b| b.get(id).is_some())
    }
    /// Loaded functions and function tags, if the host loads data packs.
    fn functions(&self) -> Option<&FunctionLibrary> {
        None
    }
    /// `Advancement.name` (`[title]`, or the id without a display); `None` when the
    /// advancement does not exist.
    fn advancement_name(&self, _id: &str) -> Option<Text> {
        None
    }
    /// Every advancement id, in load order (`/advancement ... everything`).
    fn advancement_ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// The advancement's criterion names.
    fn advancement_criteria(&self, _id: &str) -> Vec<String> {
        Vec::new()
    }
    /// Its parents, nearest first.
    fn advancement_parents(&self, _id: &str) -> Vec<String> {
        Vec::new()
    }
    /// Its descendants, depth first (`AdvancementCommands.addChildren`).
    fn advancement_descendants(&self, _id: &str) -> Vec<String> {
        Vec::new()
    }
    /// `AdvancementCommands.Action.perform` for one advancement: grants every remaining
    /// criterion (or revokes every obtained one); whether anything changed.
    fn change_advancement(&mut self, _player: &Self::Entity, _id: &str, _revoke: bool) -> bool {
        false
    }
    /// `performCriterion`.
    fn change_criterion(&mut self, _player: &Self::Entity, _id: &str, _criterion: &str, _revoke: bool) -> bool {
        false
    }
    /// `PlayerAdvancements.flushDirty(player, showAdvancements)`.
    fn flush_advancements(&mut self, _player: &Self::Entity, _show: bool) {}
    /// Ids of the recipes a recipe book can hold (not special), in registry order
    /// (`/recipe ... *`; `ResourceKeyArgument.getRecipe` rejects special ones).
    fn recipe_ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// `awardRecipes` (or `resetRecipes` with `take`) for a player: how many changed.
    fn change_recipes(&mut self, _player: &Self::Entity, _recipes: &[String], _take: bool) -> i32 {
        0
    }
    /// Scheduled functions (`/schedule`).
    fn timers(&self) -> Option<&TimerQueue> {
        None
    }
    fn timers_mut(&mut self) -> Option<&mut TimerQueue> {
        None
    }
    /// The overworld's game time, for `/schedule`.
    fn game_time(&self) -> i64 {
        0
    }
    /// Available and enabled data packs.
    fn data_packs(&self) -> Option<DataPacks> {
        None
    }
    /// Looks for packs added or removed since (`PackRepository.reload`).
    fn refresh_packs(&mut self) {}
    /// `MinecraftServer.reloadResources`: reloads functions, tags, recipes and loot tables
    /// from `selected` packs, in order (`None`: the enabled packs plus newly found ones, as
    /// `/reload` does).
    fn reload_packs(&mut self, _selected: Option<Vec<String>>) {}
    /// `/datapack create`: an empty pack in the world's `datapacks` directory.
    fn create_pack(&mut self, id: &str, _description: &Text) -> Result<(), CommandError> {
        Err(CommandError::new(tr!("commands.datapack.create.io_failure", id)))
    }
    /// Sends a play packet to one player (titles and the action bar).
    fn send_packet(&mut self, _player: &Self::Entity, _packet: Bytes) {}

    // ---- server administration: whitelist, bans, saving, defaults ----

    /// The whitelist and ban lists, shared with the login checks; `None` when the host keeps
    /// none (the commands then fail).
    fn access(&self) -> Option<kiln_link::access::SharedAccess> {
        None
    }
    /// `ServerPlayer.getIpAddress`.
    fn player_ip(&self, _player: &Self::Entity) -> Option<String> {
        None
    }
    /// `MinecraftServer.setAutoSave`: whether it changed.
    fn set_auto_save(&mut self, _on: bool) -> bool {
        false
    }
    /// `MinecraftServer.saveEverything`: whether saving worked.
    fn save_all(&mut self, _flush: bool) -> bool {
        true
    }
    /// `setDefaultGameType` + `enforceGameTypeForPlayers`: players whose mode changed.
    fn set_default_game_mode(&mut self, _mode: GameMode) -> i32 {
        0
    }
    /// `setPlayerIdleTimeout` (minutes, 0 = off).
    fn set_idle_timeout(&mut self, _minutes: i32) {}
    /// A sound's variant seed (`level.getRandom().nextLong()`).
    fn random_seed(&mut self) -> i64 {
        0
    }

    // ---- stopwatches and post effects ----

    /// Stopwatch ids (`Stopwatches.ids`).
    fn stopwatch_ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// Starts a stopwatch; false if `id` exists.
    fn stopwatch_create(&mut self, _id: &str) -> bool {
        false
    }
    /// Seconds the stopwatch has run.
    fn stopwatch_seconds(&self, _id: &str) -> Option<f64> {
        None
    }
    /// Restarts it from zero; false if there is none.
    fn stopwatch_restart(&mut self, _id: &str) -> bool {
        false
    }
    fn stopwatch_remove(&mut self, _id: &str) -> bool {
        false
    }
    /// `ServerPlayer.getPostEffects`.
    fn post_effects(&self, _player: &Self::Entity) -> Vec<String> {
        Vec::new()
    }
    /// `addPostEffect`: whether it was added.
    fn add_post_effect(&mut self, _player: &Self::Entity, _id: &str) -> bool {
        false
    }
    fn remove_post_effect(&mut self, _player: &Self::Entity, _id: &str) -> bool {
        false
    }
    fn clear_post_effects(&mut self, _player: &Self::Entity) -> bool {
        false
    }

    // ---- waypoints (the locator bar) ----

    /// The display names of `dimension`'s waypoint transmitters
    /// (`ServerWaypointManager.transmitters`).
    fn waypoints(&self, _dimension: &str) -> Vec<Text> {
        Vec::new()
    }
    /// Whether `entity` can be a waypoint (a living entity, `WaypointArgument`).
    fn is_waypoint(&self, entity: &Self::Entity) -> bool {
        entity.is_player()
    }
    /// `WaypointCommand.mutateIcon`: whether the host keeps icons for `entity`.
    fn modify_waypoint(&mut self, _entity: &Self::Entity, _change: &WaypointChange) -> bool {
        false
    }
    // ---- data, tag, item, loot, clear, enchant, attribute, damage, ride, rotate, spectate,
    // swing and fetchprofile (`entity_data` is above, with `function ... with entity`) -----

    /// `EntityDataAccessor.setData` for a non-player: `Entity.load(data)` keeping its UUID.
    fn set_entity_data(&mut self, _entity: &Self::Entity, _data: &Tag) -> Result<(), CommandError> {
        Err(CommandError::unsupported("Entity data"))
    }
    /// `BlockDataAccessor.setData`: loads `data` into the block entity at `pos`.
    fn set_block_entity_data(&mut self, _dimension: &str, _pos: [i32; 3], _data: &Tag) -> Result<(), CommandError> {
        Err(CommandError::unsupported("Block entity data"))
    }
    /// Command storage ids with data (`/data ... storage` suggestions).
    fn storage_ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// `Entity.entityTags`.
    fn entity_tags(&mut self, entity: &Self::Entity) -> Vec<String> {
        entity.tags().to_vec()
    }
    /// `Entity.addTag`: false when present or the entity has 1024 tags.
    fn add_entity_tag(&mut self, _entity: &Self::Entity, _tag: &str) -> bool {
        false
    }
    /// `Entity.removeTag`.
    fn remove_entity_tag(&mut self, _entity: &Self::Entity, _tag: &str) -> bool {
        false
    }
    /// `Entity.forceSetRotation` to an absolute `[yaw, pitch]`.
    fn rotate_entity(&mut self, _entity: &Self::Entity, _rotation: [f32; 2]) {}
    /// `LivingEntity.swing(hand, animation)`; false for entities that are not living.
    fn swing_arm(&mut self, _entity: &Self::Entity, _offhand: bool, _animation: &str, _duration: i32) -> bool {
        false
    }
    /// `Entity.getVehicle`.
    fn vehicle_of(&mut self, _entity: &Self::Entity) -> Option<Self::Entity> {
        None
    }
    /// `getSelfAndPassengers` (recursively).
    fn self_and_passengers(&mut self, entity: &Self::Entity) -> Vec<Self::Entity> {
        vec![entity.clone()]
    }
    /// `Entity.startRiding(vehicle, force, true)`: whether the entity now rides.
    fn start_riding(&mut self, _entity: &Self::Entity, _vehicle: &Self::Entity) -> bool {
        false
    }
    /// `Entity.stopRiding`.
    fn stop_riding(&mut self, _entity: &Self::Entity) {}
    /// `hurtServer` with a damage source of `damage_type` (`at` a position, `by` a direct
    /// entity, `from` a causing entity); whether the entity was hurt.
    fn damage_entity(
        &mut self,
        _entity: &Self::Entity,
        _amount: f32,
        _damage_type: &str,
        _at: Option<[f64; 3]>,
        _by: Option<&Self::Entity>,
        _from: Option<&Self::Entity>,
    ) -> Result<bool, CommandError> {
        Err(CommandError::unsupported("Damage"))
    }
    /// `EntityType.clientTrackingRange() != 0`: whether a player may spectate it.
    fn can_spectate(&self, _entity: &Self::Entity) -> bool {
        true
    }
    /// `ServerPlayer.setCamera` (`None`: back to the player itself).
    fn set_camera(&mut self, _player: &Self::Entity, _target: Option<&Self::Entity>) {}
    /// The item in `slot` (a [`slots`](crate::slots) id) of a container block or entity, as
    /// item stack NBT (`None` for an empty slot); `None` when there is no such slot. Callers
    /// check [`is_container`](Self::is_container) for blocks first.
    fn slot_item(&mut self, _holder: &ItemHolder<Self::Entity>, _slot: i32) -> Option<Option<Tag>> {
        None
    }
    /// Puts an item (stack NBT, `None` to empty it) into `slot`; false when the slot does not
    /// exist or refuses the item.
    fn set_slot_item(&mut self, _holder: &ItemHolder<Self::Entity>, _slot: i32, _item: Option<&Tag>) -> bool {
        false
    }
    /// Whether the block at `pos` is a container (`Container` block entity).
    fn is_container(&mut self, _dimension: &str, _pos: [i32; 3]) -> bool {
        false
    }
    /// The inventory slots `/clear` goes through for a player, in `Inventory` order (main,
    /// equipment), then the crafting grid and the cursor.
    fn clear_slots(&self, _player: &Self::Entity) -> Vec<i32> {
        Vec::new()
    }
    /// Sends inventory changes made by commands (`containerMenu.broadcastChanges`).
    fn inventory_changed(&mut self, _player: &Self::Entity) {}
    /// Enchantment definitions: `max_level`; `None` for unknown enchantments.
    fn enchantment_max_level(&self, _enchantment: &str) -> Option<i32> {
        None
    }
    /// An attribute of a living entity: `Err(())` for entities that are not living, `Ok(None)`
    /// when it lacks the attribute.
    #[allow(clippy::result_unit_err)]
    fn attribute(&mut self, _entity: &Self::Entity, _attribute: &str) -> Result<Option<AttributeState>, ()> {
        Err(())
    }
    /// `AttributeInstance.setBaseValue` (the attribute exists).
    fn set_attribute_base(&mut self, _entity: &Self::Entity, _attribute: &str, _value: f64) {}
    /// `AttributeMap.resetBaseValue` to the type's default (the attribute exists).
    fn reset_attribute_base(&mut self, _entity: &Self::Entity, _attribute: &str) {}
    /// `addPermanentModifier` (`operation`: 0 add_value, 1 add_multiplied_base, 2
    /// add_multiplied_total; the id is not present yet).
    fn add_attribute_modifier(&mut self, _entity: &Self::Entity, _attribute: &str, _id: &str, _amount: f64, _operation: u8) {}
    /// `removeModifier`: whether it was there.
    fn remove_attribute_modifier(&mut self, _entity: &Self::Entity, _attribute: &str, _id: &str) -> bool {
        false
    }
    /// Rolls loot for `/loot` (stacks as item stack NBT, already split to stack sizes) and the
    /// table used, if it was named.
    fn roll_loot(&mut self, _source: &LootSource<Self::Entity>) -> Result<(Vec<Tag>, Option<String>), CommandError> {
        Err(CommandError::unsupported("Loot"))
    }
    /// `LivingEntity.getItemBySlot(MAINHAND/OFFHAND)`: `None` for entities that are not living.
    fn hand_item(&mut self, _entity: &Self::Entity, _offhand: bool) -> Option<Option<Tag>> {
        None
    }
    /// `Inventory.add(copy)`: whether anything was added.
    fn give_stack(&mut self, _player: &Self::Entity, _item: &Tag) -> bool {
        false
    }
    /// Spawns an item entity at `pos` with the default pickup delay.
    fn spawn_item(&mut self, _dimension: &str, _pos: [f64; 3], _item: &Tag) {}
    /// `Container.getContainerSize` of the container at `pos`.
    fn container_size(&mut self, _dimension: &str, _pos: [i32; 3]) -> Option<i32> {
        None
    }
    /// `ItemStack.getMaxStackSize` of item stack NBT.
    fn item_max_stack(&self, _item: &Tag) -> i32 {
        64
    }
    /// `ItemStack.getHoverName` of item stack NBT.
    fn item_name(&self, item: &Tag) -> Text {
        let id = match item.get("id") {
            Some(Tag::String(s)) => s.as_str(),
            _ => "minecraft:air",
        };
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let kind = if kiln_data::builtin_id("minecraft:block", id).is_some() { "block" } else { "item" };
        Text::translate(format!("{kind}.{ns}.{path}"), Vec::new())
    }
    /// The slots a slot source (a registry id or an inline definition) selects, evaluated with
    /// `container` as the `container` parameter and the source entity as `this`.
    fn slot_source_tree(&mut self, _source: &LootTableArg, _container: &ItemHolder<Self::Entity>) -> Result<SlotTree<Self::Entity>, CommandError> {
        Err(CommandError::unsupported("Slot sources"))
    }
    /// Applies an item modifier (`/item modify`, `/item ... from ... <modifier>`) to item stack
    /// NBT; `None` when the modifier is unknown.
    fn apply_item_modifier(&mut self, _modifier: &LootTableArg, _item: &Tag) -> Result<Tag, CommandError> {
        Err(CommandError::unsupported("Item modifiers"))
    }
    /// `EnchantCommand` on one entity's main hand item.
    fn enchant_held(&mut self, _entity: &Self::Entity, _enchantment: &str, _level: i32) -> EnchantOutcome {
        EnchantOutcome::NotLiving
    }
    // ---- worldborder, tick, forceload, random, locate, place, fillbiome, spreadplayers ----

    /// The world border of `dimension`.
    fn world_border(&mut self, _dimension: &str) -> BorderInfo {
        BorderInfo::default()
    }
    /// Applies a border change (players in the level are told, the level saves it).
    fn change_world_border(&mut self, _dimension: &str, _change: BorderChange) {}
    /// The level's game time, for border moves.
    fn level_game_time(&self, _dimension: &str) -> i64 {
        self.game_time()
    }
    fn tick_rate(&self) -> TickRateInfo {
        TickRateInfo::default()
    }
    /// Applies `/tick` actions; the returned flag is the manager method's result.
    fn change_tick_rate(&mut self, _action: TickRateAction) -> bool {
        false
    }
    /// `ServerLevel.getForceLoadedChunks` of `dimension`.
    fn forced_chunks(&self, _dimension: &str) -> Vec<[i32; 2]> {
        Vec::new()
    }
    /// `ServerLevel.setChunkForced`: whether it changed.
    fn set_chunk_forced(&mut self, _dimension: &str, _chunk: [i32; 2], _forced: bool) -> bool {
        false
    }
    /// `Mth.randomBetweenInclusive` on random sequence `sequence` (the server's, seeded from
    /// the world seed) or, without one, the level random.
    fn random_between(&mut self, _sequence: Option<&Identifier>, min: i32, _max: i32) -> i32 {
        min
    }
    /// `RandomSequences.reset(id, seed, salt, includeWorldSeed, includeSequenceId)`, with the
    /// sequence defaults when `params` is `None`.
    fn reset_random_sequence(&mut self, _id: &Identifier, _params: Option<(i32, bool, bool)>) {}
    /// `RandomSequences.clear` (after `setSeedDefaults` when `defaults` is given): how many
    /// sequences there were.
    fn clear_random_sequences(&mut self, _defaults: Option<(i32, bool, bool)>) -> i32 {
        0
    }
    /// Ids of the existing random sequences, for suggestions.
    fn random_sequence_ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// `PlayerList.broadcastSystemMessage`: to every player and the server log.
    fn broadcast_system_message(&mut self, text: Text) {
        self.send_success(text, false);
    }
    /// `ServerLevel.findClosestBiome3d(origin, 6400, 32, 64)` over the generator's biome
    /// source, for biomes `matches` accepts. `None` when nothing matches.
    fn locate_biome(&mut self, _dimension: &str, _origin: [i32; 3], _matches: &dyn Fn(&str) -> bool) -> Option<Located> {
        None
    }
    /// `ChunkGenerator.findNearestMapStructure(level, structures, origin, 100, false)`.
    fn locate_structure(&mut self, _dimension: &str, _origin: [i32; 3], _structures: &[String]) -> Option<Located> {
        None
    }
    /// Structure ids of the level's registry (`minecraft:worldgen/structure`).
    fn structure_ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// Structure ids in tag `tag`, if the tag exists.
    fn structure_tag(&self, _tag: &str) -> Option<Vec<String>> {
        None
    }
    /// `PoiManager.findClosestWithType(types, origin, 256, ANY)`.
    fn locate_poi(&mut self, _dimension: &str, _origin: [i32; 3], _matches: &dyn Fn(&str) -> bool) -> Option<Located> {
        None
    }
    /// `/place`: places at `pos` (chunks already checked loaded where vanilla checks before
    /// placing); the error is vanilla's failure.
    fn place(&mut self, _dimension: &str, _what: &Placement, _pos: [i32; 3]) -> Result<(), CommandError> {
        Err(CommandError::unsupported("place"))
    }
    /// The biome id at quart position `quart` of a loaded chunk.
    fn noise_biome(&mut self, _dimension: &str, _quart: [i32; 3]) -> Option<String> {
        None
    }
    /// `/fillbiome`: sets `biome` in the quart cells of loaded chunks inside `[min, max]` (block
    /// coordinates, quantized) whose biome `filter` accepts and differs; resends the chunks.
    /// Returns how many cells changed, or `None` when a chunk is not loaded.
    fn fill_biome(&mut self, _dimension: &str, _min: [i32; 3], _max: [i32; 3], _biome: &str, _filter: &dyn Fn(&str) -> bool) -> Option<i32> {
        None
    }
}


/// An entity's attribute instance as `/attribute` reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeState {
    pub base: f64,
    /// `getAttributeValue`.
    pub value: f64,
    /// Modifier ids and amounts.
    pub modifiers: Vec<(String, f64)>,
}

/// A `loot_table` / `loot_modifier` argument: a registry id or an inline definition (SNBT).
#[derive(Debug, Clone, PartialEq)]
pub enum LootTableArg {
    Id(String),
    Inline(Tag),
}

/// Where `/loot` takes items from.
pub enum LootSource<E> {
    /// `loot <table>`: the chest parameter set at the source's position.
    Table { table: LootTableArg, origin: [f64; 3], dimension: String, this: Option<E> },
    /// `fish <table> <pos> [tool]`.
    Fish { table: LootTableArg, pos: [i32; 3], dimension: String, tool: Option<Tag>, this: Option<E> },
    /// `kill <target>`: the entity's loot table, as killed by the source (magic damage).
    Kill { target: E, origin: [f64; 3], killer: Option<E> },
    /// `mine <pos> [tool]`: the block's drops.
    Mine { pos: [i32; 3], dimension: String, tool: Option<Tag>, this: Option<E> },
}

/// Where `/item` and `/loot` put items: a container block or an entity.
#[derive(Clone)]
pub enum ItemHolder<E> {
    Block { dimension: String, pos: [i32; 3] },
    Entity(E),
}

/// `SlotCollection`: the slots a slot source selected, in order.
#[derive(Clone)]
pub enum SlotTree<E> {
    Empty,
    /// Existing slots (holder and slot id).
    Slots(Vec<(ItemHolder<E>, i32)>),
    Concat(Vec<SlotTree<E>>),
    Filtered(Box<SlotTree<E>>, std::rc::Rc<dyn Fn(&Tag) -> bool>),
    Limited(Box<SlotTree<E>>, usize),
}

/// What `/enchant` did to one target.
#[derive(Debug, Clone, PartialEq)]
pub enum EnchantOutcome {
    NotLiving,
    NoItem,
    /// The item cannot take the enchantment (unsupported or incompatible); its hover name.
    Incompatible(Text),
    Applied,
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
