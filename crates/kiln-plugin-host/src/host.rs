//! The store side of a call: `HostState`, the per-call frame (handles, buffered effects), the
//! host implementations of the imported interfaces, and stores.
//!
//! The frame lives in the store and is reused call after call (its vectors keep their
//! capacity): starting a call is a few stores, not allocations.

use crate::effects::{BlockChange, EffectKind, ItemSpec, MenuSpec, PlayerInfo, SpawnSpec};
use crate::manifest::{Capability, Manifest};
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

/// Effects, operations and tasks one call may queue.
const MAX_EFFECTS: usize = 1024;
/// Changes in one `set-blocks`.
const MAX_BLOCK_CHANGES: usize = 4096;

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
    /// Ordering key of the call's operations, tasks and effects (the acting player).
    pub source: u128,
    pub players: Vec<u128>,
    /// Names of `players` (by index; the strings keep their capacity across calls).
    names: Vec<String>,
    operators: Vec<bool>,
    /// What the host knew of each of `players` when the call started.
    pub infos: Vec<PlayerInfo>,
    pub cells: Vec<CellKey>,
    /// Entities of the event with their plugin data (moved in for the call, moved back out).
    pub entities: Vec<(u128, EntityData)>,
    pub writes: Vec<(Target, String, Option<Vec<u8>>)>,
    pub ops: Vec<(u64, wit::AtomicOp)>,
    /// Effects with their tickets (0 for messages).
    pub effects: Vec<(u64, EffectKind)>,
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
    /// The index of a player handle of this call.
    pub fn player_index(&self, generation: u32, h: u64) -> wasmtime::Result<usize> {
        let i = self.resolve(generation, KIND_PLAYER, h)?;
        if i >= self.players.len() {
            wasmtime::bail!("bad player handle");
        }
        Ok(i)
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

    /// A player of the call (handle index = position), with their name and what the host
    /// knows of them.
    pub fn push_player(&mut self, uuid: u128, name: &str, operator: bool, info: Option<&PlayerInfo>) {
        let i = self.players.len();
        self.players.push(uuid);
        if self.names.len() <= i {
            self.names.push(String::new());
        }
        let n = &mut self.names[i];
        n.clear();
        n.push_str(name);
        self.operators.push(operator);
        self.infos.push(info.copied().unwrap_or_default());
    }

    pub fn is_clean(&self) -> bool {
        self.writes.is_empty() && self.ops.is_empty() && self.effects.is_empty() && self.tasks.is_empty() && self.cancels.is_empty()
    }

    /// Forgets the last call (keeping the capacity) and starts the next one.
    pub fn reset(&mut self, global_ctx: bool, source: u128) {
        self.serial = self.serial.wrapping_add(1);
        self.global_ctx = global_ctx;
        self.source = source;
        self.players.clear();
        self.operators.clear();
        self.infos.clear();
        self.cells.clear();
        self.entities.clear();
        self.writes.clear();
        self.ops.clear();
        self.effects.clear();
        self.tasks.clear();
        self.cancels.clear();
        self.deny_msg = None;
        self.seq = None;
        self.n = 0;
        self.rng = None;
        self.snapshot = None;
    }

    pub fn player_name(&self, i: usize) -> &str {
        &self.names[i]
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
    /// The other plugins' instances lent for this call, when the plugin may raise events
    /// (`events.raise`) and others subscribed to `custom`.
    pub peers: Option<Box<crate::Peers>>,
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

    /// Queues an effect and returns its ticket.
    fn effect(&mut self, kind: EffectKind) -> wasmtime::Result<u64> {
        if self.frame()?.effects.len() >= MAX_EFFECTS {
            wasmtime::bail!("too many effects in one call");
        }
        let (ticket, _) = self.new_id();
        self.frame.effects.push((ticket, kind));
        Ok(ticket)
    }

    /// `<plugin id>:<name>`: the namespace of ids a plugin makes up (menus, tags, boss bars).
    fn owned(&self, name: &str) -> String {
        format!("{}:{name}", self.id)
    }

    fn item_spec(&self, s: wit::ItemStack) -> wasmtime::Result<ItemSpec> {
        if s.count == 0 || s.count > 99 {
            wasmtime::bail!("an item stack has 1 to 99 items");
        }
        if s.item.len() > 128 || s.lore.len() > 64 || s.name.as_ref().is_some_and(|n| n.len() > 64) {
            wasmtime::bail!("item stack too large");
        }
        if s.tag.as_ref().is_some_and(|t| t.len() > 128 || t.is_empty()) {
            wasmtime::bail!("an item tag is 1 to 128 bytes");
        }
        Ok(ItemSpec {
            item: s.item,
            count: s.count,
            name: s.name.map(spans),
            lore: s.lore.into_iter().map(spans).collect(),
            tag: s.tag.map(|t| self.owned(&t)),
            model: s.model,
            glint: s.glint,
        })
    }
}

fn spans(v: Vec<wit::Span>) -> Vec<Span> {
    v.into_iter().map(from_wit_span).collect()
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
        if self.frame()?.ops.len() >= MAX_EFFECTS {
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
        let i = f.player_index(generation, p)?;
        Ok(f.player_name(i).to_owned())
    }

    fn deny_message(&mut self, text: Vec<wit::Span>) -> wasmtime::Result<()> {
        self.frame()?.deny_msg = Some(text.into_iter().map(from_wit_span).collect());
        Ok(())
    }

    fn player_info(&mut self, p: u64) -> wasmtime::Result<wit::PlayerInfo> {
        let generation = self.generation;
        let f = self.frame()?;
        let i = f.player_index(generation, p)?;
        let v = f.infos[i];
        Ok(wit::PlayerInfo {
            level: v.level,
            pos: (v.pos[0], v.pos[1], v.pos[2]),
            rot: (v.rot[0], v.rot[1]),
            health: v.health,
            food: v.food,
            game_mode: match v.game_mode {
                1 => wit::GameMode::Creative,
                2 => wit::GameMode::Adventure,
                3 => wit::GameMode::Spectator,
                _ => wit::GameMode::Survival,
            },
            on_ground: v.on_ground,
            sneaking: v.sneaking,
            sprinting: v.sprinting,
            flying: v.flying,
            held: v.held,
            held_count: v.held_count,
        })
    }

    fn online(&mut self) -> wasmtime::Result<Vec<wit::OnlinePlayer>> {
        self.frame()?;
        let list = self.shared.online.lock().unwrap().clone();
        Ok(list.iter().map(|p| wit::OnlinePlayer { uuid: wit_uuid(p.uuid), name: p.name.clone(), level: p.level }).collect())
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
            Kind::DamageType => crate::RegistryKind::DamageType,
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
        if self.frame.effects.len() >= MAX_EFFECTS {
            wasmtime::bail!("too many effects in one call");
        }
        self.frame.effects.push((0, EffectKind::Message { to: Some(uuid), text: spans(text) }));
        Ok(())
    }

    fn broadcast(&mut self, text: Vec<wit::Span>) -> wasmtime::Result<()> {
        if self.frame()?.effects.len() >= MAX_EFFECTS {
            wasmtime::bail!("too many effects in one call");
        }
        self.frame.effects.push((0, EffectKind::Message { to: None, text: spans(text) }));
        Ok(())
    }
}

impl kiln::api::hud::Host for HostState {
    fn title(&mut self, to: wit::Uuid, title: Vec<wit::Span>, subtitle: Vec<wit::Span>, fade_in: u32, stay: u32, fade_out: u32) -> wasmtime::Result<u64> {
        self.effect(EffectKind::Title { to: from_wit_uuid(to), title: spans(title), subtitle: spans(subtitle), fade_in, stay, fade_out })
    }

    fn action_bar(&mut self, to: wit::Uuid, text: Vec<wit::Span>) -> wasmtime::Result<u64> {
        self.effect(EffectKind::ActionBar { to: from_wit_uuid(to), text: spans(text) })
    }

    fn sidebar(&mut self, to: wit::Uuid, title: Vec<wit::Span>, lines: Vec<Vec<wit::Span>>) -> wasmtime::Result<u64> {
        if lines.len() > 15 {
            wasmtime::bail!("a sidebar has at most 15 lines");
        }
        self.effect(EffectKind::Sidebar { to: from_wit_uuid(to), title: spans(title), lines: lines.into_iter().map(spans).collect() })
    }

    fn clear_sidebar(&mut self, to: wit::Uuid) -> wasmtime::Result<u64> {
        self.effect(EffectKind::ClearSidebar { to: from_wit_uuid(to) })
    }

    fn bossbar(
        &mut self,
        to: wit::Uuid,
        id: String,
        text: Vec<wit::Span>,
        progress: f32,
        color: wit::BossColor,
        style: wit::BossStyle,
    ) -> wasmtime::Result<u64> {
        if id.is_empty() || id.len() > 64 {
            wasmtime::bail!("a boss bar id is 1 to 64 bytes");
        }
        let id = self.owned(&id);
        self.effect(EffectKind::Bossbar {
            to: from_wit_uuid(to),
            id,
            text: spans(text),
            progress: if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 },
            color: color as u8,
            style: style as u8,
        })
    }

    fn clear_bossbar(&mut self, to: wit::Uuid, id: String) -> wasmtime::Result<u64> {
        let id = self.owned(&id);
        self.effect(EffectKind::ClearBossbar { to: from_wit_uuid(to), id })
    }
}

