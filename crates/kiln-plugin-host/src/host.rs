//! The store side of a call: `HostState`, the per-call frame (handles, buffered writes), the
//! host implementations of the imported interfaces, and instantiation.

use crate::ns::{CellKey, GlobalValue, Globals};
use crate::{Shared, Span};
use std::sync::Arc;
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
}

/// One call's context: the handles it may use and what it did, committed only if the call
/// returns normally.
pub(crate) struct Frame {
    pub serial: u64,
    /// The global instance: reads the live global namespace, its operations apply at commit.
    pub global_ctx: bool,
    /// Ordering key of the call's operations and messages (the acting player).
    pub source: u128,
    pub players: Vec<u128>,
    pub cells: Vec<CellKey>,
    pub snapshot: Arc<Globals>,
    pub writes: Vec<(Target, String, Option<Vec<u8>>)>,
    pub ops: Vec<wit::AtomicOp>,
    pub messages: Vec<(Option<u128>, Vec<Span>)>,
}

const CELL_FLAG: u64 = 0x8000;

impl Frame {
    pub fn player_handle(&self, i: usize) -> u64 {
        (self.serial << 16) | i as u64
    }
    pub fn cell_handle(&self, i: usize) -> u64 {
        (self.serial << 16) | CELL_FLAG | i as u64
    }
    fn resolve_player(&self, h: u64) -> wasmtime::Result<u128> {
        if h >> 16 != self.serial || h & CELL_FLAG != 0 {
            wasmtime::bail!("stale or foreign player handle");
        }
        self.players.get((h & 0x7fff) as usize).copied().ok_or_else(|| wasmtime::format_err!("bad player handle"))
    }
    fn resolve_cell(&self, h: u64) -> wasmtime::Result<CellKey> {
        if h >> 16 != self.serial || h & CELL_FLAG == 0 {
            wasmtime::bail!("stale or foreign cell handle");
        }
        self.cells.get((h & 0x7fff) as usize).copied().ok_or_else(|| wasmtime::format_err!("bad cell handle"))
    }
}

pub(crate) struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    pub limits: StoreLimits,
    pub plugin: usize,
    pub id: Arc<str>,
    pub shared: Arc<Shared>,
    pub frame: Option<Frame>,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl HostState {
    fn frame(&mut self) -> wasmtime::Result<&mut Frame> {
        self.frame.as_mut().ok_or_else(|| wasmtime::format_err!("host call outside an event"))
    }
}

impl wit::Host for HostState {}

impl kiln::api::state::Host for HostState {
    fn get(&mut self, s: kiln::api::state::Scope, key: String) -> wasmtime::Result<Option<Vec<u8>>> {
        let plugin = self.plugin;
        let shared = self.shared.clone();
        let f = self.frame()?;
        let target = match s {
            kiln::api::state::Scope::Player(h) => Target::Player(f.resolve_player(h)?),
            kiln::api::state::Scope::Cell(h) => Target::Cell(f.resolve_cell(h)?),
        };
        if let Some((_, _, v)) = f.writes.iter().rev().find(|(t, k, _)| *t == target && *k == key) {
            return Ok(v.clone());
        }
        Ok(match target {
            Target::Player(u) => shared.players.lock().unwrap().get(&u).and_then(|ns| ns.get(plugin, &key).cloned()),
            Target::Cell(c) => shared.cells.lock().unwrap().cells.get(&c).and_then(|ns| ns.get(plugin, &key).cloned()),
        })
    }

    fn put(&mut self, s: kiln::api::state::Scope, key: String, val: Option<Vec<u8>>) -> wasmtime::Result<()> {
        let f = self.frame()?;
        let target = match s {
            kiln::api::state::Scope::Player(h) => Target::Player(f.resolve_player(h)?),
            kiln::api::state::Scope::Cell(h) => Target::Cell(f.resolve_cell(h)?),
        };
        if key.len() > 256 || val.as_ref().is_some_and(|v| v.len() > 1 << 20) {
            wasmtime::bail!("state key or value too large");
        }
        f.writes.push((target, key, val));
        Ok(())
    }

    fn global_get(&mut self, key: String) -> wasmtime::Result<Option<wit::GlobalValue>> {
        let plugin = self.plugin;
        let shared = self.shared.clone();
        let f = self.frame()?;
        let v = if f.global_ctx {
            shared.globals.lock().unwrap().get(plugin, &key).cloned()
        } else {
            f.snapshot.get(plugin, &key).cloned()
        };
        Ok(v.map(to_wit_value))
    }

    fn submit(&mut self, op: wit::AtomicOp) -> wasmtime::Result<u64> {
        let ticket = self.shared.next_ticket.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let f = self.frame()?;
        if f.ops.len() >= 1024 {
            wasmtime::bail!("too many atomic operations in one call");
        }
        f.ops.push(op);
        Ok(ticket)
    }
}

impl kiln::api::chat::Host for HostState {
    fn send(&mut self, to: u64, text: Vec<wit::Span>) -> wasmtime::Result<()> {
        let f = self.frame()?;
        let uuid = f.resolve_player(to)?;
        f.messages.push((Some(uuid), text.into_iter().map(from_wit_span).collect()));
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

/// Links what the manifest grants: `state` and `log` always, `chat` with `player.message`,
/// and WASI (no environment, no preopens unless `fs.data`, no sockets).
pub(crate) fn linker(engine: &wasmtime::Engine, chat: bool) -> anyhow::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    kiln::api::state::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::log::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    if chat {
        kiln::api::chat::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    Ok(linker)
}

/// A fresh store for one instance of plugin `plugin`.
pub(crate) fn new_store(
    engine: &wasmtime::Engine,
    plugin: usize,
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
    let limits = StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(32).tables(32).memories(4).table_elements(1 << 16).build();
    let state = HostState { wasi: wasi.build(), table: ResourceTable::new(), limits, plugin, id, shared, frame: None };
    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limits);
    store
}

pub(crate) type Pre = InstancePre<HostState>;
