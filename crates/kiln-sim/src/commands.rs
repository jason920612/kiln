//! Commands: the simulation is the command source and host for `kiln-command`.
//!
//! The dispatcher is generic over a source type without borrowed lifetimes, so `Sim` itself is
//! the source: `CommandSource` says who is executing (a player or the console) while a command
//! runs.

use crate::{Player, Sim, players::chat_disguised};
use bytes::Bytes;
use kiln_command::selector::{Aabb, SelectorTarget, SelectorWorld};
use kiln_command::{
    ChatMessage, CommandError, Difficulty, Dispatcher, GameMode, GameRuleValue, Host, Identifier, ItemInput, Profile,
    Source, SpawnPoint, Teleport, Text, TimeAction, Weather,
};
use kiln_link::ConnId;
use kiln_proto::packets;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

const OVERWORLD: &str = "minecraft:overworld";
/// Longest tab-completion request answered for players without command-block rights (vanilla).
const MAX_SUGGESTION_LEN: usize = 256;

/// Characters chat and commands may not contain (vanilla kicks for them): the section sign,
/// control characters and DEL.
pub(crate) fn has_illegal_chars(s: &str) -> bool {
    s.chars().any(|c| c == '\u{a7}' || c < ' ' || c == '\u{7f}')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandSource {
    Console,
    Player(ConnId),
}

/// A player as seen by selectors (a snapshot, so selectors can hold it while the host mutates).
#[derive(Debug, Clone)]
pub struct PlayerRef {
    pub(crate) conn: ConnId,
    uuid: Uuid,
    name: String,
    pos: [f64; 3],
    rot: [f32; 2],
    mode: GameMode,
}

impl PlayerRef {
    fn of(conn: ConnId, p: &Player) -> Self {
        Self { conn, uuid: p.uuid, name: p.name.clone(), pos: p.pos, rot: p.rot, mode: game_mode(p.game_mode) }
    }
}

fn game_mode(id: u8) -> GameMode {
    match id {
        0 => GameMode::Survival,
        2 => GameMode::Adventure,
        3 => GameMode::Spectator,
        _ => GameMode::Creative,
    }
}

impl SelectorTarget for PlayerRef {
    fn uuid(&self) -> Uuid {
        self.uuid
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn entity_type(&self) -> &str {
        "minecraft:player"
    }
    fn position(&self) -> [f64; 3] {
        self.pos
    }
    fn rotation(&self) -> [f32; 2] {
        self.rot
    }
    fn dimension(&self) -> &str {
        OVERWORLD
    }
    fn bounding_box(&self) -> Aabb {
        let [x, y, z] = self.pos;
        Aabb { min: [x - 0.3, y, z - 0.3], max: [x + 0.3, y + 1.8, z + 0.3] }
    }
    fn eye_height(&self) -> f64 {
        1.62
    }
    fn game_mode(&self) -> Option<GameMode> {
        Some(self.mode)
    }
}

/// Server-wide state the commands change.
pub(crate) struct CommandState {
    pub dispatcher: Arc<Dispatcher<Sim>>,
    pub source: CommandSource,
    /// Operators by name (permission level 4).
    pub ops: std::collections::HashSet<String>,
    pub difficulty: Difficulty,
    pub raining: bool,
    pub thundering: bool,
    pub game_rules: HashMap<String, GameRuleValue>,
    pub seed: i64,
    /// Shuffle state for `@r` / `sort=random` (xorshift).
    pub rng: u64,
    pub stop_requested: bool,
    /// Last tick statistics report, for `/kiln tick`.
    pub last_report: Option<String>,
}

impl CommandState {
    /// Operator by name; a `prefix*` entry (from `KILN_OPS`, for load tests) matches every name
    /// starting with the prefix.
    pub fn is_op(&self, name: &str) -> bool {
        self.ops.contains(name)
            || self.ops.iter().any(|o| o.strip_suffix('*').is_some_and(|prefix| name.starts_with(prefix)))
    }

    pub fn new(ops: std::collections::HashSet<String>) -> Self {
        let mut d = Dispatcher::new();
        kiln_command::vanilla::register_all(&mut d);
        Self {
            dispatcher: Arc::new(d),
            source: CommandSource::Console,
            ops,
            difficulty: Difficulty::Normal,
            raining: false,
            thundering: false,
            game_rules: HashMap::new(),
            seed: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
            stop_requested: false,
            last_report: None,
        }
    }
}

impl Sim {
    pub(crate) fn rule_bool(&self, rule: &str) -> bool {
        matches!(Host::game_rule(self, rule), GameRuleValue::Bool(true))
    }

    pub(crate) fn permission_level_of(&self, conn: ConnId) -> u8 {
        match self.players.get(&conn) {
            Some(p) if self.commands.is_op(&p.name) => 4,
            _ => 0,
        }
    }

    /// The command tree, filtered to what the player may use.
    pub(crate) fn send_command_tree(&mut self, conn: ConnId) {
        let level = self.permission_level_of(conn);
        let pkt = self.commands.dispatcher.commands_packet(level);
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(pkt);
        }
    }

    pub(crate) fn run_command(&mut self, conn: ConnId, command: &str) {
        if has_illegal_chars(command) {
            if let Some(p) = self.players.get_mut(&conn) {
                p.disconnect("Illegal characters in chat");
            }
            return;
        }
        let name = self.players.get(&conn).map(|p| p.name.clone()).unwrap_or_default();
        info!("{name} issued server command: /{command}");
        self.execute_as(CommandSource::Player(conn), command);
    }

    /// Runs a command from the server console (permission level 4).
    pub(crate) fn run_console_command(&mut self, command: &str) {
        self.execute_as(CommandSource::Console, command);
    }

    fn execute_as(&mut self, source: CommandSource, command: &str) {
        let dispatcher = self.commands.dispatcher.clone();
        let previous = std::mem::replace(&mut self.commands.source, source);
        if let Err(e) = dispatcher.execute(command, self) {
            for line in e.chat_lines(command) {
                self.reply(line);
            }
        }
        self.commands.source = previous;
    }

    /// Queues a tab-completion request; answered once per tick (latest request wins).
    pub(crate) fn suggest(&mut self, conn: ConnId, id: i32, text: String) {
        let level = self.permission_level_of(conn);
        let Some(p) = self.players.get_mut(&conn) else { return };
        if text.encode_utf16().count() > MAX_SUGGESTION_LEN && !(level >= 2 && p.game_mode == 1) {
            return;
        }
        p.pending_suggestion = Some((id, text));
    }

    pub(crate) fn answer_suggestions(&mut self) {
        let pending: Vec<(ConnId, i32, String)> = self
            .players
            .iter_mut()
            .filter_map(|(&c, p)| p.pending_suggestion.take().map(|(id, t)| (c, id, t)))
            .collect();
        let dispatcher = self.commands.dispatcher.clone();
        for (conn, id, text) in pending {
            let previous = std::mem::replace(&mut self.commands.source, CommandSource::Player(conn));
            let pkt = dispatcher.suggestions_packet(id, &text, self);
            self.commands.source = previous;
            if let Some(p) = self.players.get_mut(&conn) {
                p.send(pkt);
            }
        }
    }

    /// System message to the current command source.
    fn reply(&mut self, text: Text) {
        match self.commands.source {
            CommandSource::Console => info!("{}", text.to_plain()),
            CommandSource::Player(conn) => {
                if let Some(p) = self.players.get_mut(&conn) {
                    p.send(packets::system_chat(text.to_nbt(), false));
                }
            }
        }
    }

    fn send_to(&mut self, conn: ConnId, pkt: Bytes) {
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(pkt);
        }
    }

    fn weather_packets(&self) -> Vec<Bytes> {
        const START_RAINING: u8 = 2;
        const STOP_RAINING: u8 = 1;
        const RAIN_LEVEL: u8 = 7;
        const THUNDER_LEVEL: u8 = 8;
        let rain = if self.commands.raining { 1.0 } else { 0.0 };
        let thunder = if self.commands.thundering { 1.0 } else { 0.0 };
        vec![
            packets::game_event(if self.commands.raining { START_RAINING } else { STOP_RAINING }, 0.0),
            packets::game_event(RAIN_LEVEL, rain),
            packets::game_event(THUNDER_LEVEL, thunder),
        ]
    }
}

