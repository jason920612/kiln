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
//! Spawn potentials (`spawn_costs`, soul sand valleys) and structure spawn overrides are not
//! simulated; biomes are read at their stored 4×4×4 cells (vanilla fuzzes the lookup).

use crate::Player;
use crate::blocks::{RegionLevel, Ticking};
use crate::entities::{Entities, Spawn};
use kiln_blocks::{BlockPos as KBlockPos, Level};
use kiln_entity::mob::{self, Category, MobKind};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_world::{Blocks, CellStore, ChunkPos};
use std::collections::HashMap;

/// `MobSpawnSettings.SpawnerData`: a type, its weight and its group size.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpawnerData {
    /// `None`: a type Kiln does not simulate (the group is skipped when picked).
    pub kind: Option<MobKind>,
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
    biomes: HashMap<u16, [Vec<SpawnerData>; N]>,
}

/// The number of spawning categories.
const N: usize = Category::SPAWNING.len();

impl SpawnTable {
    /// Reads `worldgen/biome/*.json` of the datapack at `dir`.
    pub fn load(dir: &std::path::Path) -> Option<SpawnTable> {
        let biome_dir = dir.join("data/minecraft/worldgen/biome");
        let mut t = SpawnTable::default();
        for entry in std::fs::read_dir(&biome_dir).ok()? {
            let path = entry.ok()?.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Some(id) = kiln_data::synced_id("minecraft:worldgen/biome", &format!("minecraft:{name}")) else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let spawns = &json["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"]["spawns_by_category"];
            let list = |cat: &str| -> Vec<SpawnerData> {
                spawns[cat]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| {
                                let kind = MobKind::by_name(s["type"].as_str()?);
                                let weight = s["weight"].as_i64()? as i32;
                                let (min, max, constant) = match &s["count"] {
                                    serde_json::Value::Number(n) => (n.as_i64()? as i32, n.as_i64()? as i32, true),
                                    c => (c["min_inclusive"].as_i64()? as i32, c["max_inclusive"].as_i64()? as i32, false),
                                };
                                Some(SpawnerData { kind, weight, min, max, constant })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            t.biomes.insert(id as u16, Category::SPAWNING.map(|c| list(c.name())));
        }
        Some(t)
    }

    fn list(&self, biome: u16, category: Category) -> &[SpawnerData] {
        self.biomes.get(&biome).map_or(&[], |b| &b[cat_index(category)])
    }
}

/// `WeightedList.getRandom`.
fn pick<'a>(list: &'a [SpawnerData], r: &mut LegacyRandom) -> Option<&'a SpawnerData> {
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
    /// Per player, mobs per category nearby (`LocalMobCapCalculator.MobCounts`).
    local: Vec<[i32; N]>,
    /// Players whose spawning squares (8 chunks around them) overlap form a cluster; each
    /// cluster has vanilla's category cap for its chunks, so how players are grouped into
    /// regions does not matter. Per cluster, mobs per category and the cap.
    counts: Vec<[i32; N]>,
    caps: Vec<[i32; N]>,
    table: &'a SpawnTable,
    /// The distinct chunks players stand in and each one's cluster.
    stands: Vec<ChunkPos>,
    stand_cluster: Vec<usize>,
    /// Players by the chunk they stand in, for distance checks that need only whether some
    /// player is within a radius (a crowd stands in a few chunks).
    by_chunk: std::collections::HashMap<ChunkPos, Vec<usize>>,
}

fn cat_index(c: Category) -> usize {
    CATEGORIES.iter().position(|&x| x == c).unwrap_or(0)
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
        self.stands.iter().position(|pc| (pc.x - c.x).abs() <= 8 && (pc.z - c.z).abs() <= 8).map(|k| self.stand_cluster[k])
    }

    /// Whether some player is within `sqrt(r2)` blocks of (`x`, `y`, `z`): the players of the
    /// chunks that close, each checked exactly as a scan over all players would.
    fn player_within(&self, x: f64, y: f64, z: f64, r2: f64) -> bool {
        // |dx| <= r moves the chunk by at most ceil(r / 16).
        let reach = (r2.sqrt().ceil() as i32 + 15) / 16;
        let c = ChunkPos::of_block(x.floor() as i32, z.floor() as i32);
        (c.x - reach..=c.x + reach).any(|cx| {
            (c.z - reach..=c.z + reach).any(|cz| {
                self.by_chunk.get(&ChunkPos::new(cx, cz)).is_some_and(|ps| {
                    ps.iter().any(|&i| {
                        let p = self.pos[i];
                        (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2) <= r2
                    })
                })
            })
        })
    }

    fn local_ok(&self, c: ChunkPos, cat: Category) -> bool {
        let i = cat_index(cat);
        self.pos.iter().zip(&self.local).any(|(p, n)| close_for_spawning(*p, c) && n[i] < cat.max_instances())
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
        for (p, n) in self.pos.iter().zip(self.local.iter_mut()) {
            if close_for_spawning(*p, c) {
                n[i] += 1;
            }
        }
    }
}

