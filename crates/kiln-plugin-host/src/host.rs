//! The store side of a call: `HostState`, the per-call frame (handles, buffered effects), the
//! host implementations of the imported interfaces, and stores.
//!
//! The frame lives in the store and is reused call after call (its vectors keep their
//! capacity): starting a call is a few stores, not allocations.

use crate::ns::{CellKey, EntityData, GlobalValue, Globals};
use crate::{Shared, Span, TaskTarget};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use wasmtime::component::{HasSelf, InstancePre, Linker, ResourceTable};
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    world: "plugin",
    path: "../../wit",
    imports: { default: trappable },
});

pub(crate) use exports::kiln::api::global_hooks::{Guest as GlobalGuest, GuestIndices as GlobalIndices};
pub(crate) use exports::kiln::api::region_hooks::{Guest as RegionGuest, GuestIndices as RegionIndices};
pub(crate) use kiln::api::types as wit;

/// Guest linear memory limit per instance (design §11.5).
pub(crate) const MEMORY_LIMIT: usize = 64 << 20;

/// Where a buffered write goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Player(u128),
    Cell(CellKey),
    /// Index into the frame's entities.
    Entity(usize),
}

/// A task a call scheduled (committed with the call).
pub(crate) struct NewTask {
    pub handle: u64,
    pub id: u64,
    pub target: TaskTarget,
    pub delay: u32,
    /// (seq, n) of the scheduling call: tasks due the same tick run in scheduling order.
    pub order: (u32, u32),
}

/// Handle layout: generation (8 bits), call serial (40 bits), kind (2 bits), index (14 bits).
const KIND_PLAYER: u64 = 0;
const KIND_CELL: u64 = 1;
const KIND_ENTITY: u64 = 2;
const SERIAL_MASK: u64 = (1 << 40) - 1;

/// One call's context: the handles it may use and what it did, committed only if the call
/// returns normally.
#[derive(Default)]
pub(crate) struct Frame {
    pub serial: u64,
    /// The global instance: reads the live global namespace, its operations apply at commit.
    pub global_ctx: bool,
    /// Ordering key of the call's operations, tasks and messages (the acting player).
    pub source: u128,
    pub players: Vec<u128>,
    /// Names of `players` (by index; the strings keep their capacity across calls).
    names: Vec<String>,
    pub cells: Vec<CellKey>,
    /// Entities of the event with their plugin data (moved in for the call, moved back out).
    pub entities: Vec<(u128, EntityData)>,
    pub writes: Vec<(Target, String, Option<Vec<u8>>)>,
    pub ops: Vec<(u64, wit::AtomicOp)>,
    pub messages: Vec<(Option<u128>, Vec<Span>)>,
    pub tasks: Vec<NewTask>,
    pub cancels: Vec<u64>,
    /// The message for the acting player if the handler denies (`event.deny-message`).
    pub deny_msg: Option<Vec<Span>>,
    /// Deterministic id block of the call (reserved on first use): tickets, task handles and
    /// the random stream derive from (tick, source, seq, n).
    seq: Option<u32>,
    n: u32,
    rng: Option<u64>,
    snapshot: Option<Arc<Globals>>,
}

impl Frame {
    fn handle(&self, generation: u32, kind: u64, i: usize) -> u64 {
        ((generation as u64 & 0xff) << 56) | ((self.serial & SERIAL_MASK) << 16) | (kind << 14) | i as u64
    }
    pub fn player_handle(&self, generation: u32, i: usize) -> u64 {
        self.handle(generation, KIND_PLAYER, i)
    }
    pub fn cell_handle(&self, generation: u32, i: usize) -> u64 {
        self.handle(generation, KIND_CELL, i)
    }
    pub fn entity_handle(&self, generation: u32, i: usize) -> u64 {
        self.handle(generation, KIND_ENTITY, i)
    }
    fn resolve(&self, generation: u32, kind: u64, h: u64) -> wasmtime::Result<usize> {
        let expect = self.handle(generation, kind, 0);
        if h & !0x3fff != expect {
            wasmtime::bail!("stale or foreign handle");
        }
        Ok((h & 0x3fff) as usize)
    }
    fn resolve_player(&self, generation: u32, h: u64) -> wasmtime::Result<u128> {
        let i = self.resolve(generation, KIND_PLAYER, h)?;
        self.players.get(i).copied().ok_or_else(|| wasmtime::format_err!("bad player handle"))
    }
    fn resolve_cell(&self, generation: u32, h: u64) -> wasmtime::Result<CellKey> {
        let i = self.resolve(generation, KIND_CELL, h)?;
        self.cells.get(i).copied().ok_or_else(|| wasmtime::format_err!("bad cell handle"))
    }
    fn resolve_entity(&self, generation: u32, h: u64) -> wasmtime::Result<usize> {
        let i = self.resolve(generation, KIND_ENTITY, h)?;
        if i >= self.entities.len() {
            wasmtime::bail!("bad entity handle");
        }
        Ok(i)
    }