impl Source for Sim {
    fn permission_level(&self) -> u8 {
        match self.commands.source {
            CommandSource::Console => 4,
            CommandSource::Player(conn) => self.permission_level_of(conn),
        }
    }

    fn player_names(&self) -> Vec<String> {
        self.players.values().map(|p| p.name.clone()).collect()
    }

    fn dimensions(&self) -> Vec<String> {
        vec![OVERWORLD.to_owned()]
    }
}

impl SelectorWorld for Sim {
    type Entity = PlayerRef;

    fn origin(&self) -> [f64; 3] {
        match self.commands.source {
            CommandSource::Player(conn) => self.players.get(&conn).map_or([0.0; 3], |p| p.pos),
            CommandSource::Console => [self.spawn[0] as f64, self.spawn[1] as f64, self.spawn[2] as f64],
        }
    }

    fn dimension(&self) -> &str {
        OVERWORLD
    }

    fn source_entity(&self) -> Option<PlayerRef> {
        match self.commands.source {
            CommandSource::Player(conn) => self.players.get(&conn).map(|p| PlayerRef::of(conn, p)),
            CommandSource::Console => None,
        }
    }

    fn players(&self) -> Vec<PlayerRef> {
        let mut v: Vec<PlayerRef> = self.players.iter().map(|(&c, p)| PlayerRef::of(c, p)).collect();
        v.sort_by_key(|p| p.conn); // join order
        v
    }

