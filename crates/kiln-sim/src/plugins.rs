//! WASM plugins in the tick (design §11), through `kiln-plugin-host`.
//!
//! - Plugins load from `SimConfig::plugins` (`KILN_PLUGINS_DIR`) at start; their namespaces
//!   live in `<world>/kiln/plugins`, compiled components in `<world>/kiln/plugins/cache` (or
//!   `<plugins>/.cache` without a world). Commands they register join the dispatcher.
//! - Region instance sets follow the regionizer: after every topology change each level's
//!   regions get or lose theirs ([`Sim::sync_plugin_regions`]).
//! - B0: staged hot reloads swap in, the global atomic operations queued in the last tick
//!   apply, their results go back, due tasks run (player tasks in the region holding the
//!   player, position tasks in the region owning the position).
//! - P: a player's break (start and finish of digging), use-on-block (placement and
//!   interaction), bucket use (the block the bucket would fill or empty) and entity
//!   interaction are cancellable events in the region's instances, before the vanilla
//!   handling; a denial puts the client's blocks and inventory back. Block changes that went
//!   through are observed and sent in one batch per region at the end of the phase; a
//!   survival break the server completes later (the client finished early) is observed in
//!   the block phase that completes it.
//! - Entity data (the entity scope) lives in the entity's NBT under `kiln:plugin`, so it
//!   follows the entity across regions and is saved with its chunk.
//! - PX: chat (cancel or rewrite), commands (cancel), plugin commands (global instance), and
//!   region packets queued behind them (the same checks); `/kiln plugins` lists plugins and
//!   `/kiln plugins reload <id>` reloads one (compiled off the tick, swapped in B0).
//! - Join and leave run in the global instances; messages plugins sent go out after P and
//!   after G, in a deterministic order.

use crate::region::Env;
use crate::{DIMENSIONS, Player, Sim};
use kiln_command::arguments::ArgumentType;
use kiln_command::dispatcher::{argument, literal};
use kiln_link::{ConnId, PlayIn};
use kiln_plugin_host::{
    Actor, ChatOutcome, EntityData, EntityRef, ExecMode, PlayerAt, PluginRuntime, RegionPlugins, Registries, RegistryKind,
    RuntimeConfig, Span, Verdict, World,
};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_region::{CellPos, CellSet, RegionId};
use kiln_world::{Blocks, Cell};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, OnceLock};
use tracing::{info, warn};
use uuid::Uuid;

/// Where plugins come from and how they are budgeted.
#[derive(Clone, Debug)]
pub struct PluginSettings {
    /// `<dir>/<plugin>/plugin.toml` + `plugin.wasm` (`KILN_PLUGINS_DIR`).
    pub dir: std::path::PathBuf,
    /// Ordered or strict (`KILN_PLUGIN_MODE=strict`); see `kiln_plugin_host` for the two.
    pub mode: ExecMode,
    /// Ordered mode: wall-clock budget of each cancellable call (`KILN_PLUGIN_BUDGET_US`,
    /// default 500 µs). Timeouts depend on the machine's load, so lockstep determinism tests
    /// either use a budget no call reaches or strict mode.
    pub call_budget: std::time::Duration,
    /// Strict mode: fuel of each call (`KILN_PLUGIN_FUEL`).
    pub call_fuel: u64,
    /// Seed of the plugins' random streams, tickets and task handles.
    pub seed: u64,
}

impl PluginSettings {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        let d = RuntimeConfig::default();
        PluginSettings { dir: dir.into(), mode: ExecMode::Ordered, call_budget: d.call_budget, call_fuel: d.call_fuel, seed: 0 }
    }

    /// Strict mode with `fuel` per call: budgets that replay exactly.
    pub fn strict(dir: impl Into<std::path::PathBuf>, fuel: u64) -> Self {
        PluginSettings { mode: ExecMode::Strict, call_fuel: fuel, ..PluginSettings::new(dir) }
    }
}

