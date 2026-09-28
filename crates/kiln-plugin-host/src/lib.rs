//! The WASM plugin host (design §11): wasmtime components against the WIT package `kiln:api`
//! (`wit/kiln-api.wit`), guests built for `wasm32-wasip2`.
//!
//! # Contract
//!
//! - Each plugin has one **global** instance ([`PluginRuntime`]), called only from serial
//!   phases: `init` (command registration), join, leave, plugin commands.
//! - A plugin exporting `region-hooks` has one instance per region ([`RegionPlugins`]),
//!   bound to the region and never to a thread; the embedder keeps the set in step with the
//!   regionizer ([`PluginRuntime::sync_regions`], in B0). Instances are replaced at will
//!   (split, merge, a trap), so guest memory is only a cache.
//! - Persistent state lives in host-owned namespaces: player, cell and global. A handler
//!   reaches only the players and the cell its event names (handles are valid for that call
//!   only) plus a snapshot of its global namespace at most one tick old; it changes the
//!   global namespace only through typed atomic operations, applied in B0 in an order that
//!   does not depend on threads or the region layout ([`PluginRuntime::begin_tick`]).
//! - Writes, operations and messages of a call are buffered and committed only when the call
//!   returns normally: a trap or a timeout leaves nothing behind.
//! - Every cancellable call gets its own epoch deadline (default 500 µs). A timeout is a
//!   strike; three strikes in 60 s demote the plugin to observe-only. A failed or demoted
//!   fail-closed subscription denies; fail-open carries on.
//! - Only capabilities the manifest grants are linked; memory is capped at 64 MiB per
//!   instance; instances come from the pooling allocator.
//!
//! Not in this slice: hot reload, the WASI 0.3 `async-tasks` world, fuel-based strict mode,
//! entity-scoped state, host-side event filters, per-player rate limits, `.cwasm` caching.

pub mod examples;
mod host;
pub mod manifest;
mod ns;

pub use manifest::{Capability, EventKind, FailPolicy, Manifest};
pub use ns::{CellKey, CellSidecars, GlobalValue};

use anyhow::{Context, Result};
use host::{GlobalGuest, GlobalIndices, Pre, RegionGuest, RegionIndices, wit};
use ns::{CellTable, Globals, Ns, Persist};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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

/// The player an event is about.
#[derive(Clone, Copy, Debug)]
pub struct Actor<'a> {
    pub uuid: u128,
    pub name: &'a str,
    pub operator: bool,
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
#[derive(Clone, Debug)]
pub struct CommandReg {
    pub plugin: usize,
    pub name: String,
    pub permission: u8,
}

pub struct RuntimeConfig {
    /// Namespace files (`<world>/kiln/plugins`); nothing is saved when `None`.
    pub data_dir: Option<PathBuf>,
    /// Level keys by dimension index.
    pub levels: Vec<String>,
    pub spawn: [i32; 3],
    /// Fresh time budget of each cancellable call.
    pub call_budget: Duration,
    /// Epoch ticker period.
    pub epoch_tick: Duration,
    /// Pooling allocator slots (component instances); `0` uses on-demand allocation.
    pub pool_instances: u32,
    /// Where cell data goes instead of sidecar files under `data_dir` (native worlds).
    pub cell_sidecars: Option<Arc<dyn CellSidecars>>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            data_dir: None,
            levels: vec!["minecraft:overworld".into(), "minecraft:the_nether".into(), "minecraft:the_end".into()],
            spawn: [0, 64, 0],
            call_budget: Duration::from_micros(500),
            epoch_tick: Duration::from_micros(250),
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
}

struct Health {
    strikes: VecDeque<Instant>,
    demoted: bool,
}

const STRIKES: usize = 3;
const STRIKE_WINDOW: Duration = Duration::from_secs(60);

/// A queued atomic operation of a non-global context.
struct Pending {
    tick: u64,
    source: u128,
    plugin: usize,
    op: wit::AtomicOp,
}

