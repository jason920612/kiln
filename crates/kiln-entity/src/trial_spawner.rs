//! Trial spawners (`TrialSpawnerBlockEntity`, `TrialSpawner`, `TrialSpawnerState`,
//! `TrialSpawnerStateData`): the spawner of a trial chamber waits for players, spawns waves of
//! mobs around itself while they are near, and when every mob of the trial is dead ejects loot
//! once for each player it saw, then rests for the cooldown.
//!
//! The block entity's state ([`TrialBe`]) belongs to whoever owns the level's block entities
//! (the simulation keeps it next to the mob spawners); [`tick`] runs one `tickServer` against
//! an abstract [`EntityLevel`]. The block's `trial_spawner_state` and `ominous` properties are
//! read from, and written to, the level.
//!
//! Gaps: the ominous item spawner entity (`OminousItemSpawner`, the items an ominous trial
//! rains down) does not exist yet, and the `equipment` of a spawn data is kept but not rolled.

use crate::level::{EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::{self, Category, MobKind};
use crate::spawner::{self, SpawnData, number};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// The block entity's type.
pub const TYPE: &str = "minecraft:trial_spawner";
/// The block.
pub const BLOCK: &str = "minecraft:trial_spawner";

/// `TrialSpawner.DEFAULT_TARGET_COOLDOWN_LENGTH`.
const DEFAULT_COOLDOWN: i32 = 36000;
/// `TrialSpawner.DEFAULT_PLAYER_SCAN_RANGE`.
const DEFAULT_RANGE: i32 = 14;
/// `TrialSpawner.MAX_MOB_TRACKING_DISTANCE_SQR` (47 blocks).
const MAX_TRACKING_SQR: i64 = 47 * 47;
/// `TrialSpawnerStateData.DELAY_BETWEEN_PLAYER_SCANS`.
const SCAN_EVERY: i64 = 20;
/// `TrialSpawnerState.TIME_BETWEEN_EACH_EJECTION` (`Mth.floor(30.0F)`).
const EJECT_EVERY: f32 = 30.0;
/// `DELAY_BEFORE_EJECT_AFTER_KILLING_LAST_MOB`.
const DELAY_BEFORE_EJECT: f32 = 40.0;
/// `TrialSpawnerStateData.TRIAL_OMEN_PER_BAD_OMEN_LEVEL`.
pub const TRIAL_OMEN_PER_LEVEL: i32 = 18000;

const CONSUMABLES: &str = "minecraft:spawners/trial_chamber/consumables";
const KEY: &str = "minecraft:spawners/trial_chamber/key";
const ITEMS_WHEN_OMINOUS: &str = "minecraft:spawners/trial_chamber/items_to_drop_when_ominous";

/// `TrialSpawnerConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub spawn_range: i32,
    pub total_mobs: f32,
    pub simultaneous_mobs: f32,
    pub total_mobs_added_per_player: f32,
    pub simultaneous_mobs_added_per_player: f32,
    pub ticks_between_spawn: i32,
    pub potentials: Vec<(SpawnData, i32)>,
    pub loot_tables: Vec<(String, i32)>,
    pub items_when_ominous: String,
}

impl Default for Config {
    /// `TrialSpawnerConfig.DEFAULT`.
    fn default() -> Config {
        Config {
            spawn_range: 4,
            total_mobs: 6.0,
            simultaneous_mobs: 2.0,
            total_mobs_added_per_player: 2.0,
            simultaneous_mobs_added_per_player: 1.0,
            ticks_between_spawn: 40,
            potentials: Vec::new(),
            loot_tables: vec![(CONSUMABLES.to_owned(), 1), (KEY.to_owned(), 1)],
            items_when_ominous: ITEMS_WHEN_OMINOUS.to_owned(),
        }
    }
}

fn float(t: Option<&Tag>) -> Option<f32> {
    match t? {
        Tag::Float(v) => Some(*v),
        Tag::Double(v) => Some(*v as f32),
        other => other.as_i64().map(|v| v as f32),
    }
}

fn loot_id(s: &str) -> String {
    spawner::normalize_identifier(s).unwrap_or_else(|| s.to_owned())
}