pub(crate) struct SimPlugins {
    rt: PluginRuntime,
    /// Operators, for the events' `operator` flag (refreshed in B0).
    ops: Arc<HashSet<Uuid>>,
    /// Command names registered in the dispatcher.
    registered: HashSet<String>,
}

/// A region's plugins for one phase of region work.
pub(crate) struct RegionHook<'a> {
    rp: &'a mut RegionPlugins,
    ops: Arc<HashSet<Uuid>>,
    /// Allowed breaks and placements, to observe if the block really changed.
    watch: Vec<Watch>,
}

struct Watch {
    uuid: Uuid,
    name: String,
    operator: bool,
    pos: [i32; 3],
    before: u16,
    broken: bool,
}

fn actor<'a>(p: &'a Player, ops: &HashSet<Uuid>) -> Actor<'a> {
    Actor { uuid: p.uuid.as_u128(), name: &p.name, operator: ops.contains(&p.uuid) }
}

/// The registries plugins see: levels, blocks, items and entity types by registry id, with
/// the synchronized tags.
fn registries() -> Arc<Registries> {
    static REG: OnceLock<Arc<Registries>> = OnceLock::new();
    REG.get_or_init(|| {
        let list = |r: &str| kiln_data::builtin_entries(r).unwrap_or(&[]).iter().map(|s| s.to_string()).collect();
        let levels = DIMENSIONS.iter().map(|(k, _)| (*k).to_owned()).collect();
        let reg = Registries::new(levels, list("minecraft:block"), list("minecraft:item"), list("minecraft:entity_type"));
        Arc::new(reg.with_tags(Arc::new(|kind, tag| {
            let registry = match kind {
                RegistryKind::Block => "minecraft:block",
                RegistryKind::Item => "minecraft:item",
                RegistryKind::EntityType => "minecraft:entity_type",
                RegistryKind::Level => return None,
            };
            let full = if tag.contains(':') { tag.to_owned() } else { format!("minecraft:{tag}") };
            let (_, tags) = kiln_data::registries::TAGS.iter().find(|(r, _)| *r == registry)?;
            let (_, ids) = tags.iter().find(|(t, _)| *t == full)?;
            Some(ids.iter().map(|&i| i as u32).collect())
        })))
    })
    .clone()
}

/// The block registry id of a state.
fn block_id(state: u16) -> u32 {
    static IDS: OnceLock<Vec<u32>> = OnceLock::new();
    let ids = IDS.get_or_init(|| {
        kiln_data::blocks::BLOCKS
            .iter()
            .map(|b| kiln_data::builtin_id("minecraft:block", b.name).map_or(u32::MAX, |i| i as u32))
            .collect()
    });
    ids.get(kiln_blocks::BlockId::of(state).0 as usize).copied().unwrap_or(u32::MAX)
}

/// Plugin chat text as a text component.
fn text_tag(spans: &[Span]) -> Tag {
    let part = |s: &Span| {
        let mut c = vec![("text".to_owned(), Tag::String(s.text.clone()))];
        if let Some(color) = &s.color {
            c.push(("color".into(), Tag::String(color.clone())));
        }
        if s.bold {
            c.push(("bold".into(), Tag::Byte(1)));
        }
        if s.italic {
            c.push(("italic".into(), Tag::Byte(1)));
        }
        Tag::Compound(c)
    };
    match spans {
        [one] => part(one),
        _ => Tag::Compound(vec![
            ("text".into(), Tag::String(String::new())),
            ("extra".into(), Tag::List(spans.iter().map(part).collect())),
        ]),
    }
}

fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// `Entity.calculateViewVector(xRot, yRot)`.
fn view_vector(rot: [f32; 2]) -> [f64; 3] {
    let f = rot[1] as f64 * std::f64::consts::PI / 180.0;
    let g = -rot[0] as f64 * std::f64::consts::PI / 180.0;
    let (h, i, j, k) = (g.cos(), g.sin(), f.cos(), f.sin());
    [i * j, -k, h * j]
}