    fn entities(&self, dimension: Option<&str>, _area: Option<&Aabb>) -> Vec<PlayerRef> {
        // Only players exist as entities so far.
        match dimension {
            Some(d) if d != OVERWORLD => Vec::new(),
            _ => self.players(),
        }
    }

    fn shuffle(&mut self, entities: &mut [PlayerRef]) {
        for i in (1..entities.len()).rev() {
            let r = &mut self.commands.rng;
            *r ^= *r << 13;
            *r ^= *r >> 7;
            *r ^= *r << 17;
            entities.swap(i, (*r % (i as u64 + 1)) as usize);
        }
    }
}

impl Host for Sim {
    fn source_name(&self) -> Text {
        match self.commands.source {
            CommandSource::Player(conn) => Text::literal(self.players.get(&conn).map_or("", |p| p.name.as_str())),
            CommandSource::Console => Text::literal("Server"),
        }
    }

    fn source_rotation(&self) -> [f32; 2] {
        match self.commands.source {
            CommandSource::Player(conn) => self.players.get(&conn).map_or([0.0; 2], |p| p.rot),
            CommandSource::Console => [0.0; 2],
        }
    }

    fn send_success(&mut self, text: Text, broadcast: bool) {
        if broadcast {
            // Other operators see a gray, italic "[Source: message]" (chat.type.admin).
            let admin = kiln_command::tr!("chat.type.admin", self.source_name(), text.clone()).color("gray").italic();
            let pkt = packets::system_chat(admin.to_nbt(), false);
            let me = match self.commands.source {
                CommandSource::Player(c) => Some(c),
                CommandSource::Console => None,
            };
            let ops: Vec<ConnId> = self
                .players
                .iter()
                .filter(|(c, p)| Some(**c) != me && self.commands.is_op(&p.name))
                .map(|(c, _)| *c)
                .collect();
            for c in ops {
                self.send_to(c, pkt.clone());
            }
        }
        self.reply(text);
    }

