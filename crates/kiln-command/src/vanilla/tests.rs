//! The built-in commands against a mock host, and their tree against vanilla's `commands.json`.

use super::*;
use crate::arguments::ArgumentType;
use crate::blocks::UpdateFlags;
use crate::dispatcher::NodeKind;
use crate::host::{ChatMessage, GameRuleValue, Source, SourceStack, SpawnPoint, Teleport, TimeAction, Weather};
use crate::scoreboard::Scoreboard;
use crate::selector::{Aabb, SelectorWorld};
use crate::text::Text;
use crate::types::{Difficulty, GameMode, Heightmap, Identifier, ItemInput};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::commands::{Parser, StringKind};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(super) struct Ent {
    id: u64,
    name: String,
    kind: &'static str,
    pos: [f64; 3],
    rot: [f32; 2],
    dim: &'static str,
    mode: Option<GameMode>,
}

impl SelectorTarget for Ent {
    fn uuid(&self) -> Uuid {
        Uuid::from_u64_pair(0, self.id)
    }
    fn name(&self) -> String {
        self.name.clone()
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
        Aabb { min: [x - 0.3, y, z - 0.3], max: [x + 0.3, y + 1.8, z + 0.3] }
    }
    fn eye_height(&self) -> f64 {
        1.62
    }
    fn game_mode(&self) -> Option<GameMode> {
        self.mode
    }
}

pub(super) struct Mock {
    level: u8,
    ents: Vec<Ent>,
    stack: SourceStack<Mock>,
    pub(super) effects: Vec<String>,
    pub(super) feedback: Vec<(String, bool)>,
    pub(super) chat: Vec<String>,
    ops: Vec<String>,
    difficulty: Difficulty,
    pub(super) rules: HashMap<String, GameRuleValue>,
    /// Blocks changed from the generated terrain (stone below y 64, air above).
    pub(super) blocks: HashMap<[i32; 3], u16>,
    pub(super) scoreboard: Scoreboard,
    pub(super) storage: crate::CommandStorage,
    pub(super) bossbars: crate::BossBars,
    pub(super) functions: crate::functions::FunctionLibrary,
    pub(super) timers: crate::functions::TimerQueue,
    /// Packets sent to single players: (name, packet id).
    pub(super) packets: Vec<(String, i32)>,
}

impl Mock {
    pub(super) fn new(level: u8) -> Self {
        let player = |id, name: &str, pos, dim| Ent {
            id,
            name: name.into(),
            kind: "minecraft:player",
            pos,
            rot: [0.0, 0.0],
            dim,
            mode: Some(GameMode::Survival),
        };
        let alice = player(1, "Alice", [0.5, 64.0, 0.5], "minecraft:overworld");
        Mock {
            level,
            stack: SourceStack::of_entity(alice.clone()),
            ents: vec![
                alice,
                player(2, "Bob", [10.5, 64.0, 0.5], "minecraft:overworld"),
                player(3, "Carol", [0.0, 70.0, 0.0], "minecraft:the_nether"),
                Ent {
                    id: 4,
                    name: "Zombie".into(),
                    kind: "minecraft:zombie",
                    pos: [3.0, 64.0, 0.0],
                    rot: [0.0, 0.0],
                    dim: "minecraft:overworld",
                    mode: None,
                },
            ],
            effects: Vec::new(),
            feedback: Vec::new(),
            chat: Vec::new(),
            ops: vec!["Alice".into()],
            difficulty: Difficulty::Normal,
            rules: HashMap::new(),
            blocks: HashMap::new(),
            scoreboard: Scoreboard::default(),
            storage: crate::CommandStorage::default(),
            bossbars: crate::BossBars::default(),
            functions: crate::functions::FunctionLibrary::default(),
            timers: crate::functions::TimerQueue::default(),
            packets: Vec::new(),
        }
    }

    pub(super) fn console(level: u8) -> Self {
        Mock { stack: SourceStack::new(Text::literal("Server"), "minecraft:overworld", [0.0; 3]), ..Mock::new(level) }
    }

    pub(super) fn run(&mut self, d: &Dispatcher<Mock>, cmd: &str) -> Result<i32, CommandError> {
        d.execute(cmd, self)
    }

    pub(super) fn feedback_keys(&self) -> Vec<String> {
        self.feedback.iter().map(|f| f.0.clone()).collect()
    }

    pub(super) fn block(&self, pos: [i32; 3]) -> u16 {
        self.blocks.get(&pos).copied().unwrap_or(if pos[1] < 64 { STONE } else { AIR })
    }
}

impl Source for Mock {
    type Entity = Ent;
    fn permission_level(&self) -> u8 {
        self.level
    }
    fn stack(&self) -> &SourceStack<Mock> {
        &self.stack
    }
    fn stack_mut(&mut self) -> &mut SourceStack<Mock> {
        &mut self.stack
    }
    fn player_names(&self) -> Vec<String> {
        self.players().iter().map(|p| p.name.clone()).collect()
    }
    fn dimensions(&self) -> Vec<String> {
        vec!["minecraft:overworld".into(), "minecraft:the_nether".into()]
    }
    fn fork_limit(&self) -> usize {
        match self.rules.get("minecraft:max_command_forks") {
            Some(GameRuleValue::Int(v)) => *v as usize,
            _ => 65536,
        }
    }
    fn command_limit(&self) -> i32 {
        match self.rules.get("minecraft:max_command_sequence_length") {
            Some(GameRuleValue::Int(v)) => *v,
            _ => 65536,
        }
    }
}