/// The block a bucket aims at: the first non-air block along the look within reach (voxel
/// traversal), and the block before it (where a filled bucket empties).
fn bucket_target(p: &Player, rot: [f32; 2], cells: &CellSet<Cell>) -> Option<([i32; 3], [i32; 3])> {
    let eye = p.eye_position();
    let dir = view_vector(rot);
    let reach = if p.game_mode == 1 { 5.0 } else { 4.5 };
    let mut cur = [eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32];
    let mut prev = cur;
    let step: [i32; 3] = std::array::from_fn(|i| if dir[i] > 0.0 { 1 } else { -1 });
    let delta: [f64; 3] = std::array::from_fn(|i| if dir[i] == 0.0 { f64::INFINITY } else { (1.0 / dir[i]).abs() });
    let mut t_max: [f64; 3] = std::array::from_fn(|i| {
        if dir[i] == 0.0 {
            f64::INFINITY
        } else {
            let edge = if dir[i] > 0.0 { cur[i] as f64 + 1.0 } else { cur[i] as f64 };
            (edge - eye[i]) / dir[i]
        }
    });
    for _ in 0..16 {
        let state = cells.get_block(cur[0], cur[1], cur[2])?;
        if !kiln_data::blocks_types::is_air(state) {
            return Some((cur, prev));
        }
        let axis = (0..3).min_by(|&a, &b| t_max[a].total_cmp(&t_max[b])).unwrap();
        if t_max[axis] > reach {
            return None;
        }
        prev = cur;
        cur[axis] += step[axis];
        t_max[axis] += delta[axis];
    }
    None
}

/// The item id in a hand (`None` when empty).
fn held_item(p: &Player, hand: i32) -> (Option<u32>, &'static str) {
    use kiln_item::component::EquipmentSlot;
    let held = if hand == 0 { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
    if held.is_empty() {
        return (None, "minecraft:air");
    }
    let name = kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(held.item() as usize).copied()).unwrap_or("minecraft:air");
    (Some(held.item() as u32), name)
}

/// A denied use: the client's view of the blocks and its inventory are put back.
fn restore(p: &mut Player, env: &Env, spawns: &mut Vec<crate::entities::Spawn>, blocks: &[([i32; 3], u16)], sequence: i32, msg: Option<Vec<Span>>) {
    for &(pos, state) in blocks {
        p.send(packets::block_update(pos, state));
    }
    p.ack_block_changes = p.ack_block_changes.max(sequence);
    // The client may have used up the item in its prediction.
    p.with_menu(&env.rules, spawns, |menu, _, env| menu.send_all_data_to_remote(env));
    if let Some(m) = msg {
        p.send(packets::system_chat(text_tag(&m), false));
    }
}

