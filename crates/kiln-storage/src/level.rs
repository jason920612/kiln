//! `level.dat` and the world clocks (`data/minecraft/world_clocks.dat`, where 26.x keeps the
//! time of day). Both are written back with every field Kiln does not own left as loaded.

use crate::anvil::DATA_VERSION;
use crate::{child, put, read_nbt_file, write_nbt_file};
use kiln_proto::nbt::Tag;
use std::path::{Path, PathBuf};
use tracing::warn;

const OVERWORLD_CLOCK: &str = "minecraft:overworld";
/// `Difficulty` names by id (`Data.difficulty_settings.difficulty`).
const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

/// `world_gen_settings.dat` of a default world with this seed: the vanilla generators of the
/// three dimensions.
fn world_gen_settings(seed: i64, generate_structures: bool) -> Tag {
    let s = |v: &str| Tag::String(v.to_owned());
    let compound = |fields: Vec<(&str, Tag)>| Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect());
    let multi_noise = |preset: &str| compound(vec![("preset", s(preset)), ("type", s("minecraft:multi_noise"))]);
    let dimension = |key: &str, settings: &str, biome_source: Tag| {
        (
            format!("minecraft:{key}"),
            compound(vec![
                (
                    "generator",
                    compound(vec![
                        ("settings", s(&format!("minecraft:{settings}"))),
                        ("biome_source", biome_source),
                        ("type", s("minecraft:noise")),
                    ]),
                ),
                ("type", s(&format!("minecraft:{key}"))),
            ]),
        )
    };
    let dimensions = Tag::Compound(vec![
        dimension("overworld", "overworld", multi_noise("minecraft:overworld")),
        dimension("the_nether", "nether", multi_noise("minecraft:nether")),
        dimension("the_end", "end", compound(vec![("type", s("minecraft:the_end"))])),
    ]);
    let data = compound(vec![
        ("bonus_chest", Tag::Byte(0)),
        ("seed", Tag::Long(seed)),
        ("generate_structures", Tag::Byte(i8::from(generate_structures))),
        ("dimensions", dimensions),
    ]);
    compound(vec![("data", data), ("DataVersion", Tag::Int(DATA_VERSION as i32))])
}
/// This build's brand in `ServerBrands`.
const BRAND: &str = "kiln";

/// World spawn (`Data.spawn`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSpawn {
    pub dimension: String,
    pub pos: [i32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// What a world clock keeps in `world_clocks.dat` besides its name (`ClockState`): total ticks, the fraction of a tick the rate left
/// over, the rate and whether it is paused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockSave {
    pub total: i64,
    pub partial: f32,
    pub rate: f32,
    pub paused: bool,
}

/// The level state Kiln owns.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelState {
    /// World age in ticks (`Data.Time`).
    pub game_time: i64,
    /// Total ticks of the overworld clock: the time of day.
    pub day_time: i64,
    pub spawn: WorldSpawn,
    /// `Data.DataPacks`: enabled packs in load order and disabled ones; `None` keeps the
    /// saved lists.
    pub data_packs: Option<(Vec<String>, Vec<String>)>,
    /// `Data.enabled_features`: the world's feature flags; `None` keeps the saved list.
    pub enabled_features: Option<Vec<String>>,
    /// `Data.difficulty_settings`: the difficulty (0 peaceful .. 3 hard) and whether it is
    /// locked; `None` keeps the saved settings (and leaves a new world's to vanilla's default).
    pub difficulty: Option<(u8, bool)>,
    /// The clocks Kiln runs besides the overworld's (`day_time`), by `minecraft:world_clock` id: written as they are. A clock of the
    /// file that is not among them is advanced by the ticks since the last save, as vanilla's would have been.
    pub clocks: Vec<(String, ClockSave)>,
}

/// A saved game rule: booleans are bytes in `game_rules.dat`, integers ints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedRule {
    Bool(bool),
    Int(i32),
}