impl SelectorWorld for Mock {
    fn players(&self) -> Vec<Ent> {
        self.ents.iter().filter(|e| e.is_player()).cloned().collect()
    }
    fn entities(&self, dimension: Option<&str>, _: Option<&Aabb>) -> Vec<Ent> {
        self.ents.iter().filter(|e| dimension.is_none_or(|d| d == e.dim)).cloned().collect()
    }
    fn shuffle(&mut self, _: &mut [Ent]) {}
    fn scoreboard(&self) -> Option<&Scoreboard> {
        Some(&self.scoreboard)
    }
}

impl Host for Mock {
    fn send_success(&mut self, text: Text, broadcast: bool) {
        if self.stack.silent {
            return;
        }
        self.feedback.push((text.to_plain(), broadcast));
    }
    fn send_system(&mut self, player: &Ent, text: Text) {
        self.chat.push(format!("to {}: {}", player.name, text.to_plain()));
    }
    fn broadcast_chat(&mut self, m: ChatMessage) {
        self.chat.push(format!("all: {}", m.to_text().to_plain()));
    }
    fn send_chat(&mut self, player: &Ent, m: ChatMessage) {
        self.chat.push(format!("to {}: {}", player.name, m.to_text().to_plain()));
    }
    fn send_chat_to_source(&mut self, m: ChatMessage) {
        self.chat.push(format!("to source: {}", m.to_text().to_plain()));
    }
    fn teleport(&mut self, e: &Ent, to: &Teleport) -> Result<(), CommandError> {
        self.effects.push(format!(
            "tp {} {:?} rel={:?} rot={:?} facing={:?}",
            e.name, to.pos, to.relative, to.rotation, to.facing
        ));
        Ok(())
    }
    fn set_game_mode(&mut self, p: &Ent, mode: GameMode) -> bool {
        let e = self.ents.iter_mut().find(|e| e.id == p.id).unwrap();
        let changed = e.mode != Some(mode);
        e.mode = Some(mode);
        changed
    }
    fn kill(&mut self, e: &Ent) {
        self.effects.push(format!("kill {}", e.name));
    }
    fn give(&mut self, p: &Ent, item: &ItemInput, count: i32) {
        self.effects.push(format!("give {} {} {count} {:?}", p.name, item.item, item.components));
    }
    fn kick(&mut self, p: &Ent, reason: Text) {
        self.effects.push(format!("kick {} {}", p.name, reason.to_plain()));
    }
    fn max_players(&self) -> usize {
        20
    }
    fn find_profile(&mut self, name: &str) -> Option<Profile> {
        match name {
            "Notch" => Some(Profile { uuid: Uuid::from_u64_pair(9, 9), name: "Notch".into() }),
            _ => self.players().iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(profile_of),
        }
    }
    fn is_operator(&self, p: &Profile) -> bool {
        self.ops.contains(&p.name)
    }
    fn operator_names(&self) -> Vec<String> {
        self.ops.clone()
    }
    fn set_operator(&mut self, p: &Profile, op: bool) {
        if op {
            self.ops.push(p.name.clone());
        } else {
            self.ops.retain(|n| *n != p.name);
        }
    }
    fn difficulty(&self) -> Difficulty {
        self.difficulty
    }
    fn set_difficulty(&mut self, d: Difficulty) {
        self.difficulty = d;
    }
    fn set_weather(&mut self, w: Weather, duration: Option<i32>) -> i32 {
        self.effects.push(format!("weather {} {duration:?}", w.name()));
        duration.unwrap_or(12000)
    }
    fn time(&mut self, clock: Option<&Identifier>, action: &TimeAction) -> Result<i32, CommandError> {
        self.effects.push(format!("time {:?} {action:?}", clock.map(Identifier::as_str)));
        Ok(7)
    }
    fn time_markers(&self, clock: Option<&Identifier>) -> Vec<String> {
        match clock.map(Identifier::as_str) {
            None | Some("minecraft:overworld") => vec!["minecraft:day".into(), "minecraft:night".into()],
            Some(_) => vec![],
        }
    }
    fn timelines(&self, _: Option<&Identifier>) -> Vec<String> {
        vec!["minecraft:day".into(), "minecraft:moon".into()]
    }
    fn game_rule(&self, rule: &str) -> GameRuleValue {
        self.rules.get(rule).copied().unwrap_or(match kiln_data::game_rule_default(rule) {
            Some(kiln_data::GameRuleDefault::Int(v)) => GameRuleValue::Int(v),
            Some(kiln_data::GameRuleDefault::Bool(b)) => GameRuleValue::Bool(b),
            None => match gamerules::value_type(rule) {
                ArgumentType::Bool => GameRuleValue::Bool(false),
                _ => GameRuleValue::Int(3),
            },
        })
    }
    fn set_game_rule(&mut self, rule: &str, value: GameRuleValue) {
        self.rules.insert(rule.into(), value);
    }
    fn seed(&self) -> i64 {
        -1234567890123
    }
    fn stop(&mut self) {
        self.effects.push("stop".into());
    }
    fn set_spawn_point(&mut self, p: &Ent, s: &SpawnPoint) {
        self.effects.push(format!("spawnpoint {} {:?} {} {} {}", p.name, s.pos, s.yaw, s.pitch, s.dimension));
    }
    fn set_world_spawn(&mut self, s: &SpawnPoint) -> Result<(), CommandError> {
        self.effects.push(format!("worldspawn {:?} {} {} {}", s.pos, s.yaw, s.pitch, s.dimension));
        Ok(())
    }
    fn kiln_tick(&mut self) -> Vec<Text> {
        vec![Text::literal("20.0 TPS"), Text::literal("1.5 mspt")]
    }
    fn kiln_regions(&mut self) -> Vec<Text> {
        vec![Text::literal("1 region")]
    }
    /// Chunks within 10 of the origin are loaded, in the overworld and the nether.
    fn is_chunk_loaded(&self, dimension: &str, cx: i32, cz: i32) -> bool {
        self.has_dimension(dimension) && cx.abs() <= 10 && cz.abs() <= 10
    }
    fn block_state(&mut self, _: &str, pos: [i32; 3]) -> u16 {
        if !(-64..320).contains(&pos[1]) {
            return kiln_data::blocks::default_state::VOID_AIR;
        }
        self.block(pos)
    }
    fn set_block(&mut self, _: &str, pos: [i32; 3], state: u16, nbt: Option<&Tag>, flags: UpdateFlags) -> bool {
        if self.block(pos) == state {
            return false;
        }
        self.blocks.insert(pos, state);
        if let Some(nbt) = nbt {
            self.effects.push(format!("nbt {pos:?} {}", crate::snbt::to_snbt(nbt)));
        }
        let _ = flags;
        true
    }
    fn destroy_block(&mut self, dimension: &str, pos: [i32; 3], drop: bool) -> bool {
        let old = self.block(pos);
        if kiln_data::blocks_types::is_air(old) {
            return false;
        }
        self.effects.push(format!("destroy {pos:?} {drop}"));
        self.set_block(dimension, pos, AIR, None, UpdateFlags::ALL)
    }
    fn height(&mut self, _: &str, heightmap: Heightmap, x: i32, z: i32) -> i32 {
        (-64..320).rev().find(|&y| heightmap.counts(self.block([x, y, z]))).map_or(-64, |y| y + 1)
    }
    fn biome(&mut self, _: &str, _: [i32; 3]) -> Option<String> {
        Some("minecraft:plains".into())
    }
    fn scoreboard_mut(&mut self) -> Option<&mut Scoreboard> {
        Some(&mut self.scoreboard)
    }