/// P and PX: a region packet that breaks or uses a block goes to the region's plugins first.
/// Returns whether a plugin denied it (the packet is then dropped and the client's view of
/// the blocks restored).
pub(crate) fn deny_packet(
    hook: &mut RegionHook,
    p: &mut Player,
    cells: &CellSet<Cell>,
    env: &Env,
    pkt: &PlayIn,
    spawns: &mut Vec<crate::entities::Spawn>,
) -> bool {
    let block = |pos: [i32; 3]| cells.get_block(pos[0], pos[1], pos[2]).unwrap_or(0);
    match *pkt {
        PlayIn::PlayerAction { action, pos, sequence, .. }
            if action == crate::digging::START_DESTROY_BLOCK || action == crate::digging::STOP_DESTROY_BLOCK =>
        {
            let state = block(pos);
            if kiln_data::blocks_types::is_air(state) {
                return false;
            }
            let a = actor(p, &hook.ops);
            match hook.rp.block_break(&a, pos, block_id(state)) {
                Verdict::Allow => {
                    hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos, before: state, broken: true });
                    false
                }
                Verdict::Deny(msg) => {
                    p.digging = None;
                    p.delayed_destroy = None;
                    p.send(packets::block_update(pos, state));
                    p.ack_block_changes = p.ack_block_changes.max(sequence);
                    if let Some(m) = msg {
                        p.send(packets::system_chat(text_tag(&m), false));
                    }
                    true
                }
            }
        }
        PlayIn::UseItemOn { hand, pos, face, sequence, .. } => {
            let Some(dir) = crate::blocks::direction(face) else { return false };
            let step = dir.step();
            let next = [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]];
            let (item, _) = held_item(p, hand);
            let a = actor(p, &hook.ops);
            match hook.rp.block_place(&a, next, pos, item) {
                Verdict::Allow => {
                    for at in [pos, next] {
                        hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos: at, before: block(at), broken: false });
                    }
                    false
                }
                Verdict::Deny(msg) => {
                    restore(p, env, spawns, &[(pos, block(pos)), (next, block(next))], sequence, msg);
                    true
                }
            }
        }
        // Buckets act on the block they aim at (`BucketItem.use` ray-traces from the eye):
        // the same placement check applies to the block they would fill or empty.
        PlayIn::UseItem { hand, sequence, yaw, pitch } => {
            let hand = if hand == kiln_proto::packets::serverbound::Hand::Off { 1 } else { 0 };
            let (item, name) = held_item(p, hand);
            if !name.ends_with("bucket") || name == "minecraft:milk_bucket" {
                return false;
            }
            // The packet carries the rotation the client used (`handleUseItem` applies it).
            let Some((hit, before)) = bucket_target(p, [yaw, pitch], cells) else { return false };
            let target = if name == "minecraft:bucket" { hit } else { before };
            let a = actor(p, &hook.ops);
            match hook.rp.block_place(&a, target, hit, item) {
                Verdict::Allow => {
                    hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos: target, before: block(target), broken: false });
                    false
                }
                Verdict::Deny(msg) => {
                    restore(p, env, spawns, &[(hit, block(hit)), (before, block(before))], sequence, msg);
                    true
                }
            }
        }
        _ => false,
    }
}

/// An entity's plugin data from its NBT (`kiln:plugin`: plugin id → key → bytes).
fn entity_data(extra: &[(String, Tag)]) -> EntityData {
    let mut data = EntityData::new();
    let Some((_, Tag::Compound(plugins))) = extra.iter().find(|(k, _)| k == KEY) else { return data };
    for (id, kv) in plugins {
        let Tag::Compound(kv) = kv else { continue };
        let map = kv
            .iter()
            .filter_map(|(k, v)| match v {
                Tag::ByteArray(b) => Some((k.clone(), b.iter().map(|&x| x as u8).collect())),
                _ => None,
            })
            .collect();
        data.insert(id.clone(), map);
    }
    data
}

fn set_entity_data(extra: &mut Vec<(String, Tag)>, data: &EntityData) {
    extra.retain(|(k, _)| k != KEY);
    if data.is_empty() {
        return;
    }
    let plugins = data
        .iter()
        .map(|(id, kv)| {
            let kv = kv.iter().map(|(k, v)| (k.clone(), Tag::ByteArray(v.iter().map(|&x| x as i8).collect()))).collect();
            (id.clone(), Tag::Compound(kv))
        })
        .collect();
    extra.push((KEY.to_owned(), Tag::Compound(plugins)));
}

const KEY: &str = "kiln:plugin";

