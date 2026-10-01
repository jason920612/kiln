//! The game test framework behind `/test` (vanilla's `net.minecraft.gametest.framework`,
//! `TestInstanceBlockEntity`, `TestBlock`): test instances and environments from the data
//! packs, test instance blocks in the world, and the runner that spawns test structures on a
//! grid, activates an environment, ticks the tests and reports.
//!
//! What runs: `minecraft:function` tests (the only registered test function is
//! `minecraft:always_pass`) and `minecraft:block_based` tests. Block-based tests start their
//! test block, and an accept, fail or log test block fires when it touches a powered start
//! block or a redstone block; there is no redstone circuit here, so tests that need wires,
//! pistons or observers to carry the signal time out. Structures are placed from templates
//! (entities in them are not); the environment types are `all_of`, `clock_time`,
//! `difficulty`, `function`, `game_rules` and `weather` (`timeline_attributes` is accepted and
//! has no effect).

use crate::commands::CommandSource;
use crate::{DIMENSIONS, DimId, Sim, dim_id};
use kiln_command::host::{TestCommand, TestSelection};
use kiln_command::{GameRuleValue, Host, Identifier, Text, TimeAction, UpdateFlags, Weather, tr};
use kiln_loot::Json;
use kiln_proto::nbt::Tag;
use kiln_world::Blocks;
use kiln_world::spawn::LoadChunks;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tracing::{error, info, warn};

mod command;
mod runner;

type Pos = [i32; 3];

/// `TestCommand.TEST_NEARBY_SEARCH_RADIUS` and `TEST_FULL_SEARCH_RADIUS`.
const NEARBY_RADIUS: i32 = 15;
const FULL_RADIUS: i32 = 250;
/// `GameTestBatchFactory.MAX_TESTS_PER_BATCH`.
const BATCH_SIZE: usize = 50;
/// `TestCommand.VERIFY_TEST_BATCH_SIZE`, `VERIFY_TEST_GRID_AXIS_SIZE`.
const VERIFY_BATCH: usize = 100;
const VERIFY_PER_ROW: i32 = 10;
/// `commands.test.error.too_large`: structures may not be bigger than this along an axis.
const MAX_SIZE: i32 = 48;
/// `StructureGridSpawner`: blocks between structures in a row and between rows.
const COLUMN_GAP: i32 = 5;
const ROW_GAP: i32 = 6;
/// `TestInstanceBlockEntity.STRUCTURE_OFFSET`.
const STRUCTURE_OFFSET: Pos = [0, 1, 1];
const TEST_FUNCTION_ALWAYS_PASS: &str = "minecraft:always_pass";

// ---- definitions -------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    /// `minecraft:function`: a registered test function.
    Function(String),
    /// `minecraft:block_based`: run by the test blocks in the structure.
    BlockBased,
}

/// A `minecraft:test_instance` (`TestData` and its type).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TestDef {
    pub kind: Kind,
    /// The environment's registry id; inline definitions are `[unregistered]` with a number.
    pub environment: String,
    pub dimension: String,
    pub structure: String,
    pub max_ticks: i32,
    pub setup_ticks: i32,
    pub required: bool,
    /// Extra rotation in quarter turns clockwise.
    pub rotation: u8,
    pub manual_only: bool,
    pub max_attempts: i32,
    pub required_successes: i32,
    pub sky_access: bool,
    pub padding: i32,
}

/// A `minecraft:test_environment` (`TestEnvironmentDefinition`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Env {
    AllOf(Vec<Env>),
    ClockTime { clock: String, time: i32 },
    Difficulty(String),
    Function { setup: Option<String>, teardown: Option<String> },
    GameRules(Vec<(String, GameRuleValue)>),
    Timelines,
    Weather(Weather),
}

/// What data packs define for tests.
#[derive(Default, Clone)]
pub(crate) struct Defs {
    pub tests: BTreeMap<String, TestDef>,
    pub envs: BTreeMap<String, Env>,
}

fn full_id(namespace: &str, rel: &str) -> String {
    format!("{namespace}:{rel}")
}

