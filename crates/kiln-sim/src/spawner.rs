//! Natural mob spawning (`NaturalSpawner`), per region.
//!
//! Vanilla's rules, with two regional adaptations so the outcome does not depend on how
//! regions split the world:
//!
//! - **Caps.** Vanilla caps each category at `max × spawnable chunks / 289` over the whole
//!   level, where spawnable chunks are those within 8 chunks of a player, plus a per-player
//!   local cap (`LocalMobCapCalculator`). Regions are farther apart than a player's spawning
//!   range, so each region applies the same formula to its own players' chunks and mobs: the
//!   sum over regions is vanilla's global cap, and the local caps are exact.
//! - **Randomness.** Vanilla shuffles the spawning chunks and draws everything from the
//!   level's random. Here every chunk draws from its own random, seeded by the world seed, the
//!   tick and the chunk (like random ticks, see [`crate::blocks`]), and chunks go in an order
//!   hashed from the same inputs.
//!
//! Spawn potentials (`spawn_costs`: soul sand valleys, warped forests; `NaturalSpawner.SpawnState.canSpawn`/`afterSpawn`,
//! `PotentialCalculator`) are simulated per region; biomes are read at their
//! stored 4×4×4 cells (vanilla fuzzes the lookup). Structure spawn overrides are
//! ([`crate::structure_spawns`]): inside a fortress, a swamp hut, a monument... the structure's
//! mobs replace the biome's.

use crate::Player;
use crate::blocks::{RegionLevel, Ticking};
use crate::entities::{Entities, Spawn};
use kiln_blocks::{BlockPos as KBlockPos, Level};
use kiln_entity::mob::{self, Category, MobKind};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_world::{Blocks, CellStore, ChunkPos};

/// `MobSpawnSettings.SpawnerData`: a type, its weight and its group size.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpawnerData {
    /// `None`: a type Kiln does not simulate (the group is skipped when picked).
    pub kind: Option<MobKind>,
    /// The entity type's name (empty: not a known type).
    pub type_name: &'static str,
    pub weight: i32,
    pub min: i32,
    pub max: i32,
    /// A constant count (no random draw).
    pub constant: bool,
}

/// Each biome's `minecraft:gameplay/natural_mob_spawns` for the spawning categories
/// ([`Category::SPAWNING`] order), by biome network id.
#[derive(Debug, Default)]
pub(crate) struct SpawnTable {
    /// By biome network id (looked up for every spawn attempt).
    biomes: Vec<Option<[Vec<SpawnerData>; N]>>,
    /// By biome network id: `minecraft:gameplay/creature_world_gen_spawn_probability` (chunk generation's animals).
    world_gen: Vec<f32>,
    /// By biome network id: the biome's `spawn_costs` (`MobSpawnSettings.MobSpawnCost`): entity type name, charge, energy budget.
    costs: Vec<Vec<(&'static str, f64, f64)>>,
    /// The structures' `spawn_overrides`.
    pub structures: crate::structure_spawns::StructureSpawns,
}

/// The number of spawning categories.
const N: usize = Category::SPAWNING.len();

impl SpawnTable {
    /// Reads `worldgen/biome/*.json` of the datapack at `dir`.
    pub fn load(dir: &std::path::Path) -> Option<SpawnTable> {
        let biome_dir = dir.join("data/minecraft/worldgen/biome");
        let mut t = SpawnTable { structures: crate::structure_spawns::StructureSpawns::load(dir), ..SpawnTable::default() };
        for entry in std::fs::read_dir(&biome_dir).ok()? {
            let path = entry.ok()?.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Some(id) = kiln_data::synced_id("minecraft:worldgen/biome", &format!("minecraft:{name}")) else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let spawns = &json["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"]["spawns_by_category"];
            let list = |cat: &str| -> Vec<SpawnerData> { parse_spawner_list(&spawns[cat]) };
            let id = id as usize;
            if t.biomes.len() <= id {
                t.biomes.resize_with(id + 1, || None);
            }
            t.biomes[id] = Some(Category::SPAWNING.map(|c| list(c.name())));
            if t.world_gen.len() <= id {
                t.world_gen.resize(id + 1, 0.1);
            }
            if t.costs.len() <= id {
                t.costs.resize_with(id + 1, Vec::new);
            }
            if let Some(o) = json["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"]["spawn_costs"].as_object() {
                for (name, v) in o {
                    if let (Some(ty), Some(charge), Some(budget)) = (kiln_data::entities::by_name(name), v["charge"].as_f64(), v["energy_budget"].as_f64()) {
                        t.costs[id].push((ty.name, charge, budget));
                    }
                }
            }
            if let Some(p) = json["attributes"]["minecraft:gameplay/creature_world_gen_spawn_probability"].as_f64() {
                t.world_gen[id] = p as f32;
            }
        }
        Some(t)
    }

    /// `MobSpawnSettings.getMobSpawnCost`: the charge and energy budget of `type_name` in `biome`, if the biome has a cost for it.
    pub(crate) fn cost(&self, biome: u16, type_name: &str) -> Option<(f64, f64)> {
        self.costs.get(biome as usize)?.iter().find(|c| c.0 == type_name).map(|c| (c.1, c.2))
    }

    /// `creature_world_gen_spawn_probability` of a biome (0.1 unless the biome says).
    pub(crate) fn world_gen_probability(&self, biome: u16) -> f32 {
        self.world_gen.get(biome as usize).copied().unwrap_or(0.1)
    }

    pub(crate) fn list(&self, biome: u16, category: Category) -> &[SpawnerData] {
        // (`MISC` has no spawning list: the natural spawner never asks for it.)
        let Some(i) = CATEGORIES.iter().position(|&x| x == category) else { return &[] };
        self.biomes.get(biome as usize).and_then(Option::as_ref).map_or(&[], |b| &b[i])
    }
}

/// A list of `MobSpawnSettings.SpawnerData` as JSON gives it (`type`, `weight`, `count`).
pub(crate) fn parse_spawner_list(v: &serde_json::Value) -> Vec<SpawnerData> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let name = s["type"].as_str()?;
                    let kind = MobKind::by_name(name);
                    let type_name = kiln_data::entities::by_name(name).map_or("", |t| t.name);
                    let weight = s["weight"].as_i64()? as i32;
                    let (min, max, constant) = match &s["count"] {
                        serde_json::Value::Number(n) => (n.as_i64()? as i32, n.as_i64()? as i32, true),
                        c => (c["min_inclusive"].as_i64()? as i32, c["max_inclusive"].as_i64()? as i32, false),
                    };
                    Some(SpawnerData { kind, type_name, weight, min, max, constant })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl SpawnTable {
    /// `NaturalSpawner.mobsAt` for `category` at block `pos` of `biome`: the fortress's list above
    /// nether bricks inside a fortress, else the list of a structure the position is in
    /// (`ChunkGenerator.getMobsAt`), else the biome's.
    pub(crate) fn mobs_at<'a>(&'a self, level: &RegionLevel, biome: u16, category: Category, pos: KBlockPos) -> &'a [SpawnerData] {
        let structures = crate::structure_spawns::ChunkStructures(|c: ChunkPos| level.cells.chunk(c));
        let bricks = || kiln_data::blocks_types::block_of(level.block(pos.below())).name == "minecraft:nether_bricks";
        self.mobs_in(&structures, bricks, biome, category, [pos.x, pos.y, pos.z])
    }

    /// [`SpawnTable::mobs_at`] over the loaded chunks' `structures` data.
    pub(crate) fn mobs_in<'a, 'c, S>(&'a self, structures: &S, nether_bricks_below: impl FnOnce() -> bool, biome: u16, category: Category, at: [i32; 3]) -> &'a [SpawnerData]
    where
        S: crate::structure_spawns::Structures<'c> + ?Sized,
    {
        if self.structures.is_empty() {
            return self.list(biome, category);
        }
        if category == Category::Monster && nether_bricks_below() && self.structures.in_fortress(structures, at) {
            return crate::structure_spawns::fortress_enemies();
        }
        self.structures.mobs_at(structures, at, category.name()).unwrap_or_else(|| self.list(biome, category))
    }
}

/// `WeightedList.getRandom`.
fn pick<'a>(list: &'a [SpawnerData], r: &mut (impl RandomSource + ?Sized)) -> Option<&'a SpawnerData> {
    let total: i32 = list.iter().map(|s| s.weight).sum();
    if total <= 0 {
        return None;
    }
    let mut i = r.next_int_bounded(total);
    for s in list {
        i -= s.weight;
        if i < 0 {
            return Some(s);
        }
    }
    None
}

/// `NaturalSpawner.SPAWNING_CATEGORIES`.
const CATEGORIES: [Category; N] = Category::SPAWNING;