    /// A player of the call (handle index = position), with their name.
    pub fn push_player(&mut self, uuid: u128, name: &str) {
        let i = self.players.len();
        self.players.push(uuid);
        if self.names.len() <= i {
            self.names.push(String::new());
        }
        let n = &mut self.names[i];
        n.clear();
        n.push_str(name);
    }

    pub fn is_clean(&self) -> bool {
        self.writes.is_empty() && self.ops.is_empty() && self.messages.is_empty() && self.tasks.is_empty() && self.cancels.is_empty()
    }

    /// Forgets the last call (keeping the capacity) and starts the next one.
    pub fn reset(&mut self, global_ctx: bool, source: u128) {
        self.serial = self.serial.wrapping_add(1);
        self.global_ctx = global_ctx;
        self.source = source;
        self.players.clear();
        self.cells.clear();
        self.entities.clear();
        self.writes.clear();
        self.ops.clear();
        self.messages.clear();
        self.tasks.clear();
        self.cancels.clear();
        self.deny_msg = None;
        self.seq = None;
        self.n = 0;
        self.rng = None;
        self.snapshot = None;
    }
}

/// splitmix64: the mixing function of every derived id and random stream.
pub(crate) fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub(crate) struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    pub limits: StoreLimits,
    pub plugin: usize,
    pub id: Arc<str>,
    /// The plugin's generation when this store was made.
    pub generation: u32,
    pub shared: Arc<Shared>,
    pub frame: Frame,
    /// A call is running (host calls outside one trap).
    pub active: bool,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl HostState {
    fn frame(&mut self) -> wasmtime::Result<&mut Frame> {
        if !self.active {
            wasmtime::bail!("host call outside an event");
        }
        Ok(&mut self.frame)
    }

    fn target(&self, s: &kiln::api::state::Scope) -> wasmtime::Result<Target> {
        let f = &self.frame;
        Ok(match *s {
            kiln::api::state::Scope::Player(h) => Target::Player(f.resolve_player(self.generation, h)?),
            kiln::api::state::Scope::Cell(h) => Target::Cell(f.resolve_cell(self.generation, h)?),
            kiln::api::state::Scope::Entity(h) => Target::Entity(f.resolve_entity(self.generation, h)?),
        })
    }

    /// The next derived value of the call, `mix(seed, plugin, tick, source, seq, n)`, with its
    /// (seq, n) position (the call's order among the source's calls this tick, and within it).
    fn derive(&mut self) -> (u64, (u32, u32)) {
        let shared = &self.shared;
        let f = &mut self.frame;
        let seq = *f.seq.get_or_insert_with(|| shared.next_seq(f.source));
        f.n += 1;
        let tick = shared.tick();
        let s = shared.seed ^ mix(self.plugin as u64 + 1);
        let src = (f.source as u64) ^ ((f.source >> 64) as u64).rotate_left(17);
        (mix(s ^ mix(tick ^ mix(src ^ mix(((seq as u64) << 32) | f.n as u64)))), (seq, f.n))
    }

    /// A task handle or ticket: the generation in the top byte, a derived value below.
    fn new_id(&mut self) -> (u64, (u32, u32)) {
        let (v, order) = self.derive();
        (((self.generation as u64 & 0xff) << 56) | (v & ((1 << 56) - 1)), order)
    }
}

impl wit::Host for HostState {}