impl Config {
    /// `TrialSpawnerConfig.DIRECT_CODEC`: `None` when a field is out of range or malformed.
    pub fn parse(t: &Tag) -> Option<Config> {
        let d = Config::default();
        let Tag::Compound(_) = t else { return None };
        let int_in = |name: &str, default: i32, lo: i32, hi: i32| -> Option<i32> {
            match t.get(name) {
                None => Some(default),
                Some(v) => {
                    let n = number(Some(v))? as i32;
                    (lo..=hi).contains(&n).then_some(n)
                }
            }
        };
        let float_in = |name: &str, default: f32| -> Option<f32> {
            match t.get(name) {
                None => Some(default),
                Some(v) => {
                    let n = float(Some(v))?;
                    (n >= 0.0).then_some(n)
                }
            }
        };
        let spawn_range = int_in("spawn_range", d.spawn_range, 1, 128)?;
        let total_mobs = float_in("total_mobs", d.total_mobs)?;
        let simultaneous_mobs = float_in("simultaneous_mobs", d.simultaneous_mobs)?;
        let total_mobs_added_per_player = float_in("total_mobs_added_per_player", d.total_mobs_added_per_player)?;
        let simultaneous_mobs_added_per_player = float_in("simultaneous_mobs_added_per_player", d.simultaneous_mobs_added_per_player)?;
        let ticks_between_spawn = int_in("ticks_between_spawn", d.ticks_between_spawn, 0, i32::MAX)?;
        // `WeightedList.codec`: a list of `{data, weight}` (the weight a positive int, 1 when left out).
        let weighted = |name: &str| -> Option<Option<Vec<(&Tag, i32)>>> {
            let Some(list) = t.get(name) else { return Some(None) };
            let list = list.as_list()?;
            let mut out = Vec::with_capacity(list.len());
            for item in list {
                let item = item.unwrap_list_element();
                let data = item.get("data")?;
                let weight = match item.get("weight") {
                    None => 1,
                    Some(w) => number(Some(w))? as i32,
                };
                if weight < 1 {
                    return None;
                }
                out.push((data, weight));
            }
            Some(Some(out))
        };
        let potentials = match weighted("spawn_potentials")? {
            None => d.potentials,
            Some(list) => list.into_iter().map(|(data, w)| SpawnData::parse(data).map(|s| (s, w))).collect::<Option<Vec<_>>>()?,
        };
        let loot_tables = match weighted("loot_tables_to_eject")? {
            None => d.loot_tables,
            Some(list) => list.into_iter().map(|(data, w)| Some((loot_id(data.as_str()?), w))).collect::<Option<Vec<_>>>()?,
        };
        let items_when_ominous = match t.get("items_to_drop_when_ominous") {
            None => d.items_when_ominous,
            Some(v) => loot_id(v.as_str()?),
        };
        Some(Config {
            spawn_range,
            total_mobs,
            simultaneous_mobs,
            total_mobs_added_per_player,
            simultaneous_mobs_added_per_player,
            ticks_between_spawn,
            potentials,
            loot_tables,
            items_when_ominous,
        })
    }

    /// The saved form (`DIRECT_CODEC`): fields at their default are left out.
    pub fn to_tag(&self) -> Tag {
        let d = Config::default();
        let mut f: Vec<(String, Tag)> = Vec::new();
        if self.spawn_range != d.spawn_range {
            f.push(("spawn_range".into(), Tag::Int(self.spawn_range)));
        }
        let mut put_f = |name: &str, v: f32, dv: f32| {
            if v != dv {
                f.push((name.into(), Tag::Float(v)));
            }
        };
        put_f("total_mobs", self.total_mobs, d.total_mobs);
        put_f("simultaneous_mobs", self.simultaneous_mobs, d.simultaneous_mobs);
        put_f("total_mobs_added_per_player", self.total_mobs_added_per_player, d.total_mobs_added_per_player);
        put_f("simultaneous_mobs_added_per_player", self.simultaneous_mobs_added_per_player, d.simultaneous_mobs_added_per_player);
        if self.ticks_between_spawn != d.ticks_between_spawn {
            f.push(("ticks_between_spawn".into(), Tag::Int(self.ticks_between_spawn)));
        }
        if self.potentials != d.potentials {
            let list = self.potentials.iter().map(|(s, w)| Tag::Compound(vec![("data".into(), s.to_tag()), ("weight".into(), Tag::Int(*w))])).collect();
            f.push(("spawn_potentials".into(), Tag::List(list)));
        }
        if self.loot_tables != d.loot_tables {
            let list = self.loot_tables.iter().map(|(s, w)| Tag::Compound(vec![("data".into(), Tag::String(s.clone())), ("weight".into(), Tag::Int(*w))])).collect();
            f.push(("loot_tables_to_eject".into(), Tag::List(list)));
        }
        if self.items_when_ominous != d.items_when_ominous {
            f.push(("items_to_drop_when_ominous".into(), Tag::String(self.items_when_ominous.clone())));
        }
        Tag::Compound(f)
    }

    /// `calculateTargetTotalMobs`.
    pub fn target_total(&self, additional: i32) -> i32 {
        ((self.total_mobs + self.total_mobs_added_per_player * additional as f32) as f64).floor() as i32
    }

    /// `calculateTargetSimultaneousMobs`.
    pub fn target_simultaneous(&self, additional: i32) -> i32 {
        ((self.simultaneous_mobs + self.simultaneous_mobs_added_per_player * additional as f32) as f64).floor() as i32
    }

    /// `WeightedList.getRandom` over the spawn potentials (one draw when there are any).
    fn pick_potential(&self, r: &mut LegacyRandom) -> Option<&SpawnData> {
        let total: i64 = self.potentials.iter().map(|&(_, w)| w as i64).sum();
        if self.potentials.is_empty() || total <= 0 {
            return None;
        }
        let mut i = r.next_int_bounded(total as i32);
        for (data, w) in &self.potentials {
            i -= w;
            if i < 0 {
                return Some(data);
            }
        }
        None
    }