fn chunk_random(seed: i64, game_time: i64, c: ChunkPos) -> LegacyRandom {
    let mut h = (seed as u64 ^ 0x7370_6177_6e21) ^ (game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (c.x as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ ((c.z as u32 as u64) << 32).wrapping_mul(0x1656_67B1_9E37_79F9);
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    LegacyRandom::new(h as i64)
}

/// The spawner's view of a region.
struct Spawner<'a> {
    pos: Vec<[f64; 3]>,
    /// Per player, mobs per category nearby (`LocalMobCapCalculator.MobCounts`), for the
    /// categories counted (`known`; see [`Spawner::local_ok`]).
    local: Vec<[i32; N]>,
    /// The counted mobs: chunk and category index.
    mobs: Vec<(ChunkPos, usize)>,
    /// Per stand chunk, mobs per category in the chunks at most 8 away (chessboard): every
    /// mob close to one of its players is among them, so this bounds their local counts.
    upper: Vec<[i32; N]>,
    /// Categories whose local counts are made (all players at once).
    known: [bool; N],
    /// Per counted category and stand chunk, the players that had room when last looked at.
    room: [Vec<Vec<usize>>; N],
    table: &'a SpawnTable,
    /// The `PotentialCalculator`'s point charges: the block and charge of every counted mob whose biome has a spawn cost for its type
    /// (made at the start of the tick, and a spawn adds its own).
    charges: Vec<([i32; 3], f64)>,
    /// The distinct chunks players stand in (sorted).
    stands: Vec<ChunkPos>,
    /// The players standing in `stands[k]`: `order[starts[k]..starts[k + 1]]`. Distance checks
    /// look only at the players of stand chunks close enough to matter (a crowd stands in a
    /// few chunks), each checked exactly as a scan over all players would.
    order: Vec<usize>,
    starts: Vec<usize>,
    /// The bounds (min, max corners) of each stand chunk's players' positions.
    boxes: Vec<([f64; 3], [f64; 3])>,
}

fn cat_index(c: Category) -> usize {
    CATEGORIES.iter().position(|&x| x == c).unwrap_or(0)
}

/// The least and the greatest squared distance from `at` to the points of `b` (min, max
/// corners), over `axes` (indices into x, y, z), summed in that order:
/// for every point in the box, `(p - at)²` summed the same way lies between them, rounding
/// included (each step is monotonic).
fn box_distances(b: &([f64; 3], [f64; 3]), at: [f64; 3], axes: &[usize]) -> (f64, f64) {
    let (mut near, mut far) = (0.0, 0.0);
    for &a in axes {
        let (lo, hi) = (b.0[a], b.1[a]);
        let n = if at[a] < lo { lo - at[a] } else if at[a] > hi { at[a] - hi } else { 0.0 };
        let f = (at[a] - lo).abs().max((hi - at[a]).abs());
        near += n * n;
        far += f * f;
    }
    (near, far)
}

/// Chunk distance (chessboard) at most 8 around a player's chunk: `getPlayersCloseForSpawning`
/// uses 128 blocks to the chunk center.
fn close_for_spawning(p: [f64; 3], c: ChunkPos) -> bool {
    let (cx, cz) = (c.x as f64 * 16.0 + 8.0, c.z as f64 * 16.0 + 8.0);
    (cx - p[0]).powi(2) + (cz - p[2]).powi(2) < 16384.0
}

impl Spawner<'_> {
    /// The stand chunks at most `reach` chunks (chessboard) from `c`, in order (`stands` is
    /// sorted by x first, so only the run with x in range is looked at).
    fn stands_near(&self, c: ChunkPos, reach: i32) -> impl Iterator<Item = usize> + '_ {
        let lo = self.stands.partition_point(|s| s.x < c.x - reach);
        let hi = self.stands.partition_point(|s| s.x <= c.x + reach);
        (lo..hi.max(lo)).filter(move |&k| (self.stands[k].z - c.z).abs() <= reach)
    }

    /// The players standing in chunks at most `reach` chunks (chessboard) from `c`.
    fn players_near(&self, c: ChunkPos, reach: i32) -> impl Iterator<Item = usize> + '_ {
        self.stands_near(c, reach).flat_map(|k| self.order[self.starts[k]..self.starts[k + 1]].iter().copied())
    }

    /// The players [`close_for_spawning`] to `c`: only players within 8 chunks can be (a
    /// player 9 chunks away is at least 136 blocks from the chunk's centre).
    fn close_players(&self, c: ChunkPos) -> impl Iterator<Item = usize> + '_ {
        self.players_near(c, 8).filter(move |&i| close_for_spawning(self.pos[i], c))
    }

    /// Whether some player is within `sqrt(r2)` blocks of (`x`, `y`, `z`).
    fn player_within(&self, x: f64, y: f64, z: f64, r2: f64) -> bool {
        // |dx| <= r moves the chunk by at most ceil(r / 16).
        let reach = (r2.sqrt().ceil() as i32 + 15) / 16;
        let c = ChunkPos::of_block(x.floor() as i32, z.floor() as i32);
        let at = [x, y, z];
        self.stands_near(c, reach).any(|k| {
            // The stand's box first: all its players beyond reach, or all within it.
            let (near, far) = box_distances(&self.boxes[k], at, &[0, 1, 2]);
            if near > r2 {
                return false;
            }
            if far <= r2 {
                return true;
            }
            self.order[self.starts[k]..self.starts[k + 1]].iter().any(|&i| {
                let p = self.pos[i];
                (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2) <= r2
            })
        })
    }

    /// Whether some player is [`close_for_spawning`] to `c`.
    fn any_close(&self, c: ChunkPos) -> bool {
        let centre = [c.x as f64 * 16.0 + 8.0, 0.0, c.z as f64 * 16.0 + 8.0];
        self.stands_near(c, 8).any(|k| {
            let (near, far) = box_distances(&self.boxes[k], centre, &[0, 2]);
            if near >= 16384.0 {
                return false;
            }
            if far < 16384.0 {
                return true;
            }
            self.order[self.starts[k]..self.starts[k + 1]].iter().any(|&i| close_for_spawning(self.pos[i], c))
        })
    }

    /// `LocalMobCapCalculator.canSpawn`: some player close to `c` has room for `cat`. While
    /// the stand chunks within 8 of `c` bound their players' counts below the cap, any close
    /// player has room; else the category's counts are made (once) and only the players
    /// with room are looked at.
    fn local_ok(&mut self, c: ChunkPos, cat: Category) -> bool {
        let i = cat_index(cat);
        let max = cat.max_instances();
        if !self.known[i] {
            if self.stands_near(c, 8).all(|k| self.upper[k][i] < max) {
                return self.any_close(c);
            }
            self.count_category(i, max);
        }
        let near: smallvec::SmallVec<[usize; 32]> = self.stands_near(c, 8).collect();
        for k in near {
            let room = &mut self.room[i][k];
            // Players that filled up since stay out.
            room.retain(|&p| self.local[p][i] < max);
            if room.iter().any(|&p| close_for_spawning(self.pos[p], c)) {
                return true;
            }
        }
        false
    }

    /// [`Spawner::local_ok`] without counting: `None` if it would need counts not made yet.
    fn local_ok_now(&self, c: ChunkPos, cat: Category) -> Option<bool> {
        let i = cat_index(cat);
        let max = cat.max_instances();
        if !self.known[i] {
            return self.stands_near(c, 8).all(|k| self.upper[k][i] < max).then(|| self.any_close(c));
        }
        Some(self.stands_near(c, 8).any(|k| self.room[i][k].iter().any(|&p| self.local[p][i] < max && close_for_spawning(self.pos[p], c))))
    }

    /// Every player's local count of category index `i`, and the players with room per stand.
    fn count_category(&mut self, i: usize, max: i32) {
        self.known[i] = true;
        let mut chunks: Vec<ChunkPos> = self.mobs.iter().filter(|&&(_, k)| k == i).map(|&(c, _)| c).collect();
        chunks.sort_unstable();
        // Distinct mob chunks with their mob counts.
        let mut weighted: Vec<(ChunkPos, i32)> = Vec::new();
        for c in chunks {
            match weighted.last_mut() {
                Some((last, n)) if *last == c => *n += 1,
                _ => weighted.push((c, 1)),
            }
        }
        self.room[i] = vec![Vec::new(); self.stands.len()];
        for k in 0..self.stands.len() {
            let sc = self.stands[k];
            let near: Vec<(ChunkPos, i32)> = weighted.iter().copied().filter(|(c, _)| (sc.x - c.x).abs() <= 8 && (sc.z - c.z).abs() <= 8).collect();
            for &p in &self.order[self.starts[k]..self.starts[k + 1]] {
                let at = self.pos[p];
                let n = near.iter().filter(|(c, _)| close_for_spawning(at, *c)).map(|(_, n)| n).sum::<i32>();
                self.local[p][i] = n;
                if n < max {
                    self.room[i][k].push(p);
                }
            }
        }
    }

    fn add(&mut self, c: ChunkPos, cat: Category) {
        // `MISC` mobs are not counted.
        if !CATEGORIES.contains(&cat) {
            return;
        }
        let i = cat_index(cat);
        self.mobs.push((c, i));
        let near: smallvec::SmallVec<[usize; 32]> = self.stands_near(c, 8).collect();
        for k in near {
            self.upper[k][i] += 1;
        }
        // Counted players keep their counts.
        if self.known[i] {
            let close: smallvec::SmallVec<[usize; 64]> = self.close_players(c).collect();
            for p in close {
                self.local[p][i] += 1;
            }
        }
    }
}

/// Counters of the natural spawner's decisions, the same keys the java agent of `tools/wp74/agent` writes for a vanilla server
/// (`NaturalSpawner.getRandomSpawnMobAt`, `isValidSpawnPostitionForType`, `isValidPositionForMob`, `canSpawnForCategoryGlobal`,
/// `getFilteredSpawningCategories`), to compare spawn decisions on identical worlds. Off unless `KILN_SPAWN_COUNTERS=<file>` is set (the
/// file is rewritten every 100 ticks; `KILN_SPAWN_BOX=x0,y0,z0,x1,y1,z1` adds the `IN-BOX` counts of `validPosition`). While on, the
/// speculative pass is skipped so every attempt is counted once.
pub mod stats {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    struct Config {
        file: std::path::PathBuf,
        bounds: Option<[i32; 6]>,
    }

    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    static COUNTS: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

