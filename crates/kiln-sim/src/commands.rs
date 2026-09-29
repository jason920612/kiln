//! Commands: the simulation is the command source and host for `kiln-command`.
//!
//! The dispatcher is generic over a source type without borrowed lifetimes, so `Sim` itself is
//! the source: `CommandSource` says who is executing (a player or the console) while a command
//! runs.

use crate::{Player, Sim, players::chat_disguised};
use bytes::Bytes;
use kiln_command::selector::{Aabb, SelectorTarget, SelectorWorld};
use kiln_command::{
    BossBars, ChatMessage, CommandError, Difficulty, Dispatcher, GameMode, GameRuleValue, Heightmap, Host, Identifier, ItemInput,
    Language, Profile, Scoreboard, Source, SourceStack, SpawnPoint, Teleport, Text, TimeAction, UpdateFlags, Weather,
};
use kiln_link::ConnId;
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_world::{Blocks, ChunkPos};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tracing::info;
use uuid::Uuid;

pub(crate) const OVERWORLD: &str = "minecraft:overworld";
/// Longest tab-completion request answered for players without command-block rights (vanilla).
const MAX_SUGGESTION_LEN: usize = 256;

/// Console rendering of a message: in the language file named by `KILN_LANG` (vanilla's
/// console prints its bundled `en_us`), else translation keys as `key[args]`.
pub(crate) fn console_text(text: &Text) -> String {
    static LANG: OnceLock<Option<Language>> = OnceLock::new();
    let lang = LANG.get_or_init(|| {
        let path = std::env::var_os("KILN_LANG")?;
        let lang = std::fs::read_to_string(&path).ok().and_then(|s| Language::from_json(&s));
        if lang.is_none() {
            tracing::warn!("could not read language file {}", path.to_string_lossy());
        }
        lang
    });
    match lang {
        Some(lang) => text.to_string_in(lang),
        None => text.to_plain(),
    }
}

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
    /// The player's level.
    pub(crate) dim: &'static str,
    mode: GameMode,
    /// The player's team, and their name as the team formats it.
    team: Option<String>,
    display: Text,
    /// A non-player entity: its id, type and eye height (`conn` is then [`NO_CONN`]).
    pub(crate) entity: Option<i32>,
    kind: &'static str,
    size: [f64; 2],
    eye: f64,
    alive: bool,
    /// A `LivingEntity` (players and mobs).
    living: bool,
    /// `Entity.entityTags`.
    tags: Vec<String>,
}

/// The connection of a non-player selector target: no player has it.
const NO_CONN: ConnId = ConnId::MAX;

impl PlayerRef {
    pub(crate) fn of(conn: ConnId, p: &Player, scoreboard: &Scoreboard) -> Self {
        Self {
            conn,
            uuid: p.uuid,
            name: p.name.clone(),
            pos: p.pos,
            rot: p.rot,
            dim: crate::DIMENSIONS[p.dim].0,
            mode: game_mode(p.game_mode),
            team: scoreboard.team_of(&p.name).map(|t| t.name.clone()),
            display: scoreboard.player_display_name(&p.name),
            entity: None,
            kind: "minecraft:player",
            size: [0.6, 1.8],
            eye: 1.62f32 as f64,
            alive: true,
            living: true,
            tags: p.tags(),
        }
    }

    /// A non-player entity in level `dim`.
    fn of_entity(dim: usize, e: &crate::entities::Entity, scoreboard: &Scoreboard) -> Self {
        let key = e.uuid.to_string();
        let path = e.kind.name.strip_prefix("minecraft:").unwrap_or(e.kind.name);
        let (rot, eye, alive) = match &e.phys {
            Some(p) => ([p.y_rot, p.x_rot], p.eye_height as f64, !p.is_removed() && kiln_entity::mob::data(p).is_none_or(|m| !m.is_dead_or_dying())),
            None => ([0.0, 0.0], e.kind.eye_height as f64, !e.removed),
        };
        Self {
            conn: NO_CONN,
            uuid: e.uuid,
            team: scoreboard.team_of(&key).map(|t| t.name.clone()),
            name: key,
            pos: e.pos,
            rot,
            dim: crate::DIMENSIONS[dim].0,
            mode: GameMode::Survival,
            display: match e.phys.as_ref().map(|p| &p.kind) {
                Some(kiln_entity::EntityKind::Item(d)) => crate::command_data::hover_name(&d.stack),
                _ => Text::translate(format!("entity.minecraft.{path}"), Vec::new()),
            },
            entity: Some(e.id),
            kind: e.kind.name,
            size: [e.kind.width as f64, e.kind.height as f64],
            eye,
            alive,
            living: e.phys.as_ref().is_some_and(|p| kiln_entity::mob::data(p).is_some()),
            tags: e.phys.as_ref().map_or_else(Vec::new, |p| crate::command_data::tags_in(&Tag::Compound(p.extra.clone()))),
        }
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
    fn display_name(&self) -> Text {
        self.display.clone()
    }
    fn team(&self) -> Option<&str> {
        self.team.as_deref()
    }
    fn entity_type(&self) -> &str {
        self.kind
    }
    fn position(&self) -> [f64; 3] {
        self.pos
    }
    fn rotation(&self) -> [f32; 2] {
        self.rot
    }
    fn dimension(&self) -> &str {
        self.dim
    }
    fn bounding_box(&self) -> Aabb {
        let [x, y, z] = self.pos;
        let (w, h) = (self.size[0] / 2.0, self.size[1]);
        Aabb { min: [x - w, y, z - w], max: [x + w, y + h, z + w] }
    }
    fn eye_height(&self) -> f64 {
        // `Entity.getEyeHeight` is a float.
        self.eye
    }
    fn is_alive(&self) -> bool {
        self.alive
    }
    fn game_mode(&self) -> Option<GameMode> {
        self.entity.is_none().then_some(self.mode)
    }
    fn tags(&self) -> &[String] {
        &self.tags
    }
}

/// Server-wide state the commands change.
pub(crate) struct CommandState {
    pub dispatcher: Arc<Dispatcher<Sim>>,
    /// Who receives feedback for the running command.
    pub source: CommandSource,
    /// Where and as whom it runs (`execute` changes this per fork).
    pub stack: SourceStack<Sim>,
    pub scoreboard: Scoreboard,
    pub bossbars: BossBars,
    pub storage: kiln_command::CommandStorage,
    /// Data packs, their functions and the scheduled functions.
    pub packs: crate::datapacks::Packs,
    /// Operators by name (permission level 4).
    pub ops: std::collections::HashSet<String>,
    pub difficulty: Difficulty,
    pub game_rules: HashMap<String, GameRuleValue>,
    pub seed: i64,
    /// Shuffle state for `@r` / `sort=random` (xorshift).
    pub rng: u64,
    pub stop_requested: bool,
    /// Last tick statistics report, for `/kiln tick`.
    pub last_report: Option<String>,
    /// Packets `/kiln use` made for players, handled with the next tick's packets.
    pub injected: Vec<(ConnId, kiln_link::PlayIn)>,
    /// `save-on` / `save-off`.
    pub auto_save: bool,
    /// `save-all`: saved at the end of the tick's serial phase.
    pub save_requested: bool,
    /// `setidletimeout` in minutes (0: off).
    pub idle_timeout: i32,
    /// `defaultgamemode` for worlds without a `level.dat`.
    pub default_game_mode: Option<u8>,
    /// `Stopwatches`: id, start and time accumulated before this run, in load order.
    pub stopwatches: Vec<(String, std::time::Instant, u64)>,
    pub stopwatches_dirty: bool,
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
            stack: SourceStack::new(Text::literal("Server"), OVERWORLD, [0.0; 3]),
            scoreboard: Scoreboard::default(),
            bossbars: BossBars::default(),
            storage: kiln_command::CommandStorage::default(),
            packs: crate::datapacks::Packs::new(None, "work/generated".into(), crate::datapacks::PackConfig { enabled: vec!["vanilla".into()], disabled: Vec::new(), features: None }),
            ops,
            difficulty: Difficulty::Normal,
            game_rules: HashMap::new(),
            seed: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
            stop_requested: false,
            last_report: None,
            injected: Vec::new(),
            auto_save: true,
            save_requested: false,
            idle_timeout: 0,
            default_game_mode: None,
            stopwatches: Vec::new(),
            stopwatches_dirty: false,
        }
    }
}