    fn send_system(&mut self, player: &PlayerRef, text: Text) {
        self.send_to(player.conn, packets::system_chat(text.to_nbt(), false));
    }

    fn broadcast_chat(&mut self, message: ChatMessage) {
        info!("{}", message.to_text().to_plain());
        self.broadcast(chat_disguised(&message));
    }

    fn send_chat(&mut self, player: &PlayerRef, message: ChatMessage) {
        self.send_to(player.conn, chat_disguised(&message));
    }

    fn send_chat_to_source(&mut self, message: ChatMessage) {
        match self.commands.source {
            CommandSource::Player(conn) => self.send_to(conn, chat_disguised(&message)),
            CommandSource::Console => info!("{}", message.to_text().to_plain()),
        }
    }

    fn teleport(&mut self, entity: &PlayerRef, to: &Teleport) -> Result<(), CommandError> {
        if to.dimension != OVERWORLD {
            return Err(CommandError::new(Text::literal("Other dimensions are not loaded yet")));
        }
        let Some(p) = self.players.get_mut(&entity.conn) else { return Ok(()) };
        let rot = match (to.facing, to.rotation) {
            (Some(f), _) => kiln_command::host::look_at([to.pos[0], to.pos[1] + 1.62, to.pos[2]], f),
            (None, Some(r)) => r,
            (None, None) => p.rot,
        };
        p.teleport(to.pos, rot, self.game_time);
        Ok(())
    }