    fn config() -> Option<&'static Config> {
        CONFIG
            .get_or_init(|| {
                let file = std::env::var_os("KILN_SPAWN_COUNTERS").filter(|f| !f.is_empty())?;
                let bounds = std::env::var("KILN_SPAWN_BOX").ok().and_then(|b| {
                    let v: Vec<i32> = b.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                    <[i32; 6]>::try_from(v).ok()
                });
                Some(Config { file: file.into(), bounds })
            })
            .as_ref()
    }

    /// Whether the counters are on.
    pub fn enabled() -> bool {
        config().is_some()
    }

    pub(crate) fn add(key: String) {
        *COUNTS.lock().unwrap().entry(key).or_insert(0) += 1;
    }

    /// `validPosition` of the vanilla agent: per type, and per type for positions in the box.
    pub(crate) fn valid_position(type_name: &str, pos: [i32; 3], ok: bool, game_time: i64) {
        let Some(c) = config() else { return };
        let name = type_name.strip_prefix("minecraft:").unwrap_or(type_name);
        add(format!("validPosition {name} -> {ok}"));
        if let Some(b) = c.bounds
            && (0..3).all(|a| pos[a] >= b[a] && pos[a] <= b[a + 3])
        {
            add(format!("validPosition IN-BOX {name} -> {ok}"));
            add(format!("validPosition IN-BOX Y={} {name} -> {ok}", pos[1]));
            if let Some(file) = std::env::var_os("KILN_SPAWN_TRACE") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                    let _ = writeln!(f, "{game_time} {name} {} {} {} {ok}", pos[0], pos[1], pos[2]);
                }
            }
        }
    }

    /// `start` of the vanilla agent: a pack start (counted per height for the columns of the box).
    pub(crate) fn start(category: kiln_entity::mob::Category, pos: [i32; 3]) {
        let Some(c) = config() else { return };
        if let Some(b) = c.bounds
            && pos[0] >= b[0]
            && pos[0] <= b[3]
            && pos[2] >= b[2]
            && pos[2] <= b[5]
        {
            add(format!("start {} Y={}", category_name(category), if (127..=134).contains(&pos[1]) { pos[1].to_string() } else { "other".to_owned() }));
        }
        // Every monster pack start, so that the rows of the trace are told apart by the start they belong to.
        if category == kiln_entity::mob::Category::Monster
            && let Some(file) = std::env::var_os("KILN_SPAWN_TRACE")
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = writeln!(f, "S {} {} {}", pos[0], pos[1], pos[2]);
            }
        }
    }

    /// `chunkAttempt` of the vanilla agent: one try of a chunk for a category (counted per chunk for the chunks the box touches).
    pub(crate) fn chunk_attempt(category: kiln_entity::mob::Category, chunk: [i32; 2]) {
        let Some(c) = config() else { return };
        let name = category_name(category);
        add(format!("chunkAttempt {name} all"));
        if let Some(b) = c.bounds
            && chunk[0] >= (b[0] >> 4)
            && chunk[0] <= (b[3] >> 4)
            && chunk[1] >= (b[2] >> 4)
            && chunk[1] <= (b[5] >> 4)
        {
            add(format!("chunkAttempt {name} {},{}", chunk[0], chunk[1]));
        }
    }

    pub(crate) fn valid_mob(type_name: &str, pos: [i32; 3], ok: bool) {
        let Some(c) = config() else { return };
        let name = type_name.strip_prefix("minecraft:").unwrap_or(type_name);
        add(format!("validMob {name} -> {ok}"));
        if let Some(b) = c.bounds
            && (0..3).all(|a| pos[a] >= b[a] && pos[a] <= b[a + 3])
        {
            add(format!("validMob IN-BOX {name} -> {ok}"));
        }
    }

    /// `spawned` of the vanilla agent (`SpawnState.afterSpawn`): a natural spawn, per type and for the box.
    pub(crate) fn spawned(type_name: &str, pos: [i32; 3]) {
        let Some(c) = config() else { return };
        let name = type_name.strip_prefix("minecraft:").unwrap_or(type_name);
        add(format!("spawned {name}"));
        if let Some(b) = c.bounds
            && (0..3).all(|a| pos[a] >= b[a] && pos[a] <= b[a + 3])
        {
            add(format!("spawned IN-BOX {name}"));
        }
    }

    /// `potential` of the vanilla agent (`SpawnState.canSpawn`).
    pub(crate) fn potential(type_name: &str, ok: bool) {
        if enabled() {
            add(format!("potential {} -> {ok}", type_name.strip_prefix("minecraft:").unwrap_or(type_name)));
        }
    }

    /// `localCap` of the vanilla agent (`canSpawnForCategoryLocal`).
    pub(crate) fn local_cap(c: kiln_entity::mob::Category, ok: bool) {
        if enabled() {
            add(format!("localCap {} -> {ok}", category_name(c)));
        }
    }

    pub(crate) fn pick(type_name: Option<&str>, category: kiln_entity::mob::Category, pos: [i32; 3]) {
        let Some(c) = config() else { return };
        let name = type_name.map_or("(none)", |n| n.strip_prefix("minecraft:").unwrap_or(n));
        add(format!("pick {name}"));
        if let Some(b) = c.bounds
            && (0..3).all(|a| pos[a] >= b[a] && pos[a] <= b[a + 3])
        {
            add(format!("pickAt IN-BOX {} {name}", category_name(category)));
            add(format!("pickAt IN-BOX Y={} {} {name}", pos[1], category_name(category)));
        }
    }

    /// The vanilla enum name of a category.
    pub(crate) fn category_name(c: kiln_entity::mob::Category) -> &'static str {
        use kiln_entity::mob::Category::*;
        match c {
            Monster => "MONSTER",
            Creature => "CREATURE",
            Misc => "MISC",
            Ambient => "AMBIENT",
            Axolotls => "AXOLOTLS",
            UndergroundWaterCreature => "UNDERGROUND_WATER_CREATURE",
            WaterCreature => "WATER_CREATURE",
            WaterAmbient => "WATER_AMBIENT",
        }
    }

    pub(crate) fn global(c: kiln_entity::mob::Category, chunks: i32, count: i32, ok: bool) {
        if enabled() && matches!(c, kiln_entity::mob::Category::Creature | kiln_entity::mob::Category::Monster) {
            add(format!("global {} chunks={chunks} count={count} -> {ok}", category_name(c)));
        }
    }

    pub(crate) fn categories(list: &[kiln_entity::mob::Category]) {
        if enabled() {
            add(format!("categories [{}]", list.iter().map(|c| category_name(*c)).collect::<Vec<_>>().join(", ")));
        }
    }

    /// The counters so far.
    pub fn snapshot() -> BTreeMap<String, u64> {
        COUNTS.lock().unwrap().clone()
    }

    /// Writes the counters to the file (`key<TAB>count` per line).
    pub(crate) fn dump() {
        let Some(c) = config() else { return };
        let text: String = COUNTS.lock().unwrap().iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
        let _ = std::fs::write(&c.file, text);
    }
}

/// What `canSpawnForCategoryGlobal` reads for a whole level: the mobs `createState` counts per category and `getNaturalSpawnChunkCount`.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpawnTotals {
    pub counts: [i32; N],
    pub chunks: i32,
}

/// `createState`'s counts over a list of entities: every mob that is not persistent (`isPersistenceRequired() ||
/// requiresCustomPersistence()`) and not of the `MISC` category, wherever it stands.
pub(crate) fn count_mobs(list: &[crate::entities::Entity]) -> [i32; N] {
    let mut counts = [0; N];
    for e in list {
        if e.removed {
            continue;
        }
        let Some(p) = e.phys.as_deref() else { continue };
        let Some(m) = mob::data(p) else { continue };
        if mob::persistent_for_counts(p, m) || !CATEGORIES.contains(&m.kind.category()) {
            continue;
        }
        counts[cat_index(m.kind.category())] += 1;
    }
    counts
}

/// The positions (x, z) of the players `DistanceManager` tracks for the spawn range: every player that is not skipped (a spectator is, unless
/// `spectators_generate_chunks`).
pub(crate) fn players_for_chunks<'a>(players: &'a [&mut Player], spectators_generate_chunks: bool) -> impl Iterator<Item = [f64; 2]> + 'a {
    players.iter().filter(move |p| !p.disconnected && (p.game_mode != 3 || spectators_generate_chunks)).map(|p| [p.pos[0], p.pos[2]])
}

/// `getNaturalSpawnChunkCount`: the chunks within 8 (chessboard) of the chunk any of the players stands in.
pub(crate) fn spawn_chunk_count(players: impl Iterator<Item = [f64; 2]>) -> i32 {
    let mut stands: Vec<ChunkPos> = players.map(|p| ChunkPos::of_block(p[0].floor() as i32, p[1].floor() as i32)).collect();
    stands.sort_unstable();
    stands.dedup();
    // Per row of chunks, the x ranges of the squares that cover it, merged.
    let mut rows: std::collections::BTreeMap<i32, Vec<(i32, i32)>> = std::collections::BTreeMap::new();
    for c in &stands {
        for z in c.z - 8..=c.z + 8 {
            rows.entry(z).or_default().push((c.x - 8, c.x + 8));
        }
    }
    let mut n = 0;
    for ranges in rows.values_mut() {
        ranges.sort_unstable();
        let mut end = i32::MIN;
        for &(lo, hi) in ranges.iter() {
            if lo > end {
                n += hi - lo + 1;
                end = hi;
            } else if hi > end {
                n += hi - end;
                end = hi;
            }
        }
    }
    n
}