impl Sim {
    /// Runs `f` on the data of the mob a command targets (`None`: not a live mob).
    fn with_mob<R>(&mut self, entity: &PlayerRef, f: impl FnOnce(&mut kiln_entity::mob::MobData) -> R) -> Option<R> {
        let id = entity.entity?;
        let dim = crate::dim_id(entity.dim)?;
        for r in self.dims[dim].regions.iter_mut() {
            if let Some(e) = r.part_mut().0.list.iter_mut().find(|e| e.id == id) {
                return e.phys.as_mut().and_then(kiln_entity::mob::data_mut).map(f);
            }
        }
        None
    }

    /// Replaces the contents of the block entity at `pos` with `fields` (position and id kept)
    /// and sends Block Entity Data to players with the chunk if vanilla would. Returns whether
    /// the contents changed.
    pub(crate) fn load_block_entity(&mut self, dim: crate::DimId, pos: [i32; 3], fields: &[(String, Tag)]) -> bool {
        let [x, y, z] = pos;
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let chunk_pos = ChunkPos::of_block(x, z);
        // A live container writes its state into the chunk first.
        if let Some(region) = self.dims[dim].regions.at_mut(chunk_pos.cell()) {
            let (cells, part) = region.cells_and_part_mut();
            if let Some(chunk) = cells.chunk_mut(chunk_pos) {
                part.1.containers.store(chunk_pos, chunk);
            }
        }
        let Some(chunk) = self.dims[dim].regions.chunk_mut(chunk_pos) else { return false };
        let Some(old) = chunk.block_entity(lx, y, lz).cloned() else { return false };
        let mut be = kiln_world::block_entity::BlockEntity::new(old.kind);
        if let Tag::Compound(out) = &mut be.nbt {
            out.extend(fields.iter().filter(|(k, _)| !matches!(k.as_str(), "id" | "x" | "y" | "z")).cloned());
        }
        if be == old {
            return false;
        }
        chunk.set_block_entity(lx, y, lz, be);
        // A container's live state follows the new data.
        if let Some(region) = self.dims[dim].regions.at_mut(chunk_pos.cell()) {
            let (cells, part) = region.cells_and_part_mut();
            let be = cells.chunk(chunk_pos).and_then(|c| c.block_entity(lx, y, lz));
            part.1.containers.reload(kiln_blocks::BlockPos::new(x, y, z), be);
            part.1.sculk.reload(kiln_blocks::BlockPos::new(x, y, z), be);
        }
        let Some((kind, tag)) = self.dims[dim].regions.block_entity_data(x, y, z) else { return true };
        let pkt = packets::block_entity_data(pos, kind as i32, &tag);
        for p in self.players.values_mut().filter(|p| p.dim == dim && p.sent_chunks.contains(&chunk_pos)) {
            p.send(pkt.clone());
        }
        true
    }

    pub(crate) fn rule_bool(&self, rule: &str) -> bool {
        matches!(Host::game_rule(self, rule), GameRuleValue::Bool(true))
    }

    pub(crate) fn rule_int(&self, rule: &str) -> i32 {
        match Host::game_rule(self, rule) {
            GameRuleValue::Int(v) => v,
            GameRuleValue::Bool(_) => 0,
        }
    }

    /// Loads block entity data given with a block (`BlockInput.place`); whether it changed.
    fn load_nbt(&mut self, dim: crate::DimId, pos: [i32; 3], nbt: Option<&Tag>) -> bool {
        match nbt {
            Some(Tag::Compound(fields)) => self.load_block_entity(dim, pos, fields),
            _ => false,
        }
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
        let start = self.source_stack(source);
            let stack = std::mem::replace(&mut self.commands.stack, start);
        if let Err(e) = dispatcher.execute(command, self) {
            for line in e.chat_lines(command) {
                self.reply(line);
            }
        }
        self.commands.source = previous;
        self.commands.stack = stack;
        self.flush_scoreboard();
    }

    /// Sends the scoreboard and boss bar packets queued by changes (`ServerScoreboard` and
    /// `ServerBossEvent` send them as they happen). Serial phases only.
    pub(crate) fn flush_scoreboard(&mut self) {
        for pkt in self.commands.scoreboard.take_packets() {
            self.broadcast(pkt);
        }
        for (uuid, pkt) in self.commands.bossbars.take_packets() {
            if let Some(p) = self.players.values_mut().find(|p| p.uuid == uuid) {
                p.send(pkt);
            }
        }
    }