/// State every instance of every plugin reaches (behind short locks; regions touch
/// disjoint players and cells).
pub(crate) struct Shared {
    tick: AtomicU64,
    pub(crate) players: Mutex<HashMap<u128, Ns>>,
    pub(crate) cells: Mutex<CellTable>,
    pub(crate) globals: Mutex<Globals>,
    globals_dirty: AtomicBool,
    snapshot: Mutex<Arc<Globals>>,
    pending: Mutex<Vec<Pending>>,
    outbox: Mutex<Vec<(u64, u128, Outgoing)>>,
    health: Vec<Mutex<Health>>,
    pub(crate) next_ticket: AtomicU64,
    serial: AtomicU64,
    persist: Option<Persist>,
    pub stats: Stats,
}

impl Shared {
    fn frame(&self, global_ctx: bool, source: u128, players: Vec<u128>, cells: Vec<CellKey>) -> host::Frame {
        if let Some(p) = &self.persist
            && !cells.is_empty()
        {
            let mut table = self.cells.lock().unwrap();
            for &c in &cells {
                p.ensure_cell(&mut table, c);
            }
        }
        // 47 bits of serial: handles are (serial << 16) | index.
        let serial = (self.serial.fetch_add(1, Ordering::Relaxed) + 1) & ((1 << 47) - 1);
        host::Frame {
            serial,
            global_ctx,
            source,
            players,
            cells,
            snapshot: self.snapshot.lock().unwrap().clone(),
            writes: Vec::new(),
            ops: Vec::new(),
            messages: Vec::new(),
        }
    }

    /// Makes a call's buffered effects real (the call returned normally).
    fn commit(&self, plugin: usize, f: host::Frame) {
        if !f.writes.is_empty() {
            let mut players = self.players.lock().unwrap();
            let mut cells = self.cells.lock().unwrap();
            for (target, key, val) in f.writes {
                match target {
                    host::Target::Player(u) => players.entry(u).or_default().put(plugin, key, val),
                    host::Target::Cell(c) => cells.cells.entry(c).or_default().put(plugin, key, val),
                }
            }
        }
        if !f.ops.is_empty() {
            if f.global_ctx {
                let mut g = self.globals.lock().unwrap();
                for op in f.ops {
                    apply_op(g.entry(plugin), op);
                }
                self.globals_dirty.store(true, Ordering::Relaxed);
            } else {
                let tick = self.tick.load(Ordering::Relaxed);
                let mut pending = self.pending.lock().unwrap();
                pending.extend(f.ops.into_iter().map(|op| Pending { tick, source: f.source, plugin, op }));
            }
        }
        if !f.messages.is_empty() {
            let tick = self.tick.load(Ordering::Relaxed);
            let mut out = self.outbox.lock().unwrap();
            out.extend(f.messages.into_iter().map(|(to, text)| (tick, f.source, Outgoing { to, text })));
        }
    }

    fn demoted(&self, plugin: usize) -> bool {
        self.health[plugin].lock().unwrap().demoted
    }

    fn strike(&self, plugin: usize, id: &str) {
        let now = Instant::now();
        let mut h = self.health[plugin].lock().unwrap();
        h.strikes.push_back(now);
        while h.strikes.front().is_some_and(|t| now.duration_since(*t) > STRIKE_WINDOW) {
            h.strikes.pop_front();
        }
        if h.strikes.len() >= STRIKES && !h.demoted {
            h.demoted = true;
            warn!("plugin {id}: {STRIKES} calls over budget within {}s, demoted to observe-only", STRIKE_WINDOW.as_secs());
        }
    }
}