impl kiln::api::players::Host for HostState {
    fn teleport(&mut self, who: wit::Uuid, level: u32, x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> wasmtime::Result<u64> {
        if level as usize >= self.shared.registries.levels.len() {
            wasmtime::bail!("unknown level {level}");
        }
        if ![x, y, z, yaw as f64, pitch as f64].iter().all(|v| v.is_finite()) || x.abs() > 3.0e7 || z.abs() > 3.0e7 || y.abs() > 2.0e7 {
            wasmtime::bail!("teleport target out of range");
        }
        self.effect(EffectKind::Teleport { who: from_wit_uuid(who), level, pos: [x, y, z], rot: [yaw, pitch] })
    }

    fn set_game_mode(&mut self, who: wit::Uuid, mode: wit::GameMode) -> wasmtime::Result<u64> {
        self.effect(EffectKind::GameMode { who: from_wit_uuid(who), mode: mode as u8 })
    }

    fn heal(&mut self, who: wit::Uuid) -> wasmtime::Result<u64> {
        self.effect(EffectKind::Heal { who: from_wit_uuid(who) })
    }

    fn kick(&mut self, who: wit::Uuid, reason: Vec<wit::Span>) -> wasmtime::Result<u64> {
        self.effect(EffectKind::Kick { who: from_wit_uuid(who), reason: spans(reason) })
    }
}

impl kiln::api::inventory::Host for HostState {
    fn give(&mut self, who: wit::Uuid, stack: wit::ItemStack) -> wasmtime::Result<u64> {
        let item = self.item_spec(stack)?;
        self.effect(EffectKind::Give { who: from_wit_uuid(who), item })
    }