    /// `WeightedList.getRandom` over the loot tables.
    fn pick_loot(&self, r: &mut LegacyRandom) -> Option<&str> {
        let total: i64 = self.loot_tables.iter().map(|&(_, w)| w as i64).sum();
        if self.loot_tables.is_empty() || total <= 0 {
            return None;
        }
        let mut i = r.next_int_bounded(total as i32);
        for (t, w) in &self.loot_tables {
            i -= w;
            if i < 0 {
                return Some(t);
            }
        }
        None
    }

    /// `withSpawning(type)`: only this entity spawns.
    pub fn with_spawning(&self, entity_type: &str) -> Config {
        let mut c = self.clone();
        let mut data = SpawnData::empty();
        data.set_id(entity_type);
        c.potentials = vec![(data, 1)];
        c
    }
}

/// Where a `normal_config` or `ominous_config` comes from, and how it is saved: not at all
/// (the default), as the key of a config the datapack holds, or as the config itself.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    Default,
    Key(String),
    Inline(Config),
}

impl Slot {
    /// `TrialSpawnerConfig.CODEC` (a holder: a key, or a direct config); a malformed one is `None`.
    fn parse(t: Option<&Tag>) -> Option<Slot> {
        match t {
            None => Some(Slot::Default),
            Some(Tag::String(s)) => Some(Slot::Key(spawner::normalize_identifier(s)?)),
            Some(other) => {
                let c = Config::parse(other)?;
                Some(if c == Config::default() { Slot::Default } else { Slot::Inline(c) })
            }
        }
    }

    fn to_tag(&self) -> Option<Tag> {
        match self {
            Slot::Default => None,
            Slot::Key(k) => Some(Tag::String(k.clone())),
            Slot::Inline(c) if *c == Config::default() => None,
            Slot::Inline(c) => Some(c.to_tag()),
        }
    }
}

/// The state of the block (`trial_spawner_state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Inactive,
    WaitingForPlayers,
    Active,
    WaitingForRewardEjection,
    EjectingReward,
    Cooldown,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Inactive => "inactive",
            State::WaitingForPlayers => "waiting_for_players",
            State::Active => "active",
            State::WaitingForRewardEjection => "waiting_for_reward_ejection",
            State::EjectingReward => "ejecting_reward",
            State::Cooldown => "cooldown",
        }
    }

    fn of(name: Option<&str>) -> State {
        match name {
            Some("waiting_for_players") => State::WaitingForPlayers,
            Some("active") => State::Active,
            Some("waiting_for_reward_ejection") => State::WaitingForRewardEjection,
            Some("ejecting_reward") => State::EjectingReward,
            Some("cooldown") => State::Cooldown,
            _ => State::Inactive,
        }
    }

    fn has_spinning_mob(self) -> bool {
        matches!(self, State::WaitingForPlayers | State::Active)
    }
}

/// The block entity.
#[derive(Clone, Debug)]
pub struct TrialBe {
    pub normal: Slot,
    pub ominous: Slot,
    pub target_cooldown_length: i32,
    pub required_player_range: i32,
    /// `detectedPlayers` (a set; the order is that of detection).
    pub detected: Vec<u128>,
    /// `currentMobs`.
    pub current_mobs: Vec<u128>,
    pub cooldown_ends_at: i64,
    pub next_mob_spawns_at: i64,
    pub total_mobs_spawned: i32,
    pub next_spawn_data: Option<SpawnData>,
    pub ejecting_loot_table: Option<String>,
    /// Saved fields the spawner does not model (`components`, ...).
    pub extra: Vec<(String, Tag)>,
    /// `markUpdated` was called: the clients need the block entity again.
    pub updated: bool,
    /// `setChanged` was called.
    pub changed: bool,
    /// The configs the keys name, once found.
    resolved: [Option<Arc<Config>>; 2],
    /// The display entity of `getOrCreateDisplayEntity` could be made (checked once).
    display_ok: Option<bool>,
}

impl Default for TrialBe {
    fn default() -> TrialBe {
        TrialBe {
            normal: Slot::Default,
            ominous: Slot::Default,
            target_cooldown_length: DEFAULT_COOLDOWN,
            required_player_range: DEFAULT_RANGE,
            detected: Vec::new(),
            current_mobs: Vec::new(),
            cooldown_ends_at: 0,
            next_mob_spawns_at: 0,
            total_mobs_spawned: 0,
            next_spawn_data: None,
            ejecting_loot_table: None,
            extra: Vec::new(),
            updated: false,
            changed: false,
            resolved: [None, None],
            display_ok: None,
        }
    }
}

fn uuids(t: Option<&Tag>) -> Vec<u128> {
    let mut out = Vec::new();
    for u in t.and_then(Tag::as_list).unwrap_or(&[]) {
        if let Some(u) = crate::persist::uuid_from_tag(u.unwrap_list_element())
            && !out.contains(&u)
        {
            out.push(u);
        }
    }
    out
}

