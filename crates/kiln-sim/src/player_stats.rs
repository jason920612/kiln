//! Player statistics (`ServerStatsCounter`): a value per [`Stat`] (a stat type and an entry of
//! its registry), incremented where vanilla calls `Player.awardStat`, saved as
//! `players/stats/<uuid>.json`, sent in Award Stats when the client asks (only the stats that
//! changed since the last request) and mirrored into scoreboard objectives whose criterion is
//! the stat (`minecraft.used:minecraft.stone`).
//!
//! The increment API is [`Player::award_stat`] / [`Player::reset_stat`] with a [`Stat`] (for
//! custom stats [`custom`] or the statics in [`stat`]). Region-parallel code can call them; the
//! scoreboard side is queued and applied in a serial phase ([`Sim::flush_player_updates`]).

use crate::{Player, Sim};
use bytes::{Bytes, BytesMut};
use kiln_proto::WriteExt;
use std::sync::LazyLock;

/// `minecraft:stat_type` registry order.
pub(crate) const MINED: u8 = 0;
pub(crate) const CRAFTED: u8 = 1;
pub(crate) const USED: u8 = 2;
pub(crate) const BROKEN: u8 = 3;
pub(crate) const PICKED_UP: u8 = 4;
pub(crate) const DROPPED: u8 = 5;
pub(crate) const KILLED: u8 = 6;
pub(crate) const KILLED_BY: u8 = 7;
pub(crate) const CUSTOM: u8 = 8;

const TYPE_NAMES: [&str; 9] = [
    "minecraft:mined",
    "minecraft:crafted",
    "minecraft:used",
    "minecraft:broken",
    "minecraft:picked_up",
    "minecraft:dropped",
    "minecraft:killed",
    "minecraft:killed_by",
    "minecraft:custom",
];

/// The registry each stat type counts entries of.
const TYPE_REGISTRIES: [&str; 9] = [
    "minecraft:block",
    "minecraft:item",
    "minecraft:item",
    "minecraft:item",
    "minecraft:item",
    "minecraft:item",
    "minecraft:entity_type",
    "minecraft:entity_type",
    "minecraft:custom_stat",
];

/// A statistic: stat type (`minecraft:stat_type` id) and the entry's id in that type's registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Stat {
    pub kind: u8,
    pub id: i32,
}

impl Stat {
    /// `Stats.CUSTOM.get(name)`; `None` for an unknown custom stat.
    pub fn custom(name: &str) -> Option<Stat> {
        kiln_data::builtin_id("minecraft:custom_stat", name).map(|id| Stat { kind: CUSTOM, id })
    }

    /// `Stats.BLOCK_MINED` of a block state's block.
    pub fn mined(state: u16) -> Stat {
        Stat { kind: MINED, id: kiln_data::block_logic::block_index(state) as i32 }
    }

    /// An item stat (`crafted`, `used`, `broken`, `picked_up`, `dropped`).
    pub fn item(kind: u8, item: i32) -> Stat {
        Stat { kind, id: item }
    }

    /// An entity stat (`killed`, `killed_by`) of a `minecraft:entity_type` id.
    pub fn entity(kind: u8, entity_type: i32) -> Stat {
        Stat { kind, id: entity_type }
    }

    pub fn type_name(self) -> &'static str {
        TYPE_NAMES[self.kind as usize]
    }

    /// The entry's name in its registry.
    pub fn value_name(self) -> Option<&'static str> {
        kiln_data::builtin_entries(TYPE_REGISTRIES[self.kind as usize])?.get(self.id as usize).copied()
    }

    /// By stat type and entry name (`minecraft:custom`, `minecraft:jump`).
    pub fn by_names(type_name: &str, value: &str) -> Option<Stat> {
        let kind = TYPE_NAMES.iter().position(|t| *t == type_name)?;
        let id = kiln_data::builtin_id(TYPE_REGISTRIES[kind], value)?;
        Some(Stat { kind: kind as u8, id })
    }

    /// `Stat.buildName`: the scoreboard criterion, `minecraft.custom:minecraft.jump`.
    pub fn criterion_name(self) -> String {
        let dotted = |s: &str| s.replacen(':', ".", 1);
        format!("{}:{}", dotted(self.type_name()), dotted(self.value_name().unwrap_or("minecraft:air")))
    }
}