/// Applies a typed atomic operation; a type mismatch or a failed comparison changes nothing.
fn apply_op(ns: &mut std::collections::BTreeMap<String, GlobalValue>, op: wit::AtomicOp) {
    match op {
        wit::AtomicOp::Add((key, delta)) => match ns.get_mut(&key) {
            Some(GlobalValue::Int(v)) => *v = v.wrapping_add(delta),
            Some(GlobalValue::Bytes(_)) => {}
            None => drop(ns.insert(key, GlobalValue::Int(delta))),
        },
        wit::AtomicOp::CompareAndSet(c) => {
            let expected = c.expected.map(host::from_wit_value);
            if ns.get(&c.key) == expected.as_ref() {
                ns.insert(c.key, host::from_wit_value(c.new));
            }
        }
        wit::AtomicOp::Append((key, bytes)) => match ns.get_mut(&key) {
            Some(GlobalValue::Bytes(v)) => v.extend_from_slice(&bytes),
            Some(GlobalValue::Int(_)) => {}
            None => drop(ns.insert(key, GlobalValue::Bytes(bytes))),
        },
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
    init: wit::InitInfo,
}

/// Everything fixed after loading.
struct PluginSet {
    engine: Engine,
    plugins: Vec<PluginDef>,
    /// Epoch ticks of a cancellable call's deadline.
    deadline: u64,
}

/// Epoch ticks for `init` calls (generous: first calls allocate and warm up).
const INIT_DEADLINE: u64 = 4000;

enum Outcome<R> {
    Ok(R),
    Timeout,
    Trap(wasmtime::Error),
}

/// One instance of one plugin.
struct Inst {
    store: Store<host::HostState>,
    global: Option<GlobalGuest>,
    region: Option<RegionGuest>,
}

impl Inst {
    /// Instantiates plugin `i` as a global or region instance and runs its `init`.
    fn new(set: &PluginSet, shared: &Arc<Shared>, i: usize, region: bool) -> Result<(Inst, Vec<wit::CommandSpec>)> {
        let def = &set.plugins[i];
        let mut store = host::new_store(&set.engine, i, def.id.clone(), shared.clone(), def.data_dir.as_deref());
        store.set_epoch_deadline(INIT_DEADLINE);
        let instance = def.pre.instantiate(&mut store)?;
        shared.stats.instantiations.fetch_add(1, Ordering::Relaxed);
        let mut inst = Inst { store, global: None, region: None };
        let mut commands = Vec::new();
        let frame = shared.frame(!region, 0, Vec::new(), Vec::new());
        inst.store.data_mut().frame = Some(frame);
        let r = if region {
            let guest = def.region.as_ref().context("no region-hooks export")?.load(&mut inst.store, &instance)?;
            let r = guest.call_init(&mut inst.store, &def.init);
            inst.region = Some(guest);
            r
        } else {
            let guest = def.global.load(&mut inst.store, &instance)?;
            let r = guest.call_init(&mut inst.store, &def.init).map(|c| commands = c);
            inst.global = Some(guest);
            r
        };
        let frame = inst.store.data_mut().frame.take().expect("frame");
        r.map_err(|e| anyhow::anyhow!("plugin {} init: {e:?}", def.id))?;
        shared.commit(i, frame);
        Ok((inst, commands))
    }

    /// Runs one call with a fresh deadline; commits its effects if it returned normally.
    fn call<R>(
        &mut self,
        shared: &Shared,
        plugin: usize,
        deadline: u64,
        frame: host::Frame,
        f: impl FnOnce(&mut Store<host::HostState>, Option<&GlobalGuest>, Option<&RegionGuest>) -> wasmtime::Result<R>,
    ) -> Outcome<R> {
        self.store.data_mut().frame = Some(frame);
        self.store.set_epoch_deadline(deadline);
        let r = f(&mut self.store, self.global.as_ref(), self.region.as_ref());
        let frame = self.store.data_mut().frame.take().expect("frame");
        shared.stats.calls.fetch_add(1, Ordering::Relaxed);
        match r {
            Ok(v) => {
                shared.commit(plugin, frame);
                Outcome::Ok(v)
            }
            Err(e) if e.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::Interrupt) => {
                shared.stats.timeouts.fetch_add(1, Ordering::Relaxed);
                Outcome::Timeout
            }
            Err(e) => {
                shared.stats.traps.fetch_add(1, Ordering::Relaxed);
                Outcome::Trap(e)
            }
        }
    }
}