pub struct LevelStore {
    dir: PathBuf,
    /// `level.dat` as loaded, if the world has one.
    level: Option<Tag>,
    /// `world_clocks.dat` as loaded.
    clocks: Option<Tag>,
    /// `game_rules.dat` as loaded or last written.
    game_rules: Option<Tag>,
    /// `world_gen_settings.dat` as loaded (the seed and the dimensions' generators).
    world_gen: Option<Tag>,
    /// The seed a world without `world_gen_settings.dat` is given with the next save.
    new_seed: Option<i64>,
    /// `generate-structures` for a world that gets its `world_gen_settings.dat` now.
    new_generate_structures: bool,
    /// `game_rules.dat` needs writing.
    rules_dirty: bool,
    /// Game time when the clocks were last read or written, to advance the clocks Kiln does
    /// not run.
    clocks_game_time: i64,
}

impl LevelStore {
    /// Reads the world's level data; `level.dat_old` stands in for an unreadable `level.dat`
    /// as in vanilla.
    pub fn open(world_dir: &Path) -> Self {
        let level = ["level.dat", "level.dat_old"].iter().find_map(|name| {
            let path = world_dir.join(name);
            if !path.exists() {
                return None;
            }
            read_nbt_file(&path).map_err(|e| warn!("cannot read {}: {e}", path.display())).ok()
        });
        let saved_data = |name: &str| {
            let path = world_dir.join("data/minecraft").join(name);
            path.exists().then(|| read_nbt_file(&path)).and_then(|r| r.map_err(|e| warn!("cannot read {}: {e}", path.display())).ok())
        };
        let (clocks, game_rules) = (saved_data("world_clocks.dat"), saved_data("game_rules.dat"));
        let world_gen = saved_data("world_gen_settings.dat");
        let mut store = Self {
            dir: world_dir.to_owned(),
            level,
            clocks,
            game_rules,
            world_gen,
            new_seed: None,
            new_generate_structures: true,
            rules_dirty: false,
            clocks_game_time: 0,
        };
        store.clocks_game_time = store.state().game_time;
        store
    }

    /// The clocks of `world_clocks.dat` other than the overworld's, as saved.
    pub fn clocks(&self) -> Vec<(String, ClockSave)> {
        let Some(Tag::Compound(entries)) = self.clocks.as_ref().and_then(|c| c.get("data")) else { return Vec::new() };
        entries
            .iter()
            .filter(|(name, _)| name != OVERWORLD_CLOCK)
            .map(|(name, c)| {
                let float = |key: &str, default: f32| match c.get(key) {
                    Some(Tag::Float(v)) => *v,
                    _ => default,
                };
                let paused = c.get("paused").and_then(Tag::as_i64) == Some(1);
                (name.clone(), ClockSave { total: c.get("total_ticks").and_then(Tag::as_i64).unwrap_or(0), partial: float("partial_tick", 0.0), rate: float("rate", 1.0), paused })
            })
            .collect()
    }

    /// Whether the world has a `level.dat`.
    pub fn exists(&self) -> bool {
        self.level.is_some()
    }

    fn data(&self) -> Option<&Tag> {
        self.level.as_ref()?.get("Data")
    }

    /// The saved state, with vanilla's defaults for missing fields.
    pub fn state(&self) -> LevelState {
        let data = self.data();
        let spawn = data.and_then(|d| d.get("spawn"));
        let pos = match spawn.and_then(|s| s.get("pos")) {
            Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
            _ => [0, 0, 0],
        };
        let float = |key| match spawn.and_then(|s| s.get(key)) {
            Some(Tag::Float(v)) => *v,
            _ => 0.0,
        };
        let clock = self.clocks.as_ref().and_then(|c| c.get("data")?.get(OVERWORLD_CLOCK));
        LevelState {
            game_time: data.and_then(|d| d.get("Time")?.as_i64()).unwrap_or(0),
            day_time: clock.and_then(|c| c.get("total_ticks")?.as_i64()).unwrap_or(0),
            spawn: WorldSpawn {
                dimension: spawn
                    .and_then(|s| s.get("dimension")?.as_str())
                    .unwrap_or("minecraft:overworld")
                    .to_owned(),
                pos,
                yaw: float("yaw"),
                pitch: float("pitch"),
            },
            data_packs: self.data_packs(),
            enabled_features: self.enabled_features(),
            difficulty: self.difficulty(),
            clocks: Vec::new(),
        }
    }

