//! The WASM plugin host (design §11): wasmtime components against the WIT package `kiln:api`
//! (`wit/kiln-api.wit`), guests built for `wasm32-wasip2`.
//!
//! # Contract
//!
//! - Each plugin has one **global** instance ([`PluginRuntime`]), called only from serial
//!   phases: `init` (command registration), `on-enable`/`on-disable` (hot reload), join,
//!   leave, plugin commands, global tasks, results and cancellation notices.
//! - A plugin exporting `region-hooks` has one instance per region ([`RegionPlugins`]),
//!   bound to the region and never to a thread; the embedder keeps the set in step with the
//!   regionizer ([`PluginRuntime::sync_regions`], in B0). Instances are replaced at will
//!   (split, merge, reload, a trap), so guest memory is only a cache.
//! - Persistent state lives in host-owned namespaces: player, cell, entity and global. A
//!   handler reaches only the players, the cell and the entity its event names (handles are
//!   valid for that call only and carry the plugin's generation) plus a snapshot of its global
//!   namespace at most one tick old; it changes the global namespace only through typed atomic
//!   operations, applied in B0 in an order that does not depend on threads or the region
//!   layout ([`PluginRuntime::begin_tick`]); their results arrive the next tick.
//! - Writes, operations, tasks and messages of a call are buffered and committed only when
//!   the call returns normally: a trap or a timeout leaves nothing behind.
//! - Tasks (`scheduler`) run in B0: global ones in the global instance, player ones in the
//!   region instance holding the player, position ones in the region owning the position.
//! - Hot reload ([`PluginRuntime::reload`]): the new component is compiled off the tick, then
//!   in B0 the global instance hands a state blob over (`on-disable` → `on-enable`),
//!   subscriptions and commands switch at once, every instance is replaced, and tasks of the
//!   old generation are cancelled with a notice to the new one. Owned namespaces stay as
//!   they are.
//!
//! # Budgets and the two execution modes
//!
//! - **Ordered** ([`ExecMode::Ordered`], the default): every cancellable call gets a fresh
//!   wall-clock deadline (epoch interruption, default 500 µs), region instances a wall-clock
//!   budget per tick. Fast, and deterministic as long as no call runs out of time: a call
//!   preempted by the OS can time out on one run and not on another.
//! - **Strict** ([`ExecMode::Strict`]): budgets are fuel (wasm instructions), so whether a
//!   call runs out does not depend on the machine; `env.now-millis` and WASI's clocks follow
//!   the tick, WASI's random streams are fixed. Same seed and inputs give the same results
//!   on any thread count and region layout (for tests and replays).
//! - In both modes `env.random`, tickets and task handles derive from the seed, the plugin,
//!   the tick, the acting player and the call's position among that player's calls, never
//!   from arrival order. Batched calls (observe) take their first player as the source, so
//!   operations from them are only layout-independent when they commute (`add`).
//! - A timeout is a strike; three strikes in 1,200 ticks demote the plugin to observe-only. A
//!   failed or demoted fail-closed subscription denies; fail-open carries on. Running out of
//!   the per-tick budget of an instance or the acting player's event bucket (a token bucket
//!   refilled per tick) applies the policy without a strike.
//! - Only capabilities the manifest grants are linked; memory is capped at 64 MiB per
//!   instance; instances come from the pooling allocator; compiled components are cached as
//!   `.cwasm` files.
//!
//! Not here: the WASI 0.3 `async-tasks` world.

#[cfg(feature = "async-tasks")]
mod async_tasks;
#[cfg(not(feature = "async-tasks"))]
#[path = "async_tasks_off.rs"]
mod async_tasks;
mod cache;
mod effects;
pub mod examples;
mod host;
pub mod manifest;
mod ns;

pub use effects::{BlockChange, Effect, EffectKind, ItemSpec, MenuSpec, OnlinePlayer, PlayerInfo, SpawnSpec};
pub use manifest::{Area, Capability, EventKind, FailPolicy, Filter, Manifest, ObserveKinds};
pub use ns::{CellKey, CellSidecars, EntityData, GlobalValue};

use anyhow::{Context, Result, bail};
use host::{Frame, GlobalGuest, GlobalIndices, HostState, NewTask, Pre, RegionGuest, RegionIndices, wit};
use ns::{CellTable, FastMap, Globals, Ns, Persist};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing::{info, warn};
use wasmtime::{Engine, Store};

/// Styled chat text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub color: Option<String>,
    pub bold: bool,
    pub italic: bool,
}

impl Span {
    pub fn colored(text: impl Into<String>, color: &str) -> Span {
        Span { text: text.into(), color: Some(color.to_owned()), bold: false, italic: false }
    }
}

/// The player an event is about.
#[derive(Clone, Copy, Debug)]
pub struct Actor<'a> {
    pub uuid: u128,
    pub name: &'a str,
    pub operator: bool,
    /// What `event.info` answers for this player.
    pub info: PlayerInfo,
}

impl<'a> Actor<'a> {
    pub fn new(uuid: u128, name: &'a str, operator: bool) -> Self {
        Actor { uuid, name, operator, info: PlayerInfo::default() }
    }

    pub fn with_info(mut self, info: PlayerInfo) -> Self {
        self.info = info;
        self
    }

    fn permission(&self) -> u8 {
        if self.operator { 4 } else { 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Denied, with an optional message for the acting player.
    Deny(Option<Vec<Span>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatOutcome {
    Pass,
    Cancel,
    /// Broadcast this text instead of the chat message.
    Rewrite(Vec<Span>),
}

/// A plugin message to deliver: to one player, or to everyone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub to: Option<u128>,
    pub text: Vec<Span>,
}

/// A command a plugin registered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandReg {
    pub plugin: usize,
    pub name: String,
    pub permission: u8,
}

/// Registries whose per-run ids cross the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryKind {
    Level,
    Block,
    Item,
    EntityType,
    DamageType,
}

/// Members (ids) of a tag, e.g. `(Block, "minecraft:logs")`.
pub type TagResolver = dyn Fn(RegistryKind, &str) -> Option<Vec<u32>> + Send + Sync;

/// The embedder's registries: keys by per-run id, and tags for manifest filters.
pub struct Registries {
    pub levels: Vec<String>,
    pub blocks: Vec<String>,
    pub items: Vec<String>,
    pub entity_types: Vec<String>,
    pub damage_types: Vec<String>,
    pub tags: Option<Arc<TagResolver>>,
    index: OnceLock<[HashMap<String, u32>; 5]>,
}

impl Default for Registries {
    fn default() -> Self {
        let levels = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"].map(String::from).to_vec();
        Registries::new(levels, Vec::new(), Vec::new(), Vec::new())
    }
}

impl Registries {
    pub fn new(levels: Vec<String>, blocks: Vec<String>, items: Vec<String>, entity_types: Vec<String>) -> Self {
        Registries { levels, blocks, items, entity_types, damage_types: Vec::new(), tags: None, index: OnceLock::new() }
    }

    pub fn with_tags(mut self, tags: Arc<TagResolver>) -> Self {
        self.tags = Some(tags);
        self
    }

    pub fn with_damage_types(mut self, damage_types: Vec<String>) -> Self {
        self.damage_types = damage_types;
        self
    }

    pub fn list(&self, k: RegistryKind) -> &[String] {
        match k {
            RegistryKind::Level => &self.levels,
            RegistryKind::Block => &self.blocks,
            RegistryKind::Item => &self.items,
            RegistryKind::EntityType => &self.entity_types,
            RegistryKind::DamageType => &self.damage_types,
        }
    }

    pub fn id(&self, k: RegistryKind, key: &str) -> Option<u32> {
        let index = self.index.get_or_init(|| {
            [RegistryKind::Level, RegistryKind::Block, RegistryKind::Item, RegistryKind::EntityType, RegistryKind::DamageType]
                .map(|k| self.list(k).iter().enumerate().map(|(i, s)| (s.clone(), i as u32)).collect())
        });
        let key = if key.contains(':') { std::borrow::Cow::Borrowed(key) } else { std::borrow::Cow::Owned(format!("minecraft:{key}")) };
        index[k as usize].get(key.as_ref()).copied()
    }

    /// A membership table of keys and `#tags` (unknown ones are reported and ignored).
    fn resolve(&self, k: RegistryKind, names: &[String], plugin: &str) -> Vec<bool> {
        let mut set = vec![false; self.list(k).len()];
        for n in names {
            let ids = match n.strip_prefix('#') {
                Some(tag) => self.tags.as_ref().and_then(|t| t(k, tag)),
                None => self.id(k, n).map(|i| vec![i]),
            };
            match ids {
                Some(ids) => {
                    for i in ids {
                        if let Some(s) = set.get_mut(i as usize) {
                            *s = true;
                        }
                    }
                }
                None => warn!("plugin {plugin}: filter `{n}` names nothing"),
            }
        }
        set
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExecMode {
    /// Wall-clock deadlines: see the crate docs.
    #[default]
    Ordered,
    /// Fuel budgets, a tick clock and fixed random streams: see the crate docs.
    Strict,
}

pub struct RuntimeConfig {
    /// Namespace files (`<world>/kiln/plugins`); nothing is saved when `None`.
    pub data_dir: Option<PathBuf>,
    /// Compiled components (`.cwasm`); compiled every time when `None`.
    pub cache_dir: Option<PathBuf>,
    pub registries: Arc<Registries>,
    pub spawn: [i32; 3],
    pub mode: ExecMode,
    /// Seed of `env.random`, tickets and task handles.
    pub seed: u64,
    /// Ordered mode: fresh time budget of each cancellable call.
    pub call_budget: Duration,
    /// Ordered mode: time budget of each region instance per tick.
    pub tick_budget: Duration,
    /// Epoch ticker period (ordered mode).
    pub epoch_tick: Duration,
    /// Strict mode: fresh fuel of each call, and fuel of each region instance per tick.
    pub call_fuel: u64,
    pub tick_fuel: u64,
    /// Per-player token bucket of cancellable events: capacity, and refill per second (20
    /// ticks); `0` per second turns the limit off.
    pub player_burst: u32,
    pub player_events_per_second: u32,
    /// Pooling allocator slots (component instances); `0` uses on-demand allocation.
    pub pool_instances: u32,
    /// Where cell data goes instead of sidecar files under `data_dir` (native worlds).
    pub cell_sidecars: Option<Arc<dyn CellSidecars>>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            data_dir: None,
            cache_dir: None,
            registries: Arc::new(Registries::default()),
            spawn: [0, 64, 0],
            mode: ExecMode::Ordered,
            seed: 0,
            call_budget: Duration::from_micros(500),
            tick_budget: Duration::from_millis(10),
            epoch_tick: Duration::from_micros(250),
            call_fuel: 2_000_000,
            tick_fuel: 100_000_000,
            player_burst: 64,
            player_events_per_second: 80,
            pool_instances: 512,
            cell_sidecars: None,
        }
    }
}

/// Call statistics.
#[derive(Default)]
pub struct Stats {
    pub calls: AtomicU64,
    pub traps: AtomicU64,
    pub timeouts: AtomicU64,
    pub instantiations: AtomicU64,
    /// Events a player's bucket or an instance's tick budget kept from plugins.
    pub rate_limited: AtomicU64,
    pub budget_exhausted: AtomicU64,
    pub tasks_run: AtomicU64,
    pub tasks_cancelled: AtomicU64,
    pub results_delivered: AtomicU64,
    pub reloads: AtomicU64,
    pub cache_hits: AtomicU64,
}

fn load(a: &AtomicU64) -> u64 {
    a.load(Ordering::Relaxed)
}

impl Stats {
    pub fn get(&self) -> [(&'static str, u64); 11] {
        [
            ("calls", load(&self.calls)),
            ("traps", load(&self.traps)),
            ("timeouts", load(&self.timeouts)),
            ("instantiations", load(&self.instantiations)),
            ("rate-limited", load(&self.rate_limited)),
            ("budget-exhausted", load(&self.budget_exhausted)),
            ("tasks-run", load(&self.tasks_run)),
            ("tasks-cancelled", load(&self.tasks_cancelled)),
            ("results", load(&self.results_delivered)),
            ("reloads", load(&self.reloads)),
            ("cache-hits", load(&self.cache_hits)),
        ]
    }
}

struct Health {
    /// Ticks of recent strikes.
    strikes: Mutex<VecDeque<u64>>,
    demoted: AtomicBool,
}

const STRIKES: usize = 3;
/// 60 s of ticks.
const STRIKE_WINDOW: u64 = 1200;

/// A queued atomic operation of a non-global context.
struct Pending {
    tick: u64,
    source: u128,
    plugin: usize,
    generation: u32,
    ticket: u64,
    op: wit::AtomicOp,
}

/// A job handed to a plugin's tasks component.
struct Inflight {
    plugin: usize,
    generation: u32,
    /// The plugin's own number for it.
    id: u64,
    /// The player whose call submitted it (results go back to them).
    source: u128,
}

/// An operation's outcome waiting for delivery.
struct Delivery {
    plugin: usize,
    generation: u32,
    source: u128,
    result: wit::OpResult,
}

/// Where a task runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskTarget {
    Global,
    Player(u128),
    /// Level id, block x, block z.
    Position(u32, i32, i32),
}

/// A scheduled task. Tasks due the same tick run in (scheduling tick, source, call order).
#[derive(Clone, Debug, Hash)]
struct Task {
    plugin: usize,
    generation: u32,
    handle: u64,
    id: u64,
    target: TaskTarget,
    due: u64,
}

type TaskKey = (u64, u64, u128, u32, u32);

#[derive(Default)]
struct Tasks {
    table: BTreeMap<TaskKey, Task>,
    by_handle: HashMap<(usize, u64), TaskKey>,
    /// Committed this tick, not yet in the table.
    fresh: Vec<(TaskKey, Task)>,
    cancels: Vec<(usize, u64)>,
}

/// Per-player token bucket, in 1/20 event units (refilled per tick).
#[derive(Clone, Copy)]
struct Bucket {
    units: u64,
    tick: u64,
}

/// State every instance of every plugin reaches (behind short locks; regions touch
/// disjoint players and cells).
pub(crate) struct Shared {
    tick: AtomicU64,
    pub(crate) strict: bool,
    pub(crate) seed: u64,
    pub(crate) registries: Arc<Registries>,
    pub(crate) players: Mutex<FastMap<u128, Ns>>,
    pub(crate) cells: Mutex<CellTable>,
    pub(crate) globals: Mutex<Globals>,
    globals_dirty: AtomicBool,
    pub(crate) snapshot: Mutex<Arc<Globals>>,
    pending: Mutex<Vec<Pending>>,
    deliveries: Mutex<Vec<Delivery>>,
    tasks: Mutex<Tasks>,
    /// Committed effects (tick, acting player, effect), in commit order.
    outbox: Mutex<Vec<(u64, u128, Effect)>>,
    /// The players online at the start of the tick (`event.online`).
    pub(crate) online: Mutex<Arc<Vec<OnlinePlayer>>>,
    /// The async-tasks worker (started when some plugin has a tasks component).
    async_worker: std::sync::OnceLock<Result<async_tasks::AsyncTasks_, String>>,
    /// Jobs handed to the worker and not finished: ticket to what they are.
    jobs_inflight: Mutex<HashMap<u64, Inflight>>,
    health: Vec<Health>,
    /// Plugins subscribed to op-results (by index; updated by reloads).
    wants_results: Vec<AtomicBool>,
    /// Calls per source this tick (deterministic ids).
    seqs: Mutex<FastMap<u128, u32>>,
    /// Sharded by player: region threads rarely meet on a lock.
    buckets: Vec<Mutex<FastMap<u128, Bucket>>>,
    /// Epoch ticks so far (ordered mode's clock for per-tick budgets).
    epoch: Arc<AtomicU64>,
    persist: Option<Persist>,
    pub stats: Stats,
}

impl Shared {
    pub(crate) fn next_seq(&self, source: u128) -> u32 {
        let mut s = self.seqs.lock().unwrap();
        let e = s.entry(source).or_insert(0);
        *e += 1;
        *e
    }