    fn storage_mut(&mut self) -> Option<&mut crate::CommandStorage> {
        Some(&mut self.storage)
    }

    fn functions(&self) -> Option<&crate::functions::FunctionLibrary> {
        Some(&self.functions)
    }

    fn timers(&self) -> Option<&crate::functions::TimerQueue> {
        Some(&self.timers)
    }

    fn timers_mut(&mut self) -> Option<&mut crate::functions::TimerQueue> {
        Some(&mut self.timers)
    }

    fn game_time(&self) -> i64 {
        100
    }

    fn bossbars(&self) -> Option<&crate::BossBars> {
        Some(&self.bossbars)
    }

    fn bossbars_mut(&mut self) -> Option<&mut crate::BossBars> {
        Some(&mut self.bossbars)
    }

    fn send_packet(&mut self, player: &Ent, packet: bytes::Bytes) {
        let id = kiln_proto::Reader::new(&packet).varint().unwrap();
        self.packets.push((player.name.clone(), id));
    }
}

const STONE: u16 = kiln_data::blocks::default_state::STONE;
const AIR: u16 = kiln_data::blocks::default_state::AIR;

/// The simulation keeps the dispatcher on its own thread and may share it.
#[test]
fn dispatcher_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Dispatcher<Mock>>();
}

pub(super) fn dispatcher() -> Dispatcher<Mock> {
    let mut d = Dispatcher::new();
    register_all(&mut d);
    d
}

pub(super) fn err_key(r: Result<i32, CommandError>) -> String {
    r.unwrap_err().key().unwrap().to_owned()
}

#[test]
fn gamemode() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    assert_eq!(s.run(&d, "gamemode creative"), Ok(1));
    assert_eq!(s.feedback_keys(), ["commands.gamemode.success.self[gameMode.creative]"]);
    assert_eq!(s.run(&d, "gamemode creative"), Ok(0), "unchanged");
    assert_eq!(s.run(&d, "gamemode adventure @a[distance=..20]"), Ok(2));
    assert_eq!(s.chat, ["to Bob: gameMode.changed[gameMode.adventure]"]);
    assert!(s.feedback.iter().all(|f| f.1), "gamemode feedback is broadcast to ops");
    assert_eq!(err_key(s.run(&d, "gamemode creatve")), "argument.gamemode.invalid");
    assert_eq!(err_key(s.run(&d, "gamemode creative @e")), "argument.player.entities");
    assert_eq!(err_key(Mock::console(4).run(&d, "gamemode creative")), "permissions.requires.player");
    let e = Mock::new(0).run(&d, "gamemode creative").unwrap_err();
    assert_eq!((e.key(), e.cursor()), (Some("command.unknown.command"), Some(0)), "hidden below level 2");
}