fn rotation_by_name(name: &str) -> Option<u8> {
    Some(match name {
        "none" => 0,
        "clockwise_90" => 1,
        "180" | "clockwise_180" => 2,
        "counterclockwise_90" => 3,
        _ => return None,
    })
}

fn rotation_name(steps: u8) -> &'static str {
    ["none", "clockwise_90", "180", "counterclockwise_90"][(steps % 4) as usize]
}

fn namespaced(id: &str) -> String {
    if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") }
}

impl Env {
    fn parse(json: &Json) -> Result<Env, String> {
        let ty = json.get("type").and_then(Json::as_str).ok_or("missing type")?;
        Ok(match namespaced(ty).as_str() {
            "minecraft:all_of" => {
                let list = json.get("definitions").and_then(Json::as_array).ok_or("missing definitions")?;
                Env::AllOf(list.iter().map(Env::parse).collect::<Result<_, _>>()?)
            }
            "minecraft:clock_time" => Env::ClockTime {
                clock: namespaced(json.get("clock").and_then(Json::as_str).ok_or("missing clock")?),
                time: json.get("time").and_then(Json::as_i32).ok_or("missing time")?,
            },
            "minecraft:difficulty" => {
                let d = json.get("difficulty").and_then(Json::as_str).ok_or("missing difficulty")?;
                if !["peaceful", "easy", "normal", "hard"].contains(&d) {
                    return Err(format!("unknown difficulty {d}"));
                }
                Env::Difficulty(d.to_owned())
            }
            "minecraft:function" => Env::Function {
                setup: json.get("setup").and_then(Json::as_str).map(namespaced),
                teardown: json.get("teardown").and_then(Json::as_str).map(namespaced),
            },
            "minecraft:game_rules" => {
                let rules = json.get("rules").and_then(Json::as_object).ok_or("missing rules")?;
                let mut out = Vec::new();
                for (name, value) in rules {
                    let id = namespaced(name);
                    let v = match (value.as_bool(), value.as_i32()) {
                        (Some(b), _) => GameRuleValue::Bool(b),
                        (_, Some(i)) => GameRuleValue::Int(i),
                        _ => return Err(format!("bad value for game rule {id}")),
                    };
                    out.push((id, v));
                }
                Env::GameRules(out)
            }
            "minecraft:timeline_attributes" => Env::Timelines,
            "minecraft:weather" => Env::Weather(match json.get("weather").and_then(Json::as_str) {
                Some("clear") => Weather::Clear,
                Some("rain") => Weather::Rain,
                Some("thunder") => Weather::Thunder,
                _ => return Err("weather must be clear, rain or thunder".into()),
            }),
            other => return Err(format!("unknown environment type {other}")),
        })
    }
}