    /// Loads the sidecar holding `cell` (once), when namespaces are persisted.
    pub(crate) fn ensure_cell(&self, table: &mut CellTable, cell: CellKey) {
        if let Some(p) = &self.persist {
            p.ensure_cell(table, cell);
        }
    }

    pub(crate) fn now_millis(&self) -> u64 {
        if self.strict {
            1_600_000_000_000 + self.tick() * 50
        } else {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
        }
    }

    pub(crate) fn task_pending(&self, plugin: usize, handle: u64) -> bool {
        let t = self.tasks.lock().unwrap();
        t.by_handle.contains_key(&(plugin, handle)) && !t.cancels.contains(&(plugin, handle))
            || t.fresh.iter().any(|(_, x)| x.plugin == plugin && x.handle == handle)
    }

    /// Makes a call's buffered effects real (the call returned normally). Entity writes go
    /// into the frame's entities, which the caller hands back.
    fn commit(&self, plugin: usize, generation: u32, id: &Arc<str>, f: &mut Frame) {
        if !f.writes.is_empty() {
            let mut players = None;
            let mut cells = None;
            for (target, key, val) in f.writes.drain(..) {
                match target {
                    host::Target::Player(u) => {
                        players.get_or_insert_with(|| self.players.lock().unwrap()).entry(u).or_default().put(plugin, key, val)
                    }
                    host::Target::Cell(c) => {
                        let table = cells.get_or_insert_with(|| self.cells.lock().unwrap());
                        // The sidecar first, so that a write does not hide the saved data.
                        self.ensure_cell(table, c);
                        table.cells.entry(c).or_default().put(plugin, key, val)
                    }
                    host::Target::Entity(i) => {
                        let data = &mut f.entities[i].1;
                        match val {
                            Some(v) => drop(data.entry(id.to_string()).or_default().insert(key, v)),
                            None => {
                                if let Some(kv) = data.get_mut(&**id) {
                                    kv.remove(&key);
                                    if kv.is_empty() {
                                        data.remove(&**id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let tick = self.tick();
        if !f.ops.is_empty() {
            if f.global_ctx {
                let wants = self.wants_results[plugin].load(Ordering::Relaxed);
                let mut g = self.globals.lock().unwrap();
                let mut out = Vec::new();
                for (ticket, op) in f.ops.drain(..) {
                    let r = apply_op(g.entry(plugin), ticket, op);
                    if wants {
                        out.push(Delivery { plugin, generation, source: f.source, result: r });
                    }
                }
                drop(g);
                self.deliveries.lock().unwrap().extend(out);
                self.globals_dirty.store(true, Ordering::Relaxed);
            } else {
                let mut pending = self.pending.lock().unwrap();
                pending.extend(f.ops.drain(..).map(|(ticket, op)| Pending { tick, source: f.source, plugin, generation, ticket, op }));
            }
        }
        if !f.tasks.is_empty() || !f.cancels.is_empty() {
            let mut t = self.tasks.lock().unwrap();
            for NewTask { handle, id, target, delay, order } in f.tasks.drain(..) {
                let due = tick + delay as u64;
                let key = (due, tick, f.source, order.0, order.1);
                t.fresh.push((key, Task { plugin, generation, handle, id, target, due }));
            }
            t.cancels.extend(f.cancels.drain(..).map(|h| (plugin, h)));
        }
        for j in f.jobs.drain(..) {
            self.submit_job(plugin, generation, f.source, j);
        }
        if !f.effects.is_empty() {
            let mut out = self.outbox.lock().unwrap();
            let source = f.source;
            out.extend(f.effects.drain(..).map(|(ticket, kind)| {
                (tick, source, Effect { plugin, plugin_id: id.clone(), generation, source, ticket, kind })
            }));
        }
    }

    /// The async-tasks worker, started on first use.
    pub(crate) fn tasks_worker(&self) -> Option<&async_tasks::AsyncTasks_> {
        self.async_worker.get_or_init(|| async_tasks::AsyncTasks_::start().map_err(|e| format!("{e:#}"))).as_ref().ok()
    }

    /// The engine to compile a manifest's tasks component for (the worker starts if it must);
    /// strict mode has no tasks, so the component is dropped from the manifest.
    fn tasks_engine_for(&self, manifest: &mut Manifest) -> Option<Engine> {
        if self.strict {
            manifest.tasks_wasm = None;
            return None;
        }
        manifest.tasks_wasm.as_ref()?;
        self.tasks_worker().map(|w| w.engine.clone())
    }

    /// The worker if it is running (no start).
    fn started_worker(&self) -> Option<&async_tasks::AsyncTasks_> {
        self.async_worker.get().and_then(|r| r.as_ref().ok())
    }

    /// A job a call committed goes to the plugin's tasks component; its outcome is an
    /// `op-result` later. Without a worker or in strict mode it fails at once.
    fn submit_job(&self, plugin: usize, generation: u32, source: u128, j: host::NewJob) {
        let fail = |why: &str| {
            if self.wants_results[plugin].load(Ordering::Relaxed) {
                let result = wit::OpResult { ticket: j.ticket, applied: false, value: Some(wit::GlobalValue::Bytes(why.as_bytes().to_vec())) };
                self.deliveries.lock().unwrap().push(Delivery { plugin, generation, source, result });
            }
        };
        if self.strict {
            return fail("async tasks are not available in strict mode");
        }
        match self.started_worker() {
            Some(w) => {
                self.jobs_inflight.lock().unwrap().insert(j.ticket, Inflight { plugin, generation, id: j.id, source });
                w.submit(async_tasks::JobIn { plugin, generation, ticket: j.ticket, id: j.id, kind: j.kind, payload: j.payload });
            }
            None => fail("the plugin has no tasks component running"),
        }
    }

    /// B0: jobs the worker finished become results, in ticket order.
    fn collect_jobs(&self, tick: u64) {
        let Some(w) = self.started_worker() else { return };
        w.set_tick(tick);
        let mut done = w.take_done();
        if done.is_empty() {
            return;
        }
        done.sort_by_key(|d| d.ticket);
        let mut inflight = self.jobs_inflight.lock().unwrap();
        let mut out = Vec::new();
        for d in done {
            let Some(job) = inflight.remove(&d.ticket) else { continue };
            // A job of a generation that was reloaded away was reported as cancelled instead.
            if job.generation != d.generation || !self.wants_results[job.plugin].load(Ordering::Relaxed) {
                continue;
            }
            let (applied, bytes) = match d.result {
                Ok(b) => (true, b),
                Err(why) => (false, why.into_bytes()),
            };
            let result = wit::OpResult { ticket: d.ticket, applied, value: Some(wit::GlobalValue::Bytes(bytes)) };
            out.push(Delivery { plugin: job.plugin, generation: job.generation, source: job.source, result });
        }
        drop(inflight);
        self.deliveries.lock().unwrap().extend(out);
    }

    fn demoted(&self, plugin: usize) -> bool {
        self.health[plugin].demoted.load(Ordering::Relaxed)
    }

    fn strike(&self, plugin: usize, id: &str) {
        let now = self.tick();
        let mut s = self.health[plugin].strikes.lock().unwrap();
        s.push_back(now);
        while s.front().is_some_and(|t| now - *t > STRIKE_WINDOW) {
            s.pop_front();
        }
        if s.len() >= STRIKES && !self.health[plugin].demoted.swap(true, Ordering::Relaxed) {
            warn!("plugin {id}: {STRIKES} calls over budget within {STRIKE_WINDOW} ticks, demoted to observe-only");
        }
    }

    fn bucket_shard(&self, uuid: u128) -> &Mutex<FastMap<u128, Bucket>> {
        let h = (uuid as u64 ^ (uuid >> 64) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        &self.buckets[(h >> 58) as usize % self.buckets.len()]
    }

    /// Takes one event from the player's bucket; false when it is empty.
    fn take_token(&self, uuid: u128, burst: u32, per_second: u32) -> bool {
        if per_second == 0 {
            return true;
        }
        let tick = self.tick();
        let cap = burst as u64 * 20;
        let mut b = self.bucket_shard(uuid).lock().unwrap();
        let bucket = b.entry(uuid).or_insert(Bucket { units: cap, tick });
        bucket.units = (bucket.units + (tick - bucket.tick) * per_second as u64).min(cap);
        bucket.tick = tick;
        if bucket.units >= 20 {
            bucket.units -= 20;
            true
        } else {
            false
        }
    }
}

/// Applies a typed atomic operation; a type mismatch or a failed comparison changes nothing.
fn apply_op(ns: &mut BTreeMap<String, GlobalValue>, ticket: u64, op: wit::AtomicOp) -> wit::OpResult {
    let (key, applied) = match op {
        wit::AtomicOp::Add((key, delta)) => {
            let applied = match ns.get_mut(&key) {
                Some(GlobalValue::Int(v)) => {
                    *v = v.wrapping_add(delta);
                    true
                }
                Some(GlobalValue::Bytes(_)) => false,
                None => {
                    ns.insert(key.clone(), GlobalValue::Int(delta));
                    true
                }
            };
            (key, applied)
        }
        wit::AtomicOp::CompareAndSet(c) => {
            let expected = c.expected.map(host::from_wit_value);
            let applied = ns.get(&c.key) == expected.as_ref();
            if applied {
                ns.insert(c.key.clone(), host::from_wit_value(c.new));
            }
            (c.key, applied)
        }
        wit::AtomicOp::TryAdd(t) => {
            // A missing key counts as 0; a byte value is not a number.
            let current = match ns.get(&t.key) {
                Some(GlobalValue::Int(v)) => Some(*v),
                Some(GlobalValue::Bytes(_)) => None,
                None => Some(0),
            };
            let applied = match current.and_then(|v| v.checked_add(t.delta)) {
                Some(next) if next >= t.floor => {
                    ns.insert(t.key.clone(), GlobalValue::Int(next));
                    true
                }
                _ => false,
            };
            (t.key, applied)
        }
        wit::AtomicOp::Append((key, bytes)) => {
            let applied = match ns.get_mut(&key) {
                Some(GlobalValue::Bytes(v)) => {
                    v.extend_from_slice(&bytes);
                    true
                }
                Some(GlobalValue::Int(_)) => false,
                None => {
                    ns.insert(key.clone(), GlobalValue::Bytes(bytes));
                    true
                }
            };
            (key, applied)
        }
    };
    wit::OpResult { ticket, applied, value: ns.get(&key).cloned().map(host::to_wit_value) }
}

/// A subscription's filter, resolved against the registries.
#[derive(Default)]
struct Compiled {
    blocks: Option<Vec<bool>>,
    entities: Option<Vec<bool>>,
    items: Option<Vec<bool>>,
    all_items: bool,
    names: Vec<String>,
    vanilla: bool,
    /// Level id (any when `None`), centre, radius.
    area: Option<(Option<u32>, i32, i32, i32)>,
    bypass: Option<u8>,
}

/// What a filter sees of an event.
#[derive(Clone, Copy, Default)]
struct EvInfo<'a> {
    permission: u8,
    level: u32,
    /// Block column of the event, if it has a position.
    xz: Option<(i32, i32)>,
    block: Option<u32>,
    entity_type: Option<u32>,
    item: Option<u32>,
    /// item-use: the plugin id in the held item's tag.
    tag_owner: Option<&'a str>,
    /// container-click: the plugin id of the menu, none for a vanilla container.
    menu_owner: Option<&'a str>,
    /// custom: the full event name.
    custom_name: Option<&'a str>,
}

impl Compiled {
    fn new(f: &Filter, reg: &Registries, spawn: [i32; 3], plugin: &str) -> Compiled {
        Compiled {
            blocks: (!f.blocks.is_empty()).then(|| reg.resolve(RegistryKind::Block, &f.blocks, plugin)),
            entities: (!f.entities.is_empty()).then(|| reg.resolve(RegistryKind::EntityType, &f.entities, plugin)),
            all_items: f.items.iter().any(|i| i == "*"),
            items: {
                let names: Vec<String> = f.items.iter().filter(|i| *i != "*").cloned().collect();
                (!names.is_empty()).then(|| reg.resolve(RegistryKind::Item, &names, plugin))
            },
            names: f.names.clone(),
            vanilla: f.vanilla,
            area: f.area.as_ref().map(|a| {
                let level = a.level.as_ref().map(|l| {
                    reg.id(RegistryKind::Level, l).unwrap_or_else(|| {
                        warn!("plugin {plugin}: unknown level `{l}` in an area filter");
                        u32::MAX
                    })
                });
                let (x, z) = a.center.unwrap_or((spawn[0], spawn[2]));
                (level, x, z, a.radius)
            }),
            bypass: f.bypass_permission,
        }
    }

    fn passes(&self, kind: EventKind, ev: &EvInfo, plugin: &str) -> bool {
        if self.bypass.is_some_and(|p| ev.permission >= p) {
            return false;
        }
        let member = |set: &Option<Vec<bool>>, id: Option<u32>| match set {
            None => true,
            Some(s) => id.is_some_and(|i| s.get(i as usize).copied().unwrap_or(false)),
        };
        if !member(&self.blocks, ev.block) || !member(&self.entities, ev.entity_type) {
            return false;
        }
        match kind {
            // The plugin's own tagged items; `items` adds others (`*`: all).
            EventKind::ItemUse => {
                let own = ev.tag_owner == Some(plugin);
                if !own && !self.all_items && !(self.items.is_some() && member(&self.items, ev.item)) {
                    return false;
                }
            }
            // The plugin's own menus; `vanilla` adds the others.
            EventKind::ContainerClick => match ev.menu_owner {
                Some(owner) if owner != plugin => return false,
                Some(_) => {}
                None if !self.vanilla => return false,
                None => {}
            },
            EventKind::Custom if !self.names.is_empty() => {
                if !ev.custom_name.is_some_and(|n| self.names.iter().any(|x| x == n)) {
                    return false;
                }
            }
            _ => {}
        }
        match (self.area, ev.xz) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some((level, x, z, r)), Some((ex, ez))) => {
                level.is_none_or(|l| l == ev.level) && (ex - x).abs() <= r && (ez - z).abs() <= r
            }
        }
    }
}

/// A loaded plugin: its manifest and pre-linked component.
struct PluginDef {
    manifest: Manifest,
    id: Arc<str>,
    pre: Pre,
    global: GlobalIndices,
    region: Option<RegionIndices>,
    data_dir: Option<PathBuf>,
    /// Where it was loaded from (reloads read it again).
    source: Option<PathBuf>,
    init: wit::InitInfo,
    generation: u32,
    /// Filters by event kind (subscribed events only).
    filters: Vec<Option<Compiled>>,
    /// May raise events to other plugins (`events.raise`).
    raises: bool,
    /// The compiled `async-tasks` component and what it is granted.
    tasks: Option<(wasmtime::component::Component, async_tasks::TaskGrants)>,
}

impl PluginDef {
    fn filter(&self, kind: EventKind) -> Option<&Compiled> {
        self.filters[kind.index()].as_ref()
    }
}

/// A call's budget.
#[derive(Clone, Copy)]
enum Budget {
    /// Epoch ticks of deadline (ordered).
    Epoch(u64),
    Fuel(u64),
}

/// Everything fixed between reloads.
struct PluginSet {
    engine: Engine,
    plugins: Vec<Arc<PluginDef>>,
    /// Region-hook subscribers of each event kind, in load order.
    subs: Vec<Vec<usize>>,
    call: Budget,
    /// Calls that are not cancellable events (tasks, results, observe batches, the global
    /// instance's hooks): no player waits on them, so a larger fresh budget.
    serial: Budget,
    init: Budget,
    /// Per region instance and tick, in epoch ticks or fuel.
    tick_budget: u64,
    player_burst: u32,
    player_rate: u32,
}

impl PluginSet {
    fn build(engine: Engine, plugins: Vec<Arc<PluginDef>>, cfg: &SetCfg) -> PluginSet {
        let subs = EventKind::ALL
            .iter()
            .map(|k| (0..plugins.len()).filter(|&i| plugins[i].region.is_some() && plugins[i].manifest.subscription(*k).is_some()).collect())
            .collect();
        PluginSet {
            engine,
            plugins,
            subs,
            call: cfg.call,
            serial: cfg.serial,
            init: cfg.init,
            tick_budget: cfg.tick_budget,
            player_burst: cfg.player_burst,
            player_rate: cfg.player_rate,
        }
    }

    fn subscribers(&self, kind: EventKind) -> &[usize] {
        &self.subs[kind.index()]
    }
}

/// The parts of the configuration a [`PluginSet`] keeps.
#[derive(Clone)]
struct SetCfg {
    call: Budget,
    serial: Budget,
    init: Budget,
    tick_budget: u64,
    player_burst: u32,
    player_rate: u32,
    spawn: [i32; 3],
}

enum Outcome<R> {
    Ok(R),
    Timeout,
    Trap(wasmtime::Error),
}

/// One instance of one plugin.
struct Inst {
    store: Store<HostState>,
    global: Option<GlobalGuest>,
    region: Option<RegionGuest>,
    /// Budget spent in `spent_tick` (epoch ticks or fuel).
    spent_tick: u64,
    spent: u64,
}

impl Inst {
    /// Instantiates plugin `i` as a global or region instance and runs its `init`.
    fn new(set: &PluginSet, shared: &Arc<Shared>, i: usize, region: bool) -> Result<(Inst, Vec<wit::CommandSpec>)> {
        let def = &set.plugins[i];
        let store = host::new_store(&set.engine, i, def.generation, def.id.clone(), shared.clone(), def.data_dir.as_deref());
        let mut inst = Inst { store, global: None, region: None, spent_tick: 0, spent: 0 };
        // Instantiation runs guest code too (start functions, allocations).
        match set.init {
            Budget::Epoch(d) => inst.store.set_epoch_deadline(d),
            Budget::Fuel(n) => inst.store.set_fuel(n)?,
        }
        let instance = def.pre.instantiate(&mut inst.store)?;
        shared.stats.instantiations.fetch_add(1, Ordering::Relaxed);
        let mut commands = Vec::new();
        if region {
            inst.region = Some(def.region.as_ref().context("no region-hooks export")?.load(&mut inst.store, &instance)?);
        } else {
            inst.global = Some(def.global.load(&mut inst.store, &instance)?);
        }
        inst.store.data_mut().frame.reset(!region, 0);
        let r = inst.call(shared, set.init, |store, g, rg| match (g, rg) {
            (Some(g), _) => g.call_init(store, &def.init).map(|c| commands = c),
            (_, Some(r)) => r.call_init(store, &def.init),
            _ => unreachable!(),
        });
        match r {
            Outcome::Ok(()) => Ok((inst, commands)),
            Outcome::Timeout => bail!("plugin {} init ran out of budget", def.id),
            Outcome::Trap(e) => bail!("plugin {} init: {e:?}", def.id),
        }
    }

    /// Runs one call (its frame set up by the caller) with a fresh budget; commits its
    /// effects if it returned normally. Budget spent counts towards the instance's tick.
    fn call<R>(
        &mut self,
        shared: &Shared,
        budget: Budget,
        f: impl FnOnce(&mut Store<HostState>, Option<&GlobalGuest>, Option<&RegionGuest>) -> wasmtime::Result<R>,
    ) -> Outcome<R> {
        let tick = shared.tick();
        if self.spent_tick != tick {
            self.spent_tick = tick;
            self.spent = 0;
        }
        let start = match budget {
            Budget::Epoch(d) => {
                self.store.set_epoch_deadline(d);
                shared.epoch.load(Ordering::Relaxed)
            }
            Budget::Fuel(n) => {
                let _ = self.store.set_fuel(n);
                n
            }
        };
        self.store.data_mut().active = true;
        let r = f(&mut self.store, self.global.as_ref(), self.region.as_ref());
        self.store.data_mut().active = false;
        self.spent += match budget {
            Budget::Epoch(_) => shared.epoch.load(Ordering::Relaxed) - start,
            Budget::Fuel(_) => start - self.store.get_fuel().unwrap_or(0),
        };
        match r {
            Ok(v) => {
                let st = self.store.data_mut();
                if !st.frame.is_clean() {
                    let (plugin, generation, id) = (st.plugin, st.generation, st.id.clone());
                    shared.commit(plugin, generation, &id, &mut st.frame);
                }
                Outcome::Ok(v)
            }
            Err(e) if matches!(e.downcast_ref::<wasmtime::Trap>(), Some(wasmtime::Trap::Interrupt | wasmtime::Trap::OutOfFuel)) => {
                shared.stats.timeouts.fetch_add(1, Ordering::Relaxed);
                Outcome::Timeout
            }
            Err(e) => {
                shared.stats.traps.fetch_add(1, Ordering::Relaxed);
                Outcome::Trap(e)
            }
        }
    }

    fn frame(&mut self) -> &mut Frame {
        &mut self.store.data_mut().frame
    }

    fn generation(&self) -> u32 {
        self.store.data().generation
    }

    fn over_budget(&self, shared: &Shared, limit: u64) -> bool {
        self.spent_tick == shared.tick() && self.spent >= limit
    }
}

fn wit_pos(p: [i32; 3]) -> wit::BlockPos {
    wit::BlockPos { x: p[0], y: p[1], z: p[2] }
}

fn wit_player(a: &Actor, handle: u64) -> wit::Player {
    wit::Player { handle, uuid: host::wit_uuid(a.uuid), operator: a.operator }
}

fn spans(v: Vec<wit::Span>) -> Vec<Span> {
    v.into_iter().map(host::from_wit_span).collect()
}

/// An event another plugin raised (`events.raise`), as `dispatch_custom` passes it on.
pub(crate) struct CustomEvent {
    /// `<raising plugin id>:<name>`.
    pub name: String,
    pub source: Arc<str>,
    pub payload: Vec<u8>,
}

/// The other plugins' instances, lent to a plugin that may raise events for the length of
/// one call: the instances are out of their slots meanwhile, so a raised event can only reach
/// instances that are not on the call stack (re-entering one is not possible), and events
/// raised from inside it find no peers (depth one).
pub(crate) struct Peers {
    set: Arc<PluginSet>,
    region: bool,
    insts: Vec<(usize, Inst)>,
}

/// Takes the instances of the plugins subscribed to `custom` (other than `me`) out of their
/// slots; none when `me` cannot raise events or nobody is subscribed.
fn lend_peers(set: &Arc<PluginSet>, insts: &mut [Option<Inst>], me: usize, region: bool) -> Option<Box<Peers>> {
    let mut lent = Vec::new();
    for j in 0..insts.len() {
        let def = &set.plugins[j];
        if j != me && def.manifest.subscription(EventKind::Custom).is_some() && (!region || def.region.is_some()) {
            if let Some(inst) = insts[j].take() {
                lent.push((j, inst));
            }
        }
    }
    if lent.is_empty() { None } else { Some(Box::new(Peers { set: set.clone(), region, insts: lent })) }
}

/// Runs `body` on instance `i`; a plugin that may raise events gets its peers for the call.
fn with_peers<R>(set: &Arc<PluginSet>, insts: &mut [Option<Inst>], i: usize, region: bool, body: impl FnOnce(&mut Inst) -> R) -> R {
    if !set.plugins[i].raises {
        return body(insts[i].as_mut().expect("instance"));
    }
    let mut inst = insts[i].take().expect("instance");
    inst.store.data_mut().peers = lend_peers(set, insts, i, region);
    let r = body(&mut inst);
    if let Some(p) = inst.store.data_mut().peers.take() {
        for (j, pi) in p.insts {
            insts[j] = Some(pi);
        }
    }
    insts[i] = Some(inst);
    r
}

/// Calls the lent instances that subscribed to the event, in load order, until one denies
/// (`events.raise`). A peer that traps or runs out of budget is dropped (its slot is filled
/// again later); a fail-closed subscription then denies.
pub(crate) fn dispatch_custom(
    peers: &mut Peers,
    shared: &Arc<Shared>,
    ev: &CustomEvent,
    actor: Option<(u128, bool, &str, &PlayerInfo)>,
    source: u128,
) -> Verdict {
    let set = peers.set.clone();
    let mut k = 0;
    while k < peers.insts.len() {
        let j = peers.insts[k].0;
        let def = &set.plugins[j];
        let policy = def.manifest.subscription(EventKind::Custom).map_or(FailPolicy::Open, |s| s.policy);
        let fail = if policy == FailPolicy::Closed { Verdict::Deny(None) } else { Verdict::Allow };
        let info = EvInfo { permission: actor.map_or(0, |a| if a.1 { 4 } else { 0 }), custom_name: Some(&ev.name), ..EvInfo::default() };
        if !def.filter(EventKind::Custom).is_none_or(|c| c.passes(EventKind::Custom, &info, &def.id)) {
            k += 1;
            continue;
        }
        if shared.demoted(j) || peers.insts[k].1.over_budget(shared, set.tick_budget) {
            if fail != Verdict::Allow {
                return fail;
            }
            k += 1;
            continue;
        }
        let region = peers.region;
        let inst = &mut peers.insts[k].1;
        let generation = inst.generation();
        let frame = inst.frame();
        frame.reset(!region, source);
        if let Some((uuid, operator, name, info)) = actor {
            frame.push_player(uuid, name, operator, Some(info));
        }
        let player = actor.map(|(uuid, operator, _, _)| wit::Player { handle: frame.player_handle(generation, 0), uuid: host::wit_uuid(uuid), operator });
        let wev = wit::CustomEvent { name: ev.name.clone(), source: ev.source.to_string(), payload: ev.payload.clone(), actor: player };
        shared.stats.calls.fetch_add(1, Ordering::Relaxed);
        let outcome = inst.call(shared, set.call, |store, g, r| {
            if region { r.expect("region guest").call_on_custom(store, &wev) } else { g.expect("global guest").call_on_custom(store, &wev) }
        });
        match outcome {
            Outcome::Ok(wit::Decision::Deny) => return Verdict::Deny(inst.frame().deny_msg.take()),
            Outcome::Ok(wit::Decision::Allow) => k += 1,
            failed => {
                let id = &def.id;
                match failed {
                    Outcome::Timeout => {
                        warn!("plugin {id}: custom event handler exceeded its budget");
                        shared.strike(j, id);
                    }
                    Outcome::Trap(e) => warn!("plugin {id} trapped: {e:#}"),
                    Outcome::Ok(_) => {}
                }
                peers.insts.remove(k);
                if fail != Verdict::Allow {
                    return fail;
                }
            }
        }
    }
    Verdict::Allow
}

/// Why a player appeared in a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnReason {
    Join,
    Respawn,
    LevelChange,
}

/// What a region saw, for the next observe batch.
enum Seen {
    Broken(u32),
    Placed(u32),
    Died { cause: u32, killer: Option<u128> },
    Spawned(SpawnReason),
}

struct Observation {
    what: Seen,
    uuid: u128,
    name: String,
    operator: bool,
    info: PlayerInfo,
    pos: [i32; 3],
}

impl Observation {
    fn bit(&self) -> u8 {
        match self.what {
            Seen::Broken(_) => ObserveKinds::BLOCK_BROKEN,
            Seen::Placed(_) => ObserveKinds::BLOCK_PLACED,
            Seen::Died { .. } => ObserveKinds::PLAYER_DIED,
            Seen::Spawned(_) => ObserveKinds::PLAYER_SPAWNED,
        }
    }

    fn block(&self) -> Option<u32> {
        match self.what {
            Seen::Broken(b) | Seen::Placed(b) => Some(b),
            _ => None,
        }
    }
}

/// An entity an event is about, with its plugin data (read and written by the handlers).
pub struct EntityRef<'a> {
    pub uuid: u128,
    /// Entity type id (`Registries::entity_types`).
    pub kind: u32,
    pub pos: [f64; 3],
    pub data: &'a mut EntityData,
}

/// An item as an event reports it.
#[derive(Clone, Copy, Debug)]
pub struct ItemRef<'a> {
    /// Item id (`Registries::items`).
    pub item: u32,
    pub count: u32,
    /// The plugin tag stored in the stack, `<plugin id>:<tag>`.
    pub tag: Option<&'a str>,
}

/// How a container was clicked (`ClickType` and button of the vanilla packet, flattened).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickKind {
    Left,
    Right,
    ShiftLeft,
    ShiftRight,
    Middle,
    Drop,
    Double,
    Swap,
    Drag,
    Other,
}

/// A click in a container screen.
#[derive(Clone, Copy, Debug)]
pub struct ContainerClick<'a> {
    /// The plugin menu open, `<plugin id>:<menu id>`; none for a vanilla container.
    pub menu: Option<&'a str>,
    /// The menu type key (`minecraft:generic_9x3`).
    pub container: &'a str,
    pub slot: i32,
    pub button: u8,
    pub kind: ClickKind,
    pub clicked: Option<ItemRef<'a>>,
}

fn split_owned(full: &str) -> (&str, &str) {
    full.split_once(':').unwrap_or(("", full))
}

/// What a handler is handed besides the event record: handles of its call.
#[derive(Clone, Copy, Default)]
struct Handles {
    players: [u64; 2],
    cell: u64,
    entity: u64,
}

/// A region's instances, one per plugin that exports `region-hooks`.
struct RegionInner {
    set: Arc<PluginSet>,
    shared: Arc<Shared>,
    dim: u32,
    insts: Vec<Option<Inst>>,
    observed: Vec<Observation>,
    /// Calls made here (added to the statistics in B0: no shared counter on the hot path).
    calls: u64,
}

/// The region instance of plugin `i`, instantiated if missing (after a trap).
fn ensure(set: &PluginSet, shared: &Arc<Shared>, insts: &mut [Option<Inst>], i: usize) -> bool {
    if insts[i].is_none() && set.plugins[i].region.is_some() {
        match Inst::new(set, shared, i, true) {
            Ok((inst, _)) => insts[i] = Some(inst),
            Err(e) => warn!("plugin {}: cannot instantiate for a region: {e:#}", set.plugins[i].id),
        }
    }
    insts[i].is_some()
}

/// A trap or timeout of a region instance: the instance is replaced (its state may be
/// inconsistent), and a timeout is a strike.
fn region_failed<R>(set: &PluginSet, shared: &Shared, insts: &mut [Option<Inst>], i: usize, outcome: Outcome<R>) {
    let id = &set.plugins[i].id;
    match outcome {
        Outcome::Timeout => {
            warn!("plugin {id}: call exceeded its budget");
            shared.strike(i, id);
        }
        Outcome::Trap(e) => warn!("plugin {id} trapped: {e:#}"),
        Outcome::Ok(_) => return,
    }
    insts[i] = None;
}

impl RegionInner {
    fn new(set: Arc<PluginSet>, shared: Arc<Shared>, dim: u32) -> RegionInner {
        let mut r = RegionInner { insts: (0..set.plugins.len()).map(|_| None).collect(), set, shared, dim, observed: Vec::new(), calls: 0 };
        for i in 0..r.insts.len() {
            ensure(&r.set, &r.shared, &mut r.insts, i);
        }
        r
    }

    /// Calls every subscriber of a cancellable event in load order until one denies.
    /// `actors` are the players of the event (the first is the acting one: its bucket and its
    /// ordering key); `f` gets the plugin's index, the guest, the store and the handles of the
    /// call; `decide` the result and the denial message the handler left.
    #[allow(clippy::too_many_arguments)]
    fn cancellable<'e, R>(
        &mut self,
        kind: EventKind,
        actors: &[&Actor],
        info: EvInfo<'e>,
        cell: Option<CellKey>,
        mut entity: Option<&mut EntityRef>,
        mut f: impl FnMut(usize, &RegionGuest, &mut Store<HostState>, Handles) -> wasmtime::Result<R>,
        mut decide: impl FnMut(R, Option<Vec<Span>>) -> Option<Verdict>,
    ) -> Verdict {
        let RegionInner { set, shared, insts, calls, .. } = self;
        let set: &Arc<PluginSet> = &*set;
        let subs = set.subscribers(kind);
        if subs.is_empty() {
            return Verdict::Allow;
        }
        let actor = actors[0];
        let called = |i: usize| set.plugins[i].filter(kind).is_none_or(|c| c.passes(kind, &info, &set.plugins[i].id));
        if !subs.iter().any(|&i| called(i)) {
            return Verdict::Allow;
        }
        let policy = |i: usize| set.plugins[i].manifest.subscription(kind).map_or(FailPolicy::Open, |s| s.policy);
        // The acting player's bucket: an empty one keeps the event from every plugin.
        if !shared.take_token(actor.uuid, set.player_burst, set.player_rate) {
            shared.stats.rate_limited.fetch_add(1, Ordering::Relaxed);
            let closed = subs.iter().any(|&i| called(i) && policy(i) == FailPolicy::Closed);
            return if closed { Verdict::Deny(None) } else { Verdict::Allow };
        }
        for &i in subs {
            if !called(i) {
                continue;
            }
            let fail = if policy(i) == FailPolicy::Closed { Some(Verdict::Deny(None)) } else { None };
            if shared.demoted(i) || !ensure(set, shared, insts, i) {
                match fail {
                    Some(v) => return v,
                    None => continue,
                }
            }
            if insts[i].as_ref().expect("instance").over_budget(shared, set.tick_budget) {
                shared.stats.budget_exhausted.fetch_add(1, Ordering::Relaxed);
                match fail {
                    Some(v) => return v,
                    None => continue,
                }
            }
            *calls += 1;
            let shared_ref: &Arc<Shared> = shared;
            let call_budget = set.call;
            let set_ref: &Arc<PluginSet> = set;
            let (outcome, msg) = with_peers(set_ref, insts, i, true, |inst| {
                let generation = inst.generation();
                let frame = inst.frame();
                frame.reset(false, actor.uuid);
                for a in actors {
                    frame.push_player(a.uuid, a.name, a.operator, Some(&a.info));
                }
                frame.cells.extend(cell);
                if let Some(e) = entity.as_deref_mut() {
                    frame.entities.push((e.uuid, std::mem::take(e.data)));
                }
                let handles = Handles {
                    players: [frame.player_handle(generation, 0), frame.player_handle(generation, 1)],
                    cell: frame.cell_handle(generation, 0),
                    entity: frame.entity_handle(generation, 0),
                };
                let outcome = inst.call(shared_ref, call_budget, |store, _, region| f(i, region.expect("region guest"), store, handles));
                if let Some(e) = entity.as_deref_mut() {
                    *e.data = std::mem::take(&mut inst.frame().entities[0].1);
                }
                let msg = if matches!(outcome, Outcome::Ok(_)) { inst.frame().deny_msg.take() } else { None };
                (outcome, msg)
            });
            match outcome {
                Outcome::Ok(r) => {
                    if let Some(v) = decide(r, msg) {
                        return v;
                    }
                }
                failed => {
                    region_failed(set, shared, insts, i, failed);
                    if let Some(v) = fail {
                        return v;
                    }
                }
            }
        }
        Verdict::Allow
    }

    fn info(&self, actor: &Actor, pos: Option<[i32; 3]>) -> EvInfo<'static> {
        EvInfo { permission: actor.permission(), level: self.dim, xz: pos.map(|p| (p[0], p[2])), ..EvInfo::default() }
    }