#[test]
fn teleport() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    assert_eq!(s.run(&d, "tp 1 64 -2"), Ok(1));
    assert_eq!(s.effects.pop().unwrap(), "tp Alice [1.5, 64.0, -1.5] rel=[false, false, false] rot=None facing=None");
    assert_eq!(
        s.feedback.pop().unwrap().0,
        "commands.teleport.success.location.single[Alice, 1.500000, 64.000000, -1.500000]"
    );
    s.run(&d, "teleport @a[distance=..20] ~ ~10 ~").unwrap();
    assert_eq!(
        s.effects,
        [
            "tp Alice [0.5, 74.0, 0.5] rel=[true, true, true] rot=None facing=None",
            "tp Bob [0.5, 74.0, 0.5] rel=[true, true, true] rot=None facing=None",
        ]
    );
    assert_eq!(
        s.feedback.pop().unwrap().0,
        "commands.teleport.success.location.multiple[2, 0.500000, 74.000000, 0.500000]"
    );
    s.effects.clear();
    // Local coordinates: Alice faces south (+z) with yaw 0.
    s.run(&d, "tp @s ^ ^ ^2").unwrap();
    assert!(s.effects[0].starts_with("tp Alice [0.5"), "{}", s.effects[0]);
    s.effects.clear();
    s.run(&d, "tp Bob Alice").unwrap();
    assert_eq!(
        s.effects.pop().unwrap(),
        "tp Bob [0.5, 64.0, 0.5] rel=[false, false, false] rot=Some([0.0, 0.0]) facing=None"
    );
    assert_eq!(s.feedback.pop().unwrap().0, "commands.teleport.success.entity.single[Bob, Alice]");
    s.run(&d, "tp Carol").unwrap();
    assert_eq!(
        s.effects.pop().unwrap(),
        "tp Alice [0.0, 70.0, 0.0] rel=[false, false, false] rot=Some([0.0, 0.0]) facing=None"
    );
    s.run(&d, "tp @e[type=zombie] 0 64 0 ~90 -10").unwrap();
    assert_eq!(
        s.effects.pop().unwrap(),
        "tp Zombie [0.5, 64.0, 0.5] rel=[false, false, false] rot=Some([90.0, -10.0]) facing=None"
    );
    s.run(&d, "tp Bob 0 64 0 facing entity Alice eyes").unwrap();
    assert_eq!(
        s.effects.pop().unwrap(),
        "tp Bob [0.5, 64.0, 0.5] rel=[false, false, false] rot=None facing=Some([0.5, 65.62, 0.5])"
    );
    s.run(&d, "tp Bob 0 64 0 facing 5 64 5").unwrap();
    assert_eq!(
        s.effects.pop().unwrap(),
        "tp Bob [0.5, 64.0, 0.5] rel=[false, false, false] rot=None facing=Some([5.5, 64.0, 5.5])"
    );
    assert_eq!(err_key(s.run(&d, "tp @e[type=cow] 0 0 0")), "argument.entity.notfound.entity");
    // Brigadier prefers the <destination> branch that parsed without errors, then finds
    // trailing input: vanilla reports an incorrect argument here, not the selector problem.
    assert_eq!(err_key(s.run(&d, "tp Bob @e")), "command.unknown.argument");
    assert_eq!(err_key(s.run(&d, "tp @e")), "command.unknown.command");
    assert_eq!(err_key(s.run(&d, "tp 0 40000000 0")), "commands.teleport.invalidPosition");
    // `1 2` is not a position, so it means "entity 1 to entity 2".
    assert_eq!(err_key(s.run(&d, "tp 1 2")), "argument.entity.notfound.entity");
    assert_eq!(err_key(s.run(&d, "tp @s 1 2")), "command.unknown.argument");
    assert_eq!(err_key(s.run(&d, "setworldspawn 1 2")), "argument.pos3d.incomplete");
    assert_eq!(err_key(s.run(&d, "tp")), "command.unknown.command");
}

#[test]
fn chat_commands() {
    let d = dispatcher();
    let s = &mut Mock::new(0);
    assert_eq!(s.run(&d, "msg Bob hi @a"), Ok(1));
    assert_eq!(
        s.chat,
        [
            "to source: commands.message.display.outgoing[Bob, hi @a]",
            "to Bob: commands.message.display.incoming[Alice, hi @a]"
        ]
    );
    assert_eq!(err_key(s.run(&d, "tell @a hi")), "argument.entity.selector.not_allowed");
    s.chat.clear();
    s.run(&d, "w bob psst").unwrap();
    assert_eq!(s.chat[1], "to Bob: commands.message.display.incoming[Alice, psst]");
    s.run(&d, "me waves").unwrap();
    assert_eq!(s.chat[2], "all: chat.type.emote[Alice, waves]");
    assert_eq!(err_key(s.run(&d, "say hi")), "command.unknown.command");
    let s = &mut Mock::new(2);
    s.run(&d, "say hello @a[distance=..20]!").unwrap();
    assert_eq!(s.chat, ["all: chat.type.announcement[Alice, hello Alice, Bob!]"]);
    assert_eq!(err_key(s.run(&d, "msg Nobody hi")), "argument.entity.notfound.player");
}

#[test]
fn list_kill_give() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    assert_eq!(s.run(&d, "list"), Ok(3));
    assert_eq!(s.feedback.pop().unwrap(), ("commands.list.players[3, 20, Alice, Bob, Carol]".into(), false));
    s.run(&d, "list uuids").unwrap();
    assert!(s.feedback.pop().unwrap().0.contains("commands.list.nameAndId[Bob, 00000000-0000-0000-0000-000000000002]"));
    assert_eq!(s.run(&d, "kill @e[type=minecraft:zombie]"), Ok(1));
    assert_eq!(s.feedback.pop().unwrap().0, "commands.kill.success.single[Zombie]");
    assert_eq!(s.run(&d, "kill"), Ok(1));
    assert_eq!(s.effects, ["kill Zombie", "kill Alice"]);
    assert_eq!(s.run(&d, "kill @e"), Ok(4));
    assert_eq!(s.feedback.pop().unwrap().0, "commands.kill.success.multiple[4]");
    s.effects.clear();
    assert_eq!(s.run(&d, "give Bob diamond_sword[damage=3] 2"), Ok(1));
    assert_eq!(
        s.effects.pop().unwrap(),
        "give Bob minecraft:diamond_sword 2 [(Identifier(\"minecraft:damage\"), Some(\"3\"))]"
    );
    assert_eq!(s.feedback.pop().unwrap().0, "commands.give.success.single[2, [item.minecraft.diamond_sword], Bob]");
    assert_eq!(s.run(&d, "give @a stone"), Ok(3));
    assert_eq!(s.feedback.pop().unwrap().0, "commands.give.success.multiple[1, [block.minecraft.stone], 3]");
    assert_eq!(err_key(s.run(&d, "give Bob stone 6401")), "commands.give.failed.toomanyitems");
    assert_eq!(err_key(s.run(&d, "give Bob stone 0")), "argument.integer.low");
    assert_eq!(err_key(s.run(&d, "give Bob nothing_at_all")), "argument.item.id.invalid");
}