    /// `Data.difficulty_settings`: the difficulty (0..=3) and the lock, if saved.
    pub fn difficulty(&self) -> Option<(u8, bool)> {
        let settings = self.data()?.get("difficulty_settings")?;
        let name = settings.get("difficulty")?.as_str()?;
        let id = DIFFICULTIES.iter().position(|d| *d == name)?;
        Some((id as u8, settings.get("locked").and_then(Tag::as_i64) == Some(1)))
    }

    /// `Data.difficulty_settings.hardcore`.
    pub fn hardcore(&self) -> bool {
        self.data().and_then(|d| d.get("difficulty_settings")?.get("hardcore")?.as_i64()) == Some(1)
    }

    /// Marks the world hardcore (`LevelSettings`' `hardcore` of a new world), written with the next save.
    pub fn set_hardcore(&mut self, hardcore: bool) {
        let root = self.level.get_or_insert_with(|| Tag::Compound(Vec::new()));
        put(child(child(root, "Data"), "difficulty_settings"), "hardcore", Tag::Byte(i8::from(hardcore)));
    }

    /// The world seed (`world_gen_settings.dat`), if the world has one.
    pub fn seed(&self) -> Option<i64> {
        self.world_gen.as_ref()?.get("data")?.get("seed")?.as_i64()
    }

    /// The level stems the save lists (`world_gen_settings.dat`, `dimensions`): dimension key and
    /// stem (`type` and `generator`).
    pub fn dimensions(&self) -> Vec<(String, Tag)> {
        match self.world_gen.as_ref().and_then(|w| w.get("data")?.get("dimensions")) {
            Some(Tag::Compound(fields)) => fields.clone(),
            _ => Vec::new(),
        }
    }

    /// Whether the world generates structures (`world_gen_settings.dat`), if it has settings.
    pub fn generate_structures(&self) -> Option<bool> {
        self.world_gen.as_ref()?.get("data")?.get("generate_structures")?.as_i64().map(|v| v != 0)
    }

    /// `generate-structures` for a world that has no settings yet (one that has them keeps them).
    pub fn set_generate_structures_if_missing(&mut self, generate: bool) {
        if self.world_gen.is_none() {
            self.new_generate_structures = generate;
        }
    }

    /// Gives a world that has no `world_gen_settings.dat` this seed with the next save (a seed
    /// the world has is never replaced).
    pub fn set_seed_if_missing(&mut self, seed: i64) {
        if self.world_gen.is_none() {
            self.new_seed = Some(seed);
        }
    }

    /// The saved game rules (`game_rules.dat`), by their ids.
    pub fn game_rules(&self) -> Vec<(String, SavedRule)> {
        let Some(Tag::Compound(rules)) = self.game_rules.as_ref().and_then(|g| g.get("data")) else { return Vec::new() };
        rules
            .iter()
            .filter_map(|(name, value)| match value {
                Tag::Byte(b) => Some((name.clone(), SavedRule::Bool(*b != 0))),
                other => other.as_i64().map(|v| (name.clone(), SavedRule::Int(v as i32))),
            })
            .collect()
    }

    /// Sets the game rules written with the next save (vanilla writes every rule; saved rules not
    /// listed stay as they are). Nothing is written when they are what the file already has.
    pub fn set_game_rules(&mut self, rules: &[(String, SavedRule)]) {
        let root = self.game_rules.get_or_insert_with(|| Tag::Compound(Vec::new()));
        let before = root.get("data").cloned();
        let data = child(root, "data");
        for (name, rule) in rules {
            let tag = match rule {
                SavedRule::Bool(b) => Tag::Byte(i8::from(*b)),
                SavedRule::Int(v) => Tag::Int(*v),
            };
            put(data, name, tag);
        }
        if before.as_ref() != root.get("data") || !self.dir.join("data/minecraft/game_rules.dat").exists() {
            self.rules_dirty = true;
        }
    }