impl Defs {
    /// `TestInstance` and `TestEnvironment` registries from the packs, later packs replacing
    /// earlier ones; a definition that does not decode is logged and left out.
    pub(crate) fn load(roots: &[PathBuf]) -> Defs {
        use crate::datapacks::pack_files;
        let mut defs = Defs::default();
        let mut env_json: BTreeMap<String, Json> = BTreeMap::new();
        let mut test_json: BTreeMap<String, Json> = BTreeMap::new();
        for root in roots {
            for (dir, into) in [("test_environment", &mut env_json), ("test_instance", &mut test_json)] {
                for (ns, rel, path) in pack_files(root, dir, ".json") {
                    let id = full_id(&ns, &rel);
                    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Json::parse(&t).map_err(|e| e.to_string())) {
                        Ok(j) => {
                            into.insert(id, j);
                        }
                        Err(e) => error!("Couldn't parse data file {id} from {}: {e}", path.display()),
                    }
                }
            }
        }
        for (id, json) in &env_json {
            match Env::parse(json) {
                Ok(e) => {
                    defs.envs.insert(id.clone(), e);
                }
                Err(e) => error!("Failed to parse test environment {id}: {e}"),
            }
        }
        let mut inline = 0;
        for (id, json) in &test_json {
            match defs.parse_test(json, &mut inline) {
                Ok(t) => {
                    defs.tests.insert(id.clone(), t);
                }
                Err(e) => error!("Failed to parse test instance {id}: {e}"),
            }
        }
        defs
    }

    fn parse_test(&mut self, json: &Json, inline: &mut usize) -> Result<TestDef, String> {
        let ty = namespaced(json.get("type").and_then(Json::as_str).ok_or("missing type")?);
        let kind = match ty.as_str() {
            "minecraft:function" => {
                let f = namespaced(json.get("function").and_then(Json::as_str).ok_or("missing function")?);
                if f != TEST_FUNCTION_ALWAYS_PASS {
                    return Err(format!("Unknown registry key in ResourceKey[minecraft:root / minecraft:test_function]: {f}"));
                }
                Kind::Function(f)
            }
            "minecraft:block_based" => Kind::BlockBased,
            other => return Err(format!("Unknown registry key in ResourceKey[minecraft:root / minecraft:test_instance_type]: {other}")),
        };
        let environment = match json.get("environment").ok_or("missing environment")? {
            Json::Str(id) => {
                let id = namespaced(id);
                if !self.envs.contains_key(&id) {
                    return Err(format!("Unknown registry key in ResourceKey[minecraft:root / minecraft:test_environment]: {id}"));
                }
                id
            }
            inline_def => {
                let env = Env::parse(inline_def)?;
                *inline += 1;
                let id = format!("[unregistered]#{inline}");
                self.envs.insert(id.clone(), env);
                id
            }
        };
        let int = |name: &str, default: i32, min: i32| -> Result<i32, String> {
            match json.get(name) {
                None => Ok(default),
                Some(v) => match v.as_i32() {
                    Some(n) if n >= min => Ok(n),
                    _ => Err(format!("{name} must be an integer of at least {min}")),
                },
            }
        };
        let flag = |name: &str, default: bool| json.get(name).and_then(Json::as_bool).unwrap_or(default);
        Ok(TestDef {
            kind,
            environment,
            dimension: json.get("dimension").and_then(Json::as_str).map_or("minecraft:overworld".to_owned(), namespaced),
            structure: namespaced(json.get("structure").and_then(Json::as_str).ok_or("missing structure")?),
            max_ticks: {
                let n = int("max_ticks", 0, 1)?;
                if json.get("max_ticks").is_none() {
                    return Err("missing max_ticks".into());
                }
                n
            },
            setup_ticks: int("setup_ticks", 0, 0)?,
            required: flag("required", true),
            rotation: match json.get("rotation").and_then(Json::as_str) {
                None => 0,
                Some(r) => rotation_by_name(r).ok_or_else(|| format!("unknown rotation {r}"))?,
            },
            manual_only: flag("manual_only", false),
            max_attempts: int("max_attempts", 1, 1)?,
            required_successes: int("required_successes", 1, 1)?,
            sky_access: flag("sky_access", false),
            padding: int("padding", 0, 0)?,
        })
    }

    /// Ids of the test instances, in registry order.
    pub(crate) fn test_ids(&self) -> Vec<String> {
        self.tests.keys().cloned().collect()
    }

    fn env_name(&self, id: &str) -> String {
        if id.starts_with("[unregistered]") { "[unregistered]".to_owned() } else { id.to_owned() }
    }
}

// ---- the test instance block ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Cleared,
    Running,
    Finished,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Cleared => "cleared",
            Status::Running => "running",
            Status::Finished => "finished",
        }
    }
}

/// `TestInstanceBlockEntity.Data` and the error markers.
#[derive(Clone, Debug)]
struct BlockData {
    test: Option<String>,
    size: [i32; 3],
    rotation: u8,
    ignore_entities: bool,
    status: Status,
    error: Option<Tag>,
    errors: Vec<(Pos, Tag)>,
}

impl BlockData {
    fn empty() -> BlockData {
        BlockData { test: None, size: [0; 3], rotation: 0, ignore_entities: false, status: Status::Cleared, error: None, errors: Vec::new() }
    }