#[test]
fn time_weather_gamerule() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    assert_eq!(s.run(&d, "time set 1d"), Ok(7));
    s.run(&d, "time of minecraft:overworld query time").unwrap();
    s.run(&d, "time query minecraft:moon repetition").unwrap();
    s.run(&d, "time rate 2.5").unwrap();
    s.run(&d, "time add -100").unwrap();
    s.run(&d, "time set minecraft:day").unwrap();
    s.run(&d, "time of the_end pause").unwrap();
    s.run(&d, "time query gametime").unwrap();
    assert_eq!(
        s.effects,
        [
            "time None Set(24000)",
            "time Some(\"minecraft:overworld\") QueryTime",
            "time None QueryTimeline { timeline: Identifier(\"minecraft:moon\"), repetitions: true }",
            "time None Rate(2.5)",
            "time None Add(-100)",
            "time None SetMarker(Identifier(\"minecraft:day\"))",
            "time Some(\"minecraft:the_end\") Pause",
            "time None QueryGameTime",
        ]
    );
    assert_eq!(err_key(s.run(&d, "time add 1x")), "argument.time.invalid_unit");
    assert_eq!(err_key(s.run(&d, "weather rain 0")), "argument.time.tick_count_too_low");
    assert_eq!(err_key(s.run(&d, "time rate 0")), "argument.float.low");
    assert_eq!(err_key(s.run(&d, "time of minecraft:nether query time")), "argument.resource.not_found");
    // No `gametime` under a clock: the word is tried as a timeline.
    assert_eq!(err_key(s.run(&d, "time of minecraft:overworld query gametime")), "argument.resource.not_found");
    s.effects.clear();
    assert_eq!(s.run(&d, "weather thunder 10s"), Ok(200));
    assert_eq!(s.run(&d, "weather clear"), Ok(12000));
    assert_eq!(s.effects, ["weather thunder Some(200)", "weather clear None"]);
    assert_eq!(s.feedback_keys()[..2], ["commands.weather.set.thunder", "commands.weather.set.clear"]);
    s.feedback.clear();
    assert_eq!(s.run(&d, "gamerule keep_inventory true"), Ok(1));
    assert_eq!(s.run(&d, "gamerule minecraft:keep_inventory"), Ok(1));
    assert_eq!(err_key(s.run(&d, "gamerule keep_inventory true")), "commands.gamerule.not_set");
    assert_eq!(s.run(&d, "gamerule random_tick_speed 10"), Ok(10));
    assert_eq!(err_key(s.run(&d, "gamerule max_minecart_speed 2000")), "argument.integer.big");
    assert_eq!(err_key(s.run(&d, "gamerule keep_inventory 1")), "parsing.bool.invalid");
    assert_eq!(
        s.feedback_keys(),
        [
            "commands.gamerule.set[keep_inventory, true]",
            "commands.gamerule.query[keep_inventory, true]",
            "commands.gamerule.set[random_tick_speed, 10]",
        ]
    );
}

#[test]
fn server_commands() {
    let d = dispatcher();
    let s = &mut Mock::new(4);
    assert_eq!(s.run(&d, "seed"), Ok(-1234567890123i64 as i32));
    assert_eq!(s.feedback.pop().unwrap().0, "commands.seed.success[[-1234567890123]]");
    assert_eq!(s.run(&d, "difficulty"), Ok(2));
    assert_eq!(s.run(&d, "difficulty hard"), Ok(0));
    assert_eq!(s.difficulty, Difficulty::Hard);
    assert_eq!(err_key(s.run(&d, "difficulty hard")), "commands.difficulty.failure");
    assert_eq!(s.run(&d, "op Bob"), Ok(1));
    assert_eq!(err_key(s.run(&d, "op Bob")), "commands.op.failed");
    assert_eq!(s.run(&d, "op Notch"), Ok(1));
    assert_eq!(err_key(s.run(&d, "op Nobody")), "argument.player.unknown");
    assert_eq!(s.run(&d, "deop @a"), Ok(2));
    assert_eq!(s.ops, ["Notch"]);
    assert_eq!(s.run(&d, "kick Bob"), Ok(1));
    assert_eq!(s.run(&d, "kick @a[name=!Alice] bye now"), Ok(2));
    assert_eq!(s.effects, ["kick Bob multiplayer.disconnect.kicked", "kick Bob bye now", "kick Carol bye now"]);
    s.effects.clear();
    assert_eq!(s.run(&d, "spawnpoint"), Ok(1));
    assert_eq!(s.run(&d, "spawnpoint @a[distance=..20] ~ ~1 ~ 200 100"), Ok(2));
    assert_eq!(err_key(s.run(&d, "spawnpoint Bob 30000000 0 0")), "argument.pos.outofbounds");
    assert_eq!(s.run(&d, "setworldspawn 1 2 3 ~45 ~"), Ok(1));
    assert_eq!(
        s.effects,
        [
            "spawnpoint Alice [0, 64, 0] 0 0 minecraft:overworld",
            "spawnpoint Alice [0, 65, 0] -160 90 minecraft:overworld",
            "spawnpoint Bob [0, 65, 0] -160 90 minecraft:overworld",
            "worldspawn [1, 2, 3] 45 0 minecraft:overworld",
        ]
    );
    assert!(
        s.feedback_keys().contains(
            &"commands.spawnpoint.success.multiple[0, 65, 0, -160.0, 90.0, minecraft:overworld, 2]".to_owned()
        )
    );
    assert_eq!(s.run(&d, "kiln tick"), Ok(2));
    assert_eq!(s.run(&d, "kiln regions"), Ok(1));
    assert_eq!(s.run(&d, "stop"), Ok(1));
    assert_eq!(s.effects.last().unwrap(), "stop");
    assert_eq!(err_key(Mock::new(3).run(&d, "stop")), "command.unknown.command");
}