    /// A player breaks (starts or finishes breaking) `block` at `pos` in this region's level.
    pub fn block_break(&mut self, actor: &Actor, pos: [i32; 3], block: u32) -> Verdict {
        let cell = CellKey::of_block(self.dim, pos[0], pos[2]);
        let info = EvInfo { block: Some(block), ..self.info(actor, Some(pos)) };
        let level = self.dim;
        self.cancellable(
            EventKind::BlockBreak,
            &[actor],
            info,
            Some(cell),
            None,
            // A flat record: it crosses as plain arguments, nothing is allocated in the guest.
            |_, g, store, h| {
                g.call_on_block_break(store, wit::BlockEvent { player: wit_player(actor, h.players[0]), level, pos: wit_pos(pos), block, cell: h.cell })
            },
            decision,
        )
    }

    /// `pos` is where the block (or fluid) would go, `against` the clicked block; `item` the
    /// item id in the hand used.
    pub fn block_place(&mut self, actor: &Actor, pos: [i32; 3], against: [i32; 3], item: Option<u32>) -> Verdict {
        let cell = CellKey::of_block(self.dim, pos[0], pos[2]);
        let info = self.info(actor, Some(pos));
        let level = self.dim;
        self.cancellable(
            EventKind::BlockPlace,
            &[actor],
            info,
            Some(cell),
            None,
            |_, g, store, h| {
                g.call_on_block_place(
                    store,
                    wit::PlaceEvent { player: wit_player(actor, h.players[0]), level, pos: wit_pos(pos), against: wit_pos(against), item, cell: h.cell },
                )
            },
            decision,
        )
    }