impl kiln::api::state::Host for HostState {
    fn get(&mut self, s: kiln::api::state::Scope, key: String) -> wasmtime::Result<Option<Vec<u8>>> {
        self.frame()?;
        let target = self.target(&s)?;
        let f = &self.frame;
        if let Some((_, _, v)) = f.writes.iter().rev().find(|(t, k, _)| *t == target && *k == key) {
            return Ok(v.clone());
        }
        let plugin = self.plugin;
        Ok(match target {
            Target::Player(u) => self.shared.players.lock().unwrap().get(&u).and_then(|ns| ns.get(plugin, &key).cloned()),
            Target::Cell(c) => {
                let mut table = self.shared.cells.lock().unwrap();
                self.shared.ensure_cell(&mut table, c);
                table.cells.get(&c).and_then(|ns| ns.get(plugin, &key).cloned())
            }
            Target::Entity(i) => f.entities[i].1.get(&*self.id).and_then(|kv| kv.get(&key)).cloned(),
        })
    }

    fn get_int(&mut self, s: kiln::api::state::Scope, key: String) -> wasmtime::Result<Option<i64>> {
        self.frame()?;
        let target = self.target(&s)?;
        let int = |v: &[u8]| <[u8; 8]>::try_from(v).ok().map(i64::from_le_bytes);
        let f = &self.frame;
        if let Some((_, _, v)) = f.writes.iter().rev().find(|(t, k, _)| *t == target && *k == key) {
            return Ok(v.as_deref().and_then(int));
        }
        let plugin = self.plugin;
        Ok(match target {
            Target::Player(u) => self.shared.players.lock().unwrap().get(&u).and_then(|ns| ns.get(plugin, &key).and_then(|v| int(v))),
            Target::Cell(c) => {
                let mut table = self.shared.cells.lock().unwrap();
                self.shared.ensure_cell(&mut table, c);
                table.cells.get(&c).and_then(|ns| ns.get(plugin, &key).and_then(|v| int(v)))
            }
            Target::Entity(i) => f.entities[i].1.get(&*self.id).and_then(|kv| kv.get(&key)).and_then(|v| int(v)),
        })
    }

    fn put_int(&mut self, s: kiln::api::state::Scope, key: String, val: i64) -> wasmtime::Result<()> {
        self.put(s, key, Some(val.to_le_bytes().to_vec()))
    }

    fn put(&mut self, s: kiln::api::state::Scope, key: String, val: Option<Vec<u8>>) -> wasmtime::Result<()> {
        self.frame()?;
        let target = self.target(&s)?;
        if key.len() > 256 || val.as_ref().is_some_and(|v| v.len() > 1 << 20) {
            wasmtime::bail!("state key or value too large");
        }
        self.frame.writes.push((target, key, val));
        Ok(())
    }

    fn global_get(&mut self, key: String) -> wasmtime::Result<Option<wit::GlobalValue>> {
        let plugin = self.plugin;
        let shared = self.shared.clone();
        let f = self.frame()?;
        let v = if f.global_ctx {
            shared.globals.lock().unwrap().get(plugin, &key).cloned()
        } else {
            let snap = f.snapshot.get_or_insert_with(|| shared.snapshot.lock().unwrap().clone());
            snap.get(plugin, &key).cloned()
        };
        Ok(v.map(to_wit_value))
    }

    fn submit(&mut self, op: wit::AtomicOp) -> wasmtime::Result<u64> {
        if self.frame()?.ops.len() >= 1024 {
            wasmtime::bail!("too many atomic operations in one call");
        }
        let (ticket, _) = self.new_id();
        self.frame.ops.push((ticket, op));
        Ok(ticket)
    }
}

impl kiln::api::event::Host for HostState {
    fn player_name(&mut self, p: u64) -> wasmtime::Result<String> {
        let generation = self.generation;
        let f = self.frame()?;
        let i = f.resolve(generation, KIND_PLAYER, p)?;
        f.names.get(i).filter(|_| i < f.players.len()).cloned().ok_or_else(|| wasmtime::format_err!("bad player handle"))
    }

    fn deny_message(&mut self, text: Vec<wit::Span>) -> wasmtime::Result<()> {
        self.frame()?.deny_msg = Some(text.into_iter().map(from_wit_span).collect());
        Ok(())
    }
}