fn wit_player(a: &Actor, handle: u64) -> wit::Player {
    wit::Player { handle, uuid: uuid::Uuid::from_u128(a.uuid).hyphenated().to_string(), name: a.name.to_owned(), operator: a.operator }
}

fn wit_pos(p: [i32; 3]) -> wit::BlockPos {
    wit::BlockPos { x: p[0], y: p[1], z: p[2] }
}

fn spans(v: Vec<wit::Span>) -> Vec<Span> {
    v.into_iter().map(host::from_wit_span).collect()
}

/// A block change a region saw, for the next observe batch.
struct Observation {
    broken: bool,
    uuid: u128,
    name: String,
    operator: bool,
    level: String,
    pos: [i32; 3],
    block: String,
}

/// A region's instances, one per plugin that exports `region-hooks`.
pub struct RegionPlugins {
    set: Arc<PluginSet>,
    shared: Arc<Shared>,
    dim: u32,
    insts: Vec<Option<Inst>>,
    observed: Vec<Observation>,
    observers: bool,
}

impl RegionPlugins {
    fn new(set: Arc<PluginSet>, shared: Arc<Shared>, dim: u32) -> RegionPlugins {
        let observers = set.plugins.iter().any(|p| p.region.is_some() && p.manifest.subscription(EventKind::Observe).is_some());
        let mut r = RegionPlugins { insts: (0..set.plugins.len()).map(|_| None).collect(), set, shared, dim, observed: Vec::new(), observers };
        for i in 0..r.insts.len() {
            r.ensure(i);
        }
        r
    }

    /// The region instance of plugin `i`, instantiated if missing (after a trap).
    fn ensure(&mut self, i: usize) -> bool {
        if self.insts[i].is_none() && self.set.plugins[i].region.is_some() {
            match Inst::new(&self.set, &self.shared, i, true) {
                Ok((inst, _)) => self.insts[i] = Some(inst),
                Err(e) => warn!("plugin {}: cannot instantiate for a region: {e:#}", self.set.plugins[i].id),
            }
        }
        self.insts[i].is_some()
    }

    /// Calls every subscriber of a cancellable event in load order until one denies.
    /// `f` gets the guest, the store, and the handles of the actor and the event's cell.
    fn cancellable<R>(
        &mut self,
        kind: EventKind,
        actor: &Actor,
        cell: Option<CellKey>,
        mut f: impl FnMut(&RegionGuest, &mut Store<host::HostState>, u64, u64) -> wasmtime::Result<R>,
        mut decide: impl FnMut(R) -> Option<Verdict>,
    ) -> Verdict {
        for i in 0..self.insts.len() {
            let def = &self.set.plugins[i];
            let Some(sub) = def.manifest.subscription(kind) else { continue };
            if def.region.is_none() {
                continue;
            }
            let policy = sub.policy;
            if self.shared.demoted(i) || !self.ensure(i) {
                if policy == FailPolicy::Closed {
                    return Verdict::Deny(None);
                }
                continue;
            }
            let frame = self.shared.frame(false, actor.uuid, vec![actor.uuid], cell.into_iter().collect());
            let (ph, ch) = (frame.player_handle(0), frame.cell_handle(0));
            let inst = self.insts[i].as_mut().expect("instance");
            let outcome = inst.call(&self.shared, i, self.set.deadline, frame, |store, _, region| f(region.expect("region guest"), store, ph, ch));
            match outcome {
                Outcome::Ok(r) => {
                    if let Some(v) = decide(r) {
                        return v;
                    }
                }
                failed => {
                    self.failed(i, failed);
                    if policy == FailPolicy::Closed {
                        return Verdict::Deny(None);
                    }
                }
            }
        }
        Verdict::Allow
    }