#[test]
fn help() {
    let d = dispatcher();
    let s = &mut Mock::new(0);
    let n = s.run(&d, "help").unwrap();
    let lines = s.feedback_keys();
    assert_eq!(n as usize, lines.len());
    assert!(lines.contains(&"/msg <targets> <message>".to_owned()));
    assert!(lines.contains(&"/tell -> msg".to_owned()));
    assert!(lines.contains(&"/list [uuids]".to_owned()));
    assert!(!lines.iter().any(|l| l.starts_with("/gamemode")), "not usable at level 0");
    let s = &mut Mock::new(2);
    s.run(&d, "help gamemode").unwrap();
    assert_eq!(s.feedback_keys(), ["/gamemode <gamemode> [<target>]"]);
    s.feedback.clear();
    s.run(&d, "help weather").unwrap();
    assert_eq!(
        s.feedback_keys(),
        ["/weather clear [<duration>]", "/weather rain [<duration>]", "/weather thunder [<duration>]"]
    );
    assert_eq!(err_key(s.run(&d, "help nothing")), "commands.help.failed");
}

#[test]
fn parse_errors_render_like_vanilla() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    let e = s.run(&d, "gamemode creatve").unwrap_err();
    assert_eq!(e.to_string(), "argument.gamemode.invalid[creatve] at position 9: gamemode <--[HERE]");
    let lines = e.chat_lines("gamemode creatve");
    assert_eq!(lines.len(), 2);
    let e = s.run(&d, "kill @e[type=zombie,foo=1]").unwrap_err();
    assert_eq!((e.key(), e.cursor()), (Some("argument.entity.options.unknown"), Some(20)));
    // Several alternatives failed (location, destination, targets): no single error to show.
    let e = s.run(&d, "tp @e[foo=1] 0 0 0").unwrap_err();
    assert_eq!((e.key(), e.cursor()), (Some("command.unknown.argument"), Some(3)));
    let e = s.run(&d, "kill @e extra").unwrap_err();
    assert_eq!((e.key(), e.cursor()), (Some("command.unknown.argument"), Some(8)));
    let e = s.run(&d, "notacommand").unwrap_err();
    assert_eq!((e.key(), e.cursor()), (Some("command.unknown.command"), Some(0)));
}

#[test]
fn suggestions() {
    let d = dispatcher();
    let s = &Mock::new(0);
    let root = d.complete("/", s).texts().into_iter().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(root, ["help", "list", "me", "msg", "random", "teammsg", "tell", "tm", "trigger", "w"]);
    let s4 = &Mock::new(4);
    assert_eq!(d.complete("/", s4).list.len(), COMMANDS.len());
    assert_eq!(d.complete("/gamemode ", s4).texts(), ["adventure", "creative", "spectator", "survival"]);
    assert_eq!(
        d.complete("/gamemode creative ", s4).texts(),
        ["@a", "@e", "@n", "@p", "@r", "@s", "Alice", "Bob", "Carol"]
    );
    assert_eq!(d.complete("/msg b", s).texts(), ["Bob"]);
    assert_eq!(d.complete("/tell c", s).texts(), ["Carol"], "through the redirect");
    // ask_server providers.
    assert_eq!(d.complete("/op ", s4).texts(), ["Bob", "Carol"]);
    assert_eq!(d.complete("/deop ", s4).texts(), ["Alice"]);
    assert_eq!(d.complete("/time set ", s4).texts(), ["minecraft:day", "minecraft:night"]);
    assert_eq!(d.complete("/time of minecraft:the_end set ", s4).texts(), Vec::<&str>::new());
    assert_eq!(d.complete("/time query m", s4).texts(), ["minecraft:day", "minecraft:moon"]);
    let sug = d.complete("/difficulty e", s4);
    assert_eq!((sug.start, sug.texts()), (12, vec!["easy"]));
    assert_eq!(d.complete("/gamerule keep_inv", s4).texts(), ["keep_inventory"]);
    assert_eq!(d.complete("/kiln ", s4).texts(), ["break", "recipebook", "regions", "tick", "use"]);
    // The packet answering a request: ranges in UTF-16 units of the request text.
    let p = d.suggestions_packet(5, "/msg Bob é@", s);
    let mut r = kiln_proto::Reader::new(&p);
    assert_eq!(r.varint().unwrap(), kiln_data::packets::play::clientbound::COMMAND_SUGGESTIONS);
    assert_eq!(r.varint().unwrap(), 5);
}

/// Commands come from players: no input may panic the parser, completion or execution.
#[test]
fn arbitrary_input_never_panics() {
    let d = dispatcher();
    let pieces = [
        "tp ",
        "teleport ",
        "give ",
        "gamemode ",
        "msg ",
        "say ",
        "time ",
        "of ",
        "kill ",
        "gamerule ",
        "help ",
        "@a",
        "@e[",
        "@s",
        "@",
        "[",
        "]",
        "=",
        ",",
        "!",
        "#",
        "type=",
        "limit=",
        "sort=",
        "nbt={",
        "scores={",
        "advancements={",
        "distance=..",
        "x=",
        "~",
        "^",
        " ",
        "1",
        "-2.5",
        ".5",
        "..",
        "\"",
        "'",
        "\\",
        "{",
        "}",
        "[I;",
        "é",
        "𝄞",
        "minecraft:",
        "stone",
        "diamond_sword[",
        "damage=",
        "1d",
        "creative",
        "Alice",
        "0-0-0-0-1",
        "execute ",
        "as ",
        "at ",
        "run ",
        "if ",
        "unless ",
        "block ",
        "blocks ",
        "store ",
        "result ",
        "score ",
        "positioned ",
        "over ",
        "facing ",
        "setblock ",
        "fill ",
        "clone ",
        "tellraw ",
        "{text:",
        "#minecraft:logs",
        "oak_log[",
        "axis=",
        "0x",
        "1b",
        "[B;",
        "\\u00",
        "bool(",
        "masked",
        "0 64 0 ",
    ];
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20_000 {
        let n = 1 + next() % 8;
        let input: String = (0..n).map(|_| pieces[(next() % pieces.len() as u64) as usize]).collect();
        for level in [0, 4] {
            let mut s = Mock::new(level);
            let _ = d.execute(&input, &mut s);
            let _ = d.complete(&format!("/{input}"), &s);
            let _ = d.suggestions(&input, (next() % (input.len() as u64 + 2)) as usize, &s);
        }
    }
}