    /// The stack a command starts with: the player where they stand, or the console at the
    /// world spawn (`MinecraftServer.createCommandSourceStack`).
    pub(crate) fn source_stack(&self, source: CommandSource) -> SourceStack<Sim> {
        match source {
            CommandSource::Player(conn) => match self.players.get(&conn) {
                Some(p) => SourceStack::of_entity(PlayerRef::of(conn, p, &self.commands.scoreboard)),
                None => SourceStack::new(Text::literal(""), OVERWORLD, [0.0; 3]),
            },
            CommandSource::Console => {
                SourceStack::new(Text::literal("Server"), OVERWORLD, self.spawn.map(|v| v as f64))
            }
        }
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
            let source = CommandSource::Player(conn);
            let previous = std::mem::replace(&mut self.commands.source, source);
            let start = self.source_stack(source);
            let stack = std::mem::replace(&mut self.commands.stack, start);
            let pkt = dispatcher.suggestions_packet(id, &text, self);
            self.commands.source = previous;
            self.commands.stack = stack;
            if let Some(p) = self.players.get_mut(&conn) {
                p.send(pkt);
            }
        }
    }

    /// System message to the current command source.
    fn reply(&mut self, text: Text) {
        match self.commands.source {
            CommandSource::Console => info!("{}", console_text(&text)),
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
}

impl Source for Sim {
    type Entity = PlayerRef;

    fn permission_level(&self) -> u8 {
        match self.commands.source {
            CommandSource::Console => 4,
            CommandSource::Player(conn) => self.permission_level_of(conn),
        }
    }

    fn stack(&self) -> &SourceStack<Sim> {
        &self.commands.stack
    }

    fn stack_mut(&mut self) -> &mut SourceStack<Sim> {
        &mut self.commands.stack
    }

    fn registry_ids(&self, registry: &str) -> Vec<String> {
        match registry {
            "minecraft:advancement" => self.advancements.list.iter().map(|a| a.id.clone()).collect(),
            "minecraft:recipe" => self.rules.recipes.recipes().iter().map(|r| r.id.clone()).collect(),
            "minecraft:worldgen/template_pool" => crate::world_state::worldgen_ids("worldgen/template_pool").clone(),
            "minecraft:loot_table" => self.loot.as_ref().map_or_else(Vec::new, |l| l.table_ids().iter().map(|i| i.to_string()).collect()),
            "minecraft:context_int_provider" => {
                self.loot.as_ref().map_or_else(Vec::new, |l| l.ids(kiln_loot::Kind::IntProvider).iter().map(|i| i.to_string()).collect())
            }
            "minecraft:context_float_provider" => {
                self.loot.as_ref().map_or_else(Vec::new, |l| l.ids(kiln_loot::Kind::FloatProvider).iter().map(|i| i.to_string()).collect())
            }
            "minecraft:slot_source" => {
                self.loot.as_ref().map_or_else(Vec::new, |l| l.ids(kiln_loot::Kind::SlotSource).iter().map(|i| i.to_string()).collect())
            }
            "minecraft:item_modifier" => {
                self.loot.as_ref().map_or_else(Vec::new, |l| l.ids(kiln_loot::Kind::Modifier).iter().map(|i| i.to_string()).collect())
            }
            _ => Vec::new(),
        }
    }

    fn player_names(&self) -> Vec<String> {
        self.players.values().map(|p| p.name.clone()).collect()
    }

    fn definition_error(&self, registry: &str, definition: &Tag) -> Option<String> {
        self.definition_error_of(registry, definition)
    }

    // `dimensions` keeps the default (all three vanilla levels): the nether and the end exist
    // for dimension arguments but never have loaded chunks.

    fn fork_limit(&self) -> usize {
        Host::game_rule(self, "minecraft:max_command_forks").command_result().max(0) as usize
    }

    fn command_limit(&self) -> i32 {
        Host::game_rule(self, "minecraft:max_command_sequence_length").command_result()
    }
}

impl SelectorWorld for Sim {
    fn players(&self) -> Vec<PlayerRef> {
        let sb = &self.commands.scoreboard;
        let mut v: Vec<PlayerRef> = self.players.iter().map(|(&c, p)| PlayerRef::of(c, p, sb)).collect();
        v.sort_by_key(|p| p.conn); // join order
        v
    }

    fn entities(&self, dimension: Option<&str>, _area: Option<&Aabb>) -> Vec<PlayerRef> {
        // Players, then the other entities of each level by id.
        let mut all = self.players();
        let sb = &self.commands.scoreboard;
        for (i, d) in self.dims.iter().enumerate() {
            let mut list: Vec<PlayerRef> = d
                .regions
                .iter()
                .flat_map(|r| r.part().0.list.iter())
                .filter(|e| !e.removed)
                .map(|e| PlayerRef::of_entity(i, e, sb))
                .collect();
            list.sort_by_key(|p| p.entity);
            all.extend(list);
        }
        if let Some(d) = dimension {
            all.retain(|p| p.dim == d);
        }
        all
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

    fn scoreboard(&self) -> Option<&Scoreboard> {
        Some(&self.commands.scoreboard)
    }
}

impl Host for Sim {
    fn send_success(&mut self, text: Text, broadcast: bool) {
        if self.commands.stack.silent {
            return;
        }
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
        info!("{}", console_text(&message.to_text()));
        self.broadcast(chat_disguised(&message));
    }

    fn send_chat(&mut self, player: &PlayerRef, message: ChatMessage) {
        self.send_to(player.conn, chat_disguised(&message));
    }

    fn send_chat_to_source(&mut self, message: ChatMessage) {
        match self.commands.source {
            CommandSource::Player(conn) => self.send_to(conn, chat_disguised(&message)),
            CommandSource::Console => info!("{}", console_text(&message.to_text())),
        }
    }

    fn teleport(&mut self, entity: &PlayerRef, to: &Teleport) -> Result<(), CommandError> {
        let Some(dim) = crate::dim_id(&to.dimension) else {
            return Err(CommandError::new(Text::literal("Unknown dimension")));
        };
        let Some(p) = self.players.get_mut(&entity.conn) else { return Ok(()) };
        let rot = match (to.facing, to.rotation) {
            (Some(f), _) => kiln_command::host::look_at([to.pos[0], to.pos[1] + 1.62, to.pos[2]], f),
            (None, Some(r)) => r,
            (None, None) => p.rot,
        };
        if p.dim == dim {
            p.teleport(to.pos, rot, self.game_time);
        } else {
            // `ServerPlayer.teleport` into another level.
            self.change_dimension(entity.conn, dim, to.pos, rot);
        }
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
        if let Some(id) = entity.entity {
            // Mobs die (`hurt(genericKill)`); other entities are discarded.
            let Some(dim) = crate::dim_id(entity.dim) else { return };
            for r in self.dims[dim].regions.iter_mut() {
                if let Some(e) = r.part_mut().0.list.iter_mut().find(|e| e.id == id) {
                    if let Some(p) = e.phys.as_mut() {
                        if kiln_entity::mob::data(p).is_some() {
                            kiln_entity::mob::kill(p);
                        } else {
                            p.removed = Some(kiln_entity::entity::RemovalReason::Killed);
                            // `EndCrystal.kill`: the fight hears of it.
                            if p.type_name == "minecraft:end_crystal" && dim == crate::END_ID {
                                let ev = kiln_entity::level::DragonFightEvent::CrystalDestroyed {
                                    crystal: p.id,
                                    uuid: p.uuid,
                                    pos: p.position(),
                                    kind: kiln_entity::level::DamageKind::Generic,
                                    attacker: None,
                                };
                                self.dragon_fight.send(crate::dragon_fight::FightMsg::Entity(ev));
                            }
                        }
                    }
                    break;
                }
            }
            return;
        }
        let (rules, game_time) = (self.damage_rules(), self.game_time);
        let Some(p) = self.players.get_mut(&entity.conn) else { return };
        let (mut spawns, mut deaths) = (Vec::new(), Vec::new());
        let mut ctx = crate::health::DamageCtx { rules, game_time, spawns: &mut spawns, deaths: &mut deaths, level_rng: None };
        p.hurt(f32::MAX, &crate::health::Cause::Kill.into(), &mut ctx);
        let dim = p.dim;
        self.dims[dim].spawns.extend(spawns);
        self.announce_deaths(deaths);
    }

    fn give(&mut self, player: &PlayerRef, item: &ItemInput, count: i32) {
        let Some(id) = kiln_data::builtin_id("minecraft:item", item.item.as_str()) else { return };
        let Some(p) = self.players.get_mut(&player.conn) else { return };
        let dim = p.dim;
        // `ItemInput.createItemStack`: the item with the component changes.
        let patch = {
            let mut m = kiln_item::value::MapBuilder::new();
            for (component, snbt) in &item.components {
                match snbt {
                    Some(s) => {
                        if let Ok(v) = kiln_item::component::predicate::parse_snbt(s) {
                            m.put(component.as_str(), v);
                        }
                    }
                    None => {
                        m.put(&format!("!{}", component.as_str()), kiln_item::value::Value::empty_map());
                    }
                }
            }
            kiln_item::DataComponentPatch::from_value(&m.build()).unwrap_or_default()
        };
        // Stacks of at most the item's size; what does not fit is thrown (`GiveCommand`).
        let mut left = count;
        let max = kiln_item::ItemStack::from_parts(id, 1, patch.clone()).max_stack_size();
        while left > 0 {
            let n = left.min(max);
            left -= n;
            let mut stack = kiln_item::ItemStack::from_parts(id, n, patch.clone());
            p.add_to_inventory(&mut stack);
            if !stack.is_empty() {
                let spawn = p.throw(stack);
                self.dims[dim].spawns.push(spawn);
            }
        }
        let rules = self.rules.clone();
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.broadcast_changes(env));
        self.dims[dim].spawns.extend(spawns);
    }

    fn add_effect(&mut self, entity: &PlayerRef, effect: &Identifier, duration: i32, amplifier: i32, show_particles: bool) -> Option<bool> {
        let id = crate::effects::effect_id(effect.as_str())?;
        let fx = crate::effects::Effect::new(id, duration, amplifier, false, show_particles, show_particles);
        if entity.entity.is_some() {
            return self.with_mob(entity, |m| kiln_entity::mob::effects::add_quiet(m, fx));
        }
        let p = self.players.get_mut(&entity.conn)?;
        Some(p.add_effect(fx))
    }

    fn remove_effect(&mut self, entity: &PlayerRef, effect: &Identifier) -> Option<bool> {
        let id = crate::effects::effect_id(effect.as_str())?;
        if entity.entity.is_some() {
            return self.with_mob(entity, |m| kiln_entity::mob::effects::remove(m, id));
        }
        Some(self.players.get_mut(&entity.conn)?.remove_effect(id))
    }

    fn clear_effects(&mut self, entity: &PlayerRef) -> Option<bool> {
        if entity.entity.is_some() {
            return self.with_mob(entity, kiln_entity::mob::effects::remove_all);
        }
        Some(self.players.get_mut(&entity.conn)?.remove_all_effects())
    }

    fn add_experience(&mut self, player: &PlayerRef, amount: i32, kind: kiln_command::vanilla::experience::XpKind) {
        let Some(p) = self.players.get_mut(&player.conn) else { return };
        match kind {
            kiln_command::vanilla::experience::XpKind::Points => p.give_experience_points(amount),
            kiln_command::vanilla::experience::XpKind::Levels => p.give_experience_levels(amount),
        }
    }

    fn set_experience(&mut self, player: &PlayerRef, amount: i32, kind: kiln_command::vanilla::experience::XpKind) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        match kind {
            // `ServerPlayer.setExperiencePoints`: the progress into the current level.
            kiln_command::vanilla::experience::XpKind::Points => {
                let need = p.xp_needed_for_next_level() as f32;
                if amount as f32 >= need {
                    return false;
                }
                p.xp_progress = (amount as f32 / need).clamp(0.0, (need - 1.0) / need);
            }
            kiln_command::vanilla::experience::XpKind::Levels => p.xp_level = amount,
        }
        p.sent_xp = None;
        true
    }

    fn query_experience(&mut self, player: &PlayerRef, kind: kiln_command::vanilla::experience::XpKind) -> i32 {
        let Some(p) = self.players.get(&player.conn) else { return 0 };
        match kind {
            kiln_command::vanilla::experience::XpKind::Points => (p.xp_progress * p.xp_needed_for_next_level() as f32).floor() as i32,
            kiln_command::vanilla::experience::XpKind::Levels => p.xp_level,
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

    fn advancement_name(&self, id: &str) -> Option<Text> {
        self.advancements.get(id).map(|i| self.advancements.list[i].name())
    }

    fn advancement_ids(&self) -> Vec<String> {
        self.advancements.list.iter().map(|a| a.id.clone()).collect()
    }

    fn advancement_criteria(&self, id: &str) -> Vec<String> {
        self.advancements.get(id).map_or_else(Vec::new, |i| self.advancements.list[i].criteria.iter().map(|(n, _)| n.clone()).collect())
    }

    fn advancement_parents(&self, id: &str) -> Vec<String> {
        let t = &self.advancements;
        let mut out = Vec::new();
        let mut at = t.get(id).and_then(|i| t.parent[i]);
        while let Some(i) = at {
            out.push(t.list[i].id.clone());
            at = t.parent[i];
        }
        out
    }

    fn advancement_descendants(&self, id: &str) -> Vec<String> {
        fn walk(t: &crate::advancements::Advancements, i: usize, out: &mut Vec<String>) {
            for &c in &t.children[i] {
                out.push(t.list[c].id.clone());
                walk(t, c, out);
            }
        }
        let mut out = Vec::new();
        if let Some(i) = self.advancements.get(id) {
            walk(&self.advancements, i, &mut out);
        }
        out
    }

    fn change_advancement(&mut self, player: &PlayerRef, id: &str, revoke: bool) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        let pa = &mut p.advancements;
        let Some(i) = pa.data.get(id) else { return false };
        let n = pa.data.list[i].criteria.len();
        if revoke {
            if !pa.progress[i].has_progress() {
                return false;
            }
            for c in 0..n {
                pa.revoke(i, c);
            }
        } else {
            if pa.is_done(i) {
                return false;
            }
            let now = crate::advancements::progress::now_millis();
            for c in 0..n {
                if !pa.criterion_done(i, c) {
                    pa.award(i, c, now);
                }
            }
            self.grant_completed(player.conn);
        }
        true
    }

    fn change_criterion(&mut self, player: &PlayerRef, id: &str, criterion: &str, revoke: bool) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        let pa = &mut p.advancements;
        let Some(i) = pa.data.get(id) else { return false };
        let Some(c) = pa.data.list[i].criterion_index(criterion) else { return false };
        if revoke {
            return pa.revoke(i, c);
        }
        let changed = pa.award(i, c, crate::advancements::progress::now_millis());
        self.grant_completed(player.conn);
        changed
    }

    fn flush_advancements(&mut self, player: &PlayerRef, show: bool) {
        if let Some(p) = self.players.get_mut(&player.conn)
            && let Some(pkt) = p.advancements.flush(show)
        {
            p.send(pkt);
        }
    }

    fn kiln_open_recipe_book(&mut self, player: &PlayerRef) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        p.recipe_book.settings[0] = (true, false);
        let pkt = kiln_inventory::recipe::book::recipe_book_settings(&p.recipe_book.settings);
        p.send(pkt);
        true
    }

    fn recipe_ids(&self) -> Vec<String> {
        self.rules.recipes.recipes().iter().filter(|r| !r.recipe.is_special()).map(|r| r.id.clone()).collect()
    }

    fn change_recipes(&mut self, player: &PlayerRef, recipes: &[String], take: bool) -> i32 {
        let rules = self.rules.clone();
        let Some(p) = self.players.get_mut(&player.conn) else { return 0 };
        let idx: Vec<usize> = recipes.iter().filter_map(|id| rules.recipes.index_of(id)).collect();
        if take { p.reset_recipes(&rules, &idx) } else { p.award_recipes(&rules, &idx) }
    }

    /// Online players, then (offline mode) the offline profile vanilla falls back to when the
    /// name has no Mojang account: the name in lower case and its offline UUID. Kiln does not
    /// look names up at Mojang, so names of real accounts resolve offline too.
    fn find_profile(&mut self, name: &str) -> Option<Profile> {
        if let Some(p) = self.players.values().find(|p| p.name.eq_ignore_ascii_case(name)) {
            return Some(Profile { uuid: p.uuid, name: p.name.clone() });
        }
        if self.config.online_mode {
            return None;
        }
        let lower = name.to_ascii_lowercase();
        Some(Profile { uuid: offline_uuid(&lower), name: lower })
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
        self.sync_ops();
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
        self.command_weather(weather, duration)
    }

    fn time(&mut self, clock: Option<&Identifier>, action: &TimeAction) -> Result<i32, CommandError> {
        self.command_time(clock.map(Identifier::as_str), action)
    }

    fn time_markers(&self, clock: Option<&Identifier>) -> Vec<String> {
        crate::weather::time_markers(clock.map_or(OVERWORLD, Identifier::as_str)).iter().map(|(m, _)| (*m).to_owned()).collect()
    }

    fn timelines(&self, clock: Option<&Identifier>) -> Vec<String> {
        crate::weather::timelines(clock.map_or(OVERWORLD, Identifier::as_str)).iter().map(|(t, _)| (*t).to_owned()).collect()
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
        // `MinecraftServer.onGameRuleChanged`: clients stop or restart their clocks.
        if rule == "minecraft:advance_time" {
            let pkt = self.time_packet();
            self.broadcast(pkt);
        }
        // The locator bar's connections break or are made again.
        if rule == "minecraft:locator_bar" {
            self.locator_bar_changed();
        }
    }

    fn seed(&self) -> i64 {
        self.commands.seed
    }

    fn stop(&mut self) {
        self.commands.stop_requested = true;
    }

    fn summon(&mut self, entity: &Identifier, pos: [f64; 3], nbt: Option<&Tag>, initialize: bool) -> Result<Text, CommandError> {
        let dim = crate::dim_id(kiln_command::host::Source::dimension(self)).unwrap_or(0);
        let seed = crate::mobs::loot_seed(self.config.noise.as_ref().map_or(0, |n| n.seed), self.game_time, self.dims[dim].spawns.len() as i32, 0x73756d6d);
        let name = crate::mobs::summon(&mut self.dims[dim].spawns, entity.as_str(), pos, nbt, initialize, self.commands.difficulty as u8, self.game_time, seed)
            .ok_or_else(|| CommandError::new(kiln_command::tr!("commands.summon.failed")))?;
        Ok(Text::raw(name))
    }

    fn set_spawn_point(&mut self, player: &PlayerRef, spawn: &SpawnPoint) {
        if let Some(p) = self.players.get_mut(&player.conn) {
            p.respawn = Some(spawn.pos);
            p.respawn_dim = crate::dim_id(&spawn.dimension).unwrap_or(crate::OVERWORLD_ID);
            p.respawn_forced = true;
            p.respawn_angle = spawn.yaw;
        }
    }

    fn set_world_spawn(&mut self, spawn: &SpawnPoint) -> Result<(), CommandError> {
        self.spawn = spawn.pos;
        self.spawn_rot = [spawn.yaw, spawn.pitch];
        let pkt = packets::set_default_spawn_position(OVERWORLD, spawn.pos, spawn.yaw, spawn.pitch);
        self.broadcast(pkt);
        Ok(())
    }

    fn kiln_tick(&mut self) -> Vec<Text> {
        let players = self.players.len();
        let chunks: usize = self.dims.iter().map(|d| kiln_world::Blocks::loaded_chunks(&d.regions)).sum();
        let mut lines = vec![Text::literal(format!("{players} players, {chunks} loaded chunks"))];
        match &self.commands.last_report {
            Some(r) => lines.push(Text::literal(r.clone())),
            None => lines.push(Text::literal("No tick statistics yet (reported every 30 s)")),
        }
        lines
    }

    fn kiln_use(&mut self, player: &PlayerRef, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&player.conn) else { return false };
        let pkt = kiln_link::PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: p.ack_block_changes.max(0) };
        self.commands.injected.push((player.conn, pkt));
        true
    }

    fn kiln_interact(&mut self, player: &PlayerRef, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&player.conn) else { return false };
        let at = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5];
        let nearest = self.dims[p.dim]
            .regions
            .iter()
            .flat_map(|r| r.part().0.list.iter())
            .filter(|e| !e.removed)
            .map(|e| (e.id, (0..3).map(|i| (e.pos[i] - at[i]).powi(2)).sum::<f64>()))
            .filter(|&(_, d)| d < 1.5 * 1.5)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((entity_id, _)) = nearest else { return false };
        let pkt = kiln_link::PlayIn::Interact {
            entity_id,
            hand: kiln_proto::packets::serverbound::Hand::Main,
            location: [0.0, 0.5, 0.0],
            sneaking: p.sneaking,
        };
        self.commands.injected.push((player.conn, pkt));
        true
    }

    fn kiln_break(&mut self, player: &PlayerRef, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&player.conn) else { return false };
        let pkt = kiln_link::PlayIn::PlayerAction { action: crate::digging::START_DESTROY_BLOCK, pos, face: 1, sequence: p.ack_block_changes.max(0) };
        self.commands.injected.push((player.conn, pkt));
        true
    }

    fn kiln_regions(&mut self) -> Vec<Text> {
        let mut players: std::collections::BTreeMap<(crate::DimId, kiln_region::RegionId), usize> = Default::default();
        for p in self.players.values() {
            *players.entry((p.dim, p.region)).or_default() += 1;
        }
        let mut lines = vec![Text::literal(format!("{} regions", self.region_count()))];
        for (dim, r) in self.dims.iter().enumerate().flat_map(|(i, d)| d.regions.iter().map(move |r| (i, r))) {
            let chunks: usize = r.cells().iter().map(|(_, c)| c.len()).sum();
            let anchor = r.anchor();
            lines.push(Text::literal(format!(
                "{} #{}: anchor cell {},{} (block {},{}), {} cells, {} chunks, {} players",
                crate::DIMENSIONS[dim].0,
                r.id().0,
                anchor.x,
                anchor.z,
                anchor.x * kiln_region::CELL_BLOCKS,
                anchor.z * kiln_region::CELL_BLOCKS,
                r.len(),
                chunks,
                players.get(&(dim, r.id())).copied().unwrap_or(0)
            )));
        }
        lines
    }

    fn is_chunk_loaded(&self, dimension: &str, cx: i32, cz: i32) -> bool {
        crate::dim_id(dimension).is_some_and(|d| self.dims[d].regions.chunk(ChunkPos::new(cx, cz)).is_some())
    }

    fn build_height(&self, dimension: &str) -> (i32, i32) {
        let d = self.dims[crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID)].provider.dimension;
        (d.min_y, d.min_y + d.height)
    }

    /// Unloaded positions read as void air (commands check loadedness first).
    fn block_state(&mut self, dimension: &str, pos: [i32; 3]) -> u16 {
        let [x, y, z] = pos;
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        self.dims[dim].regions.get_block(x, y, z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR)
    }

    /// `BlockEntity.saveWithFullMetadata`.
    fn entity_data(&mut self, entity: &PlayerRef) -> Option<Tag> {
        self.entity_data_of(entity.conn, entity.entity, entity.dim)
    }

    fn block_entity(&mut self, dimension: &str, pos: [i32; 3]) -> Option<Tag> {
        let [x, y, z] = pos;
        let dim = crate::dim_id(dimension)?;
        // A live container writes its state into the chunk first.
        if let Some(region) = self.dims[dim].regions.at_mut(ChunkPos::of_block(x, z).cell()) {
            let (cells, part) = region.cells_and_part_mut();
            if let Some(chunk) = cells.chunk_mut(ChunkPos::of_block(x, z)) {
                part.1.containers.store(ChunkPos::of_block(x, z), chunk);
            }
        }
        let chunk = self.dims[dim].regions.chunk(ChunkPos::of_block(x, z))?;
        chunk.block_entity((x & 15) as usize, y, (z & 15) as usize).map(|be| be.saved(pos))
    }

    /// `Level.setBlock` with the given flags; block entity data then replaces the block
    /// entity's contents. Succeeds if the state or the block entity's data changed.
    fn set_block(&mut self, dimension: &str, pos: [i32; 3], state: u16, nbt: Option<&Tag>, flags: UpdateFlags) -> bool {
        let at = block_pos(pos);
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        let state_changed = self.with_level_in(dim, pos, |level| kiln_blocks::set_block(level, at, state, flags.0)).unwrap_or(false);
        self.load_nbt(dim, pos, nbt) || state_changed
    }

    /// `BlockInput.place`: the state shaped by its neighbours except for the properties the
    /// command names.
    fn place_block(&mut self, dimension: &str, pos: [i32; 3], block: &kiln_command::BlockInput, flags: UpdateFlags) -> bool {
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        let at = block_pos(pos);
        let defined = block
            .properties
            .iter()
            .map(|&p| (p.to_owned(), kiln_blocks::state::get(block.state, p).unwrap_or_default().to_owned()))
            .collect();
        let input = kiln_blocks::commands::BlockInput { state: block.state, defined };
        let state_changed = self.with_level_in(dim, pos, |level| input.place(level, at, flags.0)).unwrap_or(false);
        self.load_nbt(dim, pos, block.nbt.as_ref()) || state_changed
    }

    fn update_neighbours(&mut self, dimension: &str, pos: [i32; 3], old: u16) {
        let at = block_pos(pos);
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        self.with_level_in(dim, pos, |level| kiln_blocks::commands::update_neighbours_on_block_set(level, at, old));
    }

    fn destroy_block(&mut self, dimension: &str, pos: [i32; 3], drop: bool) -> bool {
        let at = block_pos(pos);
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        self.with_level_in(dim, pos, |level| kiln_blocks::destroy_block(level, at, drop, kiln_blocks::flags::LIMIT)).unwrap_or(false)
    }

    fn height(&mut self, dimension: &str, heightmap: Heightmap, x: i32, z: i32) -> i32 {
        let (min_y, max_y) = Host::build_height(self, dimension);
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        let Some(chunk) = self.dims[dim].regions.chunk(ChunkPos::of_block(x, z)) else { return min_y };
        (min_y..max_y)
            .rev()
            .find(|&y| heightmap.counts(chunk.get((x & 15) as usize, y, (z & 15) as usize)))
            .map_or(min_y, |y| y + 1)
    }

    /// The stored biome of the 4x4x4 cell holding `pos` (vanilla adds a seeded jitter between
    /// neighbouring cells, which only matters at biome borders).
    fn biome(&mut self, dimension: &str, pos: [i32; 3]) -> Option<String> {
        let [x, y, z] = pos;
        let chunk = self.dims[crate::dim_id(dimension)?].regions.chunk(ChunkPos::of_block(x, z))?;
        let rel = y - chunk.min_y();
        let section = chunk.sections.get(usize::try_from(rel >> 4).ok()?)?;
        let id = match &section.biomes {
            kiln_world::section::Biomes::Single(b) => b.to_owned(),
            kiln_world::section::Biomes::Cells(cells) => {
                cells[((((rel & 15) >> 2) << 4) | (((z & 15) >> 2) << 2) | ((x & 15) >> 2)) as usize]
            }
        };
        let biomes = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome")?.1;
        biomes.get(id as usize).map(|b| (*b).to_owned())
    }

    fn scoreboard_mut(&mut self) -> Option<&mut Scoreboard> {
        Some(&mut self.commands.scoreboard)
    }

    fn bossbars(&self) -> Option<&BossBars> {
        Some(&self.commands.bossbars)
    }

    fn bossbars_mut(&mut self) -> Option<&mut BossBars> {
        Some(&mut self.commands.bossbars)
    }

    fn send_packet(&mut self, player: &PlayerRef, packet: Bytes) {
        self.send_to(player.conn, packet);
    }

    fn functions(&self) -> Option<&kiln_command::functions::FunctionLibrary> {
        Some(&self.commands.packs.library)
    }

    fn timers(&self) -> Option<&kiln_command::functions::TimerQueue> {
        Some(&self.commands.packs.timers)
    }

    fn timers_mut(&mut self) -> Option<&mut kiln_command::functions::TimerQueue> {
        Some(&mut self.commands.packs.timers)
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn data_packs(&self) -> Option<kiln_command::functions::DataPacks> {
        Some(self.commands.packs.snapshot())
    }

    fn refresh_packs(&mut self) {
        self.commands.packs.discover();
    }

    fn reload_packs(&mut self, selected: Option<Vec<String>>) {
        self.reload_data_packs(selected);
    }

    fn create_pack(&mut self, id: &str, description: &Text) -> Result<(), CommandError> {
        self.create_data_pack(id, description)
    }

    /// Kept in memory only (not saved with the world yet).
    fn storage_mut(&mut self) -> Option<&mut kiln_command::CommandStorage> {
        Some(&mut self.commands.storage)
    }

    // ---- server administration ----

    fn access(&self) -> Option<kiln_link::access::SharedAccess> {
        Some(self.config.access.clone())
    }

    fn player_ip(&self, player: &PlayerRef) -> Option<String> {
        self.players.get(&player.conn)?.address.map(|a| a.to_string())
    }

    fn set_auto_save(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.commands.auto_save, on) != on
    }

    fn save_all(&mut self, _flush: bool) -> bool {
        self.commands.save_requested = true;
        true
    }

    /// Without `force-gamemode`, players keep their modes (`enforceGameTypeForPlayers(null)`).
    fn set_default_game_mode(&mut self, mode: GameMode) -> i32 {
        self.commands.default_game_mode = Some(mode.id() as u8);
        if let Some(s) = self.storage.as_mut() {
            s.level.set_game_type(mode.id() as u8);
        }
        0
    }

    fn set_idle_timeout(&mut self, minutes: i32) {
        self.commands.idle_timeout = minutes;
    }

    fn random_seed(&mut self) -> i64 {
        // xorshift64, as for `@r`.
        let r = &mut self.commands.rng;
        *r ^= *r << 13;
        *r ^= *r >> 7;
        *r ^= *r << 17;
        *r as i64
    }

    // ---- stopwatches and post effects ----

    fn stopwatch_ids(&self) -> Vec<String> {
        self.commands.stopwatches.iter().map(|(id, ..)| id.clone()).collect()
    }

    fn stopwatch_create(&mut self, id: &str) -> bool {
        if self.commands.stopwatches.iter().any(|(i, ..)| i == id) {
            return false;
        }
        self.commands.stopwatches.push((id.to_owned(), std::time::Instant::now(), 0));
        self.commands.stopwatches_dirty = true;
        true
    }

    fn stopwatch_seconds(&self, id: &str) -> Option<f64> {
        let (_, start, before) = self.commands.stopwatches.iter().find(|(i, ..)| i == id)?;
        let ms = *before + start.elapsed().as_millis() as u64;
        Some(ms as f64 / 1000.0)
    }

    fn stopwatch_restart(&mut self, id: &str) -> bool {
        let Some(w) = self.commands.stopwatches.iter_mut().find(|(i, ..)| i == id) else { return false };
        w.1 = std::time::Instant::now();
        w.2 = 0;
        self.commands.stopwatches_dirty = true;
        true
    }

    fn stopwatch_remove(&mut self, id: &str) -> bool {
        let before = self.commands.stopwatches.len();
        self.commands.stopwatches.retain(|(i, ..)| i != id);
        self.commands.stopwatches_dirty |= self.commands.stopwatches.len() != before;
        self.commands.stopwatches.len() != before
    }

    fn post_effects(&self, player: &PlayerRef) -> Vec<String> {
        self.players.get(&player.conn).map_or_else(Vec::new, |p| p.post_effects.clone())
    }

    fn add_post_effect(&mut self, player: &PlayerRef, id: &str) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        if p.post_effects.iter().any(|e| e == id) {
            return false;
        }
        p.post_effects.push(id.to_owned());
        p.post_effects_dirty = true;
        true
    }

    fn remove_post_effect(&mut self, player: &PlayerRef, id: &str) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        let Some(i) = p.post_effects.iter().position(|e| e == id) else { return false };
        p.post_effects.remove(i);
        p.post_effects_dirty = true;
        true
    }

    fn waypoints(&self, dimension: &str) -> Vec<Text> {
        crate::dim_id(dimension).map_or_else(Vec::new, |d| self.waypoint_names(d))
    }

    fn is_waypoint(&self, entity: &PlayerRef) -> bool {
        entity.living
    }

    /// Players' icons; mobs keep none in Kiln (they transmit no waypoint without the
    /// `waypoint_transmit_range` attribute anyway).
    fn modify_waypoint(&mut self, entity: &PlayerRef, change: &kiln_command::host::WaypointChange) -> bool {
        use kiln_command::host::WaypointChange;
        if !entity.is_player() {
            return false;
        }
        self.set_waypoint_icon(entity.conn, |icon| match change {
            WaypointChange::Color(c) => icon.color = *c,
            WaypointChange::Style(s) => icon.style = s.clone().unwrap_or_else(|| crate::waypoints::DEFAULT_STYLE.to_owned()),
        })
    }

    fn clear_post_effects(&mut self, player: &PlayerRef) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        if p.post_effects.is_empty() {
            return false;
        }
        p.post_effects.clear();
        p.post_effects_dirty = true;
        true
    }

    // ---- data, tag, item, loot, clear, enchant, attribute, damage, ride, rotate, spectate,
    // swing and fetchprofile (see `command_data`; `entity_data` is above) ----------------------

    fn set_entity_data(&mut self, entity: &PlayerRef, data: &Tag) -> Result<(), CommandError> {
        self.load_entity_data(entity, data)
    }

    fn set_block_entity_data(&mut self, dimension: &str, pos: [i32; 3], data: &Tag) -> Result<(), CommandError> {
        self.set_block_entity_nbt(dimension, pos, data);
        Ok(())
    }

    fn storage_ids(&self) -> Vec<String> {
        self.commands.storage.keys().map(str::to_owned).collect()
    }

    fn entity_tags(&mut self, entity: &PlayerRef) -> Vec<String> {
        match entity.entity {
            None => self.players.get(&entity.conn).map_or_else(Vec::new, Player::tags),
            Some(_) => entity.tags.clone(),
        }
    }

    fn add_entity_tag(&mut self, entity: &PlayerRef, tag: &str) -> bool {
        self.change_entity_tag(entity, tag, true)
    }

    fn remove_entity_tag(&mut self, entity: &PlayerRef, tag: &str) -> bool {
        self.change_entity_tag(entity, tag, false)
    }

    fn rotate_entity(&mut self, entity: &PlayerRef, rotation: [f32; 2]) {
        self.rotate_target(entity, rotation);
    }

    fn swing_arm(&mut self, entity: &PlayerRef, offhand: bool, animation: &str, duration: i32) -> bool {
        self.swing_target(entity, offhand, animation, duration)
    }

    fn vehicle_of(&mut self, entity: &PlayerRef) -> Option<PlayerRef> {
        self.vehicle_of_target(entity)
    }

    fn self_and_passengers(&mut self, entity: &PlayerRef) -> Vec<PlayerRef> {
        self.self_and_passengers_of(entity)
    }

    fn start_riding(&mut self, entity: &PlayerRef, vehicle: &PlayerRef) -> bool {
        self.start_riding_target(entity, vehicle)
    }

    fn stop_riding(&mut self, entity: &PlayerRef) {
        self.stop_riding_target(entity);
    }

    fn damage_entity(
        &mut self,
        entity: &PlayerRef,
        amount: f32,
        damage_type: &str,
        _at: Option<[f64; 3]>,
        _by: Option<&PlayerRef>,
        _from: Option<&PlayerRef>,
    ) -> Result<bool, CommandError> {
        self.damage_target(entity, amount, damage_type)
    }

    fn can_spectate(&self, entity: &PlayerRef) -> bool {
        kiln_data::entities::by_name(entity.kind).is_none_or(|t| t.tracking_range != 0)
    }

    fn set_camera(&mut self, player: &PlayerRef, target: Option<&PlayerRef>) {
        self.set_camera_of(player, target);
    }

    fn slot_item(&mut self, holder: &kiln_command::host::ItemHolder<PlayerRef>, slot: i32) -> Option<Option<Tag>> {
        self.slot_item_nbt(holder, slot)
    }

    fn set_slot_item(&mut self, holder: &kiln_command::host::ItemHolder<PlayerRef>, slot: i32, item: Option<&Tag>) -> bool {
        self.set_slot_item_nbt(holder, slot, item)
    }

    fn is_container(&mut self, dimension: &str, pos: [i32; 3]) -> bool {
        self.is_container_at(dimension, pos)
    }

    /// Main inventory, armor (feet first), off hand, body and saddle; the crafting grid and
    /// cursor are not modeled here.
    fn clear_slots(&self, _player: &PlayerRef) -> Vec<i32> {
        (0..36).chain([100, 101, 102, 103, 99, 105, 106]).collect()
    }

    fn inventory_changed(&mut self, player: &PlayerRef) {
        self.broadcast_inventory(player);
    }

    fn attribute(&mut self, entity: &PlayerRef, attribute: &str) -> Result<Option<kiln_command::host::AttributeState>, ()> {
        self.attribute_state(entity, attribute)
    }

    fn set_attribute_base(&mut self, entity: &PlayerRef, attribute: &str, value: f64) {
        self.change_attribute(entity, attribute, crate::command_data::AttributeChange::Base(Some(value)));
    }

    fn reset_attribute_base(&mut self, entity: &PlayerRef, attribute: &str) {
        self.change_attribute(entity, attribute, crate::command_data::AttributeChange::Base(None));
    }

    fn add_attribute_modifier(&mut self, entity: &PlayerRef, attribute: &str, id: &str, amount: f64, operation: u8) {
        let change = crate::command_data::AttributeChange::AddModifier(id.to_owned(), amount, operation);
        self.change_attribute(entity, attribute, change);
    }

    fn remove_attribute_modifier(&mut self, entity: &PlayerRef, attribute: &str, id: &str) -> bool {
        self.change_attribute(entity, attribute, crate::command_data::AttributeChange::RemoveModifier(id.to_owned()))
    }

    fn roll_loot(&mut self, source: &kiln_command::host::LootSource<PlayerRef>) -> Result<(Vec<Tag>, Option<String>), CommandError> {
        self.roll_command_loot(source)
    }

    fn hand_item(&mut self, entity: &PlayerRef, offhand: bool) -> Option<Option<Tag>> {
        self.hand_item_nbt(entity, offhand)
    }

    fn give_stack(&mut self, player: &PlayerRef, item: &Tag) -> bool {
        self.give_stack_nbt(player, item)
    }

    fn compute_provider(
        &mut self,
        provider: &kiln_command::host::LootTableArg,
        float: bool,
        target: &kiln_command::host::ComputeTarget<PlayerRef>,
    ) -> Result<f64, kiln_command::host::ComputeError> {
        self.compute_provider_value(provider, float, target)
    }

    fn slot_source_tree(
        &mut self,
        source: &kiln_command::host::LootTableArg,
        container: &kiln_command::host::ItemHolder<PlayerRef>,
    ) -> Result<kiln_command::host::SlotTree<PlayerRef>, CommandError> {
        self.slot_tree_nbt(source, container)
    }

    fn apply_item_modifier(&mut self, modifier: &kiln_command::host::LootTableArg, item: &Tag) -> Result<Tag, CommandError> {
        self.apply_modifier_nbt(modifier, item)
    }

    fn spawn_item(&mut self, dimension: &str, pos: [f64; 3], item: &Tag) {
        self.spawn_item_nbt(dimension, pos, item);
    }

    fn container_size(&mut self, dimension: &str, pos: [i32; 3]) -> Option<i32> {
        self.container_size_at(dimension, pos)
    }

    fn item_max_stack(&self, item: &Tag) -> i32 {
        kiln_item::ItemStack::from_nbt(item).map_or(64, |s| s.max_stack_size())
    }

    fn enchantment_max_level(&self, enchantment: &str) -> Option<i32> {
        self.enchantment_max(enchantment)
    }

    fn enchant_held(&mut self, entity: &PlayerRef, enchantment: &str, level: i32) -> kiln_command::host::EnchantOutcome {
        self.enchant_target(entity, enchantment, level)
    }
    // ---- worldborder, tick, forceload, random, locate, place, fillbiome ----

    fn world_border(&mut self, dimension: &str) -> kiln_command::host::BorderInfo {
        crate::dim_id(dimension).map(|d| self.world.borders[d].info()).unwrap_or_default()
    }

    fn change_world_border(&mut self, dimension: &str, change: kiln_command::host::BorderChange) {
        if let Some(d) = crate::dim_id(dimension) {
            self.change_border(d, change);
        }
    }

    fn tick_rate(&self) -> kiln_command::host::TickRateInfo {
        let mut info = self.world.tick_rate.info();
        info.tick_times = self.world.tick_times.clone();
        let n = self.world.tick_index.clamp(1, 100);
        info.average_tick_nanos = self.world.tick_times.iter().sum::<i64>() / n as i64;
        info
    }

    fn change_tick_rate(&mut self, action: kiln_command::host::TickRateAction) -> bool {
        let mut news = crate::world_state::TickNews::default();
        let result = self.world.tick_rate.apply(action, &mut news);
        self.tick_rate_news(news);
        result
    }

    fn forced_chunks(&self, dimension: &str) -> Vec<[i32; 2]> {
        crate::dim_id(dimension).map(|d| self.world.forced[d].iter().copied().collect()).unwrap_or_default()
    }

    fn set_chunk_forced(&mut self, dimension: &str, chunk: [i32; 2], forced: bool) -> bool {
        crate::dim_id(dimension).is_some_and(|d| self.set_forced(d, chunk, forced))
    }

    fn random_between(&mut self, sequence: Option<&Identifier>, min: i32, max: i32) -> i32 {
        use kiln_javamath::random::RandomSource;
        let bound = max.wrapping_sub(min).wrapping_add(1);
        match sequence {
            Some(id) => {
                let seed = self.commands.seed;
                self.world.sequences.get(id.as_str(), seed).next_int_bounded(bound) + min
            }
            None => self.level_random_between(min, max),
        }
    }

    fn reset_random_sequence(&mut self, id: &Identifier, params: Option<(i32, bool, bool)>) {
        let seed = self.commands.seed;
        self.world.sequences.reset(id.as_str(), seed, params);
    }

    fn clear_random_sequences(&mut self, defaults: Option<(i32, bool, bool)>) -> i32 {
        self.world.sequences.clear(defaults)
    }

    fn random_sequence_ids(&self) -> Vec<String> {
        self.world.sequences.ids()
    }

    fn broadcast_system_message(&mut self, text: Text) {
        info!("{}", console_text(&text));
        self.broadcast(packets::system_chat(text.to_nbt(), false));
    }

    fn send_failure(&mut self, text: Text) {
        if self.commands.stack.silent {
            return;
        }
        self.reply(text.color("red"));
    }

    fn locate_biome(&mut self, dimension: &str, origin: [i32; 3], matches: &dyn Fn(&str) -> bool) -> Option<kiln_command::host::Located> {
        let d = crate::dim_id(dimension)?;
        let (pos, id) = Sim::locate_biome(self, d, origin, matches)?;
        Some(kiln_command::host::Located { pos, id })
    }

    fn locate_poi(&mut self, dimension: &str, origin: [i32; 3], matches: &dyn Fn(&str) -> bool) -> Option<kiln_command::host::Located> {
        let d = crate::dim_id(dimension)?;
        let (pos, id) = Sim::locate_poi(self, d, origin, matches)?;
        Some(kiln_command::host::Located { pos, id })
    }

    fn structure_ids(&self) -> Vec<String> {
        crate::world_state::worldgen_ids("worldgen/structure").clone()
    }

    fn structure_tag(&self, tag: &str) -> Option<Vec<String>> {
        crate::world_state::worldgen_tag("worldgen/structure", tag)
    }

    fn noise_biome(&mut self, dimension: &str, quart: [i32; 3]) -> Option<String> {
        self.biome(dimension, quart.map(|q| q << 2))
    }

    fn fill_biome(&mut self, dimension: &str, min: [i32; 3], max: [i32; 3], biome: &str, filter: &dyn Fn(&str) -> bool) -> Option<i32> {
        let d = crate::dim_id(dimension)?;
        Sim::fill_biome(self, d, min, max, biome, filter)
    }

    fn place(&mut self, dimension: &str, what: &kiln_command::host::Placement, pos: [i32; 3]) -> Result<(), CommandError> {
        use kiln_command::host::Placement;
        let dim = crate::dim_id(dimension).unwrap_or(crate::OVERWORLD_ID);
        match what {
            Placement::Template { id, rotation, mirror, integrity, seed, strict } => {
                self.place_template(dim, id.as_str(), pos, *rotation, *mirror, *integrity, *seed, *strict)
            }
            Placement::Feature { .. } => Err(CommandError::unsupported("place feature")),
            Placement::Jigsaw { .. } => Err(CommandError::unsupported("place jigsaw")),
            Placement::Structure(_) => Err(CommandError::unsupported("place structure")),
        }
    }
}