    /// A trap or timeout: the instance is replaced (its state may be inconsistent), and a
    /// timeout is a strike.
    fn failed<R>(&mut self, i: usize, outcome: Outcome<R>) {
        let id = &self.set.plugins[i].id;
        match outcome {
            Outcome::Timeout => {
                warn!("plugin {id}: call exceeded its budget");
                self.shared.strike(i, id);
            }
            Outcome::Trap(e) => warn!("plugin {id} trapped: {e:#}"),
            Outcome::Ok(_) => return,
        }
        self.insts[i] = None;
    }

    pub fn block_break(&mut self, actor: &Actor, level: &str, pos: [i32; 3], block: &str) -> Verdict {
        let cell = CellKey::of_block(self.dim, pos[0], pos[2]);
        self.cancellable(
            EventKind::BlockBreak,
            actor,
            Some(cell),
            |g, store, ph, ch| {
                let ev = wit::BlockEvent { player: wit_player(actor, ph), level: level.to_owned(), pos: wit_pos(pos), block: block.to_owned(), cell: ch };
                g.call_on_block_break(store, &ev)
            },
            verdict,
        )
    }

    /// `pos` is where the block would go, `against` the clicked block.
    pub fn block_place(&mut self, actor: &Actor, level: &str, pos: [i32; 3], against: [i32; 3], item: &str) -> Verdict {
        let cell = CellKey::of_block(self.dim, pos[0], pos[2]);
        self.cancellable(
            EventKind::BlockPlace,
            actor,
            Some(cell),
            |g, store, ph, ch| {
                let ev = wit::PlaceEvent {
                    player: wit_player(actor, ph),
                    level: level.to_owned(),
                    pos: wit_pos(pos),
                    against: wit_pos(against),
                    item: item.to_owned(),
                    cell: ch,
                };
                g.call_on_block_place(store, &ev)
            },
            verdict,
        )
    }

    /// A vanilla or plugin command a player is about to run (`command` without the slash).
    pub fn command(&mut self, actor: &Actor, command: &str) -> Verdict {
        self.cancellable(
            EventKind::Command,
            actor,
            None,
            |g, store, ph, _| g.call_on_command(store, &wit::CommandEvent { player: wit_player(actor, ph), command: command.to_owned() }),
            verdict,
        )
    }