/// A custom stat by name; panics for names that are not in `minecraft:custom_stat` (they are
/// vanilla constants).
pub fn custom(name: &str) -> Stat {
    Stat::custom(name).unwrap_or_else(|| panic!("unknown custom stat {name}"))
}

/// The custom stats Kiln awards on hot paths, resolved once.
#[allow(dead_code)]
pub mod stat {
    use super::{LazyLock, Stat, custom};
    macro_rules! customs {
        ($($name:ident = $id:literal),* $(,)?) => {
            $(pub static $name: LazyLock<Stat> = LazyLock::new(|| custom($id));)*
        };
    }
    customs! {
        LEAVE_GAME = "minecraft:leave_game",
        PLAY_TIME = "minecraft:play_time",
        TOTAL_WORLD_TIME = "minecraft:total_world_time",
        TIME_SINCE_DEATH = "minecraft:time_since_death",
        TIME_SINCE_REST = "minecraft:time_since_rest",
        CROUCH_TIME = "minecraft:sneak_time",
        WALK_ONE_CM = "minecraft:walk_one_cm",
        CROUCH_ONE_CM = "minecraft:crouch_one_cm",
        SPRINT_ONE_CM = "minecraft:sprint_one_cm",
        WALK_ON_WATER_ONE_CM = "minecraft:walk_on_water_one_cm",
        FALL_ONE_CM = "minecraft:fall_one_cm",
        CLIMB_ONE_CM = "minecraft:climb_one_cm",
        FLY_ONE_CM = "minecraft:fly_one_cm",
        WALK_UNDER_WATER_ONE_CM = "minecraft:walk_under_water_one_cm",
        MINECART_ONE_CM = "minecraft:minecart_one_cm",
        BOAT_ONE_CM = "minecraft:boat_one_cm",
        PIG_ONE_CM = "minecraft:pig_one_cm",
        HORSE_ONE_CM = "minecraft:horse_one_cm",
        STRIDER_ONE_CM = "minecraft:strider_one_cm",
        HAPPY_GHAST_ONE_CM = "minecraft:happy_ghast_one_cm",
        NAUTILUS_ONE_CM = "minecraft:nautilus_one_cm",
        AVIATE_ONE_CM = "minecraft:aviate_one_cm",
        SWIM_ONE_CM = "minecraft:swim_one_cm",
        JUMP = "minecraft:jump",
        DROP = "minecraft:drop",
        DAMAGE_DEALT = "minecraft:damage_dealt",
        DAMAGE_DEALT_ABSORBED = "minecraft:damage_dealt_absorbed",
        DAMAGE_DEALT_RESISTED = "minecraft:damage_dealt_resisted",
        DAMAGE_TAKEN = "minecraft:damage_taken",
        DAMAGE_BLOCKED_BY_SHIELD = "minecraft:damage_blocked_by_shield",
        DAMAGE_ABSORBED = "minecraft:damage_absorbed",
        DAMAGE_RESISTED = "minecraft:damage_resisted",
        DEATHS = "minecraft:deaths",
        MOB_KILLS = "minecraft:mob_kills",
        ANIMALS_BRED = "minecraft:animals_bred",
        PLAYER_KILLS = "minecraft:player_kills",
        TRADED_WITH_VILLAGER = "minecraft:traded_with_villager",
        TALKED_TO_VILLAGER = "minecraft:talked_to_villager",
        EAT_CAKE_SLICE = "minecraft:eat_cake_slice",
        ENCHANT_ITEM = "minecraft:enchant_item",
        OPEN_CHEST = "minecraft:open_chest",
        OPEN_ENDERCHEST = "minecraft:open_enderchest",
        OPEN_BARREL = "minecraft:open_barrel",
        OPEN_SHULKER_BOX = "minecraft:open_shulker_box",
        TRIGGER_TRAPPED_CHEST = "minecraft:trigger_trapped_chest",
        INTERACT_WITH_CRAFTING_TABLE = "minecraft:interact_with_crafting_table",
        INTERACT_WITH_FURNACE = "minecraft:interact_with_furnace",
        INTERACT_WITH_BLAST_FURNACE = "minecraft:interact_with_blast_furnace",
        INTERACT_WITH_SMOKER = "minecraft:interact_with_smoker",
        INTERACT_WITH_BREWINGSTAND = "minecraft:interact_with_brewingstand",
        INTERACT_WITH_ANVIL = "minecraft:interact_with_anvil",
        INTERACT_WITH_GRINDSTONE = "minecraft:interact_with_grindstone",
        INTERACT_WITH_STONECUTTER = "minecraft:interact_with_stonecutter",
        INTERACT_WITH_SMITHING_TABLE = "minecraft:interact_with_smithing_table",
        INTERACT_WITH_LOOM = "minecraft:interact_with_loom",
        INTERACT_WITH_CARTOGRAPHY_TABLE = "minecraft:interact_with_cartography_table",
        INSPECT_HOPPER = "minecraft:inspect_hopper",
        INSPECT_DROPPER = "minecraft:inspect_dropper",
        INSPECT_DISPENSER = "minecraft:inspect_dispenser",
        SLEEP_IN_BED = "minecraft:sleep_in_bed",
        PLAY_RECORD = "minecraft:play_record",
    }
}