/// One tick of natural spawning in a region (`ServerChunkCache.tickChunks`' spawning part).
#[inline(never)]
pub(crate) fn tick(
    level: &mut RegionLevel,
    entities: &Entities,
    players: &[&mut Player],
    ticking: &Ticking,
    spawns: &mut Vec<Spawn>,
    ctx: &kiln_sched::Ctx<'_>,
) {
    let env = level.env;
    let rules = env.mobs;

    let Some(table) = env.spawn_table.clone() else { return };
    let dt = std::time::Instant::now();
    let spawn_enemies = rules.difficulty != 0 && rules.spawn_monsters;
    let spawn_persistent = env.game_time % 400 == 0;
    if spawn_enemies && rules.spawn_phantoms {
        phantoms(level, players, spawns);
    }
    let players_all = players;
    let players: Vec<[f64; 3]> =
        players.iter().filter(|p| !p.disconnected && !p.dead && p.game_mode != 3).map(|p| p.pos).collect();
    if players.is_empty() {
        return;
    }
    // The distinct chunks players stand in (a crowd shares a few).
    let chunk_of = |p: &[f64; 3]| ChunkPos::of_block(p[0].floor() as i32, p[2].floor() as i32);
    let mut stands: Vec<ChunkPos> = players.iter().map(chunk_of).collect();
    stands.sort_unstable();
    stands.dedup();
    // Players grouped by the chunk they stand in, in player order within a chunk.
    let mut keyed: Vec<(ChunkPos, usize)> = players.iter().enumerate().map(|(i, p)| (chunk_of(p), i)).collect();
    keyed.sort_unstable();
    let order: Vec<usize> = keyed.iter().map(|&(_, i)| i).collect();
    let mut starts = Vec::with_capacity(stands.len() + 1);
    let mut next = 0;
    for &c in &stands {
        starts.push(next);
        while next < keyed.len() && keyed[next].0 == c {
            next += 1;
        }
    }
    starts.push(next);
    let boxes: Vec<([f64; 3], [f64; 3])> = (0..stands.len())
        .map(|k| {
            let mut b = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for &(_, i) in &keyed[starts[k]..starts[k + 1]] {
                for a in 0..3 {
                    b.0[a] = b.0[a].min(players[i][a]);
                    b.1[a] = b.1[a].max(players[i][a]);
                }
            }
            b
        })
        .collect();
    // `canSpawnForCategoryGlobal`, once for the tick (`getFilteredSpawningCategories`): the mobs of the whole level (`createState` counts
    // every loaded entity, near a player or not) against the cap of the chunks within 8 of any player. A level with several regions
    // has them added up by [`crate::Sim`] before the regions run; else this region is the level.
    let totals = match &env.spawn_totals {
        Some(t) => (**t).clone(),
        None => SpawnTotals {
            counts: count_mobs(&entities.list),
            chunks: spawn_chunk_count(players_for_chunks(players_all, rules.spectators_generate_chunks)),
        },
    };
    let global_ok: [bool; N] = std::array::from_fn(|i| totals.counts[i] < CATEGORIES[i].max_instances() * totals.chunks / 289);
    if stats::enabled() && env.game_time % 100 == 0 {
        stats::dump();
    }
    let dt = crate::diag::lap("s.b_caps", dt);
    let mut s = Spawner {
        pos: players.clone(),
        local: vec![[0; N]; players.len()],
        mobs: Vec::new(),
        upper: vec![[0; N]; stands.len()],
        known: [false; N],
        room: Default::default(),
        table: &table,
        charges: Vec::new(),
        stands,
        boxes,
        order,
        starts,
    };
    // `createState`: mobs per category, persistent ones excluded.
    for e in &entities.list {
        let Some(m) = e.phys.as_deref().and_then(mob::data) else { continue };
        if e.removed || e.phys.as_deref().is_none_or(|p| mob::persistent_for_counts(p, m)) {
            continue;
        }
        s.add(crate::entities::chunk_of(e.pos), m.kind.category());
        // `createState`: the charge of the mob's type in the biome it stands in.
        let at = KBlockPos::new(e.pos[0].floor() as i32, e.pos[1].floor() as i32, e.pos[2].floor() as i32);
        if !table.costs.is_empty()
            && let Some((charge, _)) = table.cost(spawn_biome(level, at), e.kind.name)
        {
            s.charges.push(([at.x, at.y, at.z], charge));
        }
    }
    // `getFilteredSpawningCategories`.
    let categories: Vec<Category> = CATEGORIES
        .into_iter()
        .filter(|c| {
            let pass = rules.spawn_mobs && (spawn_enemies || c.friendly()) && (spawn_persistent || !c.persistent());
            if pass {
                stats::global(*c, totals.chunks, totals.counts[cat_index(*c)], global_ok[cat_index(*c)]);
            }
            pass && global_ok[cat_index(*c)]
        })
        .collect();
    if rules.spawn_mobs {
        stats::categories(&categories);
    }
    // `ServerChunkCache.tickSpawningChunk`: every ticking chunk with a player near gets one more
    // tick of `InhabitedTime`, whether or not anything may spawn.
    level.cells.for_each_cell_mut(&mut |pos, cell| {
        for (c, chunk) in cell.chunks_mut(pos) {
            if ticking.contains_loading_block(c) && s.any_close(c) {
                chunk.increment_inhabited_time();
            }
        }
    });
    if categories.is_empty() {
        return;
    }
    let dt = crate::diag::lap("s.c_state", dt);
    // `collectSpawningChunks`: loaded chunks with the block-ticking status with a player within 128 blocks, which `tickSpawningChunk` then spawns in only if entities
    // tick there (`ServerLevel.canSpawnEntitiesInChunk`: within the view distance). Both are the loading tickets' levels (the view distance, not the simulation distance).
    let mut chunks: Vec<(u64, ChunkPos)> = Vec::new();
    level.cells.for_each_cell(&mut |pos, cell| {
        for (c, _) in cell.chunks(pos) {
            if ticking.contains_visible(c) && s.any_close(c) {
                let mut h = (env.seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.game_time as u64;
                h ^= (c.x as u32 as u64) << 32 | c.z as u32 as u64;
                h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                chunks.push((h ^ (h >> 29), c));
            }
        }
    });
    chunks.sort_unstable();
    let dt = crate::diag::lap("s.setup", dt);
    // The categories whose local caps may bind are counted now, so the chunks below can be
    // looked at side by side.
    for &cat in &categories {
        let (i, max) = (cat_index(cat), cat.max_instances());
        if !s.known[i] && s.upper.iter().any(|u| u[i] >= max) {
            s.count_category(i, max);
        }
    }
    // Every chunk alone against the state before the first one, in parallel: what each
    // category's caps said and whether anything spawned. Only a spawn changes what a later
    // chunk sees (its caps), and spawns are rare: in turn, a chunk that spawned nothing and
    // whose caps still say the same is done; the others run again, so the outcome is the
    // serial one whatever the workers.
    let speculated: Vec<Option<smallvec::SmallVec<[bool; N]>>> = if stats::enabled() {
        vec![None; chunks.len()]
    } else {
        let (lvl, sp, cats) = (&*level, &s, &categories);
        ctx.map_indexed_with(SPAWN_WINDOW, &chunks, |_, &(_, c)| speculate(lvl, sp, c, cats, ticking))
    };
    let dt = crate::diag::lap("s.spec", dt);
    let mut spawned_any = false;
    for (&(_, c), guess) in chunks.iter().zip(speculated) {
        if let Some(said) = guess
            && (!spawned_any || categories.iter().zip(&said).all(|(&cat, &ok)| s.local_ok(c, cat) == ok))
        {
            continue;
        }
        let mut r = chunk_random(env.seed, env.game_time, c);
        for &cat in &categories {
            let ok = s.local_ok(c, cat);
            stats::local_cap(cat, ok);
            if ok {
                let mut made = Vec::new();
                spawn_category_for_chunk(level, &s, &mut r, cat, c, ticking, &mut made);
                for (spawn, _pc, charge) in made {
                    spawns.push(spawn);
                    // (`afterSpawn.run(mob, chunk)`: the chunk the pack started in counts it for the players near, not the one the member stands in.)
                    s.add(c, cat);
                    s.charges.extend(charge);
                    spawned_any = true;
                }
            }
        }
    }
    crate::diag::lap("s.confirm", dt);
}

/// The spawning pass's chunks, a few microseconds each.
const SPAWN_WINDOW: kiln_sched::Window = kiln_sched::Window::new().item_ns(2_500);

/// Chunk `c`'s turn against `s` as it stands: per category, whether its caps let it try; `None`
/// if something spawned (or a cap needs counts not made yet).
fn speculate(
    level: &RegionLevel,
    s: &Spawner,
    c: ChunkPos,
    categories: &[Category],
    ticking: &Ticking,
) -> Option<smallvec::SmallVec<[bool; N]>> {
    let mut r = chunk_random(level.env.seed, level.env.game_time, c);
    let mut said = smallvec::SmallVec::new();
    for &cat in categories {
        let ok = s.local_ok_now(c, cat)?;
        said.push(ok);
        if ok {
            let mut made = Vec::new();
            spawn_category_for_chunk(level, s, &mut r, cat, c, ticking, &mut made);
            if !made.is_empty() {
                return None;
            }
        }
    }
    Some(said)
}

/// `PhantomSpawner.tick`, per region. Approximations: the pass comes every 1200 ticks with a
/// chance of 2 in 3 from a random seeded by the world and the time (vanilla waits 1200 to 2379
/// ticks, 1790 on average: here 1800). The insomnia statistic is `time_since_rest` (see
/// [`crate::sleep`]).
fn phantoms(level: &RegionLevel, players: &[&mut Player], spawns: &mut Vec<Spawn>) {
    let env = level.env;
    if env.game_time % 1200 != 0 {
        return;
    }
    let mut r = chunk_random(env.seed ^ 0x7068_616e_746f_6d, env.game_time, ChunkPos::new(0, 0));
    if r.next_int_bounded(3) == 0 {
        return;
    }
    let skylight = env.dim == crate::OVERWORLD_ID;
    if env.mobs.sky_darken < 5 && skylight {
        return;
    }
    for p in players.iter().filter(|p| !p.disconnected && !p.dead && p.game_mode != 3) {
        let pos = KBlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
        let sky = kiln_world::light::light_at(&*level.cells, kiln_world::chunk::LightLayer::Sky, pos.x, pos.y, pos.z).map_or(15, i32::from);
        if skylight && (pos.y < 63 || sky < 15) {
            continue;
        }
        let inhabited = inhabited_at(&*level.cells, pos.x, pos.z);
        let ctx = crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, inhabited, moon_brightness(env.mobs.day_time));
        if !(ctx.effective_difficulty > r.next_float() * 3.0) {
            continue;
        }
        let since_rest = p.stats.get(*crate::player_stats::stat::TIME_SINCE_REST).max(1);
        if r.next_int_bounded(since_rest) < 72000 {
            continue;
        }
        let up = 20 + r.next_int_bounded(15);
        let east = -10 + r.next_int_bounded(21);
        let south = -10 + r.next_int_bounded(21);
        let at = KBlockPos::new(pos.x + east, pos.y + up, pos.z + south);
        let state = level.block(at);
        if !kiln_entity::mob::path::valid_empty_spawn(state, false) || !kiln_entity::physics::fluid_state(state).is_empty() {
            continue;
        }
        let count = 1 + r.next_int_bounded(env.mobs.difficulty as i32 + 1);
        for _ in 0..count {
            let seed = r.next_long();
            let fin = crate::mobs::Finalize { ctx, seed, persistent: false, natural: false, monsters_disabled: false, camel_space: false, structures: Vec::new() };
            spawns.push(crate::mobs::spawn(MobKind::Phantom, [at.x as f64 + 0.5, at.y as f64, at.z as f64 + 0.5], Some(0.0), Some(fin)));
        }
    }
}