    /// `Data.enabled_features`, if saved.
    pub fn enabled_features(&self) -> Option<Vec<String>> {
        let list = self.data()?.get("enabled_features")?.as_list()?;
        Some(list.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect())
    }

    /// `Data.DataPacks` (`Enabled`, `Disabled`), if saved.
    pub fn data_packs(&self) -> Option<(Vec<String>, Vec<String>)> {
        let packs = self.data()?.get("DataPacks")?;
        let list = |k: &str| {
            packs.get(k).and_then(Tag::as_list).unwrap_or(&[]).iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()
        };
        Some((list("Enabled"), list("Disabled")))
    }

    /// The default game mode for new players (`Data.GameType`), if the world has one.
    pub fn game_type(&self) -> Option<u8> {
        self.data()?.get("GameType")?.as_i64().map(|g| g as u8)
    }

    /// Sets `Data.GameType` (`/defaultgamemode`), written with the next save.
    pub fn set_game_type(&mut self, game_type: u8) {
        let root = self.level.get_or_insert_with(|| Tag::Compound(Vec::new()));
        put(child(root, "Data"), "GameType", Tag::Int(i32::from(game_type)));
    }

    /// A saved game rule (`minecraft:respawn_radius`, ...) as a number (booleans are 0 or 1).
    pub fn game_rule(&self, name: &str) -> Option<i64> {
        self.game_rules.as_ref()?.get("data")?.get(name)?.as_i64()
    }

    /// Writes `level.dat` (keeping the previous one as `level.dat_old`) and the world clocks.
    pub fn save(&mut self, state: &LevelState) -> std::io::Result<()> {
        let mut root = self.level.clone().unwrap_or_else(|| Tag::Compound(Vec::new()));
        update_level(child(&mut root, "Data"), state);
        write_nbt_file(&self.dir.join("level.dat"), &root, Some(&self.dir.join("level.dat_old")))?;
        self.level = Some(root);

        if self.rules_dirty && let Some(rules) = &mut self.game_rules {
            put(rules, "DataVersion", Tag::Int(DATA_VERSION as i32));
            write_nbt_file(&self.dir.join("data/minecraft/game_rules.dat"), rules, None)?;
            self.rules_dirty = false;
        }
        if self.world_gen.is_none() && let Some(seed) = self.new_seed.take() {
            let settings = world_gen_settings(seed, self.new_generate_structures);
            write_nbt_file(&self.dir.join("data/minecraft/world_gen_settings.dat"), &settings, None)?;
            self.world_gen = Some(settings);
        }

        let mut clocks = self.clocks.clone().unwrap_or_else(|| Tag::Compound(Vec::new()));
        let elapsed = state.game_time - self.clocks_game_time;
        update_clocks(child(&mut clocks, "data"), state.day_time, elapsed, &state.clocks);
        put(&mut clocks, "DataVersion", Tag::Int(DATA_VERSION as i32));
        write_nbt_file(&self.dir.join("data/minecraft/world_clocks.dat"), &clocks, None)?;
        self.clocks = Some(clocks);
        self.clocks_game_time = state.game_time;
        Ok(())
    }
}