/// What a stat change does to the scoreboard objectives following it (applied serially).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScoreOp {
    /// `awardStat`: every objective of the stat's criterion adds this.
    Add(i32),
    /// `resetStat`: the scores are reset.
    Reset,
    /// A game-maintained criterion (`health`, `food`, ...) takes this value.
    Set(i32),
}

/// A scoreboard update for the player's objectives of one criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Criterion {
    Stat(Stat),
    /// A custom criterion by name (`health`, `deathCount`, `teamkill.red`, ...).
    Named(&'static str),
    /// A team criterion built at run time (`teamkill.<color>`, `killedByTeam.<color>`).
    Team(String),
}

/// `ServerStatsCounter` of one player.
#[derive(Debug, Default)]
pub struct PlayerStats {
    values: crate::FastMap<Stat, i32>,
    /// Changed since the client last asked (`getDirty`).
    dirty: crate::FastSet<Stat>,
    /// Scoreboard updates for the serial phase.
    pub(crate) scores: Vec<(Criterion, ScoreOp)>,
    /// `lastRecorded*` of `ServerPlayer.doTick` for the game-maintained criteria: health
    /// (with absorption, as bits), food, air, armor, total experience, level.
    pub(crate) recorded: Option<(u32, i32, i32, i32, i32, i32)>,
}

impl PlayerStats {
    pub fn get(&self, stat: Stat) -> i32 {
        self.values.get(&stat).copied().unwrap_or(0)
    }

    /// `ServerStatsCounter.setValue`.
    pub fn set(&mut self, stat: Stat, value: i32) {
        self.values.insert(stat, value);
        self.dirty.insert(stat);
    }

    /// `StatsCounter.increment`: saturating at `Integer.MAX_VALUE`.
    pub fn increment(&mut self, stat: Stat, amount: i32) {
        let v = (self.get(stat) as i64 + amount as i64).min(i32::MAX as i64) as i32;
        self.set(stat, v);
    }

    /// Every stat with a value, in (type, id) order.
    pub fn entries(&self) -> Vec<(Stat, i32)> {
        let mut v: Vec<(Stat, i32)> = self.values.iter().map(|(s, v)| (*s, *v)).collect();
        v.sort_unstable();
        v
    }

    /// `markAllDirty` (on join, so the first request gets everything).
    pub fn mark_all_dirty(&mut self) {
        self.dirty.extend(self.values.keys().copied());
    }

    /// `sendStats`: Award Stats with the changed stats.
    pub fn take_award_packet(&mut self) -> Bytes {
        let mut dirty: Vec<Stat> = self.dirty.drain().collect();
        dirty.sort_unstable();
        let mut b = BytesMut::new();
        b.put_varint(kiln_data::packets::play::clientbound::AWARD_STATS);
        b.put_varint(dirty.len() as i32);
        for s in dirty {
            b.put_varint(s.kind as i32);
            b.put_varint(s.id);
            b.put_varint(self.get(s));
        }
        b.freeze()
    }