/// `spawnCategoryForChunk` + `spawnCategoryForPosition`: what spawns, with the chunk each
/// counts in.
#[inline(never)]
/// `getRandomPosWithin`'s upper bound: `chunk.getHeight(WORLD_SURFACE, x, z) + 1`. `ChunkAccess.getHeight` is the *top block* (the heightmap's first available
/// y, minus one), so the bound is the first free y above the column: on the nether roof (bedrock to y 127) it is 128, and a start can be no higher.
/// (Kiln added one more: starts on the roof at y 129, which land next to a farm platform at 128 and drift onto it, twice as many as vanilla's.)
fn start_top(chunk: &kiln_world::chunk::Chunk, x: i32, z: i32) -> i32 {
    chunk.column_height((x & 15) as usize, (z & 15) as usize, |b| !kiln_data::blocks_types::is_air(b))
}

fn spawn_category_for_chunk(
    level: &RegionLevel,
    s: &Spawner,
    r: &mut LegacyRandom,
    cat: Category,
    c: ChunkPos,
    ticking: &Ticking,
    spawns: &mut Vec<(Spawn, ChunkPos, Option<([i32; 3], f64)>)>,
) {
    let env = level.env;
    let min_y = env.min_y;
    stats::chunk_attempt(cat, [c.x, c.z]);
    // `getRandomPosWithin`.
    let x = c.x * 16 + r.next_int_bounded(16);
    let z = c.z * 16 + r.next_int_bounded(16);
    let Some(chunk) = level.cells.chunk(c) else { return };
    let top = start_top(chunk, x, z);
    let y = r.next_int_bounded(top - min_y + 1) + min_y;
    if y < min_y + 1 {
        return;
    }
    let start = KBlockPos::new(x, y, z);
    stats::start(cat, [x, y, z]);
    if kiln_data::block_logic::is_redstone_conductor(level.block(start)) {
        return;
    }
    let mut spawned = 0;
    // The charges of the mobs this call spawned (`afterSpawn` adds them at once).
    let mut mine: Vec<([i32; 3], f64)> = Vec::new();
    for _ in 0..3 {
        let (mut px, mut pz) = (x, z);
        let mut data: Option<SpawnerData> = None;
        let mut group = mob::GroupData::default();
        let mut pack = mob::mth::ceil((r.next_float() * 4.0) as f64);
        let mut in_group = 0;
        let mut k = 0;
        while k < pack {
            k += 1;
            px += r.next_int_bounded(6) - r.next_int_bounded(6);
            pz += r.next_int_bounded(6) - r.next_int_bounded(6);
            let (fx, fz) = (px as f64 + 0.5, pz as f64 + 0.5);
            // `isRightDistanceToPlayerAndSpawnPoint`: the nearest player farther than 24.
            if s.player_within(fx, y as f64, fz, 576.0) {
                continue;
            }
            let sp = env.mobs.spawn_point;
            let (sx, sy, sz) = (sp[0] as f64 + 0.5 - fx, sp[1] as f64 + 0.5 - y as f64, sp[2] as f64 + 0.5 - fz);
            if sx * sx + sy * sy + sz * sz < 576.0 {
                continue;
            }
            let pc = ChunkPos::of_block(px, pz);
            if pc != c && !ticking.contains_visible(pc) {
                continue;
            }
            let pos = KBlockPos::new(px, y, pz);
            let biome = spawn_biome(level, pos);
            // What spawns at this very place (looked up once: the pick and `canSpawnMobAt` below
            // ask the same).
            let here = s.table.mobs_at(level, biome, cat, pos);
            if data.is_none() {
                // `getRandomSpawnMobAt`: rivers have most of their ambient water spawns taken away.
                if cat == Category::WaterAmbient && kiln_entity::mob::kinds::slime::biome_in_tag(biome as i32, "minecraft:reduce_water_ambient_spawns") && r.next_float() < 0.98 {
                    stats::pick(None, cat, [pos.x, pos.y, pos.z]);
                    break;
                }
                match pick(here, r) {
                    None => {
                        stats::pick(None, cat, [pos.x, pos.y, pos.z]);
                        break;
                    }
                    Some(d) => {
                        stats::pick(Some(d.type_name), cat, [pos.x, pos.y, pos.z]);
                        let d = d.clone();
                        pack = if d.constant { d.min } else { r.next_int_bounded(d.max - d.min + 1) + d.min };
                        data = Some(d);
                    }
                }
            }
            let d_ = data.clone().unwrap();
            let Some(kind) = d_.kind else { continue };
            // `isValidSpawnPostitionForType`.
            let t = kiln_data::entities::by_name(kind.type_name()).unwrap();
            let valid = 'valid: {
                // The nearest player within the despawn distance, unless the type may spawn far from players (`EntityType.canSpawnFarFromPlayer`: the
                // creature and misc categories, and pillagers and shulkers).
                let far_ok = matches!(kind.category(), Category::Creature | Category::Misc) || matches!(kind, MobKind::Pillager | MobKind::Shulker);
                if !far_ok && !s.player_within(fx, y as f64, fz, (cat.despawn_distance() * cat.despawn_distance()) as f64) {
                    break 'valid false;
                }
                // `canSpawnMobAt`: what was picked is among what spawns at this very place.
                if !here.iter().any(|e| *e == d_) {
                    break 'valid false;
                }
                if !placement_ok(level, pos, kind) || !check_spawn_rules(level, pos, kind, r) {
                    break 'valid false;
                }
                // `EntityType.getSpawnAABB`: slimes and magma cubes look for room for a size-4 one (four times their box).
                let scale = kiln_entity::spawner::spawn_dimensions_scale(t.name);
                no_collision(level, [fx, y as f64, fz], t.width * scale, t.height * scale)
            };
            stats::valid_position(d_.type_name, [pos.x, pos.y, pos.z], valid, env.game_time);
            if !valid {
                continue;
            }
            // `SpawnState.canSpawn` (the spawn predicate): the biome's cost for the type, and the energy the charges near the spot give it.
            let cost = s.table.cost(biome, t.name);
            if let Some((charge, budget)) = cost
                && charge != 0.0
            {
                let mut sum = 0.0f64;
                for (p, c) in s.charges.iter().chain(mine.iter()) {
                    let d2 = ((pos.x - p[0]) as f64).powi(2) + ((pos.y - p[1]) as f64).powi(2) + ((pos.z - p[2]) as f64).powi(2);
                    sum += if d2 == 0.0 { f64::INFINITY } else { c / d2.sqrt() };
                }
                if sum * charge > budget {
                    stats::potential(d_.type_name, false);
                    continue;
                }
            }
            stats::potential(d_.type_name, true);
            let yaw = r.next_float() * 360.0;
            // `isValidPositionForMob`: the walk target value and the spawn obstruction.
            let magic = light_magic(level, pos);
            let walk = if let Some(v) = kind.ext().and_then(|k| k.spawn_walk_target(&View(level), kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z))) {
                v
            } else if kind.is_animal() {
                if kiln_data::blocks_types::block_of(level.block(pos.below())).name == "minecraft:grass_block" { 10.0 } else { magic - 0.5 }
            } else {
                -(magic - 0.5)
            };
            let ignores_light = kind.ext().is_some_and(|k| k.spawn_ignores_light());
            let liquid_ok = kind.ext().is_some_and(|k| k.spawn_in_liquids());
            let mob_ok = !((walk < 0.0 && !ignores_light) || (!liquid_ok && contains_liquid(level, [fx, y as f64, fz], t.width, t.height)));
            stats::valid_mob(d_.type_name, [pos.x, pos.y, pos.z], mob_ok);
            if !mob_ok {
                continue;
            }
            // `finalizeSpawn` draws from the chunk's random.
            let inhabited = inhabited_at(&*level.cells, fx.floor() as i32, fz.floor() as i32);
            let mut ctx = crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, inhabited, moon_brightness(env.mobs.day_time));
            ctx.biome = Some(biome as i32);
            let seed = r.next_long();
            let _ = &mut group;
            let monsters_disabled = env.mobs.difficulty == 0 || !env.mobs.spawn_monsters;
            // `Husk.finalizeSpawn`: a camel husk jockey needs the room for the camel.
            let camel_space = kind == MobKind::Husk
                && kiln_data::entities::by_name("minecraft:camel_husk").is_some_and(|c| no_collision(level, [fx.floor() + 0.5, y as f64, fz.floor() + 0.5], c.width, c.height));
            // `VariantUtils`: the structures the variants ask about (a cat inside a swamp hut is all black).
            let structures = structures_at(level, [fx.floor() as i32, y, fz.floor() as i32], kind);
            let fin = crate::mobs::Finalize { ctx, seed, persistent: false, natural: true, monsters_disabled, camel_space, structures };
            // The caller counts it (nothing below reads the counts).
            let charge = cost.map(|(c, _)| ([pos.x, pos.y, pos.z], c));
            mine.extend(charge);
            spawns.push((crate::mobs::spawn(kind, [fx, y as f64, fz], Some(yaw), Some(fin)), pc, charge));
            stats::spawned(d_.type_name, [pos.x, pos.y, pos.z]);
            spawned += 1;
            in_group += 1;
            if spawned >= kind.ext().map_or(4, |k| k.max_spawn_cluster()) {
                return;
            }
            let _ = in_group;
        }
    }
}