/// Overwrites the `Data` fields Kiln owns, as vanilla `PrimaryLevelData.setTagData` writes them.
fn update_level(data: &mut Tag, state: &LevelState) {
    put(data, "Time", Tag::Long(state.game_time));
    if let Some((difficulty, locked)) = state.difficulty {
        let settings = child(data, "difficulty_settings");
        put(settings, "difficulty", Tag::String(DIFFICULTIES[usize::from(difficulty.min(3))].to_owned()));
        if settings.get("hardcore").is_none() {
            put(settings, "hardcore", Tag::Byte(0));
        }
        put(settings, "locked", Tag::Byte(i8::from(locked)));
    }
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    put(data, "LastPlayed", Tag::Long(now_ms));
    let spawn = child(data, "spawn");
    put(spawn, "dimension", Tag::String(state.spawn.dimension.clone()));
    put(spawn, "pos", Tag::IntArray(state.spawn.pos.to_vec()));
    put(spawn, "yaw", Tag::Float(state.spawn.yaw));
    put(spawn, "pitch", Tag::Float(state.spawn.pitch));

    if let Some((enabled, disabled)) = &state.data_packs {
        let strings = |v: &[String]| Tag::List(v.iter().cloned().map(Tag::String).collect());
        let packs = child(data, "DataPacks");
        put(packs, "Enabled", strings(enabled));
        put(packs, "Disabled", strings(disabled));
    }
    if let Some(features) = &state.enabled_features {
        put(data, "enabled_features", Tag::List(features.iter().cloned().map(Tag::String).collect()));
    }
    put(data, "DataVersion", Tag::Int(DATA_VERSION as i32));
    let version = child(data, "Version");
    put(version, "Id", Tag::Int(DATA_VERSION as i32));
    put(version, "Name", Tag::String(kiln_data::version::NAME.to_owned()));
    put(version, "Series", Tag::String("main".to_owned()));
    put(version, "Snapshot", Tag::Byte(0));
    let mut history = data.get("version_history").and_then(Tag::as_list).map(<[Tag]>::to_vec).unwrap_or_default();
    if history.last().and_then(Tag::as_i64) != Some(DATA_VERSION) {
        history.push(Tag::Int(DATA_VERSION as i32));
    }
    put(data, "version_history", Tag::List(history));
    // Vanilla records every server brand that saved the world and flags non-vanilla ones.
    let mut brands = data.get("ServerBrands").and_then(Tag::as_list).map(<[Tag]>::to_vec).unwrap_or_default();
    if !brands.iter().any(|b| b.as_str() == Some(BRAND)) {
        brands.push(Tag::String(BRAND.to_owned()));
    }
    put(data, "ServerBrands", Tag::List(brands));
    put(data, "WasModded", Tag::Byte(1));
}

