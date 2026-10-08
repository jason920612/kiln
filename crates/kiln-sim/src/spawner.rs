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
//! Spawn potentials (`spawn_costs`, soul sand valleys) are not simulated; biomes are read at their
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
            if let Some(p) = json["attributes"]["minecraft:gameplay/creature_world_gen_spawn_probability"].as_f64() {
                t.world_gen[id] = p as f32;
            }
        }
        Some(t)
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
        let structures = |c: ChunkPos| level.cells.chunk(c).and_then(|ch| ch.structures.as_deref());
        let bricks = || kiln_data::blocks_types::block_of(level.block(pos.below())).name == "minecraft:nether_bricks";
        self.mobs_in(&structures, bricks, biome, category, [pos.x, pos.y, pos.z])
    }

    /// [`SpawnTable::mobs_at`] over the loaded chunks' `structures` data.
    pub(crate) fn mobs_in<'a, 'c, F>(&'a self, structures: &F, nether_bricks_below: impl FnOnce() -> bool, biome: u16, category: Category, at: [i32; 3]) -> &'a [SpawnerData]
    where
        F: Fn(ChunkPos) -> Option<&'c kiln_proto::nbt::Tag>,
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
    /// Players whose spawning squares (8 chunks around them) overlap form a cluster; each
    /// cluster has vanilla's category cap for its chunks, so how players are grouped into
    /// regions does not matter. Per cluster, mobs per category and the cap.
    counts: Vec<[i32; N]>,
    caps: Vec<[i32; N]>,
    table: &'a SpawnTable,
    /// The distinct chunks players stand in (sorted) and each one's cluster.
    stands: Vec<ChunkPos>,
    stand_cluster: Vec<usize>,
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
    /// The cluster whose spawning squares hold chunk `c` (players within 8 chunks of one
    /// chunk are all in one cluster, so any stand chunk that close tells).
    fn cluster(&self, c: ChunkPos) -> Option<usize> {
        self.stands_near(c, 8).next().map(|k| self.stand_cluster[k])
    }

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
        if let Some(k) = self.cluster(c) {
            self.counts[k][i] += 1;
        }
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
    let players: Vec<[f64; 3]> =
        players.iter().filter(|p| !p.disconnected && !p.dead && p.game_mode != 3).map(|p| p.pos).collect();
    if players.is_empty() {
        return;
    }
    // Clusters of players with overlapping spawning squares: union-find over the distinct
    // chunks players stand in (a crowd shares a few chunks), each player joining its chunk's.
    let chunk_of = |p: &[f64; 3]| ChunkPos::of_block(p[0].floor() as i32, p[2].floor() as i32);
    let mut stands: Vec<ChunkPos> = players.iter().map(chunk_of).collect();
    stands.sort_unstable();
    stands.dedup();
    let mut parent: Vec<usize> = (0..stands.len()).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        parent[i] = r;
        r
    }
    for i in 0..stands.len() {
        for j in i + 1..stands.len() {
            let (a, b) = (stands[i], stands[j]);
            if (a.x - b.x).abs() <= 16 && (a.z - b.z).abs() <= 16 {
                let (ra, rb) = (root(&mut parent, i), root(&mut parent, j));
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let dt = crate::diag::lap("s.a_union", dt);
    let roots: Vec<usize> = (0..stands.len()).map(|i| root(&mut parent, i)).collect();
    let mut ids: Vec<usize> = roots.clone();
    ids.sort_unstable();
    ids.dedup();
    let stand_cluster: Vec<usize> = roots.iter().map(|r| ids.binary_search(r).unwrap()).collect();
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
    // `getNaturalSpawnChunkCount` per cluster: chunks within 8 of its players (chessboard),
    // counted on a bitmap over the cluster's bounds.
    let caps: Vec<[i32; N]> = (0..ids.len())
        .map(|k| {
            let mine = || stands.iter().zip(&stand_cluster).filter(move |(_, sc)| **sc == k).map(|(c, _)| *c);
            let (x0, x1) = (mine().map(|c| c.x).min().unwrap() - 8, mine().map(|c| c.x).max().unwrap() + 8);
            let (z0, z1) = (mine().map(|c| c.z).min().unwrap() - 8, mine().map(|c| c.z).max().unwrap() + 8);
            let w = (x1 - x0 + 1) as usize;
            let mut seen = vec![false; w * (z1 - z0 + 1) as usize];
            for c in mine() {
                for z in c.z - 8..=c.z + 8 {
                    let row = (z - z0) as usize * w;
                    seen[row + (c.x - 8 - x0) as usize..=row + (c.x + 8 - x0) as usize].fill(true);
                }
            }
            let n = seen.iter().filter(|&&b| b).count() as i32;
            CATEGORIES.map(|cat| cat.max_instances() * n / 289)
        })
        .collect();
    let dt = crate::diag::lap("s.b_caps", dt);
    let mut s = Spawner {
        pos: players.clone(),
        local: vec![[0; N]; players.len()],
        mobs: Vec::new(),
        upper: vec![[0; N]; stands.len()],
        known: [false; N],
        room: Default::default(),
        counts: vec![[0; N]; ids.len()],
        caps,
        table: &table,
        stands,
        stand_cluster,
        boxes,
        order,
        starts,
    };
    // `createState`: mobs per category, persistent ones excluded.
    for e in &entities.list {
        let Some(m) = e.phys.as_deref().and_then(mob::data) else { continue };
        if m.persistence_required || e.removed {
            continue;
        }
        s.add(crate::entities::chunk_of(e.pos), m.kind.category());
    }
    // `getFilteredSpawningCategories`; the global cap is checked per cluster below, with
    // the counts as they were at the start of the tick.
    let categories: Vec<Category> = CATEGORIES
        .into_iter()
        .filter(|c| rules.spawn_mobs && (spawn_enemies || c.friendly()) && (spawn_persistent || !c.persistent()))
        .collect();
    let start_counts = s.counts.clone();
    // `ServerChunkCache.tickSpawningChunk`: every ticking chunk with a player near gets one more
    // tick of `InhabitedTime`, whether or not anything may spawn.
    level.cells.for_each_cell_mut(&mut |pos, cell| {
        for (c, chunk) in cell.chunks_mut(pos) {
            if ticking.contains(c) && s.any_close(c) {
                chunk.increment_inhabited_time();
            }
        }
    });
    if categories.is_empty() {
        return;
    }
    let dt = crate::diag::lap("s.c_state", dt);
    // `collectSpawningChunks`: loaded, ticking chunks with a player within 128 blocks.
    let mut chunks: Vec<(u64, ChunkPos)> = Vec::new();
    level.cells.for_each_cell(&mut |pos, cell| {
        for (c, _) in cell.chunks(pos) {
            if ticking.contains(c) && s.any_close(c) {
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
    let speculated: Vec<Option<smallvec::SmallVec<[bool; N]>>> = {
        let (lvl, sp, cats, counts) = (&*level, &s, &categories, &start_counts);
        ctx.map_indexed_with(SPAWN_WINDOW, &chunks, |_, &(_, c)| speculate(lvl, sp, c, cats, counts, ticking))
    };
    let dt = crate::diag::lap("s.spec", dt);
    let mut spawned_any = false;
    for (&(_, c), guess) in chunks.iter().zip(speculated) {
        let global = |s: &Spawner, cat: Category| s.cluster(c).is_some_and(|k| start_counts[k][cat_index(cat)] < s.caps[k][cat_index(cat)]);
        if let Some(said) = guess
            && (!spawned_any || categories.iter().zip(&said).all(|(&cat, &ok)| (global(&s, cat) && s.local_ok(c, cat)) == ok))
        {
            continue;
        }
        let mut r = chunk_random(env.seed, env.game_time, c);
        for &cat in &categories {
            if global(&s, cat) && s.local_ok(c, cat) {
                let mut made = Vec::new();
                spawn_category_for_chunk(level, &s, &mut r, cat, c, ticking, &mut made);
                for (spawn, pc) in made {
                    spawns.push(spawn);
                    s.add(pc, cat);
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
    start_counts: &[[i32; N]],
    ticking: &Ticking,
) -> Option<smallvec::SmallVec<[bool; N]>> {
    let mut r = chunk_random(level.env.seed, level.env.game_time, c);
    let mut said = smallvec::SmallVec::new();
    for &cat in categories {
        let global = s.cluster(c).is_some_and(|k| start_counts[k][cat_index(cat)] < s.caps[k][cat_index(cat)]);
        let ok = global && s.local_ok_now(c, cat)?;
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
            let fin = crate::mobs::Finalize { ctx, seed, persistent: false, natural: false, monsters_disabled: false, camel_space: false };
            spawns.push(crate::mobs::spawn(MobKind::Phantom, [at.x as f64 + 0.5, at.y as f64, at.z as f64 + 0.5], Some(0.0), Some(fin)));
        }
    }
}

/// `spawnCategoryForChunk` + `spawnCategoryForPosition`: what spawns, with the chunk each
/// counts in.
#[inline(never)]
fn spawn_category_for_chunk(
    level: &RegionLevel,
    s: &Spawner,
    r: &mut LegacyRandom,
    cat: Category,
    c: ChunkPos,
    ticking: &Ticking,
    spawns: &mut Vec<(Spawn, ChunkPos)>,
) {
    let env = level.env;
    let min_y = env.min_y;
    // `getRandomPosWithin`.
    let x = c.x * 16 + r.next_int_bounded(16);
    let z = c.z * 16 + r.next_int_bounded(16);
    let Some(chunk) = level.cells.chunk(c) else { return };
    let top = chunk.column_height((x & 15) as usize, (z & 15) as usize, |b| !kiln_data::blocks_types::is_air(b)) + 1;
    let y = r.next_int_bounded(top - min_y + 1) + min_y;
    if y < min_y + 1 {
        return;
    }
    let start = KBlockPos::new(x, y, z);
    if kiln_data::block_logic::is_redstone_conductor(level.block(start)) {
        return;
    }
    let mut spawned = 0;
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
            if pc != c && !ticking.contains(pc) {
                continue;
            }
            let pos = KBlockPos::new(px, y, pz);
            let biome = biome_at(level, pos);
            // What spawns at this very place (looked up once: the pick and `canSpawnMobAt` below
            // ask the same).
            let here = s.table.mobs_at(level, biome, cat, pos);
            if data.is_none() {
                // `getRandomSpawnMobAt`: rivers have most of their ambient water spawns taken away.
                if cat == Category::WaterAmbient && kiln_entity::mob::kinds::slime::biome_in_tag(biome as i32, "minecraft:reduce_water_ambient_spawns") && r.next_float() < 0.98 {
                    break;
                }
                match pick(here, r) {
                    None => break,
                    Some(d) => {
                        let d = d.clone();
                        pack = if d.constant { d.min } else { r.next_int_bounded(d.max - d.min + 1) + d.min };
                        data = Some(d);
                    }
                }
            }
            let d_ = data.clone().unwrap();
            let Some(kind) = d_.kind else { continue };
            // `isValidSpawnPostitionForType`.
            // The nearest player within the despawn distance.
            if !s.player_within(fx, y as f64, fz, (cat.despawn_distance() * cat.despawn_distance()) as f64) {
                continue;
            }
            // `canSpawnMobAt`: what was picked is among what spawns at this very place.
            if !here.iter().any(|e| *e == d_) {
                continue;
            }
            if !placement_ok(level, pos, kind) || !check_spawn_rules(level, pos, kind, r) {
                continue;
            }
            let t = kiln_data::entities::by_name(kind.type_name()).unwrap();
            if !no_collision(level, [fx, y as f64, fz], t.width, t.height) {
                continue;
            }
            let yaw = r.next_float() * 360.0;
            // `isValidPositionForMob`: the walk target value and the spawn obstruction.
            let magic = light_magic(level, pos);
            let walk = if kind.is_animal() {
                if kiln_data::blocks_types::block_of(level.block(pos.below())).name == "minecraft:grass_block" { 10.0 } else { magic - 0.5 }
            } else {
                -(magic - 0.5)
            };
            let ignores_light = kind.ext().is_some_and(|k| k.spawn_ignores_light());
            let liquid_ok = kind.ext().is_some_and(|k| k.spawn_in_liquids());
            if (walk < 0.0 && !ignores_light) || (!liquid_ok && contains_liquid(level, [fx, y as f64, fz], t.width, t.height)) {
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
            let fin = crate::mobs::Finalize { ctx, seed, persistent: false, natural: true, monsters_disabled, camel_space };
            // The caller counts it (nothing below reads the counts).
            spawns.push((crate::mobs::spawn(kind, [fx, y as f64, fz], Some(yaw), Some(fin)), pc));
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
    let Some(k) = kind.ext() else { return spawn_position_ok(level, pos, kind.is_animal()) };
    let fluid = |p: KBlockPos| kiln_entity::physics::fluid_state(level.block(p));
    match k.placement() {
        Placement::OnGround => spawn_position_ok(level, pos, kind.is_animal()),
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
        biome_at(self.0, KBlockPos::new(pos.x, pos.y, pos.z)) as i32
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

/// `SpawnPlacementTypes.ON_GROUND.isSpawnPositionOk`.
fn spawn_position_ok(level: &RegionLevel, pos: KBlockPos, animal: bool) -> bool {
    let below = level.block(pos.below());
    kiln_entity::mob::path::valid_spawn(below, animal)
        && kiln_entity::mob::path::valid_empty_spawn(level.block(pos), animal)
        && kiln_entity::mob::path::valid_empty_spawn(level.block(pos.above()), animal)
}

/// `SpawnPlacements.checkSpawnRules` for the simulated types: animals need light and a
/// spawnable block below, monsters darkness (`Monster.isDarkEnoughToSpawn`) and a valid spawn
/// block below.
fn check_spawn_rules(level: &RegionLevel, pos: KBlockPos, kind: MobKind, r: &mut LegacyRandom) -> bool {
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

fn light_magic(level: &RegionLevel, pos: KBlockPos) -> f32 {
    let f = level.raw_brightness(pos, level.env.mobs.sky_darken) as f32 / 15.0;
    f / (4.0 - 3.0 * f)
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
    crate::testing::trace(&format!("initial_mobs {} pending, spawn_mobs {} table {}", pending.len(), env.mobs.spawn_mobs, env.spawn_table.is_some()));
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

/// The biome `WorldGenRegion.getBiome` finds at a block: the zoomed lookup over the noise biomes.
fn zoomed_biome_at(level: &RegionLevel, x: i32, y: i32, z: i32) -> u16 {
    if let Some(p) = &level.env.pipeline {
        let world = p.world().clone();
        let mut gs = kiln_worldgen::generator::GenScratch::default();
        // (`ChunkAccess.getNoiseBiome` clamps the height to the chunk's.)
        let (lo, hi) = (level.env.min_y >> 2, ((level.env.min_y + level.env.height) >> 2) - 1);
        return kiln_worldgen::generator::zoomed_biome(world.generator.zoom_seed, x, y, z, &mut |qx, qy, qz| gs.noise_biome(&world.generator, qx, qy.clamp(lo, hi), qz));
    }
    biome_at(level, KBlockPos::new(x, y, z))
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
    let name = |b: u16| kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome").map_or("?", |(_, e)| e[b as usize]);
    crate::testing::trace(&format!("initial_chunk {c:?} biome {biome} {} mid {} creatures {} p {}", name(biome), name(zoomed_biome_at(level, min_x + 8, 70, min_z + 8)), list.len(), table.world_gen_probability(biome)));
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
    let top_y = env.min_y + env.height - 1;
    let _ = top_y;
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
                let dbg = std::env::var_os("KILN_INITIAL_DEBUG").is_some();
                if dbg {
                    use std::io::Write as _;
                    let line = format!("INITIAL chunk ({}, {}) {:?} at {x},{z} top {top:?} placement {} below {} at {} sky {} bright {}", c.x, c.z, kind, placement_ok(level, top, kind), kiln_data::blocks_types::block_of(level.block(top.below())).name, kiln_data::blocks_types::block_of(level.block(top)).name, level.raw_brightness(top, 0), level.raw_brightness(top.above(), 0)) + &format!(" above {} above2 {}", kiln_data::blocks_types::block_of(level.block(top.above())).name, kiln_data::blocks_types::block_of(level.block(top.above().above())).name);
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/initial_debug.txt") {
                        let _ = writeln!(f, "{line}");
                    }
                }
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
                    let magic = light_magic(level, at);
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