    /// `ServerStatsCounter.toJson` (compact, as vanilla's Gson writes it): every stat type's
    /// entries by name, and the data version.
    pub fn to_json(&self) -> String {
        let mut by_type: Vec<(String, serde_json::Map<String, serde_json::Value>)> = Vec::new();
        for (s, v) in self.entries() {
            let Some(name) = s.value_name() else { continue };
            let t = s.type_name();
            if by_type.last().is_none_or(|(n, _)| n != t) {
                by_type.push((t.to_owned(), serde_json::Map::new()));
            }
            by_type.last_mut().unwrap().1.insert(name.to_owned(), v.into());
        }
        let stats: serde_json::Map<String, serde_json::Value> =
            by_type.into_iter().map(|(t, m)| (t, serde_json::Value::Object(m))).collect();
        let mut root = serde_json::Map::new();
        root.insert("stats".into(), serde_json::Value::Object(stats));
        root.insert("DataVersion".into(), kiln_storage::anvil::DATA_VERSION.into());
        serde_json::Value::Object(root).to_string()
    }

    /// `ServerStatsCounter.parse`: unknown stat types or entries are skipped (vanilla logs
    /// them and keeps the rest).
    pub fn from_json(text: &str) -> Result<PlayerStats, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut stats = PlayerStats::default();
        let Some(types) = root.get("stats").and_then(|s| s.as_object()) else { return Ok(stats) };
        for (t, entries) in types {
            let Some(entries) = entries.as_object() else { continue };
            for (name, v) in entries {
                let (Some(stat), Some(v)) = (Stat::by_names(&normalize(t), &normalize(name)), v.as_i64()) else { continue };
                stats.values.insert(stat, v as i32);
            }
        }
        Ok(stats)
    }
}

/// The statistic a block's menu awards when a player opens it (`useWithoutItem`), by block.
pub(crate) fn interact_stat(state: u16) -> Option<Stat> {
    let name = kiln_data::builtin_entries("minecraft:block")?.get(kiln_data::block_logic::block_index(state)).copied()?;
    let s = match name.trim_start_matches("minecraft:") {
        "chest" => &stat::OPEN_CHEST,
        "trapped_chest" => &stat::TRIGGER_TRAPPED_CHEST,
        "barrel" => &stat::OPEN_BARREL,
        n if n.ends_with("shulker_box") => &stat::OPEN_SHULKER_BOX,
        "furnace" => &stat::INTERACT_WITH_FURNACE,
        "blast_furnace" => &stat::INTERACT_WITH_BLAST_FURNACE,
        "smoker" => &stat::INTERACT_WITH_SMOKER,
        "brewing_stand" => &stat::INTERACT_WITH_BREWINGSTAND,
        "hopper" => &stat::INSPECT_HOPPER,
        "dropper" => &stat::INSPECT_DROPPER,
        "dispenser" => &stat::INSPECT_DISPENSER,
        "crafting_table" => &stat::INTERACT_WITH_CRAFTING_TABLE,
        "anvil" | "chipped_anvil" | "damaged_anvil" => &stat::INTERACT_WITH_ANVIL,
        "grindstone" => &stat::INTERACT_WITH_GRINDSTONE,
        "stonecutter" => &stat::INTERACT_WITH_STONECUTTER,
        "smithing_table" => &stat::INTERACT_WITH_SMITHING_TABLE,
        "loom" => &stat::INTERACT_WITH_LOOM,
        "cartography_table" => &stat::INTERACT_WITH_CARTOGRAPHY_TABLE,
        _ => return None,
    };
    Some(**s)
}

/// `BlockTags.CLIMBABLE` (ladders, vines, scaffolding...).
pub(crate) fn climbable(state: u16) -> bool {
    kiln_entity::blocks::has_tag(state, kiln_entity::blocks::Tag::Climbable)
}

/// An identifier with the default namespace spelled out.
fn normalize(id: &str) -> String {
    if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") }
}

impl Player {
    /// `ServerPlayer.awardStat`: the stat grows and objectives of its criterion follow.
    pub(crate) fn award_stat(&mut self, stat: Stat, amount: i32) {
        self.stats.increment(stat, amount);
        self.stats.scores.push((Criterion::Stat(stat), ScoreOp::Add(amount)));
    }