impl TrialBe {
    /// `TrialSpawner.load`: the state data, then the config (the default when it cannot be read).
    pub fn load(nbt: &Tag) -> TrialBe {
        let mut be = TrialBe::default();
        // `TrialSpawnerStateData.Packed.MAP_CODEC`: a field that cannot be read takes its default.
        be.detected = uuids(nbt.get("registered_players"));
        be.current_mobs = uuids(nbt.get("current_mobs"));
        be.cooldown_ends_at = number(nbt.get("cooldown_ends_at")).unwrap_or(0);
        be.next_mob_spawns_at = number(nbt.get("next_mob_spawns_at")).unwrap_or(0);
        be.total_mobs_spawned = number(nbt.get("total_mobs_spawned")).map_or(0, |v| v.max(0) as i32);
        be.next_spawn_data = nbt.get("spawn_data").and_then(SpawnData::parse);
        be.ejecting_loot_table = nbt.get("ejecting_loot_table").and_then(Tag::as_str).and_then(spawner::normalize_identifier);
        // `TrialSpawner.FullConfig.MAP_CODEC`: all or nothing.
        let range = number(nbt.get("required_player_range")).map_or(Some(DEFAULT_RANGE), |v| (1..=128).contains(&v).then_some(v as i32));
        let cooldown = number(nbt.get("target_cooldown_length")).map_or(Some(DEFAULT_COOLDOWN), |v| (v >= 0).then_some(v as i32));
        if let (Some(range), Some(cooldown), Some(normal), Some(ominous)) = (range, cooldown, Slot::parse(nbt.get("normal_config")), Slot::parse(nbt.get("ominous_config"))) {
            be.required_player_range = range;
            be.target_cooldown_length = cooldown;
            be.normal = normal;
            be.ominous = ominous;
        }
        if let Tag::Compound(f) = nbt {
            const OWN: [&str; 13] = [
                "registered_players",
                "current_mobs",
                "cooldown_ends_at",
                "next_mob_spawns_at",
                "total_mobs_spawned",
                "spawn_data",
                "ejecting_loot_table",
                "normal_config",
                "ominous_config",
                "target_cooldown_length",
                "required_player_range",
                "id",
                "x",
            ];
            be.extra = f.iter().filter(|(k, _)| !OWN.contains(&k.as_str()) && k != "y" && k != "z").cloned().collect();
        }
        be
    }

    /// `TrialSpawner.store` (the fields of the block entity without its `id` and position).
    pub fn save(&self) -> Vec<(String, Tag)> {
        let mut f = self.extra.clone();
        let uuid_list = |v: &[u128]| Tag::List(v.iter().map(|u| crate::persist::uuid_to_tag(*u)).collect());
        if !self.detected.is_empty() {
            f.push(("registered_players".into(), uuid_list(&self.detected)));
        }
        if !self.current_mobs.is_empty() {
            f.push(("current_mobs".into(), uuid_list(&self.current_mobs)));
        }
        if self.cooldown_ends_at != 0 {
            f.push(("cooldown_ends_at".into(), Tag::Long(self.cooldown_ends_at)));
        }
        if self.next_mob_spawns_at != 0 {
            f.push(("next_mob_spawns_at".into(), Tag::Long(self.next_mob_spawns_at)));
        }
        if self.total_mobs_spawned != 0 {
            f.push(("total_mobs_spawned".into(), Tag::Int(self.total_mobs_spawned)));
        }
        if let Some(d) = &self.next_spawn_data {
            f.push(("spawn_data".into(), d.to_tag()));
        }
        if let Some(t) = &self.ejecting_loot_table {
            f.push(("ejecting_loot_table".into(), Tag::String(t.clone())));
        }
        if let Some(t) = self.normal.to_tag() {
            f.push(("normal_config".into(), t));
        }
        if let Some(t) = self.ominous.to_tag() {
            f.push(("ominous_config".into(), t));
        }
        if self.target_cooldown_length != DEFAULT_COOLDOWN {
            f.push(("target_cooldown_length".into(), Tag::Int(self.target_cooldown_length)));
        }
        if self.required_player_range != DEFAULT_RANGE {
            f.push(("required_player_range".into(), Tag::Int(self.required_player_range)));
        }
        f
    }

    /// Looks up the configs the keys name.
    fn resolve(&mut self, level: &dyn EntityLevel) {
        for (i, slot) in [&self.normal, &self.ominous].into_iter().enumerate() {
            if self.resolved[i].is_none() {
                self.resolved[i] = match slot {
                    Slot::Default => Some(Arc::new(Config::default())),
                    Slot::Inline(c) => Some(Arc::new(c.clone())),
                    // A key the datapack does not hold fails the whole decode in vanilla: the default config.
                    Slot::Key(k) => Some(level.trial_config(k).unwrap_or_else(|| Arc::new(Config::default()))),
                };
            }
        }
    }

    /// `activeConfig`.
    fn active(&self, ominous: bool) -> Arc<Config> {
        self.resolved[usize::from(ominous)].clone().unwrap_or_default()
    }

    fn config_of(&self, ominous: bool) -> Arc<Config> {
        self.active(ominous)
    }

    fn mark_updated(&mut self) {
        self.updated = true;
        self.changed = true;
    }