// ---- tree shape against vanilla --------------------------------------------------------

const LEVELS: [&str; 5] = ["all", "moderators", "gamemasters", "admins", "owners"];

fn f32_json(v: f32) -> Value {
    json!(v.to_string().parse::<f64>().unwrap())
}

fn properties(ty: &ArgumentType) -> Option<Value> {
    let bounds = |min: Option<Value>, max: Option<Value>| {
        let mut m = serde_json::Map::new();
        if let Some(v) = min {
            m.insert("min".into(), v);
        }
        if let Some(v) = max {
            m.insert("max".into(), v);
        }
        (!m.is_empty()).then_some(Value::Object(m))
    };
    match ty.wire() {
        Parser::Integer { min, max } => bounds(min.map(|v| json!(v)), max.map(|v| json!(v))),
        Parser::Long { min, max } => bounds(min.map(|v| json!(v)), max.map(|v| json!(v))),
        Parser::Float { min, max } => bounds(min.map(f32_json), max.map(f32_json)),
        Parser::Double { min, max } => bounds(min.map(|v| json!(v)), max.map(|v| json!(v))),
        Parser::String(k) => Some(
            json!({"type": match k { StringKind::Word => "word", StringKind::Phrase => "phrase", StringKind::Greedy => "greedy" }}),
        ),
        Parser::Entity { single, players_only } => Some(json!({
            "type": if players_only { "players" } else { "entities" },
            "amount": if single { "single" } else { "multiple" },
        })),
        Parser::Time { min } => Some(json!({"min": min})),
        Parser::Registry { registry, .. } => Some(json!({"registry": registry})),
        Parser::ScoreHolder { multiple } => Some(json!({"amount": if multiple { "multiple" } else { "single" }})),
        Parser::Plain(_) => None,
    }
}

/// A node in the data generator's `commands.json` format (`ArgumentUtils.serializeNodeToJson`).
fn to_json(d: &Dispatcher<Mock>, id: crate::dispatcher::NodeId) -> Value {
    let mut o = serde_json::Map::new();
    match d.kind(id) {
        NodeKind::Root => o.insert("type".into(), json!("root")),
        NodeKind::Literal(_) => o.insert("type".into(), json!("literal")),
        NodeKind::Argument { ty, .. } => {
            o.insert("type".into(), json!("argument"));
            o.insert("parser".into(), json!(ty.wire().id()));
            if let Some(p) = properties(ty) {
                o.insert("properties".into(), p);
            }
            None
        }
    };
    if !d.children(id).is_empty() {
        let children: serde_json::Map<String, Value> =
            d.children(id).iter().map(|&c| (d.path(c).last().unwrap().to_string(), to_json(d, c))).collect();
        o.insert("children".into(), Value::Object(children));
    }
    if d.is_executable(id) {
        o.insert("executable".into(), json!(true));
    }
    // `ArgumentUtils` omits redirects to the root (an empty path).
    if let Some(r) = d.redirect_of(id).filter(|&r| r != d.root()) {
        o.insert("redirect".into(), json!(d.path(r)));
    }
    if d.permission(id) > 0 {
        o.insert(
            "permissions".into(),
            json!({"type": "minecraft:require", "permission": {"type": "minecraft:command_level", "level": LEVELS[d.permission(id) as usize]}}),
        );
    }
    Value::Object(o)
}

fn first_difference(path: &str, ours: &Value, theirs: &Value) -> Option<String> {
    match (ours, theirs) {
        (Value::Object(a), Value::Object(b)) => {
            // Each key once: visiting shared keys twice doubles the work per level.
            for k in a.keys().chain(b.keys().filter(|k| !a.contains_key(*k))) {
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => {
                        if let Some(d) = first_difference(&format!("{path}.{k}"), x, y) {
                            return Some(d);
                        }
                    }
                    (x, y) => return Some(format!("{path}.{k}: ours {x:?}, vanilla {y:?}")),
                }
            }
            None
        }
        _ => (ours != theirs).then(|| format!("{path}: ours {ours}, vanilla {theirs}")),
    }
}

#[test]
fn tree_matches_vanilla_commands_json() {
    let work = std::env::var_os("KILN_WORK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let path = work.join("generated/reports/commands.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {} not found (run the data generator)", path.display());
        return;
    };
    let vanilla: Value = serde_json::from_str(&text).unwrap();
    let d = dispatcher();
    let mut compared = 0;
    let mut nodes = 0;
    for name in COMMANDS {
        let id = d.find(&[name]).unwrap();
        let ours = to_json(&d, id);
        match vanilla["children"].get(*name) {
            Some(theirs) => {
                if let Some(diff) = first_difference(name, &ours, theirs) {
                    panic!("/{name} differs from vanilla: {diff}");
                }
                compared += 1;
                nodes += count(&ours);
            }
            None => assert_eq!(*name, "kiln", "only /kiln is Kiln-specific"),
        }
    }
    assert_eq!(compared, COMMANDS.len() - 1);
    eprintln!("{compared} commands ({nodes} nodes) match commands.json");
}

fn count(v: &Value) -> usize {
    1 + v.get("children").and_then(Value::as_object).map_or(0, |c| c.values().map(count).sum())
}