    fn entity_event(&mut self, kind: EventKind, actor: &Actor, entity: &mut EntityRef) -> Verdict {
        let p = entity.pos;
        let block = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
        let info = EvInfo { entity_type: Some(entity.kind), ..self.info(actor, Some(block)) };
        let (level, uuid, ekind) = (self.dim, entity.uuid, entity.kind);
        self.cancellable(
            kind,
            &[actor],
            info,
            Some(CellKey::of_block(self.dim, block[0], block[2])),
            Some(entity),
            |_, g, store, h| {
                let ev = wit::EntityEvent {
                    player: wit_player(actor, h.players[0]),
                    level,
                    entity: h.entity,
                    entity_uuid: host::wit_uuid(uuid),
                    kind: ekind,
                    pos: (p[0], p[1], p[2]),
                    cell: h.cell,
                };
                if kind == EventKind::EntityAttack { g.call_on_entity_attack(store, ev) } else { g.call_on_entity_interact(store, ev) }
            },
            decision,
        )
    }

    /// A player right-clicks an entity. Handlers may read and write the entity's data.
    pub fn entity_interact(&mut self, actor: &Actor, entity: &mut EntityRef) -> Verdict {
        self.entity_event(EventKind::EntityInteract, actor, entity)
    }

    /// A player hits an entity. Handlers may read and write the entity's data.
    pub fn entity_attack(&mut self, actor: &Actor, entity: &mut EntityRef) -> Verdict {
        self.entity_event(EventKind::EntityAttack, actor, entity)
    }

    /// A player is about to take `amount` damage of damage type `cause` at `pos`; `attacker`
    /// is the player responsible, if any.
    pub fn player_damage(&mut self, victim: &Actor, attacker: Option<&Actor>, pos: [i32; 3], cause: u32, amount: f32) -> Verdict {
        let info = self.info(victim, Some(pos));
        let level = self.dim;
        let mut actors = [victim, victim];
        let n = if let Some(a) = attacker {
            actors[1] = a;
            2
        } else {
            1
        };
        self.cancellable(
            EventKind::PlayerDamage,
            &actors[..n],
            info,
            Some(CellKey::of_block(self.dim, pos[0], pos[2])),
            None,
            |_, g, store, h| {
                let ev = wit::DamageEvent {
                    victim: wit_player(victim, h.players[0]),
                    level,
                    pos: wit_pos(pos),
                    cause,
                    amount,
                    attacker: attacker.map(|a| wit_player(a, h.players[1])),
                    cell: h.cell,
                };
                g.call_on_player_damage(store, ev)
            },
            decision,
        )
    }

    /// The held item is used: on the block at `target`, or in the air. Only the plugin whose
    /// tag the item carries hears of it, and plugins that subscribed to other items.
    pub fn item_use(&mut self, actor: &Actor, item: ItemRef, off_hand: bool, target: Option<[i32; 3]>) -> Verdict {
        let level = self.dim;
        let (owner, rest) = match item.tag {
            Some(t) => {
                let (o, r) = split_owned(t);
                (Some(o), Some(r))
            }
            None => (None, None),
        };
        let info = EvInfo { item: Some(item.item), tag_owner: owner, ..self.info(actor, target) };
        let set = self.set.clone();
        self.cancellable(
            EventKind::ItemUse,
            &[actor],
            info,
            None,
            None,
            |i, g, store, h| {
                // Only the owner sees the tag.
                let tag = if owner == Some(&*set.plugins[i].id) { rest.map(str::to_owned) } else { None };
                let ev = wit::ItemUseEvent {
                    player: wit_player(actor, h.players[0]),
                    level,
                    item: wit::ItemView { item: item.item, count: item.count, tag },
                    off_hand,
                    target: target.map(wit_pos),
                };
                g.call_on_item_use(store, &ev)
            },
            decision,
        )
    }

    /// A click in a container screen. A click in a plugin's menu is delivered to that plugin
    /// only, and the embedder cancels it whatever the answer; vanilla containers go to
    /// plugins subscribed with `vanilla`.
    pub fn container_click(&mut self, actor: &Actor, click: &ContainerClick) -> Verdict {
        let (owner, menu) = match click.menu {
            Some(m) => {
                let (o, r) = split_owned(m);
                (Some(o), Some(r))
            }
            None => (None, None),
        };
        let info = EvInfo { menu_owner: owner, ..self.info(actor, None) };
        self.cancellable(
            EventKind::ContainerClick,
            &[actor],
            info,
            None,
            None,
            |_, g, store, h| {
                let clicked = click.clicked.map(|c| wit::ItemView {
                    item: c.item,
                    count: c.count,
                    tag: c.tag.and_then(|t| t.split_once(':')).filter(|(o, _)| Some(*o) == owner).map(|(_, r)| r.to_owned()),
                });
                let ev = wit::ContainerClickEvent {
                    player: wit_player(actor, h.players[0]),
                    menu: menu.map(str::to_owned),
                    container: click.container.to_owned(),
                    slot: click.slot,
                    button: click.button,
                    kind: match click.kind {
                        ClickKind::Left => wit::ClickKind::Left,
                        ClickKind::Right => wit::ClickKind::Right,
                        ClickKind::ShiftLeft => wit::ClickKind::ShiftLeft,
                        ClickKind::ShiftRight => wit::ClickKind::ShiftRight,
                        ClickKind::Middle => wit::ClickKind::Middle,
                        ClickKind::Drop => wit::ClickKind::Drop,
                        ClickKind::Double => wit::ClickKind::Double,
                        ClickKind::Swap => wit::ClickKind::Swap,
                        ClickKind::Drag => wit::ClickKind::Drag,
                        ClickKind::Other => wit::ClickKind::Other,
                    },
                    clicked,
                };
                g.call_on_container_click(store, &ev)
            },
            decision,
        )
    }

    /// A vanilla or plugin command a player is about to run (`command` without the slash).
    pub fn command(&mut self, actor: &Actor, command: &str) -> Verdict {
        let info = self.info(actor, None);
        self.cancellable(
            EventKind::Command,
            &[actor],
            info,
            None,
            None,
            |_, g, store, h| g.call_on_command(store, &wit::CommandEvent { player: wit_player(actor, h.players[0]), command: command.to_owned() }),
            decision,
        )
    }