    /// `overrideEntityToSpawn` (a spawn egg on the spawner): the state data is reset and only this
    /// entity spawns; the state goes back to inactive.
    /// `lookup` finds the configs the datapack holds; the block's state is the caller's to set.
    pub fn override_entity(&mut self, entity_type: &str, lookup: &dyn Fn(&str) -> Option<Arc<Config>>) {
        let config_of = |slot: &Slot| match slot {
            Slot::Default => Config::default(),
            Slot::Inline(c) => c.clone(),
            Slot::Key(k) => lookup(k).map(|c| (*c).clone()).unwrap_or_default(),
        };
        let normal = config_of(&self.normal).with_spawning(entity_type);
        let ominous = config_of(&self.ominous).with_spawning(entity_type);
        self.reset();
        self.normal = Slot::Inline(normal.clone());
        self.ominous = Slot::Inline(ominous.clone());
        self.resolved = [Some(Arc::new(normal)), Some(Arc::new(ominous))];
        self.display_ok = None;
        self.changed = true;
    }

    /// `TrialSpawnerStateData.reset`.
    fn reset(&mut self) {
        self.current_mobs.clear();
        self.next_spawn_data = None;
        self.reset_statistics();
    }

    /// `resetStatistics`.
    fn reset_statistics(&mut self) {
        self.detected.clear();
        self.total_mobs_spawned = 0;
        self.next_mob_spawns_at = 0;
        self.cooldown_ends_at = 0;
    }

    /// `getOrCreateNextSpawnData`.
    fn next_data(&mut self, config: &Config, r: &mut LegacyRandom) -> SpawnData {
        if let Some(d) = &self.next_spawn_data {
            return d.clone();
        }
        let picked = config.pick_potential(r).cloned().unwrap_or_else(SpawnData::empty);
        self.next_spawn_data = Some(picked.clone());
        self.mark_updated();
        picked
    }

    /// `hasMobToSpawn`.
    fn has_mob_to_spawn(&mut self, config: &Config, r: &mut LegacyRandom) -> bool {
        let data = self.next_data(config, r);
        data.id().is_some() || !config.potentials.is_empty()
    }

    /// `getOrCreateDisplayEntity` for a state that spins a mob: whether the entity of the next spawn
    /// data can be made.
    fn display_entity_ok(&mut self, config: &Config, state: State, r: &mut LegacyRandom) -> bool {
        if !state.has_spinning_mob() {
            return false;
        }
        if self.display_ok == Some(true) {
            return true;
        }
        let data = self.next_data(config, r);
        let ok = data.id().is_some() && {
            let mut tag = data.entity.clone();
            tag.retain(|(k, _)| k != "Pos");
            crate::persist::load_stack(&Tag::Compound(tag), 0, &|_| 0, false).is_ok()
        };
        if ok {
            self.display_ok = Some(true);
        }
        ok
    }

    /// `setState`.
    fn set_state(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: State) {
        self.changed = true;
        let s = level.block(pos);
        let new = kiln_data::blocks_types::block_of(s).with_property(s, "trial_spawner_state", state.name()).unwrap_or(s);
        level.set_block(pos, new, 3);
    }

    /// `canSpawnInLevel`.
    fn can_spawn_in_level(&self, level: &dyn EntityLevel) -> bool {
        if !level.spawner_blocks_enabled() {
            return false;
        }
        level.difficulty() != 0 && level.spawn_mobs_rule()
    }

    /// `countAdditionalPlayers`.
    fn additional_players(&self) -> i32 {
        (self.detected.len() as i32 - 1).max(0)
    }

    /// `PlayerDetector.NO_CREATIVE_PLAYERS.detect`: the players within range that are neither
    /// creative nor spectators (in line of sight when asked), by UUID.
    fn detect(&self, level: &dyn EntityLevel, pos: BlockPos, line_of_sight: bool) -> Vec<u128> {
        let center = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
        let range = self.required_player_range as f64;
        level
            .players()
            .iter()
            .filter(|p| {
                let b = BlockPos::containing(p.pos.x, p.pos.y, p.pos.z);
                let d = [(b.x - pos.x) as f64, (b.y - pos.y) as f64, (b.z - pos.z) as f64];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range && !p.creative && !p.spectator
            })
            .filter(|p| !line_of_sight || in_line_of_sight(level, center, Vec3::new(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z)))
            .map(|p| p.uuid)
            .collect()
    }