    fn take(&mut self, who: wit::Uuid, item: String, count: u32) -> wasmtime::Result<u64> {
        if count == 0 || count > 36 * 64 || item.len() > 128 {
            wasmtime::bail!("bad item count or key");
        }
        self.effect(EffectKind::Take { who: from_wit_uuid(who), item, count })
    }

    fn clear(&mut self, who: wit::Uuid) -> wasmtime::Result<u64> {
        self.effect(EffectKind::Clear { who: from_wit_uuid(who) })
    }

    fn open_menu(&mut self, who: wit::Uuid, spec: wit::MenuSpec) -> wasmtime::Result<u64> {
        if spec.rows == 0 || spec.rows > 6 || spec.id.is_empty() || spec.id.len() > 64 {
            wasmtime::bail!("a menu has 1 to 6 rows and an id of 1 to 64 bytes");
        }
        let mut items = Vec::with_capacity(spec.items.len());
        for it in spec.items {
            if it.slot as usize >= spec.rows as usize * 9 {
                wasmtime::bail!("menu slot {} is outside {} rows", it.slot, spec.rows);
            }
            items.push((it.slot, self.item_spec(it.stack)?));
        }
        let menu = MenuSpec { id: self.owned(&spec.id), title: spans(spec.title), rows: spec.rows, items };
        self.effect(EffectKind::OpenMenu { who: from_wit_uuid(who), menu })
    }

    fn set_slot(&mut self, who: wit::Uuid, menu: String, slot: u8, stack: Option<wit::ItemStack>) -> wasmtime::Result<u64> {
        let item = stack.map(|s| self.item_spec(s)).transpose()?;
        let menu = self.owned(&menu);
        self.effect(EffectKind::SetSlot { who: from_wit_uuid(who), menu, slot, item })
    }