    /// A chat message: cancelled, rewritten (the last rewrite wins) or passed.
    pub fn chat(&mut self, actor: &Actor, message: &str) -> ChatOutcome {
        let mut rewrite = None;
        let info = self.info(actor, None);
        let v = self.cancellable(
            EventKind::Chat,
            &[actor],
            info,
            None,
            None,
            |_, g, store, h| g.call_on_chat(store, &wit::ChatEvent { player: wit_player(actor, h.players[0]), message: message.to_owned() }),
            |r, _| match r {
                wit::ChatVerdict::Pass => None,
                wit::ChatVerdict::Cancel => Some(Verdict::Deny(None)),
                wit::ChatVerdict::Rewrite(s) => {
                    rewrite = Some(spans(s));
                    None
                }
            },
        );
        match (v, rewrite) {
            (Verdict::Deny(_), _) => ChatOutcome::Cancel,
            (Verdict::Allow, Some(s)) => ChatOutcome::Rewrite(s),
            (Verdict::Allow, None) => ChatOutcome::Pass,
        }
    }

    /// Whether any plugin wants observe batches (callers can skip the bookkeeping).
    pub fn observing(&self) -> bool {
        !self.set.subscribers(EventKind::Observe).is_empty()
    }

    /// Whether any plugin wants observed events of `kind` (an [`ObserveKinds`] bit).
    pub fn observing_kind(&self, bit: u8) -> bool {
        self.set.subscribers(EventKind::Observe).iter().any(|&i| self.set.plugins[i].manifest.subscription(EventKind::Observe).is_some_and(|s| s.observe.has(bit)))
    }

    fn push_observation(&mut self, what: Seen, actor: &Actor, pos: [i32; 3]) {
        self.observed.push(Observation { what, uuid: actor.uuid, name: actor.name.to_owned(), operator: actor.operator, info: actor.info, pos });
    }

    /// Notes a block broken (`broken`) or placed by `actor`, for the next observe batch.
    pub fn observe_block(&mut self, broken: bool, actor: &Actor, pos: [i32; 3], block: u32) {
        if self.observing() {
            self.push_observation(if broken { Seen::Broken(block) } else { Seen::Placed(block) }, actor, pos);
        }
    }

    /// Notes a player's death for the next observe batch.
    pub fn observe_death(&mut self, actor: &Actor, pos: [i32; 3], cause: u32, killer: Option<u128>) {
        if self.observing_kind(ObserveKinds::PLAYER_DIED) {
            self.push_observation(Seen::Died { cause, killer }, actor, pos);
        }
    }

    /// Notes a player appearing in this region's level.
    pub fn observe_spawn(&mut self, actor: &Actor, pos: [i32; 3], reason: SpawnReason) {
        if self.observing_kind(ObserveKinds::PLAYER_SPAWNED) {
            self.push_observation(Seen::Spawned(reason), actor, pos);
        }
    }

    /// Sends the observations of this phase to observe subscribers, one batch per plugin
    /// (only what its subscription and filter let through). Demoted plugins still observe;
    /// failures only replace the instance (and strike).
    pub fn flush_observed(&mut self) {
        if self.observed.is_empty() {
            return;
        }
        let observed = std::mem::take(&mut self.observed);
        let RegionInner { set, shared, insts, calls, dim, .. } = self;
        let subs: Vec<usize> = set.subscribers(EventKind::Observe).to_vec();
        for i in subs {
            let sub = set.plugins[i].manifest.subscription(EventKind::Observe).expect("subscription");
            let mask = sub.observe;
            let filter = set.plugins[i].filter(EventKind::Observe);
            let mine: Vec<&Observation> = observed
                .iter()
                .filter(|o| {
                    mask.has(o.bit())
                        && filter.is_none_or(|c| {
                            let info = EvInfo {
                                permission: if o.operator { 4 } else { 0 },
                                level: *dim,
                                xz: Some((o.pos[0], o.pos[2])),
                                block: o.block(),
                                ..EvInfo::default()
                            };
                            c.passes(EventKind::Observe, &info, &set.plugins[i].id)
                        })
                })
                .collect();
            if mine.is_empty() || !ensure(set, shared, insts, i) {
                continue;
            }
            *calls += 1;
            let (set_ref, shared_ref, dim_v) = (&*set, &*shared, *dim);
            let serial = set.serial;
            let outcome = with_peers(set, insts, i, true, |inst| {
                let generation = inst.generation();
                let frame = inst.frame();
                frame.reset(false, mine[0].uuid);
                for o in &mine {
                    if !frame.players.contains(&o.uuid) {
                        frame.push_player(o.uuid, &o.name, o.operator, Some(&o.info));
                    }
                }
                let batch: Vec<wit::Observed> = mine
                    .iter()
                    .map(|o| {
                        let h = frame.player_handle(generation, frame.players.iter().position(|p| *p == o.uuid).unwrap());
                        let player = wit::Player { handle: h, uuid: host::wit_uuid(o.uuid), operator: o.operator };
                        let block = |b| wit::ObservedBlock { player, level: dim_v, pos: wit_pos(o.pos), block: b };
                        match o.what {
                            Seen::Broken(b) => wit::Observed::BlockBroken(block(b)),
                            Seen::Placed(b) => wit::Observed::BlockPlaced(block(b)),
                            Seen::Died { cause, killer } => wit::Observed::PlayerDied(wit::DeathEvent {
                                player,
                                level: dim_v,
                                pos: wit_pos(o.pos),
                                cause,
                                killer: killer.map(host::wit_uuid),
                            }),
                            Seen::Spawned(reason) => wit::Observed::PlayerSpawned(wit::SpawnEvent {
                                player,
                                level: dim_v,
                                pos: wit_pos(o.pos),
                                reason: match reason {
                                    SpawnReason::Join => wit::SpawnReason::Join,
                                    SpawnReason::Respawn => wit::SpawnReason::Respawn,
                                    SpawnReason::LevelChange => wit::SpawnReason::LevelChange,
                                },
                            }),
                        }
                    })
                    .collect();
                inst.call(shared_ref, serial, |store, _, g| g.expect("region guest").call_on_observe(store, &batch))
            });
            let _ = set_ref;
            if !matches!(outcome, Outcome::Ok(())) {
                region_failed(set, shared, insts, i, outcome);
            }
        }
    }

    /// A task or results delivery in this region's instance of plugin `i` (B0).
    fn run_in(&mut self, i: usize, player: Option<&PlayerAt>, cell: Option<CellKey>, f: RegionCall) -> bool {
        let RegionInner { set, shared, insts, calls, .. } = self;
        if !ensure(set, shared, insts, i) {
            return false;
        }
        *calls += 1;
        let serial = set.serial;
        let shared_ref: &Arc<Shared> = shared;
        let outcome = with_peers(set, insts, i, true, |inst| {
            let generation = inst.generation();
            let frame = inst.frame();
            frame.reset(false, player.map_or(0, |p| p.uuid));
            if let Some(p) = player {
                frame.push_player(p.uuid, &p.name, p.operator, Some(&p.info));
            }
            frame.cells.extend(cell);
            let ph = frame.player_handle(generation, 0);
            let ch = frame.cell_handle(generation, 0);
            let p = player.map(|p| wit::Player { handle: ph, uuid: host::wit_uuid(p.uuid), operator: p.operator });
            inst.call(shared_ref, serial, |store, _, g| {
                let g = g.expect("region guest");
                match f {
                    RegionCall::Task(handle, id) => g.call_on_task(store, wit::TaskEvent { handle, id, player: p, cell: cell.map(|_| ch) }),
                    RegionCall::Results(r) => g.call_on_results(store, p, &r),
                }
            })
        });
        let ok = matches!(outcome, Outcome::Ok(()));
        if !ok {
            region_failed(set, shared, insts, i, outcome);
        }
        ok
    }

    /// Replaces the set (a reload) and plugin `i`'s instance.
    fn swap(&mut self, set: Arc<PluginSet>, i: usize) {
        self.flush_observed();
        self.set = set;
        self.insts[i] = None;
        ensure(&self.set, &self.shared, &mut self.insts, i);
    }

    /// Calls made since the last call (the runtime adds them to its statistics).
    fn take_calls(&mut self) -> u64 {
        std::mem::take(&mut self.calls)
    }
}

/// A lock for data that one thread at a time uses (a region's instances: the thread working
/// the region, which the player's damage gate runs on too): taking it is one compare-and-swap
/// and releasing it one store, where a `Mutex` costs several times that on every event. It
/// spins (and yields) if it is ever contended, which the region discipline does not allow to
/// last.
struct RegionLock<T> {
    locked: AtomicBool,
    value: std::cell::UnsafeCell<T>,
}

// SAFETY: `value` is only reached through `lock`, which hands it out to one holder at a time.
unsafe impl<T: Send> Sync for RegionLock<T> {}
unsafe impl<T: Send> Send for RegionLock<T> {}

struct RegionGuard<'a, T>(&'a RegionLock<T>);

impl<T> RegionLock<T> {
    fn new(value: T) -> Self {
        RegionLock { locked: AtomicBool::new(false), value: std::cell::UnsafeCell::new(value) }
    }

    fn lock(&self) -> RegionGuard<'_, T> {
        let mut spins = 0u32;
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins < 64 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        RegionGuard(self)
    }
}

impl<T> std::ops::Deref for RegionGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.0.value.get() }
    }
}

impl<T> std::ops::DerefMut for RegionGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.0.value.get() }
    }
}

impl<T> Drop for RegionGuard<'_, T> {
    fn drop(&mut self) {
        self.0.locked.store(false, Ordering::Release);
    }
}

struct RegionShared {
    inner: RegionLock<RegionInner>,
    /// Bit `EventKind::index()` set: some plugin has instances here that subscribed to it.
    subs: std::sync::atomic::AtomicU32,
}

/// A region's plugin instances, one per plugin that exports `region-hooks`. A handle: clones
/// share the instances, which sit behind a lock that is taken per event and never contended
/// (one thread works a region at a time). The handle lets a player carry the damage gate of
/// its region ([`RegionPlugins::player_damage`] is called from inside the damage code).
#[derive(Clone)]
pub struct RegionPlugins(Arc<RegionShared>);

fn subscription_mask(set: &PluginSet) -> u32 {
    EventKind::ALL.iter().filter(|k| !set.subscribers(**k).is_empty()).fold(0, |m, k| m | 1 << k.index())
}

impl RegionPlugins {
    fn new(set: Arc<PluginSet>, shared: Arc<Shared>, dim: u32) -> RegionPlugins {
        let mask = subscription_mask(&set);
        let inner = RegionInner::new(set, shared, dim);
        RegionPlugins(Arc::new(RegionShared { inner: RegionLock::new(inner), subs: std::sync::atomic::AtomicU32::new(mask) }))
    }

    fn lock(&self) -> RegionGuard<'_, RegionInner> {
        self.0.inner.lock()
    }

    /// Whether any plugin has handlers here for `kind` (a lock-free check: callers skip the
    /// bookkeeping of events nobody wants).
    pub fn subscribed(&self, kind: EventKind) -> bool {
        self.0.subs.load(Ordering::Relaxed) & (1 << kind.index()) != 0
    }

    /// A player breaks (starts or finishes breaking) `block` at `pos` in this region's level.
    pub fn block_break(&self, actor: &Actor, pos: [i32; 3], block: u32) -> Verdict {
        if !self.subscribed(EventKind::BlockBreak) {
            return Verdict::Allow;
        }
        self.lock().block_break(actor, pos, block)
    }

    /// `pos` is where the block (or fluid) would go, `against` the clicked block; `item` the
    /// item id in the hand used.
    pub fn block_place(&self, actor: &Actor, pos: [i32; 3], against: [i32; 3], item: Option<u32>) -> Verdict {
        if !self.subscribed(EventKind::BlockPlace) {
            return Verdict::Allow;
        }
        self.lock().block_place(actor, pos, against, item)
    }

    /// A player right-clicks an entity. Handlers may read and write the entity's data.
    pub fn entity_interact(&self, actor: &Actor, entity: &mut EntityRef) -> Verdict {
        if !self.subscribed(EventKind::EntityInteract) {
            return Verdict::Allow;
        }
        self.lock().entity_interact(actor, entity)
    }

    /// A player hits an entity. Handlers may read and write the entity's data.
    pub fn entity_attack(&self, actor: &Actor, entity: &mut EntityRef) -> Verdict {
        if !self.subscribed(EventKind::EntityAttack) {
            return Verdict::Allow;
        }
        self.lock().entity_attack(actor, entity)
    }

    /// A player is about to take `amount` damage of damage type `cause` at `pos`; `attacker`
    /// is the player responsible, if any.
    pub fn player_damage(&self, victim: &Actor, attacker: Option<&Actor>, pos: [i32; 3], cause: u32, amount: f32) -> Verdict {
        if !self.subscribed(EventKind::PlayerDamage) {
            return Verdict::Allow;
        }
        self.lock().player_damage(victim, attacker, pos, cause, amount)
    }

    /// The held item is used: on the block at `target`, or in the air.
    pub fn item_use(&self, actor: &Actor, item: ItemRef, off_hand: bool, target: Option<[i32; 3]>) -> Verdict {
        if !self.subscribed(EventKind::ItemUse) {
            return Verdict::Allow;
        }
        self.lock().item_use(actor, item, off_hand, target)
    }

    /// A click in a container screen.
    pub fn container_click(&self, actor: &Actor, click: &ContainerClick) -> Verdict {
        if !self.subscribed(EventKind::ContainerClick) {
            return Verdict::Allow;
        }
        self.lock().container_click(actor, click)
    }

    /// A vanilla or plugin command a player is about to run (`command` without the slash).
    pub fn command(&self, actor: &Actor, command: &str) -> Verdict {
        if !self.subscribed(EventKind::Command) {
            return Verdict::Allow;
        }
        self.lock().command(actor, command)
    }

    /// A chat message: cancelled, rewritten (the last rewrite wins) or passed.
    pub fn chat(&self, actor: &Actor, message: &str) -> ChatOutcome {
        if !self.subscribed(EventKind::Chat) {
            return ChatOutcome::Pass;
        }
        self.lock().chat(actor, message)
    }

    /// Whether any plugin wants observe batches (callers can skip the bookkeeping).
    pub fn observing(&self) -> bool {
        self.subscribed(EventKind::Observe)
    }

    /// Whether any plugin wants observed events of `kind` (an [`ObserveKinds`] bit).
    pub fn observing_kind(&self, bit: u8) -> bool {
        self.observing() && self.lock().observing_kind(bit)
    }

    /// Notes a block broken (`broken`) or placed by `actor`, for the next observe batch.
    pub fn observe_block(&self, broken: bool, actor: &Actor, pos: [i32; 3], block: u32) {
        if self.observing() {
            self.lock().observe_block(broken, actor, pos, block);
        }
    }

    /// Notes a player's death for the next observe batch.
    pub fn observe_death(&self, actor: &Actor, pos: [i32; 3], cause: u32, killer: Option<u128>) {
        if self.observing() {
            self.lock().observe_death(actor, pos, cause, killer);
        }
    }

    /// Notes a player appearing in this region's level.
    pub fn observe_spawn(&self, actor: &Actor, pos: [i32; 3], reason: SpawnReason) {
        if self.observing() {
            self.lock().observe_spawn(actor, pos, reason);
        }
    }

    /// Sends the observations of this phase to observe subscribers.
    pub fn flush_observed(&self) {
        if self.observing() {
            self.lock().flush_observed();
        }
    }

    fn run_in(&self, i: usize, player: Option<&PlayerAt>, cell: Option<CellKey>, f: RegionCall) -> bool {
        self.lock().run_in(i, player, cell, f)
    }

    fn swap(&self, set: Arc<PluginSet>, i: usize) {
        let mask = subscription_mask(&set);
        let mut inner = self.lock();
        inner.swap(set, i);
        self.0.subs.store(mask, Ordering::Relaxed);
    }

    fn take_calls(&self) -> u64 {
        self.lock().take_calls()
    }

    fn peek_calls(&self) -> u64 {
        self.lock().calls
    }
}