    /// `ServerPlayer.resetStat`: back to zero, and the objectives' scores are reset.
    pub(crate) fn reset_stat(&mut self, stat: Stat) {
        self.stats.set(stat, 0);
        self.stats.scores.push((Criterion::Stat(stat), ScoreOp::Reset));
    }

    /// `ServerScoreboard.forAllObjectives` with an update for a custom criterion.
    pub(crate) fn update_criterion(&mut self, criterion: &'static str, op: ScoreOp) {
        self.stats.scores.push((Criterion::Named(criterion), op));
    }

    /// The per-tick stats of `ServerPlayer.doTick`, then the game-maintained criteria when
    /// their values changed.
    pub(crate) fn tick_stats(&mut self) {
        self.award_stat(*stat::PLAY_TIME, 1);
        self.award_stat(*stat::TOTAL_WORLD_TIME, 1);
        if !self.dead {
            self.award_stat(*stat::TIME_SINCE_DEATH, 1);
        }
        if self.sneaking {
            self.award_stat(*stat::CROUCH_TIME, 1);
        }
        if self.sleep.pos.is_none() {
            self.award_stat(*stat::TIME_SINCE_REST, 1);
        }
        let armor = self.armor_value();
        let now = ((self.health + self.absorption).to_bits(), self.food, self.air, armor, self.xp_total, self.xp_level);
        let last = self.stats.recorded.unwrap_or((u32::MAX, i32::MIN, i32::MIN, i32::MIN, i32::MIN, i32::MIN));
        if now.0 != last.0 {
            self.update_criterion("health", ScoreOp::Set((self.health + self.absorption).ceil() as i32));
        }
        if now.1 != last.1 {
            self.update_criterion("food", ScoreOp::Set(now.1));
        }
        if now.2 != last.2 {
            self.update_criterion("air", ScoreOp::Set(now.2));
        }
        if now.3 != last.3 {
            self.update_criterion("armor", ScoreOp::Set(now.3));
        }
        if now.4 != last.4 {
            self.update_criterion("xp", ScoreOp::Set(now.4));
        }
        if now.5 != last.5 {
            self.update_criterion("level", ScoreOp::Set(now.5));
        }
        self.stats.recorded = Some(now);
    }

    /// `ServerPlayer.checkMovementStatistics` for a move by `d` (not riding).
    pub(crate) fn movement_stats(&mut self, d: [f64; 3], in_water: bool, eyes_in_water: bool, climbing: bool) {
        if self.vehicle.is_some() || d == [0.0; 3] {
            return;
        }
        let full = ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32 * 100.0).round() as i32;
        let horizontal = ((d[0] * d[0] + d[2] * d[2]).sqrt() as f32 * 100.0).round() as i32;
        if self.swimming() {
            if full > 0 {
                self.award_stat(*stat::SWIM_ONE_CM, full);
            }
        } else if eyes_in_water {
            if full > 0 {
                self.award_stat(*stat::WALK_UNDER_WATER_ONE_CM, full);
            }
        } else if in_water {
            if horizontal > 0 {
                self.award_stat(*stat::WALK_ON_WATER_ONE_CM, horizontal);
            }
        } else if climbing {
            if d[1] > 0.0 {
                self.award_stat(*stat::CLIMB_ONE_CM, (d[1] * 100.0).round() as i32);
            }
        } else if self.on_ground {
            if horizontal > 0 {
                let s = if self.sprinting {
                    *stat::SPRINT_ONE_CM
                } else if self.sneaking {
                    *stat::CROUCH_ONE_CM
                } else {
                    *stat::WALK_ONE_CM
                };
                self.award_stat(s, horizontal);
            }
        } else if horizontal > 25 {
            self.award_stat(*stat::FLY_ONE_CM, horizontal);
        }
    }

    /// `Player.killedEntity` and `ServerPlayer.awardKillScore` for a mob this player killed.
    pub(crate) fn killed_entity(&mut self, entity_type: &str) {
        if let Some(id) = kiln_item::registry::ENTITY_TYPE.id(entity_type) {
            self.award_stat(Stat::entity(KILLED, id), 1);
        }
        self.update_criterion("totalKillCount", ScoreOp::Add(1));
        self.award_stat(*stat::MOB_KILLS, 1);
    }

    /// `ServerPlayer.checkRidingStatistics` for a move by `d` on a vehicle of `vehicle_type`.
    pub(crate) fn riding_stats(&mut self, d: [f64; 3], vehicle_type: &str) {
        if d == [0.0; 3] {
            return;
        }
        let cm = ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32 * 100.0).round() as i32;
        let t = vehicle_type.trim_start_matches("minecraft:");
        let s = if t.ends_with("minecart") {
            &stat::MINECART_ONE_CM
        } else if t.ends_with("boat") || t.ends_with("raft") {
            &stat::BOAT_ONE_CM
        } else if t == "pig" {
            &stat::PIG_ONE_CM
        } else if matches!(t, "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "llama" | "trader_llama" | "camel" | "camel_husk") {
            &stat::HORSE_ONE_CM
        } else if t == "strider" {
            &stat::STRIDER_ONE_CM
        } else if t == "happy_ghast" {
            &stat::HAPPY_GHAST_ONE_CM
        } else if t.ends_with("nautilus") {
            &stat::NAUTILUS_ONE_CM
        } else {
            return;
        };
        self.award_stat(**s, cm);
    }

    /// `getArmorValue`.
    pub(crate) fn armor_value(&self) -> i32 {
        crate::combat::floor(self.attribute(crate::combat::ARMOR))
    }

    /// Swimming pose: Kiln does not track the swimming flag yet.
    fn swimming(&self) -> bool {
        false
    }
}