    fn read(be: &Tag) -> BlockData {
        let mut d = BlockData::empty();
        if let Some(data) = be.get("data") {
            d.test = data.get("test").and_then(Tag::as_str).map(str::to_owned);
            if let Some(Tag::IntArray(v)) = data.get("size")
                && v.len() == 3
            {
                d.size = [v[0], v[1], v[2]];
            }
            d.rotation = data.get("rotation").and_then(Tag::as_str).and_then(rotation_by_name).unwrap_or(0);
            d.ignore_entities = data.get("ignore_entities").and_then(Tag::as_i64).is_some_and(|v| v != 0);
            d.status = match data.get("status").and_then(Tag::as_str) {
                Some("running") => Status::Running,
                Some("finished") => Status::Finished,
                _ => Status::Cleared,
            };
            d.error = data.get("error_message").cloned();
        }
        if let Some(Tag::List(list)) = be.get("errors") {
            for m in list {
                let m = m.unwrap_list_element();
                if let (Some(Tag::IntArray(p)), Some(text)) = (m.get("pos"), m.get("text"))
                    && p.len() == 3
                {
                    d.errors.push(([p[0], p[1], p[2]], text.clone()));
                }
            }
        }
        d
    }

    fn fields(&self) -> Vec<(String, Tag)> {
        let mut data = Vec::new();
        if let Some(t) = &self.test {
            data.push(("test".to_owned(), Tag::String(t.clone())));
        }
        data.push(("size".to_owned(), Tag::IntArray(self.size.to_vec())));
        data.push(("rotation".to_owned(), Tag::String(rotation_name(self.rotation).to_owned())));
        data.push(("ignore_entities".to_owned(), Tag::Byte(self.ignore_entities as i8)));
        data.push(("status".to_owned(), Tag::String(self.status.name().to_owned())));
        if let Some(e) = &self.error {
            data.push(("error_message".to_owned(), e.clone()));
        }
        let mut out = vec![("data".to_owned(), Tag::Compound(data))];
        if !self.errors.is_empty() {
            let list = self
                .errors
                .iter()
                .map(|(p, t)| Tag::Compound(vec![("pos".to_owned(), Tag::IntArray(p.to_vec())), ("text".to_owned(), t.clone())]))
                .collect();
            out.push(("errors".to_owned(), Tag::List(list)));
        }
        out
    }
}

/// A box of blocks, both corners included.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    min: Pos,
    max: Pos,
}

impl Bounds {
    fn from_corners(a: Pos, b: Pos) -> Bounds {
        Bounds { min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])], max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])] }
    }

    fn inflated(self, by: i32) -> Bounds {
        Bounds { min: self.min.map(|v| v - by), max: self.max.map(|v| v + by) }
    }

    fn contains(self, p: Pos) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }

    fn positions(self) -> impl Iterator<Item = Pos> {
        let (min, max) = (self.min, self.max);
        (min[0]..=max[0]).flat_map(move |x| (min[1]..=max[1]).flat_map(move |y| (min[2]..=max[2]).map(move |z| [x, y, z])))
    }

    /// The chunks it touches.
    fn chunks(self) -> impl Iterator<Item = [i32; 2]> {
        let (min, max) = (self.min, self.max);
        ((min[0] >> 4)..=(max[0] >> 4)).flat_map(move |cx| ((min[2] >> 4)..=(max[2] >> 4)).map(move |cz| [cx, cz]))
    }

    /// The box as `AABB.of(BoundingBox)` (upper corner one past).
    fn aabb(self) -> ([f64; 3], [f64; 3]) {
        (self.min.map(f64::from), self.max.map(|v| f64::from(v + 1)))
    }
}

/// The placement of a test structure around its test instance block.
#[derive(Clone, Copy, Debug)]
struct Layout {
    block: Pos,
    size: [i32; 3],
    /// `TestInstanceBlockEntity.getRotation`: the test's own turn plus the block's.
    rotation: u8,
    padding: i32,
}

impl Layout {
    /// `getStructurePos`.
    fn structure_pos(&self) -> Pos {
        let p = self.padding;
        [self.block[0] + p + STRUCTURE_OFFSET[0], self.block[1] + p + STRUCTURE_OFFSET[1], self.block[2] + p + STRUCTURE_OFFSET[2]]
    }

    /// `getTransformedSize`: quarter turns swap the horizontal axes.
    fn transformed_size(&self) -> Pos {
        let [x, y, z] = self.size;
        if self.rotation % 2 == 1 { [z, y, x] } else { [x, y, z] }
    }