enum RegionCall {
    Task(u64, u64),
    Results(Vec<wit::OpResult>),
}

fn decision(d: wit::Decision, msg: Option<Vec<Span>>) -> Option<Verdict> {
    match d {
        wit::Decision::Allow => None,
        wit::Decision::Deny => Some(Verdict::Deny(msg)),
    }
}

/// Where the embedder has an online player (tasks and results follow players).
#[derive(Clone, Debug)]
pub struct PlayerAt {
    pub uuid: u128,
    pub level: u32,
    pub region: u64,
    pub name: String,
    pub operator: bool,
    pub info: PlayerInfo,
}

/// What B0 needs to know about the world to route tasks and results.
pub trait World {
    fn player(&self, uuid: u128) -> Option<PlayerAt>;
    /// The region owning block column (x, z) of a level, if it is loaded.
    fn owner(&self, level: u32, x: i32, z: i32) -> Option<u64>;
}

/// A world without players or loaded regions (tests).
pub struct NoWorld;

impl World for NoWorld {
    fn player(&self, _: u128) -> Option<PlayerAt> {
        None
    }
    fn owner(&self, _: u32, _: i32, _: i32) -> Option<u64> {
        None
    }
}

/// Increments the engine's epoch until dropped.
struct Ticker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Ticker {
    fn start(engine: Engine, period: Duration, count: Arc<AtomicU64>) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("kiln-plugin-epoch".into())
            .spawn(move || {
                // Sleeps overshoot (about 1 ms on Windows), so the epoch follows elapsed time:
                // a deadline of n ticks is n periods however coarse the wake-ups are.
                let start = std::time::Instant::now();
                let mut epoch = 0u128;
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(period);
                    let due = start.elapsed().as_nanos() / period.as_nanos().max(1);
                    while epoch < due {
                        engine.increment_epoch();
                        epoch += 1;
                    }
                    count.store(epoch as u64, Ordering::Relaxed);
                }
            })
            .expect("epoch thread");
        Ticker { stop, thread: Some(thread) }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A compiled replacement waiting for B0.
struct Staged {
    plugin: usize,
    def: Result<PluginDef>,
    requester: Option<u128>,
}

/// What a reload did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reloaded {
    pub id: String,
    pub generation: u32,
    /// Bytes of the state blob handed over.
    pub blob: Option<usize>,
    pub cancelled_tasks: usize,
    pub region_instances: usize,
    /// The command registrations changed (the embedder re-registers and resends trees).
    pub commands_changed: bool,
}

/// All plugins: their global instances, the region instance sets, and the namespaces.
pub struct PluginRuntime {
    set: Arc<PluginSet>,
    cfg: SetCfg,
    shared: Arc<Shared>,
    globals: Vec<Option<Inst>>,
    regions: HashMap<(u32, u64), RegionPlugins>,
    commands: Vec<CommandReg>,
    cache_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    staged: Arc<Mutex<Vec<Staged>>>,
    reloaded: Vec<Reloaded>,
    _ticker: Option<Ticker>,
}

fn engine(cfg: &RuntimeConfig) -> Result<Engine> {
    let mut c = wasmtime::Config::new();
    c.wasm_component_model(true);
    match cfg.mode {
        ExecMode::Ordered => c.epoch_interruption(true),
        ExecMode::Strict => c.consume_fuel(true),
    };
    // Synchronous hot path: no component-model-async machinery in the stores.
    c.concurrency_support(false);
    if cfg.pool_instances > 0 {
        let n = cfg.pool_instances;
        let mut p = wasmtime::PoolingAllocationConfig::default();
        p.total_component_instances(n)
            .total_core_instances(n * 8)
            .total_memories(n * 2)
            .total_tables(n * 8)
            .max_core_instances_per_component(16)
            .max_memories_per_component(4)
            .max_tables_per_component(16)
            .max_memory_size(host::MEMORY_LIMIT);
        c.allocation_strategy(wasmtime::InstanceAllocationStrategy::Pooling(p));
    }
    Engine::new(&c).map_err(anyhow::Error::from)
}