impl Sim {
    /// Serial upkeep of what region work queued on players: scoreboard objectives following
    /// stats and game-maintained criteria.
    pub(crate) fn flush_stat_scores(&mut self) {
        // Without an objective that follows a statistic, the updates only go away.
        if self.commands.scoreboard.objectives().iter().all(|o| o.criterion == "dummy" || o.criterion == "trigger") {
            for p in self.players.values_mut() {
                p.stats.scores.clear();
            }
            return;
        }
        let mut conns: Vec<_> = self.players.keys().copied().collect();
        conns.sort_unstable();
        let mut touched = false;
        for conn in conns {
            let Some(p) = self.players.get_mut(&conn) else { continue };
            if p.stats.scores.is_empty() {
                continue;
            }
            let updates = std::mem::take(&mut p.stats.scores);
            let name = p.name.clone();
            let board = &mut self.commands.scoreboard;
            let objectives: Vec<(String, String)> =
                board.objectives().iter().map(|o| (o.name.clone(), o.criterion.clone())).collect();
            if objectives.iter().all(|(_, c)| c == "dummy" || c == "trigger") {
                continue;
            }
            for (criterion, op) in updates {
                let key = match &criterion {
                    Criterion::Stat(s) => s.criterion_name(),
                    Criterion::Named(n) => (*n).to_owned(),
                    Criterion::Team(t) => t.clone(),
                };
                for (obj, _) in objectives.iter().filter(|(_, c)| *c == key) {
                    touched = true;
                    match op {
                        ScoreOp::Add(n) => {
                            let mut a = board.access(&name, obj);
                            board.add(&mut a, n);
                        }
                        ScoreOp::Set(v) => {
                            let mut a = board.access(&name, obj);
                            board.set(&mut a, v);
                        }
                        ScoreOp::Reset => board.reset(&name, Some(obj)),
                    }
                }
            }
        }
        if touched {
            self.flush_scoreboard();
        }
    }