    /// `tryDetectPlayers`.
    fn try_detect_players(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: State, ominous: bool) {
        if (pos.as_long().wrapping_add(level.game_time())).rem_euclid(SCAN_EVERY) != 0 {
            return;
        }
        if state == State::Cooldown && ominous {
            return;
        }
        let seen = self.detect(level, pos, true);
        let mut omen_found = false;
        if !ominous && !seen.is_empty() {
            // `findPlayerWithOminousEffect`: trial omen first, else the last player with bad omen.
            let mut bad = None;
            let mut trial = None;
            for u in &seen {
                let Some(p) = level.player_by_uuid(*u) else { continue };
                if level.player_has_effect(p.id, "minecraft:trial_omen") {
                    trial = Some(p.id);
                    break;
                }
                if level.player_has_effect(p.id, "minecraft:bad_omen") {
                    bad = Some(p.id);
                }
            }
            let found = trial.map(|id| (id, false)).or(bad.map(|id| (id, true)));
            if let Some((id, from_bad)) = found {
                if from_bad {
                    level.transform_bad_omen(id);
                }
                let eye = level.player(id).map_or(Vec3::new(pos.x as f64, pos.y as f64, pos.z as f64), |p| Vec3::new(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z));
                level.emit(Event::LevelEvent { event: 3020, pos: BlockPos::containing(eye.x, eye.y, eye.z), data: 0 });
                self.apply_ominous(level, pos);
                omen_found = true;
            }
        }
        let ominous = ominous || omen_found;
        let _ = ominous;
        if state == State::Cooldown && !omen_found {
            return;
        }
        let found = if self.detected.is_empty() { seen } else { self.detect(level, pos, false) };
        let mut added = false;
        for u in found {
            if !self.detected.contains(&u) {
                self.detected.push(u);
                added = true;
            }
        }
        if added {
            self.next_mob_spawns_at = (level.game_time() + 40).max(self.next_mob_spawns_at);
            if !omen_found {
                let event = if ominous { 3019 } else { 3013 };
                level.emit(Event::LevelEvent { event, pos, data: self.detected.len() as i32 });
            }
        }
    }

    /// `applyOminous`: the spawner becomes ominous (its mobs go, its counts start over).
    fn apply_ominous(&mut self, level: &mut dyn EntityLevel, pos: BlockPos) {
        let s = level.block(pos);
        let new = kiln_data::blocks_types::block_of(s).with_property(s, "ominous", "true").unwrap_or(s);
        level.set_block(pos, new, 3);
        level.emit(Event::LevelEvent { event: 3020, pos, data: 1 });
        self.reset_after_becoming_ominous(level, pos);
    }

    /// `removeOminous`.
    fn remove_ominous(&mut self, level: &mut dyn EntityLevel, pos: BlockPos) {
        let s = level.block(pos);
        let new = kiln_data::blocks_types::block_of(s).with_property(s, "ominous", "false").unwrap_or(s);
        level.set_block(pos, new, 3);
    }

    /// `TrialSpawnerStateData.resetAfterBecomingOminous`.
    fn reset_after_becoming_ominous(&mut self, level: &mut dyn EntityLevel, pos: BlockPos) {
        for u in std::mem::take(&mut self.current_mobs) {
            level.discard_trial_mob(u);
        }
        self.resolve(level);
        let ominous = self.active(true);
        if !ominous.potentials.is_empty() {
            self.next_spawn_data = None;
        }
        self.total_mobs_spawned = 0;
        self.next_mob_spawns_at = level.game_time() + ominous.ticks_between_spawn as i64;
        self.mark_updated();
        self.cooldown_ends_at = level.game_time() + 160;
        let _ = pos;
    }
}

/// `inLineOfSight(level, a, b)`: the first block on the way from `b` to `a` is the one at `a`,
/// or none is.
fn in_line_of_sight(level: &dyn EntityLevel, a: Vec3, b: Vec3) -> bool {
    let target = BlockPos::containing(a.x, a.y, a.z);
    let hit = crate::clip::traverse_blocks(b, a, |p| {
        let s = level.block(p);
        let (shape, _) = crate::collision::collision_shape(s, p, &crate::collision::CollisionContext::EMPTY);
        crate::clip::shape_clips(&shape, b, a, p).then_some(p)
    });
    hit.is_none_or(|p| p == target)
}

/// A draw of the level's random: vanilla's own stream when replaying it, otherwise a stream
/// seeded by the spawner and the game time.
struct Draw {
    r: LegacyRandom,
    shared: bool,
}

fn draw(level: &mut dyn EntityLevel, pos: BlockPos) -> Draw {
    match level.shared_ai_random() {
        Some(r) => Draw { r: r.clone(), shared: true },
        None => Draw { r: level.pos_random(pos, 0x5452_4c53), shared: false },
    }
}

fn finish(level: &mut dyn EntityLevel, d: Draw) {
    if d.shared
        && let Some(r) = level.shared_ai_random()
    {
        *r = d.r;
    }
}

/// `TrialSpawner.tickServer` for the spawner at `pos`.
pub fn tick(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut TrialBe) {
    let s = level.block(pos);
    if crate::blocks::block_name(s) != BLOCK {
        return;
    }
    be.resolve(level);
    let ominous = kiln_data::blocks_types::block_of(s).property(s, "ominous") == Some("true");
    let state = State::of(kiln_data::blocks_types::block_of(s).property(s, "trial_spawner_state"));
    let mut d = draw(level, pos);
    // `currentMobs.removeIf(shouldMobBeUntracked)`.
    let before = be.current_mobs.len();
    be.current_mobs.retain(|u| {
        level.entity_by_uuid(*u).is_some_and(|e| {
            let b = e.block_position();
            e.is_alive() && {
                let (dx, dy, dz) = ((b.x - pos.x) as i64, (b.y - pos.y) as i64, (b.z - pos.z) as i64);
                dx * dx + dy * dy + dz * dz <= MAX_TRACKING_SQR
            }
        })
    });
    if be.current_mobs.len() != before {
        be.next_mob_spawns_at = level.game_time() + be.active(ominous).ticks_between_spawn as i64;
        be.changed = true;
    }
    let next = tick_and_get_next(level, pos, be, state, ominous, &mut d.r);
    if next != state {
        be.set_state(level, pos, next);
    }
    finish(level, d);
}