    /// A chat message: cancelled, rewritten (the last rewrite wins) or passed.
    pub fn chat(&mut self, actor: &Actor, message: &str) -> ChatOutcome {
        let mut rewrite = None;
        let v = self.cancellable(
            EventKind::Chat,
            actor,
            None,
            |g, store, ph, _| g.call_on_chat(store, &wit::ChatEvent { player: wit_player(actor, ph), message: message.to_owned() }),
            |r| match r {
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
        self.observers
    }

    /// Notes a block broken (`broken`) or placed by `actor`, for the next observe batch.
    pub fn observe_block(&mut self, broken: bool, actor: &Actor, level: &str, pos: [i32; 3], block: &str) {
        if self.observers {
            self.observed.push(Observation {
                broken,
                uuid: actor.uuid,
                name: actor.name.to_owned(),
                operator: actor.operator,
                level: level.to_owned(),
                pos,
                block: block.to_owned(),
            });
        }
    }

    /// Sends the observations of this phase to observe subscribers, one batch per plugin.
    /// Demoted plugins still observe; failures only replace the instance (and strike).
    pub fn flush_observed(&mut self) {
        if self.observed.is_empty() {
            return;
        }
        let observed = std::mem::take(&mut self.observed);
        let mut players: Vec<u128> = Vec::new();
        for o in &observed {
            if !players.contains(&o.uuid) {
                players.push(o.uuid);
            }
        }
        for i in 0..self.insts.len() {
            if self.set.plugins[i].manifest.subscription(EventKind::Observe).is_none() || !self.ensure(i) {
                continue;
            }
            let frame = self.shared.frame(false, players[0], players.clone(), Vec::new());
            let batch: Vec<wit::Observed> = observed
                .iter()
                .map(|o| {
                    let h = frame.player_handle(players.iter().position(|p| *p == o.uuid).unwrap());
                    let actor = Actor { uuid: o.uuid, name: &o.name, operator: o.operator };
                    let b = wit::ObservedBlock { player: wit_player(&actor, h), level: o.level.clone(), pos: wit_pos(o.pos), block: o.block.clone() };
                    if o.broken { wit::Observed::BlockBroken(b) } else { wit::Observed::BlockPlaced(b) }
                })
                .collect();
            let inst = self.insts[i].as_mut().expect("instance");
            let outcome = inst.call(&self.shared, i, self.set.deadline, frame, |store, _, g| g.expect("region guest").call_on_observe(store, &batch));
            if !matches!(outcome, Outcome::Ok(())) {
                self.failed(i, outcome);
            }
        }
    }
}

fn verdict(v: wit::Verdict) -> Option<Verdict> {
    match v {
        wit::Verdict::Allow => None,
        wit::Verdict::Deny(m) => Some(Verdict::Deny(m.map(spans))),
    }
}

/// Increments the engine's epoch until dropped.
struct Ticker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Ticker {
    fn start(engine: Engine, period: Duration) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("kiln-plugin-epoch".into())
            .spawn(move || {
                // Sleeps overshoot (about 1 ms on Windows), so the epoch follows elapsed time:
                // a deadline of n ticks is n periods however coarse the wake-ups are.
                let start = Instant::now();
                let mut epoch = 0u128;
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(period);
                    let due = start.elapsed().as_nanos() / period.as_nanos().max(1);
                    while epoch < due {
                        engine.increment_epoch();
                        epoch += 1;
                    }
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

/// All plugins: their global instances, the region instance sets, and the namespaces.
pub struct PluginRuntime {
    set: Arc<PluginSet>,
    shared: Arc<Shared>,
    globals: Vec<Option<Inst>>,
    regions: HashMap<(u32, u64), RegionPlugins>,
    commands: Vec<CommandReg>,
    _ticker: Ticker,
}

fn engine(cfg: &RuntimeConfig) -> Result<Engine> {
    let mut c = wasmtime::Config::new();
    c.wasm_component_model(true);
    c.epoch_interruption(true);
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
            let r = (|| -> Result<(Manifest, Vec<u8>)> {
                let manifest = Manifest::parse(&std::fs::read_to_string(d.join("plugin.toml"))?)?;
                let wasm = std::fs::read(d.join("plugin.wasm")).context("plugin.wasm")?;
                Ok((manifest, wasm))
            })();
            match r {
                Ok(p) => found.push(p),
                Err(e) => warn!("skipping plugin {}: {e:#}", d.display()),
            }
        }
        Self::new(found, cfg)
    }

    /// Compiles, links and instantiates the given plugins (manifest and component bytes).
    pub fn new(plugins: Vec<(Manifest, Vec<u8>)>, cfg: RuntimeConfig) -> Result<PluginRuntime> {
        let engine = engine(&cfg)?;
        let mut defs: Vec<PluginDef> = Vec::new();
        let spawn = wit_pos(cfg.spawn);
        for (manifest, wasm) in plugins {
            if defs.iter().any(|d| d.manifest.id == manifest.id) {
                warn!("skipping plugin {}: duplicate id", manifest.id);
                continue;
            }
            let id: Arc<str> = manifest.id.as_str().into();
            let r = (|| -> Result<PluginDef> {
                let component = wasmtime::component::Component::new(&engine, &wasm)?;
                let linker = host::linker(&engine, manifest.has(Capability::PlayerMessage))?;
                let pre = linker.instantiate_pre(&component).map_err(|e| {
                    e.context("an import is not linked: is a capability missing from the manifest?")
                })?;
                let global = GlobalIndices::new(&pre)?;
                let region = RegionIndices::new(&pre).ok();
                let data_dir = match (&cfg.data_dir, manifest.has(Capability::FsData)) {
                    (Some(root), true) => Some(root.join("data").join(&manifest.id)),
                    _ => None,
                };
                let config = manifest.config.iter().map(|(k, v)| wit::ConfigEntry { key: k.clone(), value: v.clone() }).collect();
                let init = wit::InitInfo { id: manifest.id.clone(), config, spawn };
                Ok(PluginDef { manifest, id: id.clone(), pre, global, region, data_dir, init })
            })();
            match r {
                Ok(d) => defs.push(d),
                Err(e) => warn!("skipping plugin {id}: {e:#}"),
            }
        }
        let deadline = (cfg.call_budget.as_nanos() / cfg.epoch_tick.as_nanos().max(1)).max(1) as u64 + 1;
        let persist = cfg.data_dir.as_ref().map(|root| Persist {
            root: root.clone(),
            levels: cfg.levels.clone(),
            ids: defs.iter().map(|d| d.manifest.id.clone()).collect(),
            sidecars: cfg.cell_sidecars.clone(),
        });
        let globals = persist.as_ref().map(Persist::load_globals).unwrap_or_default();
        let shared = Arc::new(Shared {
            tick: AtomicU64::new(0),
            players: Mutex::new(HashMap::new()),
            cells: Mutex::new(CellTable::default()),
            snapshot: Mutex::new(Arc::new(globals.clone())),
            globals: Mutex::new(globals),
            globals_dirty: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
            outbox: Mutex::new(Vec::new()),
            health: defs.iter().map(|_| Mutex::new(Health { strikes: VecDeque::new(), demoted: false })).collect(),
            next_ticket: AtomicU64::new(1),
            serial: AtomicU64::new(0),
            persist,
            stats: Stats::default(),
        });
        let set = Arc::new(PluginSet { engine: engine.clone(), plugins: defs, deadline });
        let ticker = Ticker::start(engine, cfg.epoch_tick);
        let mut globals = Vec::new();
        let mut commands = Vec::new();
        for (i, def) in set.plugins.iter().enumerate() {
            match Inst::new(&set, &shared, i, false) {
                Ok((inst, specs)) => {
                    globals.push(Some(inst));
                    if def.manifest.has(Capability::CommandRegister) {
                        commands.extend(specs.into_iter().map(|s| CommandReg { plugin: i, name: s.name, permission: s.permission.min(4) }));
                    } else if !specs.is_empty() {
                        warn!("plugin {}: commands ignored without the command.register capability", def.id);
                    }
                }
                Err(e) => {
                    warn!("plugin {}: global instance failed: {e:#}", def.id);
                    globals.push(None);
                }
            }
            info!(
                "plugin {} {} loaded ({})",
                def.id,
                def.manifest.version,
                if def.region.is_some() { "global and region worlds" } else { "global world" }
            );
        }
        *shared.snapshot.lock().unwrap() = Arc::new(shared.globals.lock().unwrap().clone());
        Ok(PluginRuntime { set, shared, globals, regions: HashMap::new(), commands, _ticker: ticker })
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

    pub fn commands(&self) -> &[CommandReg] {
        &self.commands
    }

    pub fn stats(&self) -> &Stats {
        &self.shared.stats
    }

    pub fn is_demoted(&self, plugin: usize) -> bool {
        self.shared.demoted(plugin)
    }

    /// B0: brings the region instance sets of level `dim` in line with its regions: new
    /// regions (splits, new areas) get fresh instances, regions that merged away or died
    /// lose theirs. Their queued operations and messages live in the host, so nothing is
    /// lost; guest memory is not carried over.
    pub fn sync_regions(&mut self, dim: u32, ids: impl IntoIterator<Item = u64>) {
        let ids: std::collections::HashSet<u64> = ids.into_iter().collect();
        self.regions.retain(|&(d, r), rp| {
            let keep = d != dim || ids.contains(&r);
            if !keep {
                rp.flush_observed();
            }
            keep
        });
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

    /// B0: applies the atomic operations queued since the last tick in (tick, source, arrival)
    /// order, which does not depend on threads or regions, then refreshes the snapshot the
    /// region contexts read.
    pub fn begin_tick(&mut self) {
        let mut pending = std::mem::take(&mut *self.shared.pending.lock().unwrap());
        let changed = !pending.is_empty() || self.shared.globals_dirty.swap(false, Ordering::Relaxed);
        if !pending.is_empty() {
            pending.sort_by_key(|p| (p.tick, p.source));
            let mut g = self.shared.globals.lock().unwrap();
            for p in pending {
                apply_op(g.entry(p.plugin), p.op);
            }
        }
        if changed {
            *self.shared.snapshot.lock().unwrap() = Arc::new(self.shared.globals.lock().unwrap().clone());
        }
        self.shared.tick.fetch_add(1, Ordering::Relaxed);
    }

    /// Messages plugins sent, in a deterministic order.
    pub fn take_messages(&mut self) -> Vec<Outgoing> {
        let mut out = std::mem::take(&mut *self.shared.outbox.lock().unwrap());
        out.sort_by_key(|(tick, source, _)| (*tick, *source));
        out.into_iter().map(|(_, _, m)| m).collect()
    }

    /// Calls the global instance of every plugin subscribed to `kind`.
    fn global_event(&mut self, kind: EventKind, actor: &Actor) {
        for i in 0..self.globals.len() {
            if self.set.plugins[i].manifest.subscription(kind).is_none() {
                continue;
            }
            let Some(inst) = self.globals[i].as_mut() else { continue };
            let frame = self.shared.frame(true, actor.uuid, vec![actor.uuid], Vec::new());
            let ph = frame.player_handle(0);
            let p = wit_player(actor, ph);
            let outcome = inst.call(&self.shared, i, self.set.deadline, frame, |store, g, _| {
                let g = g.expect("global guest");
                if kind == EventKind::Join { g.call_on_join(store, &p) } else { g.call_on_leave(store, &p) }
            });
            self.global_failed(i, outcome);
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
        // A fresh global instance (its `init` registrations were taken at load).
        self.globals[i] = Inst::new(&self.set, &self.shared, i, false).map(|(inst, _)| inst).map_err(|e| warn!("plugin {id}: {e:#}")).ok();
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
        if let Some(p) = &self.shared.persist {
            let ns = self.shared.players.lock().unwrap().remove(&actor.uuid);
            if let Some(ns) = ns {
                p.save_player(actor.uuid, &ns);
            }
        }
    }

    /// Runs a registered command in its plugin's global instance; returns the reply.
    pub fn run_command(&mut self, plugin: usize, actor: Option<&Actor>, name: &str, args: &str) -> Vec<Span> {
        let Some(inst) = self.globals.get_mut(plugin).and_then(Option::as_mut) else {
            return vec![Span { text: "This plugin is not running.".into(), color: Some("red".into()), bold: false, italic: false }];
        };
        let players: Vec<u128> = actor.map(|a| a.uuid).into_iter().collect();
        let frame = self.shared.frame(true, actor.map_or(0, |a| a.uuid), players, Vec::new());
        let p = actor.map(|a| wit_player(a, frame.player_handle(0)));
        let outcome = inst.call(&self.shared, plugin, self.set.deadline, frame, |store, g, _| {
            g.expect("global guest").call_on_command(store, p.as_ref(), name, args)
        });
        match outcome {
            Outcome::Ok(reply) => spans(reply),
            failed => {
                self.global_failed(plugin, failed);
                vec![Span { text: "The command failed in its plugin.".into(), color: Some("red".into()), bold: false, italic: false }]
            }
        }
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

    /// Hashes all plugin state (namespaces and queued operations are what the game sees).
    pub fn hash_state<H: std::hash::Hasher>(&self, h: &mut H) {
        use std::hash::Hash;
        ns::hash_sorted(&self.shared.players.lock().unwrap(), h);
        ns::hash_sorted(&self.shared.cells.lock().unwrap().cells, h);
        self.shared.globals.lock().unwrap().hash(h);
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
}