    /// `ServerPlayer.awardKillScore` for a player killed by another (the victim's own side
    /// happened when it died): kill counts, `teamkill.<victim's color>` for the killer and
    /// `killedByTeam.<killer's color>` for the victim.
    pub(crate) fn award_kill_score(&mut self, d: &crate::health::Death) {
        let Some(killer) = d.killer.as_deref() else { return };
        let Some(victim) = self.players.get(&d.conn).map(|p| p.name.clone()) else { return };
        if victim == killer {
            return;
        }
        let color = |name: &str| {
            self.commands.scoreboard.team_of(name).and_then(|t| t.color).map(|c| kiln_command::arguments::TEAM_COLORS[c])
        };
        let (victim_color, killer_color) = (color(&victim), color(killer));
        let victim_subject = self.players.get(&d.conn).map(|v| {
            let mut s = v.subject(None);
            s.equipment.clear();
            (s.type_id, s.pos, s.dim)
        });
        if let Some(k) = self.players.values_mut().find(|p| p.name == killer) {
            if let Some((type_id, pos, dim)) = victim_subject {
                let s = crate::advancements::criteria::Subject {
                    type_id,
                    pos,
                    dim,
                    on_ground: true,
                    on_fire: false,
                    sneaking: false,
                    sprinting: false,
                    flying: false,
                    baby: false,
                    equipment: Vec::new(),
                    world: None,
                    components: Default::default(),
                    effects: Vec::new(),
                    vehicle: None,
                    lightning_fires: None,
                };
                k.killed("minecraft:player_killed_entity", &s, "minecraft:player_attack", true);
            }
            k.update_criterion("totalKillCount", ScoreOp::Add(1));
            k.award_stat(*stat::PLAYER_KILLS, 1);
            k.update_criterion("playerKillCount", ScoreOp::Add(1));
            if let Some(id) = kiln_item::registry::ENTITY_TYPE.id("minecraft:player") {
                k.award_stat(Stat::entity(KILLED, id), 1);
            }
            if let Some(c) = victim_color {
                k.stats.scores.push((Criterion::Team(format!("teamkill.{c}")), ScoreOp::Add(1)));
            }
        }
        if let (Some(c), Some(v)) = (killer_color, self.players.get_mut(&d.conn)) {
            v.stats.scores.push((Criterion::Team(format!("killedByTeam.{c}")), ScoreOp::Add(1)));
        }
    }

    /// `players/stats/<uuid>.json`.
    pub(crate) fn stats_path(&self, uuid: uuid::Uuid) -> Option<std::path::PathBuf> {
        self.storage.as_ref().map(|s| s.dir.join("players/stats").join(format!("{}.json", uuid.hyphenated())))
    }

    pub(crate) fn load_stats(&self, uuid: uuid::Uuid) -> PlayerStats {
        let Some(path) = self.stats_path(uuid) else { return PlayerStats::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return PlayerStats::default() };
        match PlayerStats::from_json(&text) {
            Ok(mut s) => {
                s.mark_all_dirty();
                s
            }
            Err(e) => {
                tracing::error!("Couldn't parse statistics file {}: {e}", path.display());
                PlayerStats::default()
            }
        }
    }

    pub(crate) fn save_stats(&self, p: &Player) {
        let Some(path) = self.stats_path(p.uuid) else { return };
        let write = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, p.stats.to_json()));
        if let Err(e) = write {
            tracing::error!("Couldn't save stats to {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criterion_names_and_json() {
        let jump = custom("minecraft:jump");
        assert_eq!(jump.criterion_name(), "minecraft.custom:minecraft.jump");
        let stone = Stat::mined(kiln_data::blocks::default_state::STONE);
        assert_eq!(stone.value_name(), Some("minecraft:stone"));
        assert_eq!(stone.criterion_name(), "minecraft.mined:minecraft.stone");
        let mut s = PlayerStats::default();
        s.increment(jump, 3);
        s.increment(stone, 1);
        s.increment(jump, i32::MAX);
        assert_eq!(s.get(jump), i32::MAX);
        let json = s.to_json();
        assert!(json.contains(r#""minecraft:mined":{"minecraft:stone":1}"#), "{json}");
        assert!(json.contains(r#""minecraft:custom":{"minecraft:jump":2147483647}"#), "{json}");
        let back = PlayerStats::from_json(&json).unwrap();
        assert_eq!(back.entries(), s.entries());
        // Vanilla accepts ids without the namespace too.
        let short = PlayerStats::from_json(r#"{"stats":{"custom":{"jump":5,"nonsense":1}},"DataVersion":1}"#).unwrap();
        assert_eq!(short.get(jump), 5);
        assert_eq!(short.entries().len(), 1);
    }

    #[test]
    fn award_packet_lists_dirty_stats_once() {
        let mut s = PlayerStats::default();
        s.increment(custom("minecraft:jump"), 2);
        let first = s.take_award_packet();
        assert!(first.len() > 2);
        let second = s.take_award_packet();
        // Packet id and an empty map.
        assert_eq!(second.len(), 2);
    }
}