/// Compiles and links one plugin (off the tick for reloads).
#[allow(clippy::too_many_arguments)]
fn prepare(
    engine: &Engine,
    manifest: Manifest,
    wasm: &[u8],
    source: Option<PathBuf>,
    generation: u32,
    cache_dir: Option<&Path>,
    data_root: Option<&Path>,
    registries: &Registries,
    spawn: [i32; 3],
    stats: Option<&Stats>,
    tasks_engine: Option<&Engine>,
) -> Result<PluginDef> {
    let (component, hit) = cache::component(engine, wasm, cache_dir)?;
    if hit && let Some(s) = stats {
        s.cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    let linker = host::linker(engine, &manifest)?;
    let pre = linker
        .instantiate_pre(&component)
        .map_err(|e| e.context("an import is not linked: is a capability missing from the manifest?"))?;
    let global = GlobalIndices::new(&pre)?;
    let region = RegionIndices::new(&pre).ok();
    let data_dir = match (data_root, manifest.has(Capability::FsData)) {
        (Some(root), true) => Some(root.join("data").join(&manifest.id)),
        _ => None,
    };
    let config = manifest.config.iter().map(|(k, v)| wit::ConfigEntry { key: k.clone(), value: v.clone() }).collect();
    let init = wit::InitInfo { id: manifest.id.clone(), config, spawn: wit_pos(spawn), levels: registries.levels.clone(), generation };
    let filters = EventKind::ALL
        .iter()
        .map(|k| {
            // Item uses and container clicks always have their default rule (own items and
            // menus only).
            let always = matches!(k, EventKind::ItemUse | EventKind::ContainerClick);
            manifest.subscription(*k).filter(|s| always || !s.filter.is_empty()).map(|s| Compiled::new(&s.filter, registries, spawn, &manifest.id))
        })
        .collect();
    let raises = manifest.has(Capability::EventsRaise);
    // The async-tasks component compiles for its own engine.
    let tasks = match (&manifest.tasks_wasm, tasks_engine) {
        (Some(bytes), Some(e)) => {
            let (c, _) = cache::component(e, bytes, cache_dir).context("the tasks component")?;
            let grants = async_tasks::TaskGrants {
                id: manifest.id.as_str().into(),
                hosts: manifest.http_hosts().map(str::to_owned).collect(),
                timers: manifest.has(Capability::Timers),
                storage: manifest.has(Capability::Storage),
                data_dir: data_root.map(|r| r.join("tasks").join(&manifest.id)),
            };
            Some((c, grants))
        }
        (Some(_), None) => bail!("the async-tasks engine could not be started"),
        _ => None,
    };
    Ok(PluginDef { id: manifest.id.as_str().into(), manifest, pre, global, region, data_dir, source, init, generation, filters, raises, tasks })
}

fn read_plugin(dir: &Path) -> Result<(Manifest, Vec<u8>)> {
    let mut manifest = Manifest::parse(&std::fs::read_to_string(dir.join("plugin.toml"))?)?;
    if let Some(name) = &manifest.tasks {
        manifest.tasks_wasm = Some(std::fs::read(dir.join(name)).with_context(|| name.clone())?);
    }
    let wasm = std::fs::read(dir.join("plugin.wasm")).context("plugin.wasm")?;
    Ok((manifest, wasm))
}

impl PluginRuntime {
    /// Loads every `<dir>/<name>/plugin.toml` + `plugin.wasm`, in directory-name order.
    /// A plugin that fails to load is skipped with a warning.
    pub fn load_dir(dir: &Path, cfg: RuntimeConfig) -> Result<PluginRuntime> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("plugin directory {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("plugin.toml").is_file())
            .collect();
        dirs.sort();
        let mut found = Vec::new();
        for d in dirs {
            match read_plugin(&d) {
                Ok((m, w)) => found.push((m, w, Some(d))),
                Err(e) => warn!("skipping plugin {}: {e:#}", d.display()),
            }
        }
        Self::build(found, cfg)
    }

    /// Compiles, links and instantiates the given plugins (manifest and component bytes).
    pub fn new(plugins: Vec<(Manifest, Vec<u8>)>, cfg: RuntimeConfig) -> Result<PluginRuntime> {
        Self::build(plugins.into_iter().map(|(m, w)| (m, w, None)).collect(), cfg)
    }

    fn build(plugins: Vec<(Manifest, Vec<u8>, Option<PathBuf>)>, cfg: RuntimeConfig) -> Result<PluginRuntime> {
        let engine = engine(&cfg)?;
        let mut defs: Vec<Arc<PluginDef>> = Vec::new();
        let stats = Stats::default();
        let strict = cfg.mode == ExecMode::Strict;
        // The async-tasks worker starts when some plugin ships a tasks component (strict mode has
        // none: its jobs fail at once, since their outcomes depend on wall-clock time).
        let worker = if !strict && plugins.iter().any(|(m, _, _)| m.tasks_wasm.is_some()) {
            match async_tasks::AsyncTasks_::start() {
                Ok(w) => Some(w),
                // (Plugins with a tasks component are then skipped, with the reason.)
                Err(e) => {
                    warn!("async tasks: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        for (mut manifest, wasm, source) in plugins {
            if strict {
                manifest.tasks_wasm = None;
            }
            if defs.iter().any(|d| d.manifest.id == manifest.id) {
                warn!("skipping plugin {}: duplicate id", manifest.id);
                continue;
            }
            let id = manifest.id.clone();
            let r = prepare(&engine, manifest, &wasm, source, 0, cfg.cache_dir.as_deref(), cfg.data_dir.as_deref(), &cfg.registries, cfg.spawn, Some(&stats), worker.as_ref().map(|w| &w.engine));
            match r {
                Ok(d) => defs.push(Arc::new(d)),
                Err(e) => warn!("skipping plugin {id}: {e:#}"),
            }
        }
        let set_cfg = if strict {
            SetCfg {
                call: Budget::Fuel(cfg.call_fuel),
                serial: Budget::Fuel(cfg.call_fuel.saturating_mul(50)),
                init: Budget::Fuel(cfg.call_fuel.saturating_mul(500)),
                tick_budget: cfg.tick_fuel,
                player_burst: cfg.player_burst,
                player_rate: cfg.player_events_per_second,
                spawn: cfg.spawn,
            }
        } else {
            let per = cfg.epoch_tick.as_nanos().max(1);
            SetCfg {
                call: Budget::Epoch((cfg.call_budget.as_nanos() / per).max(1) as u64 + 1),
                serial: Budget::Epoch(((cfg.call_budget.as_nanos() * 50) / per).max(1) as u64 + 1),
                // Generous: first calls allocate and warm up.
                init: Budget::Epoch((Duration::from_secs(1).as_nanos() / per) as u64),
                tick_budget: (cfg.tick_budget.as_nanos() / per).max(1) as u64,
                player_burst: cfg.player_burst,
                player_rate: cfg.player_events_per_second,
                spawn: cfg.spawn,
            }
        };
        let persist = cfg.data_dir.as_ref().map(|root| Persist {
            root: root.clone(),
            levels: cfg.registries.levels.clone(),
            ids: defs.iter().map(|d| d.manifest.id.clone()).collect(),
            sidecars: cfg.cell_sidecars.clone(),
        });
        let globals = persist.as_ref().map(Persist::load_globals).unwrap_or_default();
        let epoch = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            tick: AtomicU64::new(0),
            strict,
            seed: cfg.seed,
            registries: cfg.registries.clone(),
            players: Mutex::new(FastMap::default()),
            cells: Mutex::new(CellTable::default()),
            snapshot: Mutex::new(Arc::new(globals.clone())),
            globals: Mutex::new(globals),
            globals_dirty: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
            deliveries: Mutex::new(Vec::new()),
            tasks: Mutex::new(Tasks::default()),
            outbox: Mutex::new(Vec::new()),
            async_worker: worker.map_or_else(std::sync::OnceLock::new, |w| std::sync::OnceLock::from(Ok(w))),
            jobs_inflight: Mutex::new(HashMap::new()),
            online: Mutex::new(Arc::new(Vec::new())),
            health: defs.iter().map(|_| Health { strikes: Mutex::new(VecDeque::new()), demoted: AtomicBool::new(false) }).collect(),
            wants_results: defs.iter().map(|d| AtomicBool::new(d.manifest.subscription(EventKind::OpResults).is_some())).collect(),
            seqs: Mutex::new(FastMap::default()),
            buckets: (0..64).map(|_| Mutex::new(FastMap::default())).collect(),
            epoch: epoch.clone(),
            persist,
            stats,
        });
        let set = Arc::new(PluginSet::build(engine.clone(), defs, &set_cfg));
        let ticker = (!strict).then(|| Ticker::start(engine, cfg.epoch_tick, epoch));
        let mut rt = PluginRuntime {
            set,
            cfg: set_cfg,
            shared,
            globals: Vec::new(),
            regions: HashMap::new(),
            commands: Vec::new(),
            cache_dir: cfg.cache_dir,
            data_dir: cfg.data_dir,
            staged: Arc::new(Mutex::new(Vec::new())),
            reloaded: Vec::new(),
            _ticker: ticker,
        };
        for i in 0..rt.set.plugins.len() {
            // The tasks component first, so that jobs submitted while the plugin starts find it.
            if let (Some((c, g)), Some(w)) = (&rt.set.plugins[i].tasks, rt.shared.started_worker()) {
                w.load(i, 0, c.clone(), g.clone());
            }
            let inst = rt.start_global(i, None);
            rt.globals.push(inst);
            let def = &rt.set.plugins[i];
            info!("plugin {} {} loaded ({})", def.id, def.manifest.version, if def.region.is_some() { "global and region worlds" } else { "global world" });
        }
        *rt.shared.snapshot.lock().unwrap() = Arc::new(rt.shared.globals.lock().unwrap().clone());
        Ok(rt)
    }

    /// A global instance of plugin `i`: `init` (its commands replace the plugin's
    /// registrations), then `on-enable(blob)`.
    fn start_global(&mut self, i: usize, blob: Option<Vec<u8>>) -> Option<Inst> {
        let def = self.set.plugins[i].clone();
        let (mut inst, specs) = match Inst::new(&self.set, &self.shared, i, false) {
            Ok(x) => x,
            Err(e) => {
                warn!("plugin {}: global instance failed: {e:#}", def.id);
                return None;
            }
        };
        self.commands.retain(|c| c.plugin != i);
        if def.manifest.has(Capability::CommandRegister) {
            self.commands.extend(specs.into_iter().map(|s| CommandReg { plugin: i, name: s.name, permission: s.permission.min(4) }));
        } else if !specs.is_empty() {
            warn!("plugin {}: commands ignored without the command.register capability", def.id);
        }
        inst.frame().reset(true, 0);
        let outcome = inst.call(&self.shared, self.set.init, |store, g, _| g.expect("global guest").call_on_enable(store, blob.as_deref()));
        match outcome {
            Outcome::Ok(()) => Some(inst),
            Outcome::Timeout => {
                warn!("plugin {}: on-enable ran out of budget", def.id);
                None
            }
            Outcome::Trap(e) => {
                warn!("plugin {}: on-enable trapped: {e:#}", def.id);
                None
            }
        }
    }

    /// Plugin ids in load order.
    pub fn ids(&self) -> Vec<&str> {
        self.set.plugins.iter().map(|p| p.manifest.id.as_str()).collect()
    }

    pub fn plugin_index(&self, id: &str) -> Option<usize> {
        self.set.plugins.iter().position(|p| p.manifest.id == id)
    }

    pub fn manifest(&self, plugin: usize) -> &Manifest {
        &self.set.plugins[plugin].manifest
    }

    pub fn generation(&self, plugin: usize) -> u32 {
        self.set.plugins[plugin].generation
    }

    pub fn commands(&self) -> &[CommandReg] {
        &self.commands
    }

    pub fn stats(&self) -> &Stats {
        &self.shared.stats
    }

    /// Every statistic by name, with the calls regions made since the last B0.
    pub fn stat_values(&self) -> Vec<(&'static str, u64)> {
        let local: u64 = self.regions.values().map(RegionPlugins::peek_calls).sum();
        self.shared.stats.get().iter().map(|&(k, v)| (k, if k == "calls" { v + local } else { v })).collect()
    }

    /// One statistic by name (see [`stat_values`](Self::stat_values)).
    pub fn stat(&self, name: &str) -> u64 {
        self.stat_values().into_iter().find(|(k, _)| *k == name).map_or(0, |(_, v)| v)
    }

    /// Adds the regions' call counts to the shared statistics.
    fn fold_calls(&mut self) {
        let n: u64 = self.regions.values().map(RegionPlugins::take_calls).sum();
        self.shared.stats.calls.fetch_add(n, Ordering::Relaxed);
    }

    pub fn is_demoted(&self, plugin: usize) -> bool {
        self.shared.demoted(plugin)
    }

    pub fn tick(&self) -> u64 {
        self.shared.tick()
    }

    /// Tasks scheduled and not yet run.
    pub fn pending_tasks(&self) -> usize {
        let t = self.shared.tasks.lock().unwrap();
        t.table.len() + t.fresh.len()
    }

    /// B0: brings the region instance sets of level `dim` in line with its regions: new
    /// regions (splits, new areas) get fresh instances, regions that merged away or died
    /// lose theirs. Their queued operations, tasks and messages live in the host, so nothing
    /// is lost; guest memory is not carried over.
    pub fn sync_regions(&mut self, dim: u32, ids: impl IntoIterator<Item = u64>) {
        let ids: std::collections::HashSet<u64> = ids.into_iter().collect();
        self.fold_calls();
        self.regions.retain(|&(d, r), rp| {
            let keep = d != dim || ids.contains(&r);
            if !keep {
                rp.flush_observed();
            }
            keep
        });
        self.fold_calls();
        let mut new: Vec<u64> = ids.into_iter().filter(|r| !self.regions.contains_key(&(dim, *r))).collect();
        new.sort_unstable();
        for r in new {
            let rp = RegionPlugins::new(self.set.clone(), self.shared.clone(), dim);
            self.regions.insert((dim, r), rp);
        }
    }

    pub fn region_mut(&mut self, dim: u32, region: u64) -> Option<&mut RegionPlugins> {
        self.regions.get_mut(&(dim, region))
    }

    /// Every region's instance set, for handing out to parallel region work.
    pub fn regions_mut(&mut self) -> impl Iterator<Item = ((u32, u64), &mut RegionPlugins)> {
        self.regions.iter_mut().map(|(k, v)| (*k, v))
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// B0 without a world (tests): tasks that follow players are cancelled, position tasks
    /// wait.
    pub fn begin_tick(&mut self) {
        self.begin_tick_in(&NoWorld);
    }

    /// B0: the tick advances; staged reloads swap in; the atomic operations queued since the
    /// last tick apply in (tick, source, arrival) order, which does not depend on threads or
    /// regions; the snapshot the region contexts read refreshes; results go back to their
    /// sources; due tasks run.
    pub fn begin_tick_in(&mut self, world: &dyn World) {
        self.shared.tick.fetch_add(1, Ordering::Relaxed);
        self.shared.seqs.lock().unwrap().clear();
        self.fold_calls();
        let tick = self.shared.tick();
        let staged = std::mem::take(&mut *self.staged.lock().unwrap());
        for s in staged {
            let id = self.set.plugins[s.plugin].id.clone();
            let r = s.def.and_then(|def| self.swap(s.plugin, def));
            let text = match &r {
                Ok(r) => {
                    info!("plugin {id} reloaded: {r:?}");
                    format!("Plugin {id} reloaded (generation {}, {} tasks rescheduled or dropped).", r.generation, r.cancelled_tasks)
                }
                Err(e) => {
                    warn!("plugin {id}: reload failed: {e:#}");
                    format!("Plugin {id} was not reloaded: {e:#}")
                }
            };
            if let Some(to) = s.requester {
                let color = if r.is_ok() { "green" } else { "red" };
                let effect = Effect {
                    plugin: s.plugin,
                    plugin_id: id.clone(),
                    generation: self.set.plugins[s.plugin].generation,
                    source: to,
                    ticket: 0,
                    kind: EffectKind::Message { to: Some(to), text: vec![Span::colored(text, color)] },
                };
                self.shared.outbox.lock().unwrap().push((tick, to, effect));
            }
            if let Ok(r) = r {
                self.reloaded.push(r);
            }
        }
        let mut pending = std::mem::take(&mut *self.shared.pending.lock().unwrap());
        let changed = !pending.is_empty() || self.shared.globals_dirty.swap(false, Ordering::Relaxed);
        if !pending.is_empty() {
            pending.sort_by_key(|p| (p.tick, p.source));
            let mut g = self.shared.globals.lock().unwrap();
            let mut out = Vec::new();
            for p in pending {
                let r = apply_op(g.entry(p.plugin), p.ticket, p.op);
                if self.shared.wants_results[p.plugin].load(Ordering::Relaxed) {
                    out.push(Delivery { plugin: p.plugin, generation: p.generation, source: p.source, result: r });
                }
            }
            drop(g);
            self.shared.deliveries.lock().unwrap().extend(out);
        }
        if changed {
            *self.shared.snapshot.lock().unwrap() = Arc::new(self.shared.globals.lock().unwrap().clone());
        }
        self.shared.collect_jobs(tick);
        self.deliver_results(world);
        self.run_tasks(world, tick);
    }

    /// Results of last tick's operations, to the region holding their source player (else
    /// the global instance), one call per plugin and destination.
    fn deliver_results(&mut self, world: &dyn World) {
        let all = std::mem::take(&mut *self.shared.deliveries.lock().unwrap());
        if all.is_empty() {
            return;
        }
        // One call per plugin, destination and source player, so that the handler gets the
        // player the operations came from.
        type Groups = BTreeMap<(usize, Option<(u32, u64)>, u128), (Option<PlayerAt>, Vec<wit::OpResult>)>;
        let mut groups = Groups::new();
        for d in all {
            if d.generation != self.set.plugins[d.plugin].generation {
                continue; // drained: the generation that asked is gone
            }
            let at = if d.source != 0 { world.player(d.source) } else { None };
            let region = at.as_ref().filter(|_| self.set.plugins[d.plugin].region.is_some()).map(|p| (p.level, p.region));
            let e = groups.entry((d.plugin, region, d.source)).or_insert_with(|| (at, Vec::new()));
            e.1.push(d.result);
        }
        for ((plugin, region, _), (player, results)) in groups {
            let n = results.len() as u64;
            let delivered = match region.and_then(|r| self.regions.get_mut(&r)) {
                Some(rp) => rp.run_in(plugin, player.as_ref(), None, RegionCall::Results(results)),
                None => {
                    let actor = player.as_ref().map(|p| Actor { uuid: p.uuid, name: &p.name, operator: p.operator, info: p.info });
                    self.global_call(plugin, actor.as_ref(), |store, g, p| g.call_on_results(store, p, &results))
                }
            };
            if delivered {
                self.shared.stats.results_delivered.fetch_add(n, Ordering::Relaxed);
            }
        }
    }

    /// Runs the tasks due this tick; notices of tasks that cannot run go to their global
    /// instances.
    fn run_tasks(&mut self, world: &dyn World, tick: u64) {
        let due = {
            let mut t = self.shared.tasks.lock().unwrap();
            let t = &mut *t;
            for (key, task) in std::mem::take(&mut t.fresh) {
                t.by_handle.insert((task.plugin, task.handle), key);
                t.table.insert(key, task);
            }
            for c in std::mem::take(&mut t.cancels) {
                if let Some(key) = t.by_handle.remove(&c) {
                    t.table.remove(&key);
                }
            }
            let mut due = Vec::new();
            while let Some(e) = t.table.first_entry() {
                if e.key().0 > tick {
                    break;
                }
                let task = e.remove();
                t.by_handle.remove(&(task.plugin, task.handle));
                due.push(task);
            }
            due
        };
        let mut notices: BTreeMap<usize, Vec<wit::CancelledTask>> = BTreeMap::new();
        let mut waiting = Vec::new();
        for task in due {
            let def = self.set.plugins[task.plugin].clone();
            if task.generation != def.generation {
                continue; // cancelled with a notice at the reload
            }
            let (player, cell, region) = match task.target {
                TaskTarget::Global => (None, None, None),
                TaskTarget::Player(uuid) => match world.player(uuid) {
                    Some(p) => {
                        let r = (p.level, p.region);
                        (Some(p), None, Some(r))
                    }
                    None => {
                        notices.entry(task.plugin).or_default().push(wit::CancelledTask {
                            id: task.id,
                            target: wit::TaskTarget::Player(host::wit_uuid(uuid)),
                            remaining_ticks: 0,
                            reason: wit::CancelReason::PlayerLeft,
                        });
                        continue;
                    }
                },
                TaskTarget::Position(level, x, z) => match world.owner(level, x, z) {
                    Some(r) => (None, Some(CellKey::of_block(level, x, z)), Some((level, r))),
                    None => {
                        // Not loaded: it runs when a region owns the position.
                        waiting.push(task);
                        continue;
                    }
                },
            };
            self.shared.stats.tasks_run.fetch_add(1, Ordering::Relaxed);
            let region = region.filter(|_| def.region.is_some()).and_then(|r| self.regions.get_mut(&r));
            match region {
                Some(rp) => {
                    rp.run_in(task.plugin, player.as_ref(), cell, RegionCall::Task(task.handle, task.id));
                }
                None => {
                    let handle = task.handle;
                    let id = task.id;
                    let actor = player.as_ref().map(|p| Actor { uuid: p.uuid, name: &p.name, operator: p.operator, info: p.info });
                    self.global_call(task.plugin, actor.as_ref(), |store, g, player| {
                        g.call_on_task(store, wit::TaskEvent { handle, id, player, cell: None })
                    });
                }
            }
        }
        if !waiting.is_empty() {
            let mut t = self.shared.tasks.lock().unwrap();
            for mut task in waiting {
                task.due = tick + 1;
                let key = (task.due, tick, 0, 0, t.fresh.len() as u32);
                t.fresh.push((key, task));
            }
        }
        for (plugin, list) in notices {
            self.shared.stats.tasks_cancelled.fetch_add(list.len() as u64, Ordering::Relaxed);
            self.global_call(plugin, None, |store, g, _| g.call_on_cancelled(store, &list));
        }
    }

    /// One call in plugin `i`'s global instance; a failure replaces the instance.
    fn global_call(
        &mut self,
        i: usize,
        actor: Option<&Actor>,
        f: impl FnOnce(&mut Store<HostState>, &GlobalGuest, Option<wit::Player>) -> wasmtime::Result<()>,
    ) -> bool {
        let Some(outcome) = self.global_run(i, actor, f) else { return false };
        let ok = matches!(outcome, Outcome::Ok(()));
        self.global_failed(i, outcome);
        ok
    }

    /// One call in plugin `i`'s global instance (none if it is not running), with the
    /// instance's peers lent when the plugin may raise events.
    fn global_run<R>(
        &mut self,
        i: usize,
        actor: Option<&Actor>,
        f: impl FnOnce(&mut Store<HostState>, &GlobalGuest, Option<wit::Player>) -> wasmtime::Result<R>,
    ) -> Option<Outcome<R>> {
        self.globals[i].as_ref()?;
        self.shared.stats.calls.fetch_add(1, Ordering::Relaxed);
        let (shared, budget) = (&self.shared, self.set.serial);
        Some(with_peers(&self.set, &mut self.globals, i, false, |inst| {
            let generation = inst.generation();
            let frame = inst.frame();
            frame.reset(true, actor.map_or(0, |a| a.uuid));
            if let Some(a) = actor {
                frame.push_player(a.uuid, a.name, a.operator, Some(&a.info));
            }
            let p = actor.map(|a| wit_player(a, frame.player_handle(generation, 0)));
            inst.call(shared, budget, |store, g, _| f(store, g.expect("global guest"), p))
        }))
    }

    /// Plugin messages, in a deterministic order (the other effects stay queued).
    pub fn take_messages(&mut self) -> Vec<Outgoing> {
        let mut out = self.shared.outbox.lock().unwrap();
        let (messages, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *out).into_iter().partition(|(_, _, e)| matches!(e.kind, EffectKind::Message { .. }));
        *out = rest;
        drop(out);
        let mut messages = messages;
        messages.sort_by_key(|(tick, source, _)| (*tick, *source));
        messages
            .into_iter()
            .filter_map(|(_, _, e)| match e.kind {
                EffectKind::Message { to, text } => Some(Outgoing { to, text }),
                _ => None,
            })
            .collect()
    }

    /// Every effect plugins committed (messages included), in a deterministic order: by tick
    /// and acting player, then by commit order, which does not depend on thread count or
    /// region layout. The embedder applies them at a serial point and reports each outcome
    /// with [`effect_done`](Self::effect_done).
    pub fn take_effects(&mut self) -> Vec<Effect> {
        let mut out = std::mem::take(&mut *self.shared.outbox.lock().unwrap());
        out.sort_by_key(|(tick, source, _)| (*tick, *source));
        out.into_iter().map(|(_, _, e)| e).collect()
    }

    /// The outcome of an effect taken with [`take_effects`](Self::take_effects): `applied` is
    /// false when it did nothing (the player left, they lacked the items, the entity was not
    /// the plugin's). A plugin subscribed to `op-results` hears of it the next tick.
    pub fn effect_done(&mut self, effect: &Effect, applied: bool) {
        if effect.ticket == 0 || !self.shared.wants_results[effect.plugin].load(Ordering::Relaxed) {
            return;
        }
        let result = wit::OpResult { ticket: effect.ticket, applied, value: None };
        self.shared.deliveries.lock().unwrap().push(Delivery { plugin: effect.plugin, generation: effect.generation, source: effect.source, result });
    }

    /// The players online (`event.online`): set when the set of players or their levels
    /// changes; plugins see it from the next call on.
    pub fn set_online(&mut self, players: Vec<OnlinePlayer>) {
        *self.shared.online.lock().unwrap() = Arc::new(players);
    }

    /// Reloads finished since the last call (the embedder re-registers commands).
    pub fn take_reloaded(&mut self) -> Vec<Reloaded> {
        std::mem::take(&mut self.reloaded)
    }

    /// Calls the global instance of every plugin subscribed to `kind`.
    fn global_event(&mut self, kind: EventKind, actor: &Actor) {
        for i in 0..self.globals.len() {
            if self.set.plugins[i].manifest.subscription(kind).is_none() {
                continue;
            }
            let outcome = self.global_run(i, Some(actor), |store, g, p| {
                let p = p.expect("player");
                if kind == EventKind::Join { g.call_on_join(store, p) } else { g.call_on_leave(store, p) }
            });
            if let Some(outcome) = outcome {
                self.global_failed(i, outcome);
            }
        }
    }

    fn global_failed<R>(&mut self, i: usize, outcome: Outcome<R>) {
        let id = self.set.plugins[i].id.clone();
        match outcome {
            Outcome::Ok(_) => return,
            Outcome::Timeout => {
                warn!("plugin {id}: global call exceeded its budget");
                self.shared.strike(i, &id);
            }
            Outcome::Trap(e) => warn!("plugin {id} trapped: {e:#}"),
        }
        // A fresh global instance (its registrations stay as they were).
        let commands = self.commands.clone();
        self.globals[i] = self.start_global(i, None);
        self.commands = commands;
    }

    /// A player joined: its namespace is loaded, then `on-join` runs.
    pub fn player_joined(&mut self, actor: &Actor) {
        if let Some(p) = &self.shared.persist {
            let ns = p.load_player(actor.uuid);
            self.shared.players.lock().unwrap().entry(actor.uuid).or_insert(ns);
        }
        self.global_event(EventKind::Join, actor);
    }

    /// A player left: `on-leave` runs, then its namespace is saved and unloaded.
    pub fn player_left(&mut self, actor: &Actor) {
        self.global_event(EventKind::Leave, actor);
        self.shared.bucket_shard(actor.uuid).lock().unwrap().remove(&actor.uuid);
        if let Some(p) = &self.shared.persist {
            let ns = self.shared.players.lock().unwrap().remove(&actor.uuid);
            if let Some(ns) = ns {
                p.save_player(actor.uuid, &ns);
            }
        }
    }

    /// Runs a registered command in its plugin's global instance; returns the reply.
    pub fn run_command(&mut self, plugin: usize, actor: Option<&Actor>, name: &str, args: &str) -> Vec<Span> {
        if plugin >= self.globals.len() {
            return vec![Span::colored("This plugin is not running.", "red")];
        }
        let Some(outcome) = self.global_run(plugin, actor, |store, g, p| g.call_on_command(store, p, name, args)) else {
            return vec![Span::colored("This plugin is not running.", "red")];
        };
        match outcome {
            Outcome::Ok(reply) => spans(reply),
            failed => {
                self.global_failed(plugin, failed);
                vec![Span::colored("The command failed in its plugin.", "red")]
            }
        }
    }

    /// The plugin that registered command `name`, if any.
    pub fn command_plugin(&self, name: &str) -> Option<&CommandReg> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// Hot reload, step one: reads `plugin.toml` and `plugin.wasm` of plugin `id` from where
    /// it was loaded and compiles them on a background thread; the swap happens in the next
    /// B0 after the compilation finished ([`begin_tick_in`](Self::begin_tick_in)).
    /// `requester` hears the outcome.
    pub fn request_reload(&mut self, id: &str, requester: Option<u128>) -> Result<()> {
        let i = self.plugin_index(id).with_context(|| format!("no plugin `{id}`"))?;
        let dir = self.set.plugins[i].source.clone().with_context(|| format!("plugin `{id}` was not loaded from a directory"))?;
        let (mut manifest, wasm) = read_plugin(&dir)?;
        if manifest.id != id {
            bail!("{} now declares id `{}`", dir.display(), manifest.id);
        }
        let engine = self.set.engine.clone();
        let generation = self.set.plugins[i].generation + 1;
        let (cache, data) = (self.cache_dir.clone(), self.data_dir.clone());
        let (registries, spawn, staged) = (self.shared.registries.clone(), self.cfg.spawn, self.staged.clone());
        let shared = self.shared.clone();
        std::thread::Builder::new().name(format!("kiln-plugin-compile-{id}")).spawn(move || {
            let tasks_engine = shared.tasks_engine_for(&mut manifest);
            let def = prepare(&engine, manifest, &wasm, Some(dir), generation, cache.as_deref(), data.as_deref(), &registries, spawn, Some(&shared.stats), tasks_engine.as_ref());
            staged.lock().unwrap().push(Staged { plugin: i, def, requester });
        })?;
        Ok(())
    }

    /// Hot reload now (compiles on this thread, then swaps as B0 would): for tests and for
    /// embedders that call it at a serial point.
    pub fn reload(&mut self, id: &str, mut manifest: Manifest, wasm: &[u8]) -> Result<Reloaded> {
        let i = self.plugin_index(id).with_context(|| format!("no plugin `{id}`"))?;
        if manifest.id != id {
            bail!("the new manifest declares id `{}`", manifest.id);
        }
        let source = self.set.plugins[i].source.clone();
        let tasks_engine = self.shared.tasks_engine_for(&mut manifest);
        let def = prepare(
            &self.set.engine,
            manifest,
            wasm,
            source,
            self.set.plugins[i].generation + 1,
            self.cache_dir.as_deref(),
            self.data_dir.as_deref(),
            &self.shared.registries,
            self.cfg.spawn,
            Some(&self.shared.stats),
            tasks_engine.as_ref(),
        )?;
        self.swap(i, def)
    }

    /// Hot reload, the B0 part: the old global instance's `on-disable` blob, the new set of
    /// subscriptions and commands, new global and region instances, `on-enable(blob)`, and
    /// notices for the old generation's tasks.
    fn swap(&mut self, i: usize, def: PluginDef) -> Result<Reloaded> {
        let id = def.manifest.id.clone();
        let generation = def.generation;
        // The old generation's last word.
        let mut blob = None;
        if let Some(inst) = self.globals[i].as_mut() {
            inst.frame().reset(true, 0);
            match inst.call(&self.shared, self.set.serial, |store, g, _| g.expect("global guest").call_on_disable(store)) {
                Outcome::Ok(b) => blob = b,
                Outcome::Timeout => warn!("plugin {id}: on-disable ran out of budget; no state handed over"),
                Outcome::Trap(e) => warn!("plugin {id}: on-disable trapped ({e:#}); no state handed over"),
            }
        }
        // Tasks of the old generation: cancelled, with their remaining delay.
        let tick = self.shared.tick();
        let mut cancelled = Vec::new();
        {
            let mut t = self.shared.tasks.lock().unwrap();
            let t = &mut *t;
            for (key, task) in std::mem::take(&mut t.fresh) {
                t.by_handle.insert((task.plugin, task.handle), key);
                t.table.insert(key, task);
            }
            let cancels: Vec<(usize, u64)> = std::mem::take(&mut t.cancels);
            for c in cancels {
                if let Some(key) = t.by_handle.remove(&c) {
                    t.table.remove(&key);
                }
            }
            let keys: Vec<TaskKey> = t.table.iter().filter(|(_, x)| x.plugin == i).map(|(k, _)| *k).collect();
            for k in keys {
                let task = t.table.remove(&k).expect("task");
                t.by_handle.remove(&(i, task.handle));
                let target = match task.target {
                    TaskTarget::Global => wit::TaskTarget::Global,
                    TaskTarget::Player(u) => wit::TaskTarget::Player(host::wit_uuid(u)),
                    TaskTarget::Position(l, x, z) => wit::TaskTarget::Position((l, x, z)),
                };
                cancelled.push(wit::CancelledTask {
                    id: task.id,
                    target,
                    remaining_ticks: task.due.saturating_sub(tick).min(u32::MAX as u64) as u32,
                    reason: wit::CancelReason::Reload,
                });
            }
        }
        // Jobs the old tasks component was running die with it: reported like cancelled tasks.
        if let Some(w) = self.shared.started_worker() {
            w.unload(i);
        }
        {
            let mut inflight = self.shared.jobs_inflight.lock().unwrap();
            let mut mine: Vec<(u64, u64)> = inflight.iter().filter(|(_, j)| j.plugin == i).map(|(t, j)| (*t, j.id)).collect();
            mine.sort_unstable();
            for (ticket, id) in mine {
                inflight.remove(&ticket);
                cancelled.push(wit::CancelledTask { id, target: wit::TaskTarget::Global, remaining_ticks: 0, reason: wit::CancelReason::Reload });
            }
        }
        // The swap: one new set for every region.
        let mut plugins = self.set.plugins.clone();
        plugins[i] = Arc::new(def);
        self.set = Arc::new(PluginSet::build(self.set.engine.clone(), plugins, &self.cfg));
        self.shared.wants_results[i].store(self.set.plugins[i].manifest.subscription(EventKind::OpResults).is_some(), Ordering::Relaxed);
        {
            let h = &self.shared.health[i];
            h.strikes.lock().unwrap().clear();
            h.demoted.store(false, Ordering::Relaxed);
        }
        let before = self.commands.clone();
        if let (Some((c, g)), Some(w)) = (&self.set.plugins[i].tasks, self.shared.tasks_worker()) {
            w.load(i, generation, c.clone(), g.clone());
        }
        self.globals[i] = None;
        self.globals[i] = self.start_global(i, blob.clone());
        let mut keys: Vec<(u32, u64)> = self.regions.keys().copied().collect();
        keys.sort_unstable();
        for k in &keys {
            self.regions.get_mut(k).expect("region").swap(self.set.clone(), i);
        }
        let n = cancelled.len();
        if !cancelled.is_empty() {
            self.shared.stats.tasks_cancelled.fetch_add(n as u64, Ordering::Relaxed);
            self.global_call(i, None, |store, g, _| g.call_on_cancelled(store, &cancelled));
        }
        self.shared.stats.reloads.fetch_add(1, Ordering::Relaxed);
        let region_instances = if self.set.plugins[i].region.is_some() { keys.len() } else { 0 };
        Ok(Reloaded { id, generation, blob: blob.map(|b| b.len()), cancelled_tasks: n, region_instances, commands_changed: before != self.commands })
    }

    /// Saves every namespace (autosave and shutdown).
    pub fn save(&self) {
        let Some(p) = &self.shared.persist else { return };
        for (uuid, ns) in self.shared.players.lock().unwrap().iter() {
            p.save_player(*uuid, ns);
        }
        p.save_cells(&self.shared.cells.lock().unwrap());
        p.save_globals(&self.shared.globals.lock().unwrap());
    }

    /// Hashes all plugin state (namespaces and scheduled tasks are what the game sees).
    pub fn hash_state<H: std::hash::Hasher>(&self, h: &mut H) {
        use std::hash::Hash;
        ns::hash_sorted(&self.shared.players.lock().unwrap(), h);
        ns::hash_sorted(&self.shared.cells.lock().unwrap().cells, h);
        self.shared.globals.lock().unwrap().hash(h);
        let t = self.shared.tasks.lock().unwrap();
        for (k, task) in &t.table {
            k.hash(h);
            task.hash(h);
        }
        let mut fresh: Vec<_> = t.fresh.iter().collect();
        fresh.sort_by_key(|(k, _)| *k);
        for (k, task) in fresh {
            k.hash(h);
            task.hash(h);
        }
    }

    /// A player's value of a plugin's key (tests and tools).
    pub fn player_value(&self, uuid: u128, plugin: &str, key: &str) -> Option<Vec<u8>> {
        let i = self.plugin_index(plugin)?;
        self.shared.players.lock().unwrap().get(&uuid)?.get(i, key).cloned()
    }

    /// Whether a player's namespace is loaded.
    pub fn player_loaded(&self, uuid: u128) -> bool {
        self.shared.players.lock().unwrap().contains_key(&uuid)
    }

    /// A cell's value of a plugin's key (tests and tools).
    pub fn cell_value(&self, cell: CellKey, plugin: &str, key: &str) -> Option<Vec<u8>> {
        let i = self.plugin_index(plugin)?;
        self.shared.cells.lock().unwrap().cells.get(&cell)?.get(i, key).cloned()
    }

    /// The live global value of a plugin's key.
    pub fn global_value(&self, plugin: &str, key: &str) -> Option<GlobalValue> {
        let i = self.plugin_index(plugin)?;
        self.shared.globals.lock().unwrap().get(i, key).cloned()
    }

    /// Every live global key of a plugin.
    pub fn global_entries(&self, plugin: &str) -> Vec<(String, GlobalValue)> {
        let Some(i) = self.plugin_index(plugin) else { return Vec::new() };
        let g = self.shared.globals.lock().unwrap();
        g.by_plugin.get(i).map(|kv| kv.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
    }

    /// One line per plugin for operators (`/kiln plugins`).
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (i, d) in self.set.plugins.iter().enumerate() {
            let state = if self.globals[i].is_none() {
                "not running"
            } else if self.shared.demoted(i) {
                "demoted"
            } else {
                "running"
            };
            let events: Vec<String> = d.manifest.subscriptions.iter().map(|s| format!("{:?}", s.event)).collect();
            lines.push(format!("{} {} (generation {}, {state}): {}", d.id, d.manifest.version, d.generation, events.join(", ")));
        }
        lines
    }
}