impl Sim {
    /// `ServerPlayer.sendPostEffects` for players whose post effects changed (and after
    /// joining).
    pub(crate) fn send_post_effects(&mut self) {
        for p in self.players.values_mut() {
            if std::mem::take(&mut p.post_effects_dirty) {
                let ids: Vec<&str> = p.post_effects.iter().map(String::as_str).collect();
                let pkt = packets::post_effects(&ids);
                p.send(pkt);
            }
        }
    }

    /// `Stopwatches` from `data/minecraft/stopwatches.dat` (elapsed milliseconds by id).
    pub(crate) fn load_stopwatches(&mut self) {
        let Some(storage) = &self.storage else { return };
        let Some(data) = kiln_storage::saved_data::read(&storage.dir, "stopwatches") else { return };
        let now = std::time::Instant::now();
        if let Some(Tag::Compound(entries)) = data.get("stopwatches") {
            for (id, t) in entries {
                if let Some(ms) = t.as_i64() {
                    self.commands.stopwatches.push((id.clone(), now, ms.max(0) as u64));
                }
            }
        }
    }

    /// Saves the stopwatches (their elapsed time) when they changed, and with every autosave
    /// while any run.
    pub(crate) fn save_stopwatches(&mut self) {
        let Some(storage) = &self.storage else { return };
        if !self.commands.stopwatches_dirty && self.commands.stopwatches.is_empty() {
            return;
        }
        self.commands.stopwatches_dirty = false;
        let entries = self
            .commands
            .stopwatches
            .iter()
            .map(|(id, start, before)| (id.clone(), Tag::Long((*before + start.elapsed().as_millis() as u64) as i64)))
            .collect();
        let data = Tag::Compound(vec![("stopwatches".to_owned(), Tag::Compound(entries))]);
        if let Err(e) = kiln_storage::saved_data::write(&storage.dir.clone(), "stopwatches", data) {
            tracing::warn!("failed to save stopwatches: {e}");
        }
    }