/// P: a right click on an entity goes to the region's plugins first (they may read and write
/// the entity's data). Returns whether a plugin denied it.
pub(crate) fn deny_interact(hook: &mut RegionHook, p: &mut Player, entities: &mut crate::entities::Entities, entity_id: i32) -> bool {
    let Ok(idx) = entities.list.binary_search_by_key(&entity_id, |e| e.id) else { return false };
    let e = &mut entities.list[idx];
    let Some(kind) = kiln_data::builtin_id("minecraft:entity_type", e.kind.name) else { return false };
    let (uuid, pos) = (e.uuid.as_u128(), e.pos);
    // Out of reach: vanilla drops the packet before any interaction (`canInteractWithEntity`).
    let eye = p.eye_position();
    if (0..3).map(|i| (pos[i] - eye[i]).powi(2)).sum::<f64>() > 8.0 * 8.0 {
        return false;
    }
    // The vanilla state carries the NBT fields Kiln does not model, `kiln:plugin` among them.
    let Some(phys) = e.phys.as_mut() else { return false };
    let mut data = entity_data(&phys.extra);
    let before = data.clone();
    let a = actor(p, &hook.ops);
    let mut r = EntityRef { uuid, kind: kind as u32, pos, data: &mut data };
    let v = hook.rp.entity_interact(&a, &mut r);
    if data != before {
        set_entity_data(&mut phys.extra, &data);
    }
    match v {
        Verdict::Allow => false,
        Verdict::Deny(msg) => {
            if let Some(m) = msg {
                p.send(packets::system_chat(text_tag(&m), false));
            }
            true
        }
    }
}

/// End of a phase's packets (or of the block phase): allowed breaks and placements that
/// changed their block are observed, and the batch goes to the region's observers.
pub(crate) fn after_packets(hook: &mut RegionHook, cells: &CellSet<Cell>, _env: &Env) {
    for w in std::mem::take(&mut hook.watch) {
        let now = cells.get_block(w.pos[0], w.pos[1], w.pos[2]).unwrap_or(0);
        if now == w.before || (w.broken && !kiln_data::blocks_types::is_air(now)) {
            continue;
        }
        let a = Actor { uuid: w.uuid.as_u128(), name: &w.name, operator: w.operator };
        hook.rp.observe_block(w.broken, &a, w.pos, block_id(if w.broken { w.before } else { now }));
    }
    hook.rp.flush_observed();
}

/// Before the block phase: a survival break the client finished early completes in it
/// (`delayed_destroy`); watch those blocks so the break is observed when it happens.
pub(crate) fn watch_delayed_breaks(hook: &mut RegionHook, players: &[&mut Player], cells: &CellSet<Cell>) {
    if !hook.rp.observing() {
        return;
    }
    for p in players {
        if let Some(dig) = p.delayed_destroy {
            let before = cells.get_block(dig.pos[0], dig.pos[1], dig.pos[2]).unwrap_or(0);
            let operator = hook.ops.contains(&p.uuid);
            hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator, pos: dig.pos, before, broken: true });
        }
    }
}

impl SimPlugins {
    /// The region hooks for one parallel phase, by region.
    pub(crate) fn hooks(&mut self) -> BTreeMap<(crate::DimId, RegionId), RegionHook<'_>> {
        let ops = self.ops.clone();
        self.rt
            .regions_mut()
            .map(|((dim, r), rp)| ((dim as crate::DimId, RegionId(r)), RegionHook { rp, ops: ops.clone(), watch: Vec::new() }))
            .collect()
    }

    /// One region's hook (PX: a region packet queued behind a serial one).
    pub(crate) fn hook(&mut self, dim: crate::DimId, region: RegionId) -> Option<RegionHook<'_>> {
        let ops = self.ops.clone();
        self.rt.region_mut(dim as u32, region.0).map(|rp| RegionHook { rp, ops, watch: Vec::new() })
    }
}

/// What B0 routes tasks and results with: the players and the level regions.
struct SimWorld<'a> {
    players: &'a std::collections::HashMap<ConnId, Player>,
    dims: &'a [crate::Dim],
    ops: &'a HashSet<Uuid>,
}

impl World for SimWorld<'_> {
    fn player(&self, uuid: u128) -> Option<PlayerAt> {
        let u = Uuid::from_u128(uuid);
        let p = self.players.values().find(|p| p.uuid == u && !p.disconnected)?;
        Some(PlayerAt { uuid, level: p.dim as u32, region: p.region.0, name: p.name.clone(), operator: self.ops.contains(&u) })
    }

    fn owner(&self, level: u32, x: i32, z: i32) -> Option<u64> {
        self.dims.get(level as usize)?.regions.owner(CellPos::of_block(x, z)).map(|r| r.0)
    }
}