    fn close_menu(&mut self, who: wit::Uuid) -> wasmtime::Result<u64> {
        self.effect(EffectKind::CloseMenu { who: from_wit_uuid(who) })
    }
}

impl kiln::api::entities::Host for HostState {
    fn spawn(&mut self, spec: wit::SpawnSpec) -> wasmtime::Result<wit::Uuid> {
        if spec.level as usize >= self.shared.registries.levels.len() {
            wasmtime::bail!("unknown level {}", spec.level);
        }
        let (x, y, z) = spec.pos;
        if ![x, y, z, spec.yaw as f64].iter().all(|v| v.is_finite()) || x.abs() > 3.0e7 || z.abs() > 3.0e7 || y.abs() > 2.0e7 || spec.kind.len() > 128 {
            wasmtime::bail!("spawn position out of range");
        }
        if self.frame()?.effects.len() >= MAX_EFFECTS {
            wasmtime::bail!("too many effects in one call");
        }
        // A version-4 uuid derived from the call: known to the plugin at once, the same on
        // every run.
        let (hi, _) = self.derive();
        let (lo, _) = self.derive();
        let uuid = (((mix(hi) & !0xf000) | 0x4000) as u128) << 64 | ((mix(lo) & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000) as u128;
        let (ticket, _) = self.new_id();
        let spec = SpawnSpec {
            kind: spec.kind,
            level: spec.level,
            pos: [x, y, z],
            yaw: spec.yaw,
            name: spec.name.map(spans),
            no_ai: spec.no_ai,
            invulnerable: spec.invulnerable,
            silent: spec.silent,
            no_gravity: spec.no_gravity,
            uuid,
        };
        self.frame.effects.push((ticket, EffectKind::Spawn(spec)));
        Ok(wit_uuid(uuid))
    }

    fn remove(&mut self, level: u32, id: wit::Uuid) -> wasmtime::Result<u64> {
        self.effect(EffectKind::Remove { level, uuid: from_wit_uuid(id) })
    }
}

impl kiln::api::blocks::Host for HostState {
    fn set_blocks(&mut self, cell: u64, level: u32, changes: Vec<wit::BlockChange>) -> wasmtime::Result<Result<u64, wit::EditError>> {
        let generation = self.generation;
        let cell = self.frame()?.resolve_cell(generation, cell)?;
        if level as usize >= self.shared.registries.levels.len() || level != cell.dim {
            return Ok(Err(wit::EditError::UnknownLevel));
        }
        if changes.len() > MAX_BLOCK_CHANGES {
            return Ok(Err(wit::EditError::TooMany));
        }
        let mut out = Vec::with_capacity(changes.len());
        for c in changes {
            if c.x >> 7 != cell.x || c.z >> 7 != cell.z {
                return Ok(Err(wit::EditError::OutsideCell));
            }
            if c.state.is_empty() || c.state.len() > 256 {
                return Ok(Err(wit::EditError::BadState));
            }
            out.push(BlockChange { pos: [c.x, c.y, c.z], state: c.state });
        }
        self.effect(EffectKind::SetBlocks { level, changes: out }).map(Ok)
    }
}

impl kiln::api::events::Host for HostState {
    fn raise(&mut self, name: String, payload: Vec<u8>, actor: Option<u64>) -> wasmtime::Result<wit::Decision> {
        let generation = self.generation;
        let f = self.frame()?;
        if name.is_empty() || name.len() > 64 || payload.len() > 1 << 16 {
            wasmtime::bail!("an event name is 1 to 64 bytes, a payload at most 64 KiB");
        }
        let actor = match actor {
            Some(h) => {
                let i = f.player_index(generation, h)?;
                Some((f.players[i], f.operators[i], f.player_name(i).to_owned(), f.infos[i]))
            }
            None => None,
        };
        // Depth one: a handler of a raised event has no peers lent, so it raises into nothing.
        let Some(mut peers) = self.peers.take() else { return Ok(wit::Decision::Allow) };
        let ev = crate::CustomEvent { name: self.owned(&name), source: self.id.clone(), payload, source_index: self.plugin };
        let source = self.frame.source;
        let verdict = crate::dispatch_custom(&mut peers, &self.shared, &ev, actor.as_ref().map(|(u, o, n, i)| (*u, *o, n.as_str(), i)), source);
        self.peers = Some(peers);
        Ok(match verdict {
            crate::Verdict::Allow => wit::Decision::Allow,
            crate::Verdict::Deny(_) => wit::Decision::Deny,
        })
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
    fn error(&mut self, msg: String) -> wasmtime::Result<()> {
        tracing::error!("[{}] {msg}", self.id);
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

/// Links what the manifest grants: `state`, `event`, `env`, `registry` and `log` always,
/// the effect interfaces with their capabilities, and WASI (no environment, no preopens
/// unless `fs.data`, no sockets).
pub(crate) fn linker(engine: &wasmtime::Engine, manifest: &Manifest) -> anyhow::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    kiln::api::state::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::event::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::env::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::registry::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    kiln::api::log::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    if manifest.has(Capability::PlayerMessage) {
        kiln::api::chat::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::Scheduler) {
        kiln::api::scheduler::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::PlayerHud) {
        kiln::api::hud::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::PlayerControl) {
        kiln::api::players::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::Inventory) {
        kiln::api::inventory::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::EntityControl) {
        kiln::api::entities::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::WorldWrite) {
        kiln::api::blocks::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
    }
    if manifest.has(Capability::EventsRaise) {
        kiln::api::events::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)?;
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
        peers: None,
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