impl kiln::api::env::Host for HostState {
    fn tick(&mut self) -> wasmtime::Result<u64> {
        Ok(self.shared.tick())
    }

    fn now_millis(&mut self) -> wasmtime::Result<u64> {
        Ok(self.shared.now_millis())
    }

    fn random(&mut self) -> wasmtime::Result<u64> {
        self.frame()?;
        let state = match self.frame.rng {
            Some(s) => s,
            None => self.derive().0,
        };
        let next = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        self.frame.rng = Some(next);
        Ok(mix(next))
    }
}

impl kiln::api::registry::Host for HostState {
    fn key(&mut self, k: kiln::api::registry::Kind, id: u32) -> wasmtime::Result<Option<String>> {
        Ok(self.shared.registries.list(k.into()).get(id as usize).cloned())
    }

    fn id(&mut self, k: kiln::api::registry::Kind, key: String) -> wasmtime::Result<Option<u32>> {
        Ok(self.shared.registries.id(k.into(), &key))
    }
}

impl From<kiln::api::registry::Kind> for crate::RegistryKind {
    fn from(k: kiln::api::registry::Kind) -> Self {
        use kiln::api::registry::Kind;
        match k {
            Kind::Level => crate::RegistryKind::Level,
            Kind::Block => crate::RegistryKind::Block,
            Kind::Item => crate::RegistryKind::Item,
            Kind::EntityType => crate::RegistryKind::EntityType,
        }
    }
}

/// Tasks per call, and the longest delay (a day).
const MAX_TASKS: usize = 256;
const MAX_DELAY: u32 = 24_000;

impl HostState {
    fn schedule(&mut self, target: TaskTarget, delay: u32, id: u64) -> wasmtime::Result<u64> {
        if self.frame()?.tasks.len() >= MAX_TASKS {
            wasmtime::bail!("too many tasks scheduled in one call");
        }
        let (handle, order) = self.new_id();
        self.frame.tasks.push(NewTask { handle, id, target, delay: delay.clamp(1, MAX_DELAY), order });
        Ok(handle)
    }
}

impl kiln::api::scheduler::Host for HostState {
    fn global(&mut self, delay: u32, id: u64) -> wasmtime::Result<u64> {
        self.schedule(TaskTarget::Global, delay, id)
    }

    fn for_player(&mut self, p: wit::Uuid, delay: u32, id: u64) -> wasmtime::Result<u64> {
        self.schedule(TaskTarget::Player(from_wit_uuid(p)), delay, id)
    }

    fn at_position(&mut self, level: u32, x: i32, z: i32, delay: u32, id: u64) -> wasmtime::Result<u64> {
        if level as usize >= self.shared.registries.levels.len() {
            wasmtime::bail!("unknown level {level}");
        }
        self.schedule(TaskTarget::Position(level, x, z), delay, id)
    }

    fn cancel(&mut self, t: u64) -> wasmtime::Result<bool> {
        self.frame()?;
        if (t >> 56) != (self.generation as u64 & 0xff) {
            wasmtime::bail!("task handle of another generation");
        }
        let f = &mut self.frame;
        if let Some(i) = f.tasks.iter().position(|n| n.handle == t) {
            f.tasks.remove(i);
            return Ok(true);
        }
        if f.cancels.contains(&t) || !self.shared.task_pending(self.plugin, t) {
            return Ok(false);
        }
        f.cancels.push(t);
        Ok(true)
    }
}

impl kiln::api::chat::Host for HostState {
    fn send(&mut self, to: u64, text: Vec<wit::Span>) -> wasmtime::Result<()> {
        let generation = self.generation;
        let uuid = self.frame()?.resolve_player(generation, to)?;
        self.frame.messages.push((Some(uuid), text.into_iter().map(from_wit_span).collect()));
        Ok(())
    }

    fn broadcast(&mut self, text: Vec<wit::Span>) -> wasmtime::Result<()> {
        let f = self.frame()?;
        f.messages.push((None, text.into_iter().map(from_wit_span).collect()));
        Ok(())
    }
}

impl kiln::api::log::Host for HostState {
    fn info(&mut self, msg: String) -> wasmtime::Result<()> {
        tracing::info!("[{}] {msg}", self.id);
        Ok(())
    }
    fn warn(&mut self, msg: String) -> wasmtime::Result<()> {
        tracing::warn!("[{}] {msg}", self.id);
        Ok(())
    }
}