/// `ChunkAccess.getInhabitedTime` of the chunk holding block column `x`, `z` (0 when it is not
/// loaded), for the regional difficulty there (`ServerLevel.getCurrentDifficultyAt`).
pub(crate) fn inhabited_at(cells: &impl kiln_world::Blocks, x: i32, z: i32) -> i64 {
    cells.chunk(ChunkPos::of_block(x, z)).map_or(0, |c| c.inhabited_time())
}

/// The biome natural spawning sees at `pos`: `natural_mob_spawns` is a full-resolution biome attribute, so `BiomeManager.getBiome` (the voronoi-zoomed
/// lookup that moves the borders around by up to a cell), not the stored cell.
pub(crate) fn spawn_biome(level: &RegionLevel, pos: KBlockPos) -> u16 {
    crate::weather::biome_at(level.cells, level.env, pos)
}

/// The biome of the stored 4×4×4 cell holding `pos`.
pub(crate) fn biome_at(level: &RegionLevel, pos: KBlockPos) -> u16 {
    biome_at_in(level.cells, level.env, pos)
}

/// [`biome_at`] from the cells and the environment.
pub(crate) fn biome_at_in(cells: &kiln_region::CellSet<kiln_world::Cell>, env: &crate::blocks::BlockEnv, pos: KBlockPos) -> u16 {
    let Some(chunk) = cells.chunk(ChunkPos::of_block(pos.x, pos.z)) else { return 0 };
    let rel = pos.y - env.min_y;
    let Some(section) = chunk.sections.get((rel >> 4).max(0) as usize) else { return 0 };
    match &section.biomes {
        kiln_world::section::Biomes::Single(b) => *b,
        kiln_world::section::Biomes::Cells(cells) => {
            let (qx, qy, qz) = (((pos.x & 15) >> 2) as usize, (((rel & 15) >> 2).max(0)) as usize, ((pos.z & 15) >> 2) as usize);
            cells[(qy << 4) | (qz << 2) | qx]
        }
    }
}

/// `DimensionType.moonBrightness` for the moon phase of `day_time`.
pub(crate) fn moon_brightness(day_time: i64) -> f32 {
    const PHASES: [f32; 8] = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];
    PHASES[(day_time.div_euclid(24000)).rem_euclid(8) as usize]
}

/// The type's `SpawnPlacementType.isSpawnPositionOk`.
fn placement_ok(level: &RegionLevel, pos: KBlockPos, kind: MobKind) -> bool {
    use kiln_entity::mob::ext::Placement;
    let Some(k) = kind.ext() else { return spawn_position_ok(level, pos, kind) };
    let fluid = |p: KBlockPos| kiln_entity::physics::fluid_state(level.block(p));
    match k.placement() {
        Placement::OnGround => spawn_position_ok(level, pos, kind),
        Placement::InWater => fluid(pos).kind.is_water() && !kiln_data::block_logic::is_redstone_conductor(level.block(pos.above())),
        Placement::InLava => fluid(pos).kind.is_lava(),
        Placement::NoRestrictions => true,
    }
}

/// What a type's own spawn rules see of the region.
struct View<'a, 'l>(&'a RegionLevel<'l>);

impl kiln_entity::mob::ext::SpawnView for View<'_, '_> {
    fn block(&self, pos: kiln_entity::math::BlockPos) -> u16 {
        self.0.block(KBlockPos::new(pos.x, pos.y, pos.z))
    }
    fn raw_brightness(&self, pos: kiln_entity::math::BlockPos, sky_darken: i32) -> i32 {
        self.0.raw_brightness(KBlockPos::new(pos.x, pos.y, pos.z), sky_darken)
    }
    fn sky_darken(&self) -> i32 {
        self.0.env.mobs.sky_darken
    }
    fn sky_light(&self, pos: kiln_entity::math::BlockPos) -> i32 {
        kiln_world::light::light_at(&*self.0.cells, kiln_world::chunk::LightLayer::Sky, pos.x, pos.y, pos.z).map_or(15, i32::from)
    }
    fn block_light(&self, pos: kiln_entity::math::BlockPos) -> i32 {
        kiln_world::light::light_at(&*self.0.cells, kiln_world::chunk::LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from)
    }
    fn biome(&self, pos: kiln_entity::math::BlockPos) -> i32 {
        spawn_biome(self.0, KBlockPos::new(pos.x, pos.y, pos.z)) as i32
    }
    fn difficulty(&self) -> u8 {
        self.0.env.mobs.difficulty
    }
    fn world_seed(&self) -> i64 {
        self.0.env.seed
    }
    fn moon_brightness(&self) -> f32 {
        moon_brightness(self.0.env.mobs.day_time)
    }
    fn min_y(&self) -> i32 {
        self.0.env.min_y
    }
    fn sea_level(&self) -> i32 {
        63
    }
    fn world_surface(&self, x: i32, z: i32) -> Option<i32> {
        let chunk = self.0.cells.chunk(ChunkPos::of_block(x, z))?;
        Some(chunk.column_height((x & 15) as usize, (z & 15) as usize, |b| !kiln_data::blocks_types::is_air(b)))
    }
    fn monster_block_light_limit(&self) -> i32 {
        monster_light_rules(self.0.env.dim).0
    }
    fn monster_light_test(&self) -> (i32, i32) {
        let (_, lo, hi) = monster_light_rules(self.0.env.dim);
        (lo, hi)
    }
    fn thundering(&self) -> bool {
        self.0.env.weather.weather.thundering
    }
}

/// The dimension type's `monster_spawn_block_light_limit` and the inclusive range of its
/// `monster_spawn_light_level` (overworld: 0 and a uniform 0..=7; nether: 15 and 7; end: 0
/// and 15).
pub(crate) fn monster_light_rules(dim: usize) -> (i32, i32, i32) {
    match dim {
        1 => (15, 7, 7),
        2 => (0, 15, 15),
        _ => (0, 0, 7),
    }
}

/// The structure specs (`#tag` or names) the variants of `kind` ask about (`variants::structure_specs`) that `at` is
/// inside a piece of (`StructureCheck`: `getStructureWithPieceAt(pos, structures).isValid()`). Empty for a mob whose variants
/// care about no structure.
pub(crate) fn structures_at(level: &RegionLevel, at: [i32; 3], kind: MobKind) -> Vec<String> {
    let Some(registry) = kiln_entity::mob::variants::registry_of(kind) else { return Vec::new() };
    let specs = kiln_entity::mob::variants::structure_specs(registry);
    if specs.is_empty() {
        return specs;
    }
    let structures = crate::structure_spawns::ChunkStructures(|c: ChunkPos| level.cells.chunk(c));
    specs
        .into_iter()
        .filter(|spec| {
            let ids = match spec.strip_prefix('#') {
                Some(tag) => crate::world_state::worldgen_tag("worldgen/structure", tag).unwrap_or_default(),
                None => vec![spec.clone()],
            };
            crate::structure_spawns::piece_at(&structures, at, &ids)
        })
        .collect()
}