/// Sets the overworld clock and the clocks Kiln runs (`running`), and advances the others by the ticks since the last save, as
/// vanilla's `ServerClockInstance.tick` would have.
fn update_clocks(clocks: &mut Tag, day_time: i64, elapsed: i64, running: &[(String, ClockSave)]) {
    let Tag::Compound(entries) = clocks else { return };
    for (name, clock) in entries.iter_mut() {
        if name == OVERWORLD_CLOCK || running.iter().any(|(n, _)| n == name) || elapsed <= 0 || clock.get("paused").and_then(Tag::as_i64) == Some(1) {
            continue;
        }
        let rate = match clock.get("rate") {
            Some(Tag::Float(r)) => *r as f64,
            _ => 1.0,
        };
        let partial = match clock.get("partial_tick") {
            Some(Tag::Float(p)) => *p as f64,
            _ => 0.0,
        };
        let advanced = partial + rate * elapsed as f64;
        let whole = advanced.floor();
        let total = clock.get("total_ticks").and_then(Tag::as_i64).unwrap_or(0);
        put(clock, "total_ticks", Tag::Long(total + whole as i64));
        if clock.get("partial_tick").is_some() || advanced != whole {
            put(clock, "partial_tick", Tag::Float((advanced - whole) as f32));
        }
    }
    put(child(clocks, OVERWORLD_CLOCK), "total_ticks", Tag::Long(day_time));
    for (name, run) in running {
        let clock = child(clocks, name);
        put(clock, "total_ticks", Tag::Long(run.total));
        if let Tag::Compound(fields) = clock {
            fields.retain(|(k, _)| !matches!(k.as_str(), "partial_tick" | "rate" | "paused"));
        }
        if run.partial != 0.0 {
            put(clock, "partial_tick", Tag::Float(run.partial));
        }
        if run.rate != 1.0 {
            put(clock, "rate", Tag::Float(run.rate));
        }
        if run.paused {
            put(clock, "paused", Tag::Byte(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kiln-level-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn new_world_gets_minimal_level_data() {
        let dir = scratch("new");
        let mut store = LevelStore::open(&dir);
        assert!(!store.exists());
        let state = LevelState {
            game_time: 1200,
            day_time: 7000,
            spawn: WorldSpawn { dimension: "minecraft:overworld".into(), pos: [8, 64, 8], yaw: 90.0, pitch: 0.0 },
            data_packs: Some((vec!["vanilla".into(), "file/p".into()], vec!["trade_rebalance".into()])),
            enabled_features: Some(vec!["minecraft:vanilla".into(), "minecraft:minecart_improvements".into()]),
            difficulty: Some((3, true)),
            clocks: Vec::new(),
        };
        store.save(&state).unwrap();
        let back = LevelStore::open(&dir);
        assert!(back.exists());
        assert_eq!(back.state(), state);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_unowned_fields_and_advances_other_clocks() {
        let dir = scratch("keep");
        let level = Tag::Compound(vec![(
            "Data".into(),
            Tag::Compound(vec![
                ("Time".into(), Tag::Long(100)),
                ("LevelName".into(), Tag::String("world".into())),
                ("GameType".into(), Tag::Int(0)),
                ("ServerBrands".into(), Tag::List(vec![Tag::String("vanilla".into())])),
                ("version_history".into(), Tag::List(vec![Tag::Int(5023)])),
            ]),
        )]);
        write_nbt_file(&dir.join("level.dat"), &level, None).unwrap();
        let clock = |t| Tag::Compound(vec![("total_ticks".into(), Tag::Long(t))]);
        let clocks = Tag::Compound(vec![(
            "data".into(),
            Tag::Compound(vec![("minecraft:overworld".into(), clock(100)), ("minecraft:the_end".into(), clock(100))]),
        )]);
        write_nbt_file(&dir.join("data/minecraft/world_clocks.dat"), &clocks, None).unwrap();

        let mut store = LevelStore::open(&dir);
        assert_eq!(store.game_type(), Some(0));
        let mut state = store.state();
        assert_eq!((state.game_time, state.day_time), (100, 100));
        state.game_time = 150;
        state.day_time = 6000;
        store.save(&state).unwrap();

        let root = read_nbt_file(&dir.join("level.dat")).unwrap();
        let data = root.get("Data").unwrap();
        assert_eq!(data.get("LevelName").and_then(Tag::as_str), Some("world"));
        assert_eq!(data.get("Time").and_then(Tag::as_i64), Some(150));
        let brands: Vec<_> = data.get("ServerBrands").and_then(Tag::as_list).unwrap().iter().filter_map(Tag::as_str).collect();
        assert_eq!(brands, ["vanilla", "kiln"]);
        assert_eq!(data.get("version_history").and_then(Tag::as_list).unwrap().len(), 1);
        assert!(dir.join("level.dat_old").exists());
        let clocks = read_nbt_file(&dir.join("data/minecraft/world_clocks.dat")).unwrap();
        let ticks = |c: &str| clocks.get("data")?.get(c)?.get("total_ticks")?.as_i64();
        assert_eq!((ticks("minecraft:overworld"), ticks("minecraft:the_end")), (Some(6000), Some(150)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    /// wp86: `generate-structures=false` is written into a new world's settings and read back; a world keeps what it has.
    #[test]
    fn generate_structures_is_part_of_the_new_world_and_stays() {
        let dir = scratch("genstruct");
        let mut store = LevelStore::open(&dir);
        store.set_seed_if_missing(5);
        store.set_generate_structures_if_missing(false);
        let state = store.state();
        store.save(&state).unwrap();
        let mut again = LevelStore::open(&dir);
        assert_eq!(again.generate_structures(), Some(false));
        // Asking for true now changes nothing: the world has its settings.
        again.set_generate_structures_if_missing(true);
        let state = again.state();
        again.save(&state).unwrap();
        assert_eq!(LevelStore::open(&dir).generate_structures(), Some(false));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