    fn set_game_mode(&mut self, player: &PlayerRef, mode: GameMode) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        if p.game_mode == mode as u8 {
            return false;
        }
        p.game_mode = mode as u8;
        const CHANGE_GAME_MODE: u8 = 3;
        p.send(packets::game_event(CHANGE_GAME_MODE, mode as u8 as f32));
        let pkt = crate::players::game_mode_update(p.uuid, mode as i32);
        self.broadcast(pkt);
        true
    }

    fn kill(&mut self, entity: &PlayerRef) {
        // No health or respawn yet: send the player back to spawn instead.
        let spawn = self.spawn_position();
        let to = Teleport {
            dimension: OVERWORLD.to_owned(),
            pos: spawn,
            relative: [false; 3],
            rotation: None,
            relative_rotation: [false; 2],
            facing: None,
        };
        let _ = self.teleport(entity, &to);
    }

    fn give(&mut self, player: &PlayerRef, item: &ItemInput, count: i32) {
        let Some(id) = kiln_data::builtin_id("minecraft:item", item.item.as_str()) else { return };
        let Some(p) = self.players.get_mut(&player.conn) else { return };
        // First empty slot: hotbar, then the main inventory.
        let slot = (36..45).chain(9..36).find(|&s| p.inventory[s].is_none());
        if let Some(slot) = slot {
            p.inventory[slot] = Some((id, count));
            p.inventory_state += 1;
            let pkt = packets::container_set_slot(0, p.inventory_state, slot as i16, Some((id, count)));
            p.send(pkt);
        }
    }

    fn kick(&mut self, player: &PlayerRef, reason: Text) {
        if let Some(p) = self.players.get_mut(&player.conn) {
            p.flush();
            p.sink.disconnect(packets::play_disconnect_text(reason.to_nbt()));
        }
    }

    fn max_players(&self) -> usize {
        self.config.max_players
    }

    fn find_profile(&mut self, name: &str) -> Option<Profile> {
        if let Some(p) = self.players.values().find(|p| p.name.eq_ignore_ascii_case(name)) {
            return Some(Profile { uuid: p.uuid, name: p.name.clone() });
        }
        None
    }

    fn is_operator(&self, profile: &Profile) -> bool {
        self.commands.is_op(&profile.name)
    }

    fn operator_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.commands.ops.iter().cloned().collect();
        v.sort();
        v
    }

    fn set_operator(&mut self, profile: &Profile, op: bool) {
        if op {
            self.commands.ops.insert(profile.name.clone());
        } else {
            self.commands.ops.remove(&profile.name);
        }
        let conn = self.players.iter().find(|(_, p)| p.name == profile.name).map(|(c, _)| *c);
        if let Some(c) = conn {
            self.send_command_tree(c);
        }
    }

    fn difficulty(&self) -> Difficulty {
        self.commands.difficulty
    }

    fn set_difficulty(&mut self, difficulty: Difficulty) {
        self.commands.difficulty = difficulty;
        self.broadcast(packets::change_difficulty(difficulty as u8, false));
    }

    fn set_weather(&mut self, weather: Weather, duration: Option<i32>) -> i32 {
        self.commands.raining = weather != Weather::Clear;
        self.commands.thundering = weather == Weather::Thunder;
        for pkt in self.weather_packets() {
            self.broadcast(pkt);
        }
        duration.unwrap_or(6000)
    }

    fn time(&mut self, _clock: Option<&Identifier>, action: &TimeAction) -> Result<i32, CommandError> {
        let result = match action {
            TimeAction::Set(t) => {
                self.day_time = *t as i64;
                self.reply(kiln_command::tr!("commands.time.set", *t));
                *t
            }
            TimeAction::Add(t) => {
                self.day_time += *t as i64;
                let now = self.day_time as i32;
                self.reply(kiln_command::tr!("commands.time.set", now));
                now
            }
            TimeAction::QueryGameTime => {
                let t = self.game_time as i32;
                self.reply(kiln_command::tr!("commands.time.query", t));
                t
            }
            TimeAction::QueryTime => {
                let t = self.day_time as i32;
                self.reply(kiln_command::tr!("commands.time.query", t));
                t
            }
            _ => return Err(CommandError::new(Text::literal("Not supported yet"))),
        };
        let pkt = self.time_packet();
        self.broadcast(pkt);
        Ok(result)
    }

    fn game_rule(&self, rule: &str) -> GameRuleValue {
        self.commands.game_rules.get(rule).copied().unwrap_or(match kiln_data::game_rule_default(rule) {
            Some(kiln_data::GameRuleDefault::Int(v)) => GameRuleValue::Int(v),
            Some(kiln_data::GameRuleDefault::Bool(b)) => GameRuleValue::Bool(b),
            None => GameRuleValue::Bool(false),
        })
    }

    fn set_game_rule(&mut self, rule: &str, value: GameRuleValue) {
        self.commands.game_rules.insert(rule.to_owned(), value);
    }

    fn seed(&self) -> i64 {
        self.commands.seed
    }

    fn stop(&mut self) {
        self.commands.stop_requested = true;
    }

    fn set_spawn_point(&mut self, player: &PlayerRef, spawn: &SpawnPoint) {
        if let Some(p) = self.players.get_mut(&player.conn) {
            p.respawn = Some(spawn.pos);
        }
    }

    fn set_world_spawn(&mut self, spawn: &SpawnPoint) -> Result<(), CommandError> {
        self.spawn = spawn.pos;
        let pkt = packets::set_default_spawn_position(OVERWORLD, spawn.pos, spawn.yaw, spawn.pitch);
        self.broadcast(pkt);
        Ok(())
    }

    fn kiln_tick(&mut self) -> Vec<Text> {
        let players = self.players.len();
        let chunks = self.world.loaded_chunks();
        let mut lines = vec![Text::literal(format!("{players} players, {chunks} loaded chunks"))];
        match &self.commands.last_report {
            Some(r) => lines.push(Text::literal(r.clone())),
            None => lines.push(Text::literal("No tick statistics yet (reported every 30 s)")),
        }
        lines
    }

    fn kiln_regions(&mut self) -> Vec<Text> {
        vec![Text::literal("1 region (the regionizer is not enabled yet)")]
    }
}