/// `SpawnPlacementTypes.ON_GROUND.isSpawnPositionOk`.
pub(crate) fn spawn_position_ok(level: &RegionLevel, pos: KBlockPos, kind: MobKind) -> bool {
    let animal = kind.is_animal();
    let below = level.block(pos.below());
    // `BlockBehaviour.isValidSpawn` answers per type for a few blocks (checked on every block state and entity type):
    // leaves for ocelots and parrots, ice for polar bears, magma blocks for the fire-immune.
    let by_type = match kiln_data::blocks_types::block_of(below).name {
        "minecraft:ice" | "minecraft:frosted_ice" => kind == MobKind::PolarBear,
        "minecraft:magma_block" => kind.fire_immune(),
        n if n.ends_with("_leaves") => matches!(kind, MobKind::Ocelot | MobKind::Parrot),
        _ => false,
    };
    (by_type || kiln_entity::mob::path::valid_spawn(below, animal))
        && kiln_entity::mob::path::valid_empty_spawn(level.block(pos), animal)
        && kiln_entity::mob::path::valid_empty_spawn(level.block(pos.above()), animal)
}

/// `SpawnPlacements.checkSpawnRules` for the simulated types: animals need light and a
/// spawnable block below, monsters darkness (`Monster.isDarkEnoughToSpawn`) and a valid spawn
/// block below.
pub(crate) fn check_spawn_rules(level: &RegionLevel, pos: KBlockPos, kind: MobKind, r: &mut LegacyRandom) -> bool {
    if let Some(k) = kind.ext() {
        if let Some(ok) = k.check_spawn_rules(&View(level), kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z), r) {
            return ok;
        }
        if kind.category() == kiln_entity::mob::Category::Misc {
            return false;
        }
    }
    let below = level.block(pos.below());
    if kind.is_animal() {
        let bright = level.raw_brightness(pos, 0) > 8;
        return bright && kiln_entity::blocks::has_tag(below, kiln_entity::blocks::Tag::AnimalsSpawnableOn);
    }
    if level.env.mobs.difficulty == 0 {
        return false;
    }
    // `Monster.isDarkEnoughToSpawn` with the dimension's limits, then `Mob.checkMobSpawnRules`.
    kiln_entity::mob::kinds::zombie::dark_enough_view(&View(level), kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z), r) && kiln_entity::mob::path::valid_spawn(below, false)
}

/// `LevelReader.getLightLevelDependentMagicValue`: the light, lerped towards 1 by the dimension's
/// `ambient_light` (0.1 in the nether: a dark nether floor is worth 0.1, not 0).
fn light_magic(level: &RegionLevel, pos: KBlockPos) -> f32 {
    let f = level.raw_brightness(pos, level.env.mobs.sky_darken) as f32 / 15.0;
    let g = f / (4.0 - 3.0 * f);
    let ambient = kiln_data::dimension_type(crate::DIMENSIONS[level.env.dim].0).map_or(0.0, |d| d.ambient_light);
    g + ambient * (1.0 - g)
}

fn aabb(pos: [f64; 3], w: f32, h: f32) -> ([f64; 3], [f64; 3]) {
    let hw = w as f64 / 2.0;
    ([pos[0] - hw, pos[1], pos[2] - hw], [pos[0] + hw, pos[1] + h as f64, pos[2] + hw])
}

/// `noCollision` against blocks for a new mob's box.
fn no_collision(level: &RegionLevel, pos: [f64; 3], w: f32, h: f32) -> bool {
    let (lo, hi) = aabb(pos, w, h);
    for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
        for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
            for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                for b in kiln_data::block_props::collision(level.block(KBlockPos::new(x, y, z))) {
                    let (bx, by, bz) = (x as f64, y as f64, z as f64);
                    if lo[0] < bx + b[3] as f64 && hi[0] > bx + b[0] as f64 && lo[1] < by + b[4] as f64 && hi[1] > by + b[1] as f64 && lo[2] < bz + b[5] as f64 && hi[2] > bz + b[2] as f64 {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// The mobs chunk generation makes (`ChunkGenerator.spawnOriginalMobs` → `NaturalSpawner.spawnMobsForChunkGeneration`) for
/// the freshly generated chunks `pending`: the animals of the biome at the chunk's corner, in groups, from a random seeded by
/// the world seed and the chunk, as vanilla's SPAWN status does (the blocks of a loaded chunk are the ones its SPAWN saw: no
/// feature of a farther chunk reaches it).
pub(crate) fn initial_mobs(level: &RegionLevel, pending: &[ChunkPos], spawns: &mut Vec<Spawn>) {
    let env = level.env;
    if !env.mobs.spawn_mobs {
        return;
    }
    let Some(table) = env.spawn_table.clone() else { return };
    for &c in pending {
        if level.cells.chunk(c).is_some() {
            initial_chunk(level, &table, c, spawns);
        }
    }
}

/// The biome `WorldGenRegion.getBiome` finds at a block: the zoomed lookup over the noise biomes,
/// read from the chunks that have them (`ChunkAccess.getNoiseBiome`), computed for the others.
fn zoomed_biome_at(level: &RegionLevel, x: i32, y: i32, z: i32) -> u16 {
    if let Some(p) = &level.env.pipeline {
        let world = p.world().clone();
        // (`ChunkAccess.getNoiseBiome` clamps the height to the chunk's.)
        let (lo, hi) = (level.env.min_y >> 2, ((level.env.min_y + level.env.height) >> 2) - 1);
        return kiln_worldgen::generator::zoomed_biome(world.generator.zoom_seed, x, y, z, &mut |qx, qy, qz| {
            let qy = qy.clamp(lo, hi);
            match level.cells.chunk(ChunkPos::new(qx >> 2, qz >> 2)) {
                Some(chunk) => stored_quart_biome(chunk, level.env.min_y, qx, qy, qz),
                // (As `to_chunk` stores it: the network id of the generator's biome.)
                None => {
                    let i = unloaded_noise_biome(&world, qx, qy, qz);
                    kiln_data::synced_id("minecraft:worldgen/biome", &world.generator.biomes[i as usize].name).unwrap_or(0) as u16
                }
            }
        });
    }
    biome_at(level, KBlockPos::new(x, y, z))
}

/// The biome a chunk that is not loaded stores at a quart, computed as its generation did. The
/// neighbours of new chunks are asked for again by the next new chunks, so each thread keeps the
/// last few it computed (up to [`UNLOADED_BIOME_CHUNKS`], then it starts over).
fn unloaded_noise_biome(world: &std::sync::Arc<kiln_worldgen::Worldgen>, qx: i32, qy: i32, qz: i32) -> u16 {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Option<(usize, usize, kiln_worldgen::generator::GenScratch)>> = const { RefCell::new(None) };
    }
    let generator = std::sync::Arc::as_ptr(world) as usize;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let fresh = !matches!(&*c, Some((g, n, _)) if *g == generator && *n < UNLOADED_BIOME_CHUNKS);
        if fresh {
            *c = Some((generator, 0, kiln_worldgen::generator::GenScratch::default()));
        }
        let (_, n, gs) = c.as_mut().unwrap();
        *n += 1;
        gs.noise_biome(&world.generator, qx, qy, qz)
    })
}

/// Chunks of biomes a thread keeps for [`unloaded_noise_biome`] (3 KB each) before it starts over.
const UNLOADED_BIOME_CHUNKS: usize = 64;

/// A loaded chunk's stored biome at a quart (`qy` within the chunk).
fn stored_quart_biome(chunk: &kiln_world::chunk::Chunk, min_y: i32, qx: i32, qy: i32, qz: i32) -> u16 {
    let ry = qy - (min_y >> 2);
    match chunk.sections.get((ry >> 2) as usize).map(|s| &s.biomes) {
        Some(kiln_world::section::Biomes::Single(b)) => *b,
        Some(kiln_world::section::Biomes::Cells(cells)) => cells[(((ry & 3) << 4) | ((qz & 3) << 2) | (qx & 3)) as usize],
        None => 0,
    }
}

/// `NaturalSpawner.getTopNonCollidingPos`.
fn top_non_colliding(level: &RegionLevel, kind: MobKind, x: i32, z: i32) -> KBlockPos {
    use kiln_entity::mob::ext::Placement;
    let env = level.env;
    // `SpawnPlacements.getHeightmapType`: ocelots and parrots stand on leaves.
    let leaves = matches!(kind, MobKind::Ocelot | MobKind::Parrot);
    let h = level
        .cells
        .chunk(ChunkPos::of_block(x, z))
        .map_or(env.min_y, |ch| ch.column_height((x & 15) as usize, (z & 15) as usize, |s| if leaves { kiln_data::block_props::motion_blocking(s) } else { kiln_data::block_props::motion_blocking_no_leaves(s) }));
    let mut pos = KBlockPos::new(x, h, z);
    if env.dim == crate::NETHER_ID {
        loop {
            pos = pos.below();
            if kiln_data::blocks_types::is_air(level.block(pos)) {
                break;
            }
        }
        loop {
            pos = pos.below();
            if !(kiln_data::blocks_types::is_air(level.block(pos)) && pos.y > env.min_y) {
                break;
            }
        }
    }
    let placement = kind.ext().map_or(Placement::OnGround, |k| k.placement());
    if placement == Placement::OnGround {
        let below = pos.below();
        if kiln_entity::mob::path::pathfindable_land(level.block(below)) {
            return below;
        }
    }
    pos
}