    /// Copies the operator names to the login checks (operators bypass the whitelist).
    pub(crate) fn sync_ops(&mut self) {
        let ops = self.commands.ops.clone();
        self.config.access.write().unwrap_or_else(std::sync::PoisonError::into_inner).ops = ops;
    }

    /// `ServerPlayer.resetLastActionTime` on what the player does, and the idle kick
    /// (`ServerGamePacketListenerImpl.tick` with a `player-idle-timeout`).
    pub(crate) fn track_idle(&mut self, packets: &[(ConnId, kiln_link::PlayIn)]) {
        use kiln_link::PlayIn;
        let now = std::time::Instant::now();
        for (conn, pkt) in packets {
            let Some(p) = self.players.get_mut(conn) else { continue };
            let active = match pkt {
                PlayIn::KeepAlive { .. }
                | PlayIn::ChunkBatchReceived { .. }
                | PlayIn::ClientTickEnd
                | PlayIn::ClientInformation(_)
                | PlayIn::AcceptTeleport { .. }
                | PlayIn::PlayerLoaded => false,
                PlayIn::Move { pos, rot, .. } => pos.is_some_and(|v| v != p.pos) || rot.is_some_and(|r| r != p.rot),
                _ => true,
            };
            if active {
                p.last_action = now;
            }
        }
        let minutes = self.commands.idle_timeout;
        if minutes <= 0 {
            return;
        }
        let limit = std::time::Duration::from_secs(minutes as u64 * 60);
        let idle: Vec<ConnId> =
            self.players.iter().filter(|(_, p)| !p.disconnected && now.duration_since(p.last_action) > limit).map(|(c, _)| *c).collect();
        for conn in idle {
            if let Some(p) = self.players.get_mut(&conn) {
                p.flush();
                p.sink.disconnect(packets::play_disconnect_text(kiln_command::tr!("multiplayer.disconnect.idling").to_nbt()));
            }
        }
    }
}

/// `UUIDUtil.createOfflinePlayerUUID`: a version 3 UUID of `OfflinePlayer:<name>`.
fn offline_uuid(name: &str) -> Uuid {
    use md5::Digest;
    let mut h: [u8; 16] = md5::Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

fn block_pos(p: [i32; 3]) -> kiln_blocks::BlockPos {
    kiln_blocks::BlockPos::new(p[0], p[1], p[2])
}