    /// `getStructureBoundingBox`.
    fn structure_bounds(&self) -> Bounds {
        let pos = self.structure_pos();
        let s = self.transformed_size();
        Bounds::from_corners(pos, [pos[0] + s[0] - 1, pos[1] + s[1] - 1, pos[2] + s[2] - 1])
    }

    /// `getTestBoundingBox`.
    fn test_bounds(&self) -> Bounds {
        self.structure_bounds().inflated(self.padding)
    }

    /// `getStartCorner`: where the template's origin goes so it lands inside the bounds.
    fn start_corner(&self) -> Pos {
        let [x, y, z] = self.structure_pos();
        let [sx, _, sz] = self.size;
        match self.rotation % 4 {
            0 => [x, y, z],
            1 => [x + sz - 1, y, z],
            2 => [x + sx - 1, y, z + sz - 1],
            _ => [x, y, z + sx - 1],
        }
    }
}

// ---- the runner ----------------------------------------------------------------------------

/// Why a test failed: a message shown to players and the tick it happened on.
#[derive(Clone, Debug)]
struct Failure {
    /// `GameTestAssertException.getDescription`: `test.error.tick` of the message and tick;
    /// for other exceptions their message.
    text: Text,
    /// A position to mark in the structure, if the failure names one.
    at: Option<Pos>,
    /// A failed assertion (the description is shown as it is) rather than an exception.
    assertion: bool,
}

/// `GameTestInfo`.
struct Info {
    test: String,
    def: TestDef,
    /// `extraRotation`, in quarter turns.
    extra_rotation: u8,
    dim: DimId,
    /// Tries the run may make (`RetryOptions`): below 1 without limit.
    tries: i32,
    halt_on_failure: bool,
    block: Option<Pos>,
    placed: bool,
    tick: i32,
    started: bool,
    done: bool,
    error: Option<Failure>,
    /// The exception's class, for failures that are not assertions.
    error_kind: &'static str,
    /// `ReportGameListener` state shared by the reruns of a test.
    report: usize,
    /// The test blocks a block-based test found when it started (position, mode).
    test_blocks: Vec<(Pos, String)>,
    /// Test blocks that fired since the last check (`TestBlockEntity.triggered`).
    triggered: Vec<Pos>,
    /// Test blocks that are powered (`TestBlockEntity.powered`), and the powered start blocks.
    powered: Vec<Pos>,
    starts_powered: Vec<Pos>,
    started_at: std::time::Instant,
    ran_ms: u64,
}

#[derive(Default, Clone, Copy)]
struct Report {
    attempts: i32,
    successes: i32,
}

/// One row of tests in a dimension (`StructureGridSpawner.DimensionGridState`).
struct Grid {
    first: Pos,
    next: Pos,
    row_min: [f64; 3],
    row_max: [f64; 3],
    in_row: i32,
    last_batch: Vec<usize>,
}

struct Batch {
    env: String,
    dim: DimId,
    index: usize,
    infos: Vec<usize>,
}

/// An activated environment: what to restore when its batch is over.
struct Activation {
    env: Env,
    dim: DimId,
    saved: Vec<Saved>,
}

enum Saved {
    Rules(String, GameRuleValue),
    Difficulty(String),
    Time(String, i32),
    Weather,
    Teardown(String),
}

pub(crate) struct Runner {
    origin: Origin,
    infos: Vec<Info>,
    batches: Vec<Batch>,
    /// The batch running (`runBatch(index)` was called for `current`).
    current: usize,
    stopped: bool,
    env: Option<Activation>,
    reports: Vec<Report>,
    per_row: i32,
    grid_clears_on_batch: bool,
    halt_on_error: bool,
    clear_between_batches: bool,
    batch_size: usize,
    /// Where the grid of a dimension starts (`createTestPositionAround`).
    origins: HashMap<DimId, Pos>,
    grids: HashMap<DimId, Grid>,
    /// The `MultipleTestTracker` of the whole run (the summary) and of the running batch.
    tracked: Vec<usize>,
    batch_tracked: Vec<usize>,
    scheduled_reruns: Vec<usize>,
    /// The dimensions of the tests that finished (`TestSummaryDisplayer.dimensions`).
    summary_dims: Vec<DimId>,
    /// The run is over (a halting failure); it is dropped after the tick.
    terminated: bool,
    /// The infos being ticked.
    ticking: Vec<usize>,
}