pub(crate) fn to_wit_value(v: GlobalValue) -> wit::GlobalValue {
    match v {
        GlobalValue::Int(i) => wit::GlobalValue::Int(i),
        GlobalValue::Bytes(b) => wit::GlobalValue::Bytes(b),
    }
}

pub(crate) fn from_wit_value(v: wit::GlobalValue) -> GlobalValue {
    match v {
        wit::GlobalValue::Int(i) => GlobalValue::Int(i),
        wit::GlobalValue::Bytes(b) => GlobalValue::Bytes(b),
    }
}

pub(crate) fn from_wit_span(s: wit::Span) -> Span {
    Span { text: s.text, color: s.color, bold: s.bold, italic: s.italic }
}

pub(crate) fn wit_uuid(u: u128) -> wit::Uuid {
    wit::Uuid { hi: (u >> 64) as u64, lo: u as u64 }
}

pub(crate) fn from_wit_uuid(u: wit::Uuid) -> u128 {
    ((u.hi as u128) << 64) | u.lo as u128
}

/// Links what the manifest grants: `state`, `env`, `registry` and `log` always, `chat` with
/// `player.message`, `scheduler` with `scheduler`, and WASI (no environment, no preopens
/// unless `fs.data`, no sockets).
pub(crate) fn linker(engine: &wasmtime::Engine, chat: bool, scheduler: bool) -> anyhow::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    kiln::api::state::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::event::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::env::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::registry::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::log::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    if chat {
        kiln::api::chat::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if scheduler {
        kiln::api::scheduler::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    Ok(linker)
}

/// WASI clocks of strict mode: they follow the server tick.
struct TickClock(Arc<Shared>);

impl wasmtime_wasi::HostWallClock for TickClock {
    fn resolution(&self) -> std::time::Duration {
        std::time::Duration::from_millis(50)
    }
    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.0.now_millis())
    }
}

impl wasmtime_wasi::HostMonotonicClock for TickClock {
    fn resolution(&self) -> u64 {
        50_000_000
    }
    fn now(&self) -> u64 {
        self.0.tick() * 50_000_000
    }
}

/// A fresh store for one instance of plugin `plugin` (generation `generation`).
pub(crate) fn new_store(
    engine: &wasmtime::Engine,
    plugin: usize,
    generation: u32,
    id: Arc<str>,
    shared: Arc<Shared>,
    data_dir: Option<&std::path::Path>,
) -> Store<HostState> {
    let mut wasi = wasmtime_wasi::WasiCtxBuilder::new();
    // No stdio: a guest panic shows up as a trap in the host log.
    if let Some(dir) = data_dir {
        let _ = std::fs::create_dir_all(dir);
        if let Err(e) = wasi.preopened_dir(dir, "/data", wasmtime_wasi::filesystem::FsPerms::ReadWrite) {
            tracing::warn!("[{id}] cannot open its data directory: {e}");
        }
    }
    if shared.strict {
        // Replays exactly: WASI's clocks follow the tick, its random streams are fixed.
        let seed = mix(shared.seed ^ mix(plugin as u64 + 1));
        let bytes: Vec<u8> = (0..32u64).flat_map(|i| mix(seed ^ i).to_le_bytes()).collect();
        wasi.secure_random(wasmtime_wasi::random::Deterministic::new(bytes.clone()));
        wasi.insecure_random(wasmtime_wasi::random::Deterministic::new(bytes));
        wasi.insecure_random_seed(((seed as u128) << 64) | mix(seed) as u128);
        wasi.wall_clock(TickClock(shared.clone()));
        wasi.monotonic_clock(TickClock(shared.clone()));
    }
    let limits = StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(32).tables(32).memories(4).table_elements(1 << 16).build();
    let state = HostState {
        wasi: wasi.build(),
        table: ResourceTable::new(),
        limits,
        plugin,
        id,
        generation,
        shared,
        frame: Frame::default(),
        active: false,
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limits);
    store
}

pub(crate) type Pre = InstancePre<HostState>;

impl Shared {
    pub(crate) fn tick(&self) -> u64 {
        self.tick.load(Ordering::Relaxed)
    }
}