/// `TrialSpawnerState.tickAndGetNext`.
fn tick_and_get_next(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut TrialBe, state: State, ominous: bool, r: &mut LegacyRandom) -> State {
    let config = be.active(ominous);
    let now = level.game_time();
    match state {
        State::Inactive => {
            if be.display_entity_ok(&config, State::WaitingForPlayers, r) {
                State::WaitingForPlayers
            } else {
                state
            }
        }
        State::WaitingForPlayers => {
            if !be.can_spawn_in_level(level) {
                be.reset_statistics();
                return state;
            }
            if !be.has_mob_to_spawn(&config, r) {
                return State::Inactive;
            }
            be.try_detect_players(level, pos, state, ominous);
            if be.detected.is_empty() { state } else { State::Active }
        }
        State::Active => {
            if !be.can_spawn_in_level(level) {
                be.reset_statistics();
                return State::WaitingForPlayers;
            }
            if !be.has_mob_to_spawn(&config, r) {
                return State::Inactive;
            }
            let additional = be.additional_players();
            be.try_detect_players(level, pos, state, ominous);
            // (The ominous item spawner of `spawnOminousOminousItemSpawner` is not simulated.)
            if be.total_mobs_spawned >= config.target_total(additional) {
                if be.current_mobs.is_empty() {
                    be.cooldown_ends_at = now + be.target_cooldown_length as i64;
                    be.total_mobs_spawned = 0;
                    be.next_mob_spawns_at = 0;
                    be.changed = true;
                    return State::WaitingForRewardEjection;
                }
            } else if now >= be.next_mob_spawns_at && (be.current_mobs.len() as i32) < config.target_simultaneous(additional)
                && let Some(uuid) = spawn_mob(level, pos, be, &config, ominous, r)
            {
                be.current_mobs.push(uuid);
                be.total_mobs_spawned += 1;
                be.next_mob_spawns_at = now + config.ticks_between_spawn as i64;
                be.changed = true;
                if let Some(next) = config.pick_potential(r) {
                    be.next_spawn_data = Some(next.clone());
                    be.mark_updated();
                }
            }
            state
        }
        State::WaitingForRewardEjection => {
            // `isReadyToOpenShutter(level, 40.0F, targetCooldownLength)`.
            let since = be.cooldown_ends_at - be.target_cooldown_length as i64;
            if now as f32 >= since as f32 + DELAY_BEFORE_EJECT {
                level.emit(Event::Sound { pos: center(pos), sound: "minecraft:block.trial_spawner.open_shutter", source: "block", volume: 1.0, pitch: 1.0 });
                State::EjectingReward
            } else {
                state
            }
        }
        State::EjectingReward => {
            // `isReadyToEjectItems(level, 30.0F, targetCooldownLength)`.
            let since = be.cooldown_ends_at - be.target_cooldown_length as i64;
            if (now - since) as f32 % EJECT_EVERY != 0.0 {
                return state;
            }
            if be.detected.is_empty() {
                level.emit(Event::Sound { pos: center(pos), sound: "minecraft:block.trial_spawner.close_shutter", source: "block", volume: 1.0, pitch: 1.0 });
                be.ejecting_loot_table = None;
                be.changed = true;
                return State::Cooldown;
            }
            if be.ejecting_loot_table.is_none() {
                be.ejecting_loot_table = config.pick_loot(r).map(str::to_owned);
            }
            if let Some(table) = be.ejecting_loot_table.clone() {
                eject_reward(level, pos, &table);
            }
            be.detected.remove(0);
            be.changed = true;
            state
        }
        State::Cooldown => {
            be.try_detect_players(level, pos, state, ominous);
            if !be.detected.is_empty() {
                be.total_mobs_spawned = 0;
                be.next_mob_spawns_at = 0;
                be.changed = true;
                State::Active
            } else if now >= be.cooldown_ends_at {
                be.remove_ominous(level, pos);
                be.reset();
                be.changed = true;
                State::WaitingForPlayers
            } else {
                state
            }
        }
    }
}

fn center(pos: BlockPos) -> Vec3 {
    Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5)
}

/// `TrialSpawner.ejectReward`: the table's items fly out of the top (`3014` when there were any).
fn eject_reward(level: &mut dyn EntityLevel, pos: BlockPos, table: &str) {
    if level.trial_eject(table, pos) {
        level.emit(Event::LevelEvent { event: 3014, pos, data: 0 });
    }
}