impl Sim {
    /// Loads the plugins of `SimConfig::plugins` and registers their commands.
    pub(crate) fn load_plugins(&mut self) {
        let Some(settings) = self.config.plugins.clone() else { return };
        let data_dir = self.config.world.as_ref().map(|w| w.join("kiln").join("plugins"));
        let cache_dir = data_dir.as_ref().map_or_else(|| settings.dir.join(".cache"), |d| d.join("cache"));
        let cfg = RuntimeConfig {
            data_dir,
            cache_dir: Some(cache_dir),
            registries: registries(),
            spawn: self.spawn,
            mode: settings.mode,
            seed: settings.seed,
            call_budget: settings.call_budget,
            call_fuel: settings.call_fuel,
            ..RuntimeConfig::default()
        };
        let rt = match PluginRuntime::load_dir(&settings.dir, cfg) {
            Ok(rt) => rt,
            Err(e) => {
                warn!("plugins: {e:#}");
                return;
            }
        };
        info!("{} plugins loaded from {} ({:?} mode)", rt.ids().len(), settings.dir.display(), settings.mode);
        let dispatcher = Arc::get_mut(&mut self.commands.dispatcher).expect("dispatcher not shared yet");
        dispatcher.register(
            literal::<Sim>("kiln").requires(kiln_command::vanilla::LEVEL_GAMEMASTERS).then(
                literal::<Sim>("plugins").executes(|_, sim: &mut Sim| Ok(sim.kiln_plugins())).then(literal::<Sim>("reload").then(
                    argument::<Sim>("plugin", ArgumentType::word()).executes(|ctx, sim: &mut Sim| {
                        let id = ctx.arg_text("plugin").unwrap_or("").to_owned();
                        Ok(sim.kiln_plugins_reload(&id))
                    }),
                )),
            ),
        );
        self.plugins = Some(SimPlugins { rt, ops: Arc::new(HashSet::new()), registered: HashSet::new() });
        self.register_plugin_commands();
        self.sync_plugin_regions();
    }

    /// Registers plugin commands the dispatcher does not have yet (at load, and after a
    /// reload added some; commands are looked up by name when they run). Needs the only
    /// reference to the dispatcher: false if it is shared right now.
    fn register_plugin_commands(&mut self) -> bool {
        let Some(pl) = self.plugins.as_mut() else { return true };
        let new: Vec<(String, u8, usize)> =
            pl.rt.commands().iter().filter(|c| !pl.registered.contains(&c.name)).map(|c| (c.name.clone(), c.permission, c.plugin)).collect();
        if new.is_empty() {
            return true;
        }
        let Some(dispatcher) = Arc::get_mut(&mut self.commands.dispatcher) else { return false };
        for (name, permission, plugin) in new {
            pl.registered.insert(name.clone());
            if dispatcher.find(&[name.as_str()]).is_some() {
                warn!("plugin {}: command /{name} already exists", pl.rt.ids()[plugin]);
                continue;
            }
            let name2 = name.clone();
            let name3 = name.clone();
            dispatcher.register(
                literal::<Sim>(&name)
                    .requires(permission)
                    .executes(move |_, sim| Ok(sim.run_plugin_command(&name2, "")))
                    .then(
                        argument::<Sim>("args", ArgumentType::greedy_string())
                            .executes(move |ctx, sim: &mut Sim| Ok(sim.run_plugin_command(&name3, ctx.arg_text("args").unwrap_or("")))),
                    ),
            );
        }
        true
    }