/// One tick of natural spawning in a region (`ServerChunkCache.tickChunks`' spawning part).
pub(crate) fn tick(level: &mut RegionLevel, entities: &Entities, players: &[&mut Player], ticking: &Ticking, spawns: &mut Vec<Spawn>) {
    let env = level.env;
    let rules = env.mobs;
    let Some(table) = env.spawn_table.clone() else { return };
    if !rules.spawn_mobs {
        return;
    }
    let spawn_enemies = rules.difficulty != 0 && rules.spawn_monsters;
    let spawn_persistent = env.game_time % 400 == 0;
    if spawn_enemies {
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
    let roots: Vec<usize> = (0..stands.len()).map(|i| root(&mut parent, i)).collect();
    let mut ids: Vec<usize> = roots.clone();
    ids.sort_unstable();
    ids.dedup();
    let stand_cluster: Vec<usize> = roots.iter().map(|r| ids.binary_search(r).unwrap()).collect();
    let mut by_chunk: std::collections::HashMap<ChunkPos, Vec<usize>> = Default::default();
    for (i, p) in players.iter().enumerate() {
        by_chunk.entry(chunk_of(p)).or_default().push(i);
    }
    // `getNaturalSpawnChunkCount` per cluster: chunks within 8 of its players (chessboard).
    let mut near: Vec<std::collections::HashSet<ChunkPos>> = vec![Default::default(); ids.len()];
    for (k, c) in stands.iter().enumerate() {
        for x in c.x - 8..=c.x + 8 {
            for z in c.z - 8..=c.z + 8 {
                near[stand_cluster[k]].insert(ChunkPos::new(x, z));
            }
        }
    }
    let caps: Vec<[i32; N]> =
        near.iter().map(|n| CATEGORIES.map(|cat| cat.max_instances() * n.len() as i32 / 289)).collect();
    let mut s = Spawner {
        pos: players.clone(),
        local: vec![[0; N]; players.len()],
        counts: vec![[0; N]; ids.len()],
        caps,
        table: &table,
        stands,
        stand_cluster,
        by_chunk,
    };
    // `createState`: mobs per category, persistent ones excluded.
    for e in &entities.list {
        let Some(m) = e.phys.as_ref().and_then(mob::data) else { continue };
        if m.persistence_required || e.removed {
            continue;
        }
        s.add(crate::entities::chunk_of(e.pos), m.kind.category());
    }
    // `getFilteredSpawningCategories`; the global cap is checked per cluster below, with
    // the counts as they were at the start of the tick.
    let categories: Vec<Category> =
        CATEGORIES.into_iter().filter(|c| (spawn_enemies || c.friendly()) && (spawn_persistent || !c.persistent())).collect();
    let start_counts = s.counts.clone();
    if categories.is_empty() {
        return;
    }
    // `collectSpawningChunks`: loaded, ticking chunks with a player within 128 blocks.
    let mut chunks: Vec<(u64, ChunkPos)> = Vec::new();
    level.cells.for_each_cell(&mut |pos, cell| {
        for (c, _) in cell.chunks(pos) {
            if ticking.contains(c) && players.iter().any(|p| close_for_spawning(*p, c)) {
                let mut h = (env.seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.game_time as u64;
                h ^= (c.x as u32 as u64) << 32 | c.z as u32 as u64;
                h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                chunks.push((h ^ (h >> 29), c));
            }
        }
    });
    chunks.sort_unstable();
    for (_, c) in chunks {
        let mut r = chunk_random(env.seed, env.game_time, c);
        for &cat in &categories {
            let global = s.cluster(c).is_some_and(|k| start_counts[k][cat_index(cat)] < s.caps[k][cat_index(cat)]);
            if global && s.local_ok(c, cat) {
                spawn_category_for_chunk(level, &mut s, &mut r, cat, c, ticking, spawns);
            }
        }
    }
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
        let ctx = crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, moon_brightness(env.mobs.day_time));
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
            let fin = crate::mobs::Finalize { ctx, seed, persistent: false };
            spawns.push(crate::mobs::spawn(MobKind::Phantom, [at.x as f64 + 0.5, at.y as f64, at.z as f64 + 0.5], Some(0.0), Some(fin)));
        }
    }
}

/// `spawnCategoryForChunk` + `spawnCategoryForPosition`.
fn spawn_category_for_chunk(
    level: &mut RegionLevel,
    s: &mut Spawner,
    r: &mut LegacyRandom,
    cat: Category,
    c: ChunkPos,
    ticking: &Ticking,
    spawns: &mut Vec<Spawn>,
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
            if data.is_none() {
                match pick(s.table.list(biome, cat), r) {
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
            if !s.table.list(biome, cat).iter().any(|e| *e == d_) {
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
            let mut ctx = crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, moon_brightness(env.mobs.day_time));
            ctx.biome = Some(biome as i32);
            let seed = r.next_long();
            let _ = &mut group;
            spawns.push(crate::mobs::spawn(kind, [fx, y as f64, fz], Some(yaw), Some(crate::mobs::Finalize { ctx, seed, persistent: false })));
            s.add(pc, cat);
            spawned += 1;
            in_group += 1;
            if spawned >= kind.ext().map_or(4, |k| k.max_spawn_cluster()) {
                return;
            }
            let _ = in_group;
        }
    }
}

/// The biome of the stored 4×4×4 cell holding `pos`.
fn biome_at(level: &RegionLevel, pos: KBlockPos) -> u16 {
    let Some(chunk) = level.cells.chunk(ChunkPos::of_block(pos.x, pos.z)) else { return 0 };
    let rel = pos.y - level.env.min_y;
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
    let sky = kiln_world::light::light_at(&*level.cells, kiln_world::chunk::LightLayer::Sky, pos.x, pos.y, pos.z).map_or(15, i32::from);
    if sky > r.next_int_bounded(32) {
        return false;
    }
    let block = kiln_world::light::light_at(&*level.cells, kiln_world::chunk::LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from);
    if block > 0 {
        return false;
    }
    let light = level.raw_brightness(pos, level.env.mobs.sky_darken);
    if light > r.next_int_bounded(8) {
        return false;
    }
    kiln_entity::mob::path::valid_spawn(below, false)
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