/// `TrialSpawner.spawnMob`: the new mob's UUID.
fn spawn_mob(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut TrialBe, config: &Config, ominous: bool, r: &mut LegacyRandom) -> Option<u128> {
    let data = be.next_data(config, r);
    let type_name = data.id()?.to_owned();
    let et = kiln_data::entities::by_name(&type_name)?;
    // `Pos` of the saved entity, else a random place around the spawner.
    let saved_pos = data.entity.iter().find(|(k, _)| k == "Pos").and_then(|(_, v)| match v.as_list()? {
        [x, y, z] => Some((x.as_f64()?, y.as_f64()?, z.as_f64()?)),
        _ => None,
    });
    let (x, y, z) = match saved_pos {
        Some(p) => p,
        None => {
            let range = config.spawn_range as f64;
            let x = pos.x as f64 + (r.next_double() - r.next_double()) * range + 0.5;
            let y = (pos.y + r.next_int_bounded(3) - 1) as f64;
            let z = pos.z as f64 + (r.next_double() - r.next_double()) * range + 0.5;
            (x, y, z)
        }
    };
    // `getSpawnAABB` and `noCollision`.
    let scale = spawner::spawn_dimensions_scale(et.name);
    let half = (scale * et.width / 2.0) as f64;
    let height = (scale * et.height) as f64;
    let spawn_box = Aabb::new(x - half, y, z - half, x + half, y + height, z + half);
    if !crate::collision::no_collision(level, &crate::collision::CollisionContext::EMPTY, i32::MIN, &spawn_box) {
        return None;
    }
    if !in_line_of_sight(level, center(pos), Vec3::new(x, y, z)) {
        return None;
    }
    let at = BlockPos::containing(x, y, z);
    if !spawner::check_spawn_rules_for(&*level, et.name, at, r, true) {
        return None;
    }
    if let Some(rules) = &data.rules {
        let sky = (level.sky_light(at) - level.sky_darken()).max(0);
        if !rules.valid(level.block_light(at), sky) {
            return None;
        }
    }
    // `EntityType.loadEntityRecursive`: the entity and its riders, from the saved form.
    let mut tag = data.entity.clone();
    tag.retain(|(k, _)| k != "Pos");
    tag.push(("Pos".into(), Tag::List(vec![Tag::Double(x), Tag::Double(y), Tag::Double(z)])));
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let (mut e, mut riders) = crate::persist::load_stack(&Tag::Compound(tag), id, &|u| if u != 0 { u as i64 ^ (u >> 64) as i64 } else { seed }, false).ok()?;
    for rd in &mut riders {
        rd.entity.id = level.next_entity_id();
    }
    // `snapTo(x, y, z, random * 360, 0)`.
    let yaw = r.next_float() * 360.0;
    e.set_pos(Vec3::new(x, y, z));
    e.y_rot = yaw;
    e.x_rot = 0.0;
    e.set_old_pos_and_rot();
    // (The mob keeps the UUID the spawner remembers it by.)
    if e.uuid == 0 {
        let hi = (level.fresh_seed() as u64 & !0xF000) | 0x4000;
        let lo = (level.fresh_seed() as u64 & !(0xC000u64 << 48)) | (0x8000u64 << 48);
        e.uuid = ((hi as u128) << 64) | lo as u128;
    }
    let uuid = e.uuid;
    let mut companions = Vec::new();
    let mut nearby_chicken = false;
    if matches!(e.kind, crate::entity::EntityKind::Mob(_)) {
        if !spawner::spawn_obstruction_ok(level, &e) {
            return None;
        }
        if data.entity.len() == 1 {
            let ctx = spawner::spawn_context(level, &e);
            let mut group = mob::GroupData { monsters_disabled: !level.spawning_monsters(), ..Default::default() };
            group.camel_space = false;
            mob::finalize_spawn(&mut e, r, &ctx, &mut group, false);
            companions = std::mem::take(&mut group.companions);
            for c in &mut companions {
                c.entity.id = level.next_entity_id();
            }
            nearby_chicken = group.nearby_chicken;
        }
        // `setPersistenceRequired`.
        if let Some(m) = mob::data_mut(&mut e) {
            m.persistence_required = true;
        }
    }
    let riders: Vec<mob::Companion> = riders
        .into_iter()
        .map(|rd| mob::Companion { entity: rd.entity, seat: if rd.vehicle == 0 { mob::Seat::OnMob } else { mob::Seat::OnCompanion(rd.vehicle - 1) } })
        .chain(companions)
        .collect();
    if let Some(m) = mob::data_mut(&mut e) {
        m.spawn_anim = true;
    }
    mob::kinds::slime::pin_move_yaw_of(&mut e);
    if !level.add_entity_stack(e, riders, data.entity.len() != 1, nearby_chicken) {
        return None;
    }
    // `levelEvent(3011, pos, flame)` at the spawner and `3012` where the mob stands; the
    // `entity_place` game event.
    let flame = i32::from(ominous);
    level.emit(Event::LevelEvent { event: 3011, pos, data: flame });
    level.emit(Event::LevelEvent { event: 3012, pos: at, data: flame });
    level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: Vec3::new(at.x as f64 + 0.5, at.y as f64 + 0.5, at.z as f64 + 0.5), entity: None });
    let _ = (Category::Misc, MobKind::by_name);
    Some(uuid)
}