    fn plugin_reply(&mut self, spans: &[Span]) {
        let pkt = packets::system_chat(text_tag(spans), false);
        match self.commands.source {
            crate::commands::CommandSource::Player(conn) => {
                if let Some(p) = self.players.get_mut(&conn) {
                    p.send(pkt);
                }
            }
            crate::commands::CommandSource::Console => info!("{}", plain(spans)),
        }
    }

    fn run_plugin_command(&mut self, name: &str, args: &str) -> i32 {
        let source = self.commands.source;
        let Some(pl) = self.plugins.as_mut() else { return 0 };
        let Some(plugin) = pl.rt.command_plugin(name).map(|c| c.plugin) else {
            self.plugin_reply(&[Span::colored(format!("/{name} is no longer provided by a plugin."), "red")]);
            return 0;
        };
        let player = match source {
            crate::commands::CommandSource::Player(conn) => self.players.get(&conn),
            crate::commands::CommandSource::Console => None,
        };
        let a = player.map(|p| actor(p, &pl.ops));
        let reply = pl.rt.run_command(plugin, a.as_ref(), name, args);
        if !reply.is_empty() {
            self.plugin_reply(&reply);
        }
        self.deliver_plugin_messages();
        1
    }

    /// `/kiln plugins`.
    fn kiln_plugins(&mut self) -> i32 {
        let Some(pl) = self.plugins.as_ref() else {
            self.plugin_reply(&[Span::colored("No plugins are loaded.", "gray")]);
            return 0;
        };
        let lines = pl.rt.describe();
        let stats: Vec<String> = pl.rt.stat_values().iter().map(|(k, v)| format!("{k} {v}")).collect();
        for l in &lines {
            self.plugin_reply(&[Span::colored(l.clone(), "gray")]);
        }
        self.plugin_reply(&[Span::colored(stats.join(", "), "dark_gray")]);
        lines.len() as i32
    }

    /// `/kiln plugins reload <id>`: compiles the plugin again from its directory off the
    /// tick; the swap and its report follow in a later B0.
    fn kiln_plugins_reload(&mut self, id: &str) -> i32 {
        let requester = match self.commands.source {
            crate::commands::CommandSource::Player(conn) => self.players.get(&conn).map(|p| p.uuid.as_u128()),
            crate::commands::CommandSource::Console => None,
        };
        let Some(pl) = self.plugins.as_mut() else { return 0 };
        match pl.rt.request_reload(id, requester) {
            Ok(()) => {
                self.plugin_reply(&[Span::colored(format!("Reloading plugin {id}..."), "gray")]);
                1
            }
            Err(e) => {
                self.plugin_reply(&[Span::colored(format!("Cannot reload {id}: {e:#}"), "red")]);
                0
            }
        }
    }

    /// Keeps each level's region instance sets in step with its regions.
    pub(crate) fn sync_plugin_regions(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        for (dim, d) in self.dims.iter().enumerate() {
            pl.rt.sync_regions(dim as u32, d.regions.iter().map(|r| r.id().0));
        }
    }

    /// B0: who is an operator; staged reloads, the global operations of the last tick, their
    /// results, due tasks; commands a reload added.
    pub(crate) fn plugins_b0(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        let ops: HashSet<Uuid> = self.players.values().filter(|p| self.commands.is_op(&p.name)).map(|p| p.uuid).collect();
        if ops != *pl.ops {
            pl.ops = Arc::new(ops);
        }
        let world = SimWorld { players: &self.players, dims: &self.dims, ops: &pl.ops };
        pl.rt.begin_tick_in(&world);
        let reloaded = pl.rt.take_reloaded();
        if reloaded.iter().any(|r| r.commands_changed) && self.register_plugin_commands() {
            let conns: Vec<ConnId> = self.players.keys().copied().collect();
            for conn in conns {
                self.send_command_tree(conn);
            }
        }
    }