/// Where a command's output goes and as whom it ran (`CommandSourceStack`).
#[derive(Clone)]
pub(crate) struct Origin {
    pub source: CommandSource,
    pub stack: kiln_command::SourceStack<Sim>,
}

/// `TestCommand`'s state: the definitions, the runner, the tests that failed last.
#[derive(Default)]
pub(crate) struct GameTests {
    pub defs: Defs,
    runner: Option<Runner>,
    last_failed: Vec<String>,
}

impl GameTests {
    pub(crate) fn active(&self) -> bool {
        self.runner.is_some()
    }
}

/// Text of a translation for a test id or a coordinate triple.
fn test_name(id: &str) -> Text {
    Text::literal(id)
}

fn coordinates(p: Pos) -> Text {
    tr!("chat.coordinates", p[0], p[1], p[2])
}

impl Sim {
    // ---- world access ----

    fn block_entity_kind() -> u16 {
        kiln_world::block_entity::type_id("minecraft:test_instance_block").expect("test_instance_block block entity type")
    }

    /// The saved NBT of the test instance block entity at `pos`, if there is one.
    fn test_block_nbt(&self, dim: DimId, pos: Pos) -> Option<Tag> {
        let chunk = self.dims[dim].regions.chunk(kiln_world::ChunkPos::of_block(pos[0], pos[2]))?;
        let be = chunk.block_entity((pos[0] & 15) as usize, pos[1], (pos[2] & 15) as usize)?;
        (be.kind == Self::block_entity_kind()).then(|| be.nbt.clone())
    }

    fn test_block_data(&self, dim: DimId, pos: Pos) -> Option<BlockData> {
        self.test_block_nbt(dim, pos).map(|t| BlockData::read(&t))
    }

    fn set_test_block_data(&mut self, dim: DimId, pos: Pos, data: &BlockData) {
        self.load_block_entity(dim, pos, &data.fields());
    }

    /// The layout of the test instance block at `pos` (its data and its test's rotation and
    /// padding).
    fn layout_of(&self, pos: Pos, data: &BlockData) -> Layout {
        let def = data.test.as_ref().and_then(|t| self.commands.gametests.defs.tests.get(t));
        Layout {
            block: pos,
            size: data.size,
            rotation: (def.map_or(0, |d| d.rotation) + data.rotation) % 4,
            padding: def.map_or(0, |d| d.padding),
        }
    }