fn initial_chunk(level: &RegionLevel, table: &SpawnTable, c: ChunkPos, spawns: &mut Vec<Spawn>) {
    use crate::entities::Body;
    let env = level.env;
    let (min_x, min_z) = (c.x * 16, c.z * 16);
    // `region.getBiome(center.getWorldPosition().atY(region.getMaxY()))`.
    let biome = zoomed_biome_at(level, min_x, env.min_y + env.height - 1, min_z);
    let list = table.list(biome, Category::Creature);
    if list.is_empty() {
        return;
    }
    let probability = table.world_gen_probability(biome);
    // `WorldgenRandom` with `setDecorationSeed(seed, minX, minZ)`.
    let mut r = kiln_worldgen::random::WorldgenRandom::legacy(0);
    r.set_decoration_seed(env.seed, min_x, min_z);
    // `WorldGenRegion.getRandom()`: the region's positional random, which `finalizeSpawn` draws from.
    let mut region_r: Box<dyn RandomSource> = match &env.pipeline {
        Some(p) => Box::new(p.world().generator.region_random.at(min_x, 0, min_z)),
        None => Box::new(LegacyRandom::new(env.seed ^ ((c.x as i64) << 32) ^ c.z as i64)),
    };
    let moon = moon_brightness(env.mobs.day_time);
    let mut made = 0u64;
    while r.next_float() < probability {
        let Some(d) = pick(list, &mut r) else { continue };
        let count = if d.constant { d.min } else { r.next_int_bounded(d.max - d.min + 1) + d.min };
        let mut group = mob::GroupData::default();
        let mut x = min_x + r.next_int_bounded(16);
        let mut z = min_z + r.next_int_bounded(16);
        let (start_x, start_z) = (x, z);
        for _ in 0..count {
            let mut spawned = false;
            let mut attempt = 0;
            while !spawned && attempt < 4 {
                attempt += 1;
                let Some(kind) = d.kind else {
                    // (A type Kiln does not simulate: the position still moves on.)
                    adjust(&mut r, &mut x, &mut z, (min_x, min_z), (start_x, start_z));
                    continue;
                };
                let top = top_non_colliding(level, kind, x, z);
                if placement_ok(level, top, kind) {
                    let t = kiln_data::entities::by_name(kind.type_name()).unwrap();
                    let w = t.width as f64;
                    let px = (x as f64).clamp(min_x as f64 + w, min_x as f64 + 16.0 - w);
                    let pz = (z as f64).clamp(min_z as f64 + w, min_z as f64 + 16.0 - w);
                    if !no_collision(level, [px, top.y as f64, pz], t.width, t.height) {
                        continue;
                    }
                    let at = KBlockPos::new(px.floor() as i32, top.y, pz.floor() as i32);
                    if !check_spawn_rules(level, at, kind, &mut LegacyRandom::new(0)) {
                        continue;
                    }
                    let yaw = r.next_float() * 360.0;
                    // `Mob.checkSpawnRules` (the walk target value) and `checkSpawnObstruction`.
                    // (`WorldGenRegion.getSkyDarken` is 0.)
                    let magic = { let f = level.raw_brightness(at, 0) as f32 / 15.0; f / (4.0 - 3.0 * f) };
                    let walk = if kind.is_animal() {
                        if kiln_data::blocks_types::block_of(level.block(at.below())).name == "minecraft:grass_block" { 10.0 } else { magic - 0.5 }
                    } else {
                        0.0
                    };
                    let ignores_light = kind.ext().is_some_and(|k| k.spawn_ignores_light());
                    let liquid_ok = kind.ext().is_some_and(|k| k.spawn_in_liquids());
                    let fits = !(walk < 0.0 && !ignores_light) && (liquid_ok || !contains_liquid(level, [px, top.y as f64, pz], t.width, t.height));
                    if fits {
                        let mut ctx = crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, moon);
                        ctx.biome = Some(biome as i32);
                        made += 1;
                        let seed = (env.seed as u64 ^ (c.x as u32 as u64) << 20 ^ (c.z as u32 as u64) << 40 ^ made.wrapping_mul(0x9E37_79B9_7F4A_7C15)) as i64;
                        let mut e = mob::new(kind, 0, 0, seed);
                        e.set_pos(kiln_entity::math::Vec3::new(px, top.y as f64, pz));
                        e.y_rot = yaw;
                        e.x_rot = 0.0;
                        e.set_old_pos_and_rot();
                        if let Some(m) = mob::data_mut(&mut e) {
                            m.y_head_rot = yaw;
                            m.y_body_rot = yaw;
                            m.y_head_rot_o = yaw;
                            m.y_body_rot_o = yaw;
                        }
                        mob::finalize_spawn(&mut e, &mut *region_r, &ctx, &mut group, false);
                        let companions = std::mem::take(&mut group.companions);
                        let chicken = group.nearby_chicken;
                        let body = if companions.is_empty() && !chicken { Body::Ready(Box::new(e)) } else { Body::Stacked(Box::new(e), companions, false, chicken) };
                        if let Some(log) = crate::testing::INITIAL_LOG.lock().unwrap().as_mut() {
                            log.push(((c.x, c.z), t.name, [px, top.y as f64, pz], yaw));
                        }
                        spawns.push(Spawn { kind: t, pos: [px, top.y as f64, pz], vel: [0.0; 3], body });
                        spawned = true;
                    }
                }
                adjust(&mut r, &mut x, &mut z, (min_x, min_z), (start_x, start_z));
            }
        }
    }
}

/// The step to the next place of a group: a little way from the last, kept inside the chunk.
fn adjust(r: &mut dyn RandomSource, x: &mut i32, z: &mut i32, min: (i32, i32), start: (i32, i32)) {
    *x += r.next_int_bounded(5) - r.next_int_bounded(5);
    *z += r.next_int_bounded(5) - r.next_int_bounded(5);
    while *x < min.0 || *x >= min.0 + 16 || *z < min.1 || *z >= min.1 + 16 {
        *x = start.0 + r.next_int_bounded(5) - r.next_int_bounded(5);
        *z = start.1 + r.next_int_bounded(5) - r.next_int_bounded(5);
    }
}

/// `containsAnyLiquid` over the mob's box (`checkSpawnObstruction`).
fn contains_liquid(level: &RegionLevel, pos: [f64; 3], w: f32, h: f32) -> bool {
    let (lo, hi) = aabb(pos, w, h);
    for x in lo[0].floor() as i32..hi[0].ceil() as i32 {
        for y in lo[1].floor() as i32..hi[1].ceil() as i32 {
            for z in lo[2].floor() as i32..hi[2].ceil() as i32 {
                if kiln_data::blocks_types::has_fluid(level.block(KBlockPos::new(x, y, z))) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod potential_tests {
    /// `PotentialCalculator.getPotentialEnergyChange` against `MobSpawnCost.energyBudget` as `canSpawn` asks it.
    fn can_spawn(charges: &[([i32; 3], f64)], pos: [i32; 3], charge: f64, budget: f64) -> bool {
        let mut sum = 0.0f64;
        for (p, c) in charges {
            let d2 = ((pos[0] - p[0]) as f64).powi(2) + ((pos[1] - p[1]) as f64).powi(2) + ((pos[2] - p[2]) as f64).powi(2);
            sum += if d2 == 0.0 { f64::INFINITY } else { c / d2.sqrt() };
        }
        sum * charge <= budget
    }

    #[test]
    fn a_warped_forest_enderman_keeps_its_distance() {
        // warped_forest: enderman charge 1.0, budget 0.12: one enderman 9 blocks away gives 1/9 = 0.111 (allowed), 8 blocks 0.125 (not)
        let one = [([0, 64, 0], 1.0)];
        assert!(can_spawn(&[], [3, 64, 0], 1.0, 0.12), "nothing near");
        assert!(can_spawn(&one, [9, 64, 0], 1.0, 0.12));
        assert!(!can_spawn(&one, [8, 64, 0], 1.0, 0.12));
        assert!(!can_spawn(&one, [0, 64, 0], 1.0, 0.12), "the very block: infinite");
        // two endermen add up: 1/12 + 1/12 = 0.1667 > 0.12 although each alone is far enough
        assert!(!can_spawn(&[([0, 64, 0], 1.0), ([24, 64, 0], 1.0)], [12, 64, 0], 1.0, 0.12));
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;
    use kiln_world::chunk::Chunk;
    use kiln_world::section::Section;

    /// wp77: a pack start's highest y is the first free block above the column (`chunk.getHeight(WORLD_SURFACE) + 1`, where `getHeight` is the top block):
    /// on the nether roof, bedrock up to y 127, 128 and not 129; above a platform at 128, 129. Found with the lock-step of the gold farm (starts at
    /// y 129 in every column of the roof were twice as many as vanilla's: they drift onto a platform and spawn).
    #[test]
    fn a_start_reaches_no_higher_than_the_first_free_block_above_the_column() {
        let (air, bedrock, gold) = (kiln_data::blocks::default_state::AIR, kiln_data::blocks::default_state::BEDROCK, kiln_data::blocks::default_state::STONE);
        let mut chunk = Chunk::new((0..9).map(|_| Section::filled(air, 0)).collect(), 0);
        chunk.set(3, 127, 5, bedrock);
        assert_eq!(start_top(&chunk, 3, 5), 128, "bedrock to 127: the bound is 128");
        chunk.set(4, 128, 5, gold);
        assert_eq!(start_top(&chunk, 4, 5), 129, "a platform at 128: 129");
        assert_eq!(start_top(&chunk, 9, 9), 0, "an empty column: the world's bottom");
    }
}