// ---- commands packet --------------------------------------------------------------------

#[test]
fn commands_packet_flags() {
    use kiln_proto::Reader;
    let d = dispatcher();
    let decode = |level: u8| {
        let p = d.commands_packet(level);
        let mut r = Reader::new(&p);
        assert_eq!(r.varint().unwrap(), kiln_data::packets::play::clientbound::COMMANDS);
        let n = r.varint().unwrap();
        let mut literals = Vec::new();
        let mut ask_server = Vec::new();
        let mut restricted = Vec::new();
        for _ in 0..n {
            let flags = r.u8().unwrap();
            let children = r.varint().unwrap();
            for _ in 0..children {
                r.varint().unwrap();
            }
            if flags & 0x08 != 0 {
                r.varint().unwrap();
            }
            match flags & 3 {
                1 => {
                    let name = r.string(32767).unwrap().to_owned();
                    if flags & 0x20 != 0 {
                        restricted.push(name.clone());
                    }
                    literals.push(name);
                }
                2 => {
                    let name = r.string(32767).unwrap().to_owned();
                    let parser = kiln_data::builtin_entries("minecraft:command_argument_type").unwrap()
                        [r.varint().unwrap() as usize];
                    // Skip properties for the parsers used by the built-in commands.
                    match parser {
                        "brigadier:integer" | "brigadier:float" => {
                            let f = r.u8().unwrap();
                            r.bytes(4 * (f & 1) as usize + 4 * ((f >> 1) & 1) as usize).unwrap();
                        }
                        "brigadier:double" => {
                            let f = r.u8().unwrap();
                            r.bytes(8 * (f & 1) as usize + 8 * ((f >> 1) & 1) as usize).unwrap();
                        }
                        "brigadier:string" => drop(r.varint().unwrap()),
                        "minecraft:entity" | "minecraft:score_holder" => drop(r.u8().unwrap()),
                        "minecraft:time" => drop(r.i32().unwrap()),
                        "minecraft:resource" | "minecraft:resource_key" | "minecraft:resource_or_tag" | "minecraft:resource_or_tag_key" => drop(r.string(32767).unwrap()),
                        _ => {}
                    }
                    if flags & 0x10 != 0 {
                        match r.string(32767).unwrap() {
                            "minecraft:ask_server" => ask_server.push(name),
                            other => assert_eq!((name.as_str(), other), ("entity", "minecraft:summonable_entities")),
                        }
                    }
                }
                _ => {}
            }
        }
        assert_eq!(r.varint().unwrap(), 0, "root index");
        r.finish().unwrap();
        (n, literals, ask_server, restricted)
    };
    let (n0, lit0, ask0, res0) = decode(0);
    assert!(lit0.contains(&"msg".to_owned()) && !lit0.contains(&"gamemode".to_owned()));
    assert!(ask0 == ["objective"] && res0.is_empty(), "trigger suggests its objectives");
    let (n4, lit4, ask4, res4) = decode(4);
    assert!(n4 > n0 + 100);
    assert!(lit4.contains(&"kiln".to_owned()));
    let allowed = ["targets", "timemarker", "timeline", "target", "source", "id", "objective", "members", "name", "function", "existing", "criterion", "rate", "time", "sequence"];
    ask4.iter().for_each(|a| assert!(allowed.contains(&a.as_str()), "{a}"));
    // op, deop, time's markers/timelines at both levels, execute's score holders (if and
    // unless: target + 5 sources each; store result and success: targets) and boss bars, and
    // scoreboard's 11 score holders plus `players enable`'s trigger objectives, trigger's
    // objective, team's join and leave members and bossbar's remove, set and get ids.
    // Functions: function, schedule function/clear, datapack enable/after/before/disable,
    // execute if/unless function.
    // Advancement criteria: grant and revoke only.
    // tick rate, step and sprint times; random value, roll and reset sequences.
    assert_eq!(ask4.len(), 6 + 2 * 6 + 2 * 2 + 11 + 1 + 1 + 2 + 3 + 1 + 2 + 4 + 2 + 2 + 3 + 3);
    assert!(res4.contains(&"stop".to_owned()) && res4.contains(&"tp".to_owned()) && !res4.contains(&"msg".to_owned()));
}

/// Writes packet bodies (without the packet id) for `tools/vanilla_decode.py` when
/// `KILN_COMMAND_DUMP` names a directory.
#[test]
fn dump_packets_for_vanilla_check() {
    let d = dispatcher();
    let s = &Mock::new(4);
    let mut error_text = Mock::new(2).run(&d, "gamemode creatve").unwrap_err().chat_lines("gamemode creatve");
    error_text.push(crate::tr!("commands.seed.success", super::server::copy_on_click("42")));
    let packets = [
        ("commands_level4.bin", d.commands_packet(4)),
        ("commands_level0.bin", d.commands_packet(0)),
        ("command_suggestions.bin", d.suggestions_packet(9, "/gamemode creative ", s)),
        ("command_suggestions_ask.bin", d.suggestions_packet(10, "/time set ", s)),
        ("system_chat_error_message.bin", kiln_proto::packets::system_chat(error_text[0].to_nbt(), false)),
        ("system_chat_error_context.bin", kiln_proto::packets::system_chat(error_text[1].to_nbt(), false)),
        ("system_chat_seed.bin", kiln_proto::packets::system_chat(error_text[2].to_nbt(), false)),
    ];
    let Ok(dir) = std::env::var("KILN_COMMAND_DUMP") else { return };
    // Relative to the workspace root (tests run in the crate directory).
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, p) in packets {
        let mut r = kiln_proto::Reader::new(&p);
        r.varint().unwrap();
        std::fs::write(dir.join(name), r.rest()).unwrap();
    }
}