    /// Test instance blocks of `dim` within `radius` of `center` (`StructureUtils.findTestBlocks`,
    /// over the loaded chunks), in position order.
    fn find_test_blocks(&self, dim: DimId, center: Pos, radius: i32) -> Vec<Pos> {
        let kind = Self::block_entity_kind();
        let mut out = Vec::new();
        let r2 = i64::from(radius) * i64::from(radius);
        for region in self.dims[dim].regions.iter() {
            for (cell_pos, cell) in region.cells().iter() {
                for (cp, chunk) in cell.chunks(cell_pos) {
                    // Cheap reject: the chunk's closest point.
                    let dx = ((cp.x * 16).max(center[0]).min(cp.x * 16 + 15) - center[0]).abs();
                    let dz = ((cp.z * 16).max(center[2]).min(cp.z * 16 + 15) - center[2]).abs();
                    if i64::from(dx) > i64::from(radius) || i64::from(dz) > i64::from(radius) {
                        continue;
                    }
                    for ((x, y, z), be) in chunk.block_entities() {
                        if be.kind != kind {
                            continue;
                        }
                        let p = [cp.x * 16 + x as i32, y, cp.z * 16 + z as i32];
                        let d: i64 = (0..3).map(|i| i64::from(p[i] - center[i]).pow(2)).sum();
                        if d <= r2 {
                            out.push(p);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// `StructureUtils.clearSpaceForStructure`: stone below the box's second layer, air above,
    /// and the entities in it (players excepted) gone.
    fn clear_space(&mut self, dim: DimId, bounds: Bounds) {
        let floor = bounds.min[1] + 1;
        let dimension = DIMENSIONS[dim].0;
        let flags = UpdateFlags::placement(true);
        for p in bounds.positions() {
            let state = if p[1] < floor { kiln_data::blocks::default_state::STONE } else { kiln_data::blocks::default_state::AIR };
            self.set_block(dimension, p, state, None, flags);
        }
        self.discard_entities(dim, bounds, 0.0);
    }

    /// Discards the non-player entities inside `bounds` inflated by `inflate`.
    fn discard_entities(&mut self, dim: DimId, bounds: Bounds, inflate: f64) {
        let (lo, hi) = bounds.aabb();
        for r in self.dims[dim].regions.iter_mut() {
            for e in r.part_mut().0.list.iter_mut() {
                let inside = (0..3).all(|i| e.pos[i] >= lo[i] - inflate && e.pos[i] <= hi[i] + inflate);
                if inside && !e.removed {
                    match e.phys.as_mut() {
                        Some(p) => {
                            p.removed.get_or_insert(kiln_entity::entity::RemovalReason::Discarded);
                        }
                        None => e.removed = true,
                    }
                }
            }
        }
    }

    /// `ServerLevel.setChunkForced` over the box's chunks.
    fn force_load(&mut self, dim: DimId, bounds: Bounds) {
        for c in bounds.chunks() {
            self.set_forced(dim, c, true);
        }
    }

    fn unforce(&mut self, dim: DimId, chunks: &[[i32; 2]]) {
        for &c in chunks {
            self.set_forced(dim, c, false);
        }
    }

    // ---- feedback ----

    /// Runs `f` with `source` as the command source (results come after the command ended).
    pub(crate) fn as_source<R>(&mut self, source: CommandSource, f: impl FnOnce(&mut Sim) -> R) -> R {
        let previous = std::mem::replace(&mut self.commands.source, source);
        let start = self.source_stack(source);
        let stack = std::mem::replace(&mut self.commands.stack, start);
        let r = f(self);
        self.commands.source = previous;
        self.commands.stack = stack;
        r
    }

    /// The source a command runs as, to answer it later (`CommandSourceStack`): who receives
    /// the output, and the executing entity and position.
    pub(crate) fn origin(&self) -> Origin {
        Origin { source: self.commands.source, stack: self.commands.stack.clone() }
    }

    /// Runs `f` as `origin` (results that come after the command ended).
    pub(crate) fn as_origin<R>(&mut self, origin: &Origin, f: impl FnOnce(&mut Sim) -> R) -> R {
        let previous = std::mem::replace(&mut self.commands.source, origin.source);
        let stack = std::mem::replace(&mut self.commands.stack, origin.stack.clone());
        let r = f(self);
        self.commands.source = previous;
        self.commands.stack = stack;
        r
    }

    /// `ReportGameListener.say`: to every player.
    fn say(&mut self, color: &'static str, message: String) {
        let text = Text::literal(message).color(color);
        let pkt = kiln_proto::packets::system_chat(text.to_nbt(), false);
        for p in self.players.values_mut() {
            p.send(pkt.clone());
        }
    }

    /// The test area's origin (`playerAndTestInfo`): the source's column at the surface of
    /// `dim`.
    fn test_info(&mut self, dim: DimId) -> (Pos, i32) {
        // `getPlayer() == null ? getPosition() : player.position()`.
        let at = match self.commands.stack.entity.as_ref().filter(|e| kiln_command::SelectorTarget::is_player(*e)) {
            Some(player) => kiln_command::SelectorTarget::position(player),
            None => self.commands.stack.position,
        };
        let [x, _, z] = at.map(|v| v.floor() as i32);
        let surface = Host::height(self, DIMENSIONS[dim].0, kiln_command::Heightmap::WorldSurface, x, z);
        ([x, 384, z], surface)
    }

    /// `createTestPositionAround`.
    fn test_position_around(&mut self, dim: DimId) -> Pos {
        let (p, surface) = self.test_info(dim);
        [p[0], surface, p[2] + 3]
    }

    fn source_dim(&self) -> DimId {
        dim_id(&self.commands.stack.dimension).unwrap_or(crate::OVERWORLD_ID)
    }
}