    /// PX: chat goes through the player's region instances. Returns whether a plugin handled
    /// it (cancelled, or rewrote and broadcast it).
    pub(crate) fn plugin_chat(&mut self, conn: ConnId, message: &str) -> bool {
        let Some(pl) = self.plugins.as_mut() else { return false };
        let Some(p) = self.players.get(&conn) else { return false };
        let a = actor(p, &pl.ops);
        let Some(rp) = pl.rt.region_mut(p.dim as u32, p.region.0) else { return false };
        match rp.chat(&a, message) {
            ChatOutcome::Pass => false,
            ChatOutcome::Cancel => {
                info!("<{}> {message} (cancelled by a plugin)", p.name);
                true
            }
            ChatOutcome::Rewrite(spans) => {
                info!("{}", plain(&spans));
                self.broadcast(packets::system_chat(text_tag(&spans), false));
                true
            }
        }
    }

    /// PX: whether a region instance cancels a player's command.
    pub(crate) fn plugin_command_denied(&mut self, conn: ConnId, command: &str) -> bool {
        let Some(pl) = self.plugins.as_mut() else { return false };
        let Some(p) = self.players.get_mut(&conn) else { return false };
        let a = Actor { uuid: p.uuid.as_u128(), name: &p.name, operator: pl.ops.contains(&p.uuid) };
        let Some(rp) = pl.rt.region_mut(p.dim as u32, p.region.0) else { return false };
        match rp.command(&a, command) {
            Verdict::Allow => false,
            Verdict::Deny(msg) => {
                if let Some(m) = msg {
                    p.send(packets::system_chat(text_tag(&m), false));
                }
                true
            }
        }
    }

    pub(crate) fn plugins_joined(&mut self, conn: ConnId) {
        let Some(pl) = self.plugins.as_mut() else { return };
        let Some(p) = self.players.get(&conn) else { return };
        if self.commands.is_op(&p.name) {
            Arc::make_mut(&mut pl.ops).insert(p.uuid);
        }
        pl.rt.player_joined(&actor(p, &pl.ops));
    }

    pub(crate) fn plugins_left(&mut self, p: &Player) {
        let Some(pl) = self.plugins.as_mut() else { return };
        pl.rt.player_left(&actor(p, &pl.ops));
    }

    /// Sends what plugins said (after P and after G).
    pub(crate) fn deliver_plugin_messages(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        for m in pl.rt.take_messages() {
            let pkt = packets::system_chat(text_tag(&m.text), false);
            match m.to {
                None => self.broadcast(pkt),
                Some(u) => {
                    let uuid = Uuid::from_u128(u);
                    if let Some(p) = self.players.values_mut().find(|p| p.uuid == uuid) {
                        p.send(pkt);
                    }
                }
            }
        }
    }

    pub(crate) fn save_plugins(&self) {
        if let Some(pl) = &self.plugins {
            pl.rt.save();
        }
    }

    /// Plugin calls, traps and timeouts so far (tests and tools).
    pub fn plugin_stats(&self) -> (u64, u64, u64) {
        let Some(pl) = &self.plugins else { return (0, 0, 0) };
        (pl.rt.stat("calls"), pl.rt.stat("traps"), pl.rt.stat("timeouts"))
    }

    /// Every plugin statistic by name (tests and tools).
    pub fn plugin_stat(&self, name: &str) -> u64 {
        self.plugins.as_ref().map_or(0, |pl| pl.rt.stat(name))
    }

    /// A player's value of a plugin's key (tests and tools).
    pub fn plugin_player_value(&self, uuid: Uuid, plugin: &str, key: &str) -> Option<Vec<u8>> {
        self.plugins.as_ref()?.rt.player_value(uuid.as_u128(), plugin, key)
    }

    /// A plugin's current generation (tests and tools).
    pub fn plugin_generation(&self, plugin: &str) -> Option<u32> {
        let rt = &self.plugins.as_ref()?.rt;
        Some(rt.generation(rt.plugin_index(plugin)?))
    }

    pub(crate) fn hash_plugins(&self, h: &mut impl std::hash::Hasher) {
        if let Some(pl) = &self.plugins {
            pl.rt.hash_state(h);
        }
    }
}
